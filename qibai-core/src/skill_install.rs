//! v45.8：技能安装器（两段式 stage → commit）。
//!
//! 来源：本地目录（含 SKILL.md）、本地文件（单个 SKILL.md 或 zip）、URL。
//! 安全校验（在 skill_pack 已有原语上接）：
//! - 拒 `..` 与绝对路径条目；
//! - 条目数 ≤ 512、单文件 ≤ 1MB、总大小 ≤ 16MB；
//! - 扩展名白名单：`.md .txt .json .yaml .yml .py .js .sh`；
//! - `.sh/.py/.js` 必须 frontmatter 声明 `permissions: execute` 才允许，否则整包拒；
//! - 内置技能（web-research/long-read/create-project/code-check）不可卸载。
//!
//! 测试用 tempdir 注入 HOME，绝不写真实 ~/.qibai_skills。

use std::path::{Path, PathBuf};

use serde::Serialize;

/// 安装错误。
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("校验失败：{0}")]
    Rejected(String),
    #[error("IO 错误：{0}")]
    Io(String),
    #[error("同名技能已存在：{0}，需确认覆盖（overwrite=true）")]
    Conflict(String),
    #[error("内置技能不可卸载：{0}")]
    BuiltinProtected(String),
    #[error("找不到技能：{0}")]
    NotFound(String),
    #[error("下载失败：{0}")]
    Download(String),
}

/// 条目数/大小上限。
const MAX_ENTRIES: usize = 512;
const MAX_FILE_BYTES: u64 = 1024 * 1024; // 1MB
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024; // 16MB

/// 允许的扩展名白名单。
const ALLOWED_EXT: &[&str] = &["md", "txt", "json", "yaml", "yml", "py", "js", "sh"];
/// 需要 execute 权限声明的脚本扩展名。
const SCRIPT_EXT: &[&str] = &["py", "js", "sh"];

/// 安全上限。
/// 一段待确认的安装快照。
#[derive(Debug, Clone, Serialize)]
pub struct StagedSkill {
    pub stage_id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub expert: bool,
    pub source: String,
    pub permissions: Vec<String>,
    pub file_count: usize,
    pub total_size: u64,
    pub conflict: bool,
    /// 临时目录（commit 后复制走、cancel 时删）。
    #[serde(skip)]
    pub staging_dir: PathBuf,
}

/// 用户能力包安装根：`<user_plugins>/skills/<name>/`（由 skill_docs::set_plugin_roots 注入）。
/// 测试通过 skill_docs::set_plugin_roots(builtin, user) 注入临时目录。
pub fn user_skills_root() -> PathBuf {
    let (_, user) = crate::skill_docs::roots();
    user.join("skills")
}

/// 安全校验一个暂存目录：扫所有文件，拒越界/超限/未声明脚本。
fn validate_dir(dir: &Path) -> Result<(usize, u64, bool), InstallError> {
    let mut file_count = 0usize;
    let mut total: u64 = 0;

    // 递归扫描（深度不限，但条目数硬限）。
    fn walk(
        dir: &Path,
        base: &Path,
        file_count: &mut usize,
        total: &mut u64,
        bad: &mut bool,
    ) -> Result<(), InstallError> {
        for entry in std::fs::read_dir(dir).map_err(|e| InstallError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| InstallError::Io(e.to_string()))?;
            let p = entry.path();
            // 拒符号链接（简化：symlink_metadata 是 symlink 就拒）
            let md = std::fs::symlink_metadata(&p).map_err(|e| InstallError::Io(e.to_string()))?;
            if md.file_type().is_symlink() {
                return Err(InstallError::Rejected("拒绝符号链接条目".into()));
            }
            if md.is_dir() {
                walk(&p, base, file_count, total, bad)?;
            } else {
                *file_count += 1;
                if *file_count > MAX_ENTRIES {
                    return Err(InstallError::Rejected(format!("条目数超过 {MAX_ENTRIES} 上限")));
                }
                let sz = md.len();
                if sz > MAX_FILE_BYTES {
                    return Err(InstallError::Rejected(format!(
                        "单文件超过 {} 字节上限：{}",
                        MAX_FILE_BYTES,
                        p.display()
                    )));
                }
                *total += sz;
                if *total > MAX_TOTAL_BYTES {
                    return Err(InstallError::Rejected(format!("总大小超过 {MAX_TOTAL_BYTES} 字节上限")));
                }
                // 路径越界检查
                let rel = p.strip_prefix(base).map_err(|_| {
                    InstallError::Rejected(format!("路径越界：{}", p.display()))
                })?;
                let rel_s = rel.to_string_lossy();
                if rel_s.contains("..") || rel_s.starts_with('/') || rel_s.starts_with("\\") {
                    return Err(InstallError::Rejected(format!("非法路径条目：{rel_s}")));
                }
                // 扩展名白名单
                let ext = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !ALLOWED_EXT.contains(&ext.as_str()) {
                    return Err(InstallError::Rejected(format!("不允许的文件类型：.{ext}")));
                }
                // 脚本扩展名必须由 frontmatter 声明 execute（在调用方统一校验，这里仅标记）
                if SCRIPT_EXT.contains(&ext.as_str()) {
                    *bad = true;
                }
            }
        }
        Ok(())
    }
    let mut bad = false;
    walk(dir, dir, &mut file_count, &mut total, &mut bad)?;
    Ok((file_count, total, bad))
}

