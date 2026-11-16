//! v45.9：统一能力包扫描器。
//!
//! 能力包只有一套：`<root>/{skills,experts}/<pkg>/`，必备 `qibai-plugin.json`，
//! `SKILL.md` 可选（缺则 has_doc=false，正文空，不报错）。
//!
//! 两段 root 由 main.rs 启动时通过 `set_plugin_roots` 注入：
//! - builtin：`<src-tauri>/plugins`（随二进制分发）
//! - user：`<user_data>/plugins`（用户安装/覆盖）
//!
//! 同名时 user 覆盖 builtin。`.disabled/<name>` 空文件 = 停用（不注入 prompt，但
//! read_doc 仍可读）。

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// 一条能力包目录项。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillDocEntry {
    pub name: String,
    pub description: String,
    /// 正文（SKILL.md 全文）；无 SKILL.md 时为 None。
    pub body: Option<String>,
    /// "doc"（技能） | "expert"（专家）。
    pub kind: &'static str,
    /// "builtin" | "user"。
    pub source: &'static str,
    /// 包目录绝对路径。
    pub pkg_dir: String,
    /// 是否有 SKILL.md。
    pub has_doc: bool,
    /// 加载失败原因（成功为 None）。
    pub load_error: Option<String>,
}

thread_local! {
    /// 线程内的 roots 覆盖（v45.31-d）。
    ///
    /// 仓库里有一批测试要换 builtin / user 两段根；此前只能走进程级 `set_plugin_roots`，
    /// 而那是 OnceLock 语义——**只有第一次生效**，后续调用被静默忽略，多个测试并行时
    /// 会共用最先设置的目录、再被彼此 `remove_dir_all` 掉（表现为「找不到能力包」）。
    /// 线程内覆盖让每个测试各用各的目录，互不干扰。
    static L_ROOT: std::cell::RefCell<Option<(PathBuf, PathBuf)>> =
        std::cell::RefCell::new(None);
}

fn default_roots() -> (PathBuf, PathBuf) {
    let tmp = std::env::temp_dir().join("qibai_plugins_unset");
    (tmp.clone(), tmp)
}

pub fn roots() -> (PathBuf, PathBuf) {
    if let Some(r) = L_ROOT.with(|c| c.borrow().clone()) {
        return r;
    }
    R_SET_ONCE
        .get_or_init(|| RwLock::new(default_roots()))
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 注入 builtin / user 两段 plugins 根。生产由 main.rs 启动时调用一次；
/// 测试请改用 `with_plugin_roots`，避免进程级状态互相踩。
pub fn set_plugin_roots(builtin: PathBuf, user: PathBuf) {
    let lock = R_SET_ONCE.get_or_init(|| RwLock::new(default_roots()));
    *lock.write().unwrap_or_else(|e| e.into_inner()) = (builtin, user);
}

static R_SET_ONCE: OnceLock<RwLock<(PathBuf, PathBuf)>> = OnceLock::new();

/// 在当前线程内把 roots 临时指向给定目录，作用域结束后自动恢复（panic 也恢复）。
pub fn with_plugin_roots<T>(builtin: PathBuf, user: PathBuf, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<(PathBuf, PathBuf)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            L_ROOT.with(|c| *c.borrow_mut() = prev);
        }
    }
    let prev = L_ROOT.with(|c| c.replace(Some((builtin, user))));
    let _restore = Restore(prev);
    f()
}

/// 读 qibai-plugin.json，取 name / description / type。
fn read_manifest(pkg_dir: &Path) -> Result<(String, String, String), String> {
    let p = pkg_dir.join("qibai-plugin.json");
    let txt = std::fs::read_to_string(&p).map_err(|e| format!("读 qibai-plugin.json 失败：{e}"))?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("qibai-plugin.json 解析失败：{e}"))?;
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if name.is_empty() {
        return Err(format!("qibai-plugin.json 缺 name：{id}"));
    }
    let desc = v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("skill").to_string();
    Ok((name, desc, ty))
}

/// 扫一段 root：约定目录 skills/ experts/ + 根下平铺包（安装器落盘位置）。
///
/// 安装/导入走 `plugins/<id>/`（平铺），若只扫 `skills/ experts/`，用户装好的包
/// 会出现在插件注册表里却不出现在技能目录中——所以两处都扫。
fn scan_root(root: &Path, source: &'static str, out: &mut Vec<SkillDocEntry>) {
    for (sub, kind) in [("skills", "doc"), ("experts", "expert")] {
        if let Ok(rd) = std::fs::read_dir(root.join(sub)) {
            for entry in rd.flatten() {
                scan_pkg(entry.path(), kind, source, out, false);
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), "skills" | "experts" | ".disabled") {
                continue;
            }
            scan_pkg(entry.path(), "doc", source, out, true);
        }
    }
}

