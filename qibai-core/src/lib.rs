//! 《元气小小》Qibai —— AI 桌面伴侣核心引擎。
//!
//! 本 crate 与 Tauri / GUI 完全解耦：不依赖 tauri 及其 sys 系 crate，
//! 保证在无 webkit2gtk/gtk3/libsoup/javascriptcoregtk 等系统库的环境下
//! 也能独立 `cargo build` / `cargo test`。
//!
//! 模块：
//! - [`settings`]    设置持久化（JSON 存平台应用数据目录）
//! - [`llm_client`]  LlmProvider trait + 豆包 Ark 实现 + agent loop
//! - [`persona`]      女友人设 system prompt + JSON 回复解析
//! - [`skills`]       技能注册表（白名单/权限/危险操作审批）
//! - [`reminder`]     久坐/喝水/护眼调度
//! - [`storage`]      对话记录本地持久化
//! - [`task_orchestrator`] 任务编排运行时（生命周期/投递窗口/打断≠取消，借鉴 Qwen-Audio-Agent）
//! - [`event_bus`]    统一事件协议（静默更新/调度响应/打断响应）
//! - [`memory_layers`] 记忆分层与修订检查（当前请求>显式>推断）
//! - [`realtime_voice_frontend`] Realtime 兼容本地语音前端契约（引擎无关）
//! - [`doubao_work`]  豆包办公能力接入（预留 trait + 凭据运行时读取）
//! - [`engine`]       IPC 门面（tauri 命令的薄封装）
//! - [`enums`]        emotion / action 枚举
//! - [`guard`]        全屏守护演出状态机（久坐休息提醒）
//! - [`codecheck`]    本地代码静态检查（只读，node/python/启发式）

pub mod codecheck;
pub mod proc_env;
pub mod context;
pub mod weather;
pub mod aec;
pub mod connectors;
pub mod skill_pack;
pub mod skill_docs;
pub mod skill_install;
pub mod audio_pulse;
pub mod doubao_work;
pub mod engine;
pub mod enums;
pub mod expert;
pub mod fileops;
pub mod fsutil;
pub mod guard;
pub mod intimacy;
pub mod llm_client;
pub mod netsocial;
pub mod panel;
pub mod screen;
pub mod persona;
pub mod personas;
pub mod plugin_manifest;
pub mod plugin_registry;
pub mod project_context;
pub mod project_scaffold;
pub mod reminder;
pub mod settings;
pub mod skill_adapter;
pub mod skills;
pub mod social;
pub mod storage;
pub mod task;
pub mod task_exec;
pub mod task_orchestrator;
pub mod kb;
pub mod event_bus;
pub mod memory_layers;
pub mod mcp_client;
pub mod mcp;
pub mod realtime_voice_frontend;
pub mod sherpa_engine;
pub mod tts;
pub mod whisper_engine;
pub mod vault;
pub mod voice_profile;
pub mod voice_ui_cmd;
pub mod web_search;
#[cfg(test)]
pub mod test_support;

pub use engine::{Engine, EngineError, EventSink, NullSink, SendMessageOutput, TtsSpeakRequest};
pub use reminder::{ReminderEvent, ReminderKind};
pub use enums::{Action, Emotion};
pub use netsocial::{NetError, NetsocialManager, NetState};
pub use settings::Settings;
pub use vault::{VaultError, VaultManager, VaultState, VaultStatus};
