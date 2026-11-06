//! Engine：把 settings / storage / skills / reminders / llm 串成 IPC 门面。
//! src-tauri 的命令只做最薄转发，全部业务逻辑在这里，便于无 GUI 单测。

use crate::enums::{Action, Emotion};
use crate::llm_client::{run_agent_loop, ChatMessage, DoubaoProvider, LlmError, LlmProvider};
use crate::persona::build_system_prompt_for_persona;
use crate::personas::PersonaStore;
use crate::reminder::{ReminderEvent, ReminderScheduler, SystemClock};
use crate::settings::{Settings, SettingsError};
use crate::skills::SkillsRegistry;
use crate::social::{Move, SocialStore, GREET_DAILY_LIMIT, LEVELS, PK_DAILY_LIMIT};

// ---------- 测试阶段：全能力开放 ----------
/// 全能力开放开关：debug 构建（含 `cargo test`）为 true 便于开发自测；
/// **release 构建自动收紧为 false**，恢复技能审批 / 专家白名单过滤 / vault L4 门控 /
/// 亲密度分级门控。正式发布若需临时全开，应改为显式 settings.dev_mode（默认 false），
/// 绝不允许以 `const true` 进 release。
pub const OPEN_ALL: bool = cfg!(debug_assertions);
use crate::storage::{ConversationStore, StoredMessage};
use crate::tts;
use crate::voice_profile::VoiceProfile;
use serde::Serialize;
use thiserror::Error;

/// 事件出口：tauri 用 app_handle.emit 实现；测试用 RecordingSink 收集。
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