/// 扫单个包目录；没有 qibai-plugin.json 的目录直接跳过（平铺扫描会撞上杂目录）。
fn scan_pkg(
    pkg_dir: PathBuf,
    kind: &'static str,
    source: &'static str,
    out: &mut Vec<SkillDocEntry>,
    skip_no_manifest: bool,
) {
    if !pkg_dir.is_dir() {
        return;
    }
    // 平铺扫描会撞上杂目录：没有清单就直接跳过；约定目录里缺清单仍记 load_error（便于诊断）。
    if skip_no_manifest && !pkg_dir.join("qibai-plugin.json").is_file() {
        return;
    }
    let folder = pkg_dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    match read_manifest(&pkg_dir) {
        Ok((name, desc, _ty)) => {
            let sk = pkg_dir.join("SKILL.md");
            let (body, has_doc) = match std::fs::read_to_string(&sk) {
                Ok(b) => (Some(b), true),
                Err(_) => (None, false),
            };
            out.push(SkillDocEntry {
                name,
                description: desc,
                body,
                kind,
                source,
                pkg_dir: pkg_dir.display().to_string(),
                has_doc,
                load_error: None,
            });
        }
        Err(e) => {
            eprintln!("[qibai] skill: load failed {folder} {e}");
            out.push(SkillDocEntry {
                name: folder,
                description: String::new(),
                body: None,
                kind,
                source,
                pkg_dir: pkg_dir.display().to_string(),
                has_doc: false,
                load_error: Some(e),
            });
        }
    }
}

/// 合并 builtin + user（user 同名覆盖 builtin）。
pub fn catalog() -> Vec<SkillDocEntry> {
    let (b, u) = roots();
    catalog_with(&b, &u)
}

/// 测试注入版本：显式传两段根，绕开 OnceLock。
#[doc(hidden)]
pub fn catalog_with(builtin: &Path, user: &Path) -> Vec<SkillDocEntry> {
    let mut builtin_items = Vec::new();
    scan_root(builtin, "builtin", &mut builtin_items);
    let mut user_items = Vec::new();
    scan_root(user, "user", &mut user_items);
    let mut map: std::collections::BTreeMap<String, SkillDocEntry> = std::collections::BTreeMap::new();
    for e in builtin_items.into_iter() {
        map.insert(e.name.clone(), e);
    }
    for e in user_items.into_iter() {
        map.insert(e.name.clone(), e);
    }
    map.into_values().collect()
}

fn disabled_marker_path(name: &str) -> Option<PathBuf> {
    let (_, user) = roots();
    Some(user.join(".disabled").join(name))
}

pub fn is_disabled(name: &str) -> bool {
    disabled_marker_path(name).map(|p| p.exists()).unwrap_or(false)
}

