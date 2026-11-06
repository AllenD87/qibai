//! 统一事件协议（Unified Event Protocol 轻量落地）。
//!
//! 借鉴 Qwen-Audio-Agent：应用/设备状态变化统一走事件协议，按响应策略分三类：
//! - **静默更新（SilentUpdate）**：悄悄进上下文，不说话、不打扰用户
//!   （如 desk-changed、env.phase 变化、亲密度微小变动）；
//! - **调度响应（ScheduledResponse）**：排进安全窗口后自然回应
//!   （如提醒到期、后台任务结果）；
//! - **打断响应（InterruptResponse）**：立即介入当前对话
//!   （如 barge-in、紧急提醒）。
//!
//! 本轮只做后端事件源与 schema；前端消费点（订阅 qibai:// 事件的方式、
//! 与 F2 环境表情的联动建议）写成文档，不动前端 6 文件。
//!
//! 保留事件 kind 命名空间：`env.*` / `device.*` / `app.*` / `task.*` / `reminder.*`。

use serde::{Deserialize, Serialize};

/// 事件来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// 应用内部状态（对话/引擎/编排器）。
    Application,
    /// 设备状态（麦克风/播放/桌面环境）。
    Device,
    /// 用户交互（点击/触摸/语音打断）。
    UserInteraction,
    /// 环境状态（久置/高频点击/待机——对应前端 F2 环境表情状态机）。
    Environment,
    /// 系统定时（提醒/看门狗）。
    SystemTimer,
}

/// 响应策略——协议的核心字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsePolicy {
    /// 静默更新：进上下文，不发声。
    SilentUpdate,
    /// 调度响应：等安全窗口再呈现。
    ScheduledResponse,
    /// 打断响应：立即介入。
    InterruptResponse,
}

/// 事件信封：所有状态变化统一形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// 单调 id（调试追踪用）。
    pub id: u64,
    pub source: EventSource,
    /// 保留命名空间：`env.*` / `device.*` / `app.*` / `task.*` / `reminder.*`。
    pub kind: String,
    pub payload: serde_json::Value,
    pub policy: ResponsePolicy,
    /// 去重键：同键在窗口内合并（保留最新），防抖（如高频 desk 变化）。
    pub dedup_key: Option<String>,
    pub ts_ms: u64,
}

/// 有界事件总线（纯内存，测试友好；时钟由调用方注入）。
#[derive(Debug)]
pub struct EventBus {
    history: Vec<EventEnvelope>,
    seq: u64,
    /// 历史上限，防桌宠长跑无限增长。
    max_history: usize,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            seq: 0,
            max_history: 256,
        }
    }
}

/// 保留事件清单（与前端/文档对齐；新增 kind 必须先登记命名空间）。
pub mod reserved {
    /// 环境阶段变化（前端 F2 envPhase: idle/lazy/excited → 反向推回 Rust 上下文）。
    pub const ENV_PHASE_CHANGED: &str = "env.phase_changed";
    /// 桌面/前台窗口变化。
    pub const DESK_CHANGED: &str = "device.desk_changed";
    /// 语音打断（barge-in）。
    pub const BARGE_IN: &str = "user.barge_in";
    /// 任务状态变化（编排器）。
    pub const TASK_STATUS: &str = "task.status_changed";
    /// 提醒到期（对齐现有 qibai://reminder）。
    pub const REMINDER_DUE: &str = "reminder.due";
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发布事件。同 dedup_key 的未消费事件合并为最新一条（防抖）。
    pub fn publish(
        &mut self,
        source: EventSource,
        kind: &str,
        payload: serde_json::Value,
        policy: ResponsePolicy,
        dedup_key: Option<String>,
        now_ms: u64,
    ) -> EventEnvelope {
        self.seq += 1;
        if let Some(key) = &dedup_key {
            if let Some(latest) = self
                .history
                .iter_mut()
                .rev()
                .find(|e| e.dedup_key.as_deref() == Some(key.as_str()))
            {
                latest.kind = kind.to_string();
                latest.payload = payload;
                latest.policy = policy;
                latest.ts_ms = now_ms;
                latest.id = self.seq;
                return latest.clone();
            }
        }
        let env = EventEnvelope {
            id: self.seq,
            source,
            kind: kind.to_string(),
            payload,
            policy,
            dedup_key,
            ts_ms: now_ms,
        };
        self.history.push(env.clone());
        if self.history.len() > self.max_history {
            let drop = self.history.len() - self.max_history;
            self.history.drain(0..drop);
        }
        env
    }

    /// 按策略筛选未读事件（前端轮询/面板用）。
    pub fn by_policy(&self, policy: ResponsePolicy) -> impl Iterator<Item = &EventEnvelope> {
        self.history.iter().filter(move |e| e.policy == policy)
    }

    pub fn history(&self) -> &[EventEnvelope] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn publish_classifies_policy() {
        let mut bus = EventBus::new();
        let e = bus.publish(
            EventSource::Environment,
            reserved::ENV_PHASE_CHANGED,
            json!({"phase": "lazy"}),
            ResponsePolicy::SilentUpdate,
            Some("env-phase".into()),
            1000,
        );
        assert_eq!(e.source, EventSource::Environment);
        assert_eq!(e.policy, ResponsePolicy::SilentUpdate);
        // 静默更新：不进打断队列
        assert!(bus.by_policy(ResponsePolicy::InterruptResponse).next().is_none());
        assert_eq!(bus.by_policy(ResponsePolicy::SilentUpdate).count(), 1);
    }

    #[test]
    fn dedup_coalesces_same_key() {
        let mut bus = EventBus::new();
        bus.publish(
            EventSource::Device,
            reserved::DESK_CHANGED,
            json!({"active": "a"}),
            ResponsePolicy::SilentUpdate,
            Some("desk".into()),
            1000,
        );
        bus.publish(
            EventSource::Device,
            reserved::DESK_CHANGED,
            json!({"active": "b"}),
            ResponsePolicy::SilentUpdate,
            Some("desk".into()),
            1100,
        );
        assert_eq!(bus.history().len(), 1);
        assert_eq!(bus.history()[0].payload["active"], "b");
        assert_eq!(bus.history()[0].ts_ms, 1100);
    }

    #[test]
    fn interrupt_events_are_separate_channel() {
        let mut bus = EventBus::new();
        bus.publish(
            EventSource::UserInteraction,
            reserved::BARGE_IN,
            json!({"reason": "user-barge-in"}),
            ResponsePolicy::InterruptResponse,
            None,
            1000,
        );
        assert_eq!(bus.by_policy(ResponsePolicy::InterruptResponse).count(), 1);
    }

    #[test]
    fn history_is_bounded() {
        let mut bus = EventBus::new();
        for i in 0..300 {
            bus.publish(
                EventSource::Application,
                "app.tick",
                json!({"i": i}),
                ResponsePolicy::SilentUpdate,
                None,
                i,
            );
        }
        assert!(bus.history().len() <= 256);
    }
}
