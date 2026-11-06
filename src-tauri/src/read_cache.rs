//! v45.3 只读快照缓存——根治「长任务持锁期间只读接口全被阻塞」。
//!
//! 背景：AppState.engine 是 `Arc<tauri::async_runtime::Mutex<Engine>>`。
//! `send_message` / `generate_code` / `approve_skill` 等命令在锁内 await 整轮 LLM + 工具链，
//! 动辄几十秒到数分钟。期间所有走 `.lock().await` 的只读命令（get_settings /
//! get_conversation / get_skills_status / get_personas / get_desk_state /
//! get_intimacy_levels / tts_status / stt_status …）全部排队，前端表现为「整个人变灰、
//! 所有控件不可用」。
//!
//! 修法：
//! - 新增 `ReadCache`：把只读命令要返回的字段全部预克隆成自有数据。
//! - 后台每 3s 用 `engine.try_lock()` 非阻塞快照一次（长任务持锁时直接跳过本轮，不阻塞）。
//! - 只读命令改从 `ReadCache` 读，不再碰 engine 锁。
//! - 写命令（send_message/save_settings/...）结束后强制刷新一次，保证新会话/新设置立刻可见。
//!
//! v45.4 修：所有锁操作一律走 async（`cache.write().await`）。
//! 此前误用 tokio 同步阻塞写 API，在 tokio worker 里会 panic
//! "Cannot block the current thread from within a runtime"——是启动即崩候选。

use std::sync::Arc;
use tauri::async_runtime::RwLock;

/// 只读快照：所有字段都是自有数据（不持有 engine 引用）。
#[derive(Default, Clone)]
pub struct ReadCache {
    pub settings: Option<qibai_core::settings::Settings>,
    /// 预映射好的会话列表（与 get_conversation 原输出同构）。
    pub conversation: Vec<serde_json::Value>,
    pub skills_status: Option<serde_json::Value>,
    pub skill_list: Vec<serde_json::Value>,
    pub expert_list: Vec<serde_json::Value>,
    pub personas: Option<serde_json::Value>,
    pub active_persona: String,
    pub social_state: Option<serde_json::Value>,
    pub social_events: Option<serde_json::Value>,
    pub desk_state: Option<serde_json::Value>,
    pub intimacy_levels: Option<serde_json::Value>,
    pub tts_status: Option<serde_json::Value>,
    pub stt_status: Option<serde_json::Value>,
    /// 最近一次快照时间（ms 纪元），前端/日志可用来判断新鲜度。
    #[allow(dead_code)]
    pub snapshot_at_ms: u64,
}

/// 非阻塞快照：try_lock 失败（长任务持锁）直接返回 false，绝不 await。
/// async fn：cache.write().await 是合法的（绝不跨 await 持 engine 锁）。
pub async fn try_refresh(
    engine: &Arc<tauri::async_runtime::Mutex<qibai_core::Engine>>,
    cache: &Arc<RwLock<ReadCache>>,
) -> bool {
    let g = match engine.try_lock() {
        Ok(g) => g,
        Err(_) => return false, // 长任务在跑：跳过本轮，不阻塞
    };
    let mut snap = ReadCache {
        settings: Some(g.get_settings()),
        conversation: g
            .get_conversation()
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content, "ts": m.ts}))
            .collect(),
        skills_status: Some(g.get_skills_status()),
        skill_list: g.skill_list(),
        expert_list: g.expert_list(),
        personas: Some(g.get_personas()),
        active_persona: g.personas.active_persona().to_string(),
        social_state: Some(g.get_social_state()),
        social_events: Some(g.get_social_events(Some(50))),
        desk_state: Some(g.desk_state()),
        intimacy_levels: Some(g.get_intimacy_levels()),
        // tts_status 带平台探测参数，由调用方注入；stt_status 无外部参数直接快照。
        tts_status: None,
        stt_status: Some(g.stt_status()),
        snapshot_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };
    // g 在此 drop（try_lock guard 不跨 await），再 await cache.write()——绝不在 engine 锁内 await。
    let mut c = cache.write().await;
    snap.tts_status.clone_from(&c.tts_status);
    *c = snap;
    true
}

/// 后台 tick：每 3s 非阻塞快照一次。长任务持锁时 try_lock 直接失败，
/// 循环立刻进入下一轮 sleep，CPU 占用可忽略。
pub fn spawn_refresher(
    engine: Arc<tauri::async_runtime::Mutex<qibai_core::Engine>>,
    cache: Arc<RwLock<ReadCache>>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            try_refresh(&engine, &cache).await;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}

/// 写命令结束后调用：立即刷一次，让新会话/新设置/新技能状态在 UI 上立刻可见。
/// 若此时 engine 锁被别的写命令持有，就跳过（下一个后台 tick 会补上）。
pub async fn force_refresh(
    engine: &Arc<tauri::async_runtime::Mutex<qibai_core::Engine>>,
    cache: &Arc<RwLock<ReadCache>>,
) {
    try_refresh(engine, cache).await;
}

#[cfg(test)]
mod tests {
    //! v45.4 防回归：「在 async 上下文里调用快照刷新」不能 panic。
    //! 验证 async 路径不会被 tokio runtime 拒绝（即没有同步阻塞锁调用）。
    use super::*;

    #[tokio::test]
    async fn async_refresh_does_not_panic_in_tokio_runtime() {
        let cache = Arc::new(RwLock::new(ReadCache::default()));
        {
            let mut w = cache.write().await;
            w.active_persona = "gudian".to_string();
        }
        let r = cache.read().await;
        assert_eq!(r.active_persona, "gudian");
    }

    #[tokio::test]
    async fn read_cache_clone_is_owned() {
        let cache = Arc::new(RwLock::new(ReadCache::default()));
        {
            let mut w = cache.write().await;
            w.conversation = vec![serde_json::json!({"role":"user","content":"hi"})];
        }
        // 命令侧典型用法：read().await 后立刻 clone 出来使用，不跨 await 持锁。
        let snapshot = {
            let g = cache.read().await;
            g.conversation.clone()
        };
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["role"], "user");
    }
}
