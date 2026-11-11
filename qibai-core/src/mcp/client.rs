//! MCP stdio JSON-RPC 2.0 客户端。
//!
//! 传输约定：**每行一个完整 JSON 对象**（换行分隔），不使用 Content-Length 头。
//! 握手严格顺序：`initialize` → 收到 result → `notifications/initialized` →
//! 允许 `tools/list`。顺序不对的调用必须报错而不是静默发出去。
//!
//! 同步实现：stdout 由后台线程逐行读出，经 mpsc 通道投回主线程；主线程用
//! `recv_timeout` 实现读超时（不引入 async / 不依赖 tokio runtime）。

use crate::mcp::{CallToolResult, McpServerConfig, McpTool};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// MCP 协议版本（客户端声明）。
pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpClient {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    /// 后台 stdout 读线程投回的原始 JSON 行。包 Mutex 是为了让 `McpClient: Sync`
    /// （`ToolExecutor` trait 要求 `Send + Sync`）；所有访问都在 `&mut self` 下。
    pub(crate) lines_rx: Mutex<mpsc::Receiver<String>>,
    /// 已判定为通知（无请求 id）、暂未被取走的消息。
    pub(crate) notifications: Mutex<Vec<Value>>,
    pub(crate) stderr_tail: Arc<Mutex<Vec<String>>>,
    pub(crate) next_id: u64,
    pub(crate) timeout: Duration,
    pub(crate) initialized: bool,
    pub(crate) server: String,
}

