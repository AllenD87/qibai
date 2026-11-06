//! 测试辅助：串行化所有读写进程级环境变量的测试，避免并发污染。
#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 获取全局环境变量锁。持锁期间不要 await（避免跨 await 持有 MutexGuard）。
pub fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        // 防 poison：单个测试 panic 不传染后续所有测试。
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// 生成一个测试专用的临时目录（每次调用唯一）。
pub fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("qibai-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
