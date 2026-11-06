//! 插件包清单（`qibai-plugin.json`）。
//!
//! - [`Manifest`]：清单 Schema（skill / expert / connector 三类）。
//! - [`ManifestLoader`]：解析 + WorkBuddy 风格字段同义词归一化 + 校验。
//! - [`Permission`]：权限已知集合（声明即校验，未知权限 → 校验错误）。
//!
//! 官方 WorkBuddy 格式到手后，仅需调整 [`SYNONYM_MAP`]，业务代码不动。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// WorkBuddy 风格字段同义词映射表（契约 §3.1）。
/// 每项 = (规范字段名, [可接受的别名列表，含规范名自身])。
pub const SYNONYM_MAP: &[(&str, &[&str])] = &[
    ("type", &["type", "capability", "plugin_type", "category"]),
    ("kind", &["kind", "backend", "executor", "entry_kind"]),
    ("entry", &["entry", "exec", "invoke"]),
    ("command", &["command", "script", "cmd"]),
    ("http", &["http", "url", "endpoint"]),
    ("mcp", &["mcp", "stdio", "mcp_server"]),
    (
        "permissions",
        &["permissions", "perms", "grants", "required_permissions"],
    ),
    ("functions", &["functions", "tools", "actions"]),
    (
        "system_prompt",
        &["system_prompt", "prompt", "systemPrompt"],
    ),
    ("skills", &["skills", "skill_ids", "whitelist"]),
    (
        "persona_binding",
        &["persona_binding", "persona", "binding"],
    ),
];

/// 插件类型（serde 小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    Skill,
    Expert,
    Connector,
}

impl PluginType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginType::Skill => "skill",
            PluginType::Expert => "expert",
            PluginType::Connector => "connector",
        }
    }
}

/// 执行后端类型（entry.kind）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Command,
    Http,
    Builtin,
    /// MCP（Model Context Protocol）stdio 服务器：以子进程方式启动，
    /// 通过 JSON-RPC（stdin/stdout）暴露工具集，工具可被 LLM 调用。
    Mcp,
}

impl EntryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryKind::Command => "command",
            EntryKind::Http => "http",
            EntryKind::Builtin => "builtin",
            EntryKind::Mcp => "mcp",
        }
    }
}

/// MCP 服务器启动声明（entry.kind=mcp）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSpec {
    /// 启动命令（可执行文件路径或 basename，按 PATH 解析）。
    pub command: String,
    /// 附加启动参数（如 `--config x.json`）。
    #[serde(default)]
    pub args: Vec<String>,
}

/// 执行入口声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrySpec {
    pub kind: EntryKind,
    /// command 类：相对插件目录的脚本/命令路径。
    #[serde(default)]
    pub command: Option<String>,
    /// http 类：POST 端点 URL。
    #[serde(default)]
    pub http: Option<String>,
    /// http 类：自定义请求头（连接器场景，如授权头）。
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
    /// mcp 类：MCP stdio 服务器启动声明。
    #[serde(default)]
    pub mcp: Option<McpSpec>,
}

/// LLM 工具 schema（function calling 视图）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema 对象；缺省视为 `{}`。
    #[serde(default)]
    pub parameters: serde_json::Value,
}

/// 专家音色/形象绑定（可选）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaBinding {
    /// 已知 persona id（如 gudian）；未知字符串原样透传。
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub mood: Option<String>,
}

/// 插件清单（归一化后的规范形状）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// 唯一 id（反向域名）。
    pub id: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// 语义化版本 `x.y.z`。
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    /// 权限声明（字符串）；加载后校验为已知 [`Permission`]。
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub entry: Option<EntrySpec>,
    #[serde(default)]
    pub functions: Vec<FunctionSpec>,
    /// type=expert 必填。
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// 专家技能白名单（plugin id 或 function name；空 = 全开放）。
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub persona_binding: Option<PersonaBinding>,
}