pub fn set_enabled(name: &str, enabled: bool) -> Result<(), String> {
    let marker = disabled_marker_path(name).ok_or("路径不可用")?;
    if enabled {
        let _ = std::fs::remove_file(&marker);
    } else {
        let parent = marker.parent().ok_or("路径异常")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        std::fs::write(&marker, b"").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 读某包 SKILL.md 正文（user 优先，回落 builtin；缺则 NotFound）。
pub fn read_doc(name: &str) -> Result<String, String> {
    for e in catalog().into_iter() {
        if e.name == name {
            if let Some(b) = e.body {
                return Ok(b);
            }
            return Err(format!("包 {name} 没有 SKILL.md（qibai-plugin.json 存在但无文档）"));
        }
    }
    Err(format!("找不到能力包：{name}"))
}

/// 渲染 system prompt 一段（只列名 + 一句话描述，不粘正文）。
pub fn render_catalog_prompt() -> String {
    let items = catalog();
    if items.is_empty() {
        return String::new();
    }
    let mut s = String::from("【可用技能/专家（按名调用对应 SKILL.md 步骤）】\n");
    for e in items.iter().filter(|e| !is_disabled(&e.name)) {
        let tag = if e.kind == "expert" { "专家" } else { "技能" };
        s.push_str(&format!("- [{}] {}：{}\n", tag, e.name, e.description));
    }
    s.push_str("需要时用 skill_read 按名加载正文步骤执行；没装的不要硬编。\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pkg(root: &Path, sub: &str, folder: &str, name: &str, desc: &str, with_doc: bool) {
        let pkg = root.join(sub).join(folder);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("qibai-plugin.json"),
            format!("{{\"id\":\"com.test.{name}\",\"type\":\"{sub}\",\"name\":\"{name}\",\"description\":\"{desc}\"}}"),
        ).unwrap();
        if with_doc {
            std::fs::write(pkg.join("SKILL.md"), format!("---\nname: {name}\n---\n正文 {name}\n")).unwrap();
        }
    }

    #[test]
    fn scan_skills_and_experts() {
        let b = std::env::temp_dir().join(format!("qibai_pkg_b_{}", std::process::id()));
        let u = std::env::temp_dir().join(format!("qibai_pkg_u_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
        make_pkg(&b, "skills", "websearch", "网络搜索", "搜关键词", true);
        make_pkg(&b, "experts", "code-expert", "代码专家", "代码答疑", false);
        make_pkg(&u, "skills", "websearch", "网络搜索", "用户覆盖版", true);
        let items = catalog_with(&b, &u);
        // user 覆盖 builtin，同名只一条
        assert_eq!(items.iter().filter(|e| e.name == "网络搜索").count(), 1);
        let ws = items.iter().find(|e| e.name == "网络搜索").unwrap();
        assert_eq!(ws.source, "user");
        assert!(ws.description.contains("用户覆盖"));
        assert!(ws.has_doc);
        assert_eq!(ws.kind, "doc");
        // 专家无 SKILL.md：has_doc=false 不报错
        let ce = items.iter().find(|e| e.name == "代码专家").unwrap();
        assert!(!ce.has_doc);
        assert_eq!(ce.kind, "expert");
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn flat_installed_pack_is_listed() {
        // 安装器（skill_install / skill_pack）把包装到 `<root>/<id>/`（平铺），
        // 不套 skills/ 子目录；这层必须被扫到，否则装好的包在插件表里有、技能目录里没有。
        let b = std::env::temp_dir().join(format!("qibai_flat_b_{}", std::process::id()));
        let u = std::env::temp_dir().join(format!("qibai_flat_u_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
        make_pkg(&u, "", "animacode", "Animacode 动画成片", "写 scene.json 出片", true);
        let items = catalog_with(&b, &u);
        let ac = items
            .iter()
            .find(|e| e.name == "Animacode 动画成片")
            .expect("平铺安装的包应出现在技能目录");
        assert!(ac.has_doc, "SKILL.md 应读到");
        assert_eq!(ac.source, "user");
        // 杂目录（无清单）不产生 load_error 噪音
        std::fs::create_dir_all(u.join("some-cache")).unwrap();
        let items = catalog_with(&b, &u);
        assert!(items.iter().all(|e| e.name != "some-cache"));
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn missing_manifest_recorded_as_load_error() {
        let b = std::env::temp_dir().join(format!("qibai_pkg_bad_{}", std::process::id()));
        let u = std::env::temp_dir().join(format!("qibai_pkg_bad_u_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        std::fs::create_dir_all(b.join("skills").join("orphan")).unwrap();
        let items = catalog_with(&b, &u);
        let orphan = items.iter().find(|e| e.name == "orphan").unwrap();
        assert!(orphan.load_error.is_some(), "缺 qibai-plugin.json 应记 load_error");
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn read_doc_returns_body() {
        let b = std::env::temp_dir().join(format!("qibai_pkg_r_{}", std::process::id()));
        let u = std::env::temp_dir().join(format!("qibai_pkg_r_u_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        make_pkg(&b, "skills", "code-check", "代码检查", "只读分析", true);
        let items = catalog_with(&b, &u);
        let doc = items.iter().find(|e| e.name == "代码检查").unwrap();
        assert!(doc.body.as_ref().unwrap().contains("正文 代码检查"));
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn disable_marker_filters_prompt() {
        // v45.31-d：线程内 roots 覆盖 + 唯一目录，不再依赖进程级 OnceLock，故解除 ignore。
        // 禁用标记写在 `<user_root>/.disabled/<name>`，跟着线程覆盖走，不污染真实目录。
        let b = crate::test_support::unique_temp_dir("pkg-d");
        let u = crate::test_support::unique_temp_dir("pkg-d-user");
        make_pkg(&b, "skills", "websearch", "网络搜索", "搜", true);
        with_plugin_roots(b.clone(), u.clone(), || {
            let p1 = render_catalog_prompt();
            assert!(p1.contains("网络搜索"));
            set_enabled("网络搜索", false).unwrap();
            assert!(is_disabled("网络搜索"));
            let p2 = render_catalog_prompt();
            assert!(!p2.contains("网络搜索"));
            // 禁用后 read_doc 仍可读
            assert!(read_doc("网络搜索").is_ok());
            set_enabled("网络搜索", true).unwrap();
        });
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&u);
    }
}
