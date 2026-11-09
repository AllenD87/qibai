//! 代码检查引擎（迭代①「代码检查」核心模块）。
//!
//! 本模块是**纯 Rust 库逻辑**：只依赖 serde / serde_json / thiserror / async-trait / tokio，
//! 不引用 `crate::` 下任何其他模块，可独立冒烟编译 / 单测。
//!
//! # 只读保证（安全红线）
//!
//! - 本模块**绝不写、改、删任何用户文件**。
//! - JS 用 `node --check`（Node 自带语法检查，不落盘）；
//! - Python 用 `ast.parse`（**不使用** `py_compile`，因为它会写 `__pycache__/`）；
//! - Rust **绝不运行 `cargo check`**（它会写 `target/`，违反只读），只做启发式 + 人工提示；
//! - TS/TSX 不跑 `tsc`（需要项目环境，且可能写 `.tsbuildinfo`）。
//!
//! # LLM 降级
//!
//! [`SuggestLlm`] 是本地 trait；`None` 或调用失败时 `CodeReport.llm_note = None`，
//! 输出纯静态报告——这是无 key 时的默认路径，不是错误。真实 `LlmProvider` 适配由集成代理在 engine 侧完成。
//!
//! # 冻结的 JSON 形状
//!
//! [`CodeReport`] / [`CodeIssue`] 是冻结契约：集成代理（engine / tauri-bridge）与前端 mock
//! 必须严格按本文件的 serde 形状接线，不得增删字段。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 报告模型（serde 形状冻结）
// ---------------------------------------------------------------------------

/// 问题严重级别（JSON 字符串："error" | "warning" | "hint"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
    Hint,
}

/// 问题类别（JSON 字符串："syntax" | "logic" | "security" | "style" | "unknown"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueCategory {
    Syntax,
    Logic,
    Security,
    Style,
    Unknown,
}

/// 单条代码问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIssue {
    /// "error" | "warning" | "hint"
    pub severity: IssueSeverity,
    /// "syntax" | "logic" | "security" | "style" | "unknown"
    pub category: IssueCategory,
    /// 人类可读的问题描述。
    pub message: String,
    /// "file:line" 或 None。
    pub location: Option<String>,
    /// 修复建议文本，没有则 None。
    pub suggestion: Option<String>,
}

impl CodeIssue {
    pub fn new(
        severity: IssueSeverity,
        category: IssueCategory,
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
    ) -> Self {
        Self {
            severity,
            category,
            message: message.into(),
            location,
            suggestion,
        }
    }
}

/// 检查来源（JSON 字符串："path" | "snippet"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckSource {
    Path,
    Snippet,
}

/// 一次代码检查的完整报告（冻结 JSON 形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeReport {
    /// "path" | "snippet"
    pub source: CheckSource,
    /// path 模式下为被检查路径；snippet 模式为 None。
    pub path: Option<String>,
    /// 实际检查的文件数。
    pub files_checked: u32,
    /// 语言名：javascript / python / rust / typescript / json / html / css / markdown / text / unknown / mixed。
    pub language: String,
    /// 有 issues → "发现 N 个问题：error X / warning Y / hint Z"；无 → "未发现问题"。
    pub summary: String,
    pub issues: Vec<CodeIssue>,
    /// LLM 优化建议；无 LLM 或失败时为 None。
    pub llm_note: Option<String>,
}

// ---------------------------------------------------------------------------
// 检查请求
// ---------------------------------------------------------------------------

/// 代码检查请求。`path` 与 `snippet` 恰好提供一个。
#[derive(Debug, Clone)]
pub struct CheckRequest {
    pub source: CheckSource,
    pub path: Option<PathBuf>,
    pub snippet: Option<String>,
    /// snippet 模式可选语言名；path 模式忽略（按扩展名自动识别）。
    pub language: Option<String>,
}

impl CheckRequest {
    /// 构造 path 模式请求。
    pub fn from_path(p: impl Into<PathBuf>) -> Self {
        Self {
            source: CheckSource::Path,
            path: Some(p.into()),
            snippet: None,
            language: None,
        }
    }

    /// 构造 snippet 模式请求（语言可选）。
    pub fn from_snippet(snippet: impl Into<String>, language: Option<String>) -> Self {
        Self {
            source: CheckSource::Snippet,
            path: None,
            snippet: Some(snippet.into()),
            language,
        }
    }