impl McpClient {
    /// 启动子进程并建管道。**不**自动握手（调用方显式 `initialize`）。
    pub fn spawn(cfg: &McpServerConfig, timeout: Duration) -> Result<Self, String> {
        let mut cmd = std::process::Command::new(&cfg.command);
        cmd.args(&cfg.args);
        crate::proc_env::apply_std(&mut cmd);
        cmd.envs(cfg.env.iter());
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 MCP server `{}` 失败：{e}", cfg.name))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("无法取得 `{}` 的 stdin", cfg.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("无法取得 `{}` 的 stdout", cfg.name))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("无法取得 `{}` 的 stderr", cfg.name))?;

        // stderr：后台线程持续读，只留尾部 20 行。
        let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let mut g = tail.lock().unwrap();
                    g.push(line);
                    if g.len() > 20 {
                        let overflow = g.len() - 20;
                        g.drain(0..overflow);
                    }
                }
            });
        }

        // stdout：后台线程逐行读出投回；线程退出即 drop(tx) → rx 断开。
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            lines_rx: Mutex::new(rx),
            notifications: Mutex::new(Vec::new()),
            stderr_tail,
            next_id: 0,
            timeout,
            initialized: false,
            server: cfg.name.clone(),
        })
    }

    /// 握手：initialize → 校验 result → notifications/initialized。
    pub fn initialize(&mut self) -> Result<Value, String> {
        if self.initialized {
            return Err(format!("`{}` 已完成 initialize，请勿重复", self.server));
        }
        let id = self.alloc_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "qibai",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        self.write_raw(&req)?;
        let resp = self.read_response(id)?;
        let result = resp
            .get("result")
            .ok_or_else(|| format!("`{}` 的 initialize 响应缺少 result", self.server))?;
        let server_ver = result
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if server_ver != PROTOCOL_VERSION {
            return Err(format!(
                "MCP 协议版本不匹配：客户端声明 {PROTOCOL_VERSION}，server `{}` 握手返回 {server_ver:?}",
                self.server
            ));
        }
        // 通知：不等待响应。
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        self.write_raw(&note)?;
        self.initialized = true;
        Ok(result.clone())
    }

    /// 拉取工具清单，处理 `cursor` 分页循环取全。
    pub fn list_tools(&mut self) -> Result<Vec<McpTool>, String> {
        if !self.initialized {
            return Err(format!("`{}` 未完成 initialize，不能 list_tools", self.server));
        }
        let mut tools = Vec::new();
        let mut cursor: Option<Value> = None;
        let mut pages = 0;
        loop {
            pages += 1;
            if pages > 50 {
                return Err(format!(
                    "`{}` 的 tools/list 分页超过 50 页上限，疑似死循环",
                    self.server
                ));
            }
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let id = self.alloc_id();
            let req = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/list",
                "params": params
            });
            self.write_raw(&req)?;
            let resp = self.read_response(id)?;
            let result = resp
                .get("result")
                .ok_or_else(|| format!("`{}` 的 tools/list 响应缺少 result", self.server))?;
            if let Some(arr) = result.get("tools").and_then(|t| t.as_array()) {
                for t in arr {
                    let name = t
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let description = t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input_schema = t.get("inputSchema").cloned().unwrap_or(Value::Null);
                    tools.push(McpTool {
                        name,
                        description,
                        input_schema,
                    });
                }
            }
            match result.get("nextCursor") {
                None | Some(Value::Null) => break,
                Some(c) if c.is_string() && c.as_str().unwrap().is_empty() => break,
                Some(c) => cursor = Some(c.clone()),
            }
        }
        Ok(tools)
    }

    /// 调用工具，保留 `isError`。
    pub fn call_tool(&mut self, name: &str, args: Value) -> Result<CallToolResult, String> {
        if !self.initialized {
            return Err(format!("`{}` 未完成 initialize，不能 call_tool", self.server));
        }
        let id = self.alloc_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        self.write_raw(&req)?;
        let resp = self.read_response(id)?;
        let result = resp
            .get("result")
            .ok_or_else(|| format!("`{}` 的 tools/call 响应缺少 result", self.server))?;
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut content = String::new();
        if let Some(arr) = result.get("content").and_then(|c| c.as_array()) {
            let texts: Vec<&str> = arr
                .iter()
                .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect();
            content = texts.join("\n\n");
        }
        Ok(CallToolResult { content, is_error })
    }

    /// 非阻塞读取已到达的通知（`notifications/tools/list_changed` 等）。
    pub fn poll_notifications(&mut self) -> Vec<Value> {
        // 先把后台线程通道里尚未入队的非阻塞取完。
        while let Ok(line) = {
            self.lines_rx.lock().unwrap().try_recv()
        } {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("method").is_some() {
                    self.notifications.lock().unwrap().push(v);
                }
            }
        }
        let mut q = self.notifications.lock().unwrap();
        let mut out = Vec::new();
        let mut i = 0;
        while i < q.len() {
            let is_list_changed = q[i].get("method").and_then(|m| m.as_str())
                == Some("notifications/tools/list_changed");
            if is_list_changed {
                out.push(q.remove(i));
            } else {
                i += 1;
            }
        }
        out
    }

    /// 子进程是否已退出（非 0 退出视为崩溃，由 registry 判）。
    pub fn has_exited(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => None,
        }
    }

    /// stderr 尾部若干行（崩溃原因展示）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    // ---------- 内部辅助 ----------

    fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 序列化一行 + '\n' + flush。
    fn write_raw(&mut self, v: &Value) -> Result<(), String> {
        let mut s = serde_json::to_string(v).map_err(|e| format!("序列化 JSON 失败：{e}"))?;
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).map_err(|e| {
            format!("写 server `{}` 管道失败：{e}", self.server)
        })?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush server `{}` 失败：{e}", self.server))?;
        Ok(())
    }

    /// 阻塞等待 id 匹配的响应；超时用自管 deadline（recv_timeout）。
    /// 非 JSON 行跳过；有 method 的消息当通知入队。
    fn read_response(&mut self, expected_id: u64) -> Result<Value, String> {
        loop {
            let recv = self.lines_rx.lock().unwrap().recv_timeout(self.timeout);
            match recv {
                Ok(line) => {
                    let v: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue, // 跳过非 JSON 行
                    };
                    if v.get("id").and_then(|i| i.as_u64()) == Some(expected_id) {
                        if let Some(err) = v.get("error") {
                            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                            let msg = err
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("未知错误");
                            return Err(format!("RPC 错误（code={code}）：{msg}"));
                        }
                        return Ok(v);
                    }
                    // 其他消息：有 method 视为通知，入队；否则视为 stray 响应丢弃。
                    if v.get("method").is_some() {
                        self.notifications.lock().unwrap().push(v);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "等待 server `{}` 响应超时（{} 秒）",
                        self.server,
                        self.timeout.as_secs_f64()
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(code) = self.has_exited() {
                        return Err(format!(
                            "server `{}` 已退出（exit code={code}）",
                            self.server
                        ));
                    }
                    return Err(format!("server `{}` 响应通道已关闭", self.server));
                }
            }
        }
    }
}
