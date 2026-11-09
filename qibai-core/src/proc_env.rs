//! 子进程 PATH 补全：让「导入的插件包」在桌面应用里也能找到 node / ffmpeg / chromium。
//!
//! 为什么需要：从 Finder / 桌面图标启动的 macOS 应用继承的是 launchd 的极简环境
//! （通常只有 `/usr/bin:/bin:/usr/sbin:/sbin`），Homebrew 的 `/opt/homebrew/bin`、
//! nvm 的 `~/.nvm/versions/node/<v>/bin` 都不在里面；Windows 上 `Program Files\nodejs`
//! 也常常不在图形会话的 PATH 里。表现为：插件装了、按钮点了、审批通过了，但一 spawn
//! 就 `No such file or directory`，用户只看到一句「审批失败」。
//!
//! 策略：不改系统环境，只给**我们自己拉起的子进程**注入一份更全的 PATH
//! （原 PATH 在前，常见安装目录在后，可再叠加 `QIBAI_EXTRA_PATH`）。

use std::ffi::OsString;
use std::path::PathBuf;

/// 用户可手动追加目录的环境变量（多个用系统路径分隔符隔开）。
pub const EXTRA_ENV: &str = "QIBAI_EXTRA_PATH";

/// 常见可执行目录（只收真实存在的，顺序：包管理器 → 系统目录）。
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    // --- 跨平台通用 ---
    if cfg!(target_os = "windows") {
        for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(key).map(PathBuf::from) {
                v.push(base.join("nodejs"));
                v.push(base.join("npm"));
            }
            if key == "LOCALAPPDATA" {
                if let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                    v.push(base.join("Programs").join("nodejs"));
                    v.push(base.join("Microsoft").join("WinGet").join("Links"));
                }
            }
        }
        if let Some(h) = &home {
            v.push(h.join("scoop").join("shims"));
            v.push(h.join(".volta").join("bin"));
        }
    } else {
        // macOS / Linux 包管理器目录
        if cfg!(target_os = "macos") {
            v.push(PathBuf::from("/opt/homebrew/bin"));
            v.push(PathBuf::from("/opt/homebrew/sbin"));
            v.push(PathBuf::from("/opt/local/bin")); // MacPorts
        }
        v.push(PathBuf::from("/usr/local/bin"));
        v.push(PathBuf::from("/usr/local/sbin"));
        v.push(PathBuf::from("/usr/bin"));
        v.push(PathBuf::from("/bin"));
        v.push(PathBuf::from("/snap/bin"));
        if let Some(h) = &home {
            v.push(h.join(".local").join("bin"));
            v.push(h.join(".cargo").join("bin"));
            v.push(h.join(".volta").join("bin"));
            v.push(h.join(".asdf").join("shims"));
            v.push(h.join(".local").join("share").join("mise").join("shims"));
            v.push(h.join(".local").join("share").join("fnm"));
        }
    }

    // --- Node 版本管理器（nvm / fnm），按版本目录展开 ---
    if let Some(h) = &home {
        let mut roots = vec![h.join(".nvm").join("versions").join("node")];
        roots.push(h.join(".local").join("share").join("fnm").join("node-versions"));
        roots.push(
            h.join("Library")
                .join("Application Support")
                .join("fnm")
                .join("node-versions"),
        );
        for root in roots {
            push_version_bins(&root, &mut v);
        }
    }

    if let Some(extra) = std::env::var_os(EXTRA_ENV) {
        v.extend(std::env::split_paths(&extra));
    }

    // 去重 + 只留存在目录
    v.retain(|p| p.is_dir());
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if !out.iter().any(|q| q == &p) {
            out.push(p);
        }
    }
    out
}

/// `fnm` 的版本目录有两种布局：`<root>/node-versions/<ver>/installation/bin`
/// 与 `<root>/<ver>/installation/bin`，两种都尝试。
fn push_version_bins(root: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        out.push(p.join("installation").join("bin"));
        out.push(p.join("bin"));
    }
}

/// 原 PATH + 常见安装目录，拼成给子进程用的 PATH。
pub fn augmented_path() -> OsString {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(cur) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&cur));
    }
    parts.extend(candidate_dirs());
    let mut dedup: Vec<PathBuf> = Vec::new();
    for p in parts {
        if !dedup.iter().any(|q| q == &p) {
            dedup.push(p);
        }
    }
    std::env::join_paths(dedup).unwrap_or_else(|_| OsString::new())
}

/// 给 `std::process::Command` 注入补全后的 PATH。
pub fn apply_std(cmd: &mut std::process::Command) {
    let p = augmented_path();
    if !p.is_empty() {
        cmd.env("PATH", p);
    }
}

/// 给 `tokio::process::Command` 注入补全后的 PATH。
pub fn apply_tokio(cmd: &mut tokio::process::Command) {
    let p = augmented_path();
    if !p.is_empty() {
        cmd.env("PATH", p);
    }
}

