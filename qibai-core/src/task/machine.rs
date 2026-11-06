//! 任务状态机（方案 §4.2）。
//!
//! 显式迁移表：`transition(&mut Task, Event)`，非法迁移返回 `Err`（绝不静默）。
//! 副作用（写 plan.json / 快照上下文 / 插 Run 日志）通过 [`TaskHook`] trait 注入，
//! 测试时可换成计数器实现；生产接线由 engine 那条线完成。

use thiserror::Error;

use super::model::{Plan, Run, Task, TaskStatus};

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("invalid transition: {from:?} + {event}")]
    InvalidTransition { from: TaskStatus, event: String },
    #[error("plan not approved")]
    PlanNotApproved,
    #[error("no plan")]
    NoPlan,
    #[error("invalid step index: {0}")]
    InvalidStepIndex(usize),
    #[error("hook side-effect failed: {0}")]
    Hook(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("store error: {0}")]
    Store(#[from] super::store::StoreError),
}

/// 状态机事件（触发迁移的外部动作）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEvent {
    /// draft -> planned：模型产出结构化计划。
    PlanCreated,
    /// planned -> running：用户批准计划。要求 plan.approved_at.is_some()。
    PlanApproved,
    /// running -> waiting_confirm：撞到高危工具。
    NeedConfirm,
    /// waiting_confirm -> running：用户同意继续。
    ConfirmResolved,
    /// running -> paused：用户暂停 / 预算暂停。
    Pause,
    /// paused/interrupted -> running：恢复执行。
    Resume,
    /// running -> done：所有步骤完成。
    MarkDone,
    /// running -> partial：部分步骤成功、部分失败。
    MarkPartial,
    /// running -> failed：不可恢复错误。
    MarkFailed,
    /// 任意非终态 -> cancelled：用户取消。
    Cancel,
    /// 启动恢复扫描：running -> interrupted（仅 scan 内部用）。
    MarkInterrupted,
}

impl std::fmt::Display for TaskEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskEvent::PlanCreated => "plan_created",
            TaskEvent::PlanApproved => "plan_approved",
            TaskEvent::NeedConfirm => "need_confirm",
            TaskEvent::ConfirmResolved => "confirm_resolved",
            TaskEvent::Pause => "pause",
            TaskEvent::Resume => "resume",
            TaskEvent::MarkDone => "mark_done",
            TaskEvent::MarkPartial => "mark_partial",
            TaskEvent::MarkFailed => "mark_failed",
            TaskEvent::Cancel => "cancel",
            TaskEvent::MarkInterrupted => "mark_interrupted",
        };
        write!(f, "{s}")
    }
}

/// 副作用钩子：默认全空实现，生产层可以把「写盘 / 上下文快照 / Run 日志」接进来。
///
/// 设计为 trait object 友好（&dyn TaskHook），测试时传一个计数桩。
pub trait TaskHook {
    /// 计划首次定稿或修订后（task.status 已迁移到 planned）。
    fn on_plan_saved(&self, _task: &Task) -> Result<(), String> {
        Ok(())
    }
    /// 进入 running 前（快照上下文、建 Run 日志的时机）。
    fn on_enter_running(&self, _task: &Task) -> Result<(), String> {
        Ok(())
    }
    /// 即将进入 waiting_confirm（冻结该步）。
    fn on_waiting_confirm(&self, _task: &Task, _step_id: &str) -> Result<(), String> {
        Ok(())
    }
    /// 追加一条 Run 日志。
    fn on_run_appended(&self, _task: &Task, _run: &Run) -> Result<(), String> {
        Ok(())
    }
}

/// 空钩子（默认）。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopHook;
impl TaskHook for NoopHook {}

/// 核心迁移表。返回迁移后的新状态；非法迁移返回 Err。
pub fn next_status(current: TaskStatus, event: TaskEvent) -> Result<TaskStatus, TaskError> {
    use TaskStatus::*;
    let next = match (current, event) {
        (Draft, TaskEvent::PlanCreated) => Planned,
        (Planned, TaskEvent::PlanApproved) => Running,
        (Running, TaskEvent::NeedConfirm) => WaitingConfirm,
        (WaitingConfirm, TaskEvent::ConfirmResolved) => Running,
        (Running, TaskEvent::Pause) => Paused,
        (Paused, TaskEvent::Resume) => Running,
        (Interrupted, TaskEvent::Resume) => Running,
        (Running, TaskEvent::MarkDone) => Done,
        (Running, TaskEvent::MarkPartial) => Partial,
        (Running, TaskEvent::MarkFailed) => Failed,
        (Running, TaskEvent::MarkInterrupted) => Interrupted,
        (WaitingConfirm, TaskEvent::MarkInterrupted) => Interrupted,
        (Paused, TaskEvent::MarkInterrupted) => Interrupted,
        // 任意非终态 -> cancelled
        (_s @ (Draft | Planned | Running | WaitingConfirm | Paused | Interrupted), TaskEvent::Cancel)
            => Cancelled,
        (from, ev) => {
            return Err(TaskError::InvalidTransition {
                from,
                event: ev.to_string(),
            })
        }
    };
    Ok(next)
}

