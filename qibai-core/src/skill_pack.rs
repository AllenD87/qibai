//! 技能包能力：把插件/技能目录打包为可分发的 `.qbpk`（zip）技能包，
//! 支持导入安装（安全解压 → manifest 校验 → 注册表安装）与导出打包。
//!
//! 技能包 = 标准 zip 归档：
//! - 根目录含 `qibai-plugin.json`（Manifest）；
//! - 其余为插件资源（scripts/ 等）。
//!
//! 安全红线（导入侧）：
//! - 拒绝路径穿越（`..`、绝对路径、盘符前缀）与符号链接条目；
//! - 单文件 ≤ 20MB、解压总量 ≤ 200MB、条目数 ≤ 2000（zip 炸弹防护）；
//! - 解压先落临时目录，manifest 校验通过后才复制进 `plugins/`（原子性）。

use crate::plugin_manifest::ManifestLoader;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

/// 技能包文件扩展名。
pub const SKILL_PACK_EXT: &str = "qbpk";

/// 解压安全上限。
const MAX_ENTRIES: usize = 2000;
const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024; // 200MB
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024; // 单文件 20MB

/// 技能包错误。
#[derive(Debug, Clone, thiserror::Error)]
#[error("技能包错误 [{field}]: {message}")]
pub struct SkillPackError {
    pub field: String,
    pub message: String,
}

impl SkillPackError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// 跳过打包的目录（运行时产物 / 版本库，不属于技能内容）。
fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".DS_Store" | "__pycache__"
    )
}

/// 把插件目录打包为技能包 zip。
pub fn pack(dir: &Path, zip_path: &Path) -> Result<(), SkillPackError> {
    if !dir.is_dir() {
        return Err(SkillPackError::new("dir", format!("{} 不是目录", dir.display())));
    }
    let manifest_file = dir.join("qibai-plugin.json");
    if !manifest_file.exists() {
        return Err(SkillPackError::new(
            "manifest",
            "插件目录缺少 qibai-plugin.json",
        ));
    }
    let file = std::fs::File::create(zip_path)
        .map_err(|e| SkillPackError::new("zip", format!("创建技能包失败: {e}")))?;
    let mut zw = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    walk(dir, dir, &mut zw, &options, 0)?;
    zw.finish()
        .map_err(|e| SkillPackError::new("zip", format!("技能包写盘失败: {e}")))?;
    Ok(())
}

/// 递归写入目录内容；`base` 为打包根，写入相对路径。
fn walk(
    root: &Path,
    dir: &Path,
    zw: &mut zip::ZipWriter<std::fs::File>,
    options: &zip::write::FileOptions<()>,
    depth: usize,
) -> Result<(), SkillPackError> {
    if depth > 64 {
        return Err(SkillPackError::new("zip", "目录层级过深"));
    }
    let rd = std::fs::read_dir(dir)
        .map_err(|e| SkillPackError::new("zip", format!("读取目录失败: {e}")))?;
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            walk(root, &path, zw, options, depth + 1)?;
        } else if path.is_file() {
            if name == ".DS_Store" || name == "Thumbs.db" {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| SkillPackError::new("zip", format!("相对路径计算失败: {e}")))?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let content = std::fs::read(&path)
                .map_err(|e| SkillPackError::new("zip", format!("读取 {} 失败: {e}", path.display())))?;
            // 保留 unix 执行位（技能包脚本在解压后需可执行）。
            #[cfg(unix)]
            let entry_options = {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path)
                    .map(|m| m.permissions().mode())
                    .unwrap_or(0o644);
                options.unix_permissions(mode)
            };
            #[cfg(not(unix))]
            let entry_options = *options;
            zw.start_file(&rel_str, entry_options)
                .map_err(|e| SkillPackError::new("zip", format!("写入条目失败: {e}")))?;
            zw.write_all(&content)
                .map_err(|e| SkillPackError::new("zip", format!("写入内容失败: {e}")))?;
        }
    }
    Ok(())
}