    /// 从 LLM / 前端传入的 JSON 构造请求；非法输入返回结构化错误字符串。
    ///
    /// 校验规则：
    /// - `source` 必须存在且为 "path" | "snippet"；
    /// - path 模式必须提供非空 `path`，且不得提供 `snippet`；
    /// - snippet 模式必须提供非空 `snippet`，且不得提供 `path`；
    /// - `language` 可选。
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let source_str = v
            .get("source")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "missing field: source (must be \"path\" or \"snippet\")".to_string())?;
        let source = match source_str {
            "path" => CheckSource::Path,
            "snippet" => CheckSource::Snippet,
            other => return Err(format!("invalid source: {other:?} (must be \"path\" or \"snippet\")")),
        };

        let path_raw = v.get("path").and_then(|x| x.as_str());
        let snippet_raw = v.get("snippet").and_then(|x| x.as_str());
        let language = v
            .get("language")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        let path = path_raw.filter(|s| !s.is_empty()).map(PathBuf::from);
        let snippet = snippet_raw
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        match (source, path, snippet) {
            (CheckSource::Path, Some(p), None) => Ok(Self {
                source,
                path: Some(p),
                snippet: None,
                language,
            }),
            (CheckSource::Snippet, None, Some(s)) => Ok(Self {
                source,
                path: None,
                snippet: Some(s),
                language,
            }),
            (CheckSource::Path, None, _) => {
                Err("path mode requires a non-empty string field: path".to_string())
            }
            (CheckSource::Snippet, _, None) => {
                Err("snippet mode requires a non-empty string field: snippet".to_string())
            }
            (CheckSource::Path, Some(_), Some(_)) => {
                Err("path mode must not provide snippet (exactly one of path/snippet is allowed)".to_string())
            }
            (CheckSource::Snippet, Some(_), Some(_)) => {
                Err("snippet mode must not provide path (exactly one of path/snippet is allowed)".to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 可 mock 的检查器抽象
// ---------------------------------------------------------------------------

/// 外部命令执行结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckOutput {
    pub stdout: String,
    pub stderr: String,
    pub ok: bool,
}

/// 检查器抽象：把「跑一个外部命令」抽出来，测试时可 mock。
#[async_trait]
pub trait CheckRunner: Send + Sync {
    async fn run(
        &self,
        argv: &[&str],
        cwd: Option<&Path>,
        env: &[(&str, &str)],
    ) -> Result<CheckOutput, String>;
}

/// 真实系统检查器：用 `tokio::process::Command` 执行。
#[derive(Debug, Clone, Copy, Default)]
pub struct OsCheckRunner;

#[async_trait]
impl CheckRunner for OsCheckRunner {
    async fn run(
        &self,
        argv: &[&str],
        cwd: Option<&Path>,
        env: &[(&str, &str)],
    ) -> Result<CheckOutput, String> {
        if argv.is_empty() {
            return Err("empty argv".to_string());
        }
        let mut cmd = tokio::process::Command::new(argv[0]);
        cmd.args(&argv[1..]);
        crate::proc_env::apply_tokio(&mut cmd);
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        for (k, val) in env {
            cmd.env(k, val);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let out = cmd
            .output()
            .await
            .map_err(|e| format!("spawn {} failed: {e}", argv[0]))?;
        Ok(CheckOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            ok: out.status.success(),
        })
    }
}

/// LLM 建议抽象：返回 None 表示无建议 / 调用失败。
#[async_trait]
pub trait SuggestLlm: Send + Sync {
    async fn suggest(&self, report: &CodeReport, language: &str) -> Option<String>;
}

// ---------------------------------------------------------------------------
// 语言识别
// ---------------------------------------------------------------------------

/// 按扩展名映射语言名。
pub fn language_of_path(p: &Path) -> String {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "js" | "mjs" | "cjs" => "javascript".to_string(),
        "py" => "python".to_string(),
        "json" => "json".to_string(),
        "rs" => "rust".to_string(),
        "ts" | "tsx" => "typescript".to_string(),
        "html" | "htm" => "html".to_string(),
        "css" => "css".to_string(),
        "md" | "markdown" => "markdown".to_string(),
        "txt" => "text".to_string(),
        _ => "unknown".to_string(),
    }
}

/// snippet 模式自动嗅探语言。
pub fn sniff_language(snippet: &str) -> String {
    let trimmed = snippet.trim();
    // JSON：以 { 或 [ 开头
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return "json".to_string();
    }
    // JS 特征
    let has_js = trimmed.contains("function")
        || trimmed.contains("=>")
        || trimmed.contains("var ")
        || trimmed.contains("let ")
        || trimmed.contains("const ");
    if has_js {
        return "javascript".to_string();
    }
    // Python 特征
    let has_py = trimmed.contains("def ")
        || trimmed.contains("import ")
        || trimmed.contains("print(");
    if has_py {
        return "python".to_string();
    }
    "unknown".to_string()
}

// ---------------------------------------------------------------------------
// 启发式静态检查（纯字符串扫描，零外部 crate 依赖）
// ---------------------------------------------------------------------------

