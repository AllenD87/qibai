//! MCP → 内部工具表的桥接：命名空间、冲突检测、schema 生成、来源标记。
//!
//! 命名规则：`mcp__<server>__<tool>`。冲突（与本地工具或另一 server 重名）时
//! **不许静默覆盖**——该工具不可见，并留下日志与冲突清单。

use crate::mcp::registry::McpRegistry;
use crate::mcp::McpTool;
use serde_json::{json, Value};
use std::collections::HashSet;

/// 命名空间前缀。
pub const NS_PREFIX: &str = "mcp__";

/// 拼出对模型可见的工具名。
pub fn namespaced(server: &str, tool: &str) -> String {
    format!("{NS_PREFIX}{server}__{tool}")
}

/// 解析 `mcp__<server>__<tool>`；不符合命名规则返回 None。
pub fn parse_namespaced(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(NS_PREFIX)?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool.to_string()))
}

/// 解析结果 + 冲突清单。
pub struct BridgedTools {
    pub schemas: Vec<Value>,
    /// (命名空间名, 冲突原因) —— 命中的工具不进入 schemas。
    pub conflicts: Vec<(String, String)>,
}

/// 生成给模型看的 function schema。`local_names` 是本地技能/底座原语已有工具名，
/// 用于检测重名；`seen` 记录已占用名字（含前面已接受的 MCP 工具）。
pub fn build_schemas(
    reg: &McpRegistry,
    /* (server, tool) 列表 */ tools: &[(String, McpTool)],
    local_names: &HashSet<String>,
) -> BridgedTools {
    let _ = reg;
    let mut out = Vec::new();
    let mut conflicts = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (server, tool) in tools {
        let full = namespaced(server, &tool.name);
        if local_names.contains(&full) {
            conflicts.push((full.clone(), format!("与本地工具重名：{full}")));
            continue;
        }
        if !seen.insert(full.clone()) {
            conflicts.push((full.clone(), format!("与其他 MCP 工具重名：{full}")));
            continue;
        }
        let description = if tool.description.is_empty() {
            format!("MCP 工具（来源：MCP / {server}）")
        } else {
            format!("{}（来源：MCP / {server}）", tool.description)
        };
        let params = if tool.input_schema.is_null() {
            json!({"type": "object", "properties": {}})
        } else {
            tool.input_schema.clone()
        };
        out.push(json!({
            "type": "function",
            "function": {
                "name": full,
                "description": description,
                "parameters": params,
            }
        }));
    }

    BridgedTools {
        schemas: out,
        conflicts,
    }
}

/// 调用记录用的来源标记（UI 上必须能分清是本地技能还是外部进程干的）。
pub fn source_label(server: &str) -> String {
    format!("MCP / {server}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: String::new(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn namespace_shape_and_parse() {
        let full = namespaced("fs", "read_file");
        assert_eq!(full, "mcp__fs__read_file");
        assert_eq!(
            parse_namespaced(&full),
            Some(("fs".to_string(), "read_file".to_string()))
        );
        assert_eq!(parse_namespaced("code_read_file"), None);
        assert_eq!(parse_namespaced("mcp__broken"), None);
    }

    #[test]
    fn conflict_with_local_is_blocked_not_overwritten() {
        let reg = McpRegistry::new();
        let mut local = HashSet::new();
        local.insert("mcp__fs__read_file".to_string());
        let tools = vec![("fs".to_string(), tool("read_file"))];
        let b = build_schemas(&reg, &tools, &local);
        assert!(b.schemas.is_empty(), "冲突工具不许进工具表");
        assert_eq!(b.conflicts.len(), 1);
        assert!(b.conflicts[0].1.contains("本地工具重名"));
    }

    #[test]
    fn two_servers_same_tool_name_coexist() {
        let reg = McpRegistry::new();
        let local = HashSet::new();
        // 两个 server 各有一个同名工具：命名空间不同，不算冲突
        let tools = vec![
            ("fs".to_string(), tool("read_file")),
            ("git".to_string(), tool("read_file")),
        ];
        let b = build_schemas(&reg, &tools, &local);
        assert_eq!(b.schemas.len(), 2);
        assert!(b.conflicts.is_empty());
        let names: Vec<String> = b
            .schemas
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"mcp__fs__read_file".to_string()));
        assert!(names.contains(&"mcp__git__read_file".to_string()));
    }

    #[test]
    fn duplicate_same_server_tool_blocked() {
        let reg = McpRegistry::new();
        let local = HashSet::new();
        let tools = vec![
            ("fs".to_string(), tool("read_file")),
            ("fs".to_string(), tool("read_file")),
        ];
        let b = build_schemas(&reg, &tools, &local);
        assert_eq!(b.schemas.len(), 1);
        assert_eq!(b.conflicts.len(), 1);
    }
}
