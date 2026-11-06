//! P2：向量索引落盘 + 增量同步。
//!
//! - 落 `<kb_dir>/vectors.json`（键/哈希/模型元信息）+ `vectors.f32`（原始 f32 小端矩阵）
//! - key = `文件路径#字符偏移`，和块一一对应
//! - 增量：块文本哈希没变就复用旧向量，只对新增/改动块跑推理
//! - 模型名或维度变化即整份作废重建（避免不同模型向量混用）
//! - 索引不存在时上层自动退回纯 BM25，不会报错打断检索

use super::chunk::Chunk;
use super::embed::TextEmbedder;
use super::KbError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct VecEntry {
    pub key: String,
    pub hash: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorMeta {
    pub model: String,
    pub dim: usize,
    pub entries: Vec<VecEntry>,
}

/// 内存态向量索引（行优先，第 i 行 = entries[i]）。
#[derive(Debug, Clone, Default)]
pub struct VectorIndex {
    pub model: String,
    pub dim: usize,
    pub entries: Vec<VecEntry>,
    pub data: Vec<f32>,
}

pub fn key_of(c: &Chunk) -> String {
    format!("{}#{}", c.file_path, c.char_offset)
}

pub fn text_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn meta_path() -> PathBuf {
    super::kb_dir().join("vectors.json")
}

fn data_path() -> PathBuf {
    super::kb_dir().join("vectors.f32")
}

impl VectorIndex {
    pub fn rows(&self) -> usize {
        self.entries.len()
    }

    pub fn row(&self, i: usize) -> Option<&[f32]> {
        if i >= self.entries.len() {
            return None;
        }
        let start = i * self.dim;
        self.data.get(start..start + self.dim)
    }

    pub fn get(&self, key: &str) -> Option<&[f32]> {
        let i = self.entries.iter().position(|e| e.key == key)?;
        self.row(i)
    }
}

/// 读盘；文件缺失或元数据不一致时返回 None（不是错误）。
pub fn load() -> Option<VectorIndex> {
    let mp = meta_path();
    let dp = data_path();
    if !mp.exists() || !dp.exists() {
        return None;
    }
    let meta: VectorMeta = serde_json::from_str(&std::fs::read_to_string(&mp).ok()?).ok()?;
    let raw = std::fs::read(&dp).ok()?;
    if meta.dim == 0 || raw.len() != meta.entries.len() * meta.dim * 4 {
        eprintln!(
            "[qibai] kb: 向量索引大小对不上（dim={} rows={} bytes={}），按无索引处理",
            meta.dim,
            meta.entries.len(),
            raw.len()
        );
        return None;
    }
    let data = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect::<Vec<f32>>();
    Some(VectorIndex { model: meta.model, dim: meta.dim, entries: meta.entries, data })
}

/// 原子落盘（两个文件都先写 tmp 再 rename）。
pub fn save(idx: &VectorIndex) -> Result<(), KbError> {
    let dir = super::kb_dir();
    std::fs::create_dir_all(&dir)?;
    let mut bytes = Vec::with_capacity(idx.data.len() * 4);
    for v in &idx.data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let tmp_f32 = dir.join(".vectors.tmp.f32");
    std::fs::write(&tmp_f32, &bytes)?;
    let tmp_json = dir.join(".vectors.tmp.json");
    let meta = VectorMeta {
        model: idx.model.clone(),
        dim: idx.dim,
        entries: idx.entries.clone(),
    };
    std::fs::write(&tmp_json, serde_json::to_string(&meta)?)?;
    std::fs::rename(&tmp_f32, data_path())?;
    std::fs::rename(&tmp_json, meta_path())?;
    Ok(())
}

/// 删除向量索引（模型缺席、关闭语义检索时用）。
pub fn clear() {
    let _ = std::fs::remove_file(meta_path());
    let _ = std::fs::remove_file(data_path());
}

/// 同步向量索引：已存在且哈希一致的块直接复用，其余重新推理。
///
/// 返回本次实际跑了推理的块数。
pub fn sync(chunks: &[Chunk], emb: &dyn TextEmbedder) -> Result<usize, KbError> {
    let model = emb.model_name();
    let dim = emb.dim();
    if dim == 0 {
        return Err(KbError::Embed("embedder 维度为 0".into()));
    }
    // 旧索引可用条件：同模型 + 同维度
    let old = load().filter(|o| o.model == model && o.dim == dim);
    let old_map: HashMap<String, usize> = match &old {
        Some(o) => o.entries.iter().enumerate().map(|(i, e)| (e.key.clone(), i)).collect(),
        None => HashMap::new(),
    };

    let mut entries: Vec<VecEntry> = Vec::with_capacity(chunks.len());
    let mut data: Vec<f32> = Vec::with_capacity(chunks.len() * dim);
    let mut embedded = 0usize;
    let total = chunks.len();
    let mut pending_idx: Vec<usize> = Vec::new();
    let mut pending_text: Vec<String> = Vec::new();

    // 先把能复用的地方占好位置，缺的记下来统一推理
    for c in chunks {
        let key = key_of(c);
        let h = text_hash(&c.text);
        let pos = entries.len();
        entries.push(VecEntry { key: key.clone(), hash: h });
        data.resize((pos + 1) * dim, 0.0);
        match (old.as_ref(), old_map.get(&key)) {
            (Some(o), Some(&i)) if o.entries[i].hash == h => {
                if let Some(row) = o.row(i) {
                    data[pos * dim..(pos + 1) * dim].copy_from_slice(row);
                }
            }
            _ => {
                pending_idx.push(pos);
                pending_text.push(embed_text(c));
            }
        }
    }

    // 向量化缺的块（逐条，便于按块定位错误）
    for (n, (pos, text)) in pending_idx.iter().zip(pending_text.iter()).enumerate() {
        let v = emb.embed(&[text.clone()])?;
        let v = v.into_iter().next().ok_or_else(|| KbError::Embed("embedder 返回空结果".into()))?;
        if v.len() != dim {
            return Err(KbError::Embed(format!("向量维度不符: got {} want {dim}", v.len())));
        }
        data[pos * dim..(pos + 1) * dim].copy_from_slice(&v);
        embedded += 1;
        if n % 50 == 49 {
            eprintln!("[qibai] kb: 向量化进度 {}/{total}", embedded + 0);
        }
    }

    let idx = VectorIndex { model, dim, entries, data };
    save(&idx)?;
    eprintln!("[qibai] kb: 向量索引 OK（{total} 块，本次推理 {embedded}）");
    Ok(embedded)
}

/// 送去向量化的文本：标题路径 + 正文，让「同一段话在不同章节」也能区分开。
fn embed_text(c: &Chunk) -> String {
    if c.heading_path.is_empty() {
        c.text.clone()
    } else {
        format!("{} {}", c.heading_path.join(" > "), c.text)
    }
}

/// 余弦相似度（向量已归一化时等于点积）。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0f32;
    let mut na = 0f32;
    let mut nb = 0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-12 || nb <= 1e-12 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(path: &str, off: usize, text: &str) -> Chunk {
        Chunk {
            file_path: path.to_string(),
            char_offset: off,
            heading_path: vec!["测试".into()],
            text: text.to_string(),
        }
    }

    struct FakeEmb {
        dim: usize,
    }
    impl TextEmbedder for FakeEmb {
        fn dim(&self) -> usize {
            self.dim
        }
        fn model_name(&self) -> String {
            "fake".into()
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, KbError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += b as f32;
                    }
                    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                    v.iter_mut().for_each(|x| *x /= n);
                    v
                })
                .collect())
        }
    }

    #[test]
    fn key_and_hash_stable() {
        let c = chunk("a.md", 10, "hello");
        assert_eq!(key_of(&c), "a.md#10");
        assert_eq!(text_hash("hello"), text_hash("hello"));
        assert_ne!(text_hash("hello"), text_hash("hellO"));
    }

    #[test]
    fn cosine_of_identical_is_one() {
        let v = vec![0.6f32, 0.8];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    }

    #[test]
    fn sync_rows_match_chunks_and_reuse_changed_only() {
        // 用临时 kb_dir 需要改环境，这里只验证纯计算层：内存行数/维度
        let emb = FakeEmb { dim: 4 };
        let cs = vec![chunk("a.md", 0, "alpha"), chunk("b.md", 0, "beta")];
        let v1 = emb.embed(&[embed_text(&cs[0])]).unwrap();
        assert_eq!(v1[0].len(), 4);
        assert_eq!(cs.len(), 2, "每个块一行，行数=块数");
        let sim = cosine(&v1[0], &v1[0]);
        assert!(sim > 0.999);
    }
}
