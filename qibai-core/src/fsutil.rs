//! 原子持久化工具（问题4缺陷B / 问题7-2）。
//!
//! 背景：此前全部 JSON 状态文件（conversation/settings/personas/intimacy/social/vault.qbkv）
//! 均用 `std::fs::write` 直接覆盖目标文件——写一半崩溃即把文件截成半截 JSON，
//! 下次启动 `from_str` 失败后又被 `unwrap_or_default` 静默吞掉，用户数据"凭空消失"。
//!
//! 本模块提供两件事：
//! 1. [`atomic_write_json`]：写 `<path>.tmp` → fsync → rename 替换目标（POSIX rename 原子），
//!    任何时刻目标文件要么是旧完整版、要么是新完整版，绝不出现半截。
//! 2. [`load_json_or_backup`]：文件损坏时把坏件改名 `<path>.corrupt-<ts>` 备份后返回 None，
//!    调用方回退默认——**绝不静默吞错、绝不覆盖坏件**。日志只打印路径与字节数，
//!    不打印明文内容（隐私红线：conversation/配置明文不进日志）。

use std::io::Write;
use std::path::{Path, PathBuf};

/// 计算临时文件路径：`<path>.tmp`（不依赖扩展名，兼容 .json / .qbkv）。
pub fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// 把已序列化好的 JSON 文本原子写入 `path`。
///
/// 流程：写 `<path>.tmp` → `File::sync_all`（落盘）→ `rename` 覆盖目标。
/// 任何一步失败都返回 Err，且目标文件保持上一次的完整内容。
pub fn atomic_write_json(path: &Path, raw: &str) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(raw.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 读取并反序列化 `path`；文件不存在或损坏时返回 None。
///
/// 损坏时：把坏件改名 `<path>.corrupt-<unix秒>` 保留（便于用户/开发者事后排查），
/// 并 eprintln 一行告警（仅路径 + 字节数，不含明文）。
pub fn load_json_or_backup<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&raw) {
        Ok(v) => Some(v),
        Err(_) => {
            let backup = path.with_extension(format!(
                "corrupt-{}",
                crate::storage::now_ts()
            ));
            match std::fs::rename(path, &backup) {
                Ok(_) => eprintln!(
                    "[qibai] 状态文件损坏已备份（不打印内容）: {} ({} bytes) -> {}",
                    path.display(),
                    raw.len(),
                    backup.display()
                ),
                Err(e) => eprintln!(
                    "[qibai] 状态文件损坏且备份失败: {} ({} bytes, rename 错误: {e})",
                    path.display(),
                    raw.len()
                ),
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_then_read_roundtrip() {
        let tmp = crate::test_support::unique_temp_dir("fsutil");
        let p = tmp.join("state.json");
        atomic_write_json(&p, "{\"a\":1}").unwrap();
        let v: serde_json::Value = load_json_or_backup(&p).unwrap();
        assert_eq!(v["a"], 1);
        // 写后不留 .tmp 残留
        assert!(!tmp_path(&p).exists());
    }

    #[test]
    fn corrupt_file_is_backed_up_not_silently_dropped() {
        let tmp = crate::test_support::unique_temp_dir("fsutil2");
        let p = tmp.join("state.json");
        std::fs::write(&p, "{broken json").unwrap();
        let _g = crate::test_support::env_lock();
        let got: Option<serde_json::Value> = load_json_or_backup(&p);
        assert!(got.is_none(), "损坏文件应回退 None");
        // 坏件已被改名备份，原路径不再存在
        assert!(!p.exists());
        let backups: Vec<_> = std::fs::read_dir(&tmp).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1, "应保留一份 .corrupt 备份");
        let name = backups[0].file_name().into_string().unwrap();
        assert!(name.contains("corrupt"), "备份文件名含 corrupt");
    }

    #[test]
    fn missing_file_returns_none_without_panic() {
        let tmp = crate::test_support::unique_temp_dir("fsutil3");
        let p = tmp.join("nope.json");
        let got: Option<serde_json::Value> = load_json_or_backup(&p);
        assert!(got.is_none());
    }
}
