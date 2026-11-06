//! MCP（Model Context Protocol）stdio 客户端。
//!
//! 通过子进程 stdin/stdout 以「每行一个 JSON-RPC 2.0 消息」的 MCP 传输与
//! MCP server 通信：`initialize` → `notifications/initialized` →
//! `tools/list` → `tools/call`。
//!
//! - 协议版本：2024-11-05（MCP 首个稳定版本）。
//! - 会话复用：一次 `connect` 建立并持有子进程，多个工具调用共享同一会话，
//!   避免重复 initialize 的开销与状态破坏。
//! - 超时：单次调用默认 10s（与 SkillAdapter 对齐）；超时/断线统一降级为
//!   可读错误文本，由上层转成 LLM 友好回复。
//! - 安全：MCP server 由插件 manifest 显式声明（command + args），启动命令
//!   的 basename 复用 `skill_adapter::is_dangerous_command` 危险判定，命中则
//!   在 invoke 路径转逐次审批，禁止无批准启动任意进程。

use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// MCP 工具声明（tools/list 结果项）。
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 已建立的 MCP 会话：持有子进程与读写管道，可多次调用工具。
#[derive(Debug)]
pub struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// 连接参数（来自 plugin manifest 的 `entry.mcp`）。
#[derive(Debug, Clone)]
pub struct McpSpec {
    /// MCP server 启动命令（可执行文件路径或 basename，按 PATH 解析）。
    pub command: String,
    /// 附加启动参数。
    pub args: Vec<String>,
}

impl McpSession {
    /// 启动子进程并完成 MCP 握手：initialize + notifications/initialized。
    pub async fn connect(
        spec: &McpSpec,
        timeout: Duration,
    ) -> Result<Self, String> {
        let mut child = tokio::process::Command::new(&spec.command)
            .args(&spec.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("MCP server 启动失败: {e}"))?;

        let stdin = child.stdin.take().ok_or("MCP server stdin 不可用")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("MCP server stdout 不可用")?;
        let mut session = McpSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };

        tokio::time::timeout(timeout, async {
            let req = json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "qibai", "version": "0.1.0" }
                }
            });
            session.write_line(&req).await?;
            let resp = session.read_response(0).await?;
            if let Some(err) = resp.get("error") {
                return Err(format!("initialize 失败: {err}"));
            }
            // notifications/initialized（无 id，无需响应）
            let notif = json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            });
            session.write_line(&notif).await?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| format!("MCP initialize 超时（{timeout:?}）"))??;
        Ok(session)
    }

    /// 列出 MCP server 提供的全部工具。
    pub async fn list_tools(&mut self, timeout: Duration) -> Result<Vec<McpTool>, String> {
        tokio::time::timeout(timeout, async {
            let id = self.next_id();
            let req = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list" });
            self.write_line(&req).await?;
            let resp = self.read_response(id).await?;
            if let Some(err) = resp.get("error") {
                return Err(format!("tools/list 失败: {err}"));
            }
            let tools = resp
                .pointer("/result/tools")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            Ok(tools
                .into_iter()
                .filter_map(|t| {
                    let name = t.get("name")?.as_str()?.to_string();
                    let description = t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input_schema = t.get("inputSchema").cloned().unwrap_or_else(|| json!({}));
                    Some(McpTool { name, description, input_schema })
                })
                .collect())
        })
        .await
        .map_err(|_| format!("tools/list 超时（{timeout:?}）"))?
    }

    /// 调用 MCP 工具；返回文本结果（text content 拼接），失败返回可读错误。
    pub async fn call_tool(
        &mut self,
        tool: &str,
        arguments: &Value,
        timeout: Duration,
    ) -> Result<String, String> {
        tokio::time::timeout(timeout, async {
            let id = self.next_id();
            let req = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments }
            });
            self.write_line(&req).await?;
            let resp = self.read_response(id).await?;
            if let Some(err) = resp.get("error") {
                return Err(format!("tools/call 失败: {err}"));
            }
            if let Some(is_err) = resp.pointer("/result/isError").and_then(|v| v.as_bool()) {
                if is_err {
                    let text = extract_text(&resp);
                    return Err(format!("MCP 工具执行失败: {text}"));
                }
            }
            Ok(extract_text(&resp))
        })
        .await
        .map_err(|_| format!("tools/call 超时（{timeout:?}）"))?
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn write_line(&mut self, msg: &Value) -> Result<(), String> {
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("写入 MCP stdin 失败: {e}"))?;
        self.stdin.write_all(b"\n").await.map_err(|e| e.to_string())?;
        self.stdin.flush().await.map_err(|e| e.to_string())
    }

    /// 从 stdout 逐行读取，跳过 server 主动推送的消息（无 id 或 id 不匹配），
    /// 直到读到目标 id 的响应。
    async fn read_response(&mut self, target_id: u64) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(|e| format!("读取 MCP stdout 失败: {e}"))?;
            if n == 0 {
                return Err("MCP server 已退出（stdout 关闭）".to_string());
            }
            let msg: Value = serde_json::from_str(line.trim()).map_err(|e| {
                format!("MCP 响应解析失败: {e}（原始: {}）", line.trim().chars().take(80).collect::<String>())
            })?;
            let msg_id = msg.get("id");
            let is_response = msg_id
                .map(|v| v.as_u64() == Some(target_id))
                .unwrap_or(false);
            if is_response {
                return Ok(msg);
            }
            // 跳过 notification / 其它 id 的响应 / server 请求
        }
    }
}

