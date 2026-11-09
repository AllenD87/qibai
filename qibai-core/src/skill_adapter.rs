//! 外部插件执行体（T4，契约 §3.3）。
//!
//! - **Command**：`Command::new(entry.command)`，入参 args 序列化为单行 JSON 经 stdin 传入；10s 超时。
//! - **Http**：POST `entry.http`，body = `{ "args": args }`；10s 超时；响应文本作为结果。
//! - 超时/失败 → `SkillOutcome::Denied("技能执行失败（已降级）：{原因}")`（LLM 友好告知）。
//! - Connector 类型 → 预留拒绝。
//! - 危险命令判定（T8）：命中逐次批准，由 registry 在 invoke 路径转 pending。

use crate::plugin_manifest::{EntryKind, Manifest, PluginType};
use crate::skills::SkillOutcome;
use std::path::Path;
use std::time::Duration;
#[cfg(test)]
use std::path::PathBuf;

/// command-kind 外部技能逐次批准的危险命令 basename 集合（契约 §3.3）。
/// 问题7隐私路由：脚本解释器（bash/sh/python/node）能执行任意代码，一并纳入逐次批准，
/// 否则"经 bash -c 跑任意命令"会绕过危险命令黑名单。
const DANGEROUS_BASENAMES: &[&str] = &[
    "rm", "dd", "shutdown", "poweroff", "reboot", "halt", "mv", "chmod", "chown", "kill", "pkill",
    "systemctl", "docker", "curl",
    // 脚本解释器：可注入任意命令，视同危险操作需批准。
    "bash", "sh", "zsh", "python", "python3", "node",
];

/// command-kind 外部技能危险判定：basename 命中危险表（含 `mkfs.*` 前缀）或 args 含 `-rf`。
pub fn is_dangerous_command(command: &str, args: &serde_json::Value) -> bool {
    let base = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    if base.strip_prefix("mkfs.").is_some() {
        return true;
    }
    if DANGEROUS_BASENAMES.contains(&base) {
        return true;
    }
    // args（JSON 文本）含 `-rf` 即视作危险参数组合。
    args.to_string().contains("-rf")
}

/// 外部插件执行体；内置包不经过这里。
pub struct SkillAdapter {
    timeout: Duration,
    /// 已建立的 MCP 会话缓存（plugin_id → session），跨调用复用。
    /// 用 tokio Mutex 保证锁守护跨 await 时 future 仍 Send（engine 的 invoke 要求 Send）。
    mcp_sessions:
        tokio::sync::Mutex<std::collections::BTreeMap<String, crate::mcp_client::McpSession>>,
}

impl Default for SkillAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillAdapter {
    /// 默认 10s 超时（契约 §3.3）。
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            mcp_sessions: tokio::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// 测试注入的更短超时。
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            mcp_sessions: tokio::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// 按 manifest.entry.kind 分派执行；任何失败统一降级为 Denied。
    pub async fn execute(
        &self,
        plugin_dir: &Path,
        manifest: &Manifest,
        fn_name: &str,
        args: &serde_json::Value,
    ) -> SkillOutcome {
        let Some(entry) = &manifest.entry else {
            return self.degrade("清单缺少 entry 声明");
        };
        // 连接器：mcp 与 http 后端开放；command 后端连接器暂不开放（用户配置的连接器
        // 只允许 MCP stdio 或 HTTP POST，避免用户级命令型连接器绕过外部插件审批红线）。
        if manifest.plugin_type == PluginType::Connector && entry.kind == EntryKind::Command {
            return SkillOutcome::Denied(
                "连接器不支持 command 后端，仅支持 mcp（MCP stdio 服务器）或 http（HTTP 端点）".to_string(),
            );
        }
        let result: Result<String, String> = match entry.kind {
            EntryKind::Command => match entry.command {
                Some(ref cmd) => self.run_command(&plugin_dir.join(cmd), args).await,
                None => Err("kind=command 缺少 command 路径".to_string()),
            },
            EntryKind::Http => match entry.http {
                Some(ref url) => self
                    .run_http(url, &entry.http_headers, args)
                    .await,
                None => Err("kind=http 缺少 http 端点".to_string()),
            },
            EntryKind::Mcp => match entry.mcp {
                Some(ref spec) => {
                    let mcp_spec = crate::mcp_client::McpSpec {
                        command: spec.command.clone(),
                        args: spec.args.clone(),
                    };
                    self.run_mcp(&manifest.id, &mcp_spec, fn_name, args).await
                }
                None => Err("kind=mcp 缺少 mcp 声明".to_string()),
            },
            EntryKind::Builtin => Err("内置包不走 adapter".to_string()),
        };
        match result {
            Ok(text) => SkillOutcome::Ok(text),
            Err(e) => self.degrade(&e),
        }
    }

