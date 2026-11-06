//! 文件/文件夹操作（安全第一，限 $HOME 沙盒）。
//!
//! 供 `file_ops` 技能调用。安全约束：
//! - 路径必须落在 $HOME 内：~ 展开 → 拒绝 `..` 穿越 → canonicalize 复查。
//! - 黑名单系统根目录（/etc /usr /var /proc …）。
//! - $HOME 根目录本身只允许 list/open，不允许 delete/rename/move。
//! - delete 不在本模块执行——由 skills.rs 分发层挂起 PendingApproval，批准后再回调本模块。

use std::fs::OpenOptions;
use std::path::{Component, Path, PathBuf};

/// 系统根目录黑名单（精确命中或作为顶层目录命中均拒绝）。
pub const BLOCKED_ROOTS: [&str; 8] = ["/etc", "/usr", "/boot", "/var", "/proc", "/sys", "/dev", "/root"];

/// 默认主目录：优先 BaseDirs（跨平台），兜底 HOME 环境变量。
pub fn default_home() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// 常用目录别名（中文 / 英文，大小写不敏感；值为主目录下的子目录名，空串表示主目录本身）。
/// 用户口语里几乎不会说 `~/Downloads`，只会说「下载」「桌面」「文档」，这里直接认。
pub static DIR_ALIASES: &[(&str, &str)] = &[
    ("下载", "Downloads"),
    ("下载文件夹", "Downloads"),
    ("下载目录", "Downloads"),
    ("downloads", "Downloads"),
    ("download", "Downloads"),
    ("桌面", "Desktop"),
    ("桌面文件夹", "Desktop"),
    ("desktop", "Desktop"),
    ("文档", "Documents"),
    ("文档文件夹", "Documents"),
    ("文稿", "Documents"),
    ("documents", "Documents"),
    ("图片", "Pictures"),
    ("照片", "Pictures"),
    ("相册", "Pictures"),
    ("pictures", "Pictures"),
    ("音乐", "Music"),
    ("music", "Music"),
    ("视频", "Videos"),
    ("videos", "Videos"),
    ("公开", "Public"),
    ("模板", "Templates"),
    ("主目录", ""),
    ("家目录", ""),
    ("根目录", ""),
    ("home", ""),
    ("~", ""),
];

/// 去掉口语里的包裹与尾巴：引号、「文件夹/目录/folder」后缀。
fn strip_oral_noise(raw: &str) -> String {
    let s = raw.trim();
    let s = s.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』' | '《' | '》'
        )
    });
    let s = s.trim().trim_end_matches(['。', '！', '!', '.', '，', ',', ' ']);
    let mut out = s.trim().to_string();
    for suf in ["文件夹", "目录", "folder", "Folder", "FOLDER"] {
        if let Some(rest) = out.strip_suffix(suf) {
            if !rest.trim().is_empty() {
                out = rest.trim().to_string();
            }
        }
    }
    out
}

/// 口语路径 → 绝对路径（不做安全校验，供 sanitize_path 与建议提示共用）。
/// 规则：`/` 开头原样；`~` 开头展开；其余先查别名，再按「主目录下的相对名」解析。
pub fn resolve_user_path(raw: &str, home: &str) -> PathBuf {
    let home_p = PathBuf::from(home.trim_end_matches('/'));
    let s = strip_oral_noise(raw);
    if s.is_empty() || s == "~" {
        return home_p;
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return if rest.trim().is_empty() {
            home_p
        } else {
            home_p.join(rest.trim())
        };
    }
    if s.starts_with('/') {
        return PathBuf::from(&s);
    }
    let lower = s.to_lowercase();
    for (k, v) in DIR_ALIASES {
        if lower == *k {
            return if v.is_empty() { home_p } else { home_p.join(v) };
        }
    }
    // 中文常把英文目录名直接说成中文以外的写法：Downloads/Desktop…
    home_p.join(&s)
}

