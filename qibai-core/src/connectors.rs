//! 自定义连接器（用户运行时配置的外部能力接入）。
//!
//! 用户可在设置/控制台中添加连接器，分为两类：
//! - **Mcp**：MCP stdio 服务器（command + args），物化为 `entry.kind=mcp` 插件；
//! - **Http**：HTTP POST 端点（url + headers），物化为 `entry.kind=http` 插件。
//!
//! 连接器以「泛型入口函数」`fn_<id>` 暴露给 LLM：MCP 连接器调用时用
//! `arguments.__tool` 指定 MCP 工具名、`arguments.arguments` 传工具入参；
//! HTTP 连接器调用时 POST url，body = `{ "args": <arguments> }`。
//!
//! 物化路径：`plugins/<id>/qibai-plugin.json`（+ 空 README），由
//! `PluginRegistry::scan` 统一扫描注册，与外部插件共享启停/权限/工具表链路。
//! 安全：命令启动复用 `skill_adapter::is_dangerous_command` 危险判定拒绝；
//! HTTP 端点仅允许 https/http；外部插件的 `files:write` 硬拒红线依然生效。

use crate::plugin_manifest::{
    EntryKind, EntrySpec, FunctionSpec, Manifest, McpSpec, PluginType,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 连接器类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    /// MCP stdio 服务器（command + args）。
    Mcp,
    /// HTTP POST 端点（url + headers）。
    Http,
}

/// 用户自定义连接器配置（settings.json `connectors` 段；与前端表单契约一致）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectorConfig {
    /// 唯一 id（小写字母/数字/下划线/连字符，安全字符白名单）。
    pub id: String,
    /// 展示名。
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ConnectorKind,
    /// mcp：MCP server 启动命令（可执行文件路径或 PATH basename）。
    #[serde(default)]
    pub command: String,
    /// mcp：附加启动参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// http：POST 端点 URL（https/http）。
    #[serde(default)]
    pub url: String,
    /// http：请求头（可选，如授权头）。
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    /// 是否启用（物化后自动同步 registry enabled）。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 连接器配置阶段硬拒的破坏性命令 basename（不含脚本解释器：
/// python3/node/bash 等是 MCP server 的常见宿主，允许配置；调用期
/// 仍由 `skill_adapter::is_dangerous_command` 危险判定逐次审批）。
const DESTRUCTIVE_BASENAMES: &[&str] = &[
    "rm", "dd", "shutdown", "poweroff", "reboot", "halt", "mv", "chmod", "chown", "kill", "pkill",
    "systemctl", "docker", "curl",
];

fn is_destructive_command(command: &str) -> bool {
    let base = std::path::Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    if base.strip_prefix("mkfs.").is_some() {
        return true;
    }
    DESTRUCTIVE_BASENAMES.contains(&base)
}

/// 连接器配置校验错误。
#[derive(Debug, Clone, thiserror::Error)]
#[error("连接器配置错误 [{field}]: {message}")]
pub struct ConnectorError {
    pub field: String,
    pub message: String,
}

impl ConnectorError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl ConnectorConfig {
    /// 入口函数名（LLM 工具视图）：`fn_<id>`。
    pub fn function_name(&self) -> String {
        format!("fn_{}", self.id)
    }

    /// 入口函数描述（LLM 可见，说明泛型调用约定）。
    fn function_description(&self) -> String {
        match self.kind {
            ConnectorKind::Mcp => format!(
                "调用连接器「{}」（MCP 服务器）。参数：__tool 必填，为 MCP 工具名；arguments 可选，为传给该工具的参数对象。",
                self.name
            ),
            ConnectorKind::Http => format!(
                "调用连接器「{}」（HTTP 端点）。参数将作为 JSON 对象 POST 到端点，响应文本即结果。",
                self.name
            ),
        }
    }