/// 安全解压到目标目录；全部校验通过后才开始写盘（先收集清单）。
pub fn unpack(zip_path: &Path, dest: &Path) -> Result<Vec<PathBuf>, SkillPackError> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| SkillPackError::new("zip", format!("打开技能包失败: {e}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| SkillPackError::new("zip", format!("技能包解析失败: {e}")))?;
    let count = archive.len();
    if count > MAX_ENTRIES {
        return Err(SkillPackError::new(
            "zip",
            format!("条目数 {count} 超上限 {MAX_ENTRIES}"),
        ));
    }
    // 第一遍：收集并校验（路径安全 + 大小预算）。
    #[derive(Default)]
    struct Planned {
        idx: usize,
        path: PathBuf,
        size: u64,
    }
    let mut planned: Vec<Planned> = Vec::with_capacity(count);
    let mut total: u64 = 0;
    for i in 0..count {
        let zf = archive
            .by_index(i)
            .map_err(|e| SkillPackError::new("zip", format!("读取条目失败: {e}")))?;
        let name = zf.enclosed_name().ok_or_else(|| {
            SkillPackError::new(
                "zip",
                format!("条目 {} 含非法路径（.. / 绝对路径 / 盘符）", i),
            )
        })?;
        // 只允许普通文件与目录；拒绝符号链接（zip 的 unix 模式位）。
        #[allow(clippy::cast_possible_wrap)]
        {
            let mode = zf.unix_mode().unwrap_or(0);
            let is_symlink = (mode & 0o170000) == 0o120000;
            if is_symlink {
                return Err(SkillPackError::new(
                    "zip",
                    format!("条目 {} 为符号链接，拒绝", i),
                ));
            }
        }
        if zf.is_dir() {
            continue;
        }
        let size = zf.size();
        if size > MAX_FILE_BYTES {
            return Err(SkillPackError::new(
                "zip",
                format!("条目 {} 大小 {size} 超单文件上限 {MAX_FILE_BYTES}", i),
            ));
        }
        total += size;
        if total > MAX_TOTAL_BYTES {
            return Err(SkillPackError::new(
                "zip",
                format!("解压总量超上限 {MAX_TOTAL_BYTES}"),
            ));
        }
        planned.push(Planned {
            idx: i,
            path: dest.join(name),
            size,
        });
    }
    // 第二遍：实际写盘（按原 zip 条目索引读取）。
    std::fs::create_dir_all(dest)
        .map_err(|e| SkillPackError::new("zip", format!("创建解压目录失败: {e}")))?;
    let mut written: Vec<PathBuf> = Vec::new();
    for p in planned.iter() {
        let mut zf = archive
            .by_index(p.idx)
            .map_err(|e| SkillPackError::new("zip", format!("读取条目失败: {e}")))?;
        #[cfg(unix)]
        let unix_mode = zf.unix_mode();
        if let Some(parent) = p.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SkillPackError::new("zip", format!("创建目录失败: {e}")))?;
        }
        let mut out = std::fs::File::create(&p.path)
            .map_err(|e| SkillPackError::new("zip", format!("写入文件失败: {e}")))?;
        // 按预算上限拷贝，防止压缩炸弹放大。
        let mut remaining = p.size;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if remaining == 0 {
                break;
            }
            let want = std::cmp::min(buf.len() as u64, remaining) as usize;
            let n = zf
                .read(&mut buf[..want])
                .map_err(|e| SkillPackError::new("zip", format!("读取失败: {e}")))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .map_err(|e| SkillPackError::new("zip", format!("写入失败: {e}")))?;
            remaining -= n as u64;
        }
        // 恢复 unix 执行位（打包时保留的权限位）。
        #[cfg(unix)]
        if let Some(mode) = unix_mode {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p.path, std::fs::Permissions::from_mode(mode));
        }
        written.push(p.path.clone());
    }
    Ok(written)
}

