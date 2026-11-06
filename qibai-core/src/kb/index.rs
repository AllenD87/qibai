//! v45.10 P1：索引落盘 + 增量重建。
//!
//! - 落 `<data_dir>/kb/index.json` + `meta.json`（原子写 tmp+rename）
//! - 按 mtime+size 增量：未变文件跳过
//! - 只收 `.md`/`.txt`
//! - 幂等：重复 reindex 结果一致
//! - 文件删除后 reindex 把它移出索引
//! - 不跟随符号链接逃出 root（canonicalize 后校验）

use super::{chunk::*, KbError, KbStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexedFile {
    pub path: String,
    pub mtime_ms: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub files: Vec<IndexedFile>,
    pub chunks: Vec<Chunk>,
    pub indexed_at: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct IndexConfig {
    pub max_chunks: usize,
    pub chunk_cfg: ChunkConfig,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self { max_chunks: 20000, chunk_cfg: ChunkConfig::default() }
    }
}

/// 列出目录下所有 .md/.txt 文件（递归，不跟符号链接逃逸）。
fn list_files(root: &Path) -> Result<Vec<PathBuf>, KbError> {
    let root_canon = std::fs::canonicalize(root)
        .map_err(|e| KbError::Unreadable(format!("{}: {e}", root.display())))?;
    let mut out = Vec::new();
    walk(&root_canon, &root_canon, &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<(), KbError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(KbError::Unreadable(format!("{}: {e}", dir.display()))),
    };
    for ent in entries {
        let ent = ent?;
        let p = ent.path();
        // 不跟随符号链接
        let ft = ent.file_type()?;
        if ft.is_symlink() { continue; }
        if ft.is_dir() {
            // 校验仍在 root 内
            let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if !canon.starts_with(root) { continue; }
            walk(&p, root, out)?;
        } else if ft.is_file() {
            let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if !canon.starts_with(root) { continue; }
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext == "md" || ext == "txt" {
                    out.push(p);
                }
            }
        }
    }
    Ok(())
}

fn file_sig(p: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta.modified().ok()?
        .duration_since(std::time::UNIX_EPOCH).ok()?
        .as_millis() as u64;
    Some((mtime, meta.len()))
}

/// 全量重建（增量：未变文件跳过）。
pub fn reindex(folders: &[String], cfg: IndexConfig) -> Result<Index, KbError> {
    let idx = load_index().unwrap_or_default();
    let old_by_path: HashMap<String, IndexedFile> = idx.files.iter().cloned().map(|f| (f.path.clone(), f)).collect();
    let mut new_files: Vec<IndexedFile> = Vec::new();
    let mut new_chunks: Vec<Chunk> = Vec::new();

    for folder in folders {
        let root = PathBuf::from(folder);
        if !root.is_dir() {
            return Err(KbError::Unreadable(format!("不是目录或不存在: {folder}")));
        }
        let files = list_files(&root)?;
        for f in files {
            let fstr = f.to_string_lossy().to_string();
            let Some((mtime, size)) = file_sig(&f) else { continue };
            let changed = match old_by_path.get(&fstr) {
                Some(old) => old.mtime_ms != mtime || old.size != size,
                None => true,
            };
            new_files.push(IndexedFile { path: fstr.clone(), mtime_ms: mtime, size });
            if changed {
                let content = std::fs::read_to_string(&f)?;
                let mut cs = chunk_file(&fstr, &content, cfg.chunk_cfg)?;
                new_chunks.append(&mut cs);
            } else {
                // 未变：从旧索引挑出该文件的块
                let old: Vec<Chunk> = idx.chunks.iter().filter(|c| c.file_path == fstr).cloned().collect();
                new_chunks.extend(old);
            }
        }
    }

    // 上限检查
    if new_chunks.len() > cfg.max_chunks {
        return Err(KbError::TooManyChunks { would_be: new_chunks.len(), limit: cfg.max_chunks });
    }

    new_chunks.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.char_offset.cmp(&b.char_offset)));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let new_idx = Index { files: new_files, chunks: new_chunks, indexed_at: now };
    save_index(&new_idx)?;
    Ok(new_idx)
}

pub fn load_index() -> Result<Index, KbError> {
    let p = super::kb_dir().join("index.json");
    if !p.exists() { return Ok(Index::default()); }
    let s = std::fs::read_to_string(p)?;
    Ok(serde_json::from_str(&s)?)
}

fn save_index(idx: &Index) -> Result<(), KbError> {
    let dir = super::kb_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(".index.tmp.json");
    std::fs::write(&tmp, serde_json::to_string_pretty(idx)?)?;
    std::fs::rename(&tmp, dir.join("index.json"))?;
    Ok(())
}

pub fn status(idx: &Index, folders: &[String]) -> KbStatus {
    KbStatus {
        folders: folders.to_vec(),
        files: idx.files.len(),
        chunks: idx.chunks.len(),
        indexed_at: idx.indexed_at,
        last_error: None,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kb_{}_{}", tag, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reindex_idempotent() {
        let root = tmp_dir("idem");
        fs::write(root.join("a.md"), "# 部署\n安装步骤一。\n\n## 环境变量\n配置说明二。").unwrap();
        let folders = vec![root.to_string_lossy().to_string()];
        let cfg = IndexConfig::default();
        let i1 = reindex(&folders, cfg).unwrap();
        let n1 = i1.chunks.len();
        let i2 = reindex(&folders, cfg).unwrap();
        assert_eq!(i2.chunks.len(), n1, "重复 reindex 块数应一致");
        assert_eq!(i2.chunks, i1.chunks, "重复 reindex 块内容应一致");
    }

    #[test]
    fn deleted_file_drops_chunks() {
        let root = tmp_dir("del");
        fs::write(root.join("a.md"), "alpha alpha alpha").unwrap();
        fs::write(root.join("b.md"), "beta beta beta").unwrap();
        let folders = vec![root.to_string_lossy().to_string()];
        let cfg = IndexConfig::default();
        let i1 = reindex(&folders, cfg).unwrap();
        let n1 = i1.chunks.len();
        fs::remove_file(root.join("a.md")).unwrap();
        let i2 = reindex(&folders, cfg).unwrap();
        assert!(i2.chunks.len() < n1, "删文件后块数应下降");
        assert!(i2.chunks.iter().all(|c| !c.file_path.ends_with("a.md")));
    }

    #[test]
    fn max_chunks_err_has_numbers() {
        let root = tmp_dir("limit");
        fs::write(root.join("a.md"), "alpha").unwrap();
        let folders = vec![root.to_string_lossy().to_string()];
        let cfg = IndexConfig { max_chunks: 0, chunk_cfg: ChunkConfig::default() };
        let r = reindex(&folders, cfg);
        match r {
            Err(KbError::TooManyChunks { would_be, limit }) => {
                assert!(would_be > limit);
            }
            other => panic!("应返回 TooManyChunks: {:?}", other),
        }
    }

    #[test]
    fn nonexistent_folder_readable_error() {
        let folders = vec!["/tmp/definitely_not_exist_xyz_12345".into()];
        let r = reindex(&folders, IndexConfig::default());
        assert!(r.is_err());
    }
}
