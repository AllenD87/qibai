//! MCP server 生命周期管理：spawn / 守护 / 崩溃重启（上限 3 次）/ 停用。

use crate::mcp::bridge::namespaced;
use crate::mcp::client::McpClient;
use crate::mcp::{CallToolResult, McpServerConfig, McpState, McpTool};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// 崩溃重启上限（超过即标 `Unavailable`）。
pub const MAX_RESTARTS: u32 = 3;
/// 单次调用超时默认值。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ServerHandle {
    pub cfg: McpServerConfig,
    pub state: McpState,
    pub tools: Vec<McpTool>,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub client: Option<McpClient>,
}

pub struct McpRegistry {
    pub servers: HashMap<String, ServerHandle>,
    pub timeout: Duration,
}

impl McpRegistry {
    /// 空注册表：**不启动任何子进程**（默认零改动）。
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// 由设置构造：enabled=false 的只登记状态为 Disabled，不 spawn。
    pub fn from_configs(cfgs: &[McpServerConfig]) -> Self {
        let mut reg = Self::new();
        for cfg in cfgs {
            let name = cfg.name.clone();
            let mut handle = ServerHandle {
                cfg: cfg.clone(),
                state: McpState::Starting,
                tools: Vec::new(),
                restarts: 0,
                last_error: None,
                client: None,
            };
            if !cfg.enabled {
                handle.state = McpState::Disabled;
                reg.servers.insert(name, handle);
                continue;
            }
            match Self::boot(cfg, reg.timeout) {
                Ok((client, tools)) => {
                    handle.client = Some(client);
                    handle.tools = tools;
                    handle.state = McpState::Ready;
                }
                Err(e) => {
                    handle.state = McpState::Unavailable(e.clone());
                    handle.last_error = Some(e);
                }
            }
            reg.servers.insert(name, handle);
        }
        reg
    }

