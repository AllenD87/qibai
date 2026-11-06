//! MCP（Model Context Protocol）接入通道 —— v45.12。
//!
//! 与既有本地技能包（`qibai-plugin.json` + `SKILL.md`，走 `mcp_client.rs` /
//! `skill_adapter.rs`）**完全并行、互不影响**：本模块是新增的第二条接入通道，
//! 不替换、不修改任何技能加载逻辑。
//!
//! - [`client`]   stdio JSON-RPC 2.0 客户端（每行一个完整 JSON，不是 Content-Length 头）
//! - [`registry`] server 生命周期（spawn / 守护 / stderr 捕获 / 崩溃重启上限）
//! - [`bridge`]   工具命名空间 `mcp__<server>__<tool>` 与冲突检测
//!
//! 范围限制（用户已拍板）：只做 stdio；不做 Streamable HTTP、resources、prompts、
//! sampling，也不做在线市场。`transport` 字段仅作占位。

pub mod bridge;
pub mod client;
pub mod registry;

use serde::{Deserialize, Serialize};

/// 单个 MCP server 配置（存于 `Settings::mcp_servers`，默认空数组）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// 唯一标识，也是工具命名空间里的 `<server>` 段。
    pub name: String,
    /// 传输方式。当前只支持 `stdio`；`http` 仅占位，未实现。
    #[serde(default = "default_transport")]
    pub transport: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_trust")]
    pub trust: McpTrust,
}

fn default_transport() -> String {
    "stdio".to_string()
}
fn default_enabled() -> bool {
    true
}
fn default_trust() -> McpTrust {
    McpTrust::Confirm
}

/// 信任档位：`readonly` 自动放行只读工具；`confirm`（默认）一律走 waiting_confirm。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTrust {
    Readonly,
    Confirm,
}

impl Default for McpTrust {
    fn default() -> Self {
        McpTrust::Confirm
    }
}

/// server 的运行状态（`status` 字段直接给 UI，必须真实，不许写死"已连接"）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpState {
    /// 配置里 enabled=false，未启动任何子进程。
    Disabled,
    Starting,
    Ready,
    /// 崩溃重启超限或启动失败，附原因。
    Unavailable(String),
}

impl McpState {
    pub fn as_str(&self) -> &str {
        match self {
            McpState::Disabled => "disabled",
            McpState::Starting => "starting",
            McpState::Ready => "ready",
            McpState::Unavailable(_) => "unavailable",
        }
    }
}

/// `tools/list` 返回的工具（cursor 分页循环取全后的合并结果）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// `tools/call` 的结果。`is_error` 必须原样保留并回灌给模型。
#[derive(Debug, Clone)]
pub struct CallToolResult {
    /// 已拼成可读文本的 content（多段 text 用空行连接）。
    pub content: String,
    pub is_error: bool,
}