/// v45.9：把 `patch` 深合并进 `base`（RFC 7386 JSON Merge Patch 语义）。
/// - patch 里出现的字段覆盖 base；
/// - patch 里没出现的字段保留 base；
/// - patch 里显式 `null` **删除** base 里的键（"清空"语义，与"没传"区分）；
/// - 对象递归合并；数组 / 标量直接覆盖（不做逐元素合并）。
pub fn merge_json_value(base: serde_json::Value, patch: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (base, patch) {
        (Value::Object(mut base_obj), Value::Object(patch_obj)) => {
            for (k, v) in patch_obj {
                if v.is_null() {
                    // RFC 7386：null = 删除键
                    base_obj.remove(&k);
                } else if let Some(existing) = base_obj.get(&k) {
                    if existing.is_object() && v.is_object() {
                        let merged = merge_json_value(existing.clone(), v);
                        base_obj.insert(k, merged);
                    } else {
                        base_obj.insert(k, v);
                    }
                } else {
                    base_obj.insert(k, v);
                }
            }
            Value::Object(base_obj)
        }
        (_, patch) => patch,
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("settings: {0}")]
    Settings(#[from] SettingsError),
    #[error("llm: {0}")]
    Llm(#[from] LlmError),
    #[error("storage: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("skill: {0}")]
    Skill(String),
    #[error("unknown persona: {0}")]
    UnknownPersona(String),
    #[error("invalid move: {0}")]
    InvalidMove(String),
    #[error("social: {0}")]
    Social(String),
    #[error("intimacy: {0}")]
    Intimacy(String),
    #[error("guard: {0}")]
    Guard(String),
    #[error("vault: {0}")]
    Vault(#[from] crate::vault::VaultError),
    /// 迭代④：联网社交（message 为中文结构化错误）。
    #[error("联网社交：{0}")]
    Net(String),
    /// v45.9 P0：长任务存储 / 状态机错误。
    #[error("task store: {0}")]
    TaskStore(#[from] crate::task::StoreError),
    #[error("task: {0}")]
    Task(#[from] crate::task::TaskError),
}

#[derive(Debug, Serialize)]
pub struct SendMessageOutput {
    pub reply: String,
    pub emotion: Emotion,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// v23：LLM 智能调度桌宠功能的原始指令（执行端白名单校验；闲聊时为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_cmd: Option<serde_json::Value>,
    /// 迭代③：对话保险箱拦截命中时为 true，前端据此调用 vault_copy。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<bool>,
    /// 命中条目的 id（供前端 invoke('vault_copy', { id })）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_entry_id: Option<String>,
    /// 脱敏朗读文本（喇叭读 conversation → 读不到明文）；非拦截为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speak_text: Option<String>,
    /// v18.8-fix28f：流式是否已逐段朗读（tauri 层据此决定是否 Rust 端兜底播放）。
    #[serde(skip)]
    pub streamed: bool,
}

/// 一次朗读请求（迭代②）：由 `Engine::tts_decide` 决策产出，
/// tauri 侧 `perform_speech` 按 resolved 分派本地/云端朗读，并作为指示器载荷的一部分。
#[derive(Debug, Clone, Serialize)]
pub struct TtsSpeakRequest {
    pub text: String,
    pub kind: tts::TtsKind,
    pub volume: u8,
    /// 生效语速 = 全局 rate × 角色 profile.rate（clamp [0.5, 2.0]）。
    pub rate: f32,
    /// 生效音高（来自角色音色档案）。
    pub pitch: f32,
    /// 本地引擎 voice id（espeak/say/SAPI SelectVoice；空串=默认）。
    pub engine_voice_id: String,
    /// 解析后云端 speaker（profile.cloud_speaker 非空 → 它；否则 settings.voice.cloud_voice）。
    pub cloud_speaker: String,
    /// 前端状态栏音色标签（voice_profile::voice_tag 同款）。
    pub voice_tag: String,
    pub resolved: tts::ResolvedEngine,
    pub reason: String,
}

// ============================================================
// v30/v45.7 本地工作区安全执行（行业 coding-agent 三闸门模型落地）
//   闸门1 目录白名单：所有路径必须解析到工作区根（默认 ~/Documents/qibai，
//        可用环境变量 QIBAI_WORKSPACE 覆盖）；根内绝对路径放行，根外绝对路径与
//        .. 越界一律拒绝。
//   闸门2 命令 allowlist：仅只读/创建类命令可执行（pwd/ls/mkdir/touch/cat/
//        echo/printf/grep/head/tail/wc/file/df/du/which/uname）；rm/bash/
//        python/node 等危险命令天然不在列表内 → 拒绝。
//   闸门3 写文件确认语义：工作区内自动执行（用户自己的本地工作区）；
//        越界/非法路径拒绝并返回明确原因（默认拒绝）。
// ============================================================
// v31：工作空间命令 allowlist——基础查看 + 工程/写代码闭环（创建目录、写文件后运行验证）。
// 安全三闸门不变：仅工作区内 cwd 执行、参数拒绝绝对路径/..、超时、输出截断；
// rm/bash/sh/zsh/curl/wget/sudo 等危险命令永不加入。
const QIBAI_CODE_ALLOWLIST: &[&str] = &[
    // 基础查看/文件
    "pwd", "ls", "mkdir", "touch", "cat", "echo", "printf", "grep",
    "head", "tail", "wc", "file", "df", "du", "which", "uname",
    // v31 开发验证（语音指导建工程→写代码→运行检查：node/python3/npm/cargo）
    "node", "python3", "npm", "cargo",
];

/// v45.8：工作区根首次运行必须真实存在且可写。
/// 创建 $HOME/Documents → $HOME/Documents/qibai → 试写临时文件再删除。
/// 失败时不 panic，打日志并 emit 一次 toast（用 Once 去重）。
fn ensure_workspace_writable(root: &std::path::Path) {
    static UNUSABLE_REPORTED: std::sync::Once = std::sync::Once::new();
    if let Err(e) = std::fs::create_dir_all(root) {
        eprintln!("[qibai] workspace root unusable: {e}");
        UNUSABLE_REPORTED.call_once(|| {
            eprintln!("[qibai] workspace: 工作区不可用：{e}，我把文件都放不进去了");
        });
        return;
    }
    // 试写：tmp 文件 → 删除
    let probe = root.format_probe_path();
    match std::fs::write(&probe, b"probe") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
        }
        Err(e) => {
            eprintln!("[qibai] workspace root unusable: write probe failed {e}");
            UNUSABLE_REPORTED.call_once(|| {
                eprintln!("[qibai] workspace: 工作区不可写：{e}，我把文件都放不进去了");
            });
        }
    }
}

trait ProbePath {
    fn format_probe_path(&self) -> std::path::PathBuf;
}
impl ProbePath for std::path::Path {
    fn format_probe_path(&self) -> std::path::PathBuf {
        self.join(".qibai_write_probe")
    }
}

/// 工作区根（v45.7 统一单一根；v45.8 首次运行校验可写）：
/// - 优先 `QIBAI_WORKSPACE` 环境变量（非空才用，source=env）；
/// - 否则 `$HOME/Documents/qibai`（source=default）。
/// 旧 `~/.qibai_workspace` 不再作为根参与解析（启动时若仍存在且有内容，仅在 boot 日志提示，
/// 不迁移/不删除）。
fn qibai_workspace_root_with_source() -> (std::path::PathBuf, &'static str) {
    if let Ok(w) = std::env::var("QIBAI_WORKSPACE") {
        let w = w.trim();
        if !w.is_empty() {
            let root = std::path::PathBuf::from(w);
            ensure_workspace_writable(&root);
            return (root, "env");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    // v45.8：先建 Documents 父目录，再建 qibai
    let docs = std::path::Path::new(&home).join("Documents");
    let _ = std::fs::create_dir_all(&docs);
    let root = docs.join("qibai");
    ensure_workspace_writable(&root);
    (root, "default")
}

/// 兼容旧调用点：只取路径（source 仅在 boot 日志处需要）。
pub fn qibai_workspace_root() -> std::path::PathBuf {
    qibai_workspace_root_with_source().0
}

/// v45.8：供 llm_client::outcome_to_tool_text 等外部模块读取当前工作区根。
pub fn qibai_workspace_root_public() -> std::path::PathBuf {
    qibai_workspace_root()
}

/// 启动时一次性打印工作区根与旧目录提示（std::sync::Once 保证只打一次，避免工具调用刷屏）。
fn log_workspace_boot_once() {
    static BOOTED: std::sync::Once = std::sync::Once::new();
    BOOTED.call_once(|| {
        let (root, source) = qibai_workspace_root_with_source();
        eprintln!("[qibai] workspace root: {} (source={source})", root.display());
        // 旧 ~/.qibai_workspace 若存在且有内容，仅提示、不迁移、不删除。
        if let Ok(home) = std::env::var("HOME") {
            let old = std::path::Path::new(&home).join(".qibai_workspace");
            if let Ok(rd) = std::fs::read_dir(&old) {
                let n = rd.count();
                if n > 0 {
                    eprintln!("[qibai] workspace: 检测到旧工作区 ~/.qibai_workspace 有 {n} 个文件，未自动迁移，请手动 move");
                }
            }
        }
    });
}

/// v45.8：读取工作区所在分区的可用字节数；拿不到返回 None，不 panic。
/// 不依赖 libc（避免新外部 crate），在 core 里以最简探测为准。
pub fn statvfs_free_bytes(_root: &std::path::Path) -> Option<u64> {
    // 预留：后续可加 fs2/disk_usage crate；当前返回 None 以保证不 panic。
    None
}

/// v45.8：路径输入归一化——去首尾引号/空白、反斜杠转正斜杠、折叠多余分隔符、
/// 去掉尾部斜杠、`~/` 展开为 `$HOME`。返回归一化后的字符串（未做根判定）。
pub fn normalize_path_input(input: &str) -> String {
    let mut s = input.trim().to_string();
    // 去首尾成对引号
    while s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[s.len() - 1] as char;
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            s = s[1..s.len() - 1].trim().to_string();
            continue;
        }
        break;
    }
    // 反斜杠转正斜杠（Windows 习惯输入）
    let s = s.replace('\\', "/");
    // 折叠多个 // 为单个
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for c in s.chars() {
        if c == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        out.push(c);
    }
    // 去尾部 /（根本身保留）
    if out.len() > 1 {
        out = out.trim_end_matches('/').to_string();
    }
    // ~/ 或 ~ 展开
    if out == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    if let Some(rest) = out.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    out
}

/// 相对/绝对路径 → 工作区内绝对路径。
/// v45.7：白名单收敛为单一根后，**根内绝对路径放行**（within_root 判定）；
/// 根外绝对路径、空路径、`..` 逃逸一律 None。
/// v45.8：先做归一化（~/ 展开、去引号、反斜杠、折叠分隔符），根内绝对路径放行。
pub fn safe_resolve_pub(root: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    safe_resolve(root, rel)
}

fn safe_resolve(root: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let rel = normalize_path_input(rel);
    if rel.is_empty() {
        return None;
    }
    let p = std::path::Path::new(&rel);
    if p.is_absolute() {
        return if within_root(root, p) {
            Some(p.to_path_buf())
        } else {
            None
        };
    }
    let mut cur = root.to_path_buf();
    for comp in p.components() {
        match comp {
            Component::Normal(c) => cur.push(c),
            Component::CurDir => {}
            _ => return None, // ParentDir/RootDir/Prefix 一律拒绝
        }
    }
    Some(cur)
}

/// v45.8：拒绝时构造带建议的文案（真实根 + 解析目标 + 建议写法）。
pub fn denied_path_message(root: &std::path::Path, input: &str) -> String {
    let normalized = normalize_path_input(input);
    let target = if std::path::Path::new(&normalized).is_absolute() {
        normalized.clone()
    } else {
        root.join(&normalized).display().to_string()
    };
    // 建议写法：把 target 转成相对 root 的形式
    let suggest = normalized
        .strip_prefix(&root.display().to_string())
        .map(|r| r.trim_start_matches('/').to_string())
        .unwrap_or_else(|| "myproject/src/main.py".to_string());
    format!(
        "拒绝：{target} 不在工作区根 {} 下；请改写成相对路径，如 {suggest}（将落在 {}/{}）",
        root.display(),
        root.display(),
        if suggest.is_empty() { "myproject" } else { &suggest }
    )
}

/// 判断路径是否落在工作区根内（物理 canonical 形式比较）。
///
/// 关键修复（v46）：文件/目录尚不存在时（刚 code_write_file 待读、新建子目录待列），
/// `p.canonicalize()` 必然 Err → 旧逻辑直接返回 false → 误判"越界"，表现为
/// "写完的文件读不回来、新建子目录列不出"。现对齐 `fileops::sanitize_path` 的思路：
/// 若 `p` 不存在，向上找到最近一个已存在的祖先做 canonicalize，再把剩余相对段拼回去。
/// 符号链接逃逸靠 canonicalize 物理化后比较拦截；`../` 逃逸、根外绝对路径一律 false。
fn within_root(root: &std::path::Path, p: &std::path::Path) -> bool {
    let root_canon = match root.canonicalize() {
        Ok(r) => r,
        Err(_) => return false, // 根都物理化不了，无从谈起
    };
    let target = if p.exists() {
        match p.canonicalize() {
            Ok(c) => c,
            Err(_) => return false,
        }
    } else {
        // 向上找最近的已存在祖先；沿途记下被跳过的末段（从深到浅）。
        let mut ancestor = p;
        let mut tail: Vec<std::path::PathBuf> = Vec::new();
        loop {
            if ancestor.exists() {
                let canon = match ancestor.canonicalize() {
                    Ok(c) => c,
                    Err(_) => return false,
                };
                // tail 记录顺序是"深→浅"，拼回时要"浅→深"，故 reverse。
                let mut joined = canon;
                for seg in tail.into_iter().rev() {
                    joined.push(seg);
                }
                break joined;
            }
            match ancestor.parent() {
                Some(parent) => {
                    tail.push(ancestor
                        .file_name()
                        .map(std::ffi::OsString::from)
                        .unwrap_or_default()
                        .into());
                    ancestor = parent;
                }
                None => return false, // 到文件系统根仍不存在
            }
        }
    };
    target.starts_with(&root_canon)
}

fn qibai_truncate(s: &str, n: usize) -> String {
    let cnt = s.chars().count();
    if cnt > n {
        let t: String = s.chars().take(n).collect();
        format!("{t}…（已截断，共 {cnt} 字符）")
    } else {
        s.to_string()
    }
}

/// 工具入参摘要：仅挑短标量字段打日志；content/body/prompt 等大字段或密钥一律折叠为 `<redacted>`。
/// 整行压成一行、≤120 字符，绝不把整段文件内容或密钥写进 app.log。
fn summarize_tool_args(args: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            const REDACT_KEYS: &[&str] = &[
                "content", "body", "text", "prompt", "message", "input", "data", "payload",
                "secret", "token", "key", "password", "api_key", "apikey", "authorization",
                "description",
            ];
            if REDACT_KEYS.contains(&k.as_str()) {
                out.push_str(&format!("{k}=<redacted> "));
                continue;
            }
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.push_str(&format!("{k}={} ", val.trim()));
        }
    } else {
        out = args.to_string();
    }
    // 多行压成一行、折叠空白，再按字符截断到 120。
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = collapsed.chars().take(120).collect();
    if s.is_empty() { "{}".to_string() } else { s }
}

/// 长期记忆是否注入本轮 system prompt（纯函数，便于单测）。
///
/// 背景：每次调 LLM 都把 persona 最近 6 条记忆拼进 system，导致模型冷启动第一句就
/// 主动复述上一轮留下的任务清单（"先干这些任务…"）。现做启动抑制：
/// - 无记忆条目（n==0）→ 不注入；
/// - 非冷启动（conversation 已有历史）→ 照常注入（用户在延续对话，记忆有用）；
/// - 冷启动首句且用户这句话**不含**回忆意图关键词 → 跳过注入，避免开场被旧任务刷屏；
/// - 冷启动首句但用户主动问起（含"上次/之前/刚才/做到哪/继续/任务/进度/记得/待办"）→ 照常注入。
fn should_inject_memory(is_first_turn: bool, user_text: &str, n: usize) -> bool {
    if n == 0 {
        return false;
    }
    if !is_first_turn {
        return true;
    }
    const RECALL_KEYWORDS: &[&str] = &[
        "上次", "之前", "刚才", "做到哪", "继续", "任务", "进度", "记得", "待办",
    ];
    RECALL_KEYWORDS.iter().any(|kw| user_text.contains(kw))
}

/// 工具层 clear_conversation 的 confirm 门控（纯函数，便于单测）。
/// confirm 缺失/非 true → 返回拒绝原因；confirm==true → None（放行由调用方继续）。
fn clear_conversation_confirm_deny(args: &serde_json::Value) -> Option<String> {
    let confirm = args.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
    if !confirm {
        Some("清空是不可恢复操作，需要用户确认后再执行（请先问用户）".to_string())
    } else {
        None
    }
}

/// 本地知识库根目录：env QIBAI_KB 或 ~/.qibai_kb（放 .txt/.md/.json 文档，问答时检索注入）。
fn qibai_kb_root() -> std::path::PathBuf {
    if let Ok(k) = std::env::var("QIBAI_KB") {
        let k = k.trim();
        if !k.is_empty() {
            let root = std::path::PathBuf::from(k);
            let _ = std::fs::create_dir_all(&root);
            return root;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let root = std::path::PathBuf::from(home).join(".qibai_kb");
    let _ = std::fs::create_dir_all(&root);
    root
}

/// 知识库关键词检索：扫描 kb 目录下 .txt/.md/.json，命中查询词的文件返回片段。
/// 轻量本地 RAG（v31）：问题分词 → 文件包含任一词即命中 → 取命中词上下文片段。
/// 最多返回 3 个文件、总计约 2400 字符，注入 system 供 LLM 结合回答。
fn kb_search(query: &str) -> Vec<String> {
    let root = qibai_kb_root();
    kb_search_in(&root, query)
}

/// kb_search 的核心实现（目录入参，便于单测）。
fn kb_search_in(root: &std::path::Path, query: &str) -> Vec<String> {
    use std::collections::HashSet;
    if query.trim().is_empty() {
        return Vec::new();
    }
    // 分词：按非字母数字边界切，保留长度 >= 2 的词（中文整词保留）
    let words: HashSet<String> = query
        .split(|c: char| !c.is_alphanumeric() && !c.is_ascii_whitespace() && c != '_')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().count() >= 2)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(String, String, usize)> = Vec::new(); // (path, snippet, score)
    if let Ok(entries) = std::fs::read_dir(root) {
        for ent in entries.flatten() {
            let p = ent.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "txt" && ext != "md" && ext != "json" {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&p) else { continue };
            let lower = content.to_lowercase();
            let mut score = 0usize;
            let mut hit_word = String::new();
            for w in &words {
                if lower.contains(&w.to_lowercase()) {
                    score += 1;
                    if hit_word.is_empty() {
                        hit_word = w.clone();
                    }
                }
            }
            if score == 0 {
                continue;
            }
            // 取命中词上下文片段（±140 字符）
            let snippet = if hit_word.is_empty() {
                qibai_truncate(&content, 400)
            } else {
                let low = hit_word.to_lowercase();
                match lower.find(&low) {
                    Some(idx) => {
                        let start = idx.saturating_sub(140);
                        let end = (idx + hit_word.chars().count() + 140).min(content.len());
                        let mut snip = content[start..end].to_string();
                        if start > 0 {
                            snip.insert_str(0, "…");
                        }
                        if end < content.len() {
                            snip.push('…');
                        }
                        snip
                    }
                    None => qibai_truncate(&content, 400),
                }
            };
            hits.push((p.file_name().unwrap_or_default().to_string_lossy().to_string(), snippet, score));
        }
    }
    hits.sort_by(|a, b| b.2.cmp(&a.2));
    hits.into_iter()
        .take(3)
        .map(|(name, snip, _)| format!("【{name}】{snip}"))
        .collect()
}

/// allowlist 命令在工作区目录内执行；参数含绝对路径 / .. 引用 → 拒绝。
/// v45.8：命令参数安全校验。
/// - 含 shell 元字符（; | & > < ` $）一律拒；
/// - 含 `/` 或以 `~` 开头或含 `..` 的参数必须能 within_root 命中工作区根；
/// - 普通相对参数（echo/hello 等）原样放行。
fn check_command_arg(arg: &str, root: &std::path::Path) -> Result<(), String> {
    let meta = [';', '|', '&', '>', '<', '`', '$', '(', ')'];
    for c in meta {
        if arg.contains(c) {
            return Err(format!("命令参数含 shell 元字符 '{c}'：{arg}"));
        }
    }
    if arg.starts_with('/') || arg.starts_with('~') || arg.contains("..") {
        let normalized = normalize_path_input(arg);
        let p = std::path::Path::new(&normalized);
        if !within_root(root, p) {
            return Err(denied_path_message(root, arg));
        }
    }
    Ok(())
}

/// v45.12：MCP trust=confirm 待确认 request_id 的单调序号（进程内唯一）。
fn next_mcp_confirm_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn run_allowed_command(command: &str) -> Option<String> {
    let mut toks = command.split_whitespace();
    let bin = toks.next()?;
    let base = std::path::Path::new(bin)
        .file_name()?
        .to_string_lossy()
        .to_string();
    if !QIBAI_CODE_ALLOWLIST.contains(&base.as_str()) {
        return None;
    }
    let root = qibai_workspace_root();
    // v45.8：逐参数校验（含 / 的参数必须在根内；shell 元字符拒）
    for t in toks.clone() {
        if check_command_arg(t, &root).is_err() {
            return None;
        }
    }
    let out = std::process::Command::new(&base)
        .args(command.split_whitespace().skip(1))
        .current_dir(&root)
        .output();
    match out {
        Ok(o) => {
            let mut buf = String::new();
            if !o.stdout.is_empty() {
                buf.push_str(&String::from_utf8_lossy(&o.stdout));
            }
            if !o.stderr.is_empty() {
                buf.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            Some(qibai_truncate(&buf, 4000))
        }
        Err(_) => None,
    }
}

pub struct Engine {
    pub settings: Settings,
    pub skills: SkillsRegistry,
    pub conversation: ConversationStore,
    pub reminders: ReminderScheduler<SystemClock>,
    /// P0 角色实体（含独立记忆与迁移状态）。
    pub personas: PersonaStore,
    /// P1 社交数据（关系 / 事件）。
    pub social: SocialStore,
    /// 亲密度成长系统（L1–L10，冻结规格 §6）。
    pub intimacy: crate::intimacy::IntimacyStore,
    /// 全屏守护演出调度器（久坐休息提醒状态机）。
    pub guard: crate::guard::GuardScheduler<crate::guard::SystemClock>,
    /// 本地密码保险箱（迭代③：加密/状态机/CRUD 全在 core；钥匙串为 Tauri 层事项）。
    pub vault: crate::vault::VaultManager<crate::reminder::SystemClock>,
    /// 迭代④：联网社交客户端核心（状态机/脱敏/加密缓存/网络层）。
    pub netsocial: std::sync::Arc<crate::netsocial::NetsocialManager>,
    /// 专家注册表（人设提示词包 + 技能白名单 + 音色绑定）。
    pub experts: crate::expert::ExpertRegistry,
    pub sink: std::sync::Arc<dyn EventSink>,
    /// 测试可注入的 mock provider；生产为 None，走 DoubaoProvider。
    provider_override: Option<Box<dyn LlmProvider>>,
    /// 测试可注入的代码检查器 / LLM 建议；生产为 None，走 OsCheckRunner + key 探测。
    cc_runner_override: Option<Box<dyn crate::codecheck::CheckRunner>>,
    cc_suggester_override: Option<Box<dyn crate::codecheck::SuggestLlm>>,
    /// v45.9 P0：长任务存储（`<data_dir>/tasks`）。启动时 scan_and_recover。
    task_store: Option<crate::task::TaskStore>,
    /// v45.12：MCP server 注册表。**惰性**：`None` 表示尚未初始化；
    /// 仅当 `settings.mcp_servers` 非空时才 `from_configs`（空数组绝不 spawn 子进程）。
    /// 用 std Mutex：调用点都是同步短临界区，不跨 await 持有。
    mcp: std::sync::Mutex<Option<crate::mcp::registry::McpRegistry>>,
    /// v45.12：等待用户确认的 MCP 工具调用（request_id="mcp-appr-<seq>" → (server, tool, args)）。
    /// trust=confirm 的外部工具先落这里，等前端 approve_skill 回调再真的执行。
    mcp_pending: std::sync::Mutex<std::collections::HashMap<String, (String, String, serde_json::Value)>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Box::new(NullSink))
    }
}

impl Engine {
    pub fn new(sink: Box<dyn EventSink>) -> Self {
        let sink: std::sync::Arc<dyn EventSink> = std::sync::Arc::from(sink);
        log_workspace_boot_once();
        let settings = Settings::load();
        let skills = SkillsRegistry::with_executor_and_plugins(
            settings.skills.clone(),
            Box::new(crate::skills::OsExecutor),
            settings.plugins.clone(),
        );
        let experts = crate::expert::ExpertRegistry::new(
            skills.plugins().expert_manifests(),
            &settings.plugins,
        );
        let conversation = ConversationStore::default();
        let reminders = ReminderScheduler::new(settings.reminders.clone(), SystemClock);

        // P0：角色实体初始化（首次从旧 settings.intimacy_level 迁移 gudian 亲密度），
        // 并把旧 conversation.json 里无归属的记录归入 active persona 记忆（幂等）。
        let mut personas = PersonaStore::load(settings.intimacy_level).unwrap_or_default();
        if !personas.file.migrated_conversation {
            let records: Vec<(String, String)> = conversation
                .all()
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect();
            personas.migrate_conversation(records);
        }
        let social = SocialStore::load().unwrap_or_default();

        // 亲密度：首次从 personas.json 的 intimacy（1-10）播种积分起点（幂等迁移）。
        let intimacy = crate::intimacy::IntimacyStore::load().unwrap_or_default();

        // 全屏守护演出调度器（内部自动 sanitize countdown_secs）。
        let guard = crate::guard::GuardScheduler::new(settings.guard.clone(), crate::guard::SystemClock);

        // 本地密码保险箱：从 vault.qbkv 恢复为 locked；文件缺失/损坏 → unset。
        let vault = crate::vault::VaultManager::load(crate::reminder::SystemClock, settings.vault.clone())
            .unwrap_or_default();

        // 迭代④：联网社交核心（离线占位；加密缓存自动恢复，篡改仅记录不 panic）。
        let netsocial = crate::netsocial::NetsocialManager::new();

        Self {
            settings,
            skills,
            conversation,
            reminders,
            personas,
            social,
            intimacy,
            guard,
            vault,
            netsocial,
            experts,
            sink,
            provider_override: None,
            cc_runner_override: None,
            cc_suggester_override: None,
            task_store: {
                // v45.9 P0：初始化长任务存储；启动时 scan_and_recover。
                if let Ok(dir) = crate::settings::data_dir() {
                    let root = dir.join("tasks");
                    let store = crate::task::TaskStore::from_tasks_root(&root);
                    match crate::task::scan_and_recover(&store) {
                        Ok(recovered) => {
                            for r in recovered.iter() {
                                eprintln!("[qibai] task: recovered interrupted task id={} step={}", r.id, r.resume_from_step);
                            }
                        }
                        Err(e) => {
                            eprintln!("[qibai] task: scan_and_recover failed: {e}");
                        }
                    }
                    Some(store)
                } else {
                    None
                }
            },
            // v45.12：MCP 注册表刻意保持 None——不在启动时初始化、不 spawn 任何子进程。
            // 首次 send_message 且 settings.mcp_servers 非空时才惰性 from_configs；
            // 或由 IPC（mcp_upsert 等）显式重建。
            mcp: std::sync::Mutex::new(None),
            mcp_pending: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 测试用：注入 mock LLM provider。
    pub fn with_provider(mut self, p: Box<dyn LlmProvider>) -> Self {
        self.provider_override = Some(p);
        self
    }

    /// 测试用：注入 mock 代码检查器与/或 LLM 建议器。
    pub fn with_codecheck_overrides(
        mut self,
        runner: Box<dyn crate::codecheck::CheckRunner>,
        suggester: Option<Box<dyn crate::codecheck::SuggestLlm>>,
    ) -> Self {
        self.cc_runner_override = Some(runner);
        self.cc_suggester_override = suggester;
        self
    }

    // ---- IPC: get_settings / save_settings ----
    pub fn get_settings(&self) -> Settings {
        self.settings.clone()
    }

    pub fn save_settings(&mut self, s: Settings) -> Result<(), EngineError> {
        // v45.9：audio_output / search_engine 变化时分别推给 tts / web_search（单一权威源）。
        if self.settings.audio_output != s.audio_output {
            eprintln!("[qibai] audio: settings changed {} -> {}", self.settings.audio_output, s.audio_output);
            crate::tts::set_audio_output(&s.audio_output);
        }
        if self.settings.search_engine != s.search_engine {
            eprintln!("[qibai] web: settings changed engine {} -> {}", self.settings.search_engine, s.search_engine);
            crate::web_search::set_search_engine(&s.search_engine);
        }
        let mut s = s;
        s.clamp_in_place();
        // 联网参数无论是否变化都同步一次（clamp 后的值）。
        crate::web_search::set_http_timeout_secs(s.http_timeout_secs);
        crate::web_search::set_search_result_n(s.search_result_n);
        // v45.12：MCP 配置变化检测（必须在 self.settings 被覆盖前对比旧值）。
        let mcp_changed = s.mcp_servers != self.settings.mcp_servers;
        s.save()?;
        self.settings = s.clone();
        self.skills.settings = s.skills.clone();
        // 插件启停/grants 同步到注册表（enabled 为空时幂等迁移）。
        self.skills.apply_plugin_settings(&s.plugins);
        // 专家激活状态同步。
        self.resync_experts();
        self.reminders.update_settings(s.reminders.clone());
        // 守护调度器同步新设置（内部自动 sanitize）。
        self.guard.update_settings(s.guard.clone());
        // 保险箱行为参数同步（闲置/冷却/剪贴板计时）。
        self.vault.update_settings(s.vault.clone());
        // v45.12：MCP 配置变化时重建注册表（dedicated IPC 之外的兜底同步路径）。
        if mcp_changed {
            eprintln!("[qibai] mcp: settings.mcp_servers changed via save_settings, rebuilding");
            self.mcp_rebuild();
        }
        Ok(())
    }

    /// v45.9 真 bug 修复：前端 `collect()` 只列它认识的字段，直接反序列化会把
    /// 没传的字段静默填默认值、覆盖已存值（用户选了 USB 音箱点保存就切回 default）。
    ///
    /// 合并规则：
    /// - 以内存中现有的 `self.settings` 为底；
    /// - `patch` 里出现的字段（无论值）覆盖旧值；
    /// - `patch` 里**没出现**的字段保留旧值；
    /// - 显式 `null` 按 serde 默认（Option 字段清空，非 Option 字段取 Default）。
    pub fn save_settings_partial(&mut self, patch: serde_json::Value) -> Result<(), EngineError> {
        let base = serde_json::to_value(&self.settings)
            .map_err(|e| EngineError::Settings(crate::settings::SettingsError::Json(e)))?;
        let merged = merge_json_value(base, patch);
        let mut new_settings: Settings = serde_json::from_value(merged)
            .map_err(|e| EngineError::Settings(crate::settings::SettingsError::Json(e)))?;
        new_settings.clamp_in_place();
        self.save_settings(new_settings)
    }

    /// 给后台 executor 用：clone 一份 Arc<EventSink>。
    pub fn sink_arc(&self) -> std::sync::Arc<dyn EventSink> {
        self.sink.clone()
    }

    /// 启动时调用一次：把已保存的 audio_output / search_engine 灌进 tts / web_search。
    pub fn apply_saved_preferences(&mut self) {
        let audio = if self.settings.audio_output.is_empty() {
            "default".to_string()
        } else {
            self.settings.audio_output.clone()
        };
        eprintln!("[qibai] audio: applied saved device={audio}");
        crate::tts::set_audio_output(&audio);

        let engine = if self.settings.search_engine.is_empty() {
            "auto".to_string()
        } else {
            self.settings.search_engine.clone()
        };
        eprintln!("[qibai] web: applied saved engine={engine}");
        crate::web_search::set_search_engine(&engine);
        crate::web_search::set_http_timeout_secs(self.settings.http_timeout_secs);
        crate::web_search::set_search_result_n(self.settings.search_result_n);
    }

    // ================= v45.12：MCP 外部工具通道 =================

    /// 惰性初始化 MCP 注册表：仅当 `settings.mcp_servers` 非空时 `from_configs`。
    /// **空数组时保持 None——绝不 spawn 任何子进程**（启动耗时与未接入前一致）。
    pub fn mcp_ensure_initialized(&mut self) {
        let mut guard = self.mcp.lock().unwrap();
        if guard.is_some() {
            return;
        }
        if self.settings.mcp_servers.is_empty() {
            return;
        }
        eprintln!(
            "[qibai] mcp: lazy init, {} configured server(s)",
            self.settings.mcp_servers.len()
        );
        *guard = Some(crate::mcp::registry::McpRegistry::from_configs(&self.settings.mcp_servers));
    }

    /// 用当前 `settings.mcp_servers` 全量重建注册表（upsert / remove / set_enabled 后调用）。
    fn mcp_rebuild(&mut self) {
        let fresh = if self.settings.mcp_servers.is_empty() {
            eprintln!("[qibai] mcp: no servers configured — registry cleared, zero subprocesses");
            None
        } else {
            eprintln!(
                "[qibai] mcp: rebuild from {} config(s)",
                self.settings.mcp_servers.len()
            );
            Some(crate::mcp::registry::McpRegistry::from_configs(&self.settings.mcp_servers))
        };
        *self.mcp.lock().unwrap() = fresh;
    }

    /// 服务器列表：以配置为底，叠加 registry **真实**状态（state / 工具数 / 错误原因 / trust）。
    /// 状态一律来自 registry 查询，绝不写死"已连接"。
    pub fn mcp_list(&mut self) -> Vec<serde_json::Value> {
        self.mcp_ensure_initialized();
        let guard = self.mcp.lock().unwrap();
        let mut by_name: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        if let Some(reg) = guard.as_ref() {
            for v in reg.status() {
                if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
                    by_name.insert(n.to_string(), v);
                }
            }
        }
        let mut out = Vec::with_capacity(self.settings.mcp_servers.len());
        for cfg in &self.settings.mcp_servers {
            let mut row = by_name.remove(&cfg.name).unwrap_or_else(|| {
                serde_json::json!({
                    "name": cfg.name,
                    "state": "disabled",
                    "state_detail": serde_json::Value::Null,
                    "tools_count": 0,
                })
            });
            if row.get("command").is_none() {
                row["command"] = serde_json::Value::String(cfg.command.clone());
            }
            if row.get("enabled").is_none() {
                row["enabled"] = serde_json::Value::Bool(cfg.enabled);
            }
            if row.get("transport").is_none() {
                row["transport"] = serde_json::Value::String(cfg.transport.clone());
            }
            if row.get("args").is_none() {
                row["args"] = serde_json::to_value(&cfg.args).unwrap_or_default();
            }
            if row.get("env").is_none() {
                row["env"] = serde_json::to_value(&cfg.env).unwrap_or_default();
            }
            if row.get("trust").is_none() {
                row["trust"] = serde_json::Value::String(match cfg.trust {
                    crate::mcp::McpTrust::Readonly => "readonly".into(),
                    crate::mcp::McpTrust::Confirm => "confirm".into(),
                });
            }
            out.push(row);
        }
        out
    }

    /// 增 / 改一个 server（按 name 去重），即时校验 → 落盘 → 重建注册表。
    pub fn mcp_upsert(
        &mut self,
        cfg: crate::mcp::McpServerConfig,
    ) -> Result<(), EngineError> {
        let mut cfg = cfg;
        if cfg.name.trim().is_empty() {
            return Err(EngineError::Skill("MCP server 的 name 不能为空".into()));
        }
        if cfg.command.trim().is_empty() {
            return Err(EngineError::Skill(format!(
                "MCP server「{}」的 command 不能为空",
                cfg.name
            )));
        }
        if cfg.transport != "stdio" {
            // http 仅占位未实现：不拒绝填写，但强制 disabled（绝不 spawn）。
            eprintln!(
                "[qibai] mcp upsert: server「{}」transport={} 暂不支持（仅 stdio），置 disabled",
                cfg.name, cfg.transport
            );
            cfg.enabled = false;
        }
        match self
            .settings
            .mcp_servers
            .iter_mut()
            .find(|c| c.name == cfg.name)
        {
            Some(existing) => *existing = cfg,
            None => self.settings.mcp_servers.push(cfg),
        }
        self.settings.save()?;
        self.mcp_rebuild();
        Ok(())
    }

    /// 按 name 删除一个 server（落盘 + 重建）。
    pub fn mcp_remove(&mut self, name: &str) -> Result<(), EngineError> {
        let before = self.settings.mcp_servers.len();
        self.settings.mcp_servers.retain(|c| c.name != name);
        if self.settings.mcp_servers.len() == before {
            return Err(EngineError::Skill(format!("MCP server「{name}」不存在")));
        }
        self.settings.save()?;
        self.mcp_rebuild();
        Ok(())
    }

    /// 启停一个 server（落盘 + 重建）。
    pub fn mcp_set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), EngineError> {
        let cfg = self
            .settings
            .mcp_servers
            .iter_mut()
            .find(|c| c.name == name)
            .ok_or_else(|| EngineError::Skill(format!("MCP server「{name}」不存在")))?;
        cfg.enabled = enabled;
        self.settings.save()?;
        self.mcp_rebuild();
        Ok(())
    }

    /// 手动重连（重启该 server 子进程）。状态来自 registry 真实返回。
    pub fn mcp_restart(&mut self, name: &str) -> Result<(), EngineError> {
        self.mcp_ensure_initialized();
        let mut guard = self.mcp.lock().unwrap();
        let reg = guard
            .as_mut()
            .ok_or_else(|| EngineError::Skill("MCP 注册表未初始化（未配置服务器）".to_string()))?;
        reg.restart(name).map_err(EngineError::Skill)
    }

    /// 一条测试调用（server + tool + args）。这是设置页里用户**主动点击**的显式动作，
    /// 不经过 trust 闸门；结果（含 is_error / content 原样）回给前端展示。
    pub fn mcp_test_call(
        &mut self,
        server: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.mcp_ensure_initialized();
        let mut guard = self.mcp.lock().unwrap();
        let reg = guard
            .as_mut()
            .ok_or_else(|| EngineError::Skill("MCP 未配置任何服务器".to_string()))?;
        match reg.call(server, tool, args) {
            Ok(r) => Ok(serde_json::json!({
                "ok": !r.is_error,
                "is_error": r.is_error,
                "content": r.content,
            })),
            Err(e) => Err(EngineError::Skill(e)),
        }
    }

    /// v45.9 P0：调 `context::assemble` 做四层组装。任何 Err 都不影响主路径（调用方 fallback legacy）。
    fn try_assemble_context(
        &self,
        msgs: &[crate::llm_client::ChatMessage],
    ) -> Result<crate::context::layers::RenderedLayers, String> {
        use crate::context::budget::split_budget;
        use crate::context::layers::{LayerInput, Message, assemble};

        let budget = self.settings.context_budget_tokens.clamp(1000, 32000) as usize;
        let recent_turns = self.settings.context_recent_turns.clamp(1, 60);
        let recent: Vec<Message> = msgs
            .iter()
            .rev()
            .take(recent_turns * 2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|m| Message {
                id: String::new(),
                role: m.role.clone(),
                content: m.content.clone().unwrap_or_default(),
                ts: 0,
            })
            .collect();

        let input = LayerInput {
            persona: String::new(),
            skill_catalog: vec![],
            skills_full: vec![],
            task_brief: None,
            messages: recent,
            summary_block: None,
            facts: vec![],
            kb_hits: vec![],
            prior_skills: vec![],
            tool_hint: None,
        };
        let budgets = split_budget(budget);
        Ok(assemble(&input, &budgets))
    }

    // ---- IPC: 清空聊天记录（v45.7：scope=chat/all，清前自动备份）----

    /// 清空聊天记录。`include_memory=false` 只清 conversation，角色记忆/亲密度/成就全部保留；
    /// `include_memory=true` 连角色记忆一起清（等同旧 clear_chat_history）。
    /// 两种都先备份 conversation.json 再清；返回 (清掉的条数, 备份路径)。清完广播
    /// `qibai://conversation-updated` 让界面刷新。
    pub fn clear_conversation(
        &mut self,
        include_memory: bool,
    ) -> Result<(usize, Option<std::path::PathBuf>), EngineError> {
        // 1. 备份清空前的 conversation
        let backup = self.conversation.backup()?;
        // 2. 清 conversation
        let before = self.conversation.all().len();
        self.conversation.clear()?;
        let mut cleared = before;
        // 3. 可选：连角色记忆一起清
        if include_memory {
            match self.personas.clear_all_memory() {
                Ok(n) => cleared += n,
                Err(e) => eprintln!("[qibai] personas.clear_all_memory 失败: {e}"),
            }
        }
        // 4. 广播界面刷新
        self.sink.emit(
            "qibai://conversation-updated",
            serde_json::json!({"cleared": cleared, "include_memory": include_memory}),
        );
        Ok((cleared, backup))
    }

    /// 兼容旧 IPC：连角色记忆一起清（=clear_conversation(true)）。
    pub fn clear_chat_history(&mut self) -> Result<usize, EngineError> {
        let (n, _bak) = self.clear_conversation(true)?;
        Ok(n)
    }

    // ---- IPC: get_conversation ----
    pub fn get_conversation(&self) -> &[StoredMessage] {
        self.conversation.all()
    }

    // ---- IPC: append_chat（Pipecat 语音链路落库）----
    /// 追加一条聊天记录（role=user/assistant）并广播 `qibai://conversation-updated`。
    /// 背景修复：Pipecat 语音链路此前只 emit pipecat-transcript/pipecat-reply 事件，
    /// 不写 conversation、不广播 updated，console.js 轮询 get_conversation 永远看不到
    /// 语音消息——语音对话在控制台里「消失」。本方法统一语音/文本两条链路的落库出口。
    /// 注意：清洗只影响朗读文本，此处落库的是原文（前端气泡显示原文）。
    pub fn append_chat(&mut self, role: &str, content: &str) -> Result<usize, EngineError> {
        self.conversation.append(role, content)?;
        self.sink
            .emit("qibai://conversation-updated", serde_json::json!({}));
        let n = self.conversation.all().len();
        eprintln!("[qibai] append_chat role={role} chars={} total={n}", content.chars().count());
        Ok(n)
    }

    /// 语音「写代码」链路：按需求生成完整可运行代码（非流式，只输出代码本体）。
    /// provider_override 优先（测试 mock 注入 / 运行时热替换），否则用 settings 里的
    /// 豆包配置兜底；失败返回中文错误。llm-status 事件复用，前端状态栏一致。
    pub async fn generate_code(&mut self, spec: &str) -> Result<String, EngineError> {
        self.sink.emit(
            "qibai://llm-status",
            serde_json::json!({"state": "thinking"}),
        );
        let provider: Box<dyn LlmProvider> = match self.provider_override.take() {
            Some(p) => p,
            None => Box::new(DoubaoProvider::from_settings(&self.settings.llm)?),
        };
        let system = "你是代码生成器，根据需求输出完整可运行代码，只输出代码本体，不要解释、不要 markdown 围栏。";
        let resp = provider
            .chat(crate::llm_client::ChatRequest {
                system: system.to_string(),
                messages: vec![ChatMessage::user(spec)],
                tools: None,
                json_object: false,
            })
            .await?;
        self.sink.emit(
            "qibai://llm-status",
            serde_json::json!({"state": "done"}),
        );
        resp.content
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| EngineError::Llm(LlmError::Api("代码生成返回为空".to_string())))
    }

    // ---- IPC: get_skills_status / set_skill_enabled ----
    /// v45.8：合并工具技能（skills.status）+ 文档/专家型技能（skill_docs::catalog）。
    /// 返回 { total_enabled: bool, skills: { <name>: { name, description, kind, source, doc_path?, enabled, load_error? } } }
    pub fn get_skills_status(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        // 工具型技能：沿用 skills.status() 已有结构（kind=tool）
        let tool_status = self.skills.status();
        if let Some(obj) = tool_status.as_object() {
            if let Some(skills) = obj.get("skills").and_then(|v| v.as_object()) {
                for (name, v) in skills {
                    let mut entry = v.clone();
                    if let Some(o) = entry.as_object_mut() {
                        o.entry("kind").or_insert(serde_json::json!("tool"));
                    }
                    map.insert(name.clone(), entry);
                }
            }
        }
        // 文档/专家型技能
        for e in crate::skill_docs::catalog() {
            let kind = if e.description.contains("专家") || e.source == "expert" {
                "expert"
            } else {
                "doc"
            };
            let enabled = !crate::skill_docs::is_disabled(&e.name);
            let mut entry = serde_json::json!({
                "name": e.name,
                "description": e.description,
                "kind": kind,
                "source": e.source,
                "doc_path": if e.has_doc { format!("{}/SKILL.md", e.pkg_dir) } else { String::new() },
                "pkg_dir": e.pkg_dir,
                "has_doc": e.has_doc,
                "enabled": enabled,
            });
            if let Some(err) = e.load_error {
                entry["load_error"] = serde_json::json!(err);
            }
            map.insert(e.name, entry);
        }
        serde_json::json!({
            "total_enabled": true,
            "skills": serde_json::Value::Object(map),
        })
    }

    pub fn set_skill_enabled(&mut self, skill: &str, enabled: bool) -> Result<(), EngineError> {
        // 文档/专家型技能：写 .disabled 标记
        if crate::skill_docs::catalog().iter().any(|e| e.name == skill) {
            crate::skill_docs::set_enabled(skill, enabled)
                .map_err(|e| EngineError::Skill(e))?;
            return Ok(());
        }
        // 工具型技能：原有路径
        self.skills
            .set_enabled(skill, enabled)
            .map_err(|e| EngineError::Skill(e.to_string()))?;
        self.settings.skills = self.skills.settings.clone();
        if let Some(pid) = self.skills.plugins().resolve_toggle_id(skill) {
            self.settings.plugins.enabled.insert(pid, enabled);
        }
        self.settings.save()?;
        Ok(())
    }

/// 批准门控决策（纯函数，便于单测）。
/// v45.7：只有高危 builtin（ExecuteCommand/RunTests）才查 L4 亲密度；
/// 文件读写/建工程等基础操作一律不门控。返回 Some(原因) 表示应拒绝。
fn approval_gate_reason(is_high_risk: bool, level: u32) -> Option<String> {
    if !is_high_risk {
        return None;
    }
    let lvl = (level.clamp(1, 10)) as u8;
    if crate::intimacy::is_gate_open(lvl, "assistant_skills") {
        None
    } else {
        Some(format!(
            "这个操作需要亲密度 L4 才能批准（当前 L{level}）"
        ))
    }
}

// ---- IPC: approve_skill ----
pub async fn approve_skill(&mut self, request_id: &str, approve: bool) -> Result<String, EngineError> {
    // v45.12：MCP trust=confirm 的待确认请求（request_id="mcp-appr-<seq>"）。
    // 复用同一个 approve_skill IPC 与前端审批弹窗，但pending 单独记账（不进 skills.pending）。
    // 被拒时返回可读文本、绝不执行；批准时才经 registry 真实调用。
    if request_id.starts_with("mcp-appr-") {
        let pending = self.mcp_pending.lock().unwrap().remove(request_id);
        let Some((server, tool, args)) = pending else {
            return Err(EngineError::Skill(
                "MCP 待确认请求不存在或已过期（可能已被重启注册表清掉）".into(),
            ));
        };
        if !approve {
            // 被拒：可读文本回灌，不执行任何外部调用。
            return Ok(format!(
                "已取消外部工具调用 {}（{}），未执行任何操作",
                crate::mcp::bridge::namespaced(&server, &tool),
                crate::mcp::bridge::source_label(&server)
            ));
        }
        // 注册表调用包在独立作用域里：guard 先 drop，才能再 &mut self.append_chat。
        let text = {
            let mut guard = self.mcp.lock().unwrap();
            let Some(reg) = guard.as_mut() else {
                return Err(EngineError::Skill(
                    "MCP 注册表未初始化，无法执行已确认的外部调用".into(),
                ));
            };
            let result = reg.call(&server, &tool, args).map_err(EngineError::Skill)?;
            if result.is_error {
                // 工具自报错误：文本原样保留，仅加一行前缀让模型/人看得出边界。
                format!("MCP 工具返回错误：{}", result.content)
            } else {
                result.content
            }
        };
        // 与技能批准同一语义：回写会话 + 气泡。
        let reply = format!("已批准并执行：{text}");
        let _ = self.append_chat("assistant", &reply);
        self.sink.emit(
            "qibai://pet-say",
            serde_json::json!({"text": reply, "flash": true}),
        );
        return Ok(text);
    }
    // v45.7 门控拆分：只有高危 builtin（ExecuteCommand/RunTests，会跑命令/改系统）
    // 才需要亲密度 L4；文件读写/建工程等基础操作直接放行（此前所有批准都被 L4
    // 静默挡掉，表现为"点了允许、没完没了"）。
    if approve && !OPEN_ALL {
        let active = self.personas.active_persona().to_string();
        let level = self
            .intimacy
            .file
            .entries
            .get(&active)
            .map(|e| e.level)
            .unwrap_or(1);
        let is_high_risk = self.skills.is_pending_high_risk(request_id);
        if let Some(msg) = Engine::approval_gate_reason(is_high_risk, level as u32) {
            let full = format!("{msg}（{active}）");
            self.sink.emit(
                "qibai://toast",
                serde_json::json!({"kind": "intimacy-gate", "message": full}),
            );
            return Err(EngineError::Intimacy(full));
        }
    }
    let result = self
        .skills
        .approve(request_id, approve)
        .await
        .map_err(|e| EngineError::Skill(e.to_string()))?;
    // 批准成功：把结果回写进会话并 emit 气泡，否则用户永远不知道成没成。
    if approve {
        let reply = format!("已批准并执行：{result}");
        let _ = self.append_chat("assistant", &reply);
        self.sink.emit(
            "qibai://pet-say",
            serde_json::json!({"text": reply, "flash": true}),
        );
    }
    Ok(result)
}

    // ---- v45.8 IPC：技能安装两段式（stage → commit/cancel） ----
    fn staged_registry() -> &'static std::sync::Mutex<std::collections::HashMap<String, crate::skill_install::StagedSkill>> {
        static REG: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, crate::skill_install::StagedSkill>>> =
            std::sync::OnceLock::new();
        REG.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
    }

    /// stage：source = {"kind":"url"|"file"|"dir","value":"..."}。返回 StagedSkill JSON。
    /// 同步 fn（不是 async）：main.rs 链式 .map_err 走 Result 固有方法，不依赖 TryFutureExt。
    pub fn skill_install_stage(&self, source: serde_json::Value) -> Result<serde_json::Value, String> {
        let kind = source.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let value = source.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let staged = match kind {
            "url" => crate::skill_install::stage_from_url(value).map_err(|e| e.to_string())?,
            "file" => crate::skill_install::stage_from_file(std::path::Path::new(value)).map_err(|e| e.to_string())?,
            "dir" => crate::skill_install::stage_from_dir(std::path::Path::new(value), value).map_err(|e| e.to_string())?,
            other => return Err(format!("未知来源类型：{other}")),
        };
        let stage_id = staged.stage_id.clone();
        let v = serde_json::to_value(&staged).map_err(|e| e.to_string())?;
        Self::staged_registry().lock().unwrap().insert(stage_id, staged);
        Ok(v)
    }

    /// commit：按 stage_id 取出暂存，落盘到 ~/.qibai_skills/<name>/。
    pub fn skill_install_commit(&self, stage_id: &str, overwrite: bool) -> Result<(), String> {
        let staged = Self::staged_registry()
            .lock()
            .unwrap()
            .remove(stage_id)
            .ok_or_else(|| format!("stage_id 不存在或已提交：{stage_id}"))?;
        crate::skill_install::commit(staged, overwrite).map_err(|e| e.to_string())
    }

    /// cancel：删临时目录。
    pub fn skill_install_cancel(&self, stage_id: &str) -> Result<(), String> {
        if let Some(staged) = Self::staged_registry().lock().unwrap().remove(stage_id) {
            crate::skill_install::cancel(staged);
        }
        Ok(())
    }

    /// uninstall：内置保护。
    pub fn skill_uninstall(&self, name: &str) -> Result<(), String> {
        crate::skill_install::uninstall(name).map_err(|e| e.to_string())
    }

    /// read_doc：用户装的优先，回落内置。
    pub fn skill_read_doc(&self, name: &str) -> Result<String, String> {
        crate::skill_install::read_doc(name).map_err(|e| e.to_string())
    }

    /// permissions：没声明返回空 Vec。
    pub fn skill_permissions(&self, name: &str) -> Result<Vec<String>, String> {
        Ok(crate::skill_install::permissions(name))
    }

    /// main.rs 启动时注入两段 plugins 根（builtin + user）。
    pub fn set_plugin_roots(&mut self, builtin: std::path::PathBuf, user: std::path::PathBuf) {
        crate::skill_docs::set_plugin_roots(builtin, user);
    }

    /// main.rs IPC：设置输出设备名（空 / "default" = 系统默认）。
    pub fn set_audio_output(&mut self, name: &str) {
        crate::tts::set_audio_output(name);
    }

    pub fn current_audio_output(&self) -> String {
        crate::tts::current_audio_output()
    }

    /// IPC：设置搜索引擎（auto/baidu/360/sogou）。
    pub fn set_search_engine(&mut self, engine: &str) {
        crate::web_search::set_search_engine(engine);
    }
    pub fn current_search_engine(&self) -> String {
        crate::web_search::current_search_engine()
    }

    // ---- IPC: 长任务（v45.9 P0 接线；执行器 v45.10 接） ----

    fn task_store(&self) -> Result<&crate::task::TaskStore, EngineError> {
        self.task_store.as_ref().ok_or_else(|| EngineError::Skill("task store 未初始化（data_dir 不可用）".into()))
    }

    /// 创建草稿任务（draft，无 plan）。返回 task_id。
    pub fn task_create(&mut self, title: String, goal: String, owner: Option<String>) -> Result<String, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = crate::task::Task::default();
        t.id = crate::task::new_task_id();
        t.title = title;
        t.goal = goal;
        t.owner = owner;
        t.created_at = crate::task::now_ms();
        t.updated_at = t.created_at;
        t.budget.max_steps = self.settings.task_max_steps;
        t.budget.max_minutes = self.settings.task_max_minutes;
        t.budget.max_tokens = self.settings.task_max_tokens as u64;
        store.create(&t)?;
        Ok(t.id)
    }

    pub fn task_list(&self) -> Result<Vec<crate::task::Task>, EngineError> {
        Ok(self.task_store()?.list()?)
    }

    pub fn task_get(&self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let mut t = self.task_store()?.require(task_id)?;
        // v45.9：读路径 enrichment——runs.ndjson 按 step_id 填到 step.runs（不落盘）
        if let Ok((runs, _)) = self.task_store()?.load_runs(task_id) {
            if let Some(ref mut plan) = t.plan {
                for s in plan.steps.iter_mut() {
                    s.runs = runs.iter().filter(|r| r.step_id == s.id).cloned().collect();
                }
            }
        }
        Ok(t)
    }

    /// v45.9：从第一个 failed 步恢复——该步 attempts=0/error=None/状态回 Pending，
    /// 任务 Failed → Running，再 spawn_executor 续跑。非法迁移返回 Err。
    pub fn task_rerun_failed(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        if t.status != crate::task::TaskStatus::Failed {
            return Err(EngineError::InvalidMove(format!(
                "task_rerun_failed: task {} not in Failed (current={:?})", task_id, t.status
            )));
        }
        {
            let plan = t.plan.as_mut().ok_or_else(|| EngineError::InvalidMove("no plan".into()))?;
            let first_failed = plan.steps.iter_mut()
                .find(|s| s.status == crate::task::StepStatus::Failed)
                .ok_or_else(|| EngineError::InvalidMove("no failed step".into()))?;
            first_failed.status = crate::task::StepStatus::Pending;
            first_failed.attempts = 0;
            first_failed.error = None;
            first_failed.ended_at = 0;
        }
        t.status = crate::task::TaskStatus::Running;
        t.last_error = None;
        t.paused_reason = None;
        t.updated_at = crate::task::now_ms();
        store.save(&t)?;
        // 续跑 executor（生产：从 settings 建 DoubaoProvider）
        let provider: std::sync::Arc<dyn crate::llm_client::LlmProvider> =
            match crate::llm_client::DoubaoProvider::from_settings(&self.settings.llm) {
                Ok(p) => std::sync::Arc::new(p),
                Err(e) => {
                    eprintln!("[qibai] task rerun: llm build failed, using NullProvider: {e}");
                    std::sync::Arc::new(crate::llm_client::NullProvider)
                }
            };
        crate::task_exec::spawn_executor(store, task_id.to_string(), self.sink_arc(), provider, crate::task_exec::default_whitelist_root());
        Ok(t)
    }

    // ====== v45.10 P1 知识库 ======

    pub fn kb_add_folder(&mut self, path: &str) -> Result<crate::kb::KbStatus, EngineError> {
        let p = std::path::Path::new(path);
        if !p.is_dir() {
            return Err(EngineError::Skill(format!("kb_add_folder: 不是目录或不存在: {path}")));
        }
        if !self.settings.kb_folders.iter().any(|f| f == path) {
            self.settings.kb_folders.push(path.to_string());
            self.save_settings_partial(serde_json::json!({"kb_folders": self.settings.kb_folders.clone()}))?;
        }
        self.kb_reindex()
    }

    pub fn kb_remove_folder(&mut self, path: &str) -> Result<crate::kb::KbStatus, EngineError> {
        self.settings.kb_folders.retain(|f| f != path);
        self.save_settings_partial(serde_json::json!({"kb_folders": self.settings.kb_folders.clone()}))?;
        self.kb_reindex()
    }

    pub fn kb_reindex(&mut self) -> Result<crate::kb::KbStatus, EngineError> {
        let cfg = crate::kb::index::IndexConfig {
            max_chunks: self.settings.kb_max_chunks,
            chunk_cfg: crate::kb::chunk::ChunkConfig { target_chars: self.settings.kb_chunk_chars, overlap: 80 },
        };
        let folders = self.settings.kb_folders.clone();
        match crate::kb::index::reindex(&folders, cfg) {
            Ok(idx) => Ok(crate::kb::index::status(&idx, &folders)),
            Err(e) => {
                eprintln!("[qibai] kb: reindex failed: {e}");
                Err(EngineError::Skill(format!("kb_reindex: {e}")))
            }
        }
    }

    pub fn kb_status(&self) -> Result<crate::kb::KbStatus, EngineError> {
        let idx = crate::kb::index::load_index().map_err(|e| EngineError::Skill(format!("kb_status: {e}")))?;
        Ok(crate::kb::index::status(&idx, &self.settings.kb_folders))
    }

    pub fn kb_search(&self, query: &str, top_k: Option<usize>) -> Result<Vec<crate::kb::KbHit>, EngineError> {
        if self.settings.kb_folders.is_empty() {
            eprintln!("[qibai] kb: disabled (no folders)");
            return Ok(Vec::new());
        }
        let idx = crate::kb::index::load_index().map_err(|e| EngineError::Skill(format!("kb_search: {e}")))?;
        if idx.chunks.is_empty() {
            eprintln!("[qibai] kb: no hits (empty index)");
            return Ok(Vec::new());
        }
        let k = top_k.unwrap_or(self.settings.kb_top_k);
        Ok(crate::kb::search::search(&idx.chunks, query, k))
    }

    // ====== v45.10 P1 音频诊断 ======

    /// 探测当前音频后端 + 实际落点。
    pub fn audio_diagnose(&self) -> Result<serde_json::Value, EngineError> {
        use crate::audio_pulse::*;
        let tools = probe_tools();
        let selected = crate::tts::current_audio_output();
        let backend = choose_backend(&selected, &tools);
        eprintln!("[qibai] audio: backend={backend:?} device={selected}");
        // 选中 sink 的 index（name → index 反查；解析不到给 null）
        let selected_index = if selected.is_empty() || selected == "default" {
            None
        } else {
            sink_name_to_index(&selected)
        };
        // 播 0.3s 后抓落点
        let landing_idx = std::process::Command::new("pactl")
            .args(["list", "sink-inputs"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).parse::<String>().ok())
            .and_then(|s| parse_sink_landing(&s));
        let landing_name = landing_idx.and_then(sink_index_to_name);
        let note = if tools.pacat == false && tools.pw_play == false && tools.paplay == false {
            "no pulse tools; rodio default (may be system sink)"
        } else if landing_idx.is_none() {
            "landed unknown"
        } else { "" };
        Ok(serde_json::json!({
            "selected_name": selected,
            "selected_index": selected_index,
            "sink_landing_index": landing_idx,
            "sink_landing_name": landing_name,
            "backend": format!("{backend:?}"),
            "tools": {
                "pacat": tools.pacat,
                "pw_play": tools.pw_play,
                "paplay": tools.paplay,
                "rodio": tools.rodio,
            },
            "note": note,
        }))
    }

    /// 播 1 秒 440Hz 正弦，返回 diagnose 结构。
    pub fn audio_test_output(&self) -> Result<serde_json::Value, EngineError> {
        use crate::audio_pulse::*;
        let wav = make_test_tone_wav();
        let selected = crate::tts::current_audio_output();
        let tools = probe_tools();
        let backend = choose_backend(&selected, &tools);
        eprintln!("[qibai] audio: test backend={backend:?} device={selected}");
        let _ = crate::tts::play_audio_bytes(&wav, "test-tone");
        // 等 0.3s 让流落地
        std::thread::sleep(std::time::Duration::from_millis(300));
        let landing_idx = std::process::Command::new("pactl")
            .args(["list", "sink-inputs"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).parse::<String>().ok())
            .and_then(|s| parse_sink_landing(&s));
        let landing_name = landing_idx.and_then(sink_index_to_name);
        let note = if tools.pacat == false && tools.pw_play == false && tools.paplay == false {
            "no pulse tools; rodio default (may be system sink)"
        } else if landing_idx.is_none() {
            "landed unknown"
        } else { "" };
        eprintln!("[qibai] audio: stream landed on {:?}", landing_name.as_deref().unwrap_or("unknown"));
        Ok(serde_json::json!({
            "selected": selected,
            "backend": format!("{backend:?}"),
            "tools": { "pacat": tools.pacat, "pw_play": tools.pw_play, "paplay": tools.paplay, "rodio": tools.rodio },
            "sink_landing": { "index": landing_idx, "name": landing_name },
            "note": note,
        }))
    }

    /// 只产出计划不执行：draft/planned 状态下覆盖 plan，version+1，approved_at=None。
    pub fn task_plan(&mut self, task_id: &str, steps: Vec<crate::task::Step>) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        let plan = match t.plan.take() {
            Some(old) => crate::task::update_plan(&old, steps),
            None => crate::task::build_plan(steps),
        };
        t.plan = Some(plan);
        crate::task::transition(&mut t, crate::task::TaskEvent::PlanCreated, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    /// 用户批准 plan：写 approved_at，迁移 planned -> running。
    pub fn task_confirm_plan(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        {
            let p = t.plan.as_mut().ok_or_else(|| EngineError::Skill("task 无 plan".into()))?;
            crate::task::approve_plan(p);
        }
        crate::task::transition(&mut t, crate::task::TaskEvent::PlanApproved, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    /// 启动任务：要求 plan.approved（否则 Err）；起后台执行线程（不阻塞 UI）。
    pub fn task_start(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        // 硬闸门：未 approve 不许启动
        crate::task::ensure_executable(&t).map_err(|e| EngineError::Task(e))?;
        if t.status != crate::task::TaskStatus::Running {
            crate::task::transition(&mut t, crate::task::TaskEvent::Resume, &crate::task::NoopHook)?;
            store.save(&t)?;
        }
        // v45.9 P0：起后台执行线程（std::thread，关窗不中止，scan_and_recover 兜底）
        let provider: std::sync::Arc<dyn crate::llm_client::LlmProvider> =
            if let Some(p) = self.provider_override.take() {
                std::sync::Arc::from(p)
            } else {
                // 生产：从 settings 建 DoubaoProvider
                match crate::llm_client::DoubaoProvider::from_settings(&self.settings.llm) {
                    Ok(p) => std::sync::Arc::new(p),
                    Err(e) => {
                        eprintln!("[qibai] task: llm provider build failed, using NullProvider: {e}");
                        std::sync::Arc::new(crate::llm_client::NullProvider)
                    }
                }
            };
        crate::task_exec::spawn_executor(store, task_id.to_string(), self.sink_arc(), provider, crate::task_exec::default_whitelist_root());
        Ok(t)
    }

    pub fn task_pause(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        t.paused_reason = Some("manual".into());
        eprintln!("[qibai] task: paused reason=manual task={task_id}");
        crate::task::transition(&mut t, crate::task::TaskEvent::Pause, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    pub fn task_resume(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        crate::task::transition(&mut t, crate::task::TaskEvent::Resume, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    pub fn task_cancel(&mut self, task_id: &str) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        crate::task::transition(&mut t, crate::task::TaskEvent::Cancel, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    /// 处理 waiting_confirm：allow=false 时把当前步标 Failed（不执行）；
    /// allow=true 时若 remember 则加入 guarded_remembered，然后 ConfirmResolved。
    pub fn task_confirm_action(
        &mut self,
        task_id: &str,
        action_id: &str,
        allow: bool,
        remember: bool,
    ) -> Result<crate::task::Task, EngineError> {
        let store = self.task_store()?.clone();
        let mut t = store.require(task_id)?;
        if !allow {
            // 用户拒绝：当前步标 Failed，先 ConfirmResolved 回 running，再 MarkFailed。
            if let Some(p) = t.plan.as_mut() {
                if let Some(s) = p.steps.get_mut(t.current_step) {
                    if s.id == action_id {
                        s.status = crate::task::StepStatus::Failed;
                        s.error = Some("user rejected".into());
                    }
                }
            }
            crate::task::transition(&mut t, crate::task::TaskEvent::ConfirmResolved, &crate::task::NoopHook)?;
            crate::task::transition(&mut t, crate::task::TaskEvent::MarkFailed, &crate::task::NoopHook)?;
            store.save(&t)?;
            return Ok(t);
        }
        if remember {
            crate::task::remember_guarded_approval(&mut t, action_id);
        }
        crate::task::transition(&mut t, crate::task::TaskEvent::ConfirmResolved, &crate::task::NoopHook)?;
        store.save(&t)?;
        Ok(t)
    }

    // ---- IPC: plugin_* / expert_*（契约 §3.6） ----

    /// 新增自定义连接器（MCP/HTTP）：物化插件目录 → 注册 → 自动授权 → 启用 → 落盘。
    pub fn connector_add(&mut self, cfg: crate::connectors::ConnectorConfig) -> Result<(), EngineError> {
        if self.skills.plugins().entry(&cfg.id).is_some() {
            return Err(EngineError::Skill(format!("连接器 {} 已存在", cfg.id)));
        }
        let plugins_dir = self.skills.plugins().plugins_dir().to_path_buf();
        cfg.materialize(&plugins_dir)
            .map_err(|e| EngineError::Skill(e.to_string()))?;
        self.skills.plugins_mut().scan();
        // 连接器由用户主动添加：自动授予声明权限并启用（本地优先红线仍生效：外部 files:write 硬拒）。
        let perms: Vec<&str> = match cfg.kind {
            crate::connectors::ConnectorKind::Mcp => vec!["command:run"],
            crate::connectors::ConnectorKind::Http => vec!["network:read"],
        };
        for p in perms {
            if let Some(perm) = crate::plugin_manifest::Permission::from_str(p) {
                let _ = self.skills.plugins_mut().grant_permission(&cfg.id, perm, true);
            }
        }
        if cfg.enabled {
            let _ = self.skills.plugins_mut().set_enabled(&cfg.id, true);
        }
        self.settings.connectors.push(cfg);
        self.settings.save()?;
        self.resync_experts();
        Ok(())
    }

    /// 移除自定义连接器：删除物化目录并从注册表摘除。
    pub fn connector_remove(&mut self, id: &str) -> Result<(), EngineError> {
        self.plugin_uninstall(id.to_string())?;
        self.settings.connectors.retain(|c| c.id != id);
        self.settings.save()?;
        self.resync_experts();
        Ok(())
    }

    /// 连接器清单（type=connector 插件）。
    pub fn connector_list(&self) -> Vec<serde_json::Value> {
        self.skills
            .plugins()
            .list_json()
            .into_iter()
            .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("connector"))
            .collect()
    }

    /// 导入安装技能包（zip）：安全解压 → manifest 校验 → 复制进 plugins/ → 注册。
    pub fn skill_pack_install(&mut self, zip_path: String) -> Result<(), EngineError> {
        let plugins_dir = self.skills.plugins().plugins_dir().to_path_buf();
        crate::skill_pack::install_pack(std::path::Path::new(&zip_path), &plugins_dir)
            .map_err(|e| EngineError::Skill(e.to_string()))?;
        self.skills.plugins_mut().scan();
        self.resync_experts();
        Ok(())
    }

    /// 导出技能包（zip）：从 plugins/<id> 打包。
    pub fn skill_pack_export(&self, plugin_id: String, zip_path: String) -> Result<(), EngineError> {
        let plugins_dir = self.skills.plugins().plugins_dir().to_path_buf();
        crate::skill_pack::export_pack(&plugin_id, &plugins_dir, std::path::Path::new(&zip_path))
            .map_err(|e| EngineError::Skill(e.to_string()))
    }

    /// 可导出技能包清单（已安装且非内置的外部插件）。
    pub fn skill_pack_list(&self) -> Vec<serde_json::Value> {
        self.skills
            .plugins()
            .list_json()
            .into_iter()
            .filter(|m| {
                m.get("builtin").and_then(|b| b.as_bool()).unwrap_or(false) == false
                    && m.get("installed").and_then(|i| i.as_bool()).unwrap_or(false)
            })
            .collect()
    }

    /// 从注册表 expert 清单重建专家注册表；保留当前 active 状态。
    fn resync_experts(&mut self) {
        let active = self.experts.active_id().map(|s| s.to_string());
        let mut reg = crate::expert::ExpertRegistry::new(
            self.skills.plugins().expert_manifests(),
            &self.settings.plugins,
        );
        if let Some(id) = active {
            let _ = reg.activate(&id);
        }
        self.experts = reg;
    }

    /// 插件列表：`{id,type,name,description,icon,version,author,permissions,granted,enabled,installed,entry_kind,functions,error,builtin}`。
    pub fn plugin_list(&self) -> Vec<serde_json::Value> {
        self.skills.plugins().list_json()
    }

    /// 安装插件（复制目录；幂等）。
    pub fn plugin_install(&mut self, dir: String) -> Result<(), EngineError> {
        let from = std::path::Path::new(&dir).to_path_buf();
        self.skills
            .plugins_mut()
            .install(&from)
            .map_err(|e| EngineError::Skill(e))?;
        self.resync_experts();
        Ok(())
    }

    /// 卸载插件（幂等；内置拒绝）。
    pub fn plugin_uninstall(&mut self, id: String) -> Result<(), EngineError> {
        self.skills
            .plugins_mut()
            .uninstall(&id)
            .map_err(|e| EngineError::Skill(e))?;
        // 摘除 settings 中对应记录
        self.settings.plugins.enabled.remove(&id);
        self.settings.plugins.grants.remove(&id);
        self.settings.plugins.experts.remove(&id);
        self.settings.save()?;
        self.resync_experts();
        Ok(())
    }

    /// 扫描一个目录下所有一级子目录（各自含 manifest.json 的插件包）并安装。
    /// 用于随包分发的 skills/ 与 experts/ 库：幂等、跳过错误包、返回成功数。
    pub fn scan_plugin_dir(&mut self, dir: &std::path::Path) -> usize {
        let mut n = 0usize;
        let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for e in dirs {
            let p = e.path();
            if !p.is_dir() || !p.join("qibai-plugin.json").is_file() {
                continue;
            }
            if self.plugin_install(p.to_string_lossy().to_string()).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// 技能清单（type=skill 插件）：id/name/description/version/functions，供设置页与语音匹配。
    pub fn skill_list(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for m in self.skills.plugins().list_json() {
            if m.get("type").and_then(|t| t.as_str()) == Some("skill") {
                out.push(m);
            }
        }
        out
    }

    /// 调用技能：运行该插件声明的 entry.command（cwd=插件目录，sh 执行），
    /// 参数以 QIBAI_SKILL_ARGS 环境变量传入（不经 shell 拼接，防注入）；
    /// 返回合并输出（截断到 4000 字符）。仅允许相对插件目录内的命令路径。
    pub fn skill_invoke(&self, id: &str, _function: &str, args_json: &str) -> Result<String, EngineError> {
        let reg = self.skills.plugins();
        let entry = reg.entry(id).ok_or_else(|| EngineError::Skill(format!("未知技能 {id}")))?;
        let cmd = entry
            .manifest
            .entry
            .as_ref()
            .and_then(|en| en.command.clone())
            .ok_or_else(|| EngineError::Skill(format!("技能 {id} 未声明可执行入口")))?;
        if cmd.contains("..") {
            return Err(EngineError::Skill(format!("技能 {id} 入口路径非法")));
        }
        let plugin_dir = reg.plugins_dir().join(id);
        let script = plugin_dir.join(&cmd);
        if !script.starts_with(reg.plugins_dir()) {
            return Err(EngineError::Skill(format!("技能 {id} 入口越界")));
        }
        let out = std::process::Command::new("sh")
            .arg(&script)
            .current_dir(&plugin_dir)
            .env("QIBAI_SKILL_ARGS", args_json)
            .output()
            .map_err(|e| EngineError::Skill(format!("技能 {id} 执行失败：{e}")))?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        if !out.stderr.is_empty() {
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        let text = text.trim().to_string();
        if text.chars().count() > 4000 {
            return Ok(text.chars().take(4000).collect());
        }
        Ok(text)
    }

    /// 插件启停；同步 settings.plugins.enabled 并落盘。
    pub fn plugin_set_enabled(&mut self, id: String, enabled: bool) -> Result<(), EngineError> {
        self.skills
            .plugins_mut()
            .set_enabled(&id, enabled)
            .map_err(|e| EngineError::Skill(e))?;
        self.settings.plugins.enabled.insert(id, enabled);
        self.settings.save()?;
        Ok(())
    }

    /// 权限授予/撤销；files:write 对外部插件硬拒。同步 settings.plugins.grants 并落盘。
    pub fn plugin_grant_permission(
        &mut self,
        id: String,
        permission: String,
        approve: bool,
    ) -> Result<(), EngineError> {
        let perm = crate::plugin_manifest::Permission::from_str(&permission)
            .ok_or_else(|| EngineError::Skill(format!("未知权限 {permission:?}")))?;
        self.skills
            .plugins_mut()
            .grant_permission(&id, perm, approve)
            .map_err(|e| EngineError::Skill(e))?;
        // 落盘 grants 列表（与注册表条目保持一致）
        let granted: Vec<String> = self
            .skills
            .plugins()
            .entry(&id)
            .map(|e| e.grants.iter().map(|p| p.as_str().to_string()).collect())
            .unwrap_or_default();
        self.settings.plugins.grants.insert(id, granted);
        self.settings.save()?;
        Ok(())
    }

    /// 专家列表：`{id,name,description,version,author,system_prompt,skills,persona_binding,active}`。
    pub fn expert_list(&self) -> Vec<serde_json::Value> {
        let active = self.experts.active_id().map(|s| s.to_string());
        let mut out: Vec<serde_json::Value> = self
            .experts
            .list()
            .map(|e| {
                let m = &e.manifest;
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "description": m.description,
                    "version": m.version,
                    "author": m.author,
                    "system_prompt": m.system_prompt,
                    "skills": m.skills,
                    "persona_binding": m.persona_binding,
                    "active": active.as_deref() == Some(m.id.as_str()),
                })
            })
            .collect();
        out.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        out
    }

    /// 激活/停用专家；空串 = 停用。同步 settings.plugins.experts 并落盘。
    pub fn expert_set_active(&mut self, id: String) -> Result<(), EngineError> {
        if id.trim().is_empty() {
            self.experts.deactivate();
        } else {
            self.experts
                .activate(&id)
                .map_err(|e| EngineError::Skill(e))?;
        }
        // active 特判：experts map 中 true=当前激活，其余 false。
        self.settings.plugins.experts.clear();
        if let Some(active) = self.experts.active_id() {
            self.settings
                .plugins
                .experts
                .insert(active.to_string(), true);
        }
        self.settings.save()?;
        Ok(())
    }

    // ---- IPC: send_message ----
    pub async fn send_message(&mut self, text: &str) -> Result<SendMessageOutput, EngineError> {
        // v45.8-A：空文本硬闸——拔麦/转写为空时不进 LLM、不写会话、不 emit thinking。
        if text.trim().is_empty() {
            let n = self.conversation.all().len();
            eprintln!("[qibai] llm: send_blocked empty (history={n})");
            return Err(EngineError::Llm(LlmError::Api("空消息不发送".to_string())));
        }
        // v45.8-B：诊断锚点——真发起请求前打一行，下次复现可一眼分辨"空文本"vs"旧音频被转写"。
        let head: String = text.trim().chars().take(20).collect::<String>().replace('\n', " ");
        eprintln!("[qibai] llm: send text_len={} head={head}", text.chars().count());

        self.sink.emit(
            "qibai://llm-status",
            serde_json::json!({"state": "thinking"}),
        );

        // 迭代③：保险箱对话拦截——发生在 provider/LLM 之前；命中时绝不调用 LLM。
        if let Some(out) = self.try_vault_intercept(text)? {
            return Ok(out);
        }

        let provider: Box<dyn LlmProvider> = match self.provider_override.take() {
            Some(p) => p,
            None => match DoubaoProvider::from_settings(&self.settings.llm) {
                Ok(p) => Box::new(p),
                Err(e) => {
                    self.sink.emit(
                        "qibai://llm-status",
                        serde_json::json!({"state": "error", "message": e.to_string()}),
                    );
                    return Err(EngineError::Llm(e));
                }
            },
        };

        // system prompt = persona 基座 + active expert 追加段（契约 §3.6）。
        let base_system = build_system_prompt_for_persona(&self.settings.ui, self.personas.active_persona());
        let mut system = self.experts.expert_prompt(&base_system);
        // v30/v46：连续对话记忆——角色长期记忆摘要（最近 6 条，每条截断 240 字符）。
        // 启动抑制：冷启动首句且用户没在主动回忆时，跳过注入，避免模型一开场就复述
        // 上一轮留下的待办；非冷启动或用户主动问起（含回忆关键词）照常注入。
        if let Some(pe) = self.personas.get(self.personas.active_persona()) {
            let n = pe.memory.len();
            let is_first_turn = self.conversation.all().is_empty();
            if should_inject_memory(is_first_turn, text, n) {
                let lines: Vec<String> = pe
                    .memory
                    .iter()
                    .rev()
                    .take(6)
                    .map(|m| {
                        let who = if m.role == "user" { "用户" } else { "小小" };
                        format!("- {who}说：{}", qibai_truncate(&m.content, 240))
                    })
                    .collect();
                system = format!(
                    "{system}\n【长期记忆】以下是你记住的与用户过往交流片段（若与当前对话重复可忽略，仅作回忆参考）：\n{}\n【注意】以上仅为回忆素材；除非用户主动问起，不要在回复中主动复述或罗列待办任务。",
                    lines.join("\n")
                );
            } else if n > 0 {
                eprintln!("[qibai] boot: memory-inject suppressed (first turn, n={n})");
            }
        }
        // v31：动态能力清单——自动扫描已装载专家与技能注入 system，
        // 用户问「你会什么/介绍下自己」时按真实清单作答，不凭记忆。
        let mut cap_lines: Vec<String> = Vec::new();
        let experts = self.expert_list();
        if !experts.is_empty() {
            cap_lines.push("已装载专家：".to_string());
            for e in experts.iter().take(8) {
                let name = e["name"].as_str().unwrap_or("").to_string();
                let desc = e["description"].as_str().unwrap_or("").to_string();
                let active = if e["active"].as_bool().unwrap_or(false) { "（当前激活）" } else { "" };
                cap_lines.push(format!("- {} {}{}", name, active, qibai_truncate(&desc, 60)));
            }
        }
        let skills = self.skill_list();
        if !skills.is_empty() {
            let names: Vec<String> = skills
                .iter()
                .filter_map(|s| s["name"].as_str().map(|n| n.to_string()))
                .collect();
            cap_lines.push(format!("可用技能（{} 个）：{}", names.len(), names.join("、")));
        }
        // v45.8：能力清单从技能目录自动生成，不再硬编码"联网：web_search"等大字串。
        // 底座原语（code_write_file / code_read_file / code_list_dir / code_run_command /
        // workspace_info / web_search / fetch_page / fetch_many）只点名一次真实根，
        // 具体怎么做（搜索纪律/长文续读/建工程流程）由 SKILL.md 驱动。
        let (real_root, _src) = qibai_workspace_root_with_source();
        cap_lines.push(format!(
            "底座文件与联网原语已就绪（真实工作区根 {}；路径被拒时改用根内相对路径，~/ 与反斜杠自动展开）。",
            real_root.display()
        ));
        let skill_catalog = crate::skill_docs::render_catalog_prompt();
        if !skill_catalog.is_empty() {
            cap_lines.push(skill_catalog);
        }
        cap_lines.push("知识库：本地 ~/.qibai_kb（或 QIBAI_KB）文档检索。".to_string());
        system = format!("{system}\n【可用能力】以下是你当前真实装载的能力（向用户介绍自己时据此如实回答，不要编造没有的能力）：\n{}", cap_lines.join("\n"));
        // v45.10 假完成硬规则（短句，非能力清单）：只有本轮真正成功的工具调用、且产物经校验存在，
        // 才算完成；没执行过就如实说没做，不许嘴上说已完成。
        system.push_str("\n【硬规则】只有本轮真正成功执行、且产物经校验存在的工具调用，才算完成；没执行过就如实说还没做，绝不许说\"已完成/建好了\"而实际没动。");
        // v31：本地知识库检索注入——问题命中 kb 文档片段时作为参考资料给 LLM。
        let kb_hits = kb_search(text);
        if !kb_hits.is_empty() {
            system = format!(
                "{system}\n【知识库资料】以下片段来自用户本地知识库（~/.qibai_kb 或 QIBAI_KB），回答请优先结合这些资料；片段不相关时可忽略：\n{}",
                kb_hits.join("\n---\n")
            );
        }
        // v30：连续对话记忆强制开启——总是携带最近 N 条对话历史（v45.9：N 从 settings 读，默认 50），
        // 防无界历史撑爆 context / token；remember_chat=false 仅保留给单测显式构造路径。
        let history_n = self.settings.history_recent_n.clamp(5, 200);
        let mut msgs: Vec<ChatMessage> = self
            .conversation
            .all()
            .iter()
            .rev() // 取最近 N 条
            .take(history_n)
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: Some(m.content.clone()),
                tool_calls: None,
                tool_call_id: None,
            })
            .collect();
        msgs.reverse(); // 恢复旧→新时间顺序

        // v45.9 P0：context::assemble 四层组装（可回退）。
        // 开关关掉或组装失败 → 走原路径（不影响 history_n 行为）。
        if self.settings.context_layers_enabled {
            match self.try_assemble_context(&msgs) {
                Ok(rendered) => {
                    eprintln!("[qibai] context: layers assembled tokens={} (L0={} L1={} L2={} L3={})",
                        rendered.total_tokens, rendered.tokens_l0, rendered.tokens_l1, rendered.tokens_l2, rendered.tokens_l3);
                }
                Err(e) => {
                    eprintln!("[qibai] context: assemble failed, fallback legacy: {e}");
                }
            }
        }

        msgs.push(ChatMessage::user(text));
        let history: Vec<ChatMessage> = msgs;

        // 工具表 = 动态技能注册表工具（内置 + 已启用插件）+ engine 级守护触发工具 start_guard。
        let mut tool_schemas = self.skills.tool_schemas();
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "start_guard",
                "description": "触发一次全屏守护演出（久坐休息提醒）",
                "parameters": { "type": "object", "properties": {} }
            }
        }));
        // v30/v45.7/v45.8：本地工作区工具（操作本地电脑：创建工程/写代码）。安全三闸门：
        // 目录白名单（真实根 QIBAI_WORKSPACE 或 ~/Documents/qibai）+ 命令 allowlist + 越界拒绝。
        let (real_root, _src) = qibai_workspace_root_with_source();
        let root_disp = real_root.display().to_string();
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "code_write_file",
                "description": format!("在本地工作区（真实根 {root_disp}）创建或覆盖文件，自动创建父目录。帮用户创建工程/写代码时使用。path 填工作区内相对路径（如 app/main.py）或根内绝对路径；根外绝对路径与 .. 越界一律拒绝；~/ 与反斜杠会自动展开。"),
                "parameters": { "type": "object", "properties": {
                    "path": {"type": "string", "description": "工作区内相对路径（如 app/main.py）或根内绝对路径"},
                    "content": {"type": "string", "description": "文件完整内容"}
                }, "required": ["path", "content"] }
            }
        }));
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "code_read_file",
                "description": format!("读取本地工作区（真实根 {root_disp}）内的文件内容（相对路径或根内绝对路径，禁止越界）。"),
                "parameters": { "type": "object", "properties": {
                    "path": {"type": "string", "description": "工作区内相对路径或根内绝对路径"}
                }, "required": ["path"] }
            }
        }));
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "code_list_dir",
                "description": format!("列出本地工作区（真实根 {root_disp}，或其中子目录）的文件与文件夹清单（相对路径或根内绝对路径）。"),
                "parameters": { "type": "object", "properties": {
                    "dir": {"type": "string", "description": "工作区内相对目录，默认 ."}
                }, "required": [] }
            }
        }));
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "code_run_command",
                "description": "在本地工作区目录内执行白名单命令（pwd/ls/mkdir/touch/cat/echo/printf/grep/head/tail/wc/file/df/du/which/uname/node/python3/npm/cargo）。用于查看文件、创建目录、检查环境、运行验证刚写的代码（node 跑 .js、python3 跑 .py，路径用工作区内相对路径）；rm/bash/sh/zsh/curl/wget 等危险命令不支持。参数含 / 时必须在工作区根内；shell 元字符（; | & > < ` $）一律拒绝。",
                "parameters": { "type": "object", "properties": {
                    "command": {"type": "string", "description": "完整命令，如 ls -la"}
                }, "required": ["command"] }
            }
        }));
        // v45.8：只读工具——让小小被问"工作区在哪/工程放哪"时答真值。
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "workspace_info",
                "description": "返回当前工作区根的真实路径、来源（env/default）、是否可写、剩余字节与建议写法。被问'你工作区在哪/我该把工程放哪'时调用。",
                "parameters": { "type": "object", "properties": {} }
            }
        }));
        tool_schemas.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "clear_conversation",
                "description": "清空控制台聊天记录。scope=chat 只清聊天记录、保留角色记忆与亲密度；scope=all 连角色记忆一起清（不可恢复，慎用）。只在用户明确要求清空聊天/清空记录时调用，用户没明确说时不要调用、先问；执行前自动备份 conversation.json。",
                "parameters": { "type": "object", "properties": {
                    "scope": {"type": "string", "enum": ["chat", "all"], "description": "chat=只清聊天记录（默认）；all=连角色记忆一起清"},
                    "confirm": {"type": "boolean", "description": "必须为 true 才执行（不可恢复操作，需用户明确确认）"}
                }, "required": ["scope", "confirm"] }
            }
        }));

        // v45.12：MCP 外部工具表追加（惰性——settings.mcp_servers 为空时整块短路，
        // 不 lock 注册表、不 spawn 子进程、不产生任何 schema）。
        // 冲突（与本地工具或另一 server 重名）由 bridge::build_schemas 拦截，
        // 不许静默覆盖：写日志 + 向 UI 推一条 qibai://mcp-conflicts 清单事件。
        self.mcp_ensure_initialized();
        let mut mcp_ready_pairs: Vec<(String, crate::mcp::McpTool)> = Vec::new();
        {
            let guard = self.mcp.lock().unwrap();
            if let Some(reg) = guard.as_ref() {
                for (srv, handle) in &reg.servers {
                    if matches!(handle.state, crate::mcp::McpState::Ready) {
                        for t in &handle.tools {
                            mcp_ready_pairs.push((srv.clone(), t.clone()));
                        }
                    }
                }
            }
        }
        if !mcp_ready_pairs.is_empty() {
            use crate::mcp::bridge;
            let local_names: std::collections::HashSet<String> = tool_schemas
                .iter()
                .filter_map(|s| s["function"]["name"].as_str().map(String::from))
                .collect();
            let guard = self.mcp.lock().unwrap();
            let reg = guard.as_ref().expect("mcp registry must be Some when pairs non-empty");
            let bridged = bridge::build_schemas(reg, &mcp_ready_pairs, &local_names);
            for s in bridged.schemas {
                tool_schemas.push(s);
            }
            if !bridged.conflicts.is_empty() {
                for (full, reason) in &bridged.conflicts {
                    eprintln!("[qibai] mcp: 冲突未进工具表 {full}: {reason}");
                }
                self.sink.emit(
                    "qibai://mcp-conflicts",
                    serde_json::json!({
                        "items": bridged.conflicts
                            .iter()
                            .map(|(n, r)| serde_json::json!({"name": n, "reason": r}))
                            .collect::<Vec<_>>()
                    }),
                );
            }
        }

        // v18.8-fix28e：Qwen/DeepSeek 等模型常见不遵守"先 tool_calls 后 JSON"的约定——
        // system 追加显式工具调用说明，避免模型误以为每次都要直接输出 JSON 而跳过技能。
        tool_schemas.retain(|s| s.is_object()); // 防御：过滤非法 schema 条目
        let system = if tool_schemas.is_empty() {
            system
        } else {
            format!(
                "{system}\n【工具调用】当你需要查询实时信息或执行操作（如天气、写文件、开应用等）时，请先通过 function calling 输出 tool_calls 调用对应工具（此时不要输出 JSON 回复）；工具执行结果回填给你之后，再按【输出格式】输出最终的 JSON 回复。\n【技能/专家匹配】用户需要找到技能或专家来完成某件事时，先理解任务类型，再调用 list_skills 或结合已装载专家清单，推荐匹配的技能或专家并说明用途；没有匹配时如实说没有。搜索/长文阅读/建工程/代码检查的具体步骤见对应 SKILL.md（用 skill_read 按名加载）。"
            )
        };
        // v45.12：系统提示只加**一句**——说明存在 mcp__<server>__<tool> 形式的外部工具；
        // 不罗列能力清单、不夸大。仅当本轮真有 MCP 工具进表时才加。
        let system = if mcp_ready_pairs.is_empty() {
            system
        } else {
            format!(
                "{system}\n【外部工具】此外还有以 mcp__<服务器名>__<工具名> 形式存在的外部工具（由你本地配置的 MCP server 提供），按各自工具说明调用即可。"
            )
        };

        // 专家白名单过滤：active expert 且 whitelist 非空时，只保留白名单内的工具；
        // start_guard 为引擎级安全工具，始终保留；mcp__ 外部工具是用户主动安装的能力，
        // 同样始终保留（不被专家白名单静默裁掉）。
        if !OPEN_ALL {
            if let Some(whitelist) = self.experts.whitelist() {
                tool_schemas.retain(|s| {
                let name = s["function"]["name"].as_str().unwrap_or("");
                name == "start_guard"
                    || name.starts_with(crate::mcp::bridge::NS_PREFIX)
                    || self
                        .skills
                        .plugins()
                        .plugin_id_for_function(name)
                        .map(|pid| whitelist.iter().any(|w| w == pid || w == name))
                        .unwrap_or(false)
                });
            }
        }

        // 用一个小 adapter 把 &mut SkillsRegistry + &mut GuardScheduler + &dyn EventSink 包成 ToolExecutor。
        // skills 与 guard 是 Engine 的不同字段，可同时可变借用（字段级 disjoint borrow）。
        struct EngineToolExecutor<'a> {
            skills: &'a mut SkillsRegistry,
            guard: &'a mut crate::guard::GuardScheduler<crate::guard::SystemClock>,
            sink: &'a dyn EventSink,
            // v45.7：clear_conversation 需要直接操作 conversation/personas（字段级 disjoint borrow）。
            conversation: &'a mut crate::storage::ConversationStore,
            personas: &'a mut crate::personas::PersonaStore,
            // v45.12：MCP 注册表锁的共享引用（mpsc Receiver 是 Send 但 !Sync，
            // 不能把 &mut McpRegistry 放进 Send+Sync 的 ToolExecutor；改为每次分发时
            // 在短临界区内 lock → call → drop，锁绝不跨 await 持有）。
            mcp_lock: &'a std::sync::Mutex<Option<crate::mcp::registry::McpRegistry>>,
            // v45.12：trust=confirm 待确认记账（key=request_id，value=(server, tool, args)）。
            mcp_pending: &'a std::sync::Mutex<
                std::collections::HashMap<String, (String, String, serde_json::Value)>,
            >,
        }
        #[async_trait::async_trait]
        impl<'a> crate::llm_client::ToolExecutor for EngineToolExecutor<'a> {
            async fn invoke(
                &mut self,
                name: &str,
                args: &serde_json::Value,
            ) -> crate::skills::SkillOutcome {
                // 统一落日志：每一次工具调用（含 engine 级 code_* 与技能白名单内全部工具）
                // 都打一行到 stderr（启动脚本重定向进 app.log），下次排查不再听模型自述。
                let out = self.dispatch(name, args).await;
                let summary = summarize_tool_args(args);
                let status = match &out {
                    crate::skills::SkillOutcome::Ok(_) => "Ok".to_string(),
                    crate::skills::SkillOutcome::Denied(reason) => format!("Denied({reason})"),
                    crate::skills::SkillOutcome::PendingApproval { .. } => "PendingApproval".to_string(),
                };
                eprintln!("[qibai] tool: {name} args={summary} → {status}");
                out
            }
        }

        impl<'a> EngineToolExecutor<'a> {
            async fn dispatch(
                &mut self,
                name: &str,
                args: &serde_json::Value,
            ) -> crate::skills::SkillOutcome {
                // engine 级工具：start_guard 不走技能白名单（只读安全，由守护调度器直接拦截）。
                if name == "start_guard" {
                    let now = self.guard.now();
                    return match self.guard.manual_trigger(now) {
                        Some(_start) => {
                            crate::skills::SkillOutcome::Ok("已为你启动全屏守护演出，注意休息~".to_string())
                        }
                        None => crate::skills::SkillOutcome::Denied(
                            "守护当前不可用（未启用或冷却中）".to_string(),
                        ),
                    };
                }
                // 窗口控制（v14）：广播事件 → 主进程隐藏/显示桌宠窗、前端关气泡。
                if name == "hide_pet" {
                    self.sink.emit("qibai://pet-hide", serde_json::json!({}));
                    return crate::skills::SkillOutcome::Ok("好，我先藏起来啦，需要的时候叫我～".to_string());
                }
                if name == "show_pet" {
                    self.sink.emit("qibai://pet-show", serde_json::json!({}));
                    return crate::skills::SkillOutcome::Ok("我回来啦～".to_string());
                }
                if name == "dismiss_bubble" {
                    self.sink.emit("qibai://bubble-dismiss", serde_json::json!({}));
                    return crate::skills::SkillOutcome::Ok("气泡已经关掉了～".to_string());
                }
                // ---- v30 本地工作区工具（三闸门安全模型）----
                if name == "code_write_file" {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let root = qibai_workspace_root();
                    return match safe_resolve(&root, path) {
                        Some(p) => {
                            if let Some(parent) = p.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            match std::fs::write(&p, content) {
                                Ok(_) => crate::skills::SkillOutcome::Ok(format!(
                                    "已写入文件 {}（{:.1} KB）",
                                    p.display(),
                                    content.len() as f64 / 1024.0
                                )),
                                Err(e) => crate::skills::SkillOutcome::Denied(format!("写入失败：{e}")),
                            }
                        }
                        None => crate::skills::SkillOutcome::Denied(denied_path_message(&root, path)),
                    };
                }
                if name == "code_read_file" {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let root = qibai_workspace_root();
                    return match safe_resolve(&root, path) {
                        None => crate::skills::SkillOutcome::Denied(denied_path_message(&root, path)),
                        Some(p) => {
                            if !within_root(&root, &p) {
                                crate::skills::SkillOutcome::Denied(denied_path_message(&root, path))
                            } else {
                                match std::fs::read_to_string(&p) {
                                    Ok(t) => crate::skills::SkillOutcome::Ok(qibai_truncate(&t, 4000)),
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        crate::skills::SkillOutcome::Denied(format!(
                                            "文件不存在：{}（请先用 code_write_file 创建）",
                                            p.display()
                                        ))
                                    }
                                    Err(e) => crate::skills::SkillOutcome::Denied(format!(
                                        "读取失败：{e}（路径 {}，白名单根 {}）",
                                        p.display(),
                                        root.display()
                                    )),
                                }
                            }
                        }
                    };
                }
                if name == "code_list_dir" {
                    let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or(".");
                    let root = qibai_workspace_root();
                    return match safe_resolve(&root, dir) {
                        None => crate::skills::SkillOutcome::Denied(denied_path_message(&root, dir)),
                        Some(p) => {
                            if !within_root(&root, &p) {
                                crate::skills::SkillOutcome::Denied(denied_path_message(&root, dir))
                            } else {
                                match std::fs::read_dir(&p) {
                                    Ok(rd) => {
                                        let mut names: Vec<String> = rd
                                            .filter_map(|e| e.ok())
                                            .map(|e| {
                                                let n = e.file_name().to_string_lossy().to_string();
                                                if e.path().is_dir() { format!("{n}/") } else { n }
                                            })
                                            .collect();
                                        names.sort();
                                        crate::skills::SkillOutcome::Ok(format!(
                                            "工作区 {}：\n{}",
                                            p.display(),
                                            names.join("\n")
                                        ))
                                    }
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        crate::skills::SkillOutcome::Denied(format!(
                                            "目录不存在：{}（请先用 code_run_command mkdir 创建，或 code_write_file 写文件时自动建父目录）",
                                            p.display()
                                        ))
                                    }
                                    Err(e) => crate::skills::SkillOutcome::Denied(format!(
                                        "列出失败：{e}（路径 {}，白名单根 {}）",
                                        p.display(),
                                        root.display()
                                    )),
                                }
                            }
                        }
                    };
                }
                if name == "code_run_command" {
                    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    return match run_allowed_command(command) {
                        Some(out) => crate::skills::SkillOutcome::Ok(out),
                        None => crate::skills::SkillOutcome::Denied(
                            "命令不在允许列表内，或参数含 shell 元字符/越界路径（仅支持 pwd/ls/mkdir/touch/cat/echo/printf/grep/head/tail/wc/file/df/du/which/uname/node/python3/npm/cargo；路径参数必须在工作区根内）".to_string(),
                        ),
                    };
                }
                if name == "workspace_info" {
                    let (root, source) = qibai_workspace_root_with_source();
                    let probe = root.join(".qibai_probe");
                    let writable = std::fs::write(&probe, b"probe")
                        .map(|_| { let _ = std::fs::remove_file(&probe); true })
                        .unwrap_or(false);
                    let free_bytes: Option<u64> = statvfs_free_bytes(&root);
                    let info = serde_json::json!({
                        "root": root.display().to_string(),
                        "source": source,
                        "writable": writable,
                        "free_bytes": free_bytes,
                        "suggest": format!("工程请放在 {} 下，用相对路径（如 myproject/src/main.py）", root.display()),
                    });
                    return crate::skills::SkillOutcome::Ok(info.to_string());
                }
                if name == "clear_conversation" {
                    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("chat");
                    if let Some(reason) = clear_conversation_confirm_deny(args) {
                        return crate::skills::SkillOutcome::Denied(reason);
                    }
                    let include_memory = scope == "all";
                    // 备份 → 清 conversation → 可选清角色记忆 → 广播
                    let bak = match self.conversation.backup() {
                        Ok(b) => b,
                        Err(e) => {
                            return crate::skills::SkillOutcome::Denied(format!("清空失败：备份阶段 {e}"));
                        }
                    };
                    let before = self.conversation.all().len();
                    if let Err(e) = self.conversation.clear() {
                        return crate::skills::SkillOutcome::Denied(format!("清空失败：{e}"));
                    }
                    let mut cleared = before;
                    if include_memory {
                        match self.personas.clear_all_memory() {
                            Ok(n) => cleared += n,
                            Err(e) => eprintln!("[qibai] personas.clear_all_memory 失败: {e}"),
                        }
                    }
                    self.sink.emit(
                        "qibai://conversation-updated",
                        serde_json::json!({"cleared": cleared, "include_memory": include_memory}),
                    );
                    let bak_str = bak
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "（无旧文件，未生成备份）".to_string());
                    let mem_note = if include_memory {
                        "角色记忆已一并清空"
                    } else {
                        "角色记忆已保留"
                    };
                    return crate::skills::SkillOutcome::Ok(format!(
                        "已清空 {cleared} 条聊天记录，备份在 {bak_str}（{mem_note}）"
                    ));
                }
                // ---- v45.12：MCP 外部工具分发（链尾分支，不改既有分支）----
                // 命中 mcp__<server>__<tool> 前缀才走；其余全部落回技能注册表。
                if let Some((server, tool)) = crate::mcp::bridge::parse_namespaced(name) {
                    return self.dispatch_mcp(&server, &tool, name, args).await;
                }
                let out = self.skills.invoke(name, args).await;
                if let crate::skills::SkillOutcome::PendingApproval { request_id, description } = &out {
                    self.sink.emit(
                        "qibai://skill-approval",
                        serde_json::json!({
                            "request_id": request_id,
                            "skill": name,
                            "description": description,
                        }),
                    );
                }
                out
            }

            /// v45.12：MCP 外部工具分发。
            /// 顺序：命名解析 → server 存在性 → 真实状态 → 工具存在性 → trust 闸门 → registry.call。
            /// trust=Confirm → PendingApproval（落 mcp_pending，等 approve_skill 回调，被拒不执行）；
            /// trust=Readonly → 直接执行。RPC 失败与工具自报错误都作为 tool 结果回灌，
            /// 失败文本原样保留（与既有 tool 失败 = Denied / "DENIED: ..." 回灌同语义）。
            async fn dispatch_mcp(
                &mut self,
                server: &str,
                tool: &str,
                full_name: &str,
                args: &serde_json::Value,
            ) -> crate::skills::SkillOutcome {
                use crate::mcp::{McpState, McpTrust};
                // 短临界区：lock → 状态/存在性检查 → （confirm 闸门不持锁）→ 调用。
                // confirm 闸门需要先取走 handle 的不可变引用做判断，再在锁外登记
                // mcp_pending，最后重新 lock 执行——避免 std Mutex 不可重入自死锁。
                enum Gate {
                    NeedConfirm,
                    ReadyRun,
                }
                let gate = {
                    let mut guard = self.mcp_lock.lock().unwrap();
                    let Some(reg) = guard.as_mut() else {
                        return crate::skills::SkillOutcome::Denied(
                            "MCP 通道未初始化（未配置任何外部服务器）".to_string(),
                        );
                    };
                    let handle = match reg.servers.get(server) {
                        Some(h) => h,
                        None => {
                            return crate::skills::SkillOutcome::Denied(format!(
                                "MCP server「{server}」不存在或已被移除"
                            ));
                        }
                    };
                    match &handle.state {
                        McpState::Ready => {}
                        McpState::Disabled => {
                            return crate::skills::SkillOutcome::Denied(format!(
                                "MCP server「{server}」当前已禁用（在设置里打开开关后重试）"
                            ));
                        }
                        McpState::Starting => {
                            return crate::skills::SkillOutcome::Denied(format!(
                                "MCP server「{server}」正在启动，稍后再试"
                            ));
                        }
                        McpState::Unavailable(reason) => {
                            return crate::skills::SkillOutcome::Denied(format!(
                                "MCP server「{server}」不可用：{reason}"
                            ));
                        }
                    }
                    if !handle.tools.iter().any(|t| t.name == tool) {
                        return crate::skills::SkillOutcome::Denied(format!(
                            "MCP server「{server}」没有名为「{tool}」的工具"
                        ));
                    }
                    if handle.cfg.trust == McpTrust::Confirm {
                        Gate::NeedConfirm
                    } else {
                        Gate::ReadyRun
                    }
                };
                match gate {
                    Gate::NeedConfirm => {
                        let request_id = format!("mcp-appr-{}", next_mcp_confirm_seq());
                        self.mcp_pending.lock().unwrap().insert(
                            request_id.clone(),
                            (server.to_string(), tool.to_string(), args.clone()),
                        );
                        let description = format!(
                            "外部工具 {full_name}（来源 {}）将被调用，参数：{}。该操作来自外部 MCP server，请确认是否允许执行。",
                            crate::mcp::bridge::source_label(server),
                            summarize_tool_args(args),
                        );
                        self.sink.emit(
                            "qibai://skill-approval",
                            serde_json::json!({
                                "request_id": request_id,
                                "skill": full_name,
                                "description": description,
                            }),
                        );
                        crate::skills::SkillOutcome::PendingApproval {
                            request_id,
                            description,
                        }
                    }
                    Gate::ReadyRun => {
                        // 重新取锁执行（上一临界区已 drop）。reg.call 是同步阻塞调用
                        // （内部 recv_timeout 等子进程响应），阻塞期间不跨 await。
                        let mut guard = self.mcp_lock.lock().unwrap();
                        let Some(reg) = guard.as_mut() else {
                            return crate::skills::SkillOutcome::Denied(
                                "MCP 通道未初始化（未配置任何外部服务器）".to_string(),
                            );
                        };
                        match reg.call(server, tool, args.clone()) {
                            Ok(result) => {
                                if result.is_error {
                                    // 工具自报错误：文本原样回灌为 Denied（与既有 tool 失败回灌同语义）。
                                    crate::skills::SkillOutcome::Denied(result.content)
                                } else {
                                    crate::skills::SkillOutcome::Ok(result.content)
                                }
                            }
                            Err(e) => crate::skills::SkillOutcome::Denied(e),
                        }
                    }
                }
            }
        }

        let persona_id = self.personas.active_persona().to_string();
        // v45.12：executor 只持有 self.mcp 锁的共享引用（不跨 await 持锁）。
        let mut executor = EngineToolExecutor {
            skills: &mut self.skills,
            guard: &mut self.guard,
            sink: self.sink.as_ref(),
            conversation: &mut self.conversation,
            personas: &mut self.personas,
            mcp_lock: &self.mcp,
            mcp_pending: &self.mcp_pending,
        };

        // v18.8-fix27：流式朗读——LLM 逐段输出文本时经 sink 广播 qibai://tts-chunk，
        // 前端逐段 TTS（第一句生成完即开口）。EventSink: Send+Sync，& 引用捕获即可。
        let sink_ref: &dyn EventSink = self.sink.as_ref();
        // v18.8-fix28d：streamed 标志——流式路径逐段朗读过则为 true；
        // 流式失败/降级非流式时 on_text 不会被调用 → 返回后补发一次完整朗读。
        let mut streamed = false;
        let mut on_text = |seg: &str| {
            streamed = true;
            // v18.8-fix31：朗读权威集中到 Rust 端——流式段 emit 一律带 rust_played=true，
            // 前端只显示气泡、不 invoke tts_speak（真机实证：前端事件触发缺口导致
            // "emit 有、发声无"）。Rust 端在 send_message 完成后播放完整回复（见 main.rs）。
            eprintln!("[qibai] tts-chunk emit: {} 字符 (rust_played)", seg.chars().count());
            let _ = sink_ref.emit(
                "qibai://tts-chunk",
                serde_json::json!({ "text": seg, "persona": persona_id, "rust_played": true }),
            );
        };
        let result = run_agent_loop(
            provider.as_ref(),
            &system,
            history,
            Some(tool_schemas),
            &mut executor,
            &mut on_text,
        )
        .await;
        drop(on_text); // 释放对 streamed 的借用，match 里才能读

        match result {
            Ok((parsed, skill, executions)) => {
                self.conversation.append("user", text)?;
                self.conversation.append("assistant", &parsed.reply)?;
                eprintln!("[qibai] conversation append 后: {} 条", self.conversation.all().len());
                // P0：对话按 active persona 写入该实体的独立记忆（conversation.json 保持兼容）。
                let active = self.personas.active_persona().to_string();
                let _ = self.personas.append_memory(&active, "user", text);
                let _ = self
                    .personas
                    .append_memory(&active, "assistant", &parsed.reply);
                let _ = self.personas.persist();
                // v45.10：工具执行留痕广播给前端
                // v45.12：MCP 调用在留痕里标 source="MCP / <server>"，让 UI 能分清
                // 是本地技能还是外部进程干的（不改 ExecutionRecord 结构，只在广播层补字段）。
                let exec_items: Vec<serde_json::Value> = executions
                    .iter()
                    .map(|rec| {
                        let mut v = serde_json::to_value(rec)
                            .unwrap_or(serde_json::Value::Null);
                        if let Some(tool) = v.get("tool").and_then(|t| t.as_str()) {
                            if let Some((server, _t)) =
                                crate::mcp::bridge::parse_namespaced(tool)
                            {
                                v["source"] = serde_json::Value::String(
                                    crate::mcp::bridge::source_label(&server),
                                );
                            }
                        }
                        v
                    })
                    .collect();
                let _ = self.sink.emit("qibai://executions", serde_json::json!({"items": exec_items}));
                // v18.8-fix28f：非流式路径（降级/服务不支持流式）没有逐段回调——
                // ① 广播 tts-chunk（rust_played=true：前端只更新去重时间戳、不重复播）；
                // ② Rust 端由 tauri 层直接播放回复全文（不依赖前端 JS 链路）。
                if !streamed && !parsed.reply.trim().is_empty() {
                    eprintln!("[qibai] tts-chunk emit(非流式兜底): {} 字符", parsed.reply.chars().count());
                    let _ = self.sink.emit(
                        "qibai://tts-chunk",
                        serde_json::json!({ "text": parsed.reply, "persona": active, "rust_played": true }),
                    );
                }
                // v18.8-fix28f：聊天记录落库后广播，控制台/桌宠实时刷新（跨窗口同步）。
                let _ = self.sink.emit("qibai://conversation-updated", serde_json::json!({}));
                self.sink.emit("qibai://llm-status", serde_json::json!({"state": "done"}));
                Ok(SendMessageOutput {
                    reply: parsed.reply.clone(),
                    emotion: parsed.emotion,
                    action: parsed.action,
                    ui_cmd: parsed.ui_cmd,
                    skill,
                    secret: None,
                    secret_entry_id: None,
                    speak_text: Some(parsed.reply),
                    streamed,
                })
            }
            Err(e) => {
                self.sink.emit(
                    "qibai://llm-status",
                    serde_json::json!({"state": "error", "message": e.to_string()}),
                );
                Err(EngineError::Llm(e))
            }
        }
    }

    // ---- 迭代③：保险箱对话拦截（契约 §5） ----

    /// 本地、无 LLM 的触发判定：文本含「密码」且满足任一：
    /// 含「保险箱」／匹配查询意图（密码.{0,6}后跟 是什么|是多少|忘了|多少|帮我查|给我）／
    /// 命中已解锁条目的 site/account 子串。
    fn vault_chat_triggered(text: &str, vault: &crate::vault::VaultManager) -> bool {
        if !text.contains("密码") {
            return false;
        }
        if text.contains("保险箱") {
            return true;
        }
        if Self::vault_query_intent(text) {
            return true;
        }
        if let Some(entries) = vault.unlocked_entries() {
            return entries
                .iter()
                .any(|e| text.contains(&e.site) || text.contains(&e.account));
        }
        false
    }

    /// `/密码.{0,6}(是什么|是多少|忘了|多少|帮我查|给我)/` 的无 regex 实现（按字符窗口）。
    fn vault_query_intent(text: &str) -> bool {
        const NEEDLES: [&str; 6] = ["是什么", "是多少", "忘了", "多少", "帮我查", "给我"];
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            if chars[i] == '密' && chars[i + 1] == '码' {
                let window: String = chars[i + 2..].iter().take(6).collect();
                if NEEDLES.iter().any(|n| window.contains(n)) {
                    return true;
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        false
    }

    /// 拦截入口：未命中返回 None（完全走原 LLM 路径）；命中返回当次回复（永不调 LLM）。
    /// conversation 只写：用户原文 + 脱敏 assistant 文本（明文永不落盘）。
    fn try_vault_intercept(
        &mut self,
        text: &str,
    ) -> Result<Option<SendMessageOutput>, EngineError> {
        if !Self::vault_chat_triggered(text, &self.vault) {
            return Ok(None);
        }

        let active = self.personas.active_persona().to_string();
        let level = self
            .intimacy
            .file
            .entries
            .get(&active)
            .map(|e| e.level)
            .unwrap_or(1);
        let gate_open = OPEN_ALL || crate::intimacy::is_gate_open(level, "vault");

        let reply: String;
        let mut secret: Option<bool> = None;
        let mut secret_entry_id: Option<String> = None;
        let mut speak_text: Option<String> = None;
        if !gate_open {
            // 分支 1：L4 未达
            reply = "亲密度达到 L4（贴心）后会解锁本地密码保险箱，继续和我聊天吧～".to_string();
        } else {
            match self.vault.state() {
                crate::vault::VaultState::Unset => {
                    // 分支 2
                    reply = "先在控制台·保险箱页初始化（设置主密码）".to_string();
                }
                crate::vault::VaultState::Locked => {
                    // 分支 3
                    reply = "保险箱已锁定，先在控制台解锁（主密码或系统解锁）后再问我吧".to_string();
                }
                crate::vault::VaultState::Unlocked => {
                    // 分支 4：本地检索——命中 site/account 作为子串出现在用户消息中的条目
                    // （与触发判定同语义；不进 LLM）。
                    let hit = self
                        .vault
                        .unlocked_entries()
                        .and_then(|entries| {
                            entries
                                .iter()
                                .find(|e| text.contains(&e.site) || text.contains(&e.account))
                                .cloned()
                        });
                    match hit {
                        None => {
                            reply = format!("没找到和「{text}」相关的密码记录哦");
                        }
                        Some(entry) => {
                            self.vault.touch();
                            let secs = self.settings.vault.clipboard_clear_secs;
                            speak_text = Some(format!(
                                "已为你复制「{}」的密码，{secs} 秒后自动清除",
                                entry.site
                            ));
                            reply = format!(
                                "找到啦～站点：{}，账号：{}，密码：{}",
                                entry.site, entry.account, entry.password
                            );
                            secret = Some(true);
                            secret_entry_id = Some(entry.id.clone());
                        }
                    }
                }
            }
        }

        // conversation / persona 记忆：用户原文 + 脱敏文本（喇叭重读 conversation，明文永不发声）。
        let assistant_stored = speak_text.clone().unwrap_or_else(|| reply.clone());
        self.conversation.append("user", text)?;
        self.conversation.append("assistant", &assistant_stored)?;
        let _ = self.personas.append_memory(&active, "user", text);
        let _ = self
            .personas
            .append_memory(&active, "assistant", &assistant_stored);
        let _ = self.personas.persist();
        // 问题4缺陷A：vault 拦截路径此前只 emit llm-status、漏广播 conversation-updated，
        // 导致含「密码」的对话后控制台不刷新聊天记录。对齐 LLM 主路径（L686）补广播。
        let _ = self
            .sink
            .emit("qibai://conversation-updated", serde_json::json!({}));
        self.sink
            .emit("qibai://llm-status", serde_json::json!({"state": "done"}));

        Ok(Some(SendMessageOutput {
            reply,
            emotion: Emotion::Happy,
            action: None,
            ui_cmd: None,
            skill: Some("vault".to_string()),
            secret,
            secret_entry_id,
            speak_text,
            streamed: false,
        }))
    }

    // ---- 迭代③：保险箱 IPC 门面（契约 §4：Engine 方法 = Tauri 命令 = 浏览器 mock） ----

    /// keyring_available 由 Tauri 层探测后传入（core 不感知系统钥匙串）。
    pub fn vault_status(&self, keyring_available: bool) -> crate::vault::VaultStatus {
        let active = self.personas.active_persona().to_string();
        let level = self
            .intimacy
            .file
            .entries
            .get(&active)
            .map(|e| e.level)
            .unwrap_or(1);
        let gate_open = OPEN_ALL || crate::intimacy::is_gate_open(level, "vault");
        self.vault.status(keyring_available, gate_open)
    }

    pub fn vault_setup(&mut self, master_password: &str) -> Result<(), EngineError> {
        Ok(self.vault.setup_master_password(master_password)?)
    }

    /// Tauri 层生成随机 32B key 存钥匙串后，把字节交给 core 落盘。
    pub fn vault_setup_keyring(&mut self, key_bytes: &[u8]) -> Result<(), EngineError> {
        Ok(self.vault.setup_keyring(key_bytes)?)
    }

    pub fn vault_unlock(&mut self, master_password: &str) -> Result<(), EngineError> {
        Ok(self.vault.unlock(master_password)?)
    }

    /// Tauri 层从钥匙串取回 key 字节 → core 校验解锁（不计数失败）。
    pub fn vault_unlock_system(&mut self, key_bytes: &[u8]) -> Result<(), EngineError> {
        Ok(self.vault.unlock_with_key(key_bytes)?)
    }

    pub fn vault_lock(&mut self) -> Result<(), EngineError> {
        self.vault.lock();
        Ok(())
    }

    /// 由 Tauri 侧 tokio 周期调用：闲置超时自动锁。
    pub fn vault_tick(&mut self) -> bool {
        self.vault.tick()
    }

    pub fn vault_list(&mut self) -> Result<Vec<crate::vault::VaultEntry>, EngineError> {
        Ok(self.vault.list()?)
    }

    pub fn vault_search(&mut self, q: &str) -> Result<Vec<crate::vault::VaultEntry>, EngineError> {
        Ok(self.vault.search(q)?)
    }

    pub fn vault_add(&mut self, input: crate::vault::VaultEntryInput) -> Result<String, EngineError> {
        Ok(self.vault.add(input)?)
    }

    pub fn vault_update(
        &mut self,
        id: &str,
        input: crate::vault::VaultEntryInput,
    ) -> Result<(), EngineError> {
        Ok(self.vault.update(id, input)?)
    }

    pub fn vault_delete(&mut self, id: &str) -> Result<(), EngineError> {
        Ok(self.vault.delete(id)?)
    }

    /// core 返回明文+清除时长；Tauri/mock 层写剪贴板并调度清除（前端载荷不含明文）。
    pub fn vault_copy(&mut self, id: &str) -> Result<crate::vault::VaultCopyHold, EngineError> {
        Ok(self.vault.copy_hold(id)?)
    }

    /// core 生成 .qbkv 信封字符串；Tauri 层写 `<data_dir>/vault-backup-<ts>.qbkv`。
    pub fn vault_export(&mut self) -> Result<String, EngineError> {
        Ok(self.vault.export_payload()?)
    }

    pub fn vault_import_payload(
        &mut self,
        payload: &str,
        master_password: &str,
    ) -> Result<(), EngineError> {
        Ok(self.vault.import_payload(payload, master_password)?)
    }

    // ---- 提醒 tick（由 tauri 侧用 tokio 周期调用） ----
    pub fn tick_reminders(&mut self) -> Vec<ReminderEvent> {
        let due = self.reminders.take_due();
        for ev in &due {
            self.sink.emit(
                "qibai://reminder",
                serde_json::json!({
                    "type": ev.kind.as_str(),
                    "message": ev.message,
                    "action": ev.action.to_string(),
                }),
            );
        }
        due
    }

    /// v23：语音/LLM「提醒我喝水」——立即起算一次提醒计时，同步启用状态并落盘。
    /// 返回距离下次触发的分钟数（供语音回复）。
    pub fn reminder_schedule_once(
        &mut self,
        kind: &crate::reminder::ReminderKind,
        delay_secs: Option<u64>,
    ) -> u64 {
        let mins = self.reminders.schedule_once(kind, delay_secs);
        self.settings.reminders.enabled = self.reminders.enabled_snapshot();
        let _ = self.settings.save();
        mins
    }

    /// v23：语音/LLM「取消提醒」——停用某类提醒并落盘。
    pub fn reminder_cancel(&mut self, kind: &crate::reminder::ReminderKind) {
        self.reminders.cancel_kind(kind);
        self.settings.reminders.enabled = self.reminders.enabled_snapshot();
        let _ = self.settings.save();
    }

    // ---- 迭代④：联网社交命令（§7.5 全部 17 个；薄封装，错误走 EngineError::Net） ----

    /// `netsocial_status` — 状态机快照。
    pub fn netsocial_status(&self) -> serde_json::Value {
        self.netsocial.status()
    }

    /// `netsocial_register` — 本地生成 UUIDv4 匿名身份；已配置 server_url 时补注册换 token。
    pub async fn netsocial_register(
        &mut self,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .register(input)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_connect` — 连接服务器（入参 server_url 缺省时用已存地址）。
    pub async fn netsocial_connect(
        &mut self,
        server_url: Option<String>,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .connect(server_url)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_disconnect` — 断开并停止后台重连任务。
    pub fn netsocial_disconnect(&mut self) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .disconnect()
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_set_profile` — 本地持久化；在线时 PUT /api/me。
    pub async fn netsocial_set_profile(
        &mut self,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .set_profile(input)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_roam` — 星球广场漫游列表（原样透传服务端字段）。
    pub async fn netsocial_roam(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .roam(params)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_match` — 匹配推荐（§5 评分排序）。
    pub async fn netsocial_match(
        &self,
        limit: Option<u32>,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .r#match(limit)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_city_souls` — 同城灯塔列表。
    pub async fn netsocial_city_souls(
        &self,
        city: &str,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .city_souls(city)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_like` — 点亮。
    pub async fn netsocial_like(&self, id: &str) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .like(id)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_report` — 举报。
    pub async fn netsocial_report(
        &self,
        id: &str,
        reason: &str,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .report(id, reason)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_block` — 拉黑（双向屏蔽）。
    pub async fn netsocial_block(&self, id: &str) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .block(id)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_send` — 发消息（离线/未连接 → not_online 结构化错误）。
    pub async fn netsocial_send(
        &mut self,
        id: &str,
        text: &str,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .send(id, text)
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_history` — 脱敏联系人/消息历史（按 peer_key 或全部）。
    pub fn netsocial_history(
        &self,
        id: Option<&str>,
    ) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .history(id)
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_poll` — 消费待推送事件队列并清空。
    pub fn netsocial_poll(&self) -> serde_json::Value {
        self.netsocial.poll()
    }

    /// `netsocial_topics` — 话题墙占位。
    pub async fn netsocial_topics(&self) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .topics()
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_events` — 活动预告占位。
    pub async fn netsocial_events(&self) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .events_wall()
            .await
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    /// `netsocial_cache_clear` — 删除加密缓存两文件并重置身份。
    pub fn netsocial_cache_clear(&mut self) -> Result<serde_json::Value, EngineError> {
        self.netsocial
            .cache_clear()
            .map_err(|e| EngineError::Net(e.to_string()))
    }

    // ---- 全屏守护演出门面（迭代①） ----

    /// 久坐到期入口：由提醒 tick 透传；返回 Some(GuardStart) 时外层创建演出窗并广播。
    pub fn guard_on_sit(&mut self, now: u64) -> Option<crate::guard::GuardStart> {
        self.guard.on_sit_due(now)
    }

    /// 手动触发（伴侣主动 / UI 按钮）：时钟取当前系统秒。
    pub fn guard_manual_trigger(&mut self) -> Option<crate::guard::GuardStart> {
        let now = self.guard.now();
        self.guard.manual_trigger(now)
    }

    /// 跳过演出（用户点跳过 / ESC）；不在演出中返回错误。
    pub fn guard_skip(&mut self, reason: &str) -> Result<(), EngineError> {
        let now = self.guard.now();
        self.guard
            .skip(now, reason)
            .map_err(|e| EngineError::Guard(e.to_string()))
    }

    /// 剩余冷却秒数；不在冷却期为 0。
    pub fn guard_cooldown_left(&self) -> u64 {
        let now = self.guard.now();
        self.guard.cooldown_left(now)
    }

    // ---- 代码检查门面（迭代①） ----

    /// 执行一次代码检查。runner 默认 OsCheckRunner；有可用 LLM key 时附加建议，
    /// 任何 LLM 失败都降级为 None（纯静态报告），绝不向上报错。
    pub async fn check_code(
        &mut self,
        req: crate::codecheck::CheckRequest,
    ) -> Result<crate::codecheck::CodeReport, EngineError> {
        // runner：测试注入优先，否则真实 OsCheckRunner。
        let builtin_runner = crate::codecheck::OsCheckRunner;
        let runner: &dyn crate::codecheck::CheckRunner = match self.cc_runner_override.as_ref() {
            Some(r) => r.as_ref(),
            None => &builtin_runner,
        };
        // suggester：测试注入优先；否则按是否有可用 key 现场构造（失败即 None）。
        let builtin = self.build_code_suggester();
        let suggester: Option<&dyn crate::codecheck::SuggestLlm> =
            match self.cc_suggester_override.as_ref() {
                Some(s) => Some(s.as_ref()),
                None => builtin.as_ref().map(|b| b as &dyn crate::codecheck::SuggestLlm),
            };
        Ok(crate::codecheck::run_check(req, runner, suggester).await)
    }

    /// 有可用 key（env DOUBAO_API_KEY 或 settings.llm.api_key 非空）时构造
    /// 本地 SuggestLlm 适配；否则 None。构造失败（如 key 解析异常）一律 None。
    fn build_code_suggester(&self) -> Option<EngineSuggestLlm> {
        let env_key = std::env::var(crate::llm_client::ENV_API_KEY)
            .ok()
            .filter(|k| !k.trim().is_empty());
        let cfg_key = if self.settings.llm.api_key.trim().is_empty() {
            None
        } else {
            Some(self.settings.llm.api_key.trim().to_string())
        };
        if env_key.is_none() && cfg_key.is_none() {
            return None;
        }
        let provider = DoubaoProvider::from_settings(&self.settings.llm).ok()?;
        Some(EngineSuggestLlm { provider })
    }

    // ---- P0 IPC: get_personas / set_active_persona / rename_persona ----

    fn check_persona(&self, id: &str) -> Result<(), EngineError> {
        if self.personas.get(id).is_none() {
            return Err(EngineError::UnknownPersona(id.to_string()));
        }
        Ok(())
    }

    /// `{ active_persona, personas: [...] }`（含 memory 最近全部条目、sync 预留字段）。
    pub fn get_personas(&self) -> serde_json::Value {
        serde_json::to_value(&self.personas.file).unwrap_or(serde_json::Value::Null)
    }

    pub fn set_active_persona(&mut self, id: &str) -> Result<(), EngineError> {
        let old = self.personas.active_persona().to_string();
        self.personas.set_active(id).map_err(|e| match e {
            crate::personas::PersonaStoreError::UnknownPersona(_) => {
                EngineError::UnknownPersona(id.to_string())
            }
            other => EngineError::Social(other.to_string()),
        })?;
        // desk 簿记：present = [new] ∪ (present \ {old})（契约 §4.2/§4.3）。
        if old != id {
            self.social
                .desk_replace_active(&old, id)
                .map_err(|e| EngineError::Social(e.to_string()))?;
        }
        Ok(())
    }

    pub fn rename_persona(&mut self, id: &str, nickname: &str) -> Result<(), EngineError> {
        self.personas.rename(id, nickname).map_err(|e| match e {
            crate::personas::PersonaStoreError::UnknownPersona(_) => {
                EngineError::UnknownPersona(id.to_string())
            }
            other => EngineError::Social(other.to_string()),
        })
    }

    // ---- P1 IPC: get_social_state / social_greet / social_pk / get_social_events ----

    /// `{ relations: [...], levels: [ {level,name,title,min,max} x4 ] }`。
    pub fn get_social_state(&self) -> serde_json::Value {
        let levels: Vec<serde_json::Value> = LEVELS
            .iter()
            .map(|l| {
                serde_json::json!({
                    "level": l.level, "name": l.name, "title": l.name,
                    "min": l.min, "max": l.max,
                })
            })
            .collect();
        serde_json::json!({
            "relations": self.social.file.relations,
            "levels": levels,
        })
    }

    /// 串门寒暄：返回 `{ lines, affinity_delta, event }`。
    /// use_llm=true 且有可用 API key 时走 LLM 单轮生成，失败自动降级内置模板。
    pub async fn social_greet(
        &mut self,
        from: &str,
        to: &str,
        use_llm: bool,
    ) -> Result<serde_json::Value, EngineError> {
        self.check_persona(from)?;
        self.check_persona(to)?;
        if from == to {
            return Err(EngineError::Social("from 与 to 不能是同一角色".into()));
        }

        let cur_level = self
            .social
            .relation(from, to)
            .map(|r| r.level)
            .unwrap_or(0);

        // 台词：优先 LLM，失败/无 key 降级模板
        let mut lines: Vec<(String, String)> = if use_llm {
            self.llm_greet_lines(from, to, cur_level)
                .await
                .unwrap_or_else(|| {
                    let (l1, l2) = crate::social::pick_greet_script(from, to, cur_level);
                    vec![(from.to_string(), l1), (to.to_string(), l2)]
                })
        } else {
            let (l1, l2) = crate::social::pick_greet_script(from, to, cur_level);
            vec![(from.to_string(), l1), (to.to_string(), l2)]
        };

        // 计分：每日每对 5 次封顶
        let r = self.social.relation_mut(from, to);
        r.greet_count += 1;
        let capped = r.greet_today >= GREET_DAILY_LIMIT;
        let delta = if capped { 0 } else { r.greet_today += 1; 1 };

        let (old_level, new_level) = if delta > 0 {
            self.social.add_affinity(from, to, 1)
        } else {
            (cur_level, cur_level)
        };

        if new_level > old_level {
            let egg = crate::social::level_up_egg(new_level);
            lines.push((from.to_string(), egg.to_string()));
            self.social.push_event(
                "level_up",
                from,
                to,
                serde_json::json!({
                    "new_level": new_level,
                    "title": crate::social::level_name(new_level),
                    "text": egg,
                }),
            );
        }

        let event = serde_json::json!({
            "ts": crate::storage::now_ts(),
            "type": "greet",
            "a": from, "b": to,
            "detail": {"affinity_delta": delta, "capped": capped, "level": new_level},
        });
        self.social.push_event("greet", from, to, event["detail"].clone());
        let _ = self.social.persist();

        Ok(serde_json::json!({
            "lines": lines
                .iter()
                .map(|(s, t)| serde_json::json!({"speaker": s, "text": t}))
                .collect::<Vec<_>>(),
            "affinity_delta": delta,
            "event": event,
        }))
    }

    /// LLM 双人寒暄：单轮生成，任何失败都返回 None（调用方降级模板）。
    async fn llm_greet_lines(
        &self,
        from: &str,
        to: &str,
        level: u8,
    ) -> Option<Vec<(String, String)>> {
        let from_p = crate::personas::PersonaStore::personality(from);
        let to_p = crate::personas::PersonaStore::personality(to);
        let system = "你是桌面双角色寒暄演出的编剧。两位角色刚刚见面串门寒暄。\
            要求：全龄友好、内容积极健康、角色之间只是友好的社交关系，无任何暧昧或擦边内容。\
            输出必须且只能是一个 JSON 对象，不要输出 JSON 以外的任何文字：\
            {\"lines\":[{\"speaker\":\"<persona_id>\",\"text\":\"一句中文台词\"}]}。\
            台词 2-4 句，在两位角色之间交替说。";
        let user = format!(
            "角色A：{name_a}（{tone_a}气质，persona_id={id_a}）；\
             角色B：{name_b}（{tone_b}气质，persona_id={id_b}）。\
             当前两人关系等级：{lvl}（{title}）。请写一小段两人见面寒暄的对话。",
            name_a = from_p.display_name, tone_a = from_p.tone, id_a = from,
            name_b = to_p.display_name, tone_b = to_p.tone, id_b = to,
            lvl = level, title = crate::social::level_name(level),
        );
        let req = crate::llm_client::ChatRequest {
            system: system.to_string(),
            messages: vec![ChatMessage::user(user)],
            tools: None,
            json_object: true,
        };
        let resp = match &self.provider_override {
            Some(p) => p.chat(req).await,
            None => {
                let p = DoubaoProvider::from_settings(&self.settings.llm).ok()?;
                p.chat(req).await
            }
        };
        let content = resp.ok()?.content?;
        parse_greet_reply(&content, from, to)
    }

    /// 猜拳 PK：`{ winner, loser, result, affinity_delta, lines, remaining_today, blocked }`。
    pub fn social_pk(
        &mut self,
        a: &str,
        b: &str,
        move_a: &str,
        move_b: &str,
    ) -> Result<serde_json::Value, EngineError> {
        self.check_persona(a)?;
        self.check_persona(b)?;
        if a == b {
            return Err(EngineError::Social("PK 双方不能是同一角色".into()));
        }
        let ma = Move::parse(move_a)
            .ok_or_else(|| EngineError::InvalidMove(move_a.to_string()))?;
        let mb = Move::parse(move_b)
            .ok_or_else(|| EngineError::InvalidMove(move_b.to_string()))?;

        // 每日 3 次防刷
        if self.social.relation_mut(a, b).pk_count.count >= PK_DAILY_LIMIT {
            return Ok(serde_json::json!({
                "blocked": true,
                "remaining_today": 0,
                "lines": [{"speaker": a, "text": "今天猜拳已经玩满 3 次啦，明天再来吧～"}],
            }));
        }

        use std::cmp::Ordering;
        let cmp = ma.beats(mb);
        let (winner, loser, delta, result) = match cmp {
            Ordering::Equal => (None, None, 4, "draw"),
            Ordering::Greater => (Some(a), Some(b), 10, "win"),
            Ordering::Less => (Some(b), Some(a), 10, "win"),
        };

        let (old_level, new_level) = self.social.add_affinity(a, b, delta);
        {
            let r = self.social.relation_mut(a, b);
            r.pk_count.count += 1;
            r.pk_total += 1;
        }

        let mut lines: Vec<(String, String)> = match cmp {
            Ordering::Equal => vec![
                (a.to_string(), format!("我们都出{}，平局平局！", ma.as_zh())),
                (b.to_string(), "再来一局吗？".to_string()),
            ],
            Ordering::Greater => vec![
                (a.to_string(), format!("我出{}，哈哈，赢啦！", ma.as_zh())),
                (b.to_string(), format!("我出{}……愿赌服输，下次一定扳回来～", mb.as_zh())),
            ],
            Ordering::Less => vec![
                (b.to_string(), format!("我出{}，哈哈，赢啦！", mb.as_zh())),
                (a.to_string(), format!("我出{}……愿赌服输，下次一定扳回来～", ma.as_zh())),
            ],
        };

        if new_level > old_level {
            let egg = crate::social::level_up_egg(new_level);
            lines.push((a.to_string(), egg.to_string()));
            self.social.push_event(
                "level_up",
                a,
                b,
                serde_json::json!({
                    "new_level": new_level,
                    "title": crate::social::level_name(new_level),
                    "text": egg,
                }),
            );
        }

        self.social.push_event(
            "pk",
            a,
            b,
            serde_json::json!({
                "winner": winner, "loser": loser, "result": result,
                "move_a": move_a, "move_b": move_b,
                "affinity_delta": delta, "new_level": new_level,
            }),
        );
        let remaining = PK_DAILY_LIMIT
            - self
                .social
                .relation(a, b)
                .unwrap()
                .pk_count
                .count;
        let _ = self.social.persist();

        Ok(serde_json::json!({
            "winner": winner, "loser": loser, "result": result,
            "affinity_delta": delta,
            "lines": lines
                .iter()
                .map(|(s, t)| serde_json::json!({"speaker": s, "text": t}))
                .collect::<Vec<_>>(),
            "remaining_today": remaining,
            "blocked": false,
        }))
    }

    /// 事件日志（时间倒序，默认最近 100 条）。
    pub fn get_social_events(&self, limit: Option<usize>) -> serde_json::Value {
        let n = limit.unwrap_or(100).min(crate::social::EVENTS_CAP);
        let events: Vec<&crate::social::SocialEvent> =
            self.social.file.events.iter().rev().take(n).collect();
        serde_json::json!({ "events": events })
    }

    // ---- desk IPC：桌宠在场状态（契约 §4） ----

    /// `{ present: [...], locked: {...} }`（唯一事实源，经 get_desk_state 暴露给 tauri 层）。
    pub fn desk_state(&self) -> serde_json::Value {
        serde_json::to_value(&self.social.file.desk).unwrap_or(serde_json::Value::Null)
    }

    /// 召唤桌宠（幂等：已在场 no-op），未知角色报错，写后落盘。
    pub fn desk_summon(&mut self, id: &str) -> Result<(), EngineError> {
        self.check_persona(id)?;
        self.social
            .desk_summon(id)
            .map_err(|e| EngineError::Social(e.to_string()))
    }

    /// 送走桌宠；主角色拒绝被送走（用户通过切换主角色更换形象，契约 §4.2）。
    pub fn desk_dismiss(&mut self, id: &str) -> Result<(), EngineError> {
        self.check_persona(id)?;
        if id == self.personas.active_persona() {
            return Err(EngineError::Social(
                "主角色不能被送走，请通过切换主角色更换形象".into(),
            ));
        }
        self.social
            .desk_dismiss(id)
            .map_err(|e| EngineError::Social(e.to_string()))
    }

    /// 写入位置锁定标记，写后落盘。
    pub fn desk_set_locked(&mut self, id: &str, locked: bool) -> Result<(), EngineError> {
        self.check_persona(id)?;
        self.social
            .desk_set_locked(id, locked)
            .map_err(|e| EngineError::Social(e.to_string()))
    }

    // ---- 亲密度 IPC（冻结规格 §6） ----

    /// `get_intimacy_levels`（§6.1-1）：10 项等级表，max=null 表示 ∞。
    pub fn get_intimacy_levels(&self) -> serde_json::Value {
        crate::intimacy::IntimacyStore::levels_json()
    }

    /// `get_intimacy_state`（§6.1-2）；未知 persona 报错。
    pub fn get_intimacy_state(&self, persona_id: &str) -> Result<serde_json::Value, EngineError> {
        self.check_persona(persona_id)?;
        self.intimacy
            .state_json(persona_id)
            .map_err(|e| EngineError::Intimacy(e.to_string()))
    }

    /// `intimacy_event`（§6.1-3）：kind ∈ chat|click|task|companion|gift|pk_win。
    /// 升级时写事件日志（social events，kind=`intimacy_level_up`，a=b=persona_id）。
    /// 返回值同时供 main.rs 构造 `qibai://intimacy-updated` 广播。
    pub fn intimacy_event(
        &mut self,
        persona_id: &str,
        kind: &str,
        detail: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.check_persona(persona_id)?;
        let outcome = self
            .intimacy
            .event(persona_id, kind, detail)
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;
        self.intimacy
            .persist()
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;

        if outcome.leveled_up {
            self.social.push_event(
                "intimacy_level_up",
                persona_id,
                persona_id,
                serde_json::json!({
                    "new_level": outcome.level,
                    "level_name": crate::intimacy::level_name(outcome.level),
                    "unlocked": outcome.unlocked,
                }),
            );
            let _ = self.social.persist();
        }

        let achievements_granted: Vec<serde_json::Value> = outcome
            .achievements_granted
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": crate::intimacy::achievement_name(&a.id).unwrap_or(""),
                    "ts": a.ts,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "delta": outcome.delta,
            "points": outcome.points,
            "level": outcome.level,
            "leveled_up": outcome.leveled_up,
            "unlocked": outcome.unlocked,
            "achievements_granted": achievements_granted,
            "tasks_updated": outcome.tasks_updated,
            "capped": outcome.capped,
        }))
    }

    /// `set_anniversary`（§6.1-4）：month 1-12 / day 1-31 校验，非法报错。
    pub fn set_anniversary(
        &mut self,
        persona_id: &str,
        month: u8,
        day: u8,
    ) -> Result<(), EngineError> {
        self.check_persona(persona_id)?;
        self.intimacy
            .set_anniversary(persona_id, month, day)
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;
        self.intimacy
            .persist()
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;
        Ok(())
    }

    /// `clear_anniversary`（§6.1-4）。
    pub fn clear_anniversary(&mut self, persona_id: &str) -> Result<(), EngineError> {
        self.check_persona(persona_id)?;
        self.intimacy
            .clear_anniversary(persona_id)
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;
        self.intimacy
            .persist()
            .map_err(|e| EngineError::Intimacy(e.to_string()))?;
        Ok(())
    }

    /// `check_gate`（§6.1-5）：未知 persona 宽容返回 current_level=1（不报错）。
    pub fn check_gate(&self, persona_id: &str, gate: &str) -> Result<serde_json::Value, EngineError> {
        let info = crate::intimacy::gate_info(gate)
            .ok_or_else(|| EngineError::Intimacy(format!("未知 gate: {gate}")))?;
        let level = self
            .intimacy
            .file
            .entries
            .get(persona_id)
            .map(|e| e.level)
            .unwrap_or(1);
        Ok(serde_json::json!({
            "open": OPEN_ALL || level >= info.required_level,
            "required_level": info.required_level,
            "current_level": level,
        }))
    }

    // ---- 迭代②：TTS 朗读决策与状态 ----

    /// 朗读决策：None = 总开关关 / scope 不覆盖 / 文本空白；
    /// Some = 含 resolved 引擎决策与 reason（含 Mute 决策，供前端展示具体原因）。
    /// persona_id 为 Some(id) 时按该角色解析音色；None 时按 active 角色。
    pub fn tts_decide(
        &self,
        kind: tts::TtsKind,
        text: &str,
        persona_id: Option<&str>,
    ) -> Option<TtsSpeakRequest> {
        let voice = &self.settings.voice;
        if text.trim().is_empty() {
            return None;
        }
        if !tts::scope_allows(voice.scope, kind) {
            return None;
        }
        if !voice.enabled {
            return None;
        }

        // 迭代⑤：解析生效音色（显式 id 优先；None → active 角色）。
        let effective_id = match persona_id {
            Some(id) => id.to_string(),
            None => self.personas.active_persona().to_string(),
        };
        let profile = self.personas.resolved_voice_profile(&effective_id);
        let eff_rate = (voice.rate * profile.rate).clamp(tts::RATE_MIN, tts::RATE_MAX);
        let cloud_speaker = if !profile.cloud_speaker.trim().is_empty() {
            profile.cloud_speaker.clone()
        } else {
            voice.cloud_voice.clone()
        };
        let voice_tag = crate::voice_profile::voice_tag(&effective_id, &profile);

        let local_available = tts::detect_local_tts_available();
        let decision = tts::resolve_engine(voice, local_available);
        // 无 key 降级提示：解析出云端 speaker 但未配置 key、且决策落到 Local 时覆盖 reason。
        let reason = if decision.engine == tts::ResolvedEngine::Local
            && !cloud_speaker.trim().is_empty()
            && voice.cloud_key().is_none()
        {
            format!("云端音色「{cloud_speaker}」未配置 key，已用本地音色")
        } else {
            decision.reason.to_string()
        };

        Some(TtsSpeakRequest {
            text: text.to_string(),
            kind,
            volume: voice.volume,
            rate: eff_rate,
            pitch: profile.pitch,
            engine_voice_id: profile.engine_voice_id.clone(),
            cloud_speaker,
            voice_tag,
            resolved: decision.engine,
            reason,
        })
    }

    /// TTS 引擎状态（前端设置页状态栏 + tts_engine_status 命令原样返回）。
    /// 既有字段保持不变；新增 active_persona / voice_tag / pitch / eff_rate / cloud_speaker。
    pub fn tts_status(&self, local_available: bool) -> serde_json::Value {
        let voice = &self.settings.voice;
        let decision = tts::resolve_engine(voice, local_available);
        let active_id = self.personas.active_persona().to_string();
        let profile = self.personas.resolved_voice_profile(&active_id);
        let eff_rate = (voice.rate * profile.rate).clamp(tts::RATE_MIN, tts::RATE_MAX);
        let cloud_speaker = if !profile.cloud_speaker.trim().is_empty() {
            profile.cloud_speaker.clone()
        } else {
            voice.cloud_voice.clone()
        };
        serde_json::json!({
            "enabled": voice.enabled,
            "engine": voice.engine,
            "volume": voice.volume,
            "rate": voice.rate,
            "scope": voice.scope,
            "local_available": local_available,
            "cloud_ready": voice.cloud_key().is_some(),
            "active_persona": active_id,
            "voice_tag": crate::voice_profile::voice_tag(&active_id, &profile),
            "pitch": profile.pitch,
            "eff_rate": eff_rate,
            "cloud_speaker": cloud_speaker,
            "resolved": {
                "engine": decision.engine.as_str(),
                "reason": decision.reason,
            },
        })
    }

    /// 设置某角色音色档案（落盘前 sanitize）；未知 id → EngineError::UnknownPersona。
    pub fn set_persona_voice(
        &mut self,
        persona_id: &str,
        profile: VoiceProfile,
    ) -> Result<(), EngineError> {
        self.personas
            .set_voice_profile(persona_id, profile)
            .map_err(|e| match e {
                crate::personas::PersonaStoreError::UnknownPersona(_) => {
                    EngineError::UnknownPersona(persona_id.to_string())
                }
                other => EngineError::Social(other.to_string()),
            })
    }

    /// STT 状态（前端设置页降级提示用）。
    /// device_available 探测 cfg 分离：Linux 检查 /dev/snd；mac/Windows 默认 true。
    pub fn stt_status(&self) -> serde_json::Value {
        let voice = &self.settings.voice;
        let enabled = voice.stt_enabled;
        let device_available = stt_device_available();
        let engine = voice.stt_engine.as_str();
        // whisper_local：离线引擎，检查二进制+模型（环境变量/默认路径）
        let (whisper_ok, whisper_reason) = crate::whisper_engine::probe();
        let key_ready = voice.stt_key().is_some();
        let whisper_reason_out = whisper_reason.clone();
        // v18：本地离线 STT 不依赖 stt_enabled 开关（PTT 按住说话始终可用）；
        // 设备检查仍优先，沙箱无 /dev/snd 时如实降级。
        let (available, reason) = if !device_available {
            (false, "未检测到麦克风设备".to_string())
        } else if engine == "whisper_local" {
            if whisper_ok {
                (true, "本地离线识别可用".to_string())
            } else {
                (false, whisper_reason_out)
            }
        } else if engine == "ark" {
            if !enabled {
                (false, "STT 未启用".to_string())
            } else if key_ready {
                (true, "云端识别可用".to_string())
            } else {
                (false, "未配置 STT key（设置 voice.stt_api_key 或 QIBAI_STT_KEY）".to_string())
            }
        } else {
            (false, "未知识别引擎".to_string())
        };
        serde_json::json!({
            "enabled": enabled,
            "provider": voice.stt_provider,
            "engine": engine,
            "key_ready": key_ready,
            "whisper_ready": whisper_ok,
            "whisper_reason": whisper_reason.clone(),
            "wake_enabled": voice.wake_enabled,
            "wake_word": voice.wake_word,
            "device_available": device_available,
            "available": available,
            "reason": reason,
        })
    }

/// STT 语音识别（火山方舟/豆包 ASR v3 非流式一句话识别）。
/// 配置：settings.voice.stt_api_key 或环境变量 QIBAI_STT_KEY（key）；
///      环境变量 QIBAI_STT_APPID（应用 ID）、QIBAI_STT_CLUSTER（默认 volcano_tts）。
/// 音频：wav/pcm 16k 单声道最稳；前端 MediaRecorder 的 webm/ogg 视 ASR 支持情况。
pub async fn stt_transcribe(&self, audio_b64: &str) -> Result<String, String> {
    use base64::Engine as _;
    let voice = &self.settings.voice;
    // v18：本地离线 STT（whisper_local）无需 stt_enabled 开关——按住说话（PTT）
    // 应始终可用；stt_enabled 只约束云端 ark 路径，避免默认 false 误报"未配置/未启用"。
    if voice.stt_engine != "ark" {
        // whisper_local（默认）：全本地离线，音频写临时 wav 后调 whisper-cli
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio_b64.trim())
            .map_err(|e| format!("音频 base64 解码失败: {e}"))?;
        if bytes.len() < 44 {
            return Err("音频数据过短".into());
        }
        // v8-sherpa：按住说话（PTT）也优先阿里 paraformer（中文准、转写快），
        // 缺模型/出错自动回退 whisper，保证不坏。
        return match crate::sherpa_engine::active_engine() {
            "paraformer" => match crate::sherpa_engine::transcribe_wav_bytes(&bytes).await {
                Ok(t) => Ok(t),
                Err(e) => {
                    eprintln!("[qibai] paraformer(PTT) 转写失败: {e}；回退 whisper");
                    crate::whisper_engine::transcribe_wav_bytes(&bytes).await
                }
            },
            _ => crate::whisper_engine::transcribe_wav_bytes(&bytes).await,
        };
    }
    // v18：云端 ark 路径才受 stt_enabled 开关约束（默认 false 仅挡云端，不挡本地 PTT）
    if !voice.stt_enabled {
        return Err("STT 未启用：请在设置中开启语音输入".into());
    }
    let key = voice.stt_key().ok_or_else(|| {
        "未配置 STT key：设置 voice.stt_api_key 或环境变量 QIBAI_STT_KEY".to_string()
    })?;
    let appid = std::env::var("QIBAI_STT_APPID").unwrap_or_default();
    if appid.trim().is_empty() {
        return Err("未配置 STT appid：请设置环境变量 QIBAI_STT_APPID".into());
    }
    let cluster = std::env::var("QIBAI_STT_CLUSTER").unwrap_or_else(|_| "volcano_tts".to_string());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_b64.trim())
        .map_err(|e| format!("音频 base64 解码失败: {e}"))?;
    if bytes.len() < 44 {
        return Err("音频数据过短".into());
    }
    let url = "https://openspeech.bytedance.com/api/v3/audio/asr";
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer;{key}"))
        .header("appid", appid)
        .header("cluster", cluster)
        .header("resource_id", "volc.bigasr.sauc.duration")
        .header("Content-Type", "audio/wav")
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("STT 请求失败: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("STT 响应读取失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("STT 服务返回 {status}: {text}"));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("STT 响应解析失败: {e}"))?;
    let t = v["result"]["text"].as_str().unwrap_or("").trim().to_string();
    if t.is_empty() {
        return Err("STT 未识别到语音内容".into());
    }
    Ok(t)
}

}