    /// UI 用的状态快照（name/state/tools 数/错误原因/trust）。
    pub fn status(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .servers
            .values()
            .map(|h| {
                let tools: Vec<Value> = h
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                        })
                    })
                    .collect();
                json!({
                    "name": h.cfg.name,
                    "transport": h.cfg.transport,
                    "state": h.state.as_str(),
                    "state_detail": match &h.state {
                        McpState::Unavailable(r) => Some(r.clone()),
                        _ => None,
                    },
                    "trust": h.cfg.trust,
                    "tools_count": h.tools.len(),
                    "tools": tools,
                    "restarts": h.restarts,
                    "last_error": h.last_error,
                })
            })
            .collect();
        out.sort_by_key(|v| v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string());
        out
    }

    /// 所有 ready server 的工具名（已带 `mcp__<server>__<tool>` 命名空间）。
    pub fn tool_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, h) in &self.servers {
            if h.state == McpState::Ready {
                for t in &h.tools {
                    out.push(namespaced(name, &t.name));
                }
            }
        }
        out.sort();
        out
    }

    /// 调用某个 server 的工具；崩溃则按上限重启后重试一次。
    pub fn call(&mut self, server: &str, tool: &str, args: Value) -> Result<CallToolResult, String> {
        let handle = self
            .servers
            .get_mut(server)
            .ok_or_else(|| format!("未注册的 MCP server：{server}"))?;
        match handle.state {
            McpState::Disabled => {
                return Err(format!("MCP server `{server}` 已被禁用"));
            }
            McpState::Starting => {
                return Err(format!("MCP server `{server}` 正在启动"));
            }
            McpState::Unavailable(ref e) => {
                return Err(format!("MCP server `{server}` 不可用：{e}"));
            }
            McpState::Ready => {}
        }
        if !handle.tools.iter().any(|t| t.name == tool) {
            let names: Vec<&str> = handle.tools.iter().map(|t| t.name.as_str()).collect();
            return Err(format!(
                "MCP server `{server}` 没有工具 `{tool}`，可用工具：{}",
                if names.is_empty() {
                    "(空)".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }

        // 首次尝试。
        let attempt = handle
            .client
            .as_mut()
            .expect("Ready 状态必带 client")
            .call_tool(tool, args.clone());
        match attempt {
            Ok(r) => Ok(r),
            Err(e) => {
                let died = handle.client.as_mut().and_then(|c| c.has_exited());
                // 写管道失败 / 响应通道关闭 = 子进程正处于退出竞态窗口
                // （try_wait 可能还没 reap），同样按崩溃处理。
                let pipe_death = e.contains("管道失败")
                    || e.contains("flush")
                    || e.contains("通道已关闭")
                    || e.contains("已退出");
                match died {
                    Some(code) => self.recover_after_crash(server, tool, args, code, &e),
                    None if pipe_death => {
                        // 给内核一点点时间 reap，再确认一次。
                        std::thread::sleep(std::time::Duration::from_millis(30));
                        let code = handle
                            .client
                            .as_mut()
                            .and_then(|c| c.has_exited())
                            .unwrap_or(-1);
                        self.recover_after_crash(server, tool, args, code, &e)
                    }
                    None => Err(e), // 超时 / RPC 错误，进程还活着
                }
            }
        }
    }

    pub fn restart(&mut self, server: &str) -> Result<(), String> {
        let handle = self
            .servers
            .get_mut(server)
            .ok_or_else(|| format!("未注册的 MCP server：{server}"))?;
        if let Some(mut c) = handle.client.take() {
            c.kill();
        }
        handle.restarts = 0;
        handle.tools.clear();
        handle.state = McpState::Starting;
        let cfg = handle.cfg.clone();
        match Self::boot(&cfg, self.timeout) {
            Ok((client, tools)) => {
                handle.client = Some(client);
                handle.tools = tools;
                handle.state = McpState::Ready;
                handle.last_error = None;
                Ok(())
            }
            Err(e) => {
                handle.state = McpState::Unavailable(e.clone());
                handle.last_error = Some(e);
                Err(format!("重启 server `{server}` 失败"))
            }
        }
    }

    pub fn set_enabled(&mut self, server: &str, enabled: bool) -> Result<(), String> {
        let handle = self
            .servers
            .get_mut(server)
            .ok_or_else(|| format!("未注册的 MCP server：{server}"))?;
        handle.cfg.enabled = enabled;
        if !enabled {
            if let Some(mut c) = handle.client.take() {
                c.kill();
            }
            handle.tools.clear();
            handle.state = McpState::Disabled;
            handle.last_error = None;
            return Ok(());
        }
        // 重新启用：先清掉旧进程再 boot。
        if let Some(mut c) = handle.client.take() {
            c.kill();
        }
        handle.restarts = 0;
        handle.tools.clear();
        handle.state = McpState::Starting;
        let cfg = handle.cfg.clone();
        match Self::boot(&cfg, self.timeout) {
            Ok((client, tools)) => {
                handle.client = Some(client);
                handle.tools = tools;
                handle.state = McpState::Ready;
                handle.last_error = None;
                Ok(())
            }
            Err(e) => {
                handle.state = McpState::Unavailable(e.clone());
                handle.last_error = Some(e);
                Err(format!("启用 server `{server}` 失败"))
            }
        }
    }

    pub fn configs(&self) -> Vec<McpServerConfig> {
        self.servers.values().map(|h| h.cfg.clone()).collect()
    }

    // ---------- 内部 ----------

    /// spawn + initialize + list_tools；任一步失败都 kill 并返回原因。
    fn boot(cfg: &McpServerConfig, timeout: Duration) -> Result<(McpClient, Vec<McpTool>), String> {
        let mut client = McpClient::spawn(cfg, timeout)?;
        if let Err(e) = client.initialize() {
            client.kill();
            return Err(format!("initialize 失败：{e}"));
        }
        match client.list_tools() {
            Ok(tools) => Ok((client, tools)),
            Err(e) => {
                client.kill();
                Err(format!("list_tools 失败：{e}"))
            }
        }
    }

    /// 调用失败且检测到子进程退出：kill 旧 client，按上限重启后重试一次。
    fn recover_after_crash(
        &mut self,
        server: &str,
        tool: &str,
        args: Value,
        code: i32,
        orig_err: &str,
    ) -> Result<CallToolResult, String> {
        let handle = self
            .servers
            .get_mut(server)
            .expect("server 已在 call 前校验存在");
        let tail = handle
            .client
            .as_ref()
            .map(|c| c.stderr_tail())
            .unwrap_or_default();
        let tail_line = tail
            .last()
            .cloned()
            .unwrap_or_else(|| format!("exit code={code}"));
        if let Some(mut c) = handle.client.take() {
            c.kill();
        }
        handle.restarts += 1;

        if handle.restarts > MAX_RESTARTS {
            let reason = format!("重启超限 {MAX_RESTARTS} 次：{tail_line}");
            handle.state = McpState::Unavailable(reason.clone());
            handle.last_error = Some(format!("原调用错误：{orig_err}；{reason}"));
            return Err(format!("MCP server `{server}` 崩溃且{reason}"));
        }

        let cfg = handle.cfg.clone();
        match Self::boot(&cfg, self.timeout) {
            Ok((client, tools)) => {
                handle.client = Some(client);
                handle.tools = tools;
                handle.state = McpState::Ready;
                handle.last_error = None;
                handle
                    .client
                    .as_mut()
                    .expect("刚 boot 成功")
                    .call_tool(tool, args)
            }
            Err(e) => {
                handle.last_error = Some(e.clone());
                handle.state = McpState::Unavailable(e);
                Err(format!("MCP server `{server}` 崩溃后重启失败"))
            }
        }
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}