/// 找不到路径时，按名字在主目录里做一层模糊匹配，给出最多 `limit` 条候选。
/// 只读扫描，不会创建任何东西。
pub fn suggest_similar(raw: &str, home: &str, limit: usize) -> Vec<String> {
    let key = strip_oral_noise(raw).to_lowercase();
    let key = key.rsplit('/').next().unwrap_or(&key).to_string();
    if key.is_empty() {
        return Vec::new();
    }
    // 候选关键词：原名 + 别名归一化（「下载x」里含「下载」→ 同时用 downloads 去匹配）
    let mut keys: Vec<String> = vec![key.clone()];
    for (k, v) in DIR_ALIASES {
        if v.is_empty() {
            continue;
        }
        if key.contains(k) || k.contains(&key) {
            let alt = v.to_lowercase();
            if !keys.contains(&alt) {
                keys.push(alt);
            }
        }
    }
    let hit = |name: &str| keys.iter().any(|k| name.contains(k) || k.contains(name));
    let home_p = PathBuf::from(home.trim_end_matches('/'));
    let mut out: Vec<String> = Vec::new();
    let Ok(rd) = std::fs::read_dir(&home_p) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        if hit(&name) {
            out.push(p.display().to_string());
            if out.len() >= limit {
                return out;
            }
        }
    }
    // 主目录里没有命中，再扫一层子目录（深度 1）
    for e in std::fs::read_dir(&home_p).into_iter().flatten().flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let Ok(rd2) = std::fs::read_dir(&p) else {
            continue;
        };
        for e2 in rd2.flatten() {
            let p2 = e2.path();
            if !p2.is_dir() {
                continue;
            }
            let name = e2.file_name().to_string_lossy().to_lowercase();
            if hit(&name) {
                out.push(p2.display().to_string());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// 路径消毒：返回可安全操作的绝对路径。
/// 失败一律返回中文拒绝原因（可直接朗读）。
pub fn sanitize_path(raw: &str, home: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("路径为空，告诉我要操作哪个文件或文件夹".to_string());
    }
    if raw.contains('\0') {
        return Err("路径包含非法字符".to_string());
    }
    // 1. 解析：中文别名（下载/桌面/文档…）、~ 展开、相对名按主目录解析
    let expanded = resolve_user_path(raw, home);
    // 2. 逐段重建，拒绝 .. 穿越
    let mut cur = PathBuf::new();
    for comp in expanded.components() {
        match comp {
            Component::ParentDir => return Err("路径包含 .. 穿越，已拒绝".to_string()),
            Component::CurDir => {}
            c => cur.push(c.as_os_str()),
        }
    }
    let abs = cur;
    // 3. 黑名单系统根
    let s = abs.to_string_lossy();
    if s == "/" {
        return Err("根目录 / 不允许操作".to_string());
    }
    for root in BLOCKED_ROOTS {
        if &*s == root || s.starts_with(root) && s.as_bytes().get(root.len()) == Some(&b'/') {
            return Err(format!("系统目录 {root} 不允许操作（安全限制）"));
        }
    }
    // 4. 必须在 $HOME 内（不存在的路径对父目录 canonicalize 复查，防符号链接逃逸）
    let home_norm = Path::new(home)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(home));
    let target = if abs.exists() {
        abs.canonicalize().unwrap_or_else(|_| abs.clone())
    } else {
        match abs.parent() {
            Some(p) if p.exists() => p
                .canonicalize()
                .unwrap_or_else(|_| p.to_path_buf())
                .join(abs.file_name().unwrap_or_default()),
            _ => abs.clone(),
        }
    };
    if !target.starts_with(&home_norm) {
        return Err(format!(
            "路径必须在用户主目录 {} 内，{} 越界了",
            home_norm.display(),
            target.display()
        ));
    }
    Ok(abs)
}

/// 主目录根目录本身是否命中（delete/rename/move 不允许打到 HOME 根）。
fn is_home_root(path: &Path, home: &Path) -> bool {
    path == home || path == home.canonicalize().unwrap_or_else(|_| home.to_path_buf())
}

/// 执行文件操作。action ∈ list|open|create|rename|move|copy（write/edit/delete 由分发层挂起审批）。
/// write/edit/delete 若误直接传入，这里执行后补日志；正常路径在 skills.rs 先 PendingApproval。
pub async fn run_file_op(
    action: &str,
    path: &str,
    dest: Option<&str>,
    content: Option<&str>,
    home: &str,
) -> Result<String, String> {
    // list/open 允许省略路径 = 主目录（对齐工具描述"空字符串表示主目录"）
    let arg = if path.trim().is_empty() && matches!(action, "list" | "open") {
        "~"
    } else {
        path
    };
    match action {
        "list" => op_list(arg, home),
        "open" => op_open(arg, home).await,
        "create" => op_create(path, content, home),
        "rename" | "move" => op_rename_move(action, path, dest, home),
        "copy" => op_copy(path, dest, home),
        "write" => {
            let body = content.unwrap_or("");
            write_inner(path, body, home)
        }
        "edit" => {
            let body = content.unwrap_or("");
            let mode = dest.unwrap_or("append"); // edit 的 mode 复用 dest 位（直接调用场景）
            edit_inner(path, body, mode, home)
        }
        "delete" => Err("删除操作必须先经用户批准（安全限制）".to_string()),
        other => Err(format!(
            "未知操作 {other}，支持 list/open/create/rename/move/copy/write/edit/delete"
        )),
    }
}

