//! 崩溃恢复（方案 §4.2 末尾 + §10 风险表）。
//!
//! 启动时：
//! 1. 扫 `<data_dir>/tasks/*/task.json`；
//! 2. 凡 `status = running`（含 `waiting_confirm`）的，一律改写为 `interrupted`；
//! 3. 计算「最后一个 done 步骤之后」的下标，更新 `task.current_step`；
//! 4. 落盘 task.json（原子写）。
//!
//! 续跑约定：
//! - 每步产物必须先写 `<artifacts>/<name>.tmp`，再 `commit_artifact` 原子 rename
//!   （见 [`super::store::TaskStore::commit_artifact`]）；
//! - 这样中断时落盘的要么是旧完整版产物、要么是 `.tmp` 半成品（被忽略），
//!   不会出现「半截正式产物」。

use super::machine::{TaskError, TaskEvent, transition};
use super::model::Task;
use super::store::TaskStore;

/// 一个被恢复的任务描述。
#[derive(Debug, Clone)]
pub struct RecoveredTask {
    pub id: String,
    /// 原本是 running / waiting_confirm。
    pub from_status: super::model::TaskStatus,
    /// 续跑起点（plan.steps 下标）。
    pub resume_from_step: usize,
}

/// 对 store 根做一次启动恢复扫描。
///
/// 返回所有被改写为 `interrupted` 的任务列表。已经是终态或 paused 的不动。
pub fn scan_and_recover(store: &TaskStore) -> Result<Vec<RecoveredTask>, TaskError> {
    let mut out = Vec::new();
    let all = store.list()?;
    for mut t in all {
        let need_interrupt = matches!(
            t.status,
            super::model::TaskStatus::Running | super::model::TaskStatus::WaitingConfirm
        );
        if !need_interrupt {
            continue;
        }
        let from = t.status;
        let resume_idx = next_resumable_step(&t);
        t.current_step = resume_idx;
        // 用 NoopHook：恢复阶段不重放副作用（不重写 plan.json、不重跑 Run 日志）。
        transition(&mut t, TaskEvent::MarkInterrupted, &super::machine::NoopHook)?;
        store.save(&t)?;
        out.push(RecoveredTask {
            id: t.id.clone(),
            from_status: from,
            resume_from_step: resume_idx,
        });
    }
    Ok(out)
}

