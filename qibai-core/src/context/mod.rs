//! v45.9 P0：上下文与提示词管理（四层组装 + 快照 + 压缩 v1）。
//!
//! 规格：`docs/v45.9_长任务与上下文管理方案.md` 第 5、6 节。
//!
//! # 模块边界
//! - [`budget`]   token 估算、预算切分、触发判断；
//! - [`layers`]   L0~L3 组装、智能截断（不丢数字/路径）；
//! - [`facts`]    事实卡抽取与 cap 淘汰；
//! - [`compress`] 压缩 v1（最近 N 轮 + 结构化摘要 + 防丢自校验 + Degraded 不静默）；
//! - [`snapshot`] 每次运行快照落盘、prompt_version hash、parent_run_id 链。
//!
//! # 公共 API（引擎接线用）
//! - [`ContextConfig`] / [`LayerBudgets`] / [`estimate_tokens`] / [`split_budget`] / [`should_compress`]
//! - [`Message`] / [`SkillBrief`] / [`SkillFull`] / [`TaskBrief`] / [`KbHit`] / [`PriorSkillSummary`] / [`LayerInput`] / [`RenderedLayers`] / [`assemble`]
//! - [`FactCard`] / [`FactKind`] / [`extract_facts`] / [`enforce_cap`]
//! - [`Summary`] / [`Backfill`] / [`CompressionState`] / [`CompressionOutcome`] / [`SummaryProvider`] / [`compress`] / [`render_summary_block`]
//! - [`ContextSnapshot`] / [`prompt_version`] / [`write_snapshot`] / [`walk_chain`]
//!
//! 本模块不依赖 `qibai_core::task`、`engine`、`tauri`；只依赖 serde / sha2 / std。

pub mod budget;
pub mod compress;
pub mod facts;
pub mod layers;
pub mod snapshot;

pub use budget::{estimate_tokens, should_compress, split_budget, ContextConfig, LayerBudgets};
pub use compress::{
    compress, render_summary_block, Backfill, CompressionOutcome, CompressionState, Summary,
    SummaryProvider,
};
pub use facts::{enforce_cap, extract_facts, FactCard, FactKind, FactMessage};
pub use layers::{
    assemble, KbHit, LayerInput, Message, PriorSkillSummary, RenderedLayers, SkillBrief, SkillFull,
    TaskBrief,
};
pub use snapshot::{prompt_version, walk_chain, write_snapshot, ContextSnapshot};