    /// 物化校验。
    pub fn validate(&self) -> Result<(), ConnectorError> {
        if self.id.trim().is_empty() {
            return Err(ConnectorError::new("id", "id 必填且非空"));
        }
        let id_ok = self
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !id_ok {
            return Err(ConnectorError::new(
                "id",
                "仅允许小写字母/数字/下划线/连字符（如 my_weather_mcp）",
            ));
        }
        if self.name.trim().is_empty() {
            return Err(ConnectorError::new("name", "名称必填且非空"));
        }
        match self.kind {
            ConnectorKind::Mcp => {
                if self.command.trim().is_empty() {
                    return Err(ConnectorError::new(
                        "command",
                        "MCP 连接器需填写启动命令",
                    ));
                }
                if is_destructive_command(&self.command) {
                    return Err(ConnectorError::new(
                        "command",
                        format!(
                            "命令 {:?} 命中破坏性命令表，连接器不允许使用（本地优先红线）",
                            self.command
                        ),
                    ));
                }
            }
            ConnectorKind::Http => {
                let u = self.url.trim();
                if u.is_empty() {
                    return Err(ConnectorError::new(
                        "url",
                        "HTTP 连接器需填写端点 URL",
                    ));
                }
                if !(u.starts_with("https://") || u.starts_with("http://")) {
                    return Err(ConnectorError::new(
                        "url",
                        "端点 URL 仅允许 https:// 或 http://",
                    ));
                }
            }
        }
        Ok(())
    }

    /// 生成插件 manifest（物化到 plugins/<id>/）。
    fn to_manifest(&self) -> Manifest {
        let entry = match self.kind {
            ConnectorKind::Mcp => EntrySpec {
                kind: EntryKind::Mcp,
                command: None,
                http: None,
                http_headers: BTreeMap::new(),
                mcp: Some(McpSpec {
                    command: self.command.trim().to_string(),
                    args: self.args.clone(),
                }),
            },
            ConnectorKind::Http => EntrySpec {
                kind: EntryKind::Http,
                command: None,
                http: Some(self.url.trim().to_string()),
                http_headers: self.headers.clone(),
                mcp: None,
            },
        };
        let permissions = match self.kind {
            ConnectorKind::Mcp => vec!["command:run".to_string()],
            ConnectorKind::Http => vec!["network:read".to_string()],
        };
        Manifest {
            id: self.id.clone(),
            plugin_type: PluginType::Connector,
            name: self.name.clone(),
            description: self.description.clone(),
            icon: None,
            version: "1.0.0".to_string(),
            author: Some("user".to_string()),
            permissions,
            entry: Some(entry),
            functions: vec![FunctionSpec {
                name: self.function_name(),
                description: self.function_description(),
                parameters: match self.kind {
                    ConnectorKind::Mcp => json!({
                        "type": "object",
                        "properties": {
                            "__tool": { "type": "string", "description": "MCP 工具名" },
                            "arguments": { "type": "object", "description": "传给 MCP 工具的参数" }
                        },
                        "required": ["__tool"]
                    }),
                    ConnectorKind::Http => json!({
                        "type": "object",
                        "description": "POST 到端点的 JSON 参数对象"
                    }),
                },
            }],
            system_prompt: None,
            skills: vec![],
            persona_binding: None,
        }
    }

    /// 物化：写入 `plugins/<id>/qibai-plugin.json`。幂等（覆盖同 id）。
    pub fn materialize(&self, plugins_dir: &Path) -> Result<PathBuf, ConnectorError> {
        self.validate()?;
        let m = self.to_manifest();
        crate::plugin_manifest::ManifestLoader::validate(&m)
            .map_err(|e| ConnectorError::new("manifest", e.to_string()))?;
        let dir = plugins_dir.join(&self.id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| ConnectorError::new("dir", format!("创建连接器目录失败: {e}")))?;
        let json_text = serde_json::to_string_pretty(&m)
            .map_err(|e| ConnectorError::new("manifest", format!("manifest 序列化失败: {e}")))?;
        std::fs::write(dir.join("qibai-plugin.json"), json_text)
            .map_err(|e| ConnectorError::new("dir", format!("写入 manifest 失败: {e}")))?;
        // 说明文件（供用户在目录中查阅调用约定）。
        let _ = std::fs::write(
            dir.join("README.md"),
            format!(
                "# 连接器：{}\n\n- 类型：{}\n- 入口函数：`{}`\n- 描述：{}\n\n调用约定：见 qibai-plugin.json 中 functions[].description。\n",
                self.name,
                match self.kind {
                    ConnectorKind::Mcp => "MCP stdio 服务器",
                    ConnectorKind::Http => "HTTP POST 端点",
                },
                self.function_name(),
                self.description
            ),
        );
        Ok(dir)
    }
}

