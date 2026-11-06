//! v45.9 P0 长任务底座：任务模型 / 存储 / 状态机 / Plan 模式 / 后台执行 / 崩溃恢复。
//!
//! 范围边界（严格）：
//! - 本模块只暴露**纯数据 + 纯状态机 + 本地文件存储**；不依赖 engine / LLM / 前端。
//! - 上下文四层与压缩由 `qibai-core::context`（另一条线）做；本模块只存 `ContextRef` 引用。
//! - 引擎接线（tokio 后台 executor、工具调用、IPC）由 `engine.rs` 那条线做。
//!
//! # 模块一览
//! - [`model`]：Task / Plan / Step / Run / Artifact / Budget / Policy / ContextRef / TokenUsage；
//! - [`store`]：`<data_dir>/tasks/<id>/` 目录布局、原子写、ndjson 追加、损坏容忍读回；
//! - [`machine`]：显式状态机 `transition`，副作用以 [`machine::TaskHook`] 注入；
//! - [`plan`]：Plan 模式（create / update / approve）+ 确认闸门（RiskLevel）；
//! - [`recovery`]：启动 `scan_and_recover` 把 running 改写为 interrupted 并定位续跑点。
//!
//! # 快速示例
//! ```no_run
//! use qibai_core::task::{TaskStore, Task, Step, RiskLevel, plan, machine, recovery};
//!
//! let store = TaskStore::new("/path/to/data");
//! let mut t = Task::default();
//! t.id = "t_demo".into();
//! t.title = "写周报".into();
//! store.create(&t).unwrap();
//!
//! // 未批准 plan -> can_execute == false（零工具调用）
//! assert!(!plan::can_execute(&t));
//!
//! // 启动恢复
//! let recovered = recovery::scan_and_recover(&store).unwrap();
//! ```

pub mod machine;
pub mod model;
pub mod plan;
pub mod recovery;
pub mod schedule;
pub mod store;

pub use machine::{NoopHook, TaskError, TaskEvent, TaskHook, transition};
pub use model::{
    Artifact, Budget, BudgetExceeded, ContextRef, ParallelInfo, Plan, Policy, RiskLevel, Run,
    RunOutcome, Step, StepStatus, Task, TaskStatus, TokenUsage, check_budget, new_task_id, now_ms,
};
pub use schedule::{ScheduleError, WriteConflict, build_groups, claimed_writes, conflicts_within_group, merge_order};
pub use plan::{
    approve_plan, build_plan, can_execute, create_and_approve, ensure_executable,
    needs_confirm, remember_guarded_approval, update_plan,
};
pub use recovery::{RecoveredTask, next_resumable_step, scan_and_recover};
pub use store::{StoreError, TaskStore};