/// 计算「最后一个 done 步骤之后」的下标。
///
/// - 若 plan 不存在或为空，返回 0；
/// - 若所有步骤都 done，返回 steps.len()（调用方应把任务标 done）；
/// - 否则返回第一个非 done 步骤的下标。
pub fn next_resumable_step(task: &Task) -> usize {
    match task.plan.as_ref() {
        Some(p) => p.next_pending_index(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::machine::NoopHook;
    use crate::task::model::{Plan, Step, StepStatus, TaskStatus, now_ms};
    use crate::task::plan::create_and_approve;

    fn step_done(id: &str) -> Step {
        Step {
            id: id.into(),
            status: StepStatus::Done,
            ended_at: now_ms(),
            ..Default::default()
        }
    }
    fn step_pending(id: &str) -> Step {
        Step {
            id: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn running_task_marked_interrupted_and_resume_idx_correct() {
        let tmp = crate::test_support::unique_temp_dir("recovery1");
        let store = TaskStore::new(tmp);

        // 建一个跑到第 3 步（0-based 下标 2）的任务：s0/s1 done, s2 running
        let steps = vec![
            step_done("s0"),
            step_done("s1"),
            step_pending("s2"),
            step_pending("s3"),
        ];
        let mut t = create_and_approve("t".into(), "g".into(), steps, &NoopHook).unwrap();
        t.current_step = 2;
        store.create(&t).unwrap();

        // 模拟「进程退出」，再起一次 scan
        let recovered = scan_and_recover(&store).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, t.id);
        assert_eq!(recovered[0].resume_from_step, 2, "应从第 3 步（s2）续跑");

        // 落盘后状态
        let t2 = store.require(&t.id).unwrap();
        assert_eq!(t2.status, TaskStatus::Interrupted);
        assert_eq!(t2.current_step, 2);

        // 再扫一次：已是 interrupted，不动
        let recovered2 = scan_and_recover(&store).unwrap();
        assert_eq!(recovered2.len(), 0);
    }

    #[test]
    fn paused_and_terminal_tasks_untouched() {
        let tmp = crate::test_support::unique_temp_dir("recovery2");
        let store = TaskStore::new(tmp);

        // paused 任务
        let mut t1 = Task::default();
        t1.id = "t_paused".into();
        t1.status = TaskStatus::Paused;
        store.create(&t1).unwrap();

        // done 任务
        let mut t2 = Task::default();
        t2.id = "t_done".into();
        t2.status = TaskStatus::Done;
        store.create(&t2).unwrap();

        let recovered = scan_and_recover(&store).unwrap();
        assert!(recovered.is_empty());
    }

    #[test]
    fn next_resumable_step_all_done_returns_len() {
        let mut t = Task::default();
        t.plan = Some(Plan {
            version: 1,
            approved_at: Some(1),
            steps: vec![step_done("a"), step_done("b")],
        });
        assert_eq!(next_resumable_step(&t), 2);
    }

    /// 崩溃恢复端到端：跑 3 步 -> 模拟中断 -> 重启 scan -> 从第 4 步续跑，
    /// 产物目录里每步只一份（不重复），半成品 .tmp 不留在 artifacts/。
    #[test]
    fn crash_recovery_resumes_from_step_4_without_duplicating_artifacts() {
        let tmp = crate::test_support::unique_temp_dir("recovery_e2e");
        let store = TaskStore::new(&tmp);

        // 1) 建 4 步任务并批准、进入 running
        let steps = (0..4u32)
            .map(|i| Step {
                id: format!("s{i}"),
                idx: i,
                title: format!("step {i}"),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut t = create_and_approve("e2e".into(), "g".into(), steps, &NoopHook).unwrap();
        store.create(&t).unwrap();

        // 2) 「执行」前 3 步：每步先写 tmp，再 commit_artifact；最后把 step 标 done
        for i in 0..3u32 {
            let tmp_file = store
                .artifacts_dir(&t.id)
                .unwrap()
                .join(format!("s{i}.md.tmp"));
            std::fs::write(&tmp_file, format!("output of step {i}")).unwrap();
            let final_path = store
                .commit_artifact(&t.id, &tmp_file, &format!("s{i}.md"))
                .unwrap();
            assert!(final_path.exists());
            // 标步骤 done
            let plan = t.plan.as_mut().unwrap();
            let s = plan.steps.get_mut(i as usize).unwrap();
            s.status = StepStatus::Done;
            s.ended_at = now_ms();
            s.result_summary = Some(format!("done {i}"));
        }
        t.current_step = 3; // 正要跑第 4 步（s3）
        store.save(&t).unwrap();

        // 3) 模拟崩溃：进程退出前留下一个「写到一半」的 s3 产物（tmp 未 rename）
        let half_tmp = store
            .artifacts_dir(&t.id)
            .unwrap()
            .join("s3.md.tmp");
        std::fs::write(&half_tmp, "half-written").unwrap();
        // 注意：不 commit_artifact，s3.md 正式文件不存在

        // 4) 重启：scan_and_recover
        let recovered = scan_and_recover(&store).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].resume_from_step, 3, "必须从第 4 步（下标 3）续跑");

        // 5) 续跑 s3：重新产出 tmp 并 commit
        let t2 = store.require(&t.id).unwrap();
        assert_eq!(t2.status, TaskStatus::Interrupted);
        // 模拟 resume 回 running
        let mut t2 = t2;
        transition(&mut t2, TaskEvent::Resume, &NoopHook).unwrap();
        assert_eq!(t2.status, TaskStatus::Running);

        // 重新产出 s3
        let tmp_file = store
            .artifacts_dir(&t.id)
            .unwrap()
            .join("s3.md.tmp");
        std::fs::write(&tmp_file, "final output of step 3").unwrap();
        store
            .commit_artifact(&t.id, &tmp_file, "s3.md")
            .unwrap();
        store.save(&t2).unwrap();

        // 6) 断言：artifacts/ 下只有 s0.md/s1.md/s2.md/s3.md 四个正式产物
        let art = store.artifacts_dir(&t.id).unwrap();
        let mut files: Vec<String> = std::fs::read_dir(&art)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                "s0.md".to_string(),
                "s1.md".to_string(),
                "s2.md".to_string(),
                "s3.md".to_string(),
            ],
            "每个步骤只一份正式产物，半截 .tmp 必须被 rename 走（不残留）"
        );
        // s3 内容是新写的（不是半截）
        let s3 = std::fs::read(art.join("s3.md")).unwrap();
        assert_eq!(s3, b"final output of step 3");
    }
}