/// 在补全后的 PATH 里找可执行文件；找不到返回 None。
pub fn which(name: &str) -> Option<PathBuf> {
    if name.trim().is_empty() {
        return None;
    }
    let as_path = std::path::Path::new(name);
    if as_path.is_absolute() || name.contains('/') || name.contains('\\') {
        return if as_path.is_file() {
            Some(as_path.to_path_buf())
        } else {
            None
        };
    }
    let exts: Vec<String> = if cfg!(target_os = "windows") {
        std::env::var("PATHEXT")
            .ok()
            .map(|v| {
                v.split(';')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_lowercase())
                    .collect()
            })
            .unwrap_or_else(|| vec![".exe".into(), ".cmd".into(), ".bat".into()])
    } else {
        vec![]
    };
    for dir in std::env::split_paths(&augmented_path()) {
        let base = dir.join(name);
        if base.is_file() {
            return Some(base);
        }
        for ext in &exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// 插件的启动命令可能三种形态：`node` 这种 basename（走 PATH）、
/// `scripts/now.sh` 这种相对插件目录的路径、以及含 `{PLUGIN_DIR}` 占位符的写法。
/// 这里按插件落点解析成真实可执行文件；解析不到返回 None。
pub fn resolve_launcher(cmd: &str, plugin_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    let raw = cmd.trim();
    if raw.is_empty() {
        return None;
    }
    let expanded = match plugin_dir {
        Some(d) => raw.replace("{PLUGIN_DIR}", &d.to_string_lossy()),
        None => raw.replace("{PLUGIN_DIR}", ""),
    };
    let has_sep = expanded.contains('/') || expanded.contains('\\');
    if has_sep {
        let p = PathBuf::from(&expanded);
        if p.is_absolute() {
            return p.is_file().then_some(p);
        }
        let dir = plugin_dir?;
        let joined = dir.join(&p);
        return joined.is_file().then_some(joined);
    }
    which(&expanded)
}

/// 错误文本是否像「可执行缺失」——用于把 spawn 报错翻译成人能看懂的提示。
///
/// 只看真正像子进程启动失败的写法。裸 `not found` 会误伤业务错误
/// （例如「确认已失效：找不到请求 …」曾被判成缺件，给用户的提示变成
/// 「装好后重启 / 写进 QIBAI_EXTRA_PATH」，完全指错方向）。
pub fn looks_like_missing_binary(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("os error 2")
        || m.contains("no such file or directory")
        || m.contains("cannot find the file")
        || m.contains("program not found")
        || m.contains("command not found")
        || m.contains("executable not found")
        || (m.contains("not found") && m.contains("executable"))
}

/// 缺件时统一的行动建议（中英混排的报错也能接住）。
pub fn missing_binary_hint(cmd: &str) -> String {
    format!(
        "找不到可执行 `{cmd}`。装好它（node ≥18 / ffmpeg / chromium 之类）后重启元气小小；\
         若已装但不在默认位置，把所在目录写进环境变量 {EXTRA_ENV}（多个用系统路径分隔符隔开）。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn augmented_path_keeps_original_first() {
        let old = std::env::var_os("PATH");
        let p = augmented_path();
        let parts: Vec<PathBuf> = std::env::split_paths(&p).collect();
        assert!(!parts.is_empty(), "补全后的 PATH 不能为空");
        if let Some(old) = old {
            let first_old = std::env::split_paths(&old).next();
            if let Some(f) = first_old {
                assert_eq!(parts[0], f, "原 PATH 的第一项应保持不变（避免覆盖用户环境）");
            }
        }
    }

    #[test]
    fn which_finds_shell_and_reports_missing() {
        let target = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
        assert!(which(target).is_some(), "{target} 应在补全后的 PATH 里找得到");
        assert!(which("qibai-no-such-binary-xyz").is_none());
    }

    #[test]
    fn resolve_launcher_handles_plugin_relative_scripts() {
        let dir = std::env::temp_dir().join(format!("qibai-procenv-{}", std::process::id()));
        let scripts = dir.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let f = scripts.join("now.sh");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        assert!(resolve_launcher("scripts/now.sh", Some(&dir)).is_some());
        assert!(resolve_launcher("{PLUGIN_DIR}/scripts/now.sh", Some(&dir)).is_some());
        assert!(resolve_launcher("scripts/nope.sh", Some(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_binary_error_is_recognized() {
        assert!(looks_like_missing_binary(
            "MCP server 启动失败: No such file or directory (os error 2)"
        ));
        assert!(!looks_like_missing_binary("脚本退出码 1：boom"));
    }

    /// 「确认已失效」这类业务错误不是缺件——不许给「装好后重启 / QIBAI_EXTRA_PATH」的提示。
    #[test]
    fn approval_errors_are_not_missing_binary() {
        assert!(!looks_like_missing_binary(
            "这条确认已失效：找不到请求 approval-2（可能已经执行过、或应用重启过）"
        ));
        assert!(!looks_like_missing_binary(
            "approval request not found: approval-2"
        ));
        assert!(!looks_like_missing_binary("skill not in whitelist: hack_nasa"));
        // 真缺件仍然认得出
        assert!(looks_like_missing_binary("node: command not found"));
        assert!(looks_like_missing_binary("spawn ffmpeg: No such file or directory"));
    }
}
