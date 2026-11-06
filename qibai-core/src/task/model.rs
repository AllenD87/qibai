//! v45.9 P0 长任务数据模型（方案 §4.1 权威规格）。
//!
//! 所有结构均 `Serialize + Deserialize`，并尽量给字段加 `#[serde(default)]`，
//! 保证旧 JSON 缺字段时仍能加载（与 `app_settings` 同一原则）。
//! 时间戳一律 **u64 毫秒**（与 `storage::now_ts` 的秒级不同，长任务需要亚秒断点）。

use serde::{Deserialize, Serialize};

/// 任务状态机主状态（方案 §4.2）。
///
/// ```text
/// draft ──plan──▶ planned ──confirm──▶ running ⇄ waiting_confirm
///                                       │  │
///                              pause ◀──┘  └──▶ paused ──resume──▶ running
///                                       │
///                               done / failed / cancelled
/// ```
/// `interrupted` 是恢复扫描时由 `running` 派生的中间态，用户确认后再回 `running`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 草稿：模型还没产出结构化计划。
    Draft,
    /// 已有计划但未批准（或已批准但用户尚未点开始）。
    Planned,
    /// 正在后台执行。
    Running,
    /// 撞到高危动作冻结中，等用户在面板点确认。
    WaitingConfirm,
    /// 用户主动暂停或预算暂停。
    Paused,
    /// 上次进程退出时还在 running，启动扫描时被改写为这个；可从断点续跑。
    Interrupted,
    /// 终态：所有步骤完成。
    Done,
    /// 终态：部分步骤成功、部分失败；产物保留，失败原因写在 last_error。
    Partial,
    /// 终态：重试超限或不可恢复错误；已完成的步骤与产物保留。
    Failed,
    /// 终态：用户取消；产物保留，临时文件清理。
    Cancelled,
}

impl Default for TaskStatus {
    fn default() -> Self {
        TaskStatus::Draft
    }
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Partial | TaskStatus::Failed | TaskStatus::Cancelled)
    }
    /// 可从断点恢复（interrupted / paused / running）。
    pub fn is_resumable(self) -> bool {
        matches!(self, TaskStatus::Interrupted | TaskStatus::Paused | TaskStatus::Running)
    }
}

/// 单步状态（比任务粒度更细；同一任务内多步并行不在 P0 范围）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    Running,
    WaitingConfirm,
    Done,
    Failed,
    /// 计划修订后被删除/跳过。
    Skipped,
}

/// 一次 Run 尝试的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    #[default]
    Success,
    Failure,
    UserCancelled,
    BudgetExceeded,
}

/// 动作风险等级（方案 §4.5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 读文件、搜索、抓网页——直接执行。
    #[default]
    Safe,
    /// 写工作区内文件、覆盖已有文件——首次确认，本任务内可记住。
    Guarded,
    /// 执行命令、删除、装技能、访问工作区外——每次必须确认，不可批量。
    Dangerous,
}

/// Token 用量（每次 Run / 每步汇总）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt: u64,
    #[serde(default)]
    pub completion: u64,
    #[serde(default)]
    pub total: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.prompt += other.prompt;
        self.completion += other.completion;
        self.total += other.total;
    }
}

/// 产物元信息（大文件只存路径引用，不内联字节）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Artifact {
    /// 相对 `<task_id>/artifacts/` 的路径，或绝对路径。
    #[serde(default)]
    pub path: String,
    /// 种类：`text` / `image` / `file` / `backup` ...
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub bytes: u64,
    /// 可选 sha256（hex）；用于崩溃恢复后校验产物完整性。
    #[serde(default)]
    pub sha256: Option<String>,
}

/// 预算（方案 §4.2 末尾 + §8.1 默认值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_max_minutes")]
    pub max_minutes: u32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
}

fn default_max_steps() -> u32 {
    20
}
fn default_max_minutes() -> u32 {
    60
}
fn default_max_tokens() -> u64 {
    80_000
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_steps: default_max_steps(),
            max_minutes: default_max_minutes(),
            max_tokens: default_max_tokens(),
        }
    }
}

/// 确认与重试策略。
///
/// `guarded_remembered` 记录「本任务内已确认过的 guarded 步骤 id」，
/// 持久化在 task.policy 里——重启后仍记得（不跨任务）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub confirm_level: RiskLevel,
    #[serde(default = "default_auto_retry")]
    pub auto_retry: u8,
    /// 本任务内已「不再问」的 guarded 步骤 id 列表。
    #[serde(default)]
    pub guarded_remembered: Vec<String>,
}

fn default_auto_retry() -> u8 {
    2
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            confirm_level: RiskLevel::Guarded,
            auto_retry: default_auto_retry(),
            guarded_remembered: Vec::new(),
        }
    }
}