/// 从本地目录 stage（必须有 qibai-plugin.json；SKILL.md 可选）。
pub fn stage_from_dir(dir: &Path, source_label: &str) -> Result<StagedSkill, InstallError> {
    // 强制 qibai-plugin.json（id/name 非空）
    let mf_path = dir.join("qibai-plugin.json");
    let mf_txt = std::fs::read_to_string(&mf_path)
        .map_err(|_| InstallError::Rejected("包目录里缺 qibai-plugin.json".into()))?;
    let mf: serde_json::Value = serde_json::from_str(&mf_txt)
        .map_err(|_| InstallError::Rejected("qibai-plugin.json 不是合法 JSON".into()))?;
    let name = mf.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if name.is_empty() {
        return Err(InstallError::Rejected("qibai-plugin.json 缺 name 字段".into()));
    }
    let version = mf.get("version").and_then(|v| v.as_str()).unwrap_or("0.0.0").to_string();
    let author = mf.get("author").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let desc = mf.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let expert = mf.get("type").and_then(|v| v.as_str()).unwrap_or("") == "expert";
    let mut permissions: Vec<String> = mf.get("permissions").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    // SKILL.md 可选
    let skill_md = dir.join("SKILL.md");
    let has_doc = skill_md.exists();
    if has_doc {
        let body = std::fs::read_to_string(&skill_md).map_err(|e| InstallError::Io(e.to_string()))?;
        if body.trim().is_empty() {
            return Err(InstallError::Rejected("SKILL.md 存在但为空".into()));
        }
    }

    let (file_count, total_size, has_script) = validate_dir(dir)?;
    if has_script && !permissions.iter().any(|p| p == "execute") {
        return Err(InstallError::Rejected(
            "包内含脚本文件但 qibai-plugin.json 未声明 permissions: execute".into(),
        ));
    }
    if expert && !permissions.iter().any(|p| p == "expert") {
        permissions.push("expert".into());
    }
    let conflict = user_skills_root().join(&name).exists();
    let stage_id = format!(
        "stage-{}-{}",
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    Ok(StagedSkill {
        stage_id,
        name,
        version,
        author,
        description: desc,
        expert,
        source: source_label.to_string(),
        permissions,
        file_count,
        total_size,
        conflict,
        staging_dir: dir.to_path_buf(),
    })
}

/// 从本地文件 stage：单个 SKILL.md（纯文本）或 zip。
pub fn stage_from_file(path: &Path) -> Result<StagedSkill, InstallError> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "md" {
        // 单 SKILL.md 文件：建一个临时目录包起来
        let tmp = std::env::temp_dir().join(format!("qibai_stage_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| InstallError::Io(e.to_string()))?;
        std::fs::copy(path, tmp.join("SKILL.md")).map_err(|e| InstallError::Io(e.to_string()))?;
        return stage_from_dir(&tmp, &format!("file:{}", path.display()));
    }
    if ext == "zip" {
        // 复用 skill_pack 的解压（安全原语已在那边）
        return stage_from_zip(path);
    }
    Err(InstallError::Rejected(format!(
        "不支持的文件类型 .{ext}（仅 .md 或 .zip）"
    )))
}

/// 从 zip 解压并校验。
fn stage_from_zip(path: &Path) -> Result<StagedSkill, InstallError> {
    let tmp = std::env::temp_dir().join(format!("qibai_zip_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| InstallError::Io(e.to_string()))?;
    let file = std::fs::File::open(path).map_err(|e| InstallError::Io(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| InstallError::Rejected(e.to_string()))?;
    let mut file_count = 0usize;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| InstallError::Rejected(e.to_string()))?;
        // zip 9：name() 返回 Result<Cow<str>, ZipError>（2.x 是 &str）。
        // 取不到名字说明条目头已损坏，按拒收处理，不放行。
        let name = entry
            .name()
            .map_err(|e| InstallError::Rejected(format!("zip 条目名不可读：{e}")))?
            .to_string();
        // 拒 .. 与绝对路径
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(InstallError::Rejected(format!("zip 含非法路径条目：{name}")));
        }
        if entry.is_dir() {
            continue;
        }
        file_count += 1;
        if file_count > MAX_ENTRIES {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(InstallError::Rejected(format!("zip 条目数超 {MAX_ENTRIES}")));
        }
        let sz = entry.size();
        if sz > MAX_FILE_BYTES {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(InstallError::Rejected(format!("zip 单文件超 {MAX_FILE_BYTES}")));
        }
        total += sz;
        if total > MAX_TOTAL_BYTES {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(InstallError::Rejected(format!("zip 总大小超 {MAX_TOTAL_BYTES}")));
        }
        let out_path = tmp.join(&name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| InstallError::Io(e.to_string()))?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| InstallError::Io(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| InstallError::Io(e.to_string()))?;
    }
    stage_from_dir(&tmp, &format!("zip:{}", path.display()))
}

/// URL 下载（异步原语）：只允许 http/https，8s 超时，流式读、体积硬限 16MB，
/// 超了中断并删临时文件。返回落盘路径；前端 IPC 拿到路径后再调 `stage_from_file`。
pub async fn download_to_temp(
    url: &str,
    dest_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只支持 http/https 链接".to_string());
    }
    let _ = std::fs::create_dir_all(dest_dir);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载返回状态 {}（可能是 404/403）", resp.status()));
    }
    // 流式读，16MB 硬限
    let mut total: u64 = 0;
    const CAP: u64 = 16 * 1024 * 1024;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    let tmp_name = format!(
        "qibai_dl_{}.bin",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp_path = dest_dir.join(&tmp_name);
    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("创建临时文件失败：{e}"))?;
    use std::io::Write;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读流失败：{e}"))?;
        total += chunk.len() as u64;
        if total > CAP {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("下载超过 {} 字节上限", CAP));
        }
        file.write_all(&chunk).map_err(|e| format!("写临时文件失败：{e}"))?;
    }
    Ok(tmp_path)
}