/// 从 tools/call 结果中提取文本内容（content[] 的 text 字段拼接）。
fn extract_text(resp: &Value) -> String {
    resp.pointer("/result/content")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

impl Drop for McpSession {
    fn drop(&mut self) {
        // kill_on_drop(true) 已在 spawn 时设置；这里显式触发确保清理。
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// 内嵌 mock MCP server（python3，JSON-RPC 换行分隔）：
    /// 实现 initialize / notifications/initialized / tools/list / tools/call。
    const MOCK_MCP_PY: &str = r#"
import json, sys
def send(o):
    sys.stdout.write(json.dumps(o, ensure_ascii=False) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({"jsonrpc":"2.0","id":mid,"result":{
            "protocolVersion":"2024-11-05","capabilities":{"tools":{}},
            "serverInfo":{"name":"mock-mcp","version":"1.0.0"}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc":"2.0","id":mid,"result":{"tools":[
            {"name":"ping","description":"回显测试文本",
             "inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}})
    elif method == "tools/call":
        args = (msg.get("params") or {}).get("arguments") or {}
        text = args.get("text", "")
        send({"jsonrpc":"2.0","id":mid,"result":{"content":[
            {"type":"text","text":"pong:" + text}]}})
    else:
        send({"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"method not found"}})
"#;

    fn write_mock_server(tag: &str) -> std::path::PathBuf {
        // 每测试独立目录，避免并行测试共享临时文件产生清理竞态。
        let dir = std::env::temp_dir().join(format!(
            "qibai_mcp_mock_{}_{}",
            std::process::id(),
            tag
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mock_mcp_server.py");
        std::fs::write(&path, MOCK_MCP_PY).unwrap();
        path
    }

    #[tokio::test]
    async fn mcp_handshake_list_and_call() {
        let py = write_mock_server("handshake");
        let spec = McpSpec {
            command: "python3".to_string(),
            args: vec![py.to_string_lossy().to_string()],
        };
        let mut sess = McpSession::connect(&spec, Duration::from_secs(10))
            .await
            .expect("MCP 握手失败");

        let tools = sess.list_tools(Duration::from_secs(10)).await.expect("tools/list 失败");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");

        let out = sess
            .call_tool("ping", &json!({ "text": "你好" }), Duration::from_secs(10))
            .await
            .expect("tools/call 失败");
        assert_eq!(out, "pong:你好");

        // 会话复用：第二次调用仍在同一进程上工作。
        let out2 = sess
            .call_tool("ping", &json!({ "text": "again" }), Duration::from_secs(10))
            .await
            .expect("第二次 tools/call 失败");
        assert_eq!(out2, "pong:again");
    }

    #[tokio::test]
    async fn mcp_missing_server_degrades() {
        let spec = McpSpec {
            command: "/nonexistent/qibai-mcp-server".to_string(),
            args: vec![],
        };
        let err = McpSession::connect(&spec, Duration::from_secs(5)).await;
        assert!(err.is_err());
        let _ = err.unwrap_err();
    }

    #[test]
    fn mcp_spec_is_clone_and_debug() {
        let spec = McpSpec { command: "x".into(), args: vec![] };
        let spec2 = spec.clone();
        let _ = format!("{spec2:?}");
    }

    #[test]
    fn mock_py_is_valid_python() {
        // 用 python3 编译检查内嵌脚本语法，确保测试进程能启动。
        let py = write_mock_server("pycheck");
        let mut cmd = std::process::Command::new("python3");
        cmd.arg("-m").arg("py_compile").arg(&py);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let status = cmd.status().expect("python3 不可用");
        assert!(status.success());
    }
}
