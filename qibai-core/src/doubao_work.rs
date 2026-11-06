//! 豆包工作 / 办公能力接入设计（架构预留，不在 MVP 实现真实调用）。
//!
//! 目标：后续把飞书文档 / 日历 / 邮件等办公能力作为一组 `SkillProvider`
//! 接入现有技能体系，凭据从 env / 配置运行时读取，绝不硬编码；
//! 未配置凭据时优雅降级并提示用户去设置。
//!
//! 详细架构说明见 `docs/DOUBAO_WORK_INTEGRATION.md`。

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkError {
    #[error("work skill not configured: {0}")]
    NotConfigured(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("upstream error: {0}")]
    Upstream(String),
}

/// 凭据运行时读取：env 优先，其次配置文件。
/// 不缓存任何密钥到长期内存以外的地方；每次调用按需读取。
pub trait WorkCredentialStore: Send + Sync {
    /// 读取某服务的凭据（如 "feishu_doc"、"feishu_calendar"、"feishu_mail"）。
    fn get(&self, service: &str) -> Option<String>;
}

/// 默认实现：env `<SERVICE>_TOKEN`（小写 service 转大写后拼 `_TOKEN`）→ 配置文件同名 key。
pub struct EnvConfigCredentialStore;

impl WorkCredentialStore for EnvConfigCredentialStore {
    fn get(&self, service: &str) -> Option<String> {
        let key = format!("{}_TOKEN", service.to_uppercase().replace('-', "_"));
        std::env::var(&key).ok().filter(|v| !v.is_empty())
    }
}

/// 办公技能扩展 trait：在现有 Skill 体系之外的"工作类"技能抽象。
#[async_trait]
pub trait SkillProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// 是否需要凭据。
    fn requires_credentials(&self) -> bool;
    /// 真实调用（后续实现）。
    async fn invoke(&self, args: &Value) -> Result<Value, WorkError>;
}

/// 包装执行：未配置凭据时返回友好降级文本。
pub async fn invoke_or_degrade<P: SkillProvider>(
    provider: &P,
    creds: &dyn WorkCredentialStore,
    args: &Value,
) -> Result<Value, WorkError> {
    if provider.requires_credentials() && creds.get(provider.id()).is_none() {
        return Err(WorkError::NotConfigured(format!(
            "办公技能「{}」尚未配置凭据。请到设置页绑定对应服务（环境变量或配置文件）。",
            provider.name()
        )));
    }
    provider.invoke(args).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct DummyWork {
        configured: bool,
    }

    #[async_trait]
    impl SkillProvider for DummyWork {
        fn id(&self) -> &str {
            let _ = self.configured; // 消除 unused field 警告（保留字段用于未来装配判断）
            "feishu_doc"
        }
        fn name(&self) -> &str {
            "飞书文档"
        }
        fn requires_credentials(&self) -> bool {
            true
        }
        async fn invoke(&self, _args: &Value) -> Result<Value, WorkError> {
            Ok(json!({"ok": true}))
        }
    }

    struct DummyCreds {
        has: bool,
    }
    impl WorkCredentialStore for DummyCreds {
        fn get(&self, _: &str) -> Option<String> {
            if self.has {
                Some("secret".into())
            } else {
                None
            }
        }
    }

    #[tokio::test]
    async fn degrade_when_no_creds() {
        let p = DummyWork { configured: true };
        let c = DummyCreds { has: false };
        let err = invoke_or_degrade(&p, &c, &json!({})).await.unwrap_err();
        match err {
            WorkError::NotConfigured(msg) => assert!(msg.contains("尚未配置凭据")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn passes_when_creds_present() {
        let p = DummyWork { configured: true };
        let c = DummyCreds { has: true };
        let v = invoke_or_degrade(&p, &c, &json!({})).await.unwrap();
        assert_eq!(v["ok"], true);
    }
}