    /// MCP 工具调用：按 plugin_id 复用会话，首次连接并握手 + 校验 tools/list。
    /// 连接器泛型模式：入口函数 `fn_<id>` 的调用须携带 `__tool`（实际 MCP 工具名）
    /// 与可选 `arguments`（工具入参）；普通插件保持 fn_name == 工具名。
    async fn run_mcp(
        &self,
        plugin_id: &str,
        spec: &crate::mcp_client::McpSpec,
        fn_name: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let (tool, call_args): (String, serde_json::Value) =
            if let Some(t) = args.get("__tool").and_then(|v| v.as_str()) {
                let t = t.trim().to_string();
                if t.is_empty() {
                    return Err("MCP 调用缺少 __tool（请传入实际 MCP 工具名）".to_string());
                }
                let arguments = args
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                (t, arguments)
            } else {
                (fn_name.to_string(), args.clone())
            };
        if tool.trim().is_empty() {
            return Err("MCP 调用缺少工具名（functions[].name）".to_string());
        }
        let mut sessions = self.mcp_sessions.lock().await;
        if !sessions.contains_key(plugin_id) {
            let mut sess = crate::mcp_client::McpSession::connect(spec, self.timeout).await?;
            let tools = sess.list_tools(self.timeout).await?;
            if !tools.iter().any(|t| t.name == tool) {
                return Err(format!(
                    "MCP server 未暴露工具 {tool:?}（可用：{}）",
                    tools
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            sessions.insert(plugin_id.to_string(), sess);
        }
        let sess = sessions
            .get_mut(plugin_id)
            .ok_or_else(|| "MCP 会话建立失败".to_string())?;
        sess.call_tool(&tool, &call_args, self.timeout).await
    }

    /// 降级路径：回填 LLM 友好告知（契约 §3.3）。
    pub fn degrade(&self, reason: &str) -> SkillOutcome {
        SkillOutcome::Denied(format!("技能执行失败（已降级）：{reason}"))
    }

    async fn run_command(&self, script: &Path, args: &serde_json::Value) -> Result<String, String> {
        let body = serde_json::to_string(args).map_err(|e| format!("args 序列化失败: {e}"))?;
        let mut cmd = tokio::process::Command::new(script);
        crate::proc_env::apply_tokio(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("启动脚本失败 {script:?}: {e}"))?;
        // 入参约定：单行 JSON 经 stdin 传入。
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(body.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
        }
        let out = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(r) => r.map_err(|e| format!("等待脚本输出失败: {e}"))?,
            Err(_) => {
                return Err(format!(
                    "执行超时（{}秒）",
                    self.timeout.as_secs().max(1)
                ))
            }
        };
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "脚本退出码 {}：{}",
                out.status.code().unwrap_or(-1),
                stderr.trim()
            ));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(stdout.trim().to_string())
    }

    async fn run_http(
        &self,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| format!("http 客户端初始化失败: {e}"))?;
        let mut req = client.post(url).json(&serde_json::json!({ "args": args }));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, text.trim()));
        }
        Ok(text.trim().to_string())
    }
}