fn op_list(path: &str, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    if !p.is_dir() {
        return Err(format!("{} 不是文件夹，无法列出内容", p.display()));
    }
    let mut lines = Vec::new();
    for e in std::fs::read_dir(&p).map_err(|e| format!("读取文件夹失败: {e}"))? {
        let e = e.map_err(|e| format!("读取条目失败: {e}"))?;
        let ft = e.file_type().map_err(|e| format!("读取文件类型失败: {e}"))?;
        let name = e.file_name().to_string_lossy().to_string();
        let label = if ft.is_dir() {
            "目录".to_string()
        } else {
            let n = e.metadata().map(|m| m.len()).unwrap_or(0);
            format!("文件 {n} 字节")
        };
        lines.push(format!("{name} [{label}]"));
        if lines.len() >= 50 {
            break;
        }
    }
    Ok(format!(
        "文件夹 {} 下共 {} 项：\n{}",
        p.display(),
        lines.len(),
        lines.join("\n")
    ))
}

async fn op_open(path: &str, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    // 先存在性检查：xdg-open 对不存在的路径只会静默失败，之前被当成"已尝试打开"骗过用户
    if !p.exists() {
        let mut msg = format!("没找到 {}。", p.display());
        let sugg = suggest_similar(path, home, 5);
        if sugg.is_empty() {
            msg.push_str("可以说「打开下载」「打开桌面」，或者直接给我绝对路径。");
        } else {
            msg.push_str("你要找的可能是：\n");
            msg.push_str(&sugg.join("\n"));
        }
        return Err(msg);
    }
    let is_dir = p.is_dir();
    // 文件 → 打开它所在的文件夹（open 的语义是"打开文件夹"）
    let target = if is_dir {
        p.clone()
    } else {
        p.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| p.clone())
    };
    let mut cmd = {
        #[cfg(target_os = "linux")]
        {
            let mut c = tokio::process::Command::new("xdg-open");
            c.arg(&target);
            c
        }
        #[cfg(target_os = "windows")]
        {
            let mut c = tokio::process::Command::new("explorer");
            c.arg(&target);
            c
        }
        #[cfg(target_os = "macos")]
        {
            let mut c = tokio::process::Command::new("open");
            c.arg(&target);
            c
        }
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("调用系统打开命令失败（可能没装文件管理器）: {e}"))?;
    // 350ms 内就退出且非 0，说明真的没打开成功——必须如实报错，不能假成功
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    match child.try_wait() {
        Ok(Some(st)) if !st.success() => Err(format!(
            "系统没能打开 {}（退出码 {:?}），可能缺少文件管理器或没有图形会话",
            target.display(),
            st.code()
        )),
        _ => {
            if is_dir {
                Ok(format!("已打开文件夹：{}", target.display()))
            } else {
                Ok(format!(
                    "已打开所在文件夹：{}（{} 是文件，不是目录）",
                    target.display(),
                    p.display()
                ))
            }
        }
    }
}

fn op_create(path: &str, content: Option<&str>, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    if p.exists() {
        return Err(format!("{} 已存在，未覆盖（换个名字或说「覆盖」）", p.display()));
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建父文件夹失败: {e}"))?;
    }
    let body = content.unwrap_or("");
    std::fs::write(&p, body).map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(format!("已创建文件：{}（{} 字节）", p.display(), body.len()))
}

fn op_rename_move(action: &str, path: &str, dest: Option<&str>, home: &str) -> Result<String, String> {
    let from = sanitize_path(path, home)?;
    let dest_raw = dest.ok_or_else(|| format!("{action} 需要提供目标路径 dest"))?;
    let to = sanitize_path(dest_raw, home)?;
    if !from.exists() {
        return Err(format!("源路径 {} 不存在", from.display()));
    }
    if to.exists() {
        return Err(format!("目标 {} 已存在，未覆盖", to.display()));
    }
    let home_path = Path::new(home);
    if is_home_root(&to, home_path) {
        return Err("不允许重命名/移动到主目录根目录本身（安全限制）".to_string());
    }
    std::fs::rename(&from, &to).map_err(|e| format!("{} 失败: {e}", action_label(action)))?;
    Ok(format!(
        "已{}：{} → {}",
        action_label(action),
        from.display(),
        to.display()
    ))
}