/// 权限已知集合（契约 §3.1）。未知权限 → 校验错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    FilesRead,
    FilesWrite,
    NetworkRead,
    CommandRun,
    SystemLaunch,
    ReminderSet,
    /// `system:*`：守护类系统权限的通配声明。
    SystemWildcard,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::FilesRead => "files:read",
            Permission::FilesWrite => "files:write",
            Permission::NetworkRead => "network:read",
            Permission::CommandRun => "command:run",
            Permission::SystemLaunch => "system:launch",
            Permission::ReminderSet => "reminder:set",
            Permission::SystemWildcard => "system:*",
        }
    }

    pub fn from_str(s: &str) -> Option<Permission> {
        match s.trim() {
            "files:read" => Some(Permission::FilesRead),
            "files:write" => Some(Permission::FilesWrite),
            "network:read" => Some(Permission::NetworkRead),
            "command:run" => Some(Permission::CommandRun),
            "system:launch" => Some(Permission::SystemLaunch),
            "reminder:set" => Some(Permission::ReminderSet),
            "system:*" => Some(Permission::SystemWildcard),
            _ => None,
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 清单校验错误（含字段路径，便于定位）。
#[derive(Debug, Clone, thiserror::Error)]
#[error("manifest 校验失败 [{field}]: {message}")]
pub struct ManifestError {
    pub field: String,
    pub message: String,
}

impl ManifestError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// 类型值同义词归一化：`skill|capability`→skill，`expert|assistant`→expert，`connector|integration`→connector。
fn norm_plugin_type_value(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "skill" | "capability" => "skill".to_string(),
        "expert" | "assistant" => "expert".to_string(),
        "connector" | "integration" => "connector".to_string(),
        other => other.to_string(),
    }
}

/// 执行后端值同义词归一化：`command|cli|shell`→command，`http|api|webhook`→http，`builtin|native`→builtin，`mcp|stdio|mcpserver`→mcp。
fn norm_entry_kind_value(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "command" | "cli" | "shell" => "command".to_string(),
        "http" | "api" | "webhook" => "http".to_string(),
        "builtin" | "native" => "builtin".to_string(),
        "mcp" | "stdio" | "mcpserver" => "mcp".to_string(),
        other => other.to_string(),
    }
}

fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// 字段归一化：把同义词键重命名为规范键（顶层 + entry 嵌套层）。
fn normalize(raw: &mut serde_json::Value) {
    let Some(root) = raw.as_object_mut() else {
        return;
    };
    // 顶层同义词：取第一个命中的别名为规范键。
    for (target, aliases) in SYNONYM_MAP.iter() {
        if *target == "kind" || *target == "command" || *target == "http" || *target == "mcp" {
            continue; // entry 嵌套层字段，稍后处理
        }
        let found = aliases
            .iter()
            .find_map(|a| root.get(*a).cloned())
            .or_else(|| root.get(*target).cloned());
        if let Some(v) = found {
            for a in *aliases {
                root.remove(*a);
            }
            root.insert(target.to_string(), v);
        }
    }
    // type 值归一化
    if let Some(v) = root.get("type").and_then(|t| t.as_str()) {
        let norm = norm_plugin_type_value(v);
        root.insert("type".to_string(), serde_json::json!(norm));
    }
    // entry 嵌套归一化
    if let Some(entry) = root.get_mut("entry").and_then(|e| e.as_object_mut()) {
        for (target, aliases) in SYNONYM_MAP.iter() {
            if !matches!(*target, "kind" | "command" | "http" | "mcp") {
                continue;
            }
            let found = aliases
                .iter()
                .find_map(|a| entry.get(*a).cloned())
                .or_else(|| entry.get(*target).cloned());
            if let Some(v) = found {
                for a in *aliases {
                    entry.remove(*a);
                }
                entry.insert(target.to_string(), v);
            }
        }
        // kind 值归一化
        if let Some(v) = entry.get("kind").and_then(|t| t.as_str()) {
            let norm = norm_entry_kind_value(v);
            entry.insert("kind".to_string(), serde_json::json!(norm));
        }
        // MCP 简写归一化：kind=mcp 时，entry 顶层 command/args 挪进 entry.mcp.command/args
        // （兼容 `"entry": {"kind":"mcp","command":"python3"}` 的直观写法）。
        if entry.get("kind").and_then(|k| k.as_str()) == Some("mcp") {
            let top_command = entry.remove("command");
            let top_args = entry.remove("args");
            if top_command.is_some() || top_args.is_some() {
                let mcp = entry
                    .entry("mcp")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(obj) = mcp.as_object_mut() {
                    if !obj.contains_key("command") {
                        if let Some(c) = top_command {
                            obj.insert("command".to_string(), c);
                        }
                    }
                    if !obj.contains_key("args") {
                        if let Some(a) = top_args {
                            obj.insert("args".to_string(), a);
                        }
                    }
                }
            }
        }
        // 变量替换：entry.mcp.args 中的 `{QIBAI_HOME}` → 元气小小数据目录。
        // 让示例 MCP 插件（scripts/mcp_servers/*.py）复制即用，无需手改绝对路径。
        // 取值：env QIBAI_HOME 优先，否则 `~/.qibai`。
        if let Some(mcp) = entry.get_mut("mcp").and_then(|m| m.as_object_mut()) {
            if let Some(args) = mcp.get_mut("args").and_then(|a| a.as_array_mut()) {
                let home = std::env::var("QIBAI_HOME").unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(|h| format!("{h}/.qibai"))
                        .unwrap_or_else(|_| ".".to_string())
                });
                for a in args {
                    if let Some(s) = a.as_str() {
                        *a = serde_json::json!(s.replace("{QIBAI_HOME}", &home));
                    }
                }
            }
        }
    }
}