/// 导入安装技能包：安全解压到临时目录 → manifest 校验 → 复制进 plugins/。
/// 返回校验通过的 Manifest。失败时清理临时目录。
pub fn install_pack(
    zip_path: &Path,
    plugins_dir: &Path,
) -> Result<crate::plugin_manifest::Manifest, SkillPackError> {
    let tmp = std::env::temp_dir().join(format!(
        "qibai_skillpack_{}_{}",
        std::process::id(),
        zip_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("pkg")
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    let written = unpack(zip_path, &tmp)?;
    // ManifestLoader::load 以目录为入参（内部拼 qibai-plugin.json）。
    let manifest = ManifestLoader::load(&tmp)
        .map_err(|e| SkillPackError::new("manifest", format!("清单无效: {e}")))?;
    // 外部包声明 kind=builtin → 拒绝（与 registry 一致）。
    let is_builtin_kind = manifest
        .entry
        .as_ref()
        .map(|e| e.kind == crate::plugin_manifest::EntryKind::Builtin)
        .unwrap_or(false);
    if is_builtin_kind {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(SkillPackError::new(
            "manifest",
            "技能包声明 kind=builtin 被拒绝（builtin 仅限内置包）",
        ));
    }
    // 复制进 plugins/<id>/（registry.install 同语义：覆盖同 id）。
    let target = plugins_dir.join(&manifest.id);
    let _ = std::fs::remove_dir_all(&target);
    crate::skill_adapter::copy_dir_all(&tmp, &target)
        .map_err(|e| SkillPackError::new("install", format!("安装失败: {e}")))?;
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = written; // 解压产物已复制
    // 按最终落点重载清单：`{PLUGIN_DIR}` 若按暂存目录解析，装完的入口会指向已删除的 /tmp。
    let installed = ManifestLoader::load(&target)
        .map_err(|e| SkillPackError::new("manifest", format!("安装后清单重载失败: {e}")))?;
    Ok(installed)
}

/// 导出技能包：从 plugins/<id> 打包为 zip。
pub fn export_pack(
    plugin_id: &str,
    plugins_dir: &Path,
    zip_path: &Path,
) -> Result<(), SkillPackError> {
    let dir = plugins_dir.join(plugin_id);
    if !dir.is_dir() {
        return Err(SkillPackError::new(
            "id",
            format!("插件 {plugin_id} 不存在或未安装"),
        ));
    }
    pack(&dir, zip_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qibai_skillpack_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_plugin(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(
            dir.join("qibai-plugin.json"),
            format!(
                r#"{{
  "id": "{id}",
  "type": "skill",
  "name": "测试技能",
  "version": "1.0.0",
  "permissions": ["command:run"],
  "entry": {{"kind": "command", "command": "scripts/run.sh"}},
  "functions": [{{"name": "test_fn", "description": "测试", "parameters": {{"type": "object"}}}}]
}}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                dir.join("scripts/run.sh"),
                std::fs::Permissions::from_mode(0o755),
            );
        }
    }

    #[test]
    fn pack_and_unpack_roundtrip() {
        let root = tmp_dir("rt");
        let plugin_dir = root.join("plugin");
        write_plugin(&plugin_dir, "com.test.roundtrip");
        let zip_path = root.join("test.qbpk");
        pack(&plugin_dir, &zip_path).unwrap();
        assert!(zip_path.exists());
        let dest = root.join("out");
        let written = unpack(&zip_path, &dest).unwrap();
        assert!(dest.join("qibai-plugin.json").exists());
        assert!(dest.join("scripts/run.sh").exists());
        assert!(written.len() >= 2);
    }

    #[test]
    fn install_pack_installs_into_plugins_dir() {
        let root = tmp_dir("install");
        let plugin_dir = root.join("plugin");
        write_plugin(&plugin_dir, "com.test.install");
        let zip_path = root.join("p.qbpk");
        pack(&plugin_dir, &zip_path).unwrap();
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let m = install_pack(&zip_path, &plugins).unwrap();
        assert_eq!(m.id, "com.test.install");
        assert!(plugins.join("com.test.install/qibai-plugin.json").exists());
        assert!(plugins
            .join("com.test.install/scripts/run.sh")
            .exists());
    }

    #[test]
    fn unpack_rejects_path_traversal() {
        let root = tmp_dir("evil");
        let zip_path = root.join("evil.qbpk");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        // 路径穿越条目。
        zw.start_file("../evil.txt", opts).unwrap();
        zw.write_all(b"boom").unwrap();
        zw.finish().unwrap();
        let dest = root.join("out");
        let err = unpack(&zip_path, &dest);
        assert!(err.is_err());
    }

    #[test]
    fn install_pack_rejects_builtin_kind() {
        let root = tmp_dir("builtin");
        let plugin_dir = root.join("plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("qibai-plugin.json"),
            r#"{"id":"com.test.builtin","type":"skill","name":"x","version":"1.0.0",
               "entry":{"kind":"builtin"},"functions":[]}"#,
        )
        .unwrap();
        let zip_path = root.join("b.qbpk");
        pack(&plugin_dir, &zip_path).unwrap();
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        assert!(install_pack(&zip_path, &plugins).is_err());
    }

    #[test]
    fn export_pack_roundtrip() {
        let root = tmp_dir("export");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        write_plugin(&plugins.join("com.test.export"), "com.test.export");
        let zip_path = root.join("exported.qbpk");
        export_pack("com.test.export", &plugins, &zip_path).unwrap();
        assert!(zip_path.exists());
        let dest = root.join("out");
        unpack(&zip_path, &dest).unwrap();
        assert!(dest.join("qibai-plugin.json").exists());
    }
}
