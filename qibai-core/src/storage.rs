//! 对话记录本地持久化（JSON）。
//!
//! 文件：`<data_dir>/conversation.json`，整文件读写（MVP 从简）。
//! 隐私本地优先：不上传。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::settings::data_dir;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings error: {0}")]
    Settings(#[from] crate::settings::SettingsError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
    pub ts: u64,
}

pub struct ConversationStore {
    path: std::path::PathBuf,
    messages: Vec<StoredMessage>,
}

/// 当前 Unix 秒时间戳（供 personas/social 模块共用）。
pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// epoch 秒 → `YYYYmmdd-HHMMSS`（UTC，Hinnant civil-from-days 算法；不引 chrono）。
fn utc_stamp(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86400) as i64;
    let sod = epoch_secs % 86400;
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

impl Default for ConversationStore {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self::empty())
    }
}

impl ConversationStore {
    pub fn new() -> Result<Self, StorageError> {
        let path = data_dir()?.join("conversation.json");
        // 问题4缺陷B：损坏文件由 fsutil 备份为 .corrupt-<ts> 后回退空 Vec（不再静默吞错）。
        let messages = crate::fsutil::load_json_or_backup(&path).unwrap_or_default();
        Ok(Self { path, messages })
    }

    fn empty() -> Self {
        Self {
            path: std::env::temp_dir().join("qibai-conversation-fallback.json"),
            messages: Vec::new(),
        }
    }

    pub fn append(&mut self, role: &str, content: &str) -> Result<(), StorageError> {
        self.messages.push(StoredMessage {
            role: role.to_string(),
            content: content.to_string(),
            ts: now_ts(),
        });
        self.persist()
    }

    pub fn all(&self) -> &[StoredMessage] {
        &self.messages
    }

    /// 文件路径（测试/排查用）。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 清空前备份：把 conversation.json 复制为同目录 `conversation.json.bak-<YYYYmmdd-HHMMSS>`。
    /// 文件不存在（空会话）→ 返回 Ok(None)，不报错、不创建空备份。
    pub fn backup(&self) -> Result<Option<std::path::PathBuf>, StorageError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let stamp = utc_stamp(now_ts());
        let file_name = self
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "conversation.json".to_string());
        let bak_name = format!("{file_name}.bak-{stamp}");
        let bak = self.path.with_file_name(bak_name);
        std::fs::copy(&self.path, &bak)?;
        Ok(Some(bak))
    }

    pub fn clear(&mut self) -> Result<(), StorageError> {
        self.messages.clear();
        self.persist()
    }

    fn persist(&self) -> Result<(), StorageError> {
        let raw = serde_json::to_string_pretty(&self.messages)?;
        // 问题4缺陷B：原子写（tmp+rename），防写半截损坏 conversation.json。
        crate::fsutil::atomic_write_json(&self.path, &raw)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_back() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("storage");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mut s = ConversationStore::new().unwrap();
        s.append("user", "你好").unwrap();
        s.append("assistant", "你好呀").unwrap();
        assert_eq!(s.all().len(), 2);
        assert_eq!(s.all()[0].role, "user");
        assert_eq!(s.all()[1].content, "你好呀");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn reloading_from_disk_round_trips() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("storage2");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        {
            let mut s = ConversationStore::new().unwrap();
            s.append("user", "持久化测试").unwrap();
        }
        // 重新打开
        let s2 = ConversationStore::new().unwrap();
        assert_eq!(s2.all().len(), 1);
        assert_eq!(s2.all()[0].content, "持久化测试");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn clear_empties_file() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("storage3");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mut s = ConversationStore::new().unwrap();
        s.append("user", "x").unwrap();
        s.clear().unwrap();
        assert_eq!(s.all().len(), 0);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }
}
