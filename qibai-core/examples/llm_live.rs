//! 元气小小 · 大模型真实连通验证（单机部署验收用）
//!
//! 走应用真实代码路径：DoubaoProvider（OpenAI 兼容，可接火山方舟/任何兼容端点）。
//! 用法：
//!   DOUBAO_API_KEY=<key> \
//!   DOUBAO_MODEL=<model> \
//!   [DOUBAO_BASE_URL=<chat/completions 端点>] \
//!   cargo run --example llm_live
//! 默认端点即火山方舟 https://ark.cn-beijing.volces.com/api/v3/chat/completions

use qibai_core::llm_client::{ChatMessage, ChatRequest, DoubaoProvider, LlmProvider};
use qibai_core::settings::LlmSettings;

const DEFAULT_ENDPOINT: &str = "https://ark.cn-beijing.volces.com/api/v3/chat/completions";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("DOUBAO_API_KEY")
        .map_err(|_| "缺少 DOUBAO_API_KEY：请设置环境变量后重试")?;
    let model = std::env::var("DOUBAO_MODEL")
        .unwrap_or_else(|_| "deepseek-v4-pro-ga-260813".to_string());
    let base_url = std::env::var("DOUBAO_BASE_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());

    let settings = LlmSettings {
        provider: "doubao".into(),
        base_url,
        model,
        api_key,
        remember_chat: false,
        backup_providers: Vec::new(),
    };
    let provider = DoubaoProvider::from_settings(&settings)?;

    println!("[元气小小] 正在连接大模型…（{}）", provider_model(&settings));
    let resp = provider
        .chat(ChatRequest {
            system: "你是《元气小小》桌面伴侣·古典佳人，说话温婉简短，带一点古风。".into(),
            messages: vec![ChatMessage::user("深夜还在加班，安慰我一句，不超过40字")],
            tools: None,
            json_object: false,
        })
        .await?;

    let content = resp.content.unwrap_or_default();
    println!("[元气小小] 大模型回复: {}", content.trim());
    println!("[元气小小] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功）");
    Ok(())
}

fn provider_model(s: &LlmSettings) -> &str {
    &s.model
}