/// 上下文引用（最新快照 id + 压缩产物 id）；由 context 那条线写入，本模块只存引用。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRef {
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub compression_id: Option<String>,
}

/// v45.13：并行调度信息（给 UI / 运行时读；默认空，老配置能加载）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelInfo {
    /// 最大并发数（运行时另读 settings.task_parallel_max；这里给计划层一个提示）。
    #[serde(default)]
    pub max: u32,
    /// 拓扑分层后的组：每组是 step id 列表；UI 用。
    #[serde(default)]
    pub groups: Vec<Vec<String>>,
    /// 写冲突涉及的 step id 列表（检测到但未串行化时留下记录）。
    #[serde(default)]
    pub conflicts: Vec<String>,
}

impl Default for ParallelInfo {
    fn default() -> Self {
        ParallelInfo { max: 0, groups: Vec::new(), conflicts: Vec::new() }
    }
}

/// 计划中的单步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub id: String,
    /// 在 plan 中的序号（修订后允许乱序/删除，但 idx 单调稳定）。
    #[serde(default)]
    pub idx: u32,
    #[serde(default)]
    pub title: String,
    /// 给模型看的「这一步要达成什么」。
    #[serde(default)]
    pub intent: String,
    /// 建议工具，不强制。
    #[serde(default)]
    pub tool_hint: Option<String>,
    /// 该步默认风险等级（由 planner/调用方标注）。
    #[serde(default)]
    pub risk: RiskLevel,
    #[serde(default)]
    pub status: StepStatus,
    #[serde(default)]
    pub attempts: u8,
    #[serde(default)]
    pub result_summary: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub error: Option<String>,
    /// 毫秒时间戳；0 = 未开始。
    #[serde(default)]
    pub started_at: u64,
    #[serde(default)]
    pub ended_at: u64,
    #[serde(default)]
    pub tokens: TokenUsage,
    /// v45.9：读路径 enrichment；task_get 时从 runs.ndjson 按 step_id 填入。
    /// 落盘时为空（runs 走 runs.ndjson），所以序列化到 plan.json 也是空数组，无副作用。
    #[serde(default)]
    pub runs: Vec<Run>,
    /// v45.13：依赖的 step id 列表；被依赖步骤 Done 后本步才可并行执行。
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// v45.13：本步声明要写入的目标路径（用于写冲突检测）。
    #[serde(default)]
    pub declared_writes: Vec<String>,
}

impl Default for Step {
    fn default() -> Self {
        Step {
            id: String::new(),
            idx: 0,
            title: String::new(),
            intent: String::new(),
            tool_hint: None,
            risk: RiskLevel::Safe,
            status: StepStatus::Pending,
            attempts: 0,
            result_summary: None,
            artifacts: Vec::new(),
            error: None,
            started_at: 0,
            ended_at: 0,
            tokens: TokenUsage::default(),
            runs: Vec::new(),
            depends_on: Vec::new(),
            declared_writes: Vec::new(),
        }
    }
}

impl Step {
    pub fn is_done(&self) -> bool {
        self.status == StepStatus::Done
    }
}

/// 计划（含版本号；每次修订 version + 1 并清空 approved_at）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Plan {
    #[serde(default)]
    pub version: u32,
    /// 毫秒时间戳；None = 未批准。
    #[serde(default)]
    pub approved_at: Option<u64>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn is_approved(&self) -> bool {
        self.approved_at.is_some()
    }
    /// 最后一个 `done` 步骤之后的下标（用于崩溃恢复）。
    pub fn next_pending_index(&self) -> usize {
        self.steps.iter().position(|s| !s.is_done()).unwrap_or(self.steps.len())
    }
}

/// 一次步骤执行尝试（追加日志，每行一个 JSON）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Run {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub step_id: String,
    #[serde(default)]
    pub started_at: u64,
    #[serde(default)]
    pub ended_at: u64,
    #[serde(default)]
    pub model: String,
    /// 提示词版本（hash），由 context 那条线提供。
    #[serde(default)]
    pub prompt_version: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub kb_hits: Vec<String>,
    #[serde(default)]
    pub outcome: RunOutcome,
    #[serde(default)]
    pub tokens: TokenUsage,
    /// true = 本次 tokens 是估算的（provider 未返回 usage）；false = 真实值。
    #[serde(default)]
    pub tokens_estimated: bool,
}

