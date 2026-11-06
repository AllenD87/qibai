//! MCP stdio 传输层 + 生命周期集成测试。
//!
//! 每条用例都 `spawn` 一个独立的真 echo server 进程（`src/bin/mcp_echo_server.rs`），
//! 用完即 kill / 等其退出，互不污染。

use qibai_core::mcp::client::{McpClient, PROTOCOL_VERSION};
use qibai_core::mcp::registry::McpRegistry;
use qibai_core::mcp::{McpServerConfig, McpTrust};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// 由 cargo 在编译测试时注入的 echo server 可执行文件路径。
const ECHO: &str = env!("CARGO_BIN_EXE_mcp_echo_server");

fn make_cfg(name: &str, env: BTreeMap<String, String>) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: "stdio".to_string(),
        command: ECHO.to_string(),
        args: vec![],
        env,
        enabled: true,
        trust: McpTrust::Confirm,
    }
}

fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn with_env(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
    kv.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// 1. 握手成功
#[test]
fn t01_handshake_success() {
    let mut client =
        McpClient::spawn(&make_cfg("echo", empty_env()), Duration::from_secs(5)).unwrap();
    let result: Value = client.initialize().unwrap();
    assert_eq!(result["protocolVersion"], json!("2024-11-05"));
    client.kill();
}

// 2. 版本不匹配明确失败（含双方版本号）
#[test]
fn t02_version_mismatch_explicit_error() {
    let mut client = McpClient::spawn(
        &make_cfg("echo", with_env(&[("MCP_ECHO_BAD_VERSION", "1")])),
        Duration::from_secs(5),
    )
    .unwrap();
    let err = client.initialize().unwrap_err();
    assert!(
        err.contains(PROTOCOL_VERSION),
        "错误应带客户端版本 {PROTOCOL_VERSION}：{err}"
    );
    assert!(
        err.contains("2025-06-18"),
        "错误应带 server 版本 2025-06-18：{err}"
    );
    client.kill();
}

// 3. tools/list 分页取全 5 个工具
#[test]
fn t03_list_tools_paginates_all_five() {
    let mut client =
        McpClient::spawn(&make_cfg("echo", empty_env()), Duration::from_secs(5)).unwrap();
    client.initialize().unwrap();
    let tools = client.list_tools().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        tools.len(),
        5,
        "应跨 3 页取全 5 个工具，实际：{}",
        names.join(",")
    );
    assert!(names.contains(&"echo_one"));
    assert!(names.contains(&"echo_two"));
    assert!(names.contains(&"echo_three"));
    assert!(names.contains(&"echo_four"));
    assert!(names.contains(&"echo_fail"));
    client.kill();
}

// 4. tools/call 成功
#[test]
fn t04_call_tool_success() {
    let mut client =
        McpClient::spawn(&make_cfg("echo", empty_env()), Duration::from_secs(5)).unwrap();
    client.initialize().unwrap();
    client.list_tools().unwrap();
    let r = client
        .call_tool("echo_one", json!({"message": "hello-mcp"}))
        .unwrap();
    assert!(!r.is_error);
    assert!(r.content.contains("ECHO:"), "content={}", r.content);
    assert!(r.content.contains("hello-mcp"), "content={}", r.content);
    client.kill();
}

// 5. tools/call isError=true，错误文本可取到
#[test]
fn t05_call_tool_is_error_propagated() {
    let mut client =
        McpClient::spawn(&make_cfg("echo", empty_env()), Duration::from_secs(5)).unwrap();
    client.initialize().unwrap();
    client.list_tools().unwrap();
    let r = client
        .call_tool("echo_fail", json!({"message": "boom"}))
        .unwrap();
    assert!(r.is_error, "echo_fail 必须 isError=true");
    assert!(
        r.content.contains("ECHO_ERROR:"),
        "错误文本要能回灌：{}",
        r.content
    );
    client.kill();
}

// 6. 读超时（server 延迟 > 客户端 timeout）
#[test]
fn t06_read_timeout_fires() {
    let mut client = McpClient::spawn(
        &make_cfg("echo", with_env(&[("MCP_ECHO_DELAY_MS", "2000")])),
        Duration::from_millis(500),
    )
    .unwrap();
    let err = client.initialize().unwrap_err();
    assert!(err.contains("超时"), "应报超时，实际：{err}");
    client.kill();
}

// 7. 崩溃后自动重启并成功重试
//
// CRASH_AFTER=5：boot 用掉 4 条响应（initialize + 3 页 list），第 5 条响应（首次
// call）回完后进程 exit(1)。第二次 call 时 client 发现进程已死 → registry 重启
// （restarts=1）→ 重新 boot → 重试成功。
#[test]
fn t07_crash_then_restart_succeeds() {
    let mut reg = McpRegistry::from_configs(&[make_cfg(
        "echo",
        with_env(&[("MCP_ECHO_CRASH_AFTER", "5")]),
    )]);
    {
        let st = reg.status();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0]["state"], json!("ready"), "初始应 Ready：{}", st[0]);
        assert_eq!(st[0]["tools_count"], json!(5));
    }

    // 第一次调用：成功，但 server 回完即自杀。
    let r1 = reg.call("echo", "echo_one", json!({"message": "a"})).unwrap();
    assert!(!r1.is_error);

    // 第二次调用：检测到崩溃 → 重启一次 → 重试成功。
    let r2 = reg.call("echo", "echo_one", json!({"message": "b"})).unwrap();
    assert!(!r2.is_error, "重启后重试应成功");
    assert!(r2.content.contains("ECHO:"), "content={}", r2.content);

    let st = reg.status();
    assert_eq!(st[0]["state"], json!("ready"));
    assert_eq!(
        st[0]["restarts"],
        json!(1),
        "应只重启 1 次：{}",
        st[0]
    );
    // 收尾：可能仍有残留进程。
    if let Some(h) = reg.servers.get_mut("echo") {
        if let Some(c) = h.client.as_mut() {
            c.kill();
        }
    }
}

// 8. 重启超 3 次后标 Unavailable
#[test]
fn t08_restart_limit_marks_unavailable() {
    let mut reg = McpRegistry::from_configs(&[make_cfg(
        "echo",
        with_env(&[("MCP_ECHO_CRASH_AFTER", "5")]),
    )]);

    let mut marked_unavailable = false;
    for i in 0..8 {
        match reg.call("echo", "echo_one", json!({"i": i})) {
            Ok(_) => {}
            Err(e) if e.contains("重启超限") => {
                marked_unavailable = true;
                break;
            }
            Err(_) => {}
        }
        let st = reg.status();
        if st[0]["state"] == json!("unavailable") {
            marked_unavailable = true;
            break;
        }
    }

    assert!(
        marked_unavailable,
        "重启超过 3 次后必须标 Unavailable"
    );
    let st = reg.status();
    assert_eq!(st[0]["state"], json!("unavailable"));
    let detail = st[0]["state_detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("重启超限"),
        "state_detail 应含重启超限原因：{detail}"
    );
}
