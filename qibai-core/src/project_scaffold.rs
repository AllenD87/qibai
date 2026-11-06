//! 本地工程脚手架：create_project 技能核心。
//!
//! 完整骨架（四语言 python/node/rust/web）：src/ 源码目录 + config/ 配置目录 +
//! README.md + .gitignore + 入口文件 + PROJECT.md 上下文 + 可选 git init。
//! 项目注册表 $QIBAI_DATA_DIR/projects.json 支持「继续上次任务」。

use crate::fileops::sanitize_path;
use crate::project_context::{self, ProjectContext};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 语言支持集合。
pub const LANGS: [&str; 4] = ["python", "node", "rust", "web"];

/// 项目注册表记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub name: String,
    pub path: String,
    pub lang: String,
    pub created_at: u64,
    pub updated_at: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 注册表路径：$QIBAI_DATA_DIR/projects.json → data_dir()/projects.json。
pub fn registry_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QIBAI_DATA_DIR") {
        return Some(PathBuf::from(dir).join("projects.json"));
    }
    crate::settings::data_dir().ok().map(|d| d.join("projects.json"))
}

/// 读注册表（缺失/损坏 → 空表，绝不 panic）。
pub fn load_registry() -> Vec<ProjectRecord> {
    let Some(p) = registry_path() else {
        return vec![];
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return vec![];
    };
    serde_json::from_str::<Vec<ProjectRecord>>(&text).unwrap_or_default()
}

fn save_registry(records: &[ProjectRecord]) -> Result<(), String> {
    let Some(p) = registry_path() else {
        return Err("无法定位数据目录".to_string());
    };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = serde_json::to_string_pretty(records).map_err(|e| format!("序列化注册表失败: {e}"))?;
    std::fs::write(&p, text).map_err(|e| format!("写入注册表失败: {e}"))
}

/// 登记工程（同名同路径覆盖更新）。updated_at 单调递增，避免同毫秒平局。
pub fn register_project(name: &str, path: &str, lang: &str) -> Result<(), String> {
    let now = now_ms();
    let mut records = load_registry();
    let max_ts = records.iter().map(|r| r.updated_at).max().unwrap_or(0);
    let ts = now.max(max_ts.saturating_add(1));
    if let Some(r) = records.iter_mut().find(|r| r.path == path) {
        r.updated_at = ts;
    } else {
        records.push(ProjectRecord {
            name: name.to_string(),
            path: path.to_string(),
            lang: lang.to_string(),
            created_at: ts,
            updated_at: ts,
        });
    }
    save_registry(&records)
}

/// write/edit 闭环：同步 updated_at（单调递增，防同毫秒平局）。
pub fn touch_project(path: &str) {
    let mut records = load_registry();
    let mut changed = false;
    let max_ts = records.iter().map(|r| r.updated_at).max().unwrap_or(0);
    let ts = now_ms().max(max_ts.saturating_add(1));
    for r in records.iter_mut() {
        if r.path == path {
            r.updated_at = ts;
            changed = true;
        }
    }
    if changed {
        let _ = save_registry(&records);
    }
}

/// 兄弟方 create_project 索引（~/qibai/projects-index.json）：name → 绝对路径。
fn sibling_index(home: &str) -> serde_json::Map<String, serde_json::Value> {
    let p = std::path::Path::new(home).join("qibai").join("projects-index.json");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok())
        .unwrap_or_default()
}

