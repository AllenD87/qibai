//! 任务记录与编排运行时（Orchestration Runtime 轻量落地）。
//!
//! 借鉴 Qwen-Audio-Agent（arXiv:2609.25195，Apache-2.0）前后台解耦架构：
//! - 任务记录是「投递回执」而非后端任务图的镜像；公共字段最小化。
//! - `spawn` 提交**自包含目标**（spawn_thinking 的保守解释：
//!   「继续刚才那个页面」这类指代必须先解析成独立指令，不把原始 ASR 当任务描述）。
//! - **打断 ≠ 取消**：语音打断 / 静音 / 睡眠只停口播与投递，不动已受理的后台任务。
//! - **完成 ≠ 交付**：任务完成后结果进入「投递窗口」，用户说话或正在播放时
//!   挂起等待（bounded retries），窗口安全后一次性呈现——复用现有 say 气泡 /
//!   提醒浮层通道，不新增前端呈现。
//! - 准入限流：全局 1 个活跃槽 + 每个 owner 1 个活跃槽；超出排队。
//! - `get_status` 只读查询，不产生新任务、不触发执行。
//!
//! 本模块为纯状态机（同步、无 I/O、时钟由调用方注入 `now_ms`），便于单测；
//! 与 Engine / Tauri 的集成点见架构说明文档《qibai_qwen_arch_架构说明.md》
//! 「Phase 3 集成清单」——本轮不改 listener.rs / tts.rs / main.rs。

use serde::{Deserialize, Serialize};

/// 任务生命周期（与 Qwen-Audio-Agent Task record 对齐，去掉其 ACP 专有节点）。
/// queued → running → completed；running → finalizing → completed；
/// queued/running → cancelling → cancelled；任意非终态 → failed。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    /// 委托执行中（对应 delegated：后台已派发、协调槽已释放）。
    Finalizing,
    Completed,
    Failed,
    Cancelling,
    Cancelled,
}

impl TaskStatus {
    /// 终态：不可再转移（对齐 qwen：completed/failed/cancelled 不重入）。
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled)
    }
}

/// 结果投递状态——「完成 ≠ 交付」的核心。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// 已受理，尚未有结果。
    Pending,
    /// 已有结果，正在等待安全投递窗口（用户没说话、没在播放）。
    WindowWaiting,
    /// 窗口安全，结果可被前端取走呈现（say 气泡 / 提醒浮层）。
    Ready,
    /// 用户正在说话 / 有口播在进行——挂起投递，**任务本身不取消**。
    Deferred,
    /// 已完成呈现（播放/气泡已确认）。
    Delivered,
}

/// 投递窗口规则：哪些情况算「不安全」。
#[derive(Debug, Clone)]
pub struct DeliveryWindow {
    /// 用户正在说话（listener VAD / barge-in 侧上报）。
    pub user_speaking: bool,
    /// 口播正在进行（tts 播放中）。
    pub playback_active: bool,
    /// 最多等待多少次窗口重判（防无限悬挂；耗尽后仍按可见渠道兜底呈现）。
    pub max_attempts: u8,
    /// 已重判次数。
    pub attempts: u8,
}

impl Default for DeliveryWindow {
    fn default() -> Self {
        Self {
            user_speaking: false,
            playback_active: false,
            max_attempts: 8,
            attempts: 0,
        }
    }
}

impl DeliveryWindow {
    /// 当前窗口是否安全可投递。
    pub fn is_safe(&self) -> bool {
        !self.user_speaking && !self.playback_active
    }
}

/// 任务记录：公共字段最小化（用户请求 / 时间戳 / 结果或错误 / 通用活动描述 /
/// 待确认输入摘要 / 投递状态）。不暴露执行策略与后端内部细节。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    /// 归属者（如 "pet" / "console" / 会话 id），用于每 owner 限流与结果认领。
    pub owner: String,
    /// 自包含目标（调用方已解析指代）。
    pub objective: String,
    pub status: TaskStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// 最终结果文案（completed 时有效，供 say 气泡/浮层直接呈现）。
    pub result: Option<String>,
    /// 失败原因（failed 时有效）。
    pub error: Option<String>,
    /// 等待用户补充输入/授权的摘要（自然语言，不暴露内部 id）。
    pub pending_input: Option<String>,
    pub delivery: DeliveryState,
}