/// 移除连接器：删除物化目录（由上层同步重扫 registry）。
pub fn remove(plugins_dir: &Path, id: &str) -> Result<(), ConnectorError> {
    let dir = plugins_dir.join(id);
    if !dir.exists() {
        return Ok(());
    }
    if !dir.is_dir() {
        return Err(ConnectorError::new("id", format!("{id} 不是连接器目录")));
    }
    std::fs::remove_dir_all(&dir)
        .map_err(|e| ConnectorError::new("dir", format!("删除连接器目录失败: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qibai_connectors_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mcp_connector_materializes_and_validates() {
        let root = tmp_dir("mcp");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let cfg = ConnectorConfig {
            id: "my_mcp".into(),
            name: "我的 MCP 服务器".into(),
            kind: ConnectorKind::Mcp,
            command: "python3".into(),
            args: vec!["/tmp/qibai_mock_server.py".into()],
            url: String::new(),
            headers: BTreeMap::new(),
            description: "测试连接器".into(),
            enabled: true,
        };
        cfg.validate().unwrap();
        let dir = cfg.materialize(&plugins).unwrap();
        assert!(dir.join("qibai-plugin.json").exists());
        let m = crate::plugin_manifest::ManifestLoader::load(&dir).unwrap();
        assert_eq!(m.plugin_type, PluginType::Connector);
        assert_eq!(m.entry.as_ref().unwrap().kind, EntryKind::Mcp);
        assert_eq!(m.functions.len(), 1);
        assert_eq!(m.functions[0].name, "fn_my_mcp");
    }

    #[test]
    fn http_connector_materializes() {
        let root = tmp_dir("http");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let cfg = ConnectorConfig {
            id: "my_http".into(),
            name: "HTTP 连接器".into(),
            kind: ConnectorKind::Http,
            command: String::new(),
            args: vec![],
            url: "https://example.com/api".into(),
            headers: BTreeMap::from([("Authorization".into(), "Bearer x".into())]),
            description: String::new(),
            enabled: true,
        };
        cfg.validate().unwrap();
        let dir = cfg.materialize(&plugins).unwrap();
        let m = crate::plugin_manifest::ManifestLoader::load(&dir).unwrap();
        assert_eq!(m.entry.as_ref().unwrap().kind, EntryKind::Http);
        assert_eq!(
            m.entry.as_ref().unwrap().http.as_deref(),
            Some("https://example.com/api")
        );
    }

    #[test]
    fn dangerous_command_rejected() {
        let cfg = ConnectorConfig {
            id: "bad".into(),
            name: "bad".into(),
            kind: ConnectorKind::Mcp,
            command: "rm".into(),
            args: vec![],
            url: String::new(),
            headers: BTreeMap::new(),
            description: String::new(),
            enabled: true,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn bad_id_and_url_rejected() {
        let cfg = ConnectorConfig {
            id: "Bad ID!".into(),
            name: "x".into(),
            kind: ConnectorKind::Mcp,
            command: "python3".into(),
            args: vec![],
            url: String::new(),
            headers: BTreeMap::new(),
            description: String::new(),
            enabled: true,
        };
        assert!(cfg.validate().is_err());

        let cfg2 = ConnectorConfig {
            id: "ok_id".into(),
            name: "x".into(),
            kind: ConnectorKind::Http,
            command: String::new(),
            args: vec![],
            url: "file:///etc/passwd".into(),
            headers: BTreeMap::new(),
            description: String::new(),
            enabled: true,
        };
        assert!(cfg2.validate().is_err());
    }

    #[test]
    fn remove_is_idempotent() {
        let root = tmp_dir("rm");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        remove(&plugins, "nope").unwrap(); // 不存在 → ok
        let cfg = ConnectorConfig {
            id: "gone".into(),
            name: "x".into(),
            kind: ConnectorKind::Mcp,
            command: "python3".into(),
            args: vec![],
            url: String::new(),
            headers: BTreeMap::new(),
            description: String::new(),
            enabled: true,
        };
        cfg.materialize(&plugins).unwrap();
        assert!(plugins.join("gone").exists());
        remove(&plugins, "gone").unwrap();
        assert!(!plugins.join("gone").exists());
        remove(&plugins, "gone").unwrap(); // 幂等
    }
}