/// resume_project 技能核心：返回工程上下文中文摘要。
/// name 空 → 取最近一次任务（兄弟方索引优先，其次 projects.json 注册表）。
pub fn resume_summary(name: &str, parent_dir: &str, home: &str) -> Result<String, String> {
    let n = name.trim();
    // 1. 定位工程目录
    let (project_dir, is_recent) = if n.is_empty() {
        // 兄弟方索引：取任意（无时间戳，取最后一个）
        let idx = sibling_index(home);
        let mine = recent_project();
        if let Some(rec) = mine {
            (PathBuf::from(rec.path), true)
        } else if let Some((_, v)) = idx.iter().next() {
            (PathBuf::from(v.as_str().unwrap_or("")), true)
        } else {
            return Err("还没有任何工程记录，说「创建工程」开始第一个吧".to_string());
        }
    } else {
        let parent = sanitize_path(parent_dir, home)?;
        (parent.join(n), false)
    };
    if !project_dir.is_dir() {
        return Err(format!("工程目录不存在：{}", project_dir.display()));
    }
    // 2. 读上下文（PROJECT.md 优先，兼容 .qibai-context.md）
    let text = if project_dir.join("PROJECT.md").is_file() {
        std::fs::read_to_string(project_dir.join("PROJECT.md")).unwrap_or_default()
    } else if project_dir.join(".qibai-context.md").is_file() {
        std::fs::read_to_string(project_dir.join(".qibai-context.md")).unwrap_or_default()
    } else {
        return Err(format!("{} 里没有项目上下文文件", project_dir.display()));
    };
    let dir_name = project_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    // 3. 结构化提取（PROJECT.md 分段；.qibai-context.md 键值行）
    let extract = |key: &str| -> String {
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let l = line.trim();
            if l == format!("## {key}") {
                return lines
                    .find(|l: &&str| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            if let Some(rest) = l.strip_prefix(&format!("- {key}:")) {
                return rest.trim().to_string();
            }
            if let Some(rest) = l.strip_prefix(&format!("{key}:")) {
                return rest.trim().to_string();
            }
        }
        String::new()
    };
    let goal = extract("goal");
    let progress = extract("progress");
    let mut out = String::new();
    if is_recent {
        out.push_str("这是最近一次任务。\n");
    }
    out.push_str(&format!("工程：{dir_name}（{dir}）\n", dir = project_dir.display()));
    if !goal.is_empty() {
        out.push_str(&format!("目标：{goal}\n"));
    }
    if !progress.is_empty() {
        out.push_str(&format!("上次进度：{progress}\n"));
    }
    // 文件清单
    let files: Vec<String> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("- "))
        .map(|l| l.trim().trim_start_matches("- ").to_string())
        .filter(|l| !l.contains(':') && !l.is_empty())
        .take(12)
        .collect();
    if !files.is_empty() {
        out.push_str(&format!("文件清单：{}\n", files.join("、")));
    }
    out.push_str("下一步：告诉我继续做什么（写文件 / 改文件 / 查资料）即可。");
    Ok(out)
}

/// 最近一次任务（updated_at 最大）。
pub fn recent_project() -> Option<ProjectRecord> {
    load_registry()
        .into_iter()
        .max_by_key(|r| r.updated_at)
}

/// name 净化：仅 [A-Za-z0-9_-]，1..=64 字符。
pub fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("工程名为空，给我一个名字".to_string());
    }
    if name.len() > 64 {
        return Err("工程名太长（最多 64 字符）".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("工程名只能用英文、数字、下划线和中划线（防路径注入）".to_string());
    }
    Ok(())
}