/// 对 task 执行迁移；`hook` 副作用在迁移**成功后**调用（hook 失败不回滚状态，但返回 Err）。
pub fn transition(task: &mut Task, event: TaskEvent, hook: &dyn TaskHook) -> Result<(), TaskError> {
    // 特定事件的前置校验
    match event {
        TaskEvent::PlanApproved => {
            let plan = task.plan.as_ref().ok_or(TaskError::NoPlan)?;
            if !plan.is_approved() {
                return Err(TaskError::PlanNotApproved);
            }
        }
        TaskEvent::PlanCreated => {
            // 必须有 plan 才能迁到 planned
            if task.plan.is_none() {
                return Err(TaskError::NoPlan);
            }
        }
        _ => {}
    }

    let from = task.status;
    let next = next_status(from, event)?;
    task.status = next;
    task.updated_at = super::model::now_ms();

    // 副作用
    match event {
        TaskEvent::PlanCreated => hook.on_plan_saved(task).map_err(TaskError::Hook)?,
        TaskEvent::PlanApproved | TaskEvent::Resume => {
            hook.on_enter_running(task).map_err(TaskError::Hook)?;
        }
        TaskEvent::NeedConfirm => {
            let step_id = task
                .plan
                .as_ref()
                .and_then(|p| p.steps.get(task.current_step))
                .map(|s| s.id.clone())
                .unwrap_or_default();
            hook.on_waiting_confirm(task, &step_id)
                .map_err(TaskError::Hook)?;
        }
        _ => {}
    }
    Ok(())
}