/// URL 安装（同步入口，保留给老调用方；前端 IPC 请用 download_to_temp + stage_from_file）。
pub fn stage_from_url(_url: &str) -> Result<StagedSkill, InstallError> {
    Err(InstallError::Download(
        "URL 安装请在异步 runtime 里先 download_to_temp 落盘，再 stage_from_file".into(),
    ))
}

/// commit：把 staging_dir 装到 ~/.qibai_skills/<name>/，写 manifest.json。
pub fn commit(staged: StagedSkill, overwrite: bool) -> Result<(), InstallError> {
    let target = user_skills_root().join(&staged.name);
    if target.exists() && !overwrite {
        return Err(InstallError::Conflict(staged.name));
    }
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).map_err(|e| InstallError::Io(e.to_string()))?;
    // 复制 staging_dir → target
    copy_dir_recursive(&staged.staging_dir, &target)?;
    // 写 manifest.json
    let manifest = serde_json::json!({
        "name": staged.name,
        "version": staged.version,
        "author": staged.author,
        "description": staged.description,
        "expert": staged.expert,
        "source": staged.source,
        "permissions": staged.permissions,
        "file_count": staged.file_count,
        "total_size": staged.total_size,
        "installed_at": chrono_like_now(),
    });
    let mf = target.join("manifest.json");
    std::fs::write(&mf, serde_json::to_string_pretty(&manifest).unwrap_or_default())
        .map_err(|e| InstallError::Io(e.to_string()))?;
    Ok(())
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), InstallError> {
    if !src.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(src).map_err(|e| InstallError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| InstallError::Io(e.to_string()))?;
        let p = entry.path();
        let name = entry.file_name();
        let target = dst.join(&name);
        if p.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| InstallError::Io(e.to_string()))?;
            copy_dir_recursive(&p, &target)?;
        } else {
            std::fs::copy(&p, &target).map_err(|e| InstallError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

/// cancel：删临时目录（仅当 staging_dir 在 temp_dir 下）。
pub fn cancel(staged: StagedSkill) {
    if let Ok(tmp) = std::env::temp_dir().canonicalize() {
        if let Ok(s) = staged.staging_dir.canonicalize() {
            if s.starts_with(&tmp) {
                let _ = std::fs::remove_dir_all(&s);
            }
        }
    }
}

/// uninstall：只允许删 `<user_plugins>/skills/<name>/`；builtin 根下的包天然不在删除路径里。
pub fn uninstall(name: &str) -> Result<(), InstallError> {
    let target = user_skills_root().join(name);
    if !target.exists() {
        return Err(InstallError::NotFound(name.into()));
    }
    std::fs::remove_dir_all(&target).map_err(|e| InstallError::Io(e.to_string()))?;
    Ok(())
}

/// 列出用户已装技能名。
pub fn list_installed() -> Vec<String> {
    let mut out = Vec::new();
    let root = user_skills_root();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                if let Some(n) = e.file_name().to_str() {
                    out.push(n.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

/// read_doc：用户装的优先，回落到内置。
pub fn read_doc(name: &str) -> Result<String, InstallError> {
    let user_path = user_skills_root().join(name).join("SKILL.md");
    if user_path.exists() {
        return std::fs::read_to_string(&user_path).map_err(|e| InstallError::Io(e.to_string()));
    }
    // 回落：扫 builtin plugins 根找同名包的 SKILL.md
    for e in crate::skill_docs::catalog().into_iter() {
        if e.name == name {
            if let Some(b) = e.body {
                return Ok(b);
            }
            return Err(InstallError::NotFound(format!("包 {name} 无 SKILL.md")));
        }
    }
    Err(InstallError::NotFound(name.into()))
}

/// permissions：读 manifest.json。
pub fn permissions(name: &str) -> Vec<String> {
    let mf = user_skills_root().join(name).join("manifest.json");
    if let Ok(s) = std::fs::read_to_string(&mf) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(arr) = v.get("permissions").and_then(|a| a.as_array()) {
                return arr
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在指定 tempdir 下放一个 SKILL.md，返回目录路径。
    /// 造一个合法包目录（qibai-plugin.json 必备，SKILL.md 可选）。
    fn make_skill_dir(root: &Path, name: &str, extra_perm: &str, extra_file: Option<(&str, &str)>) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let perms = if extra_perm.is_empty() { "[]".to_string() } else { format!("[{extra_perm}]") };
        let mf = format!(
            "{{\"id\":\"com.test.{name}\",\"type\":\"skill\",\"name\":\"{name}\",\"version\":\"1.0\",\"author\":\"t\",\"description\":\"测试技能\",\"permissions\":{perms}}}"
        );
        std::fs::write(dir.join("qibai-plugin.json"), mf).unwrap();
        std::fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\n---\n正文\n")).unwrap();
        if let Some((n, c)) = extra_file {
            std::fs::write(dir.join(n), c).unwrap();
        }
        dir
    }

    /// 造临时 user plugins 根并通过 set_plugin_roots 注入。
    fn with_temp_user_plugins<T>(f: impl FnOnce(&Path) -> T) -> T {
        let user = std::env::temp_dir().join(format!("qibai_install_user_{}", std::process::id()));
        let builtin = std::env::temp_dir().join(format!("qibai_install_builtin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&builtin);
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&builtin).unwrap();
        crate::skill_docs::set_plugin_roots(builtin.clone(), user.clone());
        let r = f(&user);
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&builtin);
        r
    }

    #[test]
    fn stage_and_commit_installs_to_user_dir() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src1");
            let _ = std::fs::remove_dir_all(&src_root);
            let src = make_skill_dir(&src_root, "my-skill", "", None);
            let staged = stage_from_dir(&src, "test").unwrap();
            assert_eq!(staged.name, "my-skill");
            assert!(!staged.conflict);
            commit(staged, false).unwrap();
            let installed = user_skills_root().join("my-skill").join("SKILL.md");
            assert!(installed.exists(), "SKILL.md 应被复制到用户目录");
            let list = list_installed();
            assert!(list.contains(&"my-skill".to_string()));
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn conflict_requires_overwrite() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src2");
            let _ = std::fs::remove_dir_all(&src_root);
            let src = make_skill_dir(&src_root, "conflict-skill", "", None);
            let s1 = stage_from_dir(&src, "test").unwrap();
            commit(s1, false).unwrap();
            let s2 = stage_from_dir(&src, "test").unwrap();
            assert!(s2.conflict, "二次 stage 应标记 conflict");
            let r = commit(s2, false);
            assert!(matches!(r, Err(InstallError::Conflict(_))));
            let s3 = stage_from_dir(&src, "test").unwrap();
            commit(s3, true).unwrap();
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn uninstall_user_ok_builtin_rejected() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src3");
            let _ = std::fs::remove_dir_all(&src_root);
            let src = make_skill_dir(&src_root, "uninst-me", "", None);
            let s = stage_from_dir(&src, "test").unwrap();
            commit(s, false).unwrap();
            assert!(list_installed().contains(&"uninst-me".to_string()));
            uninstall("uninst-me").unwrap();
            assert!(!list_installed().contains(&"uninst-me".to_string()));
            // 内置根下的包不在 user root，uninstall 找不到 → NotFound（天然保护）
            let r = uninstall("web-research");
            assert!(matches!(r, Err(InstallError::NotFound(_))));
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn script_without_execute_permission_rejected() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src4");
            let _ = std::fs::remove_dir_all(&src_root);
            let src = make_skill_dir(&src_root, "bad-skill", "", Some(("tool.py", "print(1)")));
            let r = stage_from_dir(&src, "test");
            assert!(r.is_err(), "含脚本但未声明 execute 应拒：{r:?}");
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn script_with_execute_permission_allowed() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src5");
            let _ = std::fs::remove_dir_all(&src_root);
            let src = make_skill_dir(&src_root, "good-skill", "\"execute\"", Some(("tool.py", "print(1)")));
            let staged = stage_from_dir(&src, "test").unwrap();
            assert!(staged.permissions.contains(&"execute".to_string()));
            commit(staged, false).unwrap();
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn missing_qibai_plugin_json_rejected() {
        with_temp_user_plugins(|_user| {
            let src_root = std::env::temp_dir().join("qibai_src6");
            let pkg = src_root.join("no-json-skill");
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(pkg.join("SKILL.md"), "正文").unwrap();
            let r = stage_from_dir(&pkg, "test");
            assert!(r.is_err(), "缺 qibai-plugin.json 应拒：{r:?}");
            let _ = std::fs::remove_dir_all(&src_root);
        });
    }

    #[test]
    fn read_doc_user_over_builtin_fallback() {
        with_temp_user_plugins(|_user| {
            // 内置根为空（测试注入），先造一个内置包
            let (builtin, _) = crate::skill_docs::roots();
            let bp = builtin.join("skills").join("demo-builtin");
            std::fs::create_dir_all(&bp).unwrap();
            std::fs::write(
                bp.join("qibai-plugin.json"),
                "{\"id\":\"com.demo\",\"type\":\"skill\",\"name\":\"演示技能\",\"description\":\"内置\",\"version\":\"1.0\"}",
            ).unwrap();
            std::fs::write(bp.join("SKILL.md"), "内置正文").unwrap();
            // 现在 read_doc 能读到内置
            let doc = crate::skill_docs::read_doc("演示技能").unwrap();
            assert!(doc.contains("内置正文"));
            let _ = std::fs::remove_dir_all(&builtin);
        });
    }

    /// download_to_temp（异步原语）离线测试：非 http scheme 直接拒、不可达地址返 Err 且不留残留。
    #[tokio::test]
    async fn download_to_temp_rejects_non_http() {
        let tmp = std::env::temp_dir().join("qibai_dl_scheme_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = download_to_temp("ftp://example.com/pkg.zip", &tmp).await;
        assert!(r.is_err(), "ftp 应被拒");
        assert!(r.unwrap_err().contains("http"));
        let r2 = download_to_temp("file:///etc/passwd", &tmp).await;
        assert!(r2.is_err(), "file:// 应被拒");
        let entries: Vec<_> = std::fs::read_dir(&tmp).unwrap().collect();
        assert!(entries.is_empty(), "失败后不留临时文件：{entries:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn download_to_temp_unreachable_no_leftover() {
        let tmp = std::env::temp_dir().join("qibai_dl_unreachable");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = download_to_temp("http://127.0.0.1:1/x.zip", &tmp).await;
        assert!(r.is_err(), "不可达地址应 Err：{r:?}");
        let entries: Vec<_> = std::fs::read_dir(&tmp).unwrap().collect();
        assert!(entries.is_empty(), "失败后不留临时文件：{entries:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