/// 递归复制目录（安装时把源包复制进 plugins/<id>/）。
pub fn copy_dir_all(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_all(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[tokio::test]
    async fn command_plugin_receives_stdin_json_and_returns_stdout() {
        let dir = unique_temp_dir("adapter-cmd");
        write_script(&dir, "now.sh", "#!/usr/bin/env bash\ncat\ndate +%Y-%m-%d\n");
        let adapter = SkillAdapter::new();
        let manifest = serde_json::from_value(serde_json::json!({
            "id": "com.test.cmd", "type": "skill", "name": "t", "version": "1.0.0",
            "entry": {"kind": "command", "command": "now.sh"},
            "functions": [{"name": "f", "parameters": {"type": "object"}}],
        }))
        .unwrap();
        match adapter.execute(&dir, &manifest, "do_demo", &serde_json::json!({"x":1})).await {
            SkillOutcome::Ok(out) => assert!(out.contains('-')),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn failure_degrades_to_llm_friendly_denied() {
        let dir = unique_temp_dir("adapter-fail");
        write_script(&dir, "bad.sh", "#!/usr/bin/env bash\necho boom 1>&2\nexit 3\n");
        let adapter = SkillAdapter::new();
        let manifest = serde_json::from_value(serde_json::json!({
            "id": "com.test.bad", "type": "skill", "name": "t", "version": "1.0.0",
            "entry": {"kind": "command", "command": "bad.sh"},
            "functions": [{"name": "f"}],
        }))
        .unwrap();
        match adapter.execute(&dir, &manifest, "do_demo", &serde_json::json!({})).await {
            SkillOutcome::Denied(msg) => {
                assert!(msg.contains("已降级"));
                assert!(msg.contains("boom"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_degrades_instead_of_hanging() {
        let dir = unique_temp_dir("adapter-timeout");
        write_script(&dir, "slow.sh", "#!/usr/bin/env bash\nsleep 5\n");
        // 200ms 超时（测试注入），脚本要睡 5s → 必然超时降级。
        let adapter = SkillAdapter::with_timeout(Duration::from_millis(200));
        let manifest = serde_json::from_value(serde_json::json!({
            "id": "com.test.slow", "type": "skill", "name": "t", "version": "1.0.0",
            "entry": {"kind": "command", "command": "slow.sh"},
            "functions": [{"name": "f"}],
        }))
        .unwrap();
        match adapter.execute(&dir, &manifest, "do_demo", &serde_json::json!({})).await {
            SkillOutcome::Denied(msg) => assert!(msg.contains("超时")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn connector_type_command_backend_rejected() {
        let adapter = SkillAdapter::new();
        let manifest = serde_json::from_value(serde_json::json!({
            "id": "com.test.conn", "type": "connector", "name": "c", "version": "1.0.0",
            "entry": {"kind": "command", "command": "run.sh"},
            "functions": [{"name": "f"}],
        }))
        .unwrap();
        match adapter.execute(Path::new("/tmp"), &manifest, "do_demo", &serde_json::json!({})).await {
            SkillOutcome::Denied(msg) => assert!(msg.contains("仅支持 mcp（MCP stdio 服务器）或 http（HTTP 端点）")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dangerous_command_detection() {
        assert!(is_dangerous_command("rm", &serde_json::json!({})));
        assert!(is_dangerous_command("/usr/bin/docker", &serde_json::json!({})));
        assert!(is_dangerous_command("mkfs.ext4", &serde_json::json!({})));
        assert!(is_dangerous_command("scripts/now.sh", &serde_json::json!({"argv":["x","-rf"]})));
        assert!(!is_dangerous_command("scripts/now.sh", &serde_json::json!({})));
        // 问题7隐私路由：脚本解释器（python3/bash）可执行任意代码，纳入危险命令需逐次批准。
        assert!(is_dangerous_command("/usr/bin/python3", &serde_json::json!({"a":"ls -la"})));
        assert!(is_dangerous_command("/bin/bash", &serde_json::json!({})));
        assert!(is_dangerous_command("node", &serde_json::json!({})));
    }

    #[tokio::test]
    async fn http_plugin_posts_args_and_returns_body() {
        use std::io::{Read, Write};
        // 本地回环一次性服务：任何请求返回 200 "loopback-pong"。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = "loopback-pong";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let adapter = SkillAdapter::with_timeout(Duration::from_secs(5));
        let manifest = serde_json::from_value(serde_json::json!({
            "id": "com.test.http", "type": "skill", "name": "h", "version": "1.0.0",
            "entry": {"kind": "http", "http": format!("http://{addr}/invoke")},
            "functions": [{"name": "f"}],
        }))
        .unwrap();
        match adapter.execute(Path::new("/tmp"), &manifest, "do_demo", &serde_json::json!({"q":"a"})).await {
            SkillOutcome::Ok(text) => assert_eq!(text, "loopback-pong"),
            other => panic!("unexpected {other:?}"),
        }
        handle.join().unwrap();
    }
}
