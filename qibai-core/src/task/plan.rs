//! Plan 模式 + 确认闸门（方案 §4.4 / §4.5）。
//!
//! 核心不变式：**未 approve 的 plan 不允许执行任何步骤**。
//! - [`can_execute`] / [`ensure_executable`] 在所有执行入口前强制检查。
//! - [`needs_confirm`] 按 RiskLevel 与 Policy 决定是否需要用户确认；
//!   dangerous 每次必确认，guarded 首次确认后可在本任务内「不再问」。

use super::machine::{TaskError, TaskHook, TaskEvent, transition};
use super::model::{Plan, Policy, RiskLevel, Step, Task, now_ms};

/// 创建一份新计划（从模型输出的结构化步骤列表）。
///
/// - 自动分配 `id`（若空）与 `idx`；
/// - `version = 1`、`approved_at = None`；
/// - 不自动迁移状态——调用方拿到 plan 后通过 [`super::machine::set_plan_and_transition`]
///   或显式 `task.plan = Some(p)` 再 `PlanCreated`。
pub fn build_plan(steps: Vec<Step>) -> Plan {
    Plan {
        version: 1,
        approved_at: None,
        steps,
    }
}

/// 修订计划：增删步骤 / 改顺序。
///
/// 规则：
/// - `version += 1`；
/// - 清空 `approved_at`（旧批准失效，必须重新批准）；
/// - 重新分配 `idx`（按新顺序 0..n-1）；
/// - 保留已有步骤的 id / title / intent / tool_hint / risk / status / attempts / tokens
///   （如果同 id 仍在新列表里）——这样修订不会丢已完成步骤的状态。
pub fn update_plan(old: &Plan, mut new_steps: Vec<Step>) -> Plan {
    // 保留旧步骤的运行时字段，按 id 匹配
    let old_by_id: std::collections::HashMap<&str, &Step> =
        old.steps.iter().map(|s| (s.id.as_str(), s)).collect();
    for (i, s) in new_steps.iter_mut().enumerate() {
        s.idx = i as u32;
        if s.id.is_empty() {
            s.id = format!("s_{}_{}", old.version + 1, i);
        }
        if let Some(o) = old_by_id.get(s.id.as_str()) {
            // 继承运行时状态
            s.status = o.status;
            s.attempts = o.attempts;
            s.result_summary = o.result_summary.clone();
            s.artifacts = o.artifacts.clone();
            s.error = o.error.clone();
            s.started_at = o.started_at;
            s.ended_at = o.ended_at;
            s.tokens = o.tokens;
        }
    }
    Plan {
        version: old.version + 1,
        approved_at: None,
        steps: new_steps,
    }
}

/// 批准计划（写 `approved_at = Some(now_ms)`）。
pub fn approve_plan(plan: &mut Plan) {
    plan.approved_at = Some(now_ms());
}

/// 是否允许执行任何步骤。这是「未批准零工具调用」的硬闸门。
pub fn can_execute(task: &Task) -> bool {
    if task.status != super::model::TaskStatus::Running {
        return false;
    }
    match task.plan.as_ref() {
        Some(p) if p.is_approved() => !p.steps.is_empty(),
        _ => false,
    }
}

/// 执行入口检查：不允许时返回 Err（绝不静默放行）。
pub fn ensure_executable(task: &Task) -> Result<(), TaskError> {
    if task.status != super::model::TaskStatus::Running {
        return Err(TaskError::InvalidTransition {
            from: task.status,
            event: "ensure_executable".into(),
        });
    }
    let p = task.plan.as_ref().ok_or(TaskError::NoPlan)?;
    if !p.is_approved() {
        return Err(TaskError::PlanNotApproved);
    }
    if p.steps.is_empty() {
        return Err(TaskError::NoPlan);
    }
    Ok(())
}

/// 确认闸门：该步是否需要用户确认。
///
/// - `safe`：false（直接执行）；
/// - `dangerous`：true（每次必确认，不可批量）；
/// - `guarded`：若 `policy.guarded_remembered` 已包含 `step_id` 则 false，否则 true。
pub fn needs_confirm(risk: RiskLevel, policy: &Policy, step_id: &str) -> bool {
    match risk {
        RiskLevel::Safe => false,
        RiskLevel::Dangerous => true,
        RiskLevel::Guarded => !policy.guarded_remembered.iter().any(|s| s == step_id),
    }
}

/// 用户对一次 guarded 确认点点了「本任务内不再问」。
///
/// 把该 step_id 加入 `policy.guarded_remembered`，持久化在 task.policy 里。
pub fn remember_guarded_approval(task: &mut Task, step_id: &str) {
    if !task.policy.guarded_remembered.iter().any(|s| s == step_id) {
        task.policy.guarded_remembered.push(step_id.to_string());
    }
}

