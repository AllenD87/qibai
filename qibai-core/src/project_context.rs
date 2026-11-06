//! 项目上下文 PROJECT.md：本地工程工作流的跨会话记忆。
//!
//! 工程目录根放置 PROJECT.md，结构化记录：目标 / 需求 / 技术方案 / 进度 / 决策 / 文件清单。
//! - `ensure_context`：建工程时生成（不存在才写，不覆盖）。
//! - `load_context`：解析为 [`ProjectContext`]。
//! - `update_context`：追加进度/文件/决策。
//! - `find_project_dir`：沿父目录向上找最近的 PROJECT.md（write/edit 闭环钩子用）。

use std::path::{Path, PathBuf};

/// PROJECT.md 文件名。
pub const CONTEXT_FILE: &str = "PROJECT.md";

/// 工程上下文结构。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectContext {
    pub goal: String,
    pub requirements: String,
    pub tech_plan: String,
    pub progress: String,
    pub decisions: String,
    pub files: Vec<String>,
}

/// 沿 path 向上查找包含 PROJECT.md 的工程目录。
pub fn find_project_dir(path: &Path) -> Option<PathBuf> {
    let mut cur = if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(|p| p.to_path_buf())
    };
    while let Some(dir) = cur {
        if dir.join(CONTEXT_FILE).is_file() {
            return Some(dir);
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

/// 生成 PROJECT.md 文本模板。
pub fn render_context(c: &ProjectContext) -> String {
    let files_block = if c.files.is_empty() {
        "（暂无）".to_string()
    } else {
        c.files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "# 项目上下文\n\n\
## goal\n{goal}\n\n\
## requirements\n{req}\n\n\
## tech_plan\n{plan}\n\n\
## progress\n{progress}\n\n\
## decisions\n{decisions}\n\n\
## files\n{files}\n",
        goal = c.goal,
        req = c.requirements,
        plan = c.tech_plan,
        progress = c.progress,
        decisions = c.decisions,
        files = files_block
    )
}

/// 建工程时生成 PROJECT.md（已存在则不覆盖）。返回写入的文件路径。
pub fn ensure_context(project_dir: &Path, ctx: &ProjectContext) -> Result<PathBuf, String> {
    let p = project_dir.join(CONTEXT_FILE);
    if p.exists() {
        return Ok(p);
    }
    std::fs::write(&p, render_context(ctx)).map_err(|e| format!("写入 PROJECT.md 失败: {e}"))?;
    Ok(p)
}

/// 解析 PROJECT.md。文件缺失 → None；字段缺失以空串兜底。
pub fn load_context(project_dir: &Path) -> Option<ProjectContext> {
    let p = project_dir.join(CONTEXT_FILE);
    let text = std::fs::read_to_string(&p).ok()?;
    Some(parse_context(&text))
}

fn parse_section<'a>(text: &'a str, name: &str) -> &'a str {
    let heading = format!("## {name}");
    let Some(start) = text.find(&heading) else {
        return "";
    };
    let body = &text[start + heading.len()..];
    let end = body.find("\n## ").unwrap_or(body.len());
    body[..end].trim()
}

fn parse_context(text: &str) -> ProjectContext {
    let files_block = parse_section(text, "files");
    let files = files_block
        .lines()
        .map(|l| l.trim())
        .filter_map(|l| l.strip_prefix("- ").map(|s| s.to_string()))
        .filter(|s| !s.is_empty() && s != "（暂无）")
        .collect();
    ProjectContext {
        goal: parse_section(text, "goal").to_string(),
        requirements: parse_section(text, "requirements").to_string(),
        tech_plan: parse_section(text, "tech_plan").to_string(),
        progress: parse_section(text, "progress").to_string(),
        decisions: parse_section(text, "decisions").to_string(),
        files,
    }
}

/// 增量更新：progress 整体替换；files 合并去重（保留原序）；decisions 追加一行。
#[derive(Debug, Clone, Default)]
pub struct ContextDelta {
    pub progress: Option<String>,
    pub files: Vec<String>,
    pub decisions: Option<String>,
}

