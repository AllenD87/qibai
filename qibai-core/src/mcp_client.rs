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
use std::path::Path;
use std::sync::{Arc, Mutex};
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
    /// server 往 stderr 说的话（启动 banner / 崩溃栈 / Cannot find module）。
    /// 以前用 `Stdio::inherit()` 直接丢掉了——于是「node 秒退」在用户侧只剩一句
    /// 「stdout 关闭」，看不出是缺文件、版本不对还是语法错。现在随错误一起抛出。
    stderr_tail: Arc<Mutex<String>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

/// 会话是否已经死了（用于上层判「该丢弃重连」而不是把错误原样抛给用户）。
pub fn is_session_dead(msg: &str) -> bool {
    msg.contains("stdout 关闭")
        || msg.contains("写入 MCP stdin 失败")
        || msg.contains("启动后立即退出")
        || msg.contains("MCP server 已退出")
}

/// 连接参数（来自 plugin manifest 的 `entry.mcp`）。
#[derive(Debug, Clone)]
pub struct McpSpec {
    /// MCP server 启动命令（可执行文件路径或 basename，按 PATH 解析）。
    pub command: String,
    /// 附加启动参数。
    pub args: Vec<String>,
}

/// 入口脚本参数的跨平台规范化。
///
/// 只处理「绝对路径 + .mjs/.js/.cjs」这一类：把 Windows 的 verbatim 前缀
/// `\\?\` 去掉、把 `/` 统一成 `\`。其它参数原样返回。
fn normalize_script_arg(a: &str) -> String {
    let p = Path::new(a);
    let is_script = p.is_absolute()
        && matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("mjs") | Some("js") | Some("cjs")
        );
    if !is_script {
        return a.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(rest) = a.strip_prefix("\\\\?\\") {
            return rest.replace('/', "\\");
        }
        if a.contains('/') {
            return a.replace('/', "\\");
        }
    }
    a.to_string()
}