/// 任务根对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// `t_20261104_xxx` 形式。
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// 用户原话目标，不进压缩。
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    /// 绑定的专家 id。
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub plan: Option<Plan>,
    /// 当前执行到第几步（plan.steps 的下标）。
    #[serde(default)]
    pub current_step: usize,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub context: ContextRef,
    #[serde(default)]
    pub last_error: Option<String>,
    /// v45.9：暂停原因（预算超限/手动暂停）；格式如 `max_steps (used=20 > limit=20)`。
    #[serde(default)]
    pub paused_reason: Option<String>,
    /// 累计 token（所有 Run 汇总，前端直接读）；预算检查用。
    #[serde(default)]
    pub tokens_total: u64,
    /// 有任何一次 Run 是估算 token 就置 true。
    #[serde(default)]
    pub tokens_estimated: bool,
    /// 累计 token（结构化，兼容老路径）；预算检查用。
    #[serde(default)]
    pub tokens_used: TokenUsage,
    /// v45.13：并行调度信息（给 UI/运行时读；默认空）。
    #[serde(default)]
    pub parallel: ParallelInfo,
}

impl Default for Task {
    fn default() -> Self {
        Task {
            id: String::new(),
            title: String::new(),
            goal: String::new(),
            status: TaskStatus::Draft,
            created_at: 0,
            updated_at: 0,
            owner: None,
            plan: None,
            current_step: 0,
            artifacts: Vec::new(),
            budget: Budget::default(),
            policy: Policy::default(),
            context: ContextRef::default(),
            last_error: None,
            paused_reason: None,
            tokens_total: 0,
            tokens_estimated: false,
            tokens_used: TokenUsage::default(),
            parallel: ParallelInfo::default(),
        }
    }
}

/// 预算超限类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExceeded {
    Steps,
    Minutes,
    Tokens,
}

impl std::fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetExceeded::Steps => write!(f, "max_steps exceeded"),
            BudgetExceeded::Minutes => write!(f, "max_minutes exceeded"),
            BudgetExceeded::Tokens => write!(f, "max_tokens exceeded"),
        }
    }
}

/// 检查预算是否超限。`elapsed_ms` 由调用方传入（便于测试）。
pub fn check_budget(task: &Task, elapsed_ms: u64) -> Option<BudgetExceeded> {
    let done = task.plan.as_ref().map(|p| p.next_pending_index()).unwrap_or(0);
    if done as u32 >= task.budget.max_steps {
        return Some(BudgetExceeded::Steps);
    }
    let minutes = elapsed_ms / 60_000;
    if minutes >= task.budget.max_minutes as u64 {
        return Some(BudgetExceeded::Minutes);
    }
    if task.tokens_used.total >= task.budget.max_tokens {
        return Some(BudgetExceeded::Tokens);
    }
    None
}

/// 当前 Unix 毫秒（任务内部统一时钟；测试时可显式传 ms，不强制走这里）。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成一个新任务 id：`t_<yyyymmdd_hhmmss>_<nanos>`。
pub fn new_task_id() -> String {
    let ms = now_ms();
    let secs = ms / 1000;
    // 简单 yyyymmdd_hhmmss（UTC，与 storage::utc_stamp 同款算法，但这里只用 6+6 位）
    let days = (secs / 86400) as i64;
    let sod = secs % 86400;
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = z - era * 400;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 1460;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 365 + yoe / 146097);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { yoe + era * 400 - 1 } else { yoe + era * 400 };
    format!("t_{y:04}{m:02}{doy:02}_{hh:02}{mm:02}{ss:02}_{:04}", ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_defaults_are_serde_compatible() {
        // 旧 JSON 缺字段时能反序列化。
        let raw = r#"{"id":"t_x","title":"hi","goal":"g"}"#;
        let t: Task = serde_json::from_str(raw).unwrap();
        assert_eq!(t.status, TaskStatus::Draft);
        assert_eq!(t.budget.max_steps, 20);
        assert_eq!(t.policy.auto_retry, 2);
        assert!(t.plan.is_none());
    }

    #[test]
    fn budget_exceeded_steps_triggers() {
        let mut t = Task::default();
        t.budget.max_steps = 3;
        // 3 步都 done 后 next_pending_index == 3 -> 超限
        let mut plan = Plan::default();
        for _ in 0..3 {
            let mut s = Step::default();
            s.status = StepStatus::Done;
            plan.steps.push(s);
        }
        t.plan = Some(plan);
        assert_eq!(check_budget(&t, 0), Some(BudgetExceeded::Steps));
    }

    #[test]
    fn budget_exceeded_tokens_triggers() {
        let mut t = Task::default();
        t.budget.max_tokens = 100;
        t.tokens_used.total = 120;
        assert_eq!(check_budget(&t, 0), Some(BudgetExceeded::Tokens));
    }

    #[test]
    fn budget_ok_when_under() {
        let t = Task::default();
        assert_eq!(check_budget(&t, 0), None);
    }
}