/// 在 task 上新增/替换 plan 并迁移到 planned（便捷函数）。
pub fn set_plan_and_transition(
    task: &mut Task,
    plan: Plan,
    hook: &dyn TaskHook,
) -> Result<(), TaskError> {
    task.plan = Some(plan);
    transition(task, TaskEvent::PlanCreated, hook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::model::{new_task_id, now_ms};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 计数桩钩子：统计各副作用被调用次数。
    #[derive(Default)]
    struct CountingHook {
        plan_saved: AtomicUsize,
        enter_running: AtomicUsize,
        waiting: AtomicUsize,
        run_appended: AtomicUsize,
    }
    impl TaskHook for CountingHook {
        fn on_plan_saved(&self, _task: &Task) -> Result<(), String> {
            self.plan_saved.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn on_enter_running(&self, _task: &Task) -> Result<(), String> {
            self.enter_running.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn on_waiting_confirm(&self, _task: &Task, _step_id: &str) -> Result<(), String> {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn on_run_appended(&self, _task: &Task, _run: &Run) -> Result<(), String> {
            self.run_appended.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn fresh_task() -> Task {
        let mut t = Task::default();
        t.id = new_task_id();
        t.created_at = now_ms();
        t.updated_at = t.created_at;
        t
    }

    fn plan_with_approved(approved: bool) -> Plan {
        let mut p = Plan::default();
        p.version = 1;
        p.approved_at = if approved { Some(now_ms()) } else { None };
        p
    }

    #[test]
    fn happy_path_draft_to_done() {
        let hook = CountingHook::default();
        let mut t = fresh_task();
        // draft -> planned
        t.plan = Some(plan_with_approved(false));
        transition(&mut t, TaskEvent::PlanCreated, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Planned);
        // 批准
        t.plan.as_mut().unwrap().approved_at = Some(now_ms());
        transition(&mut t, TaskEvent::PlanApproved, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        // pause / resume
        transition(&mut t, TaskEvent::Pause, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Paused);
        transition(&mut t, TaskEvent::Resume, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        // waiting_confirm 往返
        transition(&mut t, TaskEvent::NeedConfirm, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::WaitingConfirm);
        transition(&mut t, TaskEvent::ConfirmResolved, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        // done
        transition(&mut t, TaskEvent::MarkDone, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Done);
        // 副作用被调用
        assert_eq!(hook.plan_saved.load(Ordering::SeqCst), 1);
        assert_eq!(hook.enter_running.load(Ordering::SeqCst), 2); // approved + resume
        assert_eq!(hook.waiting.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn illegal_draft_to_done_rejected() {
        let hook = NoopHook;
        let mut t = fresh_task();
        let r = transition(&mut t, TaskEvent::MarkDone, &hook);
        assert!(r.is_err(), "draft 不能直接到 done");
    }

    #[test]
    fn planned_to_running_requires_approved_plan() {
        let hook = NoopHook;
        let mut t = fresh_task();
        t.plan = Some(plan_with_approved(false));
        transition(&mut t, TaskEvent::PlanCreated, &hook).unwrap();
        // 未批准就想 running
        let r = transition(&mut t, TaskEvent::PlanApproved, &hook);
        assert!(matches!(r, Err(TaskError::PlanNotApproved)));
        assert_eq!(t.status, TaskStatus::Planned);
    }

    #[test]
    fn terminal_state_is_frozen() {
        let hook = NoopHook;
        let mut t = fresh_task();
        t.status = TaskStatus::Done;
        assert!(transition(&mut t, TaskEvent::Resume, &hook).is_err());
        assert!(transition(&mut t, TaskEvent::Cancel, &hook).is_err());
        assert!(transition(&mut t, TaskEvent::PlanCreated, &hook).is_err());
    }

    #[test]
    fn cancel_from_any_nonterminal() {
        let hook = NoopHook;
        for s in [
            TaskStatus::Draft,
            TaskStatus::Planned,
            TaskStatus::Running,
            TaskStatus::WaitingConfirm,
            TaskStatus::Paused,
            TaskStatus::Interrupted,
        ] {
            let mut t = fresh_task();
            t.status = s;
            transition(&mut t, TaskEvent::Cancel, &hook).unwrap();
            assert_eq!(t.status, TaskStatus::Cancelled);
        }
    }

    #[test]
    fn hook_failure_propagates_without_state_mutation_after() {
        struct FailHook;
        impl TaskHook for FailHook {
            fn on_plan_saved(&self, _t: &Task) -> Result<(), String> {
                Err("disk full".into())
            }
        }
        let mut t = fresh_task();
        t.plan = Some(plan_with_approved(false));
        let r = transition(&mut t, TaskEvent::PlanCreated, &FailHook);
        assert!(matches!(r, Err(TaskError::Hook(_))));
        // 状态已经迁移（副作用失败不回滚，但调用方应看到 Err 决定要不要重试）
        assert_eq!(t.status, TaskStatus::Planned);
    }

    /// 预算超限端到端：check_budget 命中 -> 触发 Pause 迁移 -> 状态变 paused 且记 last_error。
    #[test]
    fn budget_exceeded_transitions_to_paused_with_reason() {
        use super::super::model::{BudgetExceeded, Step, StepStatus, check_budget};

        let hook = NoopHook;
        // 造一个已 running 的任务：4 步全 done（触发 max_steps=3）
        let mut t = fresh_task();
        t.budget.max_steps = 3;
        let mut plan = plan_with_approved(true);
        for i in 0..4 {
            let mut s = Step::default();
            s.id = format!("s{i}");
            s.status = StepStatus::Done;
            plan.steps.push(s);
        }
        t.plan = Some(plan);
        transition(&mut t, TaskEvent::PlanCreated, &hook).unwrap();
        t.plan.as_mut().unwrap().approved_at = Some(now_ms());
        transition(&mut t, TaskEvent::PlanApproved, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Running);

        // 预算检查命中
        let exceeded = check_budget(&t, 0);
        assert_eq!(exceeded, Some(BudgetExceeded::Steps));

        // 触发 Pause 迁移
        t.last_error = Some(format!("预算超限: {exceeded:?}"));
        transition(&mut t, TaskEvent::Pause, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Paused);
        assert!(t.last_error.as_deref().unwrap().contains("预算超限"));

        // 之后可以 Resume
        transition(&mut t, TaskEvent::Resume, &hook).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
    }

    #[test]
    fn budget_tokens_exceeded_marks_paused() {
        use super::super::model::{BudgetExceeded, TokenUsage, check_budget};
        let mut t = fresh_task();
        t.budget.max_tokens = 100;
        t.tokens_used = TokenUsage {
            prompt: 0,
            completion: 0,
            total: 150,
        };
        assert_eq!(check_budget(&t, 0), Some(BudgetExceeded::Tokens));
    }
}