/// 清单加载器：从文件/文本解析并校验。
pub struct ManifestLoader;

impl ManifestLoader {
    /// 从 `<dir>/qibai-plugin.json` 加载。
    pub fn load(dir: &Path) -> Result<Manifest, ManifestError> {
        let path = dir.join("qibai-plugin.json");
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            ManifestError::new("file", format!("读取清单失败 {path:?}: {e}"))
        })?;
        Self::parse(&raw)
    }

    /// 解析 JSON 文本：归一化 → 反序列化 → 校验。
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let mut raw: serde_json::Value =
            serde_json::from_str(text).map_err(|e| ManifestError::new("json", e.to_string()))?;
        normalize(&mut raw);
        let m: Manifest = serde_json::from_value(raw)
            .map_err(|e| ManifestError::new("shape", e.to_string()))?;
        Self::validate(&m)?;
        Ok(m)
    }

    /// 校验规则（契约 §3.1）。`is_builtin_package` 区分内置包与外部包——
    /// 外部包声明 `kind=builtin` 由注册表层拒绝（manifest 本身不感知内置归属）。
    pub fn validate(m: &Manifest) -> Result<(), ManifestError> {
        if m.id.trim().is_empty() {
            return Err(ManifestError::new("id", "必填且非空"));
        }
        if !is_semver(&m.version) {
            return Err(ManifestError::new(
                "version",
                format!("需匹配 x.y.z（当前 {:?}）", m.version),
            ));
        }
        // 权限声明校验：未知权限直接拒绝。
        for p in &m.permissions {
            if Permission::from_str(p).is_none() {
                return Err(ManifestError::new(
                    "permissions",
                    format!("未知权限 {p:?}（已知集合：files:read / files:write / network:read / command:run / system:launch / reminder:set / system:*）"),
                ));
            }
        }
        match m.plugin_type {
            PluginType::Expert => {
                // expert 需 system_prompt 非空；skills 白名单可空（=全开放）。
                match &m.system_prompt {
                    Some(s) if !s.trim().is_empty() => {}
                    _ => {
                        return Err(ManifestError::new(
                            "system_prompt",
                            "type=expert 需非空 system_prompt",
                        ))
                    }
                }
            }
            PluginType::Skill | PluginType::Connector => {
                // skill/connector 需 entry 或 functions 至少其一。
                if m.entry.is_none() && m.functions.is_empty() {
                    return Err(ManifestError::new(
                        "entry",
                        "skill/connector 需 entry 或 functions 至少其一",
                    ));
                }
                if let Some(entry) = &m.entry {
                    match entry.kind {
                        EntryKind::Command => {
                            if entry.command.as_ref().map(|c| c.trim().is_empty()).unwrap_or(true) {
                                return Err(ManifestError::new(
                                    "entry.command",
                                    "kind=command 需非空 command 路径",
                                ));
                            }
                        }
                        EntryKind::Http => {
                            if entry.http.as_ref().map(|h| h.trim().is_empty()).unwrap_or(true) {
                                return Err(ManifestError::new(
                                    "entry.http",
                                    "kind=http 需非空 http 端点",
                                ));
                            }
                        }
                        EntryKind::Mcp => {
                            let ok = entry
                                .mcp
                                .as_ref()
                                .map(|m| !m.command.trim().is_empty())
                                .unwrap_or(false);
                            if !ok {
                                return Err(ManifestError::new(
                                    "entry.mcp.command",
                                    "kind=mcp 需非空 mcp.command（MCP server 启动命令）",
                                ));
                            }
                        }
                        EntryKind::Builtin => {} // 内置包专属，由注册表层判归属
                    }
                }
            }
        }
        // functions：name 非空且本 manifest 内唯一。
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        for f in &m.functions {
            if f.name.trim().is_empty() {
                return Err(ManifestError::new("functions.name", "函数名必填且非空"));
            }
            if seen.insert(f.name.as_str(), ()).is_some() {
                return Err(ManifestError::new(
                    "functions.name",
                    format!("函数名 {} 在本 manifest 内重复", f.name),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> serde_json::Value {
        serde_json::json!({
            "id": "com.example.demo",
            "type": "skill",
            "name": "演示技能",
            "version": "1.0.0",
            "permissions": ["command:run"],
            "entry": {"kind": "command", "command": "scripts/run.sh"},
            "functions": [{"name": "do_demo", "description": "演示", "parameters": {"type": "object"}}],
        })
    }

    #[test]
    fn parses_clean_manifest() {
        let v = base_manifest();
        let m = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(m.id, "com.example.demo");
        assert_eq!(m.plugin_type, PluginType::Skill);
        assert_eq!(m.entry.unwrap().kind, EntryKind::Command);
        assert_eq!(m.functions.len(), 1);
    }

    #[test]
    fn synonym_fields_are_normalized() {
        // WorkBuddy 风格别名：capability / exec / script / tools / prompt / whitelist
        let v = serde_json::json!({
            "id": "com.example.alias",
            "capability": "assistant",
            "name": "别名专家",
            "version": "1.2.3",
            "perms": [],
            "exec": {"backend": "cli", "script": "scripts/x.sh"},
            "tools": [{"name": "f1"}],
            "prompt": "你是一位专家。",
            "whitelist": ["com.qibai.launch_app"],
            "persona": {"voice": "gudian", "mood": "professional"},
        });
        let m = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(m.plugin_type, PluginType::Expert);
        let entry = m.entry.unwrap();
        assert_eq!(entry.kind, EntryKind::Command);
        assert_eq!(entry.command.unwrap(), "scripts/x.sh");
        assert_eq!(m.functions[0].name, "f1");
        assert!(m.system_prompt.unwrap().contains("专家"));
        assert_eq!(m.skills, vec!["com.qibai.launch_app".to_string()]);
        assert_eq!(m.persona_binding.unwrap().voice.unwrap(), "gudian");
    }

    #[test]
    fn rejects_bad_version() {
        let mut v = base_manifest();
        v["version"] = serde_json::json!("1.0");
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "version");
    }

    #[test]
    fn rejects_unknown_permission() {
        let mut v = base_manifest();
        v["permissions"] = serde_json::json!(["rm -rf /"]);
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "permissions");
    }

    #[test]
    fn expert_requires_system_prompt() {
        let v = serde_json::json!({
            "id": "com.bad.expert",
            "type": "expert",
            "name": "空专家",
            "version": "1.0.0",
        });
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "system_prompt");
    }

    #[test]
    fn skill_requires_entry_or_functions() {
        let v = serde_json::json!({
            "id": "com.bad.skill",
            "type": "skill",
            "name": "空技能",
            "version": "1.0.0",
        });
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "entry");
    }

    #[test]
    fn duplicate_function_names_rejected() {
        let mut v = base_manifest();
        v["functions"] = serde_json::json!([
            {"name": "f1"}, {"name": "f1"}
        ]);
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "functions.name");
    }

    #[test]
    fn mcp_manifest_parses_and_validates() {
        let v = serde_json::json!({
            "id": "com.qibai.mcp.demo",
            "type": "connector",
            "name": "MCP 演示连接器",
            "version": "1.0.0",
            "entry": {
                "kind": "mcp",
                "command": "python3",
                "args": ["-m", "mock_server"]
            },
            "functions": [{"name": "mcp_ping", "description": "回显"}]
        });
        let m = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(m.plugin_type, PluginType::Connector);
        let entry = m.entry.unwrap();
        assert_eq!(entry.kind, EntryKind::Mcp);
        let spec = entry.mcp.unwrap();
        assert_eq!(spec.command, "python3");
        assert_eq!(spec.args, vec!["-m".to_string(), "mock_server".to_string()]);
    }

    #[test]
    fn mcp_requires_command() {
        let v = serde_json::json!({
            "id": "com.bad.mcp",
            "type": "connector",
            "name": "坏 MCP",
            "version": "1.0.0",
            "entry": {"kind": "mcp"}
        });
        let err = ManifestLoader::parse(&serde_json::to_string(&v).unwrap()).unwrap_err();
        assert_eq!(err.field, "entry.mcp.command");
    }

    #[test]
    fn permission_roundtrip_known_set() {
        for s in [
            "files:read",
            "files:write",
            "network:read",
            "command:run",
            "system:launch",
            "reminder:set",
            "system:*",
        ] {
            let p = Permission::from_str(s).unwrap();
            assert_eq!(p.as_str(), s);
        }
        assert!(Permission::from_str("files:delete").is_none());
    }
}