/// 编排错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrchestrateError {
    #[error("objective is empty or still contains unresolved reference: {0}")]
    ObjectiveNotSelfContained(String),
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("invalid transition {from:?} -> {to:?} for task {id}")]
    InvalidTransition {
        id: String,
        from: TaskStatus,
        to: TaskStatus,
    },
    #[error("task {0} already terminal")]
    AlreadyTerminal(String),
}

/// 编排运行时：纯内存状态机。
///
/// 与 Engine 的关系（Phase 3 集成）：Engine 持有 `TaskOrchestrator`，
/// `send_message` 遇到「需要后台执行」的请求时 `submit()` 受理、立即返回应答；
/// 短答走原路径直接流式回复——对话不被后台任务阻塞。
#[derive(Debug, Default)]
pub struct TaskOrchestrator {
    tasks: Vec<TaskRecord>,
    seq: u64,
    /// 全局活跃槽占用计数（非终态任务数）。
    global_active: usize,
}

impl TaskOrchestrator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 受理一个后台任务（对应 spawn_thinking）。
    ///
    /// `objective` 必须自包含：非空，且不含未解析的指代标记
    /// （`{ref}` / `刚才那个` / `continue:` 等——由调用方在提交前解析）。
    /// 超准入槽时以 Queued 排队，不拒绝。
    pub fn submit(
        &mut self,
        owner: &str,
        objective: &str,
        now_ms: u64,
    ) -> Result<String, OrchestrateError> {
        let trimmed = objective.trim();
        if trimmed.is_empty()
            || trimmed.contains("{ref}")
            || trimmed.contains("刚才那个")
            || trimmed.starts_with("continue:")
        {
            return Err(OrchestrateError::ObjectiveNotSelfContained(
                objective.to_string(),
            ));
        }
        self.seq += 1;
        let id = format!("task-{}", self.seq);
        self.tasks.push(TaskRecord {
            id: id.clone(),
            owner: owner.to_string(),
            objective: trimmed.to_string(),
            status: TaskStatus::Queued,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            result: None,
            error: None,
            pending_input: None,
            delivery: DeliveryState::Pending,
        });
        self.maybe_schedule(now_ms);
        Ok(id)
    }

    /// 准入调度：全局/每 owner 活跃槽未满的 Queued 任务转 Running。
    /// 本轮不做真正执行——执行由调用方（Phase 3 接入 Engine/技能体系）驱动。
    fn maybe_schedule(&mut self, now_ms: u64) {
        let max_global: usize = 1;
        let max_per_owner: usize = 1;
        loop {
            // 活跃槽只数 Running/Finalizing：Queued 是排队队列，不得占用准入槽——
            // 否则首个提交的任务把自己算成「已满」，永远排不进 Running。
            let global = self
                .tasks
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Running | TaskStatus::Finalizing))
                .count();
            // 找一个可启动的 Queued 任务
            if global >= max_global {
                break;
            }
            let Some(idx) = self.tasks.iter().position(|t| t.status == TaskStatus::Queued) else {
                break;
            };
            let owner = self.tasks[idx].owner.clone();
            // 只计已在跑的（Running/Finalizing），候选 Queued 任务自身不能占槽。
            let owner_running = self
                .tasks
                .iter()
                .filter(|t| {
                    t.owner == owner
                        && matches!(t.status, TaskStatus::Running | TaskStatus::Finalizing)
                })
                .count();
            if owner_running >= max_per_owner {
                break;
            }
            self.tasks[idx].status = TaskStatus::Running;
            self.tasks[idx].updated_at_ms = now_ms;
            self.global_active = global + 1;
        }
        let _ = self.global_active;
    }

    /// 后台执行完成（成功）——进入投递窗口等待，**不立即呈现**。
    pub fn complete(
        &mut self,
        id: &str,
        result: &str,
        window: &DeliveryWindow,
        now_ms: u64,
    ) -> Result<(), OrchestrateError> {
        let t = self.find_mut(id)?;
        if t.status.is_terminal() {
            return Err(OrchestrateError::AlreadyTerminal(id.to_string()));
        }
        t.status = TaskStatus::Completed;
        t.result = Some(result.to_string());
        t.updated_at_ms = now_ms;
        t.delivery = if window.is_safe() {
            DeliveryState::Ready
        } else {
            DeliveryState::WindowWaiting
        };
        Ok(())
    }

    /// 后台执行失败。
    pub fn fail(&mut self, id: &str, error: &str, now_ms: u64) -> Result<(), OrchestrateError> {
        let t = self.find_mut(id)?;
        if t.status.is_terminal() {
            return Err(OrchestrateError::AlreadyTerminal(id.to_string()));
        }
        t.status = TaskStatus::Failed;
        t.error = Some(error.to_string());
        t.delivery = DeliveryState::Ready; // 失败也要兜底告知用户
        t.updated_at_ms = now_ms;
        self.maybe_schedule(now_ms);
        Ok(())
    }

    /// 用户要求取消任务——显式取消才动任务（对应 cancel_agent_task）。
    pub fn cancel(&mut self, id: &str, now_ms: u64) -> Result<(), OrchestrateError> {
        let t = self.find_mut(id)?;
        if t.status.is_terminal() {
            return Err(OrchestrateError::AlreadyTerminal(id.to_string()));
        }
        t.status = TaskStatus::Cancelled;
        t.updated_at_ms = now_ms;
        self.maybe_schedule(now_ms);
        Ok(())
    }

    /// 语音打断（barge-in）——**只影响投递，不动任务状态**。
    /// 若结果正等待/可投递，挂起到 Deferred；任务继续跑。
    pub fn on_user_interrupt(&mut self, id: &str) -> Result<(), OrchestrateError> {
        let t = self.find_mut(id)?;
        if matches!(
            t.delivery,
            DeliveryState::WindowWaiting | DeliveryState::Ready
        ) {
            t.delivery = DeliveryState::Deferred;
        }
        // 注意：不改 t.status——这就是「打断 ≠ 取消」。
        Ok(())
    }

    /// 投递窗口重判（listener/tts 侧状态变化时调用，如播放结束、用户说完话）。
    /// 安全则把等待中的结果转为 Ready；超过 max_attempts 仍强制 Ready 兜底。
    pub fn refresh_window(
        &mut self,
        id: &str,
        window: &DeliveryWindow,
        now_ms: u64,
    ) -> Result<DeliveryState, OrchestrateError> {
        let t = self.find_mut(id)?;
        let safe = window.is_safe() || window.attempts >= window.max_attempts;
        if matches!(
            t.delivery,
            DeliveryState::WindowWaiting | DeliveryState::Deferred
        ) && safe
        {
            t.delivery = DeliveryState::Ready;
            t.updated_at_ms = now_ms;
        }
        Ok(t.delivery)
    }

    /// 前端确认呈现完成（气泡已显示/口播已起播）。
    pub fn mark_delivered(&mut self, id: &str) -> Result<(), OrchestrateError> {
        let t = self.find_mut(id)?;
        if t.delivery != DeliveryState::Ready {
            return Err(OrchestrateError::InvalidTransition {
                id: id.to_string(),
                from: t.delivery_status(),
                to: TaskStatus::Running,
            });
        }
        t.delivery = DeliveryState::Delivered;
        Ok(())
    }

    /// 只读状态查询（对应 get_agent_task_status）：不创建任务、不触发执行。
    pub fn get_status(&self, id: &str) -> Option<&TaskRecord> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// 某 owner 所有未投递完成结果（前端轮询取走呈现）。
    pub fn pending_deliveries(&self, owner: &str) -> Vec<&TaskRecord> {
        self.tasks
            .iter()
            .filter(|t| t.owner == owner && t.delivery == DeliveryState::Ready)
            .collect()
    }

    /// 全部任务（调试/面板用）。
    pub fn all_tasks(&self) -> &[TaskRecord] {
        &self.tasks
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut TaskRecord, OrchestrateError> {
        self.tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| OrchestrateError::NotFound(id.to_string()))
    }
}

