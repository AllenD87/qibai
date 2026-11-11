//! 真实 animacode MCP 走查：连接 → 列工具 → 调 anima_schema。
//!
//! 这组用例存在的意义：用户报「审批通过了，但执行失败：MCP server 已退出」，
//! 而沙箱里这条链路必须能证明是通的；同时把两种「秒退」场景钉成可读错误。

use serde_json::json;
use std::time::Duration;

const T: Duration = Duration::from_secs(15);

fn animacode_spec() -> qibai_core::mcp_client::McpSpec {
    let dir = std::fs::canonicalize("../examples/plugins/animacode").unwrap();
    qibai_core::mcp_client::McpSpec {
        command: "node".into(),
        args: vec![format!("{}/mcp/server.mjs", dir.display())],
    }
}

#[tokio::test]
async fn real_animacode_connects_lists_tools_and_calls_schema() {
    let mut s = qibai_core::mcp_client::McpSession::connect(&animacode_spec(), T)
        .await
        .expect("connect");
    let tools = s.list_tools(T).await.expect("tools/list");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for want in ["anima_schema", "anima_build", "anima_render", "anima_qa", "anima_mux"] {
        assert!(names.contains(&want), "缺少工具 {want}，实际：{names:?}");
    }
    let text = s.call_tool("anima_schema", &json!({}), T).await.expect("anima_schema");
    assert!(text.contains("scene.json"), "schema 文本异常：{}", &text[..text.len().min(80)]);
}

#[tokio::test]
async fn missing_entry_script_is_reported_before_spawning() {
    let dir = std::fs::canonicalize("../examples/plugins/animacode").unwrap();
    let spec = qibai_core::mcp_client::McpSpec {
        command: "node".into(),
        args: vec![format!("{}/mcp/not-there.mjs", dir.display())],
    };
    let err = qibai_core::mcp_client::McpSession::connect(&spec, T)
        .await
        .expect_err("应当预检失败");
    assert!(err.contains("插件入口脚本不存在"), "错误应说清缺文件：{err}");
    assert!(err.contains("not-there.mjs"), "错误应带路径：{err}");
}

#[tokio::test]
async fn instant_exit_carries_server_stderr() {
    // 模拟「node 起来就崩」：sh 往 stderr 说一句就退出 7。
    let spec = qibai_core::mcp_client::McpSpec {
        command: "sh".into(),
        args: vec!["-c".into(), "echo boom-from-server >&2; exit 7".into()],
    };
    let err = qibai_core::mcp_client::McpSession::connect(&spec, T)
        .await
        .expect_err("应当报启动即退出");
    assert!(err.contains("启动后立即退出"), "应带退出动作：{err}");
    assert!(err.contains("boom-from-server"), "应把 stderr 端上来：{err}");
    assert!(
        qibai_core::mcp_client::is_session_dead(&err),
        "该判为会话死亡，上层才会丢弃重连：{err}"
    );
}