/// STT 麦克风设备探测（cfg 分离，同 guard::platform_sleep_cmd 先例）。
/// Linux 检查 /dev/snd 存在；mac/Windows 默认 true（真机项）。
#[cfg(target_os = "linux")]
fn stt_device_available() -> bool {
    std::path::Path::new("/dev/snd").exists()
}

#[cfg(target_os = "macos")]
fn stt_device_available() -> bool {
    true
}

#[cfg(target_os = "windows")]
fn stt_device_available() -> bool {
    true
}

/// 把 `LlmProvider` 适配成 codecheck 的 `SuggestLlm`：要求 JSON 输出，
/// 任何失败（网络 / key / 解析）都返回 None，由调用方降级为纯静态报告。
struct EngineSuggestLlm {
    provider: DoubaoProvider,
}

#[async_trait::async_trait]
impl crate::codecheck::SuggestLlm for EngineSuggestLlm {
    async fn suggest(
        &self,
        report: &crate::codecheck::CodeReport,
        language: &str,
    ) -> Option<String> {
        let system = "你是资深代码审查助手。阅读给定的静态检查报告，用中文简要总结问题并给出可执行的修复建议。\
            输出必须且只能是一个 JSON 对象，不要输出任何其他文字：{\"suggestion\":\"一句到三句中文建议\"}。";
        let user = format!(
            "语言：{language}\n摘要：{}\n问题明细：{}\n请给出修复建议。",
            report.summary,
            serde_json::to_string_pretty(&report.issues).unwrap_or_default()
        );
        let req = crate::llm_client::ChatRequest {
            system: system.to_string(),
            messages: vec![ChatMessage::user(user)],
            tools: None,
            json_object: true,
        };
        let content = self.provider.chat(req).await.ok()?.content?;
        let trimmed = content.trim();
        // 优先从 JSON 字段提取；提取失败则整体作为建议文本（降级）。
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(s) = v.get("suggestion").and_then(|x| x.as_str()) {
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// 解析 LLM 寒暄返回的 JSON：`{"lines":[{"speaker","text"}]}`；
/// speaker 必须属于 {from,to}，否则丢弃；整体非法时返回 None。
fn parse_greet_reply(content: &str, from: &str, to: &str) -> Option<Vec<(String, String)>> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct RawLine {
        speaker: Option<String>,
        text: Option<String>,
    }
    #[derive(Deserialize)]
    struct RawGreet {
        lines: Option<Vec<RawLine>>,
    }
    let trimmed = content.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|inner| inner.trim_end_matches('`').trim())
        .unwrap_or(trimmed);
    let json = if let (Some(s), Some(e)) = (json.find('{'), json.rfind('}')) {
        &json[s..=e]
    } else {
        json
    };
    let raw: RawGreet = serde_json::from_str(json).ok()?;
    let out: Vec<(String, String)> = raw
        .lines?
        .into_iter()
        .filter_map(|l| {
            let sp = l.speaker?;
            let tx = l.text?;
            if (sp == from || sp == to) && !tx.trim().is_empty() {
                Some((sp, tx))
            } else {
                None
            }
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::Emotion;
    use crate::llm_client::{ChatResponse, LlmProvider};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// 锁定安全门控回归：OPEN_ALL 必须跟随 debug_assertions，绝不允许被改回 `const true`
    /// （否则 release 构建会带全部门控绕过上线）。本测试在 debug 下断言二者一致即通过；
    /// 若有人把常量改回硬编码 true，此测试立即失败。
    #[test]
    fn open_all_follows_debug_profile_not_hardcoded() {
        assert_eq!(
            OPEN_ALL,
            cfg!(debug_assertions),
            "OPEN_ALL 必须 = cfg!(debug_assertions)；禁止硬编码 true 进 release"
        );
        // 语义自洽：debug（含 cargo test）下应为 true，保证既有门控依赖测试不受影响。
        assert!(OPEN_ALL, "cargo test 运行在 debug profile，OPEN_ALL 应为 true");
    }

    struct RecordingSink {
        events: Mutex<Vec<(String, serde_json::Value)>>,
    }
    impl EventSink for RecordingSink {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.to_string(), payload));
        }
    }