impl McpSession {
    /// 启动子进程并完成 MCP 握手：initialize + notifications/initialized。
    pub async fn connect(
        spec: &McpSpec,
        timeout: Duration,
    ) -> Result<Self, String> {
        // 入口路径先规范化再校验/启动。Windows 上 `canonicalize` 会给出
        // `\\?\D:\…` 这种 verbatim 前缀，而 verbatim 路径里 `/` 不算分隔符，
        // `is_file()` 和 node 都会认定文件不存在（CI 的 windows job 就栽在这），
        // 于是明明装好的插件被误报成「入口脚本不存在」。
        let args: Vec<String> = spec.args.iter().map(|a| normalize_script_arg(a)).collect();
        // 占位符没展开时，绝不能让 node 拿错误路径去猜：那时它只会报
        // `Cannot find module '<cwd>/{PLUGIN_DIR}/mcp/server.mjs'`，看着像插件没装，
        // 实际是清单没按安装目录解析。这里直接把原因说清。
        if let Some(ph) = args
            .iter()
            .find(|a| a.contains("{PLUGIN_DIR}") || a.contains("{QIBAI_HOME}"))
        {
            return Err(format!(
                "插件启动参数里的路径占位符没有展开：{ph}\n（说明这份清单没有按插件实际安装目录解析）请在控台「插件」页删除该插件后重新导入技能包；自己手配的 MCP 服务器请直接填绝对路径。"
            ));
        }
        // 入口预检：清单里的 `{PLUGIN_DIR}` 已替换成绝对路径，这里若文件不在，
        // node 只会秒退并往 stderr 抱怨一句，用户却看到「stdout 关闭」。
        // 直接说清「插件装得不完整」，比让用户去猜 node 环境省事得多。
        if let Some(missing) = args.iter().find(|a| {
            let p = Path::new(a);
            p.is_absolute()
                && matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("mjs") | Some("js") | Some("cjs")
                )
                && !p.is_file()
        }) {
            return Err(format!(
                "插件入口脚本不存在：{missing}\n（插件包装得不完整：常见于旧版导入只落了清单、没落 mcp/ 目录）请在控台「插件」页删除 animacode 后重新导入 animacode-v2.2.0.qbpk。"
            ));
        }
        let mut cmd = tokio::process::Command::new(&spec.command);
        cmd.args(&args);
        // 桌面应用继承的 PATH 往往不含 Homebrew / nvm / Program Files\nodejs，
        // 直接 spawn 会 ENOENT（用户只看到「审批失败」）。统一补全 PATH。
        crate::proc_env::apply_tokio(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                if crate::proc_env::looks_like_missing_binary(&e.to_string()) {
                    format!(
                        "MCP server 启动失败: {e}\n{}",
                        crate::proc_env::missing_binary_hint(&spec.command)
                    )
                } else {
                    format!("MCP server 启动失败: {e}")
                }
            })?;

        let stdin = child.stdin.take().ok_or("MCP server stdin 不可用")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("MCP server stdout 不可用")?;
        // 持续收集 stderr（只留最后约 1KB，防长日志吃内存）。
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let stderr_task = child.stderr.take().map(|err| {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(err);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let mut buf = tail.lock().unwrap();
                            buf.push_str(line.trim_end());
                            buf.push('\n');
                            if buf.len() > 1200 {
                                let cut = buf.len().saturating_sub(1000);
                                let at = buf
                                    .char_indices()
                                    .find(|(i, _)| *i >= cut)
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                *buf = buf[at..].to_string();
                            }
                        }
                    }
                }
            })
        });
        let mut session = McpSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            stderr_tail,
            stderr_task,
        };

        // 启动哨兵：240ms 观察窗口内若 server 已退出，直接把退出码 + stderr 端上来。
        // 正常 node 冷启动约 100–200ms，未退出的继续握手（initialize 仍有超时兜底）。
        let mut exited: Option<std::process::ExitStatus> = None;
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(40)).await;
            match session.child.try_wait() {
                Ok(Some(st)) => {
                    exited = Some(st);
                    break;
                }
                _ => continue,
            }
        }
        if let Some(st) = exited {
            // server 退得比 stderr 读完还快时，给收集任务几十毫秒把最后几行落进缓冲。
            for _ in 0..6 {
                let has = session
                    .stderr_tail
                    .lock()
                    .map(|g| !g.trim().is_empty())
                    .unwrap_or(false);
                if has {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            let detail = session.stderr_suffix();
            let hint = if detail.is_empty() {
                "（server 没有任何 stderr 输出）".to_string()
            } else {
                detail
            };
            return Err(format!("MCP server 启动后立即退出（{st}）{hint}"));
        }

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

    /// stderr 尾部的可读片段（空则返回空串，不留多余换行）。
    fn stderr_suffix(&self) -> String {
        let buf = self.stderr_tail.lock().map(|g| g.clone()).unwrap_or_default();
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!("；server stderr:\n{trimmed}")
        }
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
            .map_err(|e| format!("写入 MCP stdin 失败: {e}{}", self.stderr_suffix()))?;
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
                let detail = self.stderr_suffix();
                // server 在 240ms 哨兵窗口外才被判定退出时（机器负载高、fork 慢），
                // 别只报「stdout 关闭」——进程已经没了，退出码才是能搜的原因。
                return Err(match self.child.try_wait() {
                    Ok(Some(st)) => format!("MCP server 启动后立即退出（{st}）{detail}"),
                    _ => format!("MCP server 已退出（stdout 关闭）{detail}"),
                });
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
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// 占位符没展开时必须给可读错误：否则 node 会把整段当相对路径，
    /// 报 `Cannot find module '<cwd>/{PLUGIN_DIR}/mcp/server.mjs'`，看着像插件没装。
    #[tokio::test]
    async fn unexpanded_placeholder_is_reported_before_spawning() {
        let spec = McpSpec {
            command: "node".to_string(),
            args: vec!["{PLUGIN_DIR}/mcp/server.mjs".to_string()],
        };
        let err = McpSession::connect(&spec, Duration::from_secs(2))
            .await
            .expect_err("必须拒绝未展开的占位符");
        assert!(err.contains("占位符没有展开"), "错误文本不含原因：{err}");
        assert!(err.contains("{PLUGIN_DIR}"), "错误文本应带上原参数：{err}");
    }

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