/// 便捷：从「用户下达目标」直接到「已批准 running」的完整链路。
///
/// 1. 构造 task；
/// 2. 建 plan；
/// 3. 迁移 draft -> planned；
/// 4. 批准；
/// 5. 迁移 planned -> running；
///
/// 每一步都过状态机；hook 副作用正常触发。
pub fn create_and_approve(
    title: String,
    goal: String,
    steps: Vec<Step>,
    hook: &dyn TaskHook,
) -> Result<Task, TaskError> {
    let mut t = Task::default();
    t.id = super::model::new_task_id();
    t.title = title;
    t.goal = goal;
    t.created_at = now_ms();
    t.updated_at = t.created_at;
    t.plan = Some(build_plan(steps));
    transition(&mut t, TaskEvent::PlanCreated, hook)?;
    approve_plan(t.plan.as_mut().unwrap());
    transition(&mut t, TaskEvent::PlanApproved, hook)?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::machine::NoopHook;
    use crate::task::model::{StepStatus, TaskStatus};

    fn step(id: &str, risk: RiskLevel) -> Step {
        Step {
            id: id.into(),
            title: format!("step {id}"),
            risk,
            ..Default::default()
        }
    }

    #[test]
    fn unapproved_plan_cannot_execute_even_if_running() {
        // 构造一个 running 但 plan 未批准的 task（绕过状态机批准步骤）
        let mut t = Task::default();
        t.id = "t_x".into();
        t.status = TaskStatus::Running;
        t.plan = Some(build_plan(vec![step("s1", RiskLevel::Safe)]));
        // approved_at 仍是 None
        assert!(!can_execute(&t));
        assert!(ensure_executable(&t).is_err());
    }

    #[test]
    fn approved_plan_can_execute() {
        let hook = NoopHook;
        let t = create_and_approve(
            "写周报".into(),
            "写一份周报".into(),
            vec![step("s1", RiskLevel::Safe)],
            &hook,
        )
        .unwrap();
        assert!(can_execute(&t));
        ensure_executable(&t).unwrap();
    }

    #[test]
    fn update_plan_bumps_version_and_clears_approved() {
        let mut p = build_plan(vec![step("s1", RiskLevel::Safe)]);
        approve_plan(&mut p);
        assert!(p.is_approved());
        let p2 = update_plan(&p, vec![step("s1", RiskLevel::Safe), step("s2", RiskLevel::Safe)]);
        assert_eq!(p2.version, 2);
        assert!(!p2.is_approved());
        assert_eq!(p2.steps.len(), 2);
        assert_eq!(p2.steps[0].idx, 0);
        assert_eq!(p2.steps[1].idx, 1);
    }

    #[test]
    fn update_plan_preserves_completed_step_state() {
        let mut p = build_plan(vec![step("s1", RiskLevel::Safe)]);
        p.steps[0].status = StepStatus::Done;
        p.steps[0].attempts = 3;
        p.steps[0].result_summary = Some("done".into());
        let p2 = update_plan(&p, vec![step("s1", RiskLevel::Safe)]);
        assert_eq!(p2.steps[0].status, StepStatus::Done);
        assert_eq!(p2.steps[0].attempts, 3);
        assert_eq!(p2.steps[0].result_summary.as_deref(), Some("done"));
    }

    #[test]
    fn risk_gate_safe_directly_through() {
        let pol = Policy::default();
        assert!(!needs_confirm(RiskLevel::Safe, &pol, "s1"));
    }

    #[test]
    fn risk_gate_dangerous_always_confirms_even_after_remember() {
        let mut pol = Policy::default();
        pol.guarded_remembered.push("s_danger".into());
        // 即便 step_id 在 remembered 列表里，dangerous 也必须每次确认
        assert!(needs_confirm(RiskLevel::Dangerous, &pol, "s_danger"));
    }

    #[test]
    fn risk_gate_guarded_second_time_no_longer_asks() {
        let mut t = Task::default();
        t.id = "t_x".into();
        // 第一次：需要确认
        assert!(needs_confirm(RiskLevel::Guarded, &t.policy, "s1"));
        // 用户点「不再问」
        remember_guarded_approval(&mut t, "s1");
        // 第二次：不再问
        assert!(!needs_confirm(RiskLevel::Guarded, &t.policy, "s1"));
        // 其他步骤仍要问
        assert!(needs_confirm(RiskLevel::Guarded, &t.policy, "s2"));
    }

    /// 一个「假 executor」：每次准备调工具就把计数 +1。
    /// 它在执行任何步骤前必须先调 [`ensure_executable`]。
    struct FakeExecutor {
        tool_calls: std::sync::atomic::AtomicUsize,
    }
    impl FakeExecutor {
        fn new() -> Self {
            Self {
                tool_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        /// 试图执行所有 pending 步骤；返回是否有任何一步真正下发了工具调用。
        fn try_run_all(&self, task: &Task) -> Result<usize, TaskError> {
            ensure_executable(task)?;
            // 上面 ensure_executable 一旦通过，才会真的去调工具。
            for s in task.plan.as_ref().unwrap().steps.iter() {
                if s.status == StepStatus::Pending {
                    self.tool_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            Ok(self.tool_calls.load(std::sync::atomic::Ordering::SeqCst))
        }
    }

    #[test]
    fn unapproved_plan_results_in_zero_tool_calls() {
        // 场景：plan 已建好，但用户还没点批准；executor 试图在后台跑。
        let mut t = Task::default();
        t.id = "t_no_approve".into();
        t.status = TaskStatus::Running;
        t.plan = Some(build_plan(vec![
            step("s1", RiskLevel::Safe),
            step("s2", RiskLevel::Safe),
            step("s3", RiskLevel::Safe),
        ]));
        // 注意：approved_at 仍是 None（未批准）。

        let ex = FakeExecutor::new();
        let r = ex.try_run_all(&t);
        assert!(r.is_err(), "未批准 plan 必须拒绝执行");
        assert_eq!(
            ex.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "未批准 plan 时绝不能有任何工具调用下发"
        );
    }

    #[test]
    fn approved_plan_allows_tool_calls() {
        let hook = NoopHook;
        let t = create_and_approve(
            "t".into(),
            "g".into(),
            vec![step("s1", RiskLevel::Safe), step("s2", RiskLevel::Safe)],
            &hook,
        )
        .unwrap();
        let ex = FakeExecutor::new();
        let n = ex.try_run_all(&t).unwrap();
        assert_eq!(n, 2);
    }
}
