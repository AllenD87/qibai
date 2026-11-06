//! v45.10 P1 知识库（本地检索：BM25 + 可选本地向量，全程不联网）。
//!
//! 四块：
//! - [`chunk`]：Markdown 按标题层级切块，单块 ~700 字、重叠 80
//! - [`index`]：落 `<data_dir>/kb/`，按 mtime+size 增量重建
//! - [`search`]：BM25（k1=1.2/b=0.75），中文二元组 + 英文按词；配了向量就走混合检索
//! - [`embed`]：本地 bge-small-zh-v1.5 向量化（tract 纯 Rust 推理，模型按需下载，不上传文本）
//! - [`vectors`]：向量索引落盘 + 增量同步
//! - 本文件：Error / KbHit / KbStatus 类型
//!
//! P2 已接向量检索：模型缺席/加载失败自动退回纯 BM25，检索本身不依赖网络。

pub mod chunk;
pub mod embed;
pub mod index;
pub mod search;
pub mod vectors;

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
    /// 向量化相关（模型缺失/推理失败）。上层按「降级到 BM25」处理，不当致命错误。
    #[error("embed: {0}")]
    Embed(String),
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
    /// 向量检索用的模型名（None = 未安装/未启用，走纯 BM25）。
    #[serde(default)]
    pub embed_model: Option<String>,
    /// 已有向量的块数。
    #[serde(default)]
    pub embed_chunks: usize,
    /// 向量化降级原因（给人看的一句话）。
    #[serde(default)]
    pub embed_note: Option<String>,
}

/// 索引在磁盘上的根目录。
pub fn kb_dir() -> PathBuf {
    crate::settings::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("qibai_kb"))
}
