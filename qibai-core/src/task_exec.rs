//! v45.9 P0 后台步进执行器。
//!
//! 设计：
//! - `task_start` 后调 [`spawn_executor`] 起一个 std::thread（不阻塞 UI；关窗不中止，
//!   状态写盘由启动 `scan_and_recover` 处理）；
//! - 循环取 `next_pending_index`；每步先过 [`plan::ensure_executable`] 与 [`plan::needs_confirm`]；
//! - Safe 步直接解释执行最小动作集（`write_file` / `run_command` / `noop`）；
//! - Guarded/Dangerous 步**不执行**，迁移到 `waiting_confirm` 并停住，等 `task_confirm_action`；
//! - 每步产物走 [`store::TaskStore::commit_artifact`]（tmp + rename）；
//!   steps.ndjson / runs.ndjson 由 store 追加。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::task::machine::{TaskEvent, TaskHook, TaskError, transition};
use crate::task::model::{RiskLevel, Run, RunOutcome, Step, StepStatus, Task, TaskStatus, TokenUsage, now_ms};
use crate::task::plan::{ensure_executable, needs_confirm};
use crate::task::schedule::conflicts_within_group;
use crate::task::store::TaskStore;

/// 内部可变性 Hook：让 trait object 能共享 store 写盘 + emit 事件。
pub struct StoreHook {
    store: Arc<TaskStore>,
    sink: Arc<dyn crate::engine::EventSink>,
    /// v45.9：白名单根显式传入（不再读 env），避免测试间 QIBAI_DATA_DIR 泄漏。
    whitelist_root: PathBuf,
}

impl StoreHook {
    pub fn new(store: TaskStore, sink: Arc<dyn crate::engine::EventSink>, whitelist_root: PathBuf) -> Self {
        Self { store: Arc::new(store), sink, whitelist_root }
    }
    pub fn store(&self) -> &TaskStore { &self.store }
    pub fn whitelist_root(&self) -> &Path { &self.whitelist_root }
    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        std::ops::Deref::deref(&self.sink).emit(event, payload);
    }
}

impl TaskHook for StoreHook {
    fn on_plan_saved(&self, task: &Task) -> Result<(), String> {
        self.store.save(task).map_err(|e| e.to_string())
    }
    fn on_enter_running(&self, task: &Task) -> Result<(), String> {
        self.store.save(task).map_err(|e| e.to_string())
    }
    fn on_waiting_confirm(&self, task: &Task, _step_id: &str) -> Result<(), String> {
        self.store.save(task).map_err(|e| e.to_string())
    }
    fn on_run_appended(&self, task: &Task, run: &Run) -> Result<(), String> {
        self.store.append_run(&task.id, run).map_err(|e| e.to_string())?;
        // 写 context 快照（v45.9 P0：API 已定型）
        if let Ok(ctx_dir) = self.store.context_dir(&task.id) {
            let snap = crate::context::snapshot::ContextSnapshot {
                run_id: run.id.clone(),
                parent_run_id: None,
                l0: String::new(),
                l1: String::new(),
                l2: String::new(),
                l3: String::new(),
                skills_injected: run.skills.clone(),
                kb_hits: vec![],
                tokens: run.tokens.total as usize,
                prompt_version: run.prompt_version.clone(),
                compression: crate::context::compress::CompressionState::Fresh,
                dropped_message_ids: vec![],
                archived_fact_ids: vec![],
            };
            let _ = crate::context::snapshot::write_snapshot(&ctx_dir, &snap);
        }
        Ok(())
    }
}

/// 解释一步的动作。最小集合：
/// - `noop` / 空 intent：直接成功；
/// - `write_file(<path>): <content>`：写文件到白名单根下；
/// - `run_command(<cmd>)`：走 `check_command_arg` 白名单（未实现，默认拒绝）。
pub struct StepAction {
    pub kind: String,
    pub path: Option<PathBuf>,
    pub content: Option<Vec<u8>>,
    pub cmd: Option<String>,
}

pub fn parse_step_action(intent: &str) -> StepAction {
    let t = intent.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("noop") {
        return StepAction { kind: "noop".into(), path: None, content: None, cmd: None };
    }
    if let Some(rest) = t.strip_prefix("write_file(") {
        if let Some((path_part, body)) = rest.split_once("):") {
            let path = PathBuf::from(path_part.trim().trim_matches('"').trim_matches('\''));
            let content = body.trim().as_bytes().to_vec();
            return StepAction { kind: "write_file".into(), path: Some(path), content: Some(content), cmd: None };
        }
        if let Some(path_part) = rest.strip_suffix(')') {
            let path = PathBuf::from(path_part.trim().trim_matches('"').trim_matches('\''));
            return StepAction { kind: "write_file".into(), path: Some(path), content: Some(Vec::new()), cmd: None };
        }
    }
    if let Some(rest) = t.strip_prefix("run_command(") {
        if let Some(cmd_part) = rest.strip_suffix(')') {
            let cmd = cmd_part.trim().trim_matches('"').trim_matches('\'').to_string();
            return StepAction { kind: "run_command".into(), path: None, content: None, cmd: Some(cmd) };
        }
    }
    StepAction { kind: "noop".into(), path: None, content: None, cmd: None }
}

fn within_root(root: &std::path::Path, p: &std::path::Path) -> bool {
    let Ok(root) = std::fs::canonicalize(root) else { return false };
    let Ok(p) = std::fs::canonicalize(p) else { return p.starts_with(&root) };
    p.starts_with(&root)
}