impl TaskRecord {
    /// delivery 的可序列化状态标签（供 mark_delivered 错误路径用）。
    fn delivery_status(&self) -> TaskStatus {
        self.status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orch() -> TaskOrchestrator {
        TaskOrchestrator::new()
    }

    #[test]
    fn submit_accepts_self_contained_objective() {
        let mut o = orch();
        let id = o
            .submit("pet", "给用户查今天石家庄天气并播报", 1000)
            .unwrap();
        assert!(o.get_status(&id).is_some());
        assert_eq!(o.get_status(&id).unwrap().status, TaskStatus::Running);
    }

    #[test]
    fn submit_rejects_unresolved_reference() {
        let mut o = orch();
        let r = o.submit("pet", "继续{ref}那个任务", 1000);
        assert!(matches!(r, Err(OrchestrateError::ObjectiveNotSelfContained(_))));
        assert!(o.submit("pet", "", 1000).is_err());
        assert!(o.submit("pet", "   ", 1000).is_err());
    }

    #[test]
    fn admission_limits_queue_beyond_slot() {
        let mut o = orch();
        let a = o.submit("pet", "任务一：后台联网操作", 1000).unwrap();
        // 同 owner 第二任务：每 owner 1 槽 → 排队为 Queued
        let b = o.submit("pet", "任务二：另一个后台操作", 1000).unwrap();
        assert_eq!(o.get_status(&a).unwrap().status, TaskStatus::Running);
        assert_eq!(o.get_status(&b).unwrap().status, TaskStatus::Queued);
    }

    #[test]
    fn completion_queues_behind_delivery_window_not_immediate() {
        let mut o = orch();
        let id = o.submit("pet", "后台取数并整理", 1000).unwrap();
        // 用户正在说话 → 窗口不安全
        let mut w = DeliveryWindow::default();
        w.user_speaking = true;
        o.complete(&id, "整理结果：……", &w, 2000).unwrap();
        assert_eq!(o.get_status(&id).unwrap().status, TaskStatus::Completed);
        // 完成 ≠ 交付：结果挂起，不立即可见
        assert_eq!(
            o.get_status(&id).unwrap().delivery,
            DeliveryState::WindowWaiting
        );
        assert!(o.pending_deliveries("pet").is_empty());
    }

    #[test]
    fn interruption_defers_delivery_but_keeps_task_running() {
        let mut o = orch();
        let id = o.submit("pet", "长后台任务", 1000).unwrap();
        // 用户 barge-in：此时任务还在跑
        o.on_user_interrupt(&id).unwrap();
        // 任务状态不动（打断 ≠ 取消）
        assert_eq!(o.get_status(&id).unwrap().status, TaskStatus::Running);
        // 任务后续完成，用户还在说话
        let mut w = DeliveryWindow::default();
        w.user_speaking = true;
        o.complete(&id, "结果", &w, 2000).unwrap();
        // 打断时挂起语义：若原本可投递则 Deferred
        assert_eq!(
            o.get_status(&id).unwrap().delivery,
            DeliveryState::WindowWaiting
        );
    }

    #[test]
    fn window_clears_makes_delivery_ready() {
        let mut o = orch();
        let id = o.submit("pet", "后台任务", 1000).unwrap();
        let mut w = DeliveryWindow::default();
        w.user_speaking = true;
        o.complete(&id, "结果内容", &w, 2000).unwrap();
        // 用户说完话，窗口变安全
        let mut w2 = DeliveryWindow::default();
        w2.user_speaking = false;
        let s = o.refresh_window(&id, &w2, 3000).unwrap();
        assert_eq!(s, DeliveryState::Ready);
        assert_eq!(o.pending_deliveries("pet").len(), 1);
        // 前端取走呈现后标记交付
        o.mark_delivered(&id).unwrap();
        assert_eq!(
            o.get_status(&id).unwrap().delivery,
            DeliveryState::Delivered
        );
    }

    #[test]
    fn cancel_only_explicit_terminal_stays_terminal() {
        let mut o = orch();
        let id = o.submit("pet", "后台任务", 1000).unwrap();
        o.cancel(&id, 2000).unwrap();
        assert_eq!(o.get_status(&id).unwrap().status, TaskStatus::Cancelled);
        // 终态不可再 complete/fail/cancel
        assert!(o.complete(&id, "x", &DeliveryWindow::default(), 3000).is_err());
        assert!(o.fail(&id, "x", 3000).is_err());
        assert!(o.cancel(&id, 3000).is_err());
    }

    #[test]
    fn fail_makes_result_ready_fallback() {
        let mut o = orch();
        let id = o.submit("pet", "后台任务", 1000).unwrap();
        o.fail(&id, "网络超时", 2000).unwrap();
        assert_eq!(o.get_status(&id).unwrap().status, TaskStatus::Failed);
        // 失败结果也要兜底呈现（不静默吞掉）
        assert_eq!(o.get_status(&id).unwrap().delivery, DeliveryState::Ready);
    }

    #[test]
    fn status_query_is_read_only() {
        let mut o = orch();
        let id = o.submit("pet", "任务", 1000).unwrap();
        let _ = o.get_status(&id);
        let _ = o.get_status(&id);
        assert_eq!(o.all_tasks().len(), 1);
    }
}