/// 读-改-写 PROJECT.md。文件缺失 → Err。
pub fn update_context(project_dir: &Path, delta: &ContextDelta) -> Result<ProjectContext, String> {
    let p = project_dir.join(CONTEXT_FILE);
    if !p.exists() {
        return Err(format!("{} 不存在，无法更新上下文", p.display()));
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取 PROJECT.md 失败: {e}"))?;
    let mut ctx = parse_context(&text);
    if let Some(prog) = &delta.progress {
        ctx.progress = prog.clone();
    }
    for f in &delta.files {
        if !ctx.files.contains(f) {
            ctx.files.push(f.clone());
        }
    }
    if let Some(dec) = &delta.decisions {
        if !dec.trim().is_empty() {
            ctx.decisions = if ctx.decisions.is_empty() {
                dec.clone()
            } else {
                format!("{}\n- {}", ctx.decisions.trim_end_matches('\n'), dec.trim_start_matches("- "))
            };
        }
    }
    std::fs::write(&p, render_context(&ctx)).map_err(|e| format!("写回 PROJECT.md 失败: {e}"))?;
    Ok(ctx)
}

/// fileops 闭环钩子：write/edit/delete 成功后维护工程上下文（失败静默，不影响主操作）。
pub fn hook_file_change(path: &Path, action: &str, note: &str) {
    let Some(dir) = find_project_dir(path) else {
        return; // 不在工程目录内，无需维护
    };
    let rel = path
        .strip_prefix(&dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let delta = match action {
        "delete" => ContextDelta {
            progress: Some(format!("已删除文件 {rel}（{note}）")),
            files: vec![],
            decisions: None,
        },
        _ => ContextDelta {
            progress: Some(format!("已{note} {rel}")),
            files: vec![rel],
            decisions: None,
        },
    };
    let _ = update_context(&dir, &delta);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "qibai_ctx_{}_{}_{}",
            label,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ensure_then_load_roundtrip() {
        let dir = tmp_dir("roundtrip");
        let ctx = ProjectContext {
            goal: "实现天气预报".into(),
            requirements: "- 输入城市".into(),
            tech_plan: "python 骨架".into(),
            progress: "骨架已建".into(),
            decisions: "- 用 requests".into(),
            files: vec!["src/main.py".into(), "README.md".into()],
        };
        ensure_context(&dir, &ctx).unwrap();
        let back = load_context(&dir).expect("应解析出上下文");
        assert_eq!(back.goal, "实现天气预报");
        assert_eq!(back.files, vec!["src/main.py", "README.md"]);
        assert!(back.tech_plan.contains("python"));
        // ensure 幂等不覆盖
        ensure_context(&dir, &ProjectContext::default()).unwrap();
        assert!(load_context(&dir).unwrap().goal.contains("天气预报"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_context_progress_and_files_merge() {
        let dir = tmp_dir("update");
        ensure_context(
            &dir,
            &ProjectContext {
                goal: "g".into(),
                files: vec!["src/main.py".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let ctx = update_context(
            &dir,
            &ContextDelta {
                progress: Some("已写完主函数".into()),
                files: vec!["src/main.py".into(), "README.md".into()],
                decisions: Some("- 新增需求：缓存结果".into()),
            },
        )
        .unwrap();
        assert_eq!(ctx.progress, "已写完主函数");
        assert_eq!(ctx.files, vec!["src/main.py", "README.md"]); // 去重
        assert!(ctx.decisions.contains("缓存结果"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_project_dir_walks_up() {
        let root = tmp_dir("find");
        let src = root.join("src").join("config");
        std::fs::create_dir_all(&src).unwrap();
        ensure_context(&root, &ProjectContext::default()).unwrap();
        let hit = find_project_dir(&src.join("main.py")).unwrap();
        assert_eq!(hit, root);
        // 无 PROJECT.md → None
        let other = tmp_dir("nofind");
        assert!(find_project_dir(&other.join("a.txt")).is_none());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn hook_updates_context_after_file_write() {
        let root = tmp_dir("hook");
        ensure_context(&root, &ProjectContext {
            goal: "g".into(),
            files: vec![],
            ..Default::default()
        }).unwrap();
        let inside = root.join("src").join("note.txt");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, "x").unwrap();
        hook_file_change(&inside, "write", "写入");
        let ctx = load_context(&root).unwrap();
        assert!(ctx.files.contains(&"src/note.txt".to_string()), "files: {:?}", ctx.files);
        assert!(ctx.progress.contains("src/note.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