fn op_copy(path: &str, dest: Option<&str>, home: &str) -> Result<String, String> {
    let from = sanitize_path(path, home)?;
    let dest_raw = dest.ok_or_else(|| "copy 需要提供目标路径 dest".to_string())?;
    let to = sanitize_path(dest_raw, home)?;
    if !from.exists() {
        return Err(format!("源路径 {} 不存在", from.display()));
    }
    if to.exists() {
        return Err(format!("目标 {} 已存在，未覆盖", to.display()));
    }
    if from.is_dir() {
        crate::skill_adapter::copy_dir_all(&from, &to).map_err(|e| format!("复制文件夹失败: {e}"))?;
    } else {
        std::fs::copy(&from, &to).map_err(|e| format!("复制文件失败: {e}"))?;
    }
    Ok(format!("已复制：{} → {}", from.display(), to.display()))
}

fn action_label(action: &str) -> &'static str {
    match action {
        "rename" => "重命名",
        "move" => "移动",
        _ => "操作",
    }
}

// ---- write / edit（LLM 生成内容落盘，走审批） ----

/// write 核心：新建文件，已存在拒绝。
fn write_inner(path: &str, content: &str, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    if p.exists() {
        return Err(format!(
            "{} 已存在，未覆盖（要改内容请用 edit，或换个文件名）",
            p.display()
        ));
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建父文件夹失败: {e}"))?;
    }
    std::fs::write(&p, content).map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(format!("已写入 {}（{} 字节）", p.display(), content.len()))
}

/// edit 核心：append 追加 / replace 全量替换；文件必须已存在且是文件。
fn edit_inner(path: &str, content: &str, mode: &str, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    if !p.exists() {
        return Err(format!("{} 不存在，无法修改（可改用 write 新建）", p.display()));
    }
    if p.is_dir() {
        return Err(format!("{} 是文件夹，不是文件，无法编辑", p.display()));
    }
    match mode {
        "append" => {
            OpenOptions::new()
                .create(false)
                .append(true)
                .open(&p)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(content.as_bytes())
                })
                .map_err(|e| format!("追加写入失败: {e}"))?;
            Ok(format!("已向 {} 追加 {} 字节", p.display(), content.len()))
        }
        "replace" => {
            std::fs::write(&p, content).map_err(|e| format!("全量替换失败: {e}"))?;
            Ok(format!("已全量替换 {}（新内容 {} 字节）", p.display(), content.len()))
        }
        other => Err(format!("未知 mode {other}（支持 append / replace）")),
    }
}

// ---- 操作审计日志（fileops.log） ----

/// 日志路径：$QIBAI_DATA_DIR/fileops.log → data_dir()/fileops.log → $HOME/.qibai/fileops.log。
pub fn fileops_log_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QIBAI_DATA_DIR") {
        return Some(PathBuf::from(dir).join("fileops.log"));
    }
    crate::settings::data_dir()
        .ok()
        .map(|d| d.join("fileops.log"))
        .or_else(|| default_home().map(|h| h.join(".qibai/fileops.log")))
}