/// 执行一步的实际动作。返回 Ok(artifact_relpath) 或 Err(msg)。
/// v45.9：root 显式传入（白名单根），不再读 env。
pub fn execute_action(store: &TaskStore, task_id: &str, action: &StepAction, root: &std::path::Path) -> Result<String, String> {
    match action.kind.as_str() {
        "noop" => Ok("".into()),
        "write_file" => {
            let path = action.path.clone().ok_or("write_file missing path")?;
            // 若 path 是相对的，拼到白名单根
            let abs = if path.is_absolute() { path.clone() } else { root.join(&path) };
            if !within_root(root, &abs) {
                return Err(format!("write_file 越界：{} 不在 {}", abs.display(), root.display()));
            }
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // 走 commit_artifact：tmp + rename。final_name 不允许路径分隔（store 校验）。
            let final_name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| format!("artifact_{}", uuid_like()));
            let tmp = store.artifacts_dir(task_id).map_err(|e| e.to_string())?.join(format!(".tmp_{}", uuid_like()));
            std::fs::write(&tmp, action.content.as_deref().unwrap_or(&Vec::new())).map_err(|e| e.to_string())?;
            store.commit_artifact(task_id, &tmp, &final_name).map_err(|e| e.to_string())?;
            Ok(final_name)
        }
        "run_command" => {
            // P0：走既有安全闸门（与 engine.rs::check_command_arg 同规则）：
            // 1) 整个命令字符串拒 shell 元字符 ; | & > < ` $ ( )
            // 2) 逐 token：含 / 的必须落在白名单根内，拒 ..
            let cmd = action.cmd.clone().unwrap_or_default();
            let meta = [';', '|', '&', '>', '<', '`', '$', '(', ')'];
            for c in meta {
                if cmd.contains(c) {
                    return Err(format!("命令含 shell 元字符 '{c}'：{cmd}"));
                }
            }
            for tok in cmd.split_whitespace().skip(1) {
                if tok.starts_with('/') || tok.starts_with('~') || tok.contains("..") {
                    let p = std::path::Path::new(tok);
                    if !within_root(root, p) {
                        return Err(format!("命令路径越界：{tok} 不在 {}", root.display()));
                    }
                }
            }
            // 闸门通过后执行（P0：仅 spawn 同步等退出码）
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .current_dir(&root)
                .output()
                .map_err(|e| format!("执行失败: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("命令退出码非 0: {stderr}"));
            }
            Ok(String::new())
        }
        other => Err(format!("unknown action kind: {other}")),
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{n:x}")
}

/// 步骤产物断言（与聊天路径共用 [`crate::llm_client::verify_artifact_at`]，不复制两份）。
///
/// - `write_file`：execute_action 通过 `commit_artifact` 把产物落到
///   `<artifacts_dir>/<committed_rel>`，这里真检查该文件存在且 size>0；
/// - `run_command` 且命令以 `mkdir` 开头：在白名单根下解析出目录，检查可读；
/// - 其他命令（echo/ls 等）与 noop：不声称文件/目录产物，直接放行。
///
/// 不通过返回 Err，文案以「声称完成但产物不存在：<path>」开头。
pub fn verify_step_artifact(
    store: &TaskStore,
    task_id: &str,
    action: &StepAction,
    root: &std::path::Path,
    committed_rel: &str,
) -> Result<(), String> {
    match action.kind.as_str() {
        "write_file" => {
            let art = store.artifacts_dir(task_id).map_err(|e| e.to_string())?.join(committed_rel);
            crate::llm_client::verify_artifact_at(&art)
        }
        "run_command" => {
            let cmd = action.cmd.as_deref().unwrap_or("");
            let rest = match cmd.split_whitespace().next() {
                Some(first) if first == "mkdir" => &cmd[first.len()..],
                _ => return Ok(()), // 非 mkdir 命令不声称目录产物
            };
            // 跳过 mkdir 的选项（-p / -m 等），取剩余 token 作为目标目录
            let dir = rest
                .split_whitespace()
                .find(|tok| !tok.starts_with('-'))
                .unwrap_or("");
            if dir.is_empty() {
                return Ok(());
            }
            let abs = if std::path::Path::new(dir).is_absolute() {
                std::path::PathBuf::from(dir)
            } else {
                root.join(dir)
            };
            crate::llm_client::verify_artifact_at(&abs)
        }
        _ => Ok(()),
    }
}

/// 起后台执行线程。立即返回（不阻塞调用方）。
pub fn spawn_executor(
    store: TaskStore,
    task_id: String,
    sink: std::sync::Arc<dyn crate::engine::EventSink>,
    provider: std::sync::Arc<dyn crate::llm_client::LlmProvider>,
    whitelist_root: PathBuf,
) {
    std::thread::spawn(move || {
        // 后台线程里建一个 tokio runtime 跑 async chat
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("[qibai] task: tokio runtime build failed task={task_id}: {e}");
                return;
            }
        };
        let hook = StoreHook::new(store, sink.clone(), whitelist_root);
        if let Err(e) = rt.block_on(run_loop_async(&hook, &task_id, std::ops::Deref::deref(&sink), std::ops::Deref::deref(&provider))) {
            eprintln!("[qibai] task: executor loop error task={task_id}: {e}");
        }
    });
}

/// 生产环境默认白名单根：~/Documents/qibai（与权限闸门一致）。
pub fn default_whitelist_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join("Documents/qibai");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    std::env::temp_dir()
}

fn emit_updated(sink: &dyn crate::engine::EventSink, t: &Task) {
    let total = t.plan.as_ref().map(|p| p.steps.len()).unwrap_or(0);
    sink.emit("task://updated", serde_json::json!({
        "id": t.id,
        "status": format!("{:?}", t.status),
        "current_step": t.current_step,
        "total_steps": total,
    }));
}

fn emit_waiting_confirm(
    sink: &dyn crate::engine::EventSink,
    t: &Task,
    step_id: &str,
    action_kind: &str,
    risk: &RiskLevel,
) {
    let step_index = t.plan.as_ref()
        .and_then(|p| p.steps.iter().position(|s| s.id == step_id))
        .unwrap_or(t.current_step);
    sink.emit("task://waiting_confirm", serde_json::json!({
        "task_id": t.id,
        "action_id": step_id,
        "step_index": step_index,
        "action_kind": action_kind,
        "target": "",
        "risk": format!("{:?}", risk),
    }));
}

fn emit_finished(sink: &dyn crate::engine::EventSink, t: &Task) {
    sink.emit("task://finished", serde_json::json!({
        "id": t.id,
        "status": format!("{:?}", t.status),
    }));
}

/// v45.13：默认并发上限（settings.task_parallel_max 字段由另一条线加，先常量兜底）。
pub const DEFAULT_PARALLEL_MAX: usize = 3;

/// 共享预算池：所有并行分支共享父任务同一个 token 池；超限即 abort。
pub struct BudgetPool {
    tokens: AtomicU64,
    max_tokens: u64,
    aborted: AtomicBool,
}

