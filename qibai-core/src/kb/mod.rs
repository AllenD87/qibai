//! v45.10 P1 知识库（纯本地 BM25，不引模型/不联网）。
//!
//! 四块：
//! - [`chunk`]：Markdown 按标题层级切块，单块 ~700 字、重叠 80
//! - [`index`]：落 `<data_dir>/kb/`，按 mtime+size 增量重建
//! - [`search`]：BM25（k1=1.2/b=0.75），中文二元组 + 英文按词
//! - 本文件：Error / KbHit / KbStatus 类型
//!
//! P2 向量检索不做（不引嵌入模型、不下载模型）。

pub mod chunk;
pub mod index;
pub mod search;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 知识库错误。
#[derive(Debug, thiserror::Error)]
pub enum KbError {
    #[error("kb 目录不可读: {0}")]
    Unreadable(String),
    #[error("路径不在所选目录内（越界/符号链接逃逸）: {0}")]
    EscapesRoot(String),
    #[error("块数超限：would_be={would_be} > limit={limit}")]
    TooManyChunks { would_be: usize, limit: usize },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// 单块命中。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KbHit {
    pub file_path: String,
    pub char_offset: usize,
    pub heading_path: Vec<String>,
    pub score: f32,
    pub snippet: String,
}

/// 知识库整体状态（IPC 返回）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KbStatus {
    pub folders: Vec<String>,
    pub files: usize,
    pub chunks: usize,
    pub indexed_at: u64,
    pub last_error: Option<String>,
}

/// 索引在磁盘上的根目录。
pub fn kb_dir() -> PathBuf {
    crate::settings::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("qibai_kb"))
}