/// 追加一行审计日志：`unix秒 action path ok|failed`。日志失败绝不影响主操作。
pub fn log_fileop(action: &str, path: &str, ok: bool, detail: &str) {
    let Some(p) = fileops_log_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let status = if ok { "ok" } else { "failed" };
    let detail: String = detail.chars().filter(|c| *c != '\n').take(80).collect();
    let line = format!("{ts} {action} {path} {status} {detail}\n");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// 批准后的写入执行入口（skills.rs approve() 回调）。
pub async fn perform_write(path: &str, content: &str, home: &str) -> Result<String, String> {
    let r = write_inner(path, content, home);
    log_fileop("write", path, r.is_ok(), &r.clone().unwrap_or_else(|e| e));
    if r.is_ok() {
        // 闭环：若落在工程目录内，自动维护 PROJECT.md 上下文 + 注册表时间戳
        if let Ok(p) = sanitize_path(path, home) {
            crate::project_context::hook_file_change(&p, "write", "写入");
            if let Some(dir) = crate::project_context::find_project_dir(&p) {
                crate::project_scaffold::touch_project(&dir.to_string_lossy());
            }
        }
    }
    r
}

/// 批准后的编辑执行入口（skills.rs approve() 回调）。
pub async fn perform_edit(path: &str, content: &str, mode: &str, home: &str) -> Result<String, String> {
    let r = edit_inner(path, content, mode, home);
    log_fileop(
        &format!("edit:{mode}"),
        path,
        r.is_ok(),
        &r.clone().unwrap_or_else(|e| e),
    );
    if r.is_ok() {
        if let Ok(p) = sanitize_path(path, home) {
            crate::project_context::hook_file_change(&p, "edit", "修改");
            if let Some(dir) = crate::project_context::find_project_dir(&p) {
                crate::project_scaffold::touch_project(&dir.to_string_lossy());
            }
        }
    }
    r
}

/// 批准后的删除执行入口（skills.rs approve() 回调）。
/// 二次消毒 + HOME 根保护，确保审批通过的操作仍在沙盒内。
pub async fn perform_delete(path: &str, home: &str) -> Result<String, String> {
    let p = sanitize_path(path, home)?;
    let home_path = Path::new(home);
    if is_home_root(&p, home_path) {
        return Err("不允许删除主目录根目录本身（安全限制）".to_string());
    }
    if !p.exists() {
        return Err(format!("{} 不存在，无需删除", p.display()));
    }
    let result = if p.is_dir() {
        std::fs::remove_dir_all(&p).map_err(|e| format!("删除文件夹失败: {e}"))
    } else {
        std::fs::remove_file(&p).map_err(|e| format!("删除文件失败: {e}"))
    };
    let r = result.map(|_| format!("已删除：{}", p.display()));
    let log_msg = r.as_ref().err().cloned().unwrap_or_default();
    log_fileop("delete", path, r.is_ok(), &log_msg);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home() -> PathBuf {
        let d = std::env::temp_dir().join(format!("qibai_fileops_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        d
    }

    // ---- sanitize_path（≥8） ----

    #[test]
    fn sanitize_expands_tilde_and_relative() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        let p = sanitize_path("~/sub/a.txt", h).unwrap();
        assert!(p.ends_with("sub/a.txt"));
        assert!(p.starts_with(&home));
        // v45：相对名不再报错，统一按主目录解析（仍在沙盒内）
        let rel = sanitize_path("relative/path", h).unwrap();
        assert!(rel.starts_with(&home));
        assert!(rel.ends_with("relative/path"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resolve_accepts_chinese_dir_aliases() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        std::fs::create_dir_all(home.join("Desktop")).unwrap();
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        // 「打开下载文件夹」「桌面」「文档」都必须解析成功，而不是报"必须是绝对路径"
        for (say, want) in [
            ("下载", "Downloads"),
            ("下载文件夹", "Downloads"),
            ("下载目录", "Downloads"),
            ("Downloads", "Downloads"),
            ("桌面", "Desktop"),
            ("文档", "Documents"),
            ("下了载", "下了载"), // 未知名字：按主目录相对名处理，不 panic
        ] {
            let p = resolve_user_path(say, h);
            assert!(p.starts_with(&home), "{say} 应落在主目录内");
            assert!(
                p.to_string_lossy().ends_with(want),
                "{say} 应解析到 {want}，实际 {}",
                p.display()
            );
        }
        // 带口语尾巴与引号也要认
        assert!(resolve_user_path("“下载”", h).ends_with("Downloads"));
        assert!(resolve_user_path("桌面。", h).ends_with("Desktop"));
        // 空路径 = 主目录
        assert_eq!(resolve_user_path("", h), home);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn suggest_similar_finds_near_miss() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        std::fs::create_dir_all(home.join("workspace")).unwrap();
        let s = suggest_similar("download", h, 5);
        assert!(s.iter().any(|x| x.ends_with("Downloads")), "应能推荐 Downloads：{s:?}");
        let s2 = suggest_similar("work", h, 5);
        assert!(s2.iter().any(|x| x.ends_with("workspace")), "应能推荐 workspace：{s2:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn open_missing_path_reports_candidates_not_silent_success() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        // 不存在的路径必须 Err，且带上候选，而不是"已尝试打开"
        let e = run_file_op("open", "下载x", None, None, h).await.unwrap_err();
        assert!(e.contains("没找到"), "错误应说明没找到：{e}");
        assert!(e.contains("Downloads"), "应给出候选：{e}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_rejects_dotdot_traversal() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        assert!(sanitize_path("~/../../etc/passwd", h).is_err());
        assert!(sanitize_path(&format!("{}/sub/../../evil", home.display()), h).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_rejects_outside_home() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        // /tmp 不在（临时 home）内
        assert!(sanitize_path("/tmp/xyz", h).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_rejects_blocked_system_roots() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        assert!(sanitize_path("/etc/passwd", h).unwrap_err().contains("系统目录"));
        assert!(sanitize_path("/usr/bin/xx", h).unwrap_err().contains("系统目录"));
        assert!(sanitize_path("/var/log/x", h).is_err());
        assert!(sanitize_path("/", h).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_rejects_symlink_escape() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        // home 内建一个指向 /etc 的符号链接
        let link = home.join("evil_link");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        #[cfg(not(unix))]
        { let _ = &link; }
        #[cfg(unix)]
        {
            let err = sanitize_path(link.to_str().unwrap(), h).unwrap_err();
            assert!(err.contains("主目录"), "实际: {err}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_accepts_new_path_under_existing_parent() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        // 尚不存在的新文件路径：父目录 canonicalize 复查后应放行
        let p = sanitize_path(&format!("{}/sub/new.txt", home.display()), h).unwrap();
        assert!(!p.exists());
        assert!(p.starts_with(&home));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_rejects_empty_and_nul() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        assert!(sanitize_path("", h).is_err());
        assert!(sanitize_path("   ", h).is_err());
        assert!(sanitize_path("a\0b", h).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sanitize_home_root_allowed_but_detected() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        let p = sanitize_path("~", h).unwrap();
        assert!(is_home_root(&p, &home));
        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- run_file_op 临时目录行为（≥6） ----

    #[tokio::test]
    async fn fileop_list_create_rename_move_copy_happy_paths() {
        let home = tmp_home();
        let h = home.to_str().unwrap();

        // create
        let msg = run_file_op("create", "~/sub/note.txt", None, Some("你好世界"), h).await.unwrap();
        assert!(msg.contains("已创建文件"));

        // list（sub 下应有 note.txt）
        let msg = run_file_op("list", "~/sub", None, None, h).await.unwrap();
        assert!(msg.contains("note.txt"), "实际: {msg}");

        // rename
        let msg = run_file_op("rename", "~/sub/note.txt", Some("~/sub/renamed.txt"), None, h).await.unwrap();
        assert!(msg.contains("重命名"));
        assert!(home.join("sub/renamed.txt").exists());
        assert!(!home.join("sub/note.txt").exists());

        // move
        let msg = run_file_op("move", "~/sub/renamed.txt", Some("~/moved.txt"), None, h).await.unwrap();
        assert!(msg.contains("移动"));
        assert!(home.join("moved.txt").exists());

        // copy
        let msg = run_file_op("copy", "~/moved.txt", Some("~/sub/copy.txt"), None, h).await.unwrap();
        assert!(msg.contains("已复制"));
        assert!(home.join("sub/copy.txt").exists());
        assert_eq!(std::fs::read_to_string(home.join("sub/copy.txt")).unwrap(), "你好世界");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn fileop_rejects_outside_home() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        assert!(run_file_op("create", "/etc/evil.txt", None, Some("x"), h).await.is_err());
        assert!(run_file_op("list", "/etc", None, None, h).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn fileop_rejects_overwrite_existing_dest() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        run_file_op("create", "~/a.txt", None, Some("A"), h).await.unwrap();
        run_file_op("create", "~/b.txt", None, Some("B"), h).await.unwrap();
        // create 覆盖拒绝
        assert!(run_file_op("create", "~/a.txt", None, Some("X"), h).await.is_err());
        // move 到已存在目标拒绝
        assert!(run_file_op("move", "~/a.txt", Some("~/b.txt"), None, h).await.is_err());
        // copy 到已存在目标拒绝
        assert!(run_file_op("copy", "~/a.txt", Some("~/b.txt"), None, h).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn fileop_delete_refused_in_module() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        let err = run_file_op("delete", "~/sub", None, None, h).await.unwrap_err();
        assert!(err.contains("批准"), "实际: {err}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn fileop_unknown_action_rejected() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        let err = run_file_op("hack", "~/sub", None, None, h).await.unwrap_err();
        assert!(err.contains("未知操作"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn perform_delete_executes_after_approval_with_safety() {
        let home = tmp_home();
        let h = home.to_str().unwrap();
        run_file_op("create", "~/trash.txt", None, Some("bye"), h).await.unwrap();
        let msg = perform_delete("~/trash.txt", h).await.unwrap();
        assert!(msg.contains("已删除"));
        assert!(!home.join("trash.txt").exists());
        // HOME 根禁止删
        assert!(perform_delete("~", h).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
