//! 最小可用的 MCP stdio server（仅测试/演示用）。
//!
//! 约定：每行一个完整 JSON-RPC 2.0 对象（换行分隔，不是 Content-Length）。
//!
//! 支持的方法：
//! - `initialize`：回 result；可用 `MCP_ECHO_BAD_VERSION=1` 返回不兼容版本号
//!   （`2025-06-18`）以触发客户端握手失败。
//! - `notifications/initialized`：无 id 的通知，不回包。
//! - `tools/list`：共 5 个工具，每页 2 个，`nextCursor` 分页（cursor 即页码字符串）。
//! - `tools/call`：把 arguments 原文 echo 回 `content[0].text`；工具名含 `fail` 时
//!   返回 `isError:true`，文本前缀 `ECHO_ERROR: `。
//!
//! 环境变量：
//! - `MCP_ECHO_DELAY_MS`：每次回包前 sleep，用于测读超时。
//! - `MCP_ECHO_CRASH_AFTER`：每成功应答 N 条后主动 `exit(1)`（在写完该条响应后），
//!   用于测崩溃与重启。
//! - `MCP_ECHO_BAD_VERSION=1`：握手返回不兼容 protocolVersion。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process;

fn tool_def(name: &str, desc: &str) -> Value {
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "任意回显内容" }
            }
        }
    })
}

fn all_tools() -> Vec<Value> {
    vec![
        tool_def("echo_one", "回显工具 1"),
        tool_def("echo_two", "回显工具 2"),
        tool_def("echo_three", "回显工具 3"),
        tool_def("echo_four", "回显工具 4"),
        tool_def("echo_fail", "故意返回 isError 的回显工具"),
    ]
}

fn main() {
    let delay_ms: u64 = std::env::var("MCP_ECHO_DELAY_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let crash_after: Option<u64> = std::env::var("MCP_ECHO_CRASH_AFTER")
        .ok()
        .and_then(|s| s.parse().ok());
    let bad_version = std::env::var("MCP_ECHO_BAD_VERSION")
        .map(|v| v == "1")
        .unwrap_or(false);

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout();
    let mut answered: u64 = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // 跳过非 JSON 行
        };

        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = req.get("id");
        let has_id = match id {
            Some(v) => !v.is_null(),
            None => false,
        };

        // 通知（无 id）不回包。
        if !has_id {
            continue;
        }
        let id = id.cloned().unwrap_or(Value::Null);

        let result: Value = match method {
            "initialize" => {
                let ver = if bad_version {
                    "2025-06-18"
                } else {
                    "2024-11-05"
                };
                json!({
                    "protocolVersion": ver,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mcp-echo-server", "version": "0.1.0" }
                })
            }
            "tools/list" => {
                let tools = all_tools();
                let cursor = req
                    .get("params")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str());
                let page: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
                const PAGE_SIZE: usize = 2;
                let start = page * PAGE_SIZE;
                if start >= tools.len() {
                    json!({ "tools": [], "nextCursor": Value::Null })
                } else {
                    let end = (start + PAGE_SIZE).min(tools.len());
                    let page_tools: Vec<&Value> = tools[start..end].iter().collect();
                    let next = if end < tools.len() {
                        json!(format!("{}", page + 1))
                    } else {
                        Value::Null
                    };
                    json!({ "tools": page_tools, "nextCursor": next })
                }
            }
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                if name.contains("fail") {
                    json!({
                        "isError": true,
                        "content": [
                            { "type": "text", "text": format!("ECHO_ERROR: {}", args) }
                        ]
                    })
                } else {
                    json!({
                        "isError": false,
                        "content": [
                            { "type": "text", "text": format!("ECHO: {}", args) }
                        ]
                    })
                }
            }
            _ => continue,
        };

        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let mut out = serde_json::to_string(&resp).unwrap_or_default();
        out.push('\n');
        if stdout.write_all(out.as_bytes()).is_err() {
            break;
        }
        if stdout.flush().is_err() {
            break;
        }
        answered += 1;

        if let Some(n) = crash_after {
            if answered >= n {
                eprintln!(
                    "echo: 已应答 {answered} 条，按 MCP_ECHO_CRASH_AFTER={n} 主动 exit(1)"
                );
                process::exit(1);
            }
        }
    }
}