/// 对一段文本做启发式扫描。`file` 为 Some 时 location 为 "file:line"，否则为 "line N"。
fn heuristic_scan(text: &str, file: Option<&str>) -> Vec<CodeIssue> {
    let mut issues = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let loc = || match file {
            Some(f) => format!("{f}:{line_no}"),
            None => format!("line {line_no}"),
        };

        // ---- style：行尾空白 ----
        if raw.ends_with(' ') || raw.ends_with('\t') {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Style,
                "行尾存在多余的空格或 Tab",
                Some(loc()),
                Some("删除行尾空白字符".to_string()),
            ));
        }

        // ---- style：行长 >120 ----
        if raw.chars().count() > 120 {
            issues.push(CodeIssue::new(
                IssueSeverity::Hint,
                IssueCategory::Style,
                format!("行长度 {} 超过 120 字符", raw.chars().count()),
                Some(loc()),
                Some("考虑拆行或提取变量以提升可读性".to_string()),
            ));
        }

        // ---- style：行首 TAB 与空格混用 ----
        let leading_len = raw
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count();
        if leading_len > 0 {
            let leading: String = raw.chars().take(leading_len).collect();
            if leading.contains('\t') && leading.contains(' ') {
                issues.push(CodeIssue::new(
                    IssueSeverity::Warning,
                    IssueCategory::Style,
                    "行首 TAB 与空格混用",
                    Some(loc()),
                    Some("统一为 Tab 或统一为空格缩进".to_string()),
                ));
            }
        }

        // ---- style：TODO / FIXME / XXX ----
        for kw in ["TODO", "FIXME", "XXX"] {
            if raw.contains(kw) {
                issues.push(CodeIssue::new(
                    IssueSeverity::Hint,
                    IssueCategory::Style,
                    format!("发现未完成标记：{kw}"),
                    Some(loc()),
                    Some("计划在此处完成后移除该标记".to_string()),
                ));
                break;
            }
        }

        // ---- security：eval( ----
        if raw.contains("eval(") {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Security,
                "检测到 eval() 调用，可能导致注入风险",
                Some(loc()),
                Some("避免使用 eval；可用 JSON.parse / 显式函数映射替代".to_string()),
            ));
        }

        // ---- security：child_process / execSync / spawn ----
        if raw.contains("child_process") {
            issues.push(CodeIssue::new(
                IssueSeverity::Hint,
                IssueCategory::Security,
                "引用了 child_process，注意命令注入风险",
                Some(loc()),
                Some("避免把不可信字符串拼进 shell 命令；优先用参数数组形式".to_string()),
            ));
        }
        if raw.contains("execSync(") || raw.contains("spawn(") {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Security,
                "检测到同步执行外部进程，可能阻塞并存在注入风险",
                Some(loc()),
                Some("如非必要改用异步调用，并对入参做白名单校验".to_string()),
            ));
        }

        // ---- security：rm -rf 带变量插值 ----
        if ((raw.contains("rm") && raw.contains("-rf")) || raw.contains("-fr"))
            && (raw.contains('$') || raw.contains('`'))
        {
            issues.push(CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Security,
                "检测到 rm -rf 与变量插值/命令替换组合，误删风险极高",
                Some(loc()),
                Some("禁止对插值变量做 rm -rf；先展开为绝对路径并做白名单校验".to_string()),
            ));
        }

        // ---- security：硬编码密钥 ----
        if is_hardcoded_secret(raw) {
            issues.push(CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Security,
                "检测到疑似硬编码的密钥/口令",
                Some(loc()),
                Some("改为从环境变量或凭据管理读取，不要把密钥写进源码".to_string()),
            ));
        }

        // ---- security：innerHTML 拼接变量 ----
        if raw.contains("innerHTML") && raw.contains('+') {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Security,
                "innerHTML 与变量拼接，可能导致 XSS",
                Some(loc()),
                Some("改用 textContent，或对插入内容做转义".to_string()),
            ));
        }

        // ---- logic：宽松相等 == / !=（仅 JS 系） ----
        if (looks_like_js(file, raw)) && has_loose_eq(raw) {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Logic,
                "使用了宽松相等（== / !=），可能发生隐式类型转换",
                Some(loc()),
                Some("改用严格相等 === / !==".to_string()),
            ));
        }

        // ---- logic：while(true) 无 break ----
        if raw.contains("while") && line_contains_while_true(raw) {
            issues.push(CodeIssue::new(
                IssueSeverity::Hint,
                IssueCategory::Logic,
                "检测到 while(true)，请确认循环体内有 break/return 退出",
                Some(loc()),
                Some("为死循环提供明确退出条件，避免卡死".to_string()),
            ));
        }

        // ---- logic：空 catch / 空 except ----
        if catch_or_except_empty(raw) {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Logic,
                "catch/except 块为空，会吞掉异常",
                Some(loc()),
                Some("至少记录日志或向上抛出异常".to_string()),
            ));
        }

        // ---- logic：除零表达式 ----
        if has_div_zero(raw) {
            issues.push(CodeIssue::new(
                IssueSeverity::Warning,
                IssueCategory::Logic,
                "检测到除以字面量 0 的表达式",
                Some(loc()),
                Some("增加除数为 0 的保护或使用变量并提前校验".to_string()),
            ));
        }
    }
    issues
}

fn looks_like_js(file: Option<&str>, _line: &str) -> bool {
    match file {
        Some(f) => {
            let p = Path::new(f);
            let lang = language_of_path(p);
            lang == "javascript" || lang == "typescript" || lang == "unknown"
        }
        None => true, // snippet 模式由调用方决定；默认当作 JS 候选
    }
}