/// 骨架文件清单（返回 (相对路径, 内容)）。
fn scaffold_files(name: &str, lang: &str, goal: &str) -> Vec<(&'static str, String)> {
    match lang {
        "python" => vec![
            (
                "src/main.py",
                format!(
                    "\"\"\"{name} —— {goal}\"\"\"\n\n\ndef main():\n    print(\"你好，{name} 已就绪！目标：{goal}\")\n\n\nif __name__ == \"__main__\":\n    main()\n"
                ),
            ),
            (
                "requirements.txt",
                format!("# {name} 依赖清单\n# 示例：requests>=2.31\n"),
            ),
            (
                "config/settings.toml",
                format!("# {name} 配置示例\n# env = \"dev\"\n"),
            ),
            (
                "README.md",
                format!("# {name}\n\n{goal}\n\n## 运行\n```bash\npython3 src/main.py\n```\n"),
            ),
            (".gitignore", "__pycache__/\n*.pyc\n.venv/\n".to_string()),
        ],
        "node" => vec![
            (
                "package.json",
                serde_json::json!({
                    "name": name,
                    "version": "0.1.0",
                    "description": goal,
                    "scripts": {"start": "node src/index.js"}
                })
                .to_string(),
            ),
            (
                "src/index.js",
                format!(
                    "// {name} —— {goal}\nconsole.log(`你好，{name} 已就绪！目标：{goal}`);\n"
                ),
            ),
            (
                "config/settings.json",
                "{}\n".to_string(),
            ),
            (
                "README.md",
                format!("# {name}\n\n{goal}\n\n## 运行\n```bash\nnpm start\n```\n"),
            ),
            (".gitignore", "node_modules/\n".to_string()),
        ],
        "rust" => vec![
            (
                "Cargo.toml",
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
                ),
            ),
            (
                "src/main.rs",
                format!(
                    "// {name} —— {goal}\nfn main() {{\n    println!(\"你好，{name} 已就绪！目标：{goal}\");\n}}\n"
                ),
            ),
            (
                "config/settings.toml",
                "# config\n".to_string(),
            ),
            (
                "README.md",
                format!("# {name}\n\n{goal}\n\n## 运行\n```bash\ncargo run\n```\n"),
            ),
            (".gitignore", "target/\n".to_string()),
        ],
        _ => vec![
            (
                "index.html",
                format!(
                    "<!doctype html>\n<html lang=\"zh\">\n<head>\n<meta charset=\"utf-8\">\n<title>{name}</title>\n</head>\n<body>\n<h1>{name}</h1>\n<p>{goal}</p>\n<script src=\"src/main.js\"></script>\n</body>\n</html>\n"
                ),
            ),
            (
                "src/main.js",
                format!(
                    "// {name} —— {goal}\ndocument.body.append(\"已加载\");\n"
                ),
            ),
            (
                "config/settings.json",
                "{}\n".to_string(),
            ),
            (
                "README.md",
                format!("# {name}\n\n{goal}\n\n用浏览器打开 index.html 预览。\n"),
            ),
            (".gitignore", "node_modules/\n".to_string()),
        ],
    }
}

fn run_cmd(cmd: &str, args: &[&str], cwd: &Path) -> bool {
    std::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 是否已处于 git 仓库内（向上找 .git）。
fn already_in_git_repo(dir: &Path) -> bool {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return true;
        }
        cur = d.parent();
    }
    false
}

/// 方案文本：PendingApproval 的 description（供朗读确认）。
pub fn build_plan(name: &str, lang: &str, goal: &str, git_init: bool) -> String {
    let files = scaffold_files(name, lang, goal);
    let file_lines: Vec<String> = files.iter().map(|(f, _)| format!("- {f}")).collect();
    let git_line = if git_init { "→ git init（如尚不是仓库）" } else { "" };
    format!(
        "即将创建 {lang} 工程「{name}」\n\
         目标：{goal}\n\
         将生成：\n{files}\n\
         - PROJECT.md 项目上下文\n\
         执行步骤：创建目录 → 写入 {n} 个文件 → 生成 PROJECT.md{git_line} → 登记工程注册表。\n\
         确认后即开始创建。",
        files = file_lines.join("\n"),
        n = files.len(),
    )
}