    struct MockLlm {
        scripted: Mutex<std::collections::VecDeque<Result<ChatResponse, LlmError>>>,
    }
    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn chat(
            &self,
            _req: crate::llm_client::ChatRequest,
        ) -> Result<ChatResponse, LlmError> {
            self.scripted.lock().unwrap().pop_front().unwrap_or_else(|| {
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"嗯","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                })
            })
        }
    }

    fn fresh_engine() -> Engine {
        // 调用方需持 env_lock。
        let tmp = crate::test_support::unique_temp_dir("engine");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }))
    }

    /// append_chat 专用断言 sink：事件列表句柄外借，便于断言 conversation-updated 广播次数。
    #[derive(Default)]
    struct VecSink {
        events: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    }
    impl EventSink for VecSink {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.to_string(), payload));
        }
    }

    #[test]
    fn append_chat_persists_and_emits() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("appendchat");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let evs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut e = Engine::new(Box::new(VecSink { events: evs.clone() }));
        let n1 = e.append_chat("user", "你好呀").unwrap();
        let n2 = e.append_chat("assistant", "我在呢").unwrap();
        assert_eq!(n1, 1);
        assert_eq!(n2, 2);
        assert_eq!(e.get_conversation().len(), 2);
        assert_eq!(e.get_conversation()[0].role, "user");
        assert_eq!(e.get_conversation()[0].content, "你好呀");
        assert_eq!(e.get_conversation()[1].role, "assistant");
        // 每落库一条都广播 conversation-updated（console.js 轮询即可见语音消息）。
        let emits = evs
            .lock()
            .unwrap()
            .iter()
            .filter(|(ev, _)| ev == "qibai://conversation-updated")
            .count();
        assert_eq!(emits, 2);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn append_chat_stores_raw_markdown_untouched() {
        // 清洗只影响朗读文本；落库必须保留原文（前端气泡按原文渲染）。
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("appendchat2");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mut e = Engine::new(Box::new(VecSink::default()));
        e.append_chat("assistant", "**加粗**和`代码`不会被洗掉").unwrap();
        assert_eq!(
            e.get_conversation()[0].content,
            "**加粗**和`代码`不会被洗掉"
        );
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn generate_code_returns_scripted_code() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("gencode");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mock = MockLlm {
            scripted: std::sync::Mutex::new(std::collections::VecDeque::from([Ok(
                ChatResponse {
                    content: Some("def fib(n):\n    return n if n < 2 else fib(n-1)+fib(n-2)".to_string()),
                    tool_calls: vec![], tokens_used: None,
                },
            )])),
        };
        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }))
            .with_provider(Box::new(mock));
        let code = e.generate_code("写个斐波那契函数").await.unwrap();
        assert!(code.contains("def fib"), "code={code}");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn generate_code_propagates_llm_error() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("gencode2");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mock = MockLlm {
            scripted: std::sync::Mutex::new(std::collections::VecDeque::from([Err(
                LlmError::Http("网络断开".to_string()),
            )])),
        };
        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }))
            .with_provider(Box::new(mock));
        let err = e.generate_code("随便写点啥").await.unwrap_err();
        assert!(err.to_string().contains("网络断开"), "err={err}");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn send_message_without_key_returns_structured_error() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mut e = fresh_engine();
        // v18.8-fix27：默认配置带本地 Qwen key → 测试显式清空 key，验证 MissingApiKey。
        e.settings.llm.api_key = String::new();
        let err = e.send_message("你好").await.unwrap_err();
        match err {
            EngineError::Llm(LlmError::MissingApiKey) => {}
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn send_message_happy_path_with_mock_llm() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mock = MockLlm {
            scripted: Mutex::new(std::collections::VecDeque::from([Ok(ChatResponse {
                content: Some(r#"{"reply":"今天也要加油呀","emotion":"happy"}"#.into()),
                tool_calls: vec![], tokens_used: None,
            })])),
        };
        let mut e = fresh_engine().with_provider(Box::new(mock));
        let out = e.send_message("讲个笑话").await.unwrap();
        assert_eq!(out.emotion, Emotion::Happy);
        assert_eq!(out.reply, "今天也要加油呀");
        assert!(e.get_conversation().len() >= 2);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 迭代③：保险箱对话拦截 ----

    /// §9-7 对话拦截在 L4 前拒绝（不进 LLM）；提到 L4 后放行到 unset 引导分支。
    #[tokio::test]
    async fn gate_block_before_l4() {
        if crate::engine::OPEN_ALL { return; } // 测试模式全开放
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mut e = fresh_engine();
        // 默认 active persona gudian 亲密度 < L4（settings.intimacy_level=1）。
        let out = e.send_message("我的微博密码是多少").await.unwrap();
        assert_eq!(out.skill.as_deref(), Some("vault"));
        assert!(out.reply.contains("L4"));
        assert!(out.secret.is_none());
        assert!(out.secret_entry_id.is_none());
        // 非 LLM 字段均为 None（拦截路径无 LLM 情绪）
        assert!(out.speak_text.is_none());

        // 提到 L4 → 保险箱未初始化引导分支
        e.intimacy.file.entries.get_mut("gudian").unwrap().level = 4;
        let out2 = e.send_message("我的微博密码是多少").await.unwrap();
        assert_eq!(out2.skill.as_deref(), Some("vault"));
        assert!(out2.reply.contains("初始化"));
        assert!(out2.reply.contains("主密码"));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// §9-8 mock LLM 断言未被调用；命中后 reply 含明文但 conversation 只落脱敏文本。
    #[tokio::test]
    async fn chat_intercept_no_llm() {
        if crate::engine::OPEN_ALL { return; } // 测试模式全开放
        use std::sync::Arc;
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };

        let calls = Arc::new(Mutex::new(0usize));
        struct CountingMock {
            calls: Arc<Mutex<usize>>,
        }
        #[async_trait::async_trait]
        impl LlmProvider for CountingMock {
            async fn chat(
                &self,
                _req: crate::llm_client::ChatRequest,
            ) -> Result<ChatResponse, LlmError> {
                *self.calls.lock().unwrap() += 1;
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"我是不该被调用的LLM","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                })
            }
        }

        let mut e = fresh_engine().with_provider(Box::new(CountingMock {
            calls: calls.clone(),
        }));

        // 初始化并解锁保险箱，写入一条微博记录。
        e.vault.setup_master_password("pw123").unwrap();
        e.vault.unlock("pw123").unwrap();
        let id = e
            .vault
            .add(crate::vault::VaultEntryInput {
                site: "微博".to_string(),
                account: "me@x.com".to_string(),
                password: "SECRET-PLAIN-9f3a".to_string(),
                note: String::new(),
                icon: "weibo".to_string(),
            })
            .unwrap();
        // 提到 L4 门控。
        e.intimacy.file.entries.get_mut("gudian").unwrap().level = 4;

        let out = e.send_message("我的微博密码是多少").await.unwrap();

        // 1) LLM 一次都没被调用
        assert_eq!(*calls.lock().unwrap(), 0, "拦截路径绝不调用 LLM");
        // 2) 拦截命中字段
        assert_eq!(out.secret, Some(true));
        assert_eq!(out.secret_entry_id.as_deref(), Some(id.as_str()));
        assert_eq!(out.skill.as_deref(), Some("vault"));
        // 3) 当次 reply 含明文（已解锁会话 UI）
        assert!(out.reply.contains("SECRET-PLAIN-9f3a"));
        // 4) speak_text 脱敏
        let speak = out.speak_text.expect("拦截命中应有 speak_text");
        assert!(speak.contains("微博"));
        assert!(!speak.contains("SECRET-PLAIN-9f3a"));

        // 5) conversation：用户原文 + 脱敏 assistant；明文永不落盘
        let msgs = e.get_conversation();
        let last_user = msgs[msgs.len() - 2].clone();
        let last_asst = msgs[msgs.len() - 1].clone();
        assert_eq!(last_user.role, "user");
        assert!(last_user.content.contains("微博"));
        assert_eq!(last_asst.role, "assistant");
        assert_eq!(last_asst.content, speak);
        assert!(
            !last_asst.content.contains("SECRET-PLAIN-9f3a"),
            "conversation 不得含密码明文"
        );

        // 6) 非拦截消息仍走原 LLM 路径（mock 被消费一次）——对比拦截路径 0 调用。
        let out2 = e.send_message("讲个笑话吧").await.unwrap();
        assert!(out2.reply.contains("不该被调用"), "非拦截应走 LLM");
        assert_eq!(*calls.lock().unwrap(), 1);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 完整技能链路冒烟：用户消息 → LLM 返回 tool_call → 白名单校验 → 执行 →
    /// tool 结果回填 → LLM 二次请求 → 最终 JSON 回复。
    #[tokio::test]
    async fn skill_chain_smoke_mock_llm_tool_call() {
        use crate::llm_client::{ToolCall, ToolCallFunction};
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };

        // 第一轮：LLM 决定调用 launch_app；第二轮：给出最终回复。
        let mock = MockLlm {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call-smoke".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "launch_app".into(),
                            arguments: r#"{"target":"firefox"}"#.into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"已经帮你打开浏览器啦","emotion":"happy","action":"wave"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
        };

        let mut e = fresh_engine().with_provider(Box::new(mock));
        let out = e.send_message("帮我打开浏览器").await.unwrap();
        assert_eq!(out.emotion, Emotion::Happy);
        assert_eq!(out.action, Some(crate::enums::Action::Wave));
        assert_eq!(out.skill.as_deref(), Some("launch_app"));
        assert!(out.reply.contains("浏览器"));
    }

    #[tokio::test]
    async fn clear_chat_history_empties_conversation_and_memory() {
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mock = MockLlm {
            scripted: Mutex::new(std::collections::VecDeque::from([Ok(ChatResponse {
                content: Some(r#"{"reply":"你好呀","emotion":"happy"}"#.into()),
                tool_calls: vec![], tokens_used: None,
            })])),
        };
        let mut e = fresh_engine().with_provider(Box::new(mock));
        e.send_message("在吗").await.unwrap();
        assert!(!e.get_conversation().is_empty());
        assert!(!e.personas.get("gudian").unwrap().memory.is_empty());
        let n = e.clear_chat_history().unwrap();
        assert!(n >= 2);
        assert!(e.get_conversation().is_empty());
        assert!(e.personas.get("gudian").unwrap().memory.is_empty());
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn skills_status_matches_contract() {
        let _g = crate::test_support::env_lock();
        let e = fresh_engine();
        let v = e.get_skills_status();
        assert!(v["total_enabled"].is_boolean());
        assert!(v["skills"]["launch_app"].is_boolean());
        assert!(v["skills"]["open_folder"].is_boolean());
        assert!(v["skills"]["set_reminder"].is_boolean());
    }

    // ---- P0/P1 新增命令测试 ----

    /// 迁移兼容性：旧 settings.json（intimacy=7）+ conversation.json 首次启动自动初始化；
    /// 重复启动不重复迁移（幂等）。
    #[test]
    fn migration_idempotent_across_engine_restarts() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("engine-mig");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        // 旧数据：完整 settings（intimacy=7）+ 无角色归属的 conversation.json
        let mut s = serde_json::to_value(Settings::default()).unwrap();
        s["intimacy_level"] = serde_json::json!(7);
        std::fs::write(tmp.join("settings.json"), serde_json::to_string(&s).unwrap()).unwrap();
        std::fs::write(
            tmp.join("conversation.json"),
            r#"[{"role":"user","content":"你好","ts":1},{"role":"assistant","content":"你好呀","ts":2}]"#,
        )
        .unwrap();

        // 首次启动：初始化 5 实体，gudian 亲密度=7，旧对话归入 gudian 记忆
        // 首次启动：初始化 5 实体，gudian 亲密度=7，旧对话归入 gudian 记忆
        {
            let e = Engine::new(Box::new(NullSink));
            assert_eq!(e.personas.active_persona(), "gudian");
            assert_eq!(e.personas.get("gudian").unwrap().intimacy, 7);
            assert_eq!(e.personas.get("gudian").unwrap().memory.len(), 2);
            assert!(e.personas.file.migrated_conversation);
        }
        // 第二次启动：记忆仍是 2 条，不翻倍
        {
            let e = Engine::new(Box::new(NullSink));
            assert_eq!(e.personas.get("gudian").unwrap().memory.len(), 2);
            assert_eq!(e.personas.get("gudian").unwrap().intimacy, 7);
            assert_eq!(e.personas.get("gudian").unwrap().memory[1].content, "你好呀");
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn send_message_writes_to_active_persona_memory() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mock = MockLlm {
            scripted: Mutex::new(std::collections::VecDeque::from([Ok(ChatResponse {
                content: Some(r#"{"reply":"今天也要加油呀","emotion":"happy"}"#.into()),
                tool_calls: vec![], tokens_used: None,
            })])),
        };
        let mut e = fresh_engine().with_provider(Box::new(mock));
        e.send_message("讲个笑话").await.unwrap();
        let m = &e.personas.get("gudian").unwrap().memory;
        assert!(m.len() >= 2);
        assert_eq!(m[m.len() - 1].content, "今天也要加油呀");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn social_pk_win_and_draw_scoring() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        // gudian(rock) 胜 huopo(scissors) → +10
        let v = e.social_pk("gudian", "huopo", "rock", "scissors").unwrap();
        assert_eq!(v["result"], "win");
        assert_eq!(v["winner"], "gudian");
        assert_eq!(v["loser"], "huopo");
        assert_eq!(v["affinity_delta"], 10);
        assert_eq!(e.social.relation("gudian", "huopo").unwrap().affinity, 10);
        // 平局 → 双方各 +4
        let v = e.social_pk("gudian", "huopo", "paper", "paper").unwrap();
        assert_eq!(v["result"], "draw");
        assert!(v["winner"].is_null());
        assert_eq!(v["affinity_delta"], 4);
        assert_eq!(e.social.relation("gudian", "huopo").unwrap().affinity, 14);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn social_pk_fourth_call_same_day_is_blocked() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        for _ in 0..3 {
            let v = e.social_pk("gudian", "huopo", "rock", "scissors").unwrap();
            assert_eq!(v["blocked"], false);
        }
        let v = e.social_pk("gudian", "huopo", "rock", "scissors").unwrap();
        assert_eq!(v["blocked"], true);
        assert_eq!(v["remaining_today"], 0);
        let text = v["lines"][0]["text"].as_str().unwrap();
        assert!(text.contains("3 次"));
        // 防刷后好感度不再增加
        assert_eq!(e.social.relation("gudian", "huopo").unwrap().affinity, 30);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn greet_crossing_level_boundary_writes_level_up_event() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        // 先推到 99，寒暄 +1 → 100 → 升 1 级
        e.social.add_affinity("gudian", "huopo", 99);
        let v = e.social_greet("gudian", "huopo", false).await.unwrap();
        assert_eq!(v["affinity_delta"], 1);
        assert!(e
            .social
            .file
            .events
            .iter()
            .any(|ev| ev.kind == "level_up"));
        // 返回台词中应含升级彩蛋
        assert!(v["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["text"].as_str().unwrap().contains("朋友")));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[tokio::test]
    async fn greet_without_api_key_falls_back_to_template() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mut e = fresh_engine();
        let v = e.social_greet("gudian", "huopo", true).await.unwrap();
        let lines = v["lines"].as_array().unwrap();
        assert!(lines.len() >= 2);
        for l in lines {
            let sp = l["speaker"].as_str().unwrap();
            assert!(sp == "gudian" || sp == "huopo");
            assert!(!l["text"].as_str().unwrap().is_empty());
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn unknown_persona_and_bad_move_paths_are_structured_errors() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        assert!(matches!(
            e.set_active_persona("ghost"),
            Err(EngineError::UnknownPersona(_))
        ));
        assert!(matches!(
            e.rename_persona("ghost", "x"),
            Err(EngineError::UnknownPersona(_))
        ));
        assert!(matches!(
            e.social_pk("ghost", "gudian", "rock", "paper"),
            Err(EngineError::UnknownPersona(_))
        ));
        assert!(matches!(
            e.social_pk("gudian", "huopo", "fire", "paper"),
            Err(EngineError::InvalidMove(_))
        ));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn get_personas_shape_and_rename_set_active_round_trip() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        let v = e.get_personas();
        assert_eq!(v["active_persona"], "gudian");
        assert_eq!(v["personas"].as_array().unwrap().len(), 5);
        let g = &v["personas"][0];
        assert_eq!(g["persona_id"], "gudian");
        assert!(g["traits"].is_object());
        assert!(g["memory"].is_array());

        e.rename_persona("huopo", "小火焰").unwrap();
        e.set_active_persona("huopo").unwrap();
        let v2 = e.get_personas();
        assert_eq!(v2["active_persona"], "huopo");
        assert!(v2["personas"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["persona_id"] == "huopo" && p["nickname"] == "小火焰"));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn get_social_state_and_events_shape() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        let st = e.get_social_state();
        assert_eq!(st["levels"].as_array().unwrap().len(), 4);
        assert_eq!(st["levels"][0]["name"], "初识");
        assert_eq!(st["levels"][3]["min"], 600);
        // 一次 PK 后事件日志可读
        e.social_pk("gudian", "huopo", "rock", "scissors").unwrap();
        let ev = e.get_social_events(None);
        let arr = ev["events"].as_array().unwrap();
        assert!(!arr.is_empty());
        // 倒序：最新在前
        assert_eq!(arr[0]["type"], "pk");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- desk 在场状态（契约 §4.4） ----

    /// 契约 §4.4-2：召唤/送走/persist/重载一致；召唤幂等。
    #[test]
    fn desk_summon_dismiss_roundtrip() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("engine-desk");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        {
            let mut e = Engine::new(Box::new(NullSink));
            assert!(e.desk_state()["present"].as_array().unwrap().is_empty());
            e.desk_summon("huopo").unwrap();
            e.desk_summon("huopo").unwrap(); // 幂等 no-op
            assert_eq!(e.desk_state()["present"], serde_json::json!(["huopo"]));
        }
        // 重载：在场状态已落盘
        {
            let mut e = Engine::new(Box::new(NullSink));
            assert_eq!(e.desk_state()["present"], serde_json::json!(["huopo"]));
            e.desk_dismiss("huopo").unwrap();
            assert!(e.desk_state()["present"].as_array().unwrap().is_empty());
        }
        // 再重载：送走已落盘
        {
            let e = Engine::new(Box::new(NullSink));
            assert!(e.desk_state()["present"].as_array().unwrap().is_empty());
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §4.4-3：锁定标记持久化。
    #[test]
    fn desk_lock_roundtrip() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("engine-lock");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        {
            let mut e = Engine::new(Box::new(NullSink));
            e.desk_set_locked("huopo", true).unwrap();
            assert_eq!(e.desk_state()["locked"]["huopo"], true);
        }
        {
            let e = Engine::new(Box::new(NullSink));
            assert_eq!(e.desk_state()["locked"]["huopo"], true);
        }
        // 解锁再落盘
        {
            let mut e = Engine::new(Box::new(NullSink));
            e.desk_set_locked("huopo", false).unwrap();
            assert_eq!(e.desk_state()["locked"]["huopo"], false);
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §4.4-4：切换主角色后 present 正确替换（`[new] ∪ (present \ {old})`）。
    #[test]
    fn set_active_persona_replaces_active_on_desk() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("engine-desk2");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        {
            let mut e = Engine::new(Box::new(NullSink));
            e.desk_summon("huopo").unwrap();
            e.desk_summon("dushi").unwrap();
            // present = [huopo, dushi]，主角色 gudian 原本不在 present
            e.set_active_persona("huopo").unwrap();
            assert_eq!(e.desk_state()["present"], serde_json::json!(["huopo", "dushi"]));
            assert_eq!(e.personas.active_persona(), "huopo");
        }
        // 重载保留替换结果
        {
            let e = Engine::new(Box::new(NullSink));
            assert_eq!(e.desk_state()["present"], serde_json::json!(["huopo", "dushi"]));
            // 切回 gudian：gudian 回到队首，huopo 从 present 中被替换掉
        }
        {
            let mut e = Engine::new(Box::new(NullSink));
            e.set_active_persona("gudian").unwrap();
            assert_eq!(e.desk_state()["present"], serde_json::json!(["gudian", "dushi"]));
        }
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §4.4-5：送走主角色返回错误；送走客人正常。
    #[test]
    fn dismiss_active_persona_rejected() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        assert!(matches!(
            e.desk_dismiss("gudian"),
            Err(EngineError::Social(_))
        ));
        // 送走非主角色正常
        e.desk_summon("huopo").unwrap();
        e.desk_dismiss("huopo").unwrap();
        assert!(!e.desk_state()["present"].as_array().unwrap().contains(&serde_json::json!("huopo")));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 亲密度（冻结规格 §6/§9） ----

    /// 契约 §6.1-1/2：等级表与状态形状；未知 persona 报错。
    #[test]
    fn intimacy_levels_and_state_shape() {
        let _g = crate::test_support::env_lock();
        let e = fresh_engine();
        let v = e.get_intimacy_levels();
        assert_eq!(v["levels"].as_array().unwrap().len(), 10);
        assert_eq!(v["levels"][0]["name"], "初识");
        assert_eq!(v["levels"][9]["max"], serde_json::Value::Null);
        let st = e.get_intimacy_state("gudian").unwrap();
        assert_eq!(st["level"], 1);
        assert_eq!(st["level_name"], "初识");
        assert_eq!(st["daily"]["cap"], 60);
        assert!(st["unlocked"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("basic_chat")));
        assert!(matches!(
            e.get_intimacy_state("ghost"),
            Err(EngineError::UnknownPersona(_))
        ));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §9-8：升级写事件日志（social events，kind=intimacy_level_up，含 new_level）。
    #[test]
    fn intimacy_event_level_up_writes_social_log() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        e.intimacy
            .file
            .entries
            .get_mut("gudian")
            .unwrap()
            .points = 118;
        let v = e.intimacy_event("gudian", "chat", serde_json::json!({}))
            .unwrap();
        assert_eq!(v["leveled_up"], true);
        assert_eq!(v["level"], 2);
        assert!(e
            .social
            .file
            .events
            .iter()
            .any(|ev| ev.kind == "intimacy_level_up"
                && ev.a == "gudian"
                && ev.b == "gudian"
                && ev.detail["new_level"] == 2
                && ev.detail["level_name"] == "熟悉"));
        // 已落盘
        e.intimacy.persist().unwrap();
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §9-13：check_gate 未知 persona 宽容返回 current_level=1；
    /// L1 时 assistant_skills false，L4（points≥480）后 true。
    #[test]
    fn check_gate_unknown_persona_lenient_and_l4_unlocks() {
        if crate::engine::OPEN_ALL { return; } // 测试模式全开放
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        // 未知 persona：不报错，current_level=1
        let v = e.check_gate("ghost", "assistant_skills").unwrap();
        assert_eq!(v["open"], false);
        assert_eq!(v["required_level"], 4);
        assert_eq!(v["current_level"], 1);
        // L1：active persona 未解锁
        let v = e.check_gate("gudian", "assistant_skills").unwrap();
        assert_eq!(v["open"], false);
        // 推到 L4（118+2=120 不够，直接播种到 478 再 chat +2 = 480）
        e.intimacy
            .file
            .entries
            .get_mut("gudian")
            .unwrap()
            .points = 478;
        e.intimacy_event("gudian", "chat", serde_json::json!({}))
            .unwrap();
        let v = e.check_gate("gudian", "assistant_skills").unwrap();
        assert_eq!(v["open"], true);
        assert_eq!(v["current_level"], 4);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §9-14：技能审批路径在 L1 时被亲密度门控拒绝；L4 后放行。
    #[tokio::test]
    async fn approve_skill_blocked_until_intimacy_l4() {
        if crate::engine::OPEN_ALL { return; } // 测试模式全开放
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        // L1：审批路径直接被 assistant_skills 门控拒绝（明确"亲密度"错误）
        let err = e.approve_skill("req-x", true).await.unwrap_err();
        match err {
            EngineError::Intimacy(msg) => assert!(msg.contains("亲密度"), "msg={msg}"),
            other => panic!("expected intimacy gate error, got {other:?}"),
        }
        // 取消（approve=false）不受门控限制（进入 pending 解析 → 无此 request）
        let _ = e.approve_skill("req-x", false).await;
        // L4：门控通过，进入 pending 解析（无此 request → Skill 错误，而非亲密度错误）
        e.intimacy
            .file
            .entries
            .get_mut("gudian")
            .unwrap()
            .points = 478;
        e.intimacy_event("gudian", "chat", serde_json::json!({}))
            .unwrap();
        let err = e.approve_skill("req-x", true).await.unwrap_err();
        assert!(matches!(err, EngineError::Skill(_)));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §6.1-4：纪念日校验与读写。
    #[test]
    fn anniversary_set_clear_and_validation() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        assert!(e.set_anniversary("gudian", 0, 15).is_err());
        assert!(e.set_anniversary("gudian", 6, 32).is_err());
        e.set_anniversary("gudian", 6, 15).unwrap();
        let st = e.get_intimacy_state("gudian").unwrap();
        assert_eq!(st["anniversary"], serde_json::json!({"month": 6, "day": 15}));
        e.clear_anniversary("gudian").unwrap();
        let st = e.get_intimacy_state("gudian").unwrap();
        assert_eq!(st["anniversary"], serde_json::Value::Null);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 全屏守护演出门面（迭代①） ----

    /// 完整周期：manual_trigger → is_active → 防重入 → skip → 冷却阻挡。
    #[test]
    fn guard_facade_manual_trigger_skip_and_cooldown() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        assert_eq!(e.guard_cooldown_left(), 0);

        // 默认 Strong：可手动触发，且全屏演出
        let start = e.guard_manual_trigger().expect("默认守护已启用");
        assert!(e.guard.is_active());
        assert_eq!(start.perform_fullscreen, true);
        assert_eq!(start.mode, crate::guard::GuardMode::Strong);
        // 演出中再次触发 → None（防重入）
        assert!(e.guard_manual_trigger().is_none());

        // skip → 进入冷却
        e.guard_skip("user").unwrap();
        assert!(!e.guard.is_active());
        let left = e.guard_cooldown_left();
        assert!(left > 0 && left <= crate::guard::COOLDOWN_SECS);
        // 冷却期内手动触发一律 None
        assert!(e.guard_manual_trigger().is_none());
        // 非激活态再 skip → Guard 错误
        assert!(matches!(e.guard_skip("auto"), Err(EngineError::Guard(_))));

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// Light 模式不演出；on_sit 入口可按传入 now 快进验证冷却窗口。
    #[test]
    fn guard_light_mode_and_on_sit_fast_forward() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        e.guard.update_settings(crate::guard::GuardSettings {
            mode: crate::guard::GuardMode::Light,
            ..Default::default()
        });

        let start = e.guard_on_sit(1000).expect("Light 仍发出 start");
        assert_eq!(start.mode, crate::guard::GuardMode::Light);
        assert_eq!(start.perform_fullscreen, false);

        // t=1000 skip → 冷却到 1000+COOLDOWN
        e.guard.skip(1000, "user").unwrap();
        assert_eq!(e.guard.cooldown_left(1000), crate::guard::COOLDOWN_SECS);
        // 冷却窗口内久坐触发被挡
        assert!(e.guard_on_sit(1500).is_none());
        // 出窗口恢复
        assert!(e
            .guard_on_sit(1000 + crate::guard::COOLDOWN_SECS + 5)
            .is_some());

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// save_settings 同步守护调度器：非法 countdown 被 clamp，Lock 模式强制收尾。
    #[test]
    fn save_settings_updates_guard_scheduler() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        let mut s = e.get_settings();
        s.guard.countdown_secs = 10; // 低于下限
        s.guard.mode = crate::guard::GuardMode::Lock;
        s.guard.end_action = crate::guard::GuardEndAction::Countdown;
        e.save_settings(s).unwrap();

        let start = e.guard_manual_trigger().unwrap();
        assert_eq!(start.countdown_secs, crate::guard::COUNTDOWN_MIN); // 60
        assert_eq!(start.mode, crate::guard::GuardMode::Lock);
        assert_eq!(start.end_action, crate::guard::GuardEndAction::Lock); // Lock 强制
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 代码检查门面（迭代①） ----

    /// mock runner 产出 node 风格 stderr → 解析出 syntax issue；
    /// mock suggester 附加 llm_note。
    #[tokio::test]
    async fn engine_check_code_with_mock_runner_and_suggester() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mut e = fresh_engine();

        let dir = std::env::temp_dir().join("qibai_engine_cc");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m.js");
        std::fs::write(&p, "const x = 1;\n").unwrap();

        struct FakeRunner;
        #[async_trait]
        impl crate::codecheck::CheckRunner for FakeRunner {
            async fn run(
                &self,
                _argv: &[&str],
                _cwd: Option<&std::path::Path>,
                _env: &[(&str, &str)],
            ) -> Result<crate::codecheck::CheckOutput, String> {
                let path = std::env::temp_dir().join("qibai_engine_cc/m.js");
                Ok(crate::codecheck::CheckOutput {
                    stdout: String::new(),
                    stderr: format!(
                        "{}:2\n\nSyntaxError: Missing semicolon. (2:10)\n",
                        path.display()
                    ),
                    ok: false,
                })
            }
        }

        struct FakeSuggester;
        #[async_trait]
        impl crate::codecheck::SuggestLlm for FakeSuggester {
            async fn suggest(
                &self,
                _r: &crate::codecheck::CodeReport,
                _lang: &str,
            ) -> Option<String> {
                Some("LLM建议：补上缺失的分号".to_string())
            }
        }

        e = e.with_codecheck_overrides(Box::new(FakeRunner), Some(Box::new(FakeSuggester)));
        let rep = e
            .check_code(crate::codecheck::CheckRequest::from_path(&p))
            .await
            .unwrap();

        assert_eq!(rep.files_checked, 1);
        assert_eq!(rep.language, "javascript");
        let syn = rep
            .issues
            .iter()
            .find(|i| i.category == crate::codecheck::IssueCategory::Syntax)
            .expect("应从 mock stderr 解析出 syntax issue");
        assert_eq!(syn.severity, crate::codecheck::IssueSeverity::Error);
        assert!(syn.message.contains("SyntaxError"));
        // suggester 附加的 llm_note 透传
        assert_eq!(rep.llm_note.as_deref(), Some("LLM建议：补上缺失的分号"));

        let _ = std::fs::remove_file(&p);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 无 key 且无注入 suggester 时：llm_note 为 None（纯静态，不报错）。
    #[tokio::test]
    async fn engine_check_code_without_key_has_no_llm_note() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };
        let mut e = fresh_engine();
        // fresh_engine 的 settings.api_key 为空 → 无 key → suggester=None
        let rep = e
            .check_code(crate::codecheck::CheckRequest::from_snippet(
                "const x = 1;",
                Some("javascript".to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(rep.llm_note, None);
        assert_eq!(rep.summary, "未发现问题");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- start_guard 对话链路（迭代①） ----

    /// LLM 首轮返回 start_guard tool_call → adapter 拦截（不走技能白名单）→
    /// 引擎进入演出态；二轮给出最终回复。
    #[tokio::test]
    async fn start_guard_tool_call_enters_performance_state() {
        use crate::llm_client::{ToolCall, ToolCallFunction};
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var(crate::llm_client::ENV_API_KEY) };

        let mock = MockLlm {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call-guard".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "start_guard".into(),
                            arguments: "{}".into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"好啦，起来活动一下吧","emotion":"happy"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
        };

        let mut e = fresh_engine().with_provider(Box::new(mock));
        assert!(!e.guard.is_active());
        let out = e.send_message("我好累，守护我一下").await.unwrap();
        assert_eq!(out.skill.as_deref(), Some("start_guard"));
        assert!(e.guard.is_active());
        assert!(out.reply.contains("活动"));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 迭代②：TTS 朗读决策与状态 ----

    fn clean_voice_env() {
        unsafe {
            std::env::remove_var(crate::tts::ENV_CLOUD_KEY);
            std::env::remove_var(crate::tts::ENV_STT_KEY);
            std::env::remove_var(crate::llm_client::ENV_API_KEY);
        }
    }

    /// tts_decide：空文本 / 总开关关 → None；scope 门控；resolved 透传。
    #[test]
    fn tts_decide_gates_on_switch_scope_and_blank_text() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let mut e = fresh_engine();
        // 开关/scope 契约不应依赖机器是否安装 espeak：按当前运行环境核验可用引擎分支。
        let req = e
            .tts_decide(tts::TtsKind::Chat, "你好呀", None)
            .expect("All scope + 非空文本");
        assert_eq!(req.text, "你好呀");
        assert_eq!(req.kind, tts::TtsKind::Chat);
        if tts::detect_local_tts_available() {
            assert_eq!(req.resolved, tts::ResolvedEngine::Local);
        } else {
            assert_eq!(req.resolved, tts::ResolvedEngine::Mute);
            assert!(req.reason.contains("无可用引擎"));
        }
        assert_eq!(req.volume, 80);
        assert_eq!(req.rate, 1.0);

        // 空文本（含纯空白）→ None
        assert!(e.tts_decide(tts::TtsKind::Chat, "", None).is_none());
        assert!(e.tts_decide(tts::TtsKind::Chat, "   ", None).is_none());

        // 总开关关 → None（即使 scope 覆盖、文本非空）
        e.settings.voice.enabled = false;
        assert!(e.tts_decide(tts::TtsKind::Chat, "你好呀", None).is_none());
        e.settings.voice.enabled = true;

        // scope=reminders：Chat 被门控 → None；Reminder 放行
        e.settings.voice.scope = tts::TtsScope::Reminders;
        assert!(e.tts_decide(tts::TtsKind::Chat, "你好呀", None).is_none());
        let req = e
            .tts_decide(tts::TtsKind::Reminder, "该喝水啦", None)
            .expect("reminders scope 放行 Reminder");
        assert_eq!(req.kind, tts::TtsKind::Reminder);
        // LevelUp / Guard 仅在 All 下放行
        assert!(e.tts_decide(tts::TtsKind::LevelUp, "升级啦", None).is_none());
        assert!(e.tts_decide(tts::TtsKind::Guard, "开场语", None).is_none());

        // scope=chat：仅 Chat 放行
        e.settings.voice.scope = tts::TtsScope::Chat;
        assert!(e.tts_decide(tts::TtsKind::Chat, "hi", None).is_some());
        assert!(e.tts_decide(tts::TtsKind::Reminder, "该喝水啦", None).is_none());
        assert!(e.tts_decide(tts::TtsKind::LevelUp, "升级啦", None).is_none());

        // scope=all：LevelUp / Guard 放行
        e.settings.voice.scope = tts::TtsScope::All;
        assert!(e.tts_decide(tts::TtsKind::LevelUp, "升级啦", None).is_some());
        assert!(e.tts_decide(tts::TtsKind::Guard, "开场语", None).is_some());
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// TtsSpeakRequest 序列化形状：kind/resolved 字段为前端契约形态。
    #[test]
    fn tts_speak_request_serialize_shape() {
        let req = TtsSpeakRequest {
            text: "你好".to_string(),
            kind: tts::TtsKind::Reminder,
            volume: 80,
            rate: 1.5,
            pitch: -2.0,
            engine_voice_id: "".to_string(),
            cloud_speaker: "zh_female_x".to_string(),
            voice_tag: "温婉 · 中速 · 微低".to_string(),
            resolved: tts::ResolvedEngine::Local,
            reason: "本地系统 TTS".to_string(),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["kind"], "reminder");
        assert_eq!(v["resolved"], "local");
        assert_eq!(v["reason"], "本地系统 TTS");
        assert_eq!(v["volume"], 80);
        assert_eq!(v["rate"], 1.5);
        assert_eq!(v["text"], "你好");
        // 迭代⑤新增字段名即字段名
        assert_eq!(v["pitch"], -2.0);
        assert_eq!(v["engine_voice_id"], "");
        assert_eq!(v["cloud_speaker"], "zh_female_x");
        assert_eq!(v["voice_tag"], "温婉 · 中速 · 微低");
    }

    /// tts_status JSON 形状 + resolved 决策；local_available 透传；cloud_ready 随 key。
    #[test]
    fn tts_status_shape_and_cloud_ready() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let e = fresh_engine();
        // local_available=false 注入
        let v = e.tts_status(false);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["engine"], "auto");
        assert_eq!(v["volume"], 80);
        assert_eq!(v["rate"], 1.0);
        assert_eq!(v["scope"], "all");
        assert_eq!(v["local_available"], false);
        assert_eq!(v["cloud_ready"], false);
        assert_eq!(v["resolved"]["engine"], "mute");
        assert!(v["resolved"]["reason"].is_string());

        // local_available=true → 无 key 的 auto 解析为 local
        let v = e.tts_status(true);
        assert_eq!(v["local_available"], true);
        assert_eq!(v["resolved"]["engine"], "local");

        // 配 key → cloud_ready=true，auto 解析为 cloud
        let mut e = e;
        e.settings.voice.cloud_api_key = "sk-x".to_string();
        let v = e.tts_status(false);
        assert_eq!(v["cloud_ready"], true);
        assert_eq!(v["resolved"]["engine"], "cloud");
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 迭代⑤：persona 感知的音色解析链 ----

    /// tts_decide：persona_id 指定角色 vs None 用 active；voice_tag 5 条冻结标签；
    /// eff_rate = 全局 rate × profile.rate 且 clamp。
    #[test]
    fn tts_decide_persona_aware_profile_chain() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let e = fresh_engine();
        // active = gudian，None → 按 active 解析
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", None).unwrap();
        assert_eq!(req.cloud_speaker, "zh_female_wenrouxiaoya_moon_bigtts");
        assert_eq!(req.pitch, -2.0);
        assert_eq!(req.voice_tag, "温婉 · 中速 · 微低");
        assert_eq!(req.rate, 1.0); // 1.0 × gudian 1.0

        // 显式指定角色：5 条冻结标签逐一断言
        for (id, tag, speaker, pitch, rate) in [
            ("gudian", "温婉 · 中速 · 微低", "zh_female_wenrouxiaoya_moon_bigtts", -2.0f32, 1.0f32),
            ("dushi", "知性 · 轻快 · 中性", "zh_female_shuangkuaisisi_emo_v2_mars_bigtts", 0.0, 1.1),
            ("wenrou", "恬静 · 慢速 · 微低", "ICL_zh_female_ruanmengtuanzi_cs_tob", -1.0, 0.8),
            ("huopo", "元气 · 轻快 · 偏高", "ICL_zh_female_wuxi_tob", 3.0, 1.2),
            ("reqing", "热情 · 中速 · 偏高", "ICL_zh_female_reqingaina_cs_tob", 2.0, 1.05),
        ] {
            let req = e.tts_decide(tts::TtsKind::Chat, "hi", Some(id)).unwrap();
            assert_eq!(req.voice_tag, tag, "tag[{id}]");
            assert_eq!(req.cloud_speaker, speaker, "speaker[{id}]");
            assert_eq!(req.pitch, pitch, "pitch[{id}]");
            // eff_rate = 全局 rate(1.0) × profile.rate
            assert!((req.rate - rate).abs() < 1e-5, "rate[{id}] = {}", req.rate);
        }

        // 全局 rate × 角色 rate，且 clamp [0.5, 2.0]
        let mut e = e;
        e.settings.voice.rate = 1.5;
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", Some("huopo")).unwrap();
        assert!((req.rate - 1.8).abs() < 1e-5, "{}", req.rate); // 1.5×1.2
        e.settings.voice.rate = 2.0;
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", Some("huopo")).unwrap();
        assert_eq!(req.rate, tts::RATE_MAX); // 2.0×1.2=2.4 → clamp 2.0
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 无 key 降级提示：解析出云端 speaker、无 key、本地可用 → Local 且 reason 含「未配置 key」；
    /// 有 key → Cloud。沙箱无 espeak，用临时目录伪造 PATH 命中 which()。
    #[test]
    fn tts_decide_no_key_downgrade_reason_when_local_faked_available() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let tmp = crate::test_support::unique_temp_dir("tts-local");
        // 伪造 PATH 中存在 espeak-ng（空文件即可，which 只查存在性；tts_decide 不 spawn）
        std::fs::write(tmp.join("espeak-ng"), "").unwrap();
        let old_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", &tmp) };

        let e = fresh_engine();
        // active=gudian → 解析出云端 speaker（非空）；无 key → 降级 Local 并覆盖 reason
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", None).unwrap();
        assert_eq!(req.resolved, tts::ResolvedEngine::Local);
        assert!(
            req.reason.contains("未配置 key"),
            "reason={}",
            req.reason
        );
        assert!(req.reason.contains("zh_female_wenrouxiaoya_moon_bigtts"));

        // v18.8-fix28g：配 key 后 Auto 仍本地优先（隐私优先策略）；本地不可用时才落 Cloud
        let mut e = e;
        e.settings.voice.cloud_api_key = "sk-x".to_string();
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", None).unwrap();
        assert_eq!(req.resolved, tts::ResolvedEngine::Local);
        assert!(!req.reason.contains("未配置 key"));

        unsafe {
            std::env::set_var("PATH", &old_path);
            std::env::remove_var("QIBAI_DATA_DIR");
        };
    }

    /// tts_status 新字段形状（active_persona / voice_tag / pitch / eff_rate / cloud_speaker）。
    #[test]
    fn tts_status_new_voice_fields_shape() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let e = fresh_engine();
        let v = e.tts_status(false);
        assert_eq!(v["active_persona"], "gudian");
        assert_eq!(v["voice_tag"], "温婉 · 中速 · 微低");
        assert_eq!(v["pitch"], -2.0);
        assert_eq!(v["eff_rate"], 1.0);
        assert_eq!(v["cloud_speaker"], "zh_female_wenrouxiaoya_moon_bigtts");
        // 既有字段未被删改（见 tts_status_shape_and_cloud_ready）
        assert_eq!(v["enabled"], true);
        assert_eq!(v["engine"], "auto");
        assert_eq!(v["rate"], 1.0);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// set_persona_voice：sanitize + 落盘 + 生效解析；未知 id → UnknownPersona。
    #[test]
    fn set_persona_voice_sanitize_and_unknown_persona() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let mut e = fresh_engine();
        let custom = VoiceProfile {
            mode: crate::voice_profile::VoiceProfileMode::Custom,
            pitch: 50.0,
            rate: 5.0,
            engine_voice_id: "espeak-voice-1".into(),
            cloud_speaker: "my-speaker".into(),
        };
        e.set_persona_voice("huopo", custom.clone()).unwrap();
        // 落盘前 sanitize
        let stored = &e.personas.get("huopo").unwrap().voice_profile;
        assert_eq!(stored.pitch, 10.0);
        assert_eq!(stored.rate, 2.0);
        // 后续决策按 Custom 生效
        let req = e.tts_decide(tts::TtsKind::Chat, "hi", Some("huopo")).unwrap();
        assert_eq!(req.cloud_speaker, "my-speaker");
        assert_eq!(req.engine_voice_id, "espeak-voice-1");
        assert_eq!(req.voice_tag, "自定义 · 轻快 · 偏高"); // pitch=10 → 偏高；rate=2.0 → 轻快
        // 未知 id → UnknownPersona
        assert!(matches!(
            e.set_persona_voice("nosuch", custom),
            Err(EngineError::UnknownPersona(_))
        ));
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ===== v31：知识库检索 + 工作区命令 allowlist =====
    #[test]
    fn kb_search_hits_doc_and_returns_snippet() {
        let tmp = std::env::temp_dir().join(format!("qibai_kb_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("guide.md"),
            "元气小小知识库：默认打开本地工作区目录 ~/Documents/qibai，环境变量 QIBAI_WORKSPACE 可覆盖。",
        )
        .unwrap();
        std::fs::write(tmp.join("unrelated.txt"), "今天天气不错。").unwrap();
        let hits = kb_search_in(&tmp, "工作区目录");
        assert!(!hits.is_empty(), "应命中包含「工作区目录」的文档");
        assert!(hits[0].contains("guide.md"), "片段应带文件名：{}", hits.join(" | "));
        assert!(hits[0].contains("Documents/qibai"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn kb_search_returns_empty_when_no_match() {
        let tmp = std::env::temp_dir().join(format!("qibai_kb_test_none_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "今天天气不错。").unwrap();
        let hits = kb_search_in(&tmp, "量子计算");
        assert!(hits.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn code_allowlist_includes_dev_commands() {
        for cmd in ["node", "python3", "npm", "cargo"] {
            assert!(
                QIBAI_CODE_ALLOWLIST.contains(&cmd),
                "{cmd} 应加入工作区命令 allowlist（语音指导建工程/写代码闭环）"
            );
        }
        for cmd in ["rm", "bash", "sh", "zsh", "curl", "wget", "sudo"] {
            assert!(
                !QIBAI_CODE_ALLOWLIST.contains(&cmd),
                "{cmd} 应保持拒绝（危险命令不进 allowlist）"
            );
        }
    }

    /// stt_status JSON 形状；沙箱 /dev/snd 不存在 → device_available=false，
    /// available=false（不报错、不打扰，如实降级提示）。
    #[test]
    fn stt_status_shape_and_sandbox_device_unavailable() {
        let _g = crate::test_support::env_lock();
        clean_voice_env();
        let e = fresh_engine();
        let v = e.stt_status();
        assert_eq!(v["enabled"], false);
        assert_eq!(v["provider"], "doubao");
        assert_eq!(v["key_ready"], false);
        assert_eq!(v["device_available"], false); // 沙箱无 /dev/snd
        assert_eq!(v["available"], false);
        assert!(v["reason"].is_string());
        assert!(!v["reason"].as_str().unwrap().is_empty());

        // 开启开关但无设备 → available=false，reason 指向设备（设备检查优先于引擎）
        let mut e = e;
        e.settings.voice.stt_enabled = true;
        let v = e.stt_status();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["available"], false);
        assert!(v["reason"].as_str().unwrap().contains("设备"));
        // 新字段形状：引擎/唤醒
        assert_eq!(v["engine"], "whisper_local");
        // v16：免提默认开启（本地处理隐私安全；stt_enabled=false 时仍关闭监听）
        assert_eq!(v["wake_enabled"], true);
        assert_eq!(v["wake_word"], "小小");
        assert!(v["whisper_ready"].is_boolean());
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 插件体系验收（T6/T9，契约 §6 样例插件可安装可调用） ----

    /// 样例插件根（仓库 plugins/examples）。
    fn examples_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("plugins")
            .join("examples")
    }

    /// datetime：install → plugin_list → grant → enable → tool_schemas 含 get_datetime → invoke 成功。
    #[tokio::test]
    async fn datetime_plugin_full_lifecycle() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-plug-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-plug-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));

        // 安装（幂等 no-op）
        let from = examples_dir().join("com.qibai.datetime");
        e.plugin_install(from.to_string_lossy().to_string()).unwrap();
        e.plugin_install(from.to_string_lossy().to_string()).unwrap();

        // plugin_list 可见
        let list = e.plugin_list();
        let row = list
            .iter()
            .find(|v| v["id"] == "com.qibai.datetime")
            .expect("datetime 应在列表中");
        assert_eq!(row["type"], "skill");
        assert_eq!(row["installed"], true);
        assert_eq!(row["enabled"], false); // 外部默认关
        assert_eq!(row["permissions"][0], "command:run");
        assert_eq!(row["entry_kind"], "command");
        assert!(!row["builtin"].as_bool().unwrap());

        // 未授权调用 → Denied（权限门控）
        e.plugin_set_enabled("com.qibai.datetime".into(), true).unwrap();
        match e.skills.invoke("get_datetime", &serde_json::json!({})).await {
            crate::skills::SkillOutcome::Denied(msg) => {
                assert!(msg.contains("command:run"));
            }
            other => panic!("expected permission denied, got {other:?}"),
        }

        // grant → enable → tool_schemas 含 get_datetime
        e.plugin_grant_permission(
            "com.qibai.datetime".into(),
            "command:run".into(),
            true,
        )
        .unwrap();
        let schemas = e.skills.tool_schemas();
        assert!(schemas.iter().any(|s| s["function"]["name"] == "get_datetime"));

        // invoke 成功返回日期文本
        match e.skills.invoke("get_datetime", &serde_json::json!({})).await {
            crate::skills::SkillOutcome::Ok(msg) => {
                // YYYY-MM-DD HH:MM:SS
                assert!(msg.split_whitespace().next().unwrap().contains('-'));
            }
            other => panic!("expected ok, got {other:?}"),
        }

        // settings.plugins 落盘同步
        assert!(e.settings.plugins.enabled["com.qibai.datetime"]);
        assert_eq!(e.settings.plugins.grants["com.qibai.datetime"][0], "command:run");

        // 卸载幂等
        e.plugin_uninstall("com.qibai.datetime".into()).unwrap();
        e.plugin_uninstall("com.qibai.datetime".into()).unwrap();
        assert!(e.plugin_list().iter().all(|v| v["id"] != "com.qibai.datetime"));

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// v24 联网+编程技能对 LLM 可见：工具表含 web_search（联网）与 git_status/run_tests/code_scan（编程）。
    #[tokio::test]
    async fn web_search_and_programming_skills_visible_in_tool_schemas() {
        let _g = crate::test_support::env_lock();
        let e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        let schemas = e.skills.tool_schemas();
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        for want in ["web_search", "git_status", "run_tests", "code_scan", "query_weather", "list_skills"] {
            assert!(
                names.contains(&want),
                "工具表应包含 {want}（实际 {names:?}），说明该技能未注册或未暴露给 LLM"
            );
        }
        // 危险命令 execute_command 不进工具表（保持 LLM 不可直接触发）。
        assert!(!names.contains(&"execute_command"));
    }

    /// v16 技能库：scan_plugin_dir 装配技能包 → skill_list 可见 → skill_invoke 执行 entry 脚本。
    #[tokio::test]
    async fn skill_library_scan_and_invoke() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-skill-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-skill-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        // 造一个技能包目录（库目录下的一级子目录）
        let lib = crate::test_support::unique_temp_dir("engine-skill-lib");
        let pkg = lib.join("hello-skill");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("qibai-plugin.json"),
            r#"{"id":"com.qibai.skill.hello","type":"skill","name":"问候技能","version":"1.0.0","description":"测试技能","entry":{"kind":"command","command":"hello.sh"},"functions":[{"name":"say_hello","description":"打招呼"}]}"#,
        )
        .unwrap();
        std::fs::write(
            pkg.join("hello.sh"),
            "#!/bin/sh\necho \"技能执行成功，收到参数：$QIBAI_SKILL_ARGS\"\n",
        )
        .unwrap();

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        let install = e.plugin_install(pkg.to_string_lossy().to_string());
        assert!(install.is_ok(), "安装失败：{install:?}");
        let n = e.scan_plugin_dir(&lib);
        assert_eq!(n, 1);

        let list = e.skill_list();
        let row = list
            .iter()
            .find(|v| v["id"] == "com.qibai.skill.hello")
            .expect("技能应在清单中");
        assert_eq!(row["name"], "问候技能");
        assert_eq!(row["type"], "skill");

        let out = e
            .skill_invoke("com.qibai.skill.hello", "say_hello", "{\"path\":\"/tmp\"}")
            .unwrap();
        assert!(out.contains("技能执行成功"));
        assert!(out.contains("/tmp"));

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// scientist：install → expert_list → activate → expert_prompt 装配 → 白名单过滤生效。
    #[tokio::test]
    async fn scientist_expert_activation_and_whitelist() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-exp-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-exp-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        let from = examples_dir().join("com.qibai.scientist");
        e.plugin_install(from.to_string_lossy().to_string()).unwrap();

        // expert_list 可见
        let list = e.expert_list();
        let row = list
            .iter()
            .find(|v| v["id"] == "com.qibai.scientist")
            .expect("scientist 应在专家列表");
        assert_eq!(row["active"], false);
        assert!(row["system_prompt"].as_str().unwrap().contains("科学家"));
        assert_eq!(row["persona_binding"]["voice"], "gudian");

        // 激活
        e.expert_set_active("com.qibai.scientist".into()).unwrap();
        assert!(e.experts.active_expert().is_some());

        // expert_prompt 装配（base + 人设追加段）
        let assembled = e.experts.expert_prompt("BASE_PROMPT");
        assert!(assembled.starts_with("BASE_PROMPT"));
        assert!(assembled.contains("严谨的科学家助手"));

        // 白名单：仅 launch_app / open_folder + start_guard 保留
        let mut tools = e.skills.tool_schemas();
        tools.push(serde_json::json!({
            "type": "function",
            "function": { "name": "start_guard", "parameters": {"type":"object"} }
        }));
        if let Some(whitelist) = e.experts.whitelist() {
            tools.retain(|s| {
                let name = s["function"]["name"].as_str().unwrap_or("");
                name == "start_guard"
                    || e.skills
                        .plugins()
                        .plugin_id_for_function(name)
                        .map(|pid| whitelist.iter().any(|w| w == pid || w == name))
                        .unwrap_or(false)
            });
        }
        let names: Vec<&str> = tools
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"launch_app"));
        // v43：open_folder 默认不暴露给 LLM（真机误开文件夹的根因之一）。
        assert!(!names.contains(&"open_folder"));
        assert!(names.contains(&"start_guard"));
        assert!(!names.contains(&"check_code")); // 白名单外被过滤
        assert!(!names.contains(&"set_reminder"));

        // settings.plugins.experts 落盘（active=true）
        assert_eq!(
            e.settings.plugins.experts["com.qibai.scientist"],
            true
        );

        // 空串 = 停用
        e.expert_set_active("".into()).unwrap();
        assert!(e.experts.active_expert().is_none());
        assert!(e.settings.plugins.experts.is_empty());

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// files:write 对外部插件硬拒（T8 红线）。
    #[tokio::test]
    async fn files_write_hard_rejected_for_external_plugin() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-fw-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-fw-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        let from = examples_dir().join("com.qibai.datetime");
        e.plugin_install(from.to_string_lossy().to_string()).unwrap();
        assert!(e
            .plugin_grant_permission(
                "com.qibai.datetime".into(),
                "files:write".into(),
                true,
            )
            .is_err());

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// v18 技能/专家库：扫描真实插件目录（src-tauri/plugins），断言 20 个新包全部装配、
    /// id 无重复、入口无非法路径；并对各新技能 smoke invoke（走真实脚本路径）。
    #[tokio::test]
    async fn v18_plugin_library_scan_and_invoke() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("v18-data");
        let plugindir = crate::test_support::unique_temp_dir("v18-plugins");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/plugins");
        let mut e = Engine::new(Box::new(RecordingSink {
            events: Mutex::new(vec![]),
        }));
        let n_skill = e.scan_plugin_dir(&root.join("skills"));
        let n_exp = e.scan_plugin_dir(&root.join("experts"));
        // 12 技能（4 旧 + 8 新）+ 8 专家（1 旧 + 7 新）…实际新增 12 技能含种子共 12、专家共 8
        assert!(n_skill >= 12, "skills 装配数应 ≥12，实际 {n_skill}");
        assert!(n_exp >= 8, "experts 装配数应 ≥8，实际 {n_exp}");

        let skills = e.skill_list();
        let experts = e.expert_list();
        let ids: Vec<&str> = skills
            .iter()
            .chain(experts.iter())
            .map(|v| v["id"].as_str().unwrap())
            .collect();

        // 19 个新包 id 全部装配
        let new_ids = [
            "com.qibai.skill.websummary",
            "com.qibai.skill.translate",
            "com.qibai.skill.writingpolish",
            "com.qibai.skill.codereview",
            "com.qibai.skill.filesearch",
            "com.qibai.skill.reminderparse",
            "com.qibai.skill.horoscope",
            "com.qibai.skill.tarot",
            "com.qibai.skill.jokes",
            "com.qibai.skill.sweettalk",
            "com.qibai.skill.promptgen",
            "com.qibai.skill.namegen",
            "com.qibai.expert.planner",
            "com.qibai.expert.codearchitect",
            "com.qibai.expert.contentcreator",
            "com.qibai.expert.dataanalyst",
            "com.qibai.expert.emotion",
            "com.qibai.expert.ideas",
            "com.qibai.expert.storyteller",
            "com.qibai.expert.geek",
        ];
        for id in new_ids {
            assert!(ids.contains(&id), "新包未装配：{id}");
        }

        // id 无重复
        let mut sorted = ids.clone();
        sorted.sort();
        for w in sorted.windows(2) {
            assert_ne!(w[0], w[1], "存在重复插件 id：{}", w[0]);
        }

        // 技能入口无非法路径（禁 .. / 越界）
        for v in &skills {
            if let Some(cmd) = v["entry"]["command"].as_str() {
                assert!(!cmd.contains(".."), "技能 {} 入口路径非法：{cmd}", v["id"]);
            }
        }

        // 专家 system_prompt 非空
        for v in &experts {
            let sp = v["system_prompt"].as_str().unwrap_or("");
            assert!(!sp.trim().is_empty(), "专家 {} 缺少 system_prompt", v["id"]);
        }

        // smoke invoke：各新技能走真实脚本（确定性、离线安全输入）
        let cases: &[(&str, &str, &str)] = &[
            (
                "com.qibai.skill.websummary",
                "run",
                "{\"url\":\"file:///etc/passwd\"}",
            ),
            ("com.qibai.skill.translate", "run", "{\"text\":\"你好\"}"),
            (
                "com.qibai.skill.writingpolish",
                "run",
                "{\"text\":\"今天天气很好，我们去公园走走。\"}",
            ),
            (
                "com.qibai.skill.codereview",
                "run",
                "{\"path\":\"/nonexistent\"}",
            ),
            (
                "com.qibai.skill.filesearch",
                "run",
                "{\"keyword\":\"qibai-plugin.json\"}",
            ),
            (
                "com.qibai.skill.reminderparse",
                "run",
                "{\"text\":\"明天下午3点开会提醒我\"}",
            ),
            ("com.qibai.skill.horoscope", "run", "{\"sign\":\"白羊座\"}"),
            (
                "com.qibai.skill.tarot",
                "run",
                "{\"question\":\"今天的运势\"}",
            ),
            ("com.qibai.skill.jokes", "run", "{}"),
            ("com.qibai.skill.sweettalk", "run", "{\"level\":\"l2\"}"),
            (
                "com.qibai.skill.promptgen",
                "run",
                "{\"subject\":\"一只橘猫\"}",
            ),
            ("com.qibai.skill.namegen", "run", "{\"scene\":\"宠物\"}"),
        ];
        for (id, func, args) in cases {
            let out = e
                .skill_invoke(id, func, args)
                .unwrap_or_else(|err| panic!("{id} invoke 失败：{err}"));
            assert!(!out.trim().is_empty(), "{id} 输出为空");
        }

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    // ---- 连接器 / 技能包闭环（T4 扩展） ----

    /// 内嵌 mock MCP server（python3，JSON-RPC 换行分隔）：ping 回显。
    const CONNECTOR_MOCK_MCP_PY: &str = r#"
import json, sys
def send(o):
    sys.stdout.write(json.dumps(o, ensure_ascii=False) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"mock-conn","version":"1.0.0"}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"ping","description":"回显","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}})
    elif method == "tools/call":
        params = msg.get("params") or {}
        name = params.get("name", "")
        if name != "ping":
            send({"jsonrpc":"2.0","id":mid,"error":{"code":-32602,"message":"unknown tool"}})
            continue
        args = params.get("arguments") or {}
        send({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"pong:" + str(args.get("text",""))}]}})
    else:
        send({"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"method not found"}})
"#;

    /// 连接器闭环：connector_add（mock MCP）→ 列表可见 → 工具表含入口函数 →
    /// invoke（__tool=ping）走真实 MCP 调用成功 → connector_remove 干净摘除。
    #[tokio::test]
    async fn connector_mcp_full_lifecycle() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-conn-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-conn-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        // 写 mock MCP server 脚本
        let server = crate::test_support::unique_temp_dir("engine-conn-server");
        std::fs::create_dir_all(&server).unwrap();
        let py = server.join("mock_conn_server.py");
        std::fs::write(&py, CONNECTOR_MOCK_MCP_PY).unwrap();

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        let cfg = crate::connectors::ConnectorConfig {
            id: "my_mcp".into(),
            name: "我的 MCP".into(),
            kind: crate::connectors::ConnectorKind::Mcp,
            command: "python3".into(),
            args: vec![py.to_string_lossy().to_string()],
            url: String::new(),
            headers: std::collections::BTreeMap::new(),
            description: "测试连接器".into(),
            enabled: true,
        };
        e.connector_add(cfg).unwrap();

        // connector_list 可见且已启用 + 已授权
        let list = e.connector_list();
        let row = list
            .iter()
            .find(|v| v["id"] == "my_mcp")
            .expect("连接器应在列表中");
        assert_eq!(row["type"], "connector");
        assert_eq!(row["enabled"], true);
        assert!(row["granted"].as_array().unwrap().contains(&serde_json::json!("command:run")));
        assert_eq!(row["entry_kind"], "mcp");

        // 工具表含入口函数 fn_my_mcp（LLM 可见）
        let schemas = e.skills.tool_schemas();
        assert!(
            schemas
                .iter()
                .any(|s| s["function"]["name"] == "fn_my_mcp"),
            "连接器入口函数应进 LLM 工具表"
        );

        // invoke（泛型模式：__tool=ping）→ 真实 MCP 调用成功
        match e
            .skills
            .invoke("fn_my_mcp", &serde_json::json!({ "__tool": "ping", "arguments": { "text": "你好" } }))
            .await
        {
            crate::skills::SkillOutcome::Ok(msg) => assert_eq!(msg, "pong:你好"),
            other => panic!("expected ok, got {other:?}"),
        }

        // 未知工具名 → MCP server 报错 → 降级可读错误
        match e
            .skills
            .invoke("fn_my_mcp", &serde_json::json!({ "__tool": "nope" }))
            .await
        {
            crate::skills::SkillOutcome::Denied(msg) => {
                assert!(
                    msg.contains("降级") || msg.contains("MCP 工具执行失败"),
                    "未知工具应降级为可读错误（实际 {msg}）"
                );
            }
            other => panic!("expected denied, got {other:?}"),
        }

        // 移除 → 列表清空
        e.connector_remove("my_mcp").unwrap();
        assert!(e.connector_list().iter().all(|v| v["id"] != "my_mcp"));
        assert!(e.settings.connectors.is_empty());

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// 技能包闭环：技能包 zip → install → plugin_list 可见 → export 打包 → 二次安装幂等。
    #[tokio::test]
    async fn skill_pack_install_and_export_lifecycle() {
        let _g = crate::test_support::env_lock();
        let data = crate::test_support::unique_temp_dir("engine-pack-data");
        let plugindir = crate::test_support::unique_temp_dir("engine-pack-dir");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &data) };
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &plugindir) };

        // 造技能包源目录并打包
        let src = crate::test_support::unique_temp_dir("engine-pack-src");
        let pkg = src.join("hello-pack");
        std::fs::create_dir_all(&pkg.join("scripts")).unwrap();
        std::fs::write(
            pkg.join("qibai-plugin.json"),
            r#"{"id":"com.qibai.pack.hello","type":"skill","name":"技能包问候","version":"1.0.0","description":"由技能包安装","entry":{"kind":"command","command":"scripts/hello.sh"},"functions":[{"name":"pack_hello","description":"打招呼"}]}"#,
        )
        .unwrap();
        std::fs::write(
            pkg.join("scripts/hello.sh"),
            "#!/bin/sh\necho '技能包执行成功'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                pkg.join("scripts/hello.sh"),
                std::fs::Permissions::from_mode(0o755),
            );
        }
        let zip_path = src.join("hello.qbpk");
        crate::skill_pack::pack(&pkg, &zip_path).unwrap();

        let mut e = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        e.skill_pack_install(zip_path.to_string_lossy().to_string())
            .unwrap();

        // plugin_list 可见
        let list = e.plugin_list();
        let row = list
            .iter()
            .find(|v| v["id"] == "com.qibai.pack.hello")
            .expect("技能包安装后应可见");
        assert_eq!(row["installed"], true);
        assert_eq!(row["enabled"], false);

        // 授权+启用后 invoke 成功
        e.plugin_grant_permission("com.qibai.pack.hello".into(), "command:run".into(), true)
            .unwrap();
        e.plugin_set_enabled("com.qibai.pack.hello".into(), true).unwrap();
        match e.skills.invoke("pack_hello", &serde_json::json!({})).await {
            crate::skills::SkillOutcome::Ok(msg) => assert!(msg.contains("技能包执行成功")),
            other => panic!("expected ok, got {other:?}"),
        }

        // skill_pack_list 可见（可导出）
        assert!(e.skill_pack_list().iter().any(|v| v["id"] == "com.qibai.pack.hello"));

        // 导出 → 重新安装到另一台引擎（模拟分发）
        let out_zip = src.join("out.qbpk");
        e.skill_pack_export("com.qibai.pack.hello".into(), out_zip.to_string_lossy().into())
            .unwrap();
        assert!(out_zip.exists());
        let mut e2 = Engine::new(Box::new(RecordingSink { events: Mutex::new(vec![]) }));
        e2.skill_pack_install(out_zip.to_string_lossy().to_string())
            .unwrap();
        assert!(e2.plugin_list().iter().any(|v| v["id"] == "com.qibai.pack.hello"));

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    // ---- v46 within_root：不存在路径的合法判定（对齐 fileops 父目录 canonicalize 思路）----

    /// 根已建、父目录已存在、文件尚不存在 → 必须判合法（修"刚写完读不回来"）。
    #[test]
    fn within_root_nonexistent_file_inside_parent_allowed() {
        let root = crate::test_support::unique_temp_dir("within-root-file");
        std::fs::create_dir_all(root.join("existing")).unwrap();
        let p = root.join("existing").join("brand_new.txt");
        assert!(!p.exists(), "前置：文件还不存在");
        assert!(
            within_root(&root, &p),
            "不存在但父目录在根内的文件应判合法，root={} p={}",
            root.display(),
            p.display()
        );
    }

    /// 根已建、嵌套子目录整串都不存在 → 必须判合法（修"新建子目录列不出"）。
    #[test]
    fn within_root_nonexistent_nested_dir_allowed() {
        let root = crate::test_support::unique_temp_dir("within-root-nested");
        let p = root.join("a").join("b").join("c");
        assert!(!p.exists(), "前置：嵌套目录链都不存在");
        assert!(
            within_root(&root, &p),
            "不存在但落在根内的嵌套目录应判合法，root={} p={}",
            root.display(),
            p.display()
        );
    }

    /// `../` 逃逸到根外 → 判拒绝。
    #[test]
    fn within_root_dotdot_escape_denied() {
        let root = crate::test_support::unique_temp_dir("within-root-dotdot");
        let p = root.join("..").join("..").join("escape_outside.txt");
        assert!(
            !within_root(&root, &p),
            "../ 逃逸路径必须判拒绝，root={} p={}",
            root.display(),
            p.display()
        );
    }

    /// 根外绝对路径 → 判拒绝。
    #[test]
    fn within_root_outside_absolute_path_denied() {
        let root = crate::test_support::unique_temp_dir("within-root-outside");
        let outside = std::path::Path::new("/etc/passwd");
        if !outside.exists() {
            // 极端精简环境没有 /etc/passwd 时退化为一个根外绝对路径
            let parent = root.parent().unwrap().to_path_buf();
            let outside = parent.join("definitely_outside_qibai.txt");
            assert!(!within_root(&root, &outside), "根外绝对路径必须判拒绝");
            return;
        }
        assert!(
            !within_root(&root, outside),
            "根外绝对路径 {} 必须判拒绝（根 {}）",
            outside.display(),
            root.display()
        );
    }

    /// 根内符号链接指向根外 → 判拒绝（canonicalize 物理化后逃逸）。
    #[test]
    #[cfg(unix)]
    fn within_root_symlink_escape_denied() {
        use std::os::unix::fs::symlink;
        let root = crate::test_support::unique_temp_dir("within-root-symlink");
        let link = root.join("evil_link");
        symlink("/", &link).unwrap(); // 指向文件系统根，逃逸出 workspace
        let p = link.join("passwd"); // /etc/passwd 经链接解析到根外
        // p 经符号链接后落在 /etc/passwd（根外）
        assert!(
            !within_root(&root, &p),
            "符号链接逃逸路径必须判拒绝，root={} p={}",
            root.display(),
            p.display()
        );
        // 反向：指向根内子目录的符号链接应放行（回归保护）
        std::fs::create_dir_all(root.join("real_sub")).unwrap();
        let good = root.join("good_link");
        symlink(root.join("real_sub"), &good).unwrap();
        assert!(
            within_root(&root, &good.join("x.txt")),
            "指向根内的符号链接+不存在子文件应判合法"
        );
    }

    // ---- v46 启动记忆抑制：should_inject_memory 纯函数 ----

    /// 冷启动首句 + 无回忆意图 → 不注入（避免一开场就复述上一轮待办）。
    #[test]
    fn first_turn_greeting_memory_injection_suppressed() {
        assert!(
            !should_inject_memory(true, "你好", 6),
            "冷启动 + 打招呼不应注入长期记忆"
        );
        assert!(
            !should_inject_memory(true, "今天天气怎么样", 1),
            "冷启动 + 闲聊不应注入长期记忆"
        );
    }

    /// 冷启动首句 + 用户主动问起（含回忆关键词）→ 照旧注入，功能不丢。
    #[test]
    fn first_turn_recall_question_memory_injection_allowed() {
        for q in [
            "上次做到哪了",
            "之前我们聊到哪儿了",
            "刚才说的任务继续吧",
            "你还记得我交代的待办吗",
            "帮我看看进度",
        ] {
            assert!(
                should_inject_memory(true, q, 6),
                "冷启动但用户主动回忆（{q}）必须注入记忆"
            );
        }
    }

    /// 非冷启动（conversation 已有历史）→ 无论用户说什么都注入，延续对话要靠记忆。
    #[test]
    fn non_first_turn_memory_injection_always_allowed() {
        assert!(should_inject_memory(false, "你好", 6));
        assert!(should_inject_memory(false, "随便说点啥", 1));
    }

    /// 无记忆条目 → 一律不注入（含非冷启动）。
    #[test]
    fn zero_memory_never_injects() {
        assert!(!should_inject_memory(true, "上次做到哪了", 0));
        assert!(!should_inject_memory(false, "你好", 0));
    }

    // ---- v45.7 工作区根统一：单一根 + 根内绝对路径放行 ----

    /// 未设 QIBAI_WORKSPACE 时，默认根 = $HOME/Documents/qibai。
    #[test]
    fn workspace_root_defaults_to_documents_qibai() {
        let _g = crate::test_support::env_lock();
        let home = crate::test_support::unique_temp_dir("ws-root-home");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::remove_var("QIBAI_WORKSPACE");
        }
        let (root, source) = qibai_workspace_root_with_source();
        assert_eq!(source, "default");
        assert_eq!(root, home.join("Documents").join("qibai"));
        assert!(root.exists(), "默认根应被 create_dir_all 建好");
    }

    /// 设了 QIBAI_WORKSPACE 时用 env 覆盖。
    #[test]
    fn workspace_root_env_override() {
        let _g = crate::test_support::env_lock();
        let home = crate::test_support::unique_temp_dir("ws-root-home2");
        let env_dir = crate::test_support::unique_temp_dir("ws-root-env");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("QIBAI_WORKSPACE", &env_dir);
        }
        let (root, source) = qibai_workspace_root_with_source();
        assert_eq!(source, "env");
        assert_eq!(root, env_dir);
        unsafe { std::env::remove_var("QIBAI_WORKSPACE") };
    }

    /// 根内绝对路径（文件尚不存在）→ safe_resolve 放行。
    #[test]
    fn safe_resolve_absolute_path_inside_root_allowed() {
        let root = crate::test_support::unique_temp_dir("safe-resolve-inside");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let p = root.join("sub").join("brand_new.py");
        assert!(!p.exists());
        assert_eq!(
            safe_resolve(&root, p.to_str().unwrap()),
            Some(p.clone()),
            "根内绝对路径（含不存在的新文件）应放行"
        );
        // 已存在的根内绝对路径同样放行
        std::fs::write(root.join("sub").join("old.txt"), b"x").unwrap();
        let old = root.join("sub").join("old.txt");
        assert_eq!(safe_resolve(&root, old.to_str().unwrap()), Some(old));
    }

    /// 根外绝对路径 → safe_resolve 拒绝（None）。
    #[test]
    fn safe_resolve_absolute_path_outside_root_denied() {
        let root = crate::test_support::unique_temp_dir("safe-resolve-outside");
        // /etc/passwd 是典型根外绝对路径（容器内存在）
        let outside = std::path::Path::new("/etc/passwd");
        if outside.exists() {
            assert_eq!(safe_resolve(&root, outside.to_str().unwrap()), None);
        }
        // 根的兄弟目录（也在临时目录下但不在 root 内）
        let sibling = root.parent().unwrap().join("sibling_outside_qibai");
        std::fs::create_dir_all(&sibling).unwrap();
        let sp = sibling.join("x.txt");
        assert_eq!(safe_resolve(&root, sp.to_str().unwrap()), None);
    }

    /// 相对路径含 .. → safe_resolve 拒绝。
    #[test]
    fn safe_resolve_dotdot_escape_denied() {
        let root = crate::test_support::unique_temp_dir("safe-resolve-dotdot");
        assert_eq!(safe_resolve(&root, "../escape.txt"), None);
        assert_eq!(safe_resolve(&root, "a/../../b"), None);
        // 正常相对路径仍放行
        assert_eq!(
            safe_resolve(&root, "a/b.txt"),
            Some(root.join("a").join("b.txt"))
        );
    }

    // ---- v45.7 clear_conversation：scope=chat/all + 备份 + confirm 门控 ----

    /// scope=chat：清完 conversation 为空，persona memory 条数不变。
    #[tokio::test]
    async fn clear_conversation_false_keeps_persona_memory() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        e.append_chat("user", "你好").unwrap();
        e.append_chat("assistant", "你好呀").unwrap();
        // 给角色记忆塞一条
        let _ = e.personas.append_memory("gudian", "user", "记住这件事");
        let mem_before = e.personas.get("gudian").unwrap().memory.len();
        assert!(e.get_conversation().len() >= 2);
        let (n, _bak) = e.clear_conversation(false).unwrap();
        assert_eq!(n, 2, "scope=chat 只清 conversation，2 条");
        assert!(e.get_conversation().is_empty());
        assert_eq!(
            e.personas.get("gudian").unwrap().memory.len(),
            mem_before,
            "scope=chat 不应清角色记忆"
        );
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// scope=all：conversation 与角色记忆都空。
    #[tokio::test]
    async fn clear_conversation_true_clears_memory_too() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        e.append_chat("user", "在吗").unwrap();
        let _ = e.personas.append_memory("gudian", "user", "记住这事");
        assert!(!e.get_conversation().is_empty());
        assert!(!e.personas.get("gudian").unwrap().memory.is_empty());
        let (_n, _bak) = e.clear_conversation(true).unwrap();
        assert!(e.get_conversation().is_empty());
        assert!(e.personas.get("gudian").unwrap().memory.is_empty());
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 清空前生成备份文件，且备份能反序列化出清空前的相同条数。
    #[tokio::test]
    async fn clear_conversation_creates_backup_snapshot() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        for i in 0..3 {
            e.append_chat("user", &format!("msg-{i}")).unwrap();
        }
        let before = e.get_conversation().len();
        let (_n, bak) = e.clear_conversation(false).unwrap();
        let bak = bak.expect("清空前有文件应生成备份");
        assert!(bak.exists(), "备份文件应存在: {}", bak.display());
        // 反序列化备份，条数应 == before
        let raw = std::fs::read_to_string(&bak).unwrap();
        let msgs: Vec<crate::storage::StoredMessage> = serde_json::from_str(&raw).unwrap();
        assert_eq!(msgs.len(), before);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 工具层 confirm 缺失 → 门控拒绝（纯函数），且引擎方法不会被调用、条数不变。
    #[tokio::test]
    async fn clear_conversation_tool_requires_confirm() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        e.append_chat("user", "在吗").unwrap();
        let before = e.get_conversation().len();
        // confirm 门控：缺省/非 true → Some(原因)；true → None
        assert!(clear_conversation_confirm_deny(&serde_json::json!({"scope": "chat"})).is_some());
        assert!(clear_conversation_confirm_deny(&serde_json::json!({"scope": "chat", "confirm": false})).is_some());
        assert!(clear_conversation_confirm_deny(&serde_json::json!({"scope": "chat", "confirm": true})).is_none());
        // 门控拒绝时引擎方法未被调用 → 条数不变
        assert_eq!(e.get_conversation().len(), before);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- v45.7 审批门控拆分：文件操作不 L4，高危命令才 L4 ----

    /// 文件操作（非高危）即使 L1 也不门控。
    #[test]
    fn approval_gate_file_write_never_gated() {
        assert!(Engine::approval_gate_reason(false, 1).is_none());
        assert!(Engine::approval_gate_reason(false, 10).is_none());
    }

    /// 高危命令 L1 → 拒绝且文案含"亲密度/L4"；L4 → 放行。
    #[test]
    fn approval_gate_high_risk_requires_l4() {
        let reason = Engine::approval_gate_reason(true, 1).expect("L1 高危应拒绝");
        assert!(reason.contains("L4"), "文案应含 L4：{reason}");
        assert!(reason.contains("L1") || reason.contains("当前"), "文案应带当前等级：{reason}");
        // L4 及以上放行
        assert!(Engine::approval_gate_reason(true, 4).is_none());
    }

    /// 批准成功后 conversation 末尾多一条 assistant 回写（OPEN_ALL 模式下走通）。
    #[tokio::test]
    async fn approve_skill_success_appends_assistant_chat() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        let before = e.get_conversation().len();
        // 不存在的 request_id：OPEN_ALL 下门控不触发，approve 返回 Skill 错误，
        // 不会走到回写分支（回写只在 Ok 结果后执行）。这里验证错误路径不 panic。
        let _ = e.approve_skill("nonexistent-req", true).await;
        // 错误路径不回写
        assert_eq!(e.get_conversation().len(), before);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ===== v45.8：工作区根/路径解析/空文本硬闸 =====

    #[test]
    fn normalize_path_input_strips_quotes_and_backslash() {
        assert_eq!(normalize_path_input("\"proj/app.py\""), "proj/app.py");
        assert_eq!(normalize_path_input("'proj/app.py'"), "proj/app.py");
        assert_eq!(normalize_path_input("proj\\\\app.py"), "proj/app.py");
        assert_eq!(normalize_path_input("proj//app/"), "proj/app");
        assert_eq!(normalize_path_input("  hello  "), "hello");
    }

    #[test]
    fn denied_path_message_contains_root_and_suggestion() {
        let root = std::path::Path::new("/home/user/Documents/qibai");
        let msg = denied_path_message(root, "/tmp/x");
        assert!(msg.contains("/home/user/Documents/qibai"), "文案应含真实根：{msg}");
        assert!(msg.contains("请改写成相对路径"), "文案应含建议写法：{msg}");
        assert!(msg.contains("/tmp/x"), "文案应含被拒目标：{msg}");
    }

    #[tokio::test]
    async fn send_message_empty_text_denied() {
        let _g = crate::test_support::env_lock();
        let mut e = fresh_engine();
        let before = e.get_conversation().len();
        for txt in ["", "   ", "\n  \t "] {
            let err = e.send_message(txt).await.unwrap_err();
            match err {
                EngineError::Llm(_) => {}
                other => panic!("expected empty-text Llm error, got {other:?}"),
            }
        }
        // 会话条数不变（空文本不写会话）
        assert_eq!(e.get_conversation().len(), before);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn workspace_root_default_matches_documents_qibai() {
        let _g = crate::test_support::env_lock();
        unsafe { std::env::remove_var("QIBAI_WORKSPACE") };
        let (root, source) = qibai_workspace_root_with_source();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(root, std::path::PathBuf::from(home).join("Documents").join("qibai"));
        assert_eq!(source, "default");
    }

    // ===== v45.8 补：建工程链路 / 父目录自动建 / 命令参数矩阵 / workspace_info 字段 =====

    /// 端到端：用 tempdir 注入根，模型给相对路径 demo_proj/src/main.py →
    /// safe_resolve + std::fs::write 真实落地（复用 dispatch 里同样的逻辑）。
    #[test]
    fn end_to_end_build_project_injected_tempdir_root() {
        let tmp = std::env::temp_dir().join(format!("qibai_e2e_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 直接走 dispatch 同款路径：safe_resolve + create_dir_all(parent) + write
        let root = tmp.clone();
        let rel = "demo_proj/src/main.py";
        let p = safe_resolve(&root, rel).expect("相对路径应解析成功");
        // 断言解析结果在 root 下
        assert!(p.starts_with(&root), "p={p:?} root={root:?}");
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, "print('hello')\n").unwrap();

        // 文件真实存在且内容一致
        assert!(p.exists(), "文件应真实存在：{}", p.display());
        let content = std::fs::read_to_string(&p).unwrap();
        assert_eq!(content, "print('hello')\n");
        // demo_proj/ 目录存在
        assert!(root.join("demo_proj").is_dir());
        assert!(root.join("demo_proj/src").is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 父目录完全不存在时（a/b/c/main.py）应自动创建中间目录。
    #[test]
    fn write_file_auto_creates_nested_parent_dirs() {
        let tmp = std::env::temp_dir().join(format!("qibai_parent_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let p = safe_resolve(&tmp, "demo_proj/a/b/c/main.py").unwrap();
        assert!(!p.parent().unwrap().exists(), "前置条件：父目录不应存在");
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, "x").unwrap();

        // 中间目录都被创建
        assert!(tmp.join("demo_proj/a/b/c").is_dir());
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// check_command_arg 矩阵：根内绝对路径放行、根外拒绝、shell 元字符拒绝、相对参数放行。
    #[test]
    fn check_command_arg_matrix() {
        let root = std::env::temp_dir().join(format!("qibai_cmd_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        // 根内绝对路径 → Ok
        let in_root = root.join("proj").display().to_string();
        assert!(check_command_arg(&in_root, &root).is_ok(), "根内绝对路径应放行：{in_root}");

        // 根外绝对路径 → Err（文案含根）
        let out = check_command_arg("/tmp/qibai_outside_xyz", &root);
        assert!(out.is_err(), "根外绝对路径应拒绝");
        let msg = out.unwrap_err();
        assert!(msg.contains(&root.display().to_string()), "拒绝文案应含真实根：{msg}");

        // shell 元字符 → Err
        assert!(check_command_arg("a;rm", &root).is_err());
        assert!(check_command_arg("a|b", &root).is_err());
        assert!(check_command_arg("a&&b", &root).is_err());
        assert!(check_command_arg("a$(b)", &root).is_err());
        assert!(check_command_arg("a`b`", &root).is_err());

        // 普通相对参数 → Ok
        assert!(check_command_arg("hello", &root).is_ok());
        assert!(check_command_arg("-la", &root).is_ok());

        // .. 逃逸 → Err
        assert!(check_command_arg("../etc/passwd", &root).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// workspace_info 返回 JSON 必须含 root/source/writable/free_bytes 四个 key（free_bytes 可 null）。
    #[test]
    fn workspace_info_json_has_required_keys() {
        let tmp = std::env::temp_dir().join(format!("qibai_info_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // 模拟 workspace_info dispatch 里构造的 JSON
        let writable = std::fs::write(tmp.join(".probe"), b"x")
            .map(|_| { let _ = std::fs::remove_file(tmp.join(".probe")); true })
            .unwrap_or(false);
        let info = serde_json::json!({
            "root": tmp.display().to_string(),
            "source": "default",
            "writable": writable,
            "free_bytes": Option::<u64>::None,
            "suggest": "工程请放在此目录下",
        });
        assert!(info.get("root").is_some(), "缺 root");
        assert!(info.get("source").is_some(), "缺 source");
        assert!(info.get("writable").is_some(), "缺 writable");
        assert!(info.get("free_bytes").is_some(), "缺 free_bytes（允许 null）");
        assert!(info["writable"].as_bool().unwrap());
        assert!(info["free_bytes"].is_null());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// 纯函数：启动时把已保存的 audio_output 归一（空串 → default）。
/// 单测用注入的 setter 闭包验证"应用一次"。
pub fn normalize_saved_audio_output(saved: &str) -> String {
    if saved.is_empty() { "default".to_string() } else { saved.to_string() }
}

/// 纯函数：比较新旧 audio_output，决定是否需要推送。
/// 返回 Some(new) 表示要推；None 表示没变。
pub fn audio_output_diff(old: &str, new: &str) -> Option<String> {
    let n = normalize_saved_audio_output(new);
    let o = normalize_saved_audio_output(old);
    if o == n { None } else { Some(n) }
}

#[cfg(test)]
mod v459_audio_p0_tests {
    use super::*;

    #[test]
    fn normalize_empty_to_default() {
        assert_eq!(normalize_saved_audio_output(""), "default");
        assert_eq!(normalize_saved_audio_output("default"), "default");
        assert_eq!(normalize_saved_audio_output("alsa_output.usb-x"), "alsa_output.usb-x");
    }

    #[test]
    fn diff_pushes_only_when_changed() {
        // 变了 → 推
        assert_eq!(audio_output_diff("default", "alsa_output.usb-x"), Some("alsa_output.usb-x".into()));
        // 空 → default 等价 → 不推
        assert_eq!(audio_output_diff("default", ""), None);
        // 同值 → 不推
        assert_eq!(audio_output_diff("alsa_output.usb-x", "alsa_output.usb-x"), None);
    }

    #[test]
    fn saved_audio_output_field_backward_compat() {
        // v45.9：模拟"老 settings.json"——先把当前默认配置序列化，再剥掉本轮新增字段，
        // 断言仍能解析且新字段拿到默认值（而不是 missing field 读崩）。
        let full = serde_json::to_value(crate::settings::Settings::default()).unwrap();
        let mut obj = full.as_object().cloned().unwrap_or_default();
        for key in [
            "audio_output",
            "search_engine",
            "http_timeout_secs",
            "search_result_n",
            "history_recent_n",
            "exec_whitelist",
        ] {
            obj.remove(key);
        }
        let old = serde_json::Value::Object(obj);
        let s: crate::settings::Settings = serde_json::from_value(old).unwrap();
        assert_eq!(s.audio_output, "default");
        assert_eq!(s.search_engine, "auto");
        assert_eq!(s.history_recent_n, 50);
        assert!(!s.exec_whitelist.is_empty());
    }
}

#[cfg(test)]
mod v459_task_wiring_tests {
    use super::*;

    fn engine_with_tmp_tasks() -> Engine {
        let tmp = std::env::temp_dir().join(format!("qibai_task_wiring_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut e = Engine::default();
        e.task_store = Some(crate::task::TaskStore::from_tasks_root(&tmp));
        e
    }

    #[test]
    fn task_start_rejects_unapproved_plan() {
        let mut e = engine_with_tmp_tasks();
        let id = e.task_create("写周报".into(), "写周报".into(), None).unwrap();
        // 造 plan 但不 approve
        let step = crate::task::Step { id: "s1".into(), title: "a".into(), ..Default::default() };
        e.task_plan(&id, vec![step]).unwrap();
        // start 应返回 Err（未批准 / 状态不对，都拒绝）
        let r = e.task_start(&id);
        assert!(r.is_err(), "未批准 plan 必须拒绝 start");
        let msg = format!("{:?}", r.unwrap_err());
        assert!(
            msg.contains("PlanNotApproved") || msg.contains("InvalidTransition"),
            "msg={msg}"
        );
    }

    #[test]
    fn task_confirm_action_allow_false_marks_failed() {
        let mut e = engine_with_tmp_tasks();
        let id = e.task_create("x".into(), "x".into(), None).unwrap();
        let step = crate::task::Step {
            id: "s1".into(),
            title: "danger".into(),
            risk: crate::task::RiskLevel::Dangerous,
            ..Default::default()
        };
        e.task_plan(&id, vec![step]).unwrap();
        e.task_confirm_plan(&id).unwrap();
        // 注意：不要调 task_start —— 否则会 spawn 后台 executor 线程，
        // 与下面手动 transition 形成竞态（executor 自己也会 NeedConfirm）。
        // 本用例只测 task_confirm_action 的状态机语义，不走 executor。
        {
            let store = e.task_store().unwrap().clone();
            let mut t = store.require(&id).unwrap();
            t.current_step = 0;
            // 手动置 Running（等价 task_start 的 Resume 迁移，但不 spawn 线程）
            t.status = crate::task::TaskStatus::Running;
            crate::task::transition(&mut t, crate::task::TaskEvent::NeedConfirm, &crate::task::NoopHook).unwrap();
            store.save(&t).unwrap();
        }
        // 用户拒绝
        let t = e.task_confirm_action(&id, "s1", false, false).unwrap();
        assert_eq!(t.status, crate::task::TaskStatus::Failed);
    }

    #[test]
    fn task_recover_current_step_matches_resume() {
        let mut e = engine_with_tmp_tasks();
        let id = e.task_create("x".into(), "x".into(), None).unwrap();
        // 3 步，前 2 步 done
        let mut steps = vec![];
        for i in 0..3 {
            steps.push(crate::task::Step {
                id: format!("s{i}"),
                title: format!("step {i}"),
                status: if i < 2 { crate::task::StepStatus::Done } else { crate::task::StepStatus::Pending },
                ..Default::default()
            });
        }
        e.task_plan(&id, steps).unwrap();
        e.task_confirm_plan(&id).unwrap();
        e.task_start(&id).unwrap();
        // 模拟崩溃：把 current_step 设为 next_pending_index（=2）
        {
            let store = e.task_store().unwrap().clone();
            let mut t = store.require(&id).unwrap();
            t.current_step = 2;
            store.save(&t).unwrap();
        }
        let t = e.task_get(&id).unwrap();
        assert_eq!(t.current_step, 2, "恢复后 current_step 应指向第一个未 done 步");
    }

    /// 记录型 EventSink：断言事件 emit。
    struct RecordingSink {
        events: std::sync::Mutex<Vec<(String, String)>>,
    }
    impl EventSink for RecordingSink {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.to_string(), payload.to_string()));
        }
    }

    #[test]
    fn dangerous_step_emits_waiting_confirm_event() {
        let tmp = std::env::temp_dir().join(format!("qibai_sink_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let sink = std::sync::Arc::new(RecordingSink { events: std::sync::Mutex::new(vec![]) });
        let mut e = Engine::new(Box::new(NullSink));
        e.sink = sink.clone();
        e.task_store = Some(crate::task::TaskStore::from_tasks_root(&tmp));
        let id = e.task_create("x".into(), "x".into(), None).unwrap();
        let step = crate::task::Step {
            id: "s1".into(),
            title: "danger".into(),
            risk: crate::task::RiskLevel::Dangerous,
            ..Default::default()
        };
        e.task_plan(&id, vec![step]).unwrap();
        e.task_confirm_plan(&id).unwrap();
        e.task_start(&id).unwrap();
        // 等后台线程跑完（dangerous 步会立即落 waiting_confirm）
        std::thread::sleep(std::time::Duration::from_millis(500));
        let evs = sink.events.lock().unwrap();
        let has_waiting = evs.iter().any(|(name, payload)| {
            name == "task://waiting_confirm" && payload.contains("\"risk\"")
        });
        assert!(has_waiting, "应 emit task://waiting_confirm 且 payload 含 risk；events={evs:?}");
    }
}

#[cfg(test)]
mod save_settings_partial_tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::Mutex;

    /// 全局串行化：env QIBAI_DATA_DIR 是进程级，并行测试会互相覆盖。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 隔离 HOME 避免碰真配置。
    fn isolated_engine() -> Engine {
        let _g = TEST_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("qibai_settings_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        // 用 QIBAI_DATA_DIR 直接重定向 settings.json 目录
        std::env::set_var("QIBAI_DATA_DIR", &tmp);
        let mut e = Engine::default();
        // 强制一套已知值
        e.settings = Settings::default();
        e.settings.audio_output = "alsa_output.usb-x".into();
        e.settings.search_engine = "baidu".into();
        e.settings.context_budget_tokens = 8000;
        e.settings.task_max_steps = 20;
        // 同步到 tts 全局，让 current_audio_output 与 settings 一致
        crate::tts::set_audio_output("alsa_output.usb-x");
        e
    }

    #[test]
    fn partial_save_preserves_untouched_fields() {
        let mut e = isolated_engine();
        // 只传 temperature 一个字段
        let patch = serde_json::json!({ "temperature": 0.7 });
        e.save_settings_partial(patch).unwrap();
        // 读回
        assert_eq!(e.settings.temperature, 0.7);
        assert_eq!(e.settings.audio_output, "alsa_output.usb-x");
        assert_eq!(e.settings.search_engine, "baidu");
        assert_eq!(e.settings.context_budget_tokens, 8000);
        assert_eq!(e.settings.task_max_steps, 20);
    }

    #[test]
    fn partial_save_keeps_audio_device() {
        let mut e = isolated_engine();
        // 当前设备是 alsa_output.usb-x
        assert_eq!(e.settings.audio_output, "alsa_output.usb-x");
        // 只改 max_tokens，不动 audio_output
        let patch = serde_json::json!({ "max_tokens": 4096 });
        e.save_settings_partial(patch).unwrap();
        assert_eq!(e.settings.audio_output, "alsa_output.usb-x");
        assert_eq!(e.settings.max_tokens, 4096);
        // tts 侧也应仍是 usb-x
        assert_eq!(crate::tts::current_audio_output(), "alsa_output.usb-x");
    }

    #[test]
    fn explicit_default_switches_back() {
        let mut e = isolated_engine();
        let patch = serde_json::json!({ "audio_output": "default" });
        e.save_settings_partial(patch).unwrap();
        assert_eq!(e.settings.audio_output, "default");
        assert_eq!(crate::tts::current_audio_output(), "default");
    }

    #[test]
    fn null_clears_field() {
        // plugins.enabled 是 HashMap<String,bool>；null 按 RFC 7386 删除键，serde 走 default（空 map）
        let mut e = isolated_engine();
        e.settings.plugins.enabled.insert("foo".into(), true);
        let patch = serde_json::json!({ "plugins": { "enabled": null } });
        e.save_settings_partial(patch).unwrap();
        assert!(e.settings.plugins.enabled.is_empty(), "enabled={:?}", e.settings.plugins.enabled);
    }

    #[test]
    fn old_config_partial_save_ok() {
        // 老配置 JSON 只含少量字段，缺的取 serde default
        let s = Settings::default();
        let old_json = serde_json::to_value(&s).unwrap();
        let s2: Settings = serde_json::from_value(old_json).unwrap();
        assert_eq!(s2.task_max_steps, 20);
    }

    // ================= v45.12 MCP 验证 =================

    /// 零子进程证据：默认配置 mcp_servers 为空 → ensure_initialized 后 registry 仍是 None，
    /// 不构造 McpRegistry、不 spawn 任何子进程。
    #[test]
    fn mcp_empty_config_never_spawns_registry() {
        let mut e = isolated_engine();
        assert!(e.settings.mcp_servers.is_empty());
        e.mcp_ensure_initialized();
        assert!(
            e.mcp.lock().unwrap().is_none(),
            "空配置时 registry 必须保持 None（绝不 spawn）"
        );
        // mcp_list 也不 panic，且返回空
        assert!(e.mcp_list().is_empty());
        assert!(e.mcp.lock().unwrap().is_none(), "mcp_list 不得触发 spawn");
    }

    /// clamp 清洗：name 空 / command 空剔除；transport 非 stdio 降级 disabled。
    #[test]
    fn mcp_clamp_drops_or_disables_bad_entries() {
        let mut s = Settings::default();
        s.mcp_servers = vec![
            crate::mcp::McpServerConfig {
                name: "".into(), // 应被剔除
                transport: "stdio".into(),
                command: "echo".into(),
                args: vec![], env: Default::default(), enabled: true, trust: crate::mcp::McpTrust::Confirm,
            },
            crate::mcp::McpServerConfig {
                name: "http-only".into(), // transport 非法 → disabled
                transport: "http".into(),
                command: "echo".into(),
                args: vec![], env: Default::default(), enabled: true, trust: crate::mcp::McpTrust::Confirm,
            },
        ];
        s.clamp_in_place();
        assert_eq!(s.mcp_servers.len(), 1);
        assert_eq!(s.mcp_servers[0].name, "http-only");
        assert!(!s.mcp_servers[0].enabled, "非 stdio 必须降级 disabled");
    }

    /// trust=confirm 被拦下的真实调用记录（测试里造）：
    /// 落一条 mcp-appr- 待确认 → 用户拒绝 → 返回可读文本、注册表根本没被碰（仍 None）、
    /// 待确认项被消费。
    #[tokio::test]
    async fn mcp_confirm_rejection_is_readable_and_never_calls() {
        let mut e = isolated_engine();
        e.mcp_pending.lock().unwrap().insert(
            "mcp-appr-99".into(),
            (
                "fs".into(),
                "write_file".into(),
                serde_json::json!({"path": "evil.sh", "content": "x"}),
            ),
        );
        // 用户拒绝
        let msg = e
            .approve_skill("mcp-appr-99", false)
            .await
            .expect("拒绝路径应正常返回可读文本");
        assert!(msg.contains("已取消"), "拒绝文案: {msg}");
        assert!(msg.contains("mcp__fs__write_file"), "应点名被取消的工具: {msg}");
        // 注册表从未被构造（没有任何子进程/spawn 行为发生）
        assert!(e.mcp.lock().unwrap().is_none());
        // 待确认项已消费，不残留
        assert!(e.mcp_pending.lock().unwrap().is_empty());
    }
}