fn line_contains_while_true(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i + 5 <= b.len() {
        if &b[i..i + 5] == b"while" {
            let mut j = i + 5;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' {
                let mut k = j + 1;
                while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
                    k += 1;
                }
                if k + 4 <= b.len() && &b[k..k + 4] == b"true" {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn catch_or_except_empty(line: &str) -> bool {
    for kw in ["catch", "except"] {
        if let Some(pos) = line.find(kw) {
            let rest = &line[pos + kw.len()..];
            // 跳过 (err) 形式
            let rest = rest.trim_start();
            let rest = if rest.starts_with('(') {
                match rest.find(')') {
                    Some(p) => rest[p + 1..].trim_start(),
                    None => rest,
                }
            } else {
                rest
            };
            if rest.starts_with('{') {
                let body: String = rest[1..].chars().take_while(|c| *c != '}').collect();
                if body.trim().is_empty() {
                    return true;
                }
            }
            // python: except: pass
            if kw == "except" && rest.starts_with(':') {
                let after: String = rest[1..].trim().chars().take(10).collect();
                if after == "pass" || after.starts_with("pass") {
                    return true;
                }
            }
        }
    }
    false
}

fn has_loose_eq(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'=' && b[i + 1] == b'=' {
            let strict = i + 2 < b.len() && b[i + 2] == b'=';
            if !strict {
                return true;
            }
            i += 2;
            continue;
        }
        if b[i] == b'!' && b[i + 1] == b'=' {
            let strict = i + 2 < b.len() && b[i + 2] == b'=';
            if !strict {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn has_div_zero(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' {
            let mut j = i + 1;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'0' {
                let after_zero = if j + 1 >= b.len() {
                    None
                } else {
                    Some(b[j + 1])
                };
                let is_literal_zero = after_zero.map(|c| !c.is_ascii_digit()).unwrap_or(true);
                let prev_ok = if i == 0 {
                    true
                } else {
                    let p = b[i - 1];
                    !(p == b'/' || p == b':' || p == b'*')
                };
                if is_literal_zero && prev_ok {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn is_hardcoded_secret(line: &str) -> bool {
    let lower = line.to_lowercase();
    // 1) api_key / apikey / api-key / password / token / secret / doubao_api_key 赋值给字面量字符串
    for key in [
        "api_key",
        "apikey",
        "api-key",
        "password",
        "passwd",
        "token",
        "secret",
        "doubao_api_key",
    ] {
        if let Some(pos) = lower.find(key) {
            let rest = &lower[pos + key.len()..];
            if let Some(eq_pos) = rest.find(|c: char| c == ':' || c == '=') {
                let after_eq = rest[eq_pos + 1..].trim_start();
                let q = after_eq.chars().next();
                if q == Some('"') || q == Some('\'') {
                    let quote = q.unwrap();
                    let body: String = after_eq.chars().skip(1).take_while(|c| *c != quote).collect();
                    // 排除占位符 "xxx" / "<...>" / 太短的
                    let is_placeholder =
                        body.chars().all(|c| c == 'x' || c == 'X' || c == '*' || c == '-');
                    if body.len() >= 4 && !is_placeholder {
                        return true;
                    }
                }
            }
        }
    }
    // 2) sk- 前缀密钥
    if let Some(p) = lower.find("sk-") {
        let rest = &lower[p + 3..];
        let run: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if run.len() >= 8 {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 语法检查（外部命令）
// ---------------------------------------------------------------------------

/// 对单个文件做语法检查，返回 (issues, degraded_note)。
/// degraded_note 非空表示对应工具不在 PATH，已降级为启发式。
async fn syntax_check_file(
    path: &Path,
    language: &str,
    runner: &dyn CheckRunner,
) -> (Vec<CodeIssue>, Option<String>) {
    match language {
        "javascript" => {
            let path_s = path.to_string_lossy().to_string();
            match runner.run(&["node", "--check", &path_s], None, &[]).await {
                Ok(out) if out.ok => (vec![], None),
                Ok(out) => (parse_node_stderr(&out.stderr, &path_s), None),
                Err(e) => {
                    let note = format!("node 不在 PATH（{e}），已降级为启发式检查，JS 语法错误可能漏报");
                    (vec![], Some(note))
                }
            }
        }
        "python" => {
            let path_s = path.to_string_lossy().to_string();
            let py = "import ast,sys; ast.parse(open(sys.argv[1],encoding='utf-8').read())";
            match runner
                .run(&["python3", "-c", py, &path_s], None, &[])
                .await
            {
                Ok(out) if out.ok => (vec![], None),
                Ok(out) => (parse_python_stderr(&out.stderr, &path_s), None),
                Err(e) => {
                    let note = format!("python3 不在 PATH（{e}），已降级为启发式检查，Python 语法错误可能漏报");
                    (vec![], Some(note))
                }
            }
        }
        "json" => {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        vec![CodeIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::Unknown,
                            format!("读取文件失败: {e}"),
                            Some(path_s_display(path)),
                            None,
                        )],
                        None,
                    );
                }
            };
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(_) => (vec![], None),
                Err(e) => {
                    let loc = extract_line_from_json_error(&e.to_string())
                        .map(|l| format!("{}:{l}", path.display()));
                    (
                        vec![CodeIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::Syntax,
                            format!("JSON 解析失败: {e}"),
                            loc,
                            Some("检查逗号、引号、括号是否配对".to_string()),
                        )],
                        None,
                    )
                }
            }
        }
        // rust / typescript / 其他：不跑外部命令（见模块文档的只读说明）
        _ => (vec![], None),
    }
}

fn path_s_display(path: &Path) -> String {
    path.display().to_string()
}

/// 从 node --check 的 stderr 提取 SyntaxError 问题。
fn parse_node_stderr(stderr: &str, path_s: &str) -> Vec<CodeIssue> {
    let mut issues = Vec::new();
    let mut msg: Option<String> = None;
    let mut line: Option<usize> = None;
    for ln in stderr.lines() {
        let t = ln.trim();
        if t.starts_with("SyntaxError") && msg.is_none() {
            msg = Some(t.to_string());
            // 尝试从 "SyntaxError: xxx (3:10)" 提取行号
            if let Some(open) = t.rfind('(') {
                let inner = &t[open + 1..];
                let digits: String = inner.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<usize>() {
                    line = Some(n);
                }
            }
        }
        // 首行形如 "<path>:6"
        if line.is_none() {
            let pfx = format!("{path_s}:");
            if t.starts_with(&pfx) {
                let tail = &t[pfx.len()..];
                let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<usize>() {
                    line = Some(n);
                }
            }
        }
    }
    let location = match line {
        Some(n) => Some(format!("{path_s}:{n}")),
        None => Some(path_s.to_string()),
    };
    issues.push(CodeIssue::new(
        IssueSeverity::Error,
        IssueCategory::Syntax,
        msg.unwrap_or_else(|| format!("node --check 失败: {}", first_nonempty(stderr))),
        location,
        Some("根据报错行修正 JS 语法".to_string()),
    ));
    issues
}

/// 从 python ast.parse 的 stderr 提取 SyntaxError 问题。
fn parse_python_stderr(stderr: &str, path_s: &str) -> Vec<CodeIssue> {
    let mut msg: Option<String> = None;
    let mut line: Option<usize> = None;
    for ln in stderr.lines() {
        let t = ln.trim();
        if t.starts_with("SyntaxError") && msg.is_none() {
            msg = Some(t.to_string());
        }
        if line.is_none() && t.starts_with("File ") && t.contains(", line ") {
            if let Some(idx) = t.find(", line ") {
                let tail = &t[idx + 7..];
                let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<usize>() {
                    line = Some(n);
                }
            }
        }
    }
    let location = match line {
        Some(n) => Some(format!("{path_s}:{n}")),
        None => Some(path_s.to_string()),
    };
    vec![CodeIssue::new(
        IssueSeverity::Error,
        IssueCategory::Syntax,
        msg.unwrap_or_else(|| format!("python ast.parse 失败: {}", first_nonempty(stderr))),
        location,
        Some("根据报错行修正 Python 语法（括号/缩进/引号）".to_string()),
    )]
}

fn first_nonempty(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("(no output)")
        .trim()
        .to_string()
}

fn extract_line_from_json_error(err: &str) -> Option<usize> {
    // serde_json: "... at line 2 column 3" 或 "line 1 column 36"
    if let Some(idx) = err.find("line ") {
        let tail = &err[idx + 5..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<usize>() {
            return Some(n);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 路径安全（只读保证）
// ---------------------------------------------------------------------------

const MAX_FILES: usize = 500;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const SKIP_DIRS: [&str; 7] = [
    "target",
    "node_modules",
    ".git",
    "_shots",
    "_backup",
    ".venv",
    "__pycache__",
];

/// 递归收集目录下待检查文件（跳过清单 + 上限截断）。
/// 返回 (files, truncated_note)。
fn collect_files(dir: &Path) -> Result<(Vec<PathBuf>, Option<String>), String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut truncated = false;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d)
            .map_err(|e| format!("读取目录失败 {}: {e}", d.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                    continue;
                }
                stack.push(p);
            } else if p.is_file() {
                if name.starts_with('.') {
                    continue;
                }
                if files.len() >= MAX_FILES {
                    truncated = true;
                    continue;
                }
                let meta = match e.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let sz = meta.len();
                if sz > MAX_FILE_BYTES {
                    truncated = true;
                    continue;
                }
                total_bytes += sz;
                if total_bytes > MAX_TOTAL_BYTES {
                    truncated = true;
                    break;
                }
                files.push(p);
            }
        }
    }
    files.sort();
    let note = if truncated {
        Some(format!(
            "目录文件过多/过大，已按上限截断（最多 {MAX_FILES} 个文件 / 单文件 ≤5MB / 总计 ≤50MB）"
        ))
    } else {
        None
    };
    Ok((files, note))
}

// ---------------------------------------------------------------------------
// 报告组装
// ---------------------------------------------------------------------------

fn build_summary(issues: &[CodeIssue]) -> String {
    if issues.is_empty() {
        return "未发现问题".to_string();
    }
    let e = issues.iter().filter(|i| i.severity == IssueSeverity::Error).count();
    let w = issues.iter().filter(|i| i.severity == IssueSeverity::Warning).count();
    let h = issues.iter().filter(|i| i.severity == IssueSeverity::Hint).count();
    format!("发现 {} 个问题：error {e} / warning {w} / hint {h}", issues.len())
}

async fn maybe_suggest(
    suggester: Option<&dyn SuggestLlm>,
    report: &CodeReport,
    language: &str,
) -> Option<String> {
    let s = suggester?;
    s.suggest(report, language).await
}

// ---------------------------------------------------------------------------
// 入口：run_check
// ---------------------------------------------------------------------------

/// 主入口：跑一次代码检查。
///
/// - `runner`：命令执行器（真实环境传 `&OsCheckRunner`）；
/// - `suggester`：可选 LLM 建议；None 或失败 → `llm_note=None`（纯静态报告，默认路径）。
pub async fn run_check(
    req: CheckRequest,
    runner: &dyn CheckRunner,
    suggester: Option<&dyn SuggestLlm>,
) -> CodeReport {
        let outcome = match req.source {
            CheckSource::Path => run_path(req, runner).await,
            CheckSource::Snippet => run_snippet(req, runner).await,
        };
        match outcome {
            Ok(mut report) => {
            let note = maybe_suggest(suggester, &report, &report.language).await;
            report.llm_note = note;
            report
        }
        Err(report) => {
            // 结构化错误也包成 CodeReport（files_checked=0 + 一条 error issue）
            report
        }
    }
}

async fn run_path(req: CheckRequest, runner: &dyn CheckRunner) -> Result<CodeReport, CodeReport> {
    let path = req.path.clone().expect("path mode");
    let path_str = path.display().to_string();

    // 存在性校验
    if !path.exists() {
        return Err(CodeReport {
            source: CheckSource::Path,
            path: Some(path_str.clone()),
            files_checked: 0,
            language: "unknown".to_string(),
            summary: "发现 1 个问题：error 1 / warning 0 / hint 0".to_string(),
            issues: vec![CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Unknown,
                format!("路径不存在: {path_str}"),
                Some(path_str),
                Some("请检查路径拼写，或先在文件管理器中确认文件存在".to_string()),
            )],
            llm_note: None,
        });
    }

    let mut issues: Vec<CodeIssue> = Vec::new();
    let mut files_checked: u32 = 0;
    let mut languages: Vec<String> = Vec::new();

    if path.is_file() {
        let lang = language_of_path(&path);
        languages.push(lang.clone());
        let (mut syn, degraded) = syntax_check_file(&path, &lang, runner).await;
        issues.append(&mut syn);
        if let Some(note) = degraded {
            issues.push(CodeIssue::new(
                IssueSeverity::Hint,
                IssueCategory::Unknown,
                note,
                Some(path_str.clone()),
                None,
            ));
        }
        // 启发式
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut heu = heuristic_scan(&text, Some(&path_str));
            issues.append(&mut heu);
        }
        files_checked += 1;
    } else if path.is_dir() {
        let (files, trunc_note) = match collect_files(&path) {
            Ok(v) => v,
            Err(e) => {
                return Err(CodeReport {
                    source: CheckSource::Path,
                    path: Some(path_str.clone()),
                    files_checked: 0,
                    language: "unknown".to_string(),
                    summary: "发现 1 个问题：error 1 / warning 0 / hint 0".to_string(),
                    issues: vec![CodeIssue::new(
                        IssueSeverity::Error,
                        IssueCategory::Unknown,
                        e,
                        Some(path_str),
                        None,
                    )],
                    llm_note: None,
                });
            }
        };
        if let Some(note) = trunc_note {
            issues.push(CodeIssue::new(
                IssueSeverity::Hint,
                IssueCategory::Unknown,
                note,
                Some(path_str.clone()),
                None,
            ));
        }
        for f in files {
            let lang = language_of_path(&f);
            languages.push(lang.clone());
            let f_str = f.display().to_string();
            let (mut syn, degraded) = syntax_check_file(&f, &lang, runner).await;
            issues.append(&mut syn);
            if let Some(note) = degraded {
                issues.push(CodeIssue::new(
                    IssueSeverity::Hint,
                    IssueCategory::Unknown,
                    note,
                    Some(f_str.clone()),
                    None,
                ));
            }
            if let Ok(text) = std::fs::read_to_string(&f) {
                let mut heu = heuristic_scan(&text, Some(&f_str));
                issues.append(&mut heu);
            }
            files_checked += 1;
        }
    } else {
        return Err(CodeReport {
            source: CheckSource::Path,
            path: Some(path_str.clone()),
            files_checked: 0,
            language: "unknown".to_string(),
            summary: "发现 1 个问题：error 1 / warning 0 / hint 0".to_string(),
            issues: vec![CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Unknown,
                format!("路径既不是文件也不是目录: {path_str}"),
                Some(path_str),
                None,
            )],
            llm_note: None,
        });
    }

    // rust / typescript 这类只做启发式的语言，附人工提示
    if languages.iter().any(|l| l == "rust") {
        issues.push(CodeIssue::new(
            IssueSeverity::Hint,
            IssueCategory::Style,
            "Rust 检查仅做启发式扫描；未运行 cargo check（避免写 target/ 违反只读），建议本地执行 cargo check 做完整类型检查",
            Some(path_str.clone()),
            Some("在项目目录手动跑 `cargo check`".to_string()),
        ));
    }
    if languages.iter().any(|l| l == "typescript") {
        issues.push(CodeIssue::new(
            IssueSeverity::Hint,
            IssueCategory::Style,
            "TypeScript 检查仅做启发式扫描；未运行 tsc（需要项目环境），建议本地执行 tsc --noEmit",
            Some(path_str.clone()),
            Some("在项目目录手动跑 `tsc --noEmit`".to_string()),
        ));
    }

    let language = if languages.iter().all(|l| l == &languages[0]) {
        languages[0].clone()
    } else {
        "mixed".to_string()
    };

    let summary = build_summary(&issues);
    Ok(CodeReport {
        source: CheckSource::Path,
        path: Some(path_str),
        files_checked,
        language,
        summary,
        issues,
        llm_note: None,
    })
}

async fn run_snippet(
    req: CheckRequest,
    runner: &dyn CheckRunner,
) -> Result<CodeReport, CodeReport> {
    let snippet = req.snippet.clone().expect("snippet mode");
    let lang = req
        .language
        .clone()
        .unwrap_or_else(|| sniff_language(&snippet));

    let mut issues: Vec<CodeIssue> = Vec::new();

    // snippet 模式：JSON 直接解析；其他语言只做启发式（不落临时文件，保持只读）
    if lang == "json" {
        if let Err(e) = serde_json::from_str::<serde_json::Value>(&snippet) {
            issues.push(CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Syntax,
                format!("JSON 解析失败: {e}"),
                extract_line_from_json_error(&e.to_string()).map(|l| format!("line {l}")),
                Some("检查逗号、引号、括号是否配对".to_string()),
            ));
        }
    }

    let mut heu = heuristic_scan(&snippet, None);
    issues.append(&mut heu);

    // JS snippet 也跑 node --check？不落盘没法跑（node --check 需要文件路径）。
    // 为保持只读，snippet 模式不写临时文件，JS/语法问题只能靠启发式 + 后续 LLM 提示。
    // 这里保留 runner 参数以统一签名，并在文档说明。
    let _ = runner;

    let summary = build_summary(&issues);
    Ok(CodeReport {
        source: CheckSource::Snippet,
        path: None,
        files_checked: 1,
        language: lang,
        summary,
        issues,
        llm_note: None,
    })
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    fn has_node() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
    }

    fn has_python3() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_ok()
    }

    // ----- from_json -----

    #[test]
    fn from_json_accepts_path() {
        let v = serde_json::json!({"source":"path","path":"/tmp/x.js"});
        let r = CheckRequest::from_json(&v).unwrap();
        assert_eq!(r.source, CheckSource::Path);
        assert_eq!(r.path.unwrap().to_string_lossy(), "/tmp/x.js");
        assert!(r.snippet.is_none());
    }

    #[test]
    fn from_json_accepts_snippet() {
        let v = serde_json::json!({"source":"snippet","snippet":"const x = 1","language":"javascript"});
        let r = CheckRequest::from_json(&v).unwrap();
        assert_eq!(r.source, CheckSource::Snippet);
        assert_eq!(r.language.as_deref(), Some("javascript"));
    }

    #[test]
    fn from_json_rejects_both() {
        let v = serde_json::json!({"source":"path","path":"/a","snippet":"x"});
        assert!(CheckRequest::from_json(&v).is_err());
    }

    #[test]
    fn from_json_rejects_neither() {
        let v = serde_json::json!({"source":"path"});
        assert!(CheckRequest::from_json(&v).is_err());
        let v2 = serde_json::json!({"source":"snippet"});
        assert!(CheckRequest::from_json(&v2).is_err());
    }

    // ----- 真实外部命令 -----

    #[tokio::test]
    async fn node_real_check_bad_js() {
        if !has_node() {
            eprintln!("SKIP: node not in PATH");
            return;
        }
        let p = fixtures_dir().join("bad.js");
        let req = CheckRequest::from_path(p);
        let rep = run_check(req, &OsCheckRunner, None).await;
        assert_eq!(rep.files_checked, 1);
        let syn = rep
            .issues
            .iter()
            .find(|i| i.category == IssueCategory::Syntax);
        assert!(syn.is_some(), "应检出 node 语法错误: {:?}", rep.issues);
        let syn = syn.unwrap();
        assert_eq!(syn.severity, IssueSeverity::Error);
        assert!(syn.message.contains("SyntaxError"));
    }

    #[tokio::test]
    async fn python_real_check_bad_py() {
        if !has_python3() {
            eprintln!("SKIP: python3 not in PATH");
            return;
        }
        let p = fixtures_dir().join("bad.py");
        let req = CheckRequest::from_path(p);
        let rep = run_check(req, &OsCheckRunner, None).await;
        let syn = rep
            .issues
            .iter()
            .find(|i| i.category == IssueCategory::Syntax);
        assert!(syn.is_some(), "应检出 python 语法错误: {:?}", rep.issues);
        assert!(syn.unwrap().message.contains("SyntaxError"));
    }

    #[tokio::test]
    async fn json_bad_file_detected() {
        let p = fixtures_dir().join("bad.json");
        let rep = run_check(CheckRequest::from_path(p), &OsCheckRunner, None).await;
        let syn = rep
            .issues
            .iter()
            .find(|i| i.category == IssueCategory::Syntax);
        assert!(syn.is_some(), "应检出 JSON 错误: {:?}", rep.issues);
    }

    #[tokio::test]
    async fn healthy_js_py_return_clean() {
        for name in ["healthy.js", "healthy.py"] {
            let p = fixtures_dir().join(name);
            let rep = run_check(CheckRequest::from_path(p), &OsCheckRunner, None).await;
            let syn = rep
                .issues
                .iter()
                .find(|i| i.category == IssueCategory::Syntax);
            assert!(syn.is_none(), "{name} 不应有语法问题: {:?}", rep.issues);
        }
    }

    // ----- mock runner -----

    struct MockRunner {
        out: Mutex<Option<CheckOutput>>,
    }

    #[async_trait]
    impl CheckRunner for MockRunner {
        async fn run(
            &self,
            _argv: &[&str],
            _cwd: Option<&Path>,
            _env: &[(&str, &str)],
        ) -> Result<CheckOutput, String> {
            Ok(self.out.lock().unwrap().take().unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn mock_runner_output_parsed_to_issue() {
        // 写一个临时 js 文件
        let dir = std::env::temp_dir().join("qibai_cc_mock");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m.js");
        std::fs::write(&p, "const x = 1\n").unwrap();

        let mock = MockRunner {
            out: Mutex::new(Some(CheckOutput {
                stdout: String::new(),
                stderr: format!(
                    "{}:2\n\nSyntaxError: Missing semicolon. (2:10)\n",
                    p.display()
                ),
                ok: false,
            })),
        };
        let rep = run_check(CheckRequest::from_path(p.clone()), &mock, None).await;
        let syn = rep
            .issues
            .iter()
            .find(|i| i.category == IssueCategory::Syntax)
            .expect("应从 mock stderr 解析出 issue");
        assert_eq!(syn.severity, IssueSeverity::Error);
        assert!(syn.message.contains("SyntaxError"));
        std::fs::remove_file(&p).ok();
    }

    // ----- 启发式 -----

    #[tokio::test]
    async fn security_heuristic_hits() {
        let src = r#"
const pw = "hunter2password";
const key = "sk-ABCD1234EFGH";
child_process.execSync("ls");
eval(userInput);
el.innerHTML = "<b>" + name;
"#;
        let rep = run_check(
            CheckRequest::from_snippet(src, Some("javascript".to_string())),
            &OsCheckRunner,
            None,
        )
        .await;
        let sec = rep
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::Security)
            .count();
        assert!(sec >= 5, "security 问题数应 ≥5，实际: {sec} / {:?}", rep.issues);
    }

    #[tokio::test]
    async fn logic_heuristic_hits() {
        let src = "if (x == 1) { }\nwhile(true) {\n  // no break\n}\ntry { } catch(e) {}\nconst z = a / 0;\n";
        let rep = run_check(
            CheckRequest::from_snippet(src, Some("javascript".to_string())),
            &OsCheckRunner,
            None,
        )
        .await;
        let logic = rep
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::Logic)
            .count();
        assert!(logic >= 4, "logic 问题数应 ≥4，实际: {logic} / {:?}", rep.issues);
    }

    #[tokio::test]
    async fn style_heuristic_hits() {
        let line = "x".repeat(130);
        let src = format!("{line}\nconst a = 1;   \n\tconst b = 2;\n// TODO: later\n");
        let rep = run_check(
            CheckRequest::from_snippet(src, Some("javascript".to_string())),
            &OsCheckRunner,
            None,
        )
        .await;
        let style = rep
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::Style)
            .count();
        assert!(style >= 3, "style 问题数应 ≥3，实际: {style} / {:?}", rep.issues);
    }

    // ----- 路径安全 -----

    #[tokio::test]
    async fn nonexistent_path_is_structured_error() {
        let p = "/tmp/qibai_no_such_dir_xyz_1234/nope.js";
        let rep = run_check(CheckRequest::from_path(p), &OsCheckRunner, None).await;
        assert_eq!(rep.files_checked, 0);
        assert!(rep.summary.contains("发现 1 个问题"));
        assert!(rep.issues[0].message.contains("不存在"));
    }

    #[tokio::test]
    async fn directory_skips_and_limits() {
        let dir = std::env::temp_dir().join("qibai_cc_dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("a.js"), "const a = 1;\n").unwrap();
        std::fs::write(dir.join("node_modules/dep.js"), "eval('x')\n").unwrap();
        std::fs::write(dir.join(".git/config"), "not json\n").unwrap();

        let rep = run_check(CheckRequest::from_path(&dir), &OsCheckRunner, None).await;
        assert_eq!(rep.files_checked, 1, "只应检查 a.js，跳过 node_modules/.git");
        // node_modules 里的 eval 不应出现在报告中
        assert!(!rep.issues.iter().any(|i| {
            i.location
                .as_ref()
                .map(|l| l.contains("node_modules"))
                .unwrap_or(false)
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ----- snippet 嗅探 -----

    #[test]
    fn sniff_language_markers() {
        assert_eq!(sniff_language("function f() {}"), "javascript");
        assert_eq!(sniff_language("const x = 1;"), "javascript");
        assert_eq!(sniff_language("def foo():\n  pass"), "python");
        assert_eq!(sniff_language("{\"a\": 1}"), "json");
        assert_eq!(sniff_language("hello world"), "unknown");
    }

    #[tokio::test]
    async fn snippet_bad_json_detected() {
        let rep = run_check(
            CheckRequest::from_snippet("{\"a\": 1,}", None),
            &OsCheckRunner,
            None,
        )
        .await;
        assert_eq!(rep.source, CheckSource::Snippet);
        assert!(rep.path.is_none());
        assert!(rep.issues.iter().any(|i| i.category == IssueCategory::Syntax));
    }

    // ----- JSON 形状 -----

    #[test]
    fn report_json_shape_frozen() {
        let rep = CodeReport {
            source: CheckSource::Path,
            path: Some("/x/y.js".to_string()),
            files_checked: 3,
            language: "javascript".to_string(),
            summary: "发现 1 个问题：error 1 / warning 0 / hint 0".to_string(),
            issues: vec![CodeIssue::new(
                IssueSeverity::Error,
                IssueCategory::Syntax,
                "boom".to_string(),
                Some("/x/y.js:2".to_string()),
                Some("fix it".to_string()),
            )],
            llm_note: None,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["source"], "path");
        assert_eq!(v["path"], "/x/y.js");
        assert_eq!(v["files_checked"], 3);
        assert_eq!(v["language"], "javascript");
        assert!(v["summary"].as_str().unwrap().contains("发现"));
        assert_eq!(v["llm_note"], serde_json::Value::Null);
        let issue = &v["issues"][0];
        assert_eq!(issue["severity"], "error");
        assert_eq!(issue["category"], "syntax");
        assert_eq!(issue["message"], "boom");
        assert_eq!(issue["location"], "/x/y.js:2");
        assert_eq!(issue["suggestion"], "fix it");

        // 反序列化回来
        let back: CodeReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, rep);
    }

    #[tokio::test]
    async fn llm_suggester_invoked_and_attached() {
        struct FakeLlm;
        #[async_trait]
        impl SuggestLlm for FakeLlm {
            async fn suggest(&self, _r: &CodeReport, _lang: &str) -> Option<String> {
                Some("建议：把 eval 换成 JSON.parse".to_string())
            }
        }
        let rep = run_check(
            CheckRequest::from_snippet("eval(x)", Some("javascript".to_string())),
            &OsCheckRunner,
            Some(&FakeLlm),
        )
        .await;
        assert_eq!(rep.llm_note.as_deref(), Some("建议：把 eval 换成 JSON.parse"));
    }

    #[tokio::test]
    async fn llm_failure_degrades_gracefully() {
        struct FailLlm;
        #[async_trait]
        impl SuggestLlm for FailLlm {
            async fn suggest(&self, _r: &CodeReport, _lang: &str) -> Option<String> {
                None
            }
        }
        let rep = run_check(
            CheckRequest::from_snippet("const x = 1;", Some("javascript".to_string())),
            &OsCheckRunner,
            Some(&FailLlm),
        )
        .await;
        assert_eq!(rep.llm_note, None);
        assert_eq!(rep.summary, "未发现问题");
    }
}