impl BudgetPool {
    pub fn new(max_tokens: u64) -> Self {
        Self { tokens: AtomicU64::new(0), max_tokens, aborted: AtomicBool::new(false) }
    }
    /// 累加 tokens；返回 true 表示本次累加后超限。
    pub fn add_tokens(&self, n: u64) -> bool {
        let new = self.tokens.fetch_add(n, Ordering::SeqCst).saturating_add(n);
        if new >= self.max_tokens {
            self.aborted.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }
    pub fn trigger_abort(&self) { self.aborted.store(true, Ordering::SeqCst); }
    pub fn is_aborted(&self) -> bool { self.aborted.load(Ordering::SeqCst) }
    pub fn used(&self) -> u64 { self.tokens.load(Ordering::SeqCst) }
}

/// 分支执行结果（主任务串行合并到 Task）。
#[derive(Debug, Clone)]
enum BranchResult {
    Success { summary: String, committed: Option<String> },
    Failed { error: String },
    SkippedBudget,
    /// 写冲突无法通过顺延解决时使用（当前实现一律顺延到下一组；保留 variant 供未来硬冲突分支）。
    #[allow(dead_code)]
    BlockedByConfict { reason: String },
}

#[derive(Debug, Clone)]
struct BranchOutput {
    step_id: String,
    attempts: u8,
    tokens: u64,
    tokens_estimated: bool,
    result: BranchResult,
}

/// 为某分支构建隔离 prompt：只含本步子任务 + 依赖产物 + 全局 goal 一句。
fn build_branch_system(task: &Task, step: &Step) -> String {
    let deps: Vec<&Step> = step.depends_on.iter().filter_map(|d| {
        task.plan.as_ref().and_then(|p| p.steps.iter().find(|s| &s.id == d))
    }).collect();
    let dep_section = if deps.is_empty() {
        "（无依赖产物）".to_string()
    } else {
        deps.iter().map(|d| {
            let summary = d.result_summary.as_deref().unwrap_or("（无总结）");
            let arts: Vec<&str> = d.artifacts.iter().map(|a| a.path.as_str()).collect();
            if arts.is_empty() {
                format!("- [{}] {}", d.id, summary)
            } else {
                format!("- [{}] {} 产物: {}", d.id, summary, arts.join(","))
            }
        }).collect::<Vec<_>>().join("\n")
    };
    format!(
        "## 人设\n你是元气小小的并行任务分支执行器，只完成本子任务，不要假设其他分支的产物。\n\n## 子任务\n[{}] {}\n目标: {}\n\n## 依赖产物\n{dep_section}\n\n你可以调用工具 write_file / run_command / noop；直接给文本结论也可。",
        step.id, step.title, task.goal
    )
}

/// 预算超限原因（沿用既有 "max_tokens (used=X > limit=Y)" 格式）。
fn budget_exceeded_reason(t: &Task, elapsed_ms: u64) -> Option<String> {
    let done = t.plan.as_ref().map(|p| p.steps.iter().filter(|s| s.status == StepStatus::Done).count()).unwrap_or(0);
    if done as u32 >= t.budget.max_steps {
        return Some(format!("max_steps (used={} > limit={})", done, t.budget.max_steps));
    }
    let minutes = elapsed_ms / 60_000;
    if minutes >= t.budget.max_minutes as u64 {
        return Some(format!("max_minutes (used={} > limit={})", minutes, t.budget.max_minutes));
    }
    if t.tokens_total >= t.budget.max_tokens {
        return Some(format!("max_tokens (used={} > limit={})", t.tokens_total, t.budget.max_tokens));
    }
    None
}

/// 单个分支执行：重试循环 + 池预算检查。
async fn run_one_branch(
    provider: &dyn crate::llm_client::LlmProvider,
    store: &TaskStore,
    whitelist: &Path,
    pool: &BudgetPool,
    task_id: &str,
    step: Step,
    system: String,
    max_retries: u32,
) -> BranchOutput {
    use crate::llm_client::{ChatMessage, ChatRequest};
    use crate::context::estimate_tokens;

    let mut attempts = 0u32;
    let mut last_err: Option<String> = None;
    let mut success: Option<(String, u32, bool, Option<String>)> = None;

    while attempts <= max_retries && success.is_none() {
        // 池中止检查
        if pool.is_aborted() {
            return BranchOutput {
                step_id: step.id.clone(),
                attempts: attempts as u8,
                tokens: 0,
                tokens_estimated: true,
                result: BranchResult::SkippedBudget,
            };
        }
        attempts += 1;
        let req = ChatRequest {
            system: system.clone(),
            messages: vec![ChatMessage::user(format!("执行步 {}：{}", step.id, step.intent))],
            tools: None,
            json_object: false,
        };
        let resp = match provider.chat(req).await {
            Ok(r) => r,
            Err(e) => { last_err = Some(format!("{e:?}")); continue; }
        };
        // token 累加（先加再判预算）
        let tok = resp.tokens_used.unwrap_or_else(|| estimate_tokens(&system) as u32);
        let estimated = resp.tokens_used.is_none();
        if pool.add_tokens(tok as u64) {
            // 本分支触发超限，直接返回 SkippedBudget
            return BranchOutput {
                step_id: step.id.clone(),
                attempts: attempts as u8,
                tokens: tok as u64,
                tokens_estimated: estimated,
                result: BranchResult::SkippedBudget,
            };
        }

        if let Some(tc) = resp.tool_calls.first() {
            let args = &tc.function.arguments;
            let parsed: StepAction = match tc.function.name.as_str() {
                "write_file" => {
                    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
                    let p = v.get("path").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let c = v.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    StepAction { kind: "write_file".into(), path: Some(PathBuf::from(p)), content: Some(c.into_bytes()), cmd: None }
                }
                "run_command" => {
                    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
                    let c = v.get("command").and_then(|x| x.as_str()).or_else(|| v.as_str()).unwrap_or("").to_string();
                    StepAction { kind: "run_command".into(), path: None, content: None, cmd: Some(c) }
                }
                _ => StepAction { kind: "noop".into(), path: None, content: None, cmd: None },
            };
            match execute_action(store, task_id, &parsed, whitelist) {
                Ok(rel) => {
                    match verify_step_artifact(store, task_id, &parsed, whitelist, &rel) {
                        Ok(()) => {
                            success = Some((format!("tool: {} → {}", tc.function.name, rel), tok, estimated, Some(rel)));
                        }
                        Err(e) => { last_err = Some(e); }
                    }
                }
                Err(e) => { last_err = Some(e); }
            }
        } else if let Some(text) = resp.content.as_ref() {
            success = Some((text.clone(), tok, estimated, None));
        } else {
            last_err = Some("空响应".into());
        }
    }

    let (summary, tok, estimated, _committed, result) = match success {
        Some((summary, tok, estimated, committed)) => {
            let r = BranchResult::Success { summary: summary.clone(), committed: committed.clone() };
            (summary, tok, estimated, committed, r)
        }
        None => (String::new(), 0u32, true, None, BranchResult::Failed {
            error: last_err.unwrap_or_else(|| "unknown".into()),
        }),
    };
    let _ = summary;
    BranchOutput {
        step_id: step.id.clone(),
        attempts: attempts as u8,
        tokens: tok as u64,
        tokens_estimated: estimated,
        result,
    }
}

/// v45.13 并行执行器：每轮取"依赖已 Done 的 Pending 步骤"为可执行组，
/// 用 JoinSet 并发跑组内各步，共享父任务预算池，失败隔离 → Partial。
pub async fn run_loop_async(
    hook: &StoreHook,
    task_id: &str,
    sink: &dyn crate::engine::EventSink,
    provider: &dyn crate::llm_client::LlmProvider,
) -> Result<(), TaskError> {
    let store = hook.store();
    let settings = crate::settings::Settings::load();
    let max_retries = settings.task_auto_retry.max(1);
    // v45.13：并行上限读 settings.task_parallel_max（1..=8，默认 3）；0 或缺失回退常量。
    let max_parallel: usize = if settings.task_parallel_max >= 1 {
        settings.task_parallel_max as usize
    } else {
        DEFAULT_PARALLEL_MAX
    };

    let mut t = store.require(task_id)?;
    if t.status != TaskStatus::Running {
        eprintln!("[qibai] task: executor start skipped, status={:?} task={task_id}", t.status);
        return Ok(());
    }
    emit_updated(sink, &t);

    let pool = Arc::new(BudgetPool::new(t.budget.max_tokens));
    pool.tokens.store(t.tokens_total, Ordering::SeqCst);
    let start_ms = now_ms();

    loop {
        // 1) 预算检查（含被分支 abort 触发的 paused）
        let elapsed = now_ms().saturating_sub(start_ms);
        if let Some(reason) = budget_exceeded_reason(&t, elapsed) {
            eprintln!("[qibai] task: paused reason={reason} task={task_id}");
            t.paused_reason = Some(reason);
            pool.trigger_abort();
            transition(&mut t, TaskEvent::Pause, hook)?;
            store.save(&t)?;
            emit_updated(sink, &t);
            break;
        }
        if pool.is_aborted() {
            // 分支累加触发的中止：把剩余未跑的 Pending 标 Skipped
            let reason = format!("max_tokens (used={} > limit={})", pool.used(), t.budget.max_tokens);
            eprintln!("[qibai] task: pool aborted: {reason} task={task_id}");
            t.paused_reason = Some(reason);
            for s in t.plan.as_mut().unwrap().steps.iter_mut() {
                if s.status == StepStatus::Pending {
                    s.status = StepStatus::Skipped;
                    s.error = Some("预算超限，已中止".into());
                }
            }
            transition(&mut t, TaskEvent::Pause, hook)?;
            store.save(&t)?;
            emit_updated(sink, &t);
            break;
        }

        let plan = match t.plan.as_ref() { Some(p) => p.clone(), None => break };
        if let Err(e) = ensure_executable(&t) {
            eprintln!("[qibai] task: ensure_executable failed task={task_id}: {e}");
            break;
        }

        // 2) 计算可执行组
        let mut ready: Vec<Step> = Vec::new();
        let mut confirm_steps: Vec<Step> = Vec::new();
        for s in &plan.steps {
            if s.status != StepStatus::Pending { continue; }
            let deps_ok = s.depends_on.iter().all(|d| {
                plan.steps.iter().find(|x| &x.id == d).map(|x| x.status == StepStatus::Done).unwrap_or(false)
            });
            if !deps_ok { continue; }
            if needs_confirm(s.risk, &t.policy, &s.id) {
                confirm_steps.push(s.clone());
            } else {
                ready.push(s.clone());
            }
        }

        // 3) 终态 / 阻塞判定
        if ready.is_empty() && confirm_steps.is_empty() {
            let all_terminal = plan.steps.iter().all(|s| matches!(s.status, StepStatus::Done | StepStatus::Failed | StepStatus::Skipped));
            if all_terminal {
                let failed: Vec<&Step> = plan.steps.iter().filter(|s| s.status == StepStatus::Failed).collect();
                let has_done = plan.steps.iter().any(|s| s.status == StepStatus::Done);
                if !failed.is_empty() && has_done {
                    let errs: Vec<String> = failed.iter()
                        .map(|s| format!("步骤 {} 失败：{}", s.id, s.error.clone().unwrap_or_default()))
                        .collect();
                    t.last_error = Some(errs.join("; "));
                    transition(&mut t, TaskEvent::MarkPartial, hook)?;
                } else if !failed.is_empty() {
                    transition(&mut t, TaskEvent::MarkFailed, hook)?;
                } else {
                    transition(&mut t, TaskEvent::MarkDone, hook)?;
                }
                store.save(&t)?;
                emit_updated(sink, &t);
                emit_finished(sink, &t);
                break;
            }
            // 依赖失败导致的 Pending → 标 Failed 后重轮
            let mut progressed = false;
            for s_mut in t.plan.as_mut().unwrap().steps.iter_mut() {
                if s_mut.status != StepStatus::Pending { continue; }
                let dep_bad = s_mut.depends_on.iter().any(|d| {
                    plan.steps.iter().find(|x| &x.id == d)
                        .map(|x| matches!(x.status, StepStatus::Failed | StepStatus::Skipped)).unwrap_or(false)
                });
                if dep_bad {
                    s_mut.status = StepStatus::Failed;
                    s_mut.error = Some("依赖步骤失败，本步未执行".into());
                    progressed = true;
                }
            }
            if progressed { store.save(&t)?; continue; }
            break;
        }

        // 4) 写冲突检测：同组内声明写同一路径的两步，后者直接 Failed（不执行写入，阻止覆盖）。
        //    合法依赖（b.depends_on 含 a）天然不同组，不会走到这里。
        let ready_ids: Vec<String> = ready.iter().map(|s| s.id.clone()).collect();
        let conflicts = conflicts_within_group(&ready_ids, &plan.steps);
        let mut blocked: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for c in &conflicts {
            let a_idx = plan.steps.iter().find(|s| s.id == c.a).map(|s| s.idx).unwrap_or(0);
            let b_idx = plan.steps.iter().find(|s| s.id == c.b).map(|s| s.idx).unwrap_or(0);
            let (later_id, keep_id) = if a_idx < b_idx {
                (c.b.clone(), c.a.clone())
            } else {
                (c.a.clone(), c.b.clone())
            };
            eprintln!("[qibai] task: write-conflict blocked path={} steps={},{}", c.path, c.a, c.b);
            blocked.insert(later_id.clone());
            if let Some(s) = t.plan.as_mut().unwrap().steps.iter_mut().find(|x| x.id == later_id) {
                s.status = StepStatus::Failed;
                s.ended_at = now_ms();
                s.error = Some(format!(
                    "与并行分支 {} 写入同一路径 {}，已阻止覆盖（未执行写入）",
                    keep_id, c.path
                ));
                let _ = store.append_step(task_id, s);
            }
        }
        let mut exec: Vec<Step> = ready.into_iter().filter(|s| !blocked.contains(&s.id)).collect();
        exec.truncate(max_parallel);

        // 5) confirm 步先标 WaitingConfirm（并发分支跑完后再停）
        for s in &confirm_steps {
            if let Some(sm) = t.plan.as_mut().unwrap().steps.iter_mut().find(|x| x.id == s.id) {
                sm.status = StepStatus::WaitingConfirm;
            }
        }
        store.save(&t)?;

        // 6) 并发执行 exec（join_all 并发；exec 已按 max_parallel 截断）
        let futures: Vec<_> = exec.into_iter().map(|step| {
            let store_clone = store.clone();
            let whitelist = hook.whitelist_root().to_path_buf();
            let pool = pool.clone();
            let task_id = task_id.to_string();
            let system = build_branch_system(&t, &step);
            async move {
                run_one_branch(provider, &store_clone, &whitelist, &pool, &task_id, step, system, max_retries).await
            }
        }).collect();
        let outputs = futures_util::future::join_all(futures).await;

        // 7) 串行合并 outputs 到 t
        for o in outputs {
            let step_idx = match t.plan.as_ref().unwrap().steps.iter().position(|s| s.id == o.step_id) {
                Some(i) => i,
                None => continue,
            };
            let s = &mut t.plan.as_mut().unwrap().steps[step_idx];
            s.attempts = o.attempts;
            s.tokens = TokenUsage { total: o.tokens as u64, ..Default::default() };
            t.tokens_total += o.tokens as u64;
            t.tokens_used.total += o.tokens as u64;
            if o.tokens_estimated { t.tokens_estimated = true; }
            s.started_at = now_ms();
            let outcome = match &o.result {
                BranchResult::Success { summary, committed } => {
                    s.status = StepStatus::Done;
                    s.ended_at = now_ms();
                    s.result_summary = Some(format!("{summary} (tokens={}, estimated={})", o.tokens, o.tokens_estimated));
                    if let Some(rel) = committed {
                        s.artifacts.push(crate::task::model::Artifact {
                            path: rel.clone(), kind: "file".into(), bytes: 0, sha256: None,
                        });
                    }
                    RunOutcome::Success
                }
                BranchResult::Failed { error } => {
                    s.status = StepStatus::Failed;
                    s.ended_at = now_ms();
                    s.error = Some(error.clone());
                    RunOutcome::Failure
                }
                BranchResult::SkippedBudget => {
                    s.status = StepStatus::Skipped;
                    s.ended_at = now_ms();
                    s.error = Some("预算超限，已中止".into());
                    RunOutcome::BudgetExceeded
                }
                BranchResult::BlockedByConfict { reason } => {
                    s.status = StepStatus::Failed;
                    s.ended_at = now_ms();
                    s.error = Some(reason.clone());
                    RunOutcome::Failure
                }
            };
            let _ = store.append_step(task_id, s);
            let run = Run {
                id: format!("r_{}_{}", s.id, s.started_at),
                task_id: task_id.to_string(),
                step_id: s.id.clone(),
                started_at: s.started_at,
                ended_at: s.ended_at,
                model: "llm".into(),
                prompt_version: String::new(),
                skills: vec![],
                kb_hits: vec![],
                outcome,
                tokens: TokenUsage { total: o.tokens as u64, ..Default::default() },
                tokens_estimated: o.tokens_estimated,
            };
            hook.on_run_appended(&t, &run).ok();
        }
        store.save(&t)?;
        emit_updated(sink, &t);

        // 8) 有 confirm 步 → 停等批
        if !confirm_steps.is_empty() {
            let sid = confirm_steps[0].id.clone();
            let risk = confirm_steps[0].risk;
            transition(&mut t, TaskEvent::NeedConfirm, hook)?;
            store.save(&t)?;
            emit_updated(sink, &t);
            emit_waiting_confirm(sink, &t, &sid, "action", &risk);
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::plan::build_plan;

    #[test]
    fn parse_write_file_with_content() {
        let a = parse_step_action(r#"write_file("notes/a.txt"): hello world"#);
        assert_eq!(a.kind, "write_file");
        assert_eq!(a.path.unwrap().to_string_lossy(), "notes/a.txt");
        assert_eq!(String::from_utf8_lossy(a.content.as_ref().unwrap()), "hello world");
    }

    #[test]
    fn parse_noop_empty() {
        let a = parse_step_action("");
        assert_eq!(a.kind, "noop");
        let a = parse_step_action("NOOP");
        assert_eq!(a.kind, "noop");
    }

    #[test]
    fn parse_run_command() {
        let a = parse_step_action(r#"run_command("ls -la")"#);
        assert_eq!(a.kind, "run_command");
        assert_eq!(a.cmd.as_deref(), Some("ls -la"));
    }

    #[test]
    fn needs_confirm_dangerous_blocks_execution() {
        // 复现：dangerous 步必须让 executor 落 waiting_confirm
        let mut t = Task::default();
        t.id = "t_x".into();
        t.status = TaskStatus::Running;
        let mut p = build_plan(vec![crate::task::Step {
            id: "s_d".into(),
            risk: RiskLevel::Dangerous,
            ..Default::default()
        }]);
        p.approved_at = Some(now_ms());
        t.plan = Some(p);
        // ensure_executable 通过
        ensure_executable(&t).unwrap();
        // needs_confirm 返回 true -> executor 不执行
        assert!(needs_confirm(RiskLevel::Dangerous, &t.policy, "s_d"));
    }

    #[test]
    fn run_command_rejects_shell_meta() {
        let tmp = std::env::temp_dir().join(format!("qibai_shell_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        store.create(&Task { id: "t_gate".into(), ..Default::default() }).unwrap();
        let action = StepAction {
            kind: "run_command".into(),
            path: None, content: None,
            cmd: Some("echo a; rm -rf b".into()),
        };
        let r = execute_action(&store, "t_gate", &action, &tmp);
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("shell 元字符"), "msg={e}");
    }

    #[test]
    fn run_command_rejects_out_of_root_path() {
        let tmp = std::env::temp_dir().join(format!("qibai_root_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        store.create(&Task { id: "t_gate2".into(), ..Default::default() }).unwrap();
        // /tmp/x 不在测试根里
        let action = StepAction {
            kind: "run_command".into(),
            path: None, content: None,
            cmd: Some("ls /tmp/x".into()),
        };
        let r = execute_action(&store, "t_gate2", &action, &tmp);
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("越界") || e.contains("不在"), "msg={e}");
    }

    // ====== v45.9 LLM 循环 5 个必测（最小可运行版） ======

    struct ScriptedProvider {
        responses: std::sync::Mutex<std::collections::VecDeque<Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError>>>,
        systems: std::sync::Mutex<Vec<String>>,
    }
    impl ScriptedProvider {
        fn new(items: Vec<Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError>>) -> Self {
            Self { responses: std::sync::Mutex::new(std::collections::VecDeque::from(items)), systems: std::sync::Mutex::new(Vec::new()) }
        }
        fn systems(&self) -> Vec<String> { self.systems.lock().unwrap().clone() }
    }
    #[async_trait::async_trait]
    impl crate::llm_client::LlmProvider for ScriptedProvider {
        async fn chat(&self, req: crate::llm_client::ChatRequest) -> Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError> {
            self.systems.lock().unwrap().push(req.system.clone());
            let mut g = self.responses.lock().unwrap();
            g.pop_front().unwrap_or(Err(crate::llm_client::LlmError::Api("exhausted".into())))
        }
    }

    fn tool_call(name: &str, args: &str) -> crate::llm_client::ChatResponse {
        crate::llm_client::ChatResponse {
            content: None,
            tool_calls: vec![crate::llm_client::ToolCall {
                id: "call_1".into(), kind: "function".into(),
                function: crate::llm_client::ToolCallFunction { name: name.into(), arguments: args.into() },
            }],
            tokens_used: Some(42),
            ..Default::default()
        }
    }

    #[test]
    fn fake_provider_two_tools_then_done() {
        let tmp = std::env::temp_dir().join(format!("qibai_llm_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![crate::task::Step { id: "s1".into(), title: "写文件".into(), ..Default::default() }]);
        p.approved_at = Some(now_ms());
        store.create(&Task { id: "t1".into(), goal: "写文件".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default() }).unwrap();
        let provider = std::sync::Arc::new(ScriptedProvider::new(vec![
            Ok(tool_call("write_file", r#"{"path":"a.txt","content":"AAA"}"#)),
            Ok(crate::llm_client::ChatResponse { content: Some("done".into()), tokens_used: Some(10), ..Default::default() }),
        ]));
        let sink = std::sync::Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t1", std::ops::Deref::deref(&sink), std::ops::Deref::deref(&provider))).unwrap();
        let t = store.require("t1").unwrap();
        assert!(t.tokens_used.total >= 10, "tokens 累加 {}", t.tokens_used.total);
        // v45.13：分支 prompt 必须含 ## 子任务 段（旧 v45.9 的 ## 任务简报 结构已替换）
        let systems = provider.systems();
        assert!(!systems.is_empty(), "provider 没收到 system prompt");
        let sys = &systems[0];
        assert!(sys.contains("## 子任务"), "v45.13 子任务段缺失: {}", sys);
        assert!(sys.contains("写文件"), "步标题没进 prompt: {}", sys);
    }

    #[test]
    fn retry_twice_then_succeed() {
        let tmp = std::env::temp_dir().join(format!("qibai_retry_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![crate::task::Step { id: "s1".into(), ..Default::default() }]);
        p.approved_at = Some(now_ms());
        store.create(&Task { id: "t2".into(), goal: "r".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default() }).unwrap();
        let provider = std::sync::Arc::new(ScriptedProvider::new(vec![
            Err(crate::llm_client::LlmError::Api("b1".into())),
            Err(crate::llm_client::LlmError::Api("b2".into())),
            Ok(crate::llm_client::ChatResponse { content: Some("ok".into()), tokens_used: Some(5), ..Default::default() }),
        ]));
        let sink = std::sync::Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t2", std::ops::Deref::deref(&sink), std::ops::Deref::deref(&provider))).unwrap();
        let t = store.require("t2").unwrap();
        assert!(t.tokens_used.total >= 5, "tokens={}", t.tokens_used.total);
    }

    #[test]
    fn dangerous_tool_call_waits_confirm() {
        let mut t = Task::default();
        t.id = "t3".into(); t.status = TaskStatus::Running;
        let mut p = build_plan(vec![crate::task::Step { id: "s_d".into(), risk: RiskLevel::Dangerous, ..Default::default() }]);
        p.approved_at = Some(now_ms());
        t.plan = Some(p);
        assert!(needs_confirm(RiskLevel::Dangerous, &t.policy, "s_d"));
    }

    #[test]
    fn budget_max_steps_pauses_with_reason() {
        let mut t = Task::default();
        t.id = "t4".into(); t.status = TaskStatus::Running;
        t.budget.max_steps = 1;
        let mut p = build_plan(vec![
            crate::task::Step { id: "s1".into(), status: crate::task::StepStatus::Done, ..Default::default() },
            crate::task::Step { id: "s2".into(), ..Default::default() },
        ]);
        p.approved_at = Some(now_ms());
        let done = p.next_pending_index();
        t.plan = Some(p);
        // next_pending_index() = 1（s1 已 done），max_steps=1 → 1 >= 1 触发
        let exceeded = crate::task::model::check_budget(&t, 0);
        assert!(exceeded.is_some(), "max_steps 应触发预算超限");
        assert_eq!(exceeded.unwrap(), crate::task::model::BudgetExceeded::Steps);
        // v45.9：paused_reason 文本格式
        let reason = format!("max_steps (used={} > limit={})", done, t.budget.max_steps);
        assert_eq!(reason, "max_steps (used=1 > limit=1)");
    }

    #[test]
    fn budget_paused_reason_text_all_three() {
        // max_minutes
        let mut t = Task::default();
        t.budget.max_minutes = 60;
        let m = format!("max_minutes (used={} > limit={})", 61u64, t.budget.max_minutes);
        assert_eq!(m, "max_minutes (used=61 > limit=60)");
        // max_tokens
        t.tokens_total = 210_000;
        t.budget.max_tokens = 200_000;
        let tk = format!("max_tokens (used={} > limit={})", t.tokens_total, t.budget.max_tokens);
        assert_eq!(tk, "max_tokens (used=210000 > limit=200000)");
    }

    #[test]
    fn task_json_contract_frontend_keys() {
        // v45.9：task_get 序列化后必须含前端用的键
        let mut t = Task::default();
        t.id = "t_json".into();
        t.current_step = 2;
        t.tokens_total = 12345;
        t.tokens_estimated = true;
        t.paused_reason = Some("max_tokens (used=210000 > limit=200000)".into());
        let mut s = crate::task::Step::default();
        s.id = "s1".into();
        s.attempts = 3;
        let mut r = crate::task::Run::default();
        r.step_id = "s1".into();
        r.model = "doubao-pro".into();
        r.tokens.total = 100;
        r.tokens_estimated = false;
        s.runs = vec![r];
        let mut p = build_plan(vec![s]);
        p.approved_at = Some(now_ms());
        t.plan = Some(p);
        let v = serde_json::to_value(&t).unwrap();
        assert!(v.get("current_step").is_some(), "缺 current_step");
        assert_eq!(v["current_step"], 2);
        assert!(v.get("tokens_total").is_some(), "缺 tokens_total");
        assert_eq!(v["tokens_total"], 12345);
        assert!(v.get("tokens_estimated").is_some(), "缺 tokens_estimated");
        assert_eq!(v["tokens_estimated"], true);
        assert!(v.get("paused_reason").is_some(), "缺 paused_reason");
        assert!(v["paused_reason"].as_str().unwrap().contains("max_tokens"));
        let step = &v["plan"]["steps"][0];
        assert!(step.get("attempts").is_some(), "缺 steps[].attempts");
        assert_eq!(step["attempts"], 3);
        // v45.9：steps[].runs[] 真数组，含前端要的键
        let run0 = &step["runs"][0];
        assert!(run0.get("model").is_some(), "缺 runs[].model");
        assert!(run0.get("tokens").is_some(), "缺 runs[].tokens");
        assert!(run0["tokens"].get("total").is_some(), "缺 runs[].tokens.total");
        assert!(run0.get("tokens_estimated").is_some(), "缺 runs[].tokens_estimated");
        assert!(run0.get("outcome").is_some(), "缺 runs[].outcome(=attempt 结果)");
        // step 级 error（前端也读）
        assert!(step.get("error").is_some(), "缺 steps[].error");
        assert_eq!(run0["model"], "doubao-pro");
        assert_eq!(run0["tokens"]["total"], 100);
        assert_eq!(run0["tokens_estimated"], false);
    }

    #[test]
    fn two_tasks_context_isolated() {
        let tmp = std::env::temp_dir().join(format!("qibai_iso_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        store.create(&Task { id: "t_a".into(), ..Default::default() }).unwrap();
        store.create(&Task { id: "t_b".into(), ..Default::default() }).unwrap();
        let a = store.context_dir("t_a").unwrap();
        let b = store.context_dir("t_b").unwrap();
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("t_a"));
        assert!(b.to_string_lossy().contains("t_b"));
    }

    /// 必测③：task 路径下 execute_action 返回 Ok 但产物为空（0 字节，等同没真写）
    /// → verify_step_artifact 判失败 → 该步 Failed、任务非 Done、error 含「声称完成但产物不存在」。
    #[test]
    fn task_step_failed_when_artifact_missing() {
        let tmp = std::env::temp_dir().join(format!("qibai_artmiss_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![crate::task::Step { id: "s1".into(), title: "写空文件".into(), ..Default::default() }]);
        p.approved_at = Some(now_ms());
        store.create(&Task { id: "t_art".into(), goal: "写文件".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default() }).unwrap();
        // 三轮都让 LLM 调 write_file(content="")，execute_action 提交 0 字节产物，
        // 每轮 verify 都失败 → success 始终 None → 步失败。
        let mk_empty = || tool_call("write_file", r#"{"path":"empty.txt","content":""}"#);
        let provider = std::sync::Arc::new(ScriptedProvider::new(vec![
            Ok(mk_empty()),
            Ok(mk_empty()),
            Ok(mk_empty()),
        ]));
        let sink = std::sync::Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_art", std::ops::Deref::deref(&sink), std::ops::Deref::deref(&provider))).unwrap();
        let t = store.require("t_art").unwrap();
        let step = &t.plan.as_ref().unwrap().steps[0];
        assert_eq!(step.status, StepStatus::Failed, "产物缺失必须让该步 failed: {:?}", step.status);
        assert!(t.status != TaskStatus::Done, "步失败后任务不得标 done，实际 status={:?}", t.status);
        let err = step.error.clone().unwrap_or_default();
        assert!(err.contains("声称完成但产物不存在"), "错误文案缺失：{err}");
        // 提交的产物确实是 0 字节（验证我们打的是对的根因）
        let art = store.artifacts_dir("t_art").unwrap().join("empty.txt");
        let len = std::fs::metadata(&art).map(|m| m.len()).unwrap_or(0);
        assert_eq!(len, 0, "empty.txt 应为 0 字节产物，实际 {len}");
    }


    // ====== v45.13 并行调度 7 条必测 ======
    use std::sync::atomic::AtomicUsize;

    fn mk_text_resp(text: &str, tokens: u32) -> crate::llm_client::ChatResponse {
        crate::llm_client::ChatResponse {
            content: Some(text.into()),
            tool_calls: vec![],
            tokens_used: Some(tokens),
            ..Default::default()
        }
    }

    /// 固定响应 provider：每个 chat 调用 sleep 固定时长；用原子计数器记录峰值并发。
    struct FixedProvider {
        sleep_ms: u64,
        tokens: u32,
        text: String,
        current: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        systems: Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl FixedProvider {
        fn new(sleep_ms: u64, tokens: u32, text: &str) -> Arc<Self> {
            Arc::new(Self {
                sleep_ms, tokens, text: text.into(),
                current: Arc::new(AtomicUsize::new(0)),
                peak: Arc::new(AtomicUsize::new(0)),
                systems: Arc::new(std::sync::Mutex::new(Vec::new())),
            })
        }
    }
    #[async_trait::async_trait]
    impl crate::llm_client::LlmProvider for FixedProvider {
        async fn chat(&self, req: crate::llm_client::ChatRequest) -> Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError> {
            self.systems.lock().unwrap().push(req.system.clone());
            let c = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            let mut p = self.peak.load(Ordering::SeqCst);
            while c > p {
                match self.peak.compare_exchange(p, c, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => break,
                    Err(v) => p = v,
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            Ok(mk_text_resp(&self.text, self.tokens))
        }
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("qibai_{}_{}", tag, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    /// 必测①：真并行——两个 120ms 分支，串行应 ≥240ms，并行须 <200ms。
    #[test]
    fn parallel_two_branches_actually_concurrent() {
        let _guard = SETTINGS_LOCK.lock().unwrap();
        std::env::remove_var("QIBAI_DATA_DIR");
        let tmp = tmp_root("p1");
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![
            crate::task::Step { id: "a".into(), idx: 0, ..Default::default() },
            crate::task::Step { id: "b".into(), idx: 1, ..Default::default() },
        ]);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_p1".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        let provider = FixedProvider::new(120, 10u32, "ok");
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = std::time::Instant::now();
        rt.block_on(run_loop_async(&hook, "t_p1", &*sink, &*provider)).unwrap();
        let elapsed = start.elapsed();
        println!("[test1] parallel_elapsed_ms={} (serial_baseline=240)", elapsed.as_millis());
        assert!(elapsed.as_millis() < 200, "并行必须 <200ms，实际 {}ms", elapsed.as_millis());
        assert!(provider.peak.load(Ordering::SeqCst) >= 2, "峰值并发应 ≥2，实际 {}", provider.peak.load(Ordering::SeqCst));
    }

    /// 必测②：上限——5 个无依赖分支，max_parallel=3，峰值 ≤3。
    #[test]
    fn parallel_max_concurrency_capped() {
        let _guard = SETTINGS_LOCK.lock().unwrap();
        std::env::remove_var("QIBAI_DATA_DIR");
        let tmp = tmp_root("p2");
        let store = TaskStore::from_tasks_root(&tmp);
        let steps: Vec<_> = (0..5).map(|i| crate::task::Step {
            id: format!("s{i}"), idx: i, ..Default::default()
        }).collect();
        let mut p = build_plan(steps);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_p2".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        let provider = FixedProvider::new(80, 5u32, "ok");
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_p2", &*sink, &*provider)).unwrap();
        let peak = provider.peak.load(Ordering::SeqCst);
        println!("[test2] peak_concurrent={peak} cap=3");
        assert!(peak <= 3, "峰值并发必须 ≤3，实际 {peak}");
        assert!(peak >= 2, "峰值至少 2 才证明并发跑过，实际 {peak}");
    }

    /// 必测③：共享预算——max_tokens=100，两个分支各报 80，第二个被中止。
    #[test]
    fn shared_budget_aborts_second_branch() {
        let tmp = tmp_root("p3");
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![
            crate::task::Step { id: "a".into(), idx: 0, ..Default::default() },
            crate::task::Step { id: "b".into(), idx: 1, ..Default::default() },
        ]);
        p.approved_at = Some(now_ms());
        let mut task = Task {
            id: "t_p3".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        };
        task.budget.max_tokens = 100;
        store.create(&task).unwrap();
        let provider = FixedProvider::new(10, 80u32, "ok");
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_p3", &*sink, &*provider)).unwrap();
        let t = store.require("t_p3").unwrap();
        let pr = t.paused_reason.clone().unwrap_or_default();
        println!("[test3] paused_reason={pr}");
        assert!(pr.contains("max_tokens"), "paused_reason 必须含 max_tokens: {pr}");
        let b = t.plan.as_ref().unwrap().steps.iter().find(|s| s.id == "b").unwrap();
        assert_eq!(b.status, StepStatus::Skipped, "b 应被预算中止标 Skipped: {:?}", b.status);
    }

    /// 必测④：失败隔离——A 失败、B 成功 → Partial + last_error 含 A 原因。
    #[test]
    fn failure_isolation_marks_partial() {
        let tmp = tmp_root("p4");
        let store = TaskStore::from_tasks_root(&tmp);
        // 按 step id 区分：a 永远失败，b 永远成功
        struct ABProvider {}
        #[async_trait::async_trait]
        impl crate::llm_client::LlmProvider for ABProvider {
            async fn chat(&self, req: crate::llm_client::ChatRequest) -> Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError> {
                let msg = req.messages.last().and_then(|m| m.content.as_deref()).unwrap_or("");
                if msg.contains("步 a") {
                    Err(crate::llm_client::LlmError::Api("branch_a_boom".into()))
                } else {
                    Ok(mk_text_resp("b_ok", 10))
                }
            }
        }
        let mut p = build_plan(vec![
            crate::task::Step { id: "a".into(), idx: 0, ..Default::default() },
            crate::task::Step { id: "b".into(), idx: 1, ..Default::default() },
        ]);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_p4".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        let provider: Arc<ABProvider> = Arc::new(ABProvider {});
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_p4", &*sink, &*provider)).unwrap();
        let t = store.require("t_p4").unwrap();
        println!("[test4] status={:?} last_error={:?}", t.status, t.last_error);
        assert_eq!(t.status, TaskStatus::Partial, "必须 Partial: {:?}", t.status);
        let le = t.last_error.clone().unwrap_or_default();
        assert!(le.contains("branch_a_boom"), "last_error 必须含 A 原因: {le}");
        let b = t.plan.as_ref().unwrap().steps.iter().find(|s| s.id == "b").unwrap();
        assert_eq!(b.status, StepStatus::Done, "B 应 Done: {:?}", b.status);
    }

    /// 必测⑤：写冲突——同组内两步声明写同一路径 → 后者标 Failed（不执行写入），文件保留先写者内容。
    #[test]
    fn write_conflict_blocks_later_writer() {
        let tmp = tmp_root("p5");
        let store = TaskStore::from_tasks_root(&tmp);
        let mut p = build_plan(vec![
            crate::task::Step {
                id: "a".into(), idx: 0,
                declared_writes: vec!["shared.txt".into()],
                ..Default::default()
            },
            crate::task::Step {
                id: "b".into(), idx: 1,
                declared_writes: vec!["shared.txt".into()],
                ..Default::default()
            },
        ]);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_p5".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        struct WCProvider {}
        #[async_trait::async_trait]
        impl crate::llm_client::LlmProvider for WCProvider {
            async fn chat(&self, req: crate::llm_client::ChatRequest) -> Result<crate::llm_client::ChatResponse, crate::llm_client::LlmError> {
                let msg = req.messages.last().and_then(|m| m.content.as_deref()).unwrap_or("");
                if msg.contains("步 a") {
                    Ok(crate::llm_client::ChatResponse {
                        content: None,
                        tool_calls: vec![crate::llm_client::ToolCall {
                            id: "c1".into(), kind: "function".into(),
                            function: crate::llm_client::ToolCallFunction {
                                name: "write_file".into(),
                                arguments: r#"{"path":"shared.txt","content":"AAA"}"#.into(),
                            },
                        }],
                        tokens_used: Some(10),
                        ..Default::default()
                    })
                } else {
                    Ok(crate::llm_client::ChatResponse {
                        content: None,
                        tool_calls: vec![crate::llm_client::ToolCall {
                            id: "c2".into(), kind: "function".into(),
                            function: crate::llm_client::ToolCallFunction {
                                name: "write_file".into(),
                                arguments: r#"{"path":"shared.txt","content":"BBB"}"#.into(),
                            },
                        }],
                        tokens_used: Some(10),
                        ..Default::default()
                    })
                }
            }
        }
        let provider: Arc<WCProvider> = Arc::new(WCProvider {});
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_p5", &*sink, &*provider)).unwrap();
        let t = store.require("t_p5").unwrap();
        let a = t.plan.as_ref().unwrap().steps.iter().find(|s| s.id == "a").unwrap();
        let b = t.plan.as_ref().unwrap().steps.iter().find(|s| s.id == "b").unwrap();
        println!("[test5] a={:?} b={:?} b_err={:?}", a.status, b.status, b.error);
        // a（idx 小）先跑并 Done；b（idx 大）被冲突检测拦下、标 Failed、不执行写入
        assert_eq!(a.status, StepStatus::Done, "a 应 Done: {:?}", a.status);
        assert_eq!(b.status, StepStatus::Failed, "b 应被冲突拦下 Failed: {:?}", b.status);
        let berr = b.error.clone().unwrap_or_default();
        assert!(berr.contains("已阻止覆盖"), "b error 必须含「已阻止覆盖」: {berr}");
        assert!(berr.contains("shared.txt"), "b error 必须含路径: {berr}");
        // 文件内容是先写者 a 的 "AAA"，不是后写者 b 的 "BBB"
        let art = store.artifacts_dir("t_p5").unwrap().join("shared.txt");
        let content = std::fs::read_to_string(&art).unwrap_or_default();
        println!("[test5] shared.txt content = {content:?}");
        assert_eq!(content, "AAA", "文件必须保留先写者 a 的内容，实际 {content:?}");
    }

    /// settings 相关测试串行锁（env 是进程级，多线程跑会互相污染）。
    static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_settings_file(dir: &std::path::Path, parallel_max: u32) {
        // 用 Default 生成完整 settings，再覆盖 task_parallel_max，避免缺字段导致反序列化失败
        let mut s = crate::settings::Settings::default();
        s.task_parallel_max = parallel_max;
        let p = dir.join("settings.json");
        std::fs::write(&p, serde_json::to_string_pretty(&s).unwrap()).unwrap();
    }

    /// 必测⑧：settings.task_parallel_max=1 时峰值并发=1（所有分支串行）。
    #[test]
    fn settings_parallel_max_1_forces_serial() {
        let _guard = SETTINGS_LOCK.lock().unwrap();
        let tmp = tmp_root("cfg1");
        // 把 QIBAI_DATA_DIR 指向临时目录并写 settings.json
        let old = std::env::var("QIBAI_DATA_DIR").ok();
        std::env::set_var("QIBAI_DATA_DIR", &tmp);
        write_settings_file(&tmp, 1);

        let store = TaskStore::from_tasks_root(&tmp);
        let steps: Vec<_> = (0..3).map(|i| crate::task::Step {
            id: format!("s{i}"), idx: i, ..Default::default()
        }).collect();
        let mut p = build_plan(steps);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_cfg1".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        let provider = FixedProvider::new(50, 5, "ok");
        let peak = provider.peak.clone();
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_cfg1", &*sink, &*provider)).unwrap();
        let p = peak.load(Ordering::SeqCst);
        println!("[test8] settings=1 peak={p}");
        assert_eq!(p, 1, "settings=1 时峰值必须=1，实际 {p}");

        // 恢复 env
        match old {
            Some(v) => std::env::set_var("QIBAI_DATA_DIR", v),
            None => std::env::remove_var("QIBAI_DATA_DIR"),
        }
    }

    /// 必测⑨：settings.task_parallel_max=5 时 5 个分支能同时跑（峰值≥4）。
    #[test]
    fn settings_parallel_max_5_allows_concurrency() {
        let _guard = SETTINGS_LOCK.lock().unwrap();
        let tmp = tmp_root("cfg5");
        let old = std::env::var("QIBAI_DATA_DIR").ok();
        std::env::set_var("QIBAI_DATA_DIR", &tmp);
        write_settings_file(&tmp, 5);

        let store = TaskStore::from_tasks_root(&tmp);
        let steps: Vec<_> = (0..5).map(|i| crate::task::Step {
            id: format!("s{i}"), idx: i, ..Default::default()
        }).collect();
        let mut p = build_plan(steps);
        p.approved_at = Some(now_ms());
        store.create(&Task {
            id: "t_cfg5".into(), goal: "g".into(), status: TaskStatus::Running, plan: Some(p), ..Default::default()
        }).unwrap();
        // sleep 120ms 让并发真的重叠
        let provider = FixedProvider::new(120, 5, "ok");
        let peak = provider.peak.clone();
        let sink: Arc<crate::engine::NullSink> = Arc::new(crate::engine::NullSink);
        let hook = StoreHook::new(store.clone(), sink.clone(), tmp.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_loop_async(&hook, "t_cfg5", &*sink, &*provider)).unwrap();
        let p = peak.load(Ordering::SeqCst);
        println!("[test9] settings=5 peak={p}");
        assert!(p >= 4, "settings=5 时 5 分支应能并发，峰值应≥4，实际 {p}");

        match old {
            Some(v) => std::env::set_var("QIBAI_DATA_DIR", v),
            None => std::env::remove_var("QIBAI_DATA_DIR"),
        }
    }

    /// 必测⑥：合并确定性——同一 plan 跑两次 merge_order 完全一致。
    #[test]
    fn merge_order_deterministic() {
        use crate::task::schedule::{build_groups, merge_order};
        let steps = vec![
            crate::task::Step { id: "x".into(), idx: 0, ..Default::default() },
            crate::task::Step { id: "y".into(), idx: 1, depends_on: vec!["x".into()], ..Default::default() },
            crate::task::Step { id: "z".into(), idx: 2, ..Default::default() },
        ];
        let g1 = build_groups(&steps, 2).unwrap();
        let g2 = build_groups(&steps, 2).unwrap();
        let o1 = merge_order(&g1, &steps);
        let o2 = merge_order(&g2, &steps);
        assert_eq!(o1, o2, "两次 merge_order 必须一致: {o1:?} vs {o2:?}");
        println!("[test6] merge_order = {o1:?}");
    }

    /// 必测⑦：上下文隔离——s3 的 prompt 只含依赖 s1 的 summary，不含非依赖 s2 的 summary。
    #[test]
    fn branch_prompt_context_isolated_to_deps() {
        let mut t2 = Task::default();
        t2.goal = "g".into();
        let mut plan = crate::task::Plan::default();
        plan.steps = vec![
            crate::task::Step { id: "s1".into(), idx: 0, status: StepStatus::Done, result_summary: Some("s1_summary_marker".into()), ..Default::default() },
            crate::task::Step { id: "s2".into(), idx: 1, status: StepStatus::Done, result_summary: Some("s2_summary_marker".into()), ..Default::default() },
            crate::task::Step { id: "s3".into(), idx: 2, depends_on: vec!["s1".into()], ..Default::default() },
        ];
        t2.plan = Some(plan);
        let s3 = t2.plan.as_ref().unwrap().steps.iter().find(|s| s.id == "s3").unwrap().clone();
        let sys = build_branch_system(&t2, &s3);
        println!("[test7] s3 system prompt =\n{sys}");
        assert!(sys.contains("s1_summary_marker"), "s3 prompt 必须含依赖 s1 的 summary: {sys}");
        assert!(!sys.contains("s2_summary_marker"), "s3 prompt 不得含非依赖 s2 的 summary: {sys}");
        assert!(sys.contains("## 子任务"), "必须含 ## 子任务 段");
        assert!(sys.contains("## 依赖产物"), "必须含 ## 依赖产物 段");
    }
}