/// 批准后执行：建目录 → 骨架文件 → PROJECT.md → git init → 登记注册表。返回分步汇报文本。
pub fn perform_create_project(
    name: &str,
    parent_dir: &str,
    lang: &str,
    goal: &str,
    git_init: bool,
    home: &str,
) -> Result<String, String> {
    validate_name(name)?;
    let parent = sanitize_path(parent_dir, home)?;
    let project_dir = parent.join(name);
    if project_dir.exists() {
        return Err("目录已存在，换个项目名试试".to_string());
    }

    // 步骤 1：建目录
    std::fs::create_dir_all(&project_dir).map_err(|e| format!("创建工程目录失败: {e}"))?;
    eprintln!("[create_project] 已创建目录 {}", project_dir.display());

    // 步骤 2：写骨架文件
    let files = scaffold_files(name, lang, goal);
    for (rel, content) in &files {
        let full = project_dir.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建子目录失败: {e}"))?;
        }
        std::fs::write(&full, content).map_err(|e| format!("写入 {rel} 失败: {e}"))?;
    }
    eprintln!("[create_project] 已写入 {} 个骨架文件", files.len());

    // 步骤 3：PROJECT.md 上下文
    let ctx = ProjectContext {
        goal: goal.to_string(),
        requirements: "（待补充）".to_string(),
        tech_plan: format!("{lang} 骨架：src/ 入口 + config/ 配置目录"),
        progress: "骨架已创建".to_string(),
        decisions: format!("- 语言：{lang}"),
        files: files.iter().map(|(f, _)| f.to_string()).collect(),
    };
    project_context::ensure_context(&project_dir, &ctx)?;
    eprintln!("[create_project] 上下文 PROJECT.md 已建立");

    // 步骤 4：git init（失败静默降级）
    let git_done = git_init && !already_in_git_repo(&project_dir) && run_cmd("git", &["init"], &project_dir);

    // 步骤 5：登记注册表
    register_project(name, &project_dir.to_string_lossy(), lang)?;

    // 汇报树
    let mut tree = vec!["├─ src/".to_string(), "├─ config/".to_string()];
    for (i, (rel, _)) in files.iter().enumerate() {
        if rel.starts_with("src/") || rel.starts_with("config/") {
            continue;
        }
        let branch = if i == files.len() - 1 { "└─" } else { "├─" };
        tree.push(format!("{branch} {rel}"));
    }
    tree.push("└─ PROJECT.md".to_string());
    let run_hint = match lang {
        "python" => format!("cd {project} && python3 src/main.py", project = project_dir.display()),
        "node" => format!("cd {project} && npm start", project = project_dir.display()),
        "rust" => format!("cd {project} && cargo run", project = project_dir.display()),
        _ => format!("用浏览器打开 {project}/index.html", project = project_dir.display()),
    };
    let git_text = if git_done { "、已初始化 git" } else { "" };
    Ok(format!(
        "工程已创建：{dir}\n{tree}\n上下文 PROJECT.md 已建立{git_text}\n已登记工程注册表。\n下一步：{run_hint}",
        dir = project_dir.display(),
        tree = tree.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    fn tmp_home() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "qibai_proj_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn validate_name_rejects_injection() {
        assert!(validate_name("my-app_1").is_ok());
        assert!(validate_name("../evil").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("中文名字").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn build_plan_contains_files_and_steps() {
        let plan = build_plan("myapp", "python", "实现天气预报", true);
        assert!(plan.contains("myapp"));
        assert!(plan.contains("src/main.py"));
        assert!(plan.contains("PROJECT.md"));
        assert!(plan.contains("确认后即开始创建"));
    }

    #[test]
    fn scaffold_all_langs_have_structure() {
        for lang in LANGS {
            let files = scaffold_files("app", lang, "目标");
            let names: Vec<&str> = files.iter().map(|(f, _)| *f).collect();
            assert!(names.contains(&".gitignore"), "{lang} 缺 .gitignore");
            assert!(names.iter().any(|f| f.starts_with("src/")), "{lang} 缺 src/");
            assert!(names.iter().any(|f| f.starts_with("config/")), "{lang} 缺 config/");
            assert!(names.contains(&"README.md"), "{lang} 缺 README");
        }
        // python 特有
        let py = scaffold_files("app", "python", "g");
        assert!(py.iter().any(|(f, _)| *f == "requirements.txt"));
        // rust 特有
        let rs = scaffold_files("app", "rust", "g");
        assert!(rs.iter().any(|(f, _)| *f == "Cargo.toml"));
    }

    #[tokio::test]
    async fn perform_creates_full_structure_and_context() {
        let _g = env_lock();
        let data = std::env::temp_dir().join(format!(
            "qibai_pjdata_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&data).unwrap();
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        let home = tmp_home();
        let h = home.to_str().unwrap();
        let out = perform_create_project("demo", "~", "python", "实现 demo", false, h).unwrap();
        assert!(out.contains("工程已创建"));
        let dir = home.join("demo");
        assert!(dir.join("src/main.py").is_file());
        assert!(dir.join("requirements.txt").is_file());
        assert!(dir.join("config/settings.toml").is_file());
        assert!(dir.join("README.md").is_file());
        assert!(dir.join(".gitignore").is_file());
        let ctx = project_context::load_context(&dir).unwrap();
        assert!(ctx.goal.contains("demo"));
        assert!(ctx.files.contains(&"src/main.py".to_string()));
        // 注册表已登记（隔离目录内）
        assert_eq!(recent_project().unwrap().name, "demo");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn perform_rejects_existing_dir_and_bad_name() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        std::fs::create_dir_all(home.join("exists")).unwrap();
        assert!(perform_create_project("exists", "~", "web", "g", false, h).is_err());
        assert!(perform_create_project("../x", "~", "web", "g", false, h).is_err());
        assert!(perform_create_project("ok", "/etc", "web", "g", false, h).is_err()); // parent 越界
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn registry_register_recent_empty() {
        let _g = env_lock();
        let tmp = std::env::temp_dir().join(format!(
            "qibai_reg_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        // 空注册表 → None
        assert!(recent_project().is_none());
        // 登记两个 → 最近命中
        register_project("a", "/home/u/a", "python").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        register_project("b", "/home/u/b", "web").unwrap();
        let recent = recent_project().unwrap();
        assert_eq!(recent.name, "b");
        // touch 更新时间
        touch_project("/home/u/a");
        assert_eq!(recent_project().unwrap().name, "a");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 脚手架冒烟（默认跑）：python main.py 语法编译 + node package.json JSON 合法。
    #[test]
    fn smoke_scaffold_templates_compile() {
        let _g = env_lock();
        let data = std::env::temp_dir().join(format!(
            "qibai_pjsm_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&data).unwrap();
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        let home = tmp_home();
        let h = home.to_str().unwrap();
        perform_create_project("smpy", "~", "python", "冒烟", false, h).unwrap();
        // python3 -m py_compile
        let out = std::process::Command::new("python3")
            .args(["-m", "py_compile"])
            .arg(home.join("smpy/src/main.py"))
            .output()
            .unwrap();
        assert!(out.status.success(), "py_compile 失败: {}", String::from_utf8_lossy(&out.stderr));

        perform_create_project("smnode", "~", "node", "冒烟", false, h).unwrap();
        let pkg = std::fs::read_to_string(home.join("smnode/package.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&pkg).expect("package.json 应合法 JSON");
        assert_eq!(v["scripts"]["start"], "node src/index.js");

        // rust 模板文件齐全
        perform_create_project("smrust", "~", "rust", "冒烟", false, h).unwrap();
        assert!(home.join("smrust/Cargo.toml").is_file());
        assert!(home.join("smrust/src/main.rs").is_file());
        // web index.html
        perform_create_project("smweb", "~", "web", "冒烟", false, h).unwrap();
        assert!(home.join("smweb/index.html").is_file());
        let html = std::fs::read_to_string(home.join("smweb/index.html")).unwrap();
        assert!(html.contains("</html>"));
        // 四个工程都在注册表中
        assert_eq!(recent_project().unwrap().name, "smweb");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&data);
    }
}
