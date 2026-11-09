//! LLM 客户端：`LlmProvider` trait（跨厂商可切换）+ 豆包 Ark 实现 + agent loop。
//!
//! - 豆包 endpoint: `https://ark.cn-beijing.volces.com/api/v3/chat/completions`（OpenAI 兼容）
//! - API key 解析顺序：环境变量 `DOUBAO_API_KEY` → 配置文件 `settings.llm.api_key`。
//!   代码绝不硬编码真实密钥；未配置时返回结构化错误 `LlmError::MissingApiKey`。
//! - 支持 tools(function calling) 与 response_format=json_object。
//! - agent loop：首请求 → 若返回 tool_calls 则执行技能 → tool message 回填 →
//!   二次请求 → 解析最终 JSON 回复 {reply, emotion, action?}。

use crate::persona::ParsedReply;
use crate::settings::LlmSettings;
use crate::skills::{SkillOutcome, SKILL_EXECUTE_COMMAND};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ENV_API_KEY: &str = "DOUBAO_API_KEY";

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("missing API key: set env {ENV_API_KEY} or settings.llm.api_key")]
    MissingApiKey,
    #[error("http error: {0}")]
    Http(String),
    #[error("api returned error: {0}")]
    Api(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("这一轮我反复试了几次都没成，先停一下（工具循环轮数到顶）")]
    MaxIterations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// JSON 字符串形式的参数。
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

/// 与 LLM 之间往返的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    /// OpenAI 兼容 tools schema。
    pub tools: Option<Vec<serde_json::Value>>,
    /// 是否要求 response_format=json_object。
    pub json_object: bool,
}

pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// v45.9：真实 token usage（provider 返回时）；None 表示 provider 未返回，调用方估算。
    pub tokens_used: Option<u32>,
}

impl Default for ChatResponse {
    fn default() -> Self {
        Self { content: None, tool_calls: vec![], tokens_used: None }
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
    /// v18.8-fix27：流式对话——逐段回调文本（供前端分段朗读）。
    /// 默认实现走非流式 chat（一次性回调全文）；实现方覆写为真流式（SSE）。
    async fn chat_stream(
        &self,
        req: ChatRequest,
        on_text: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<ChatResponse, LlmError> {
        let _ = on_text; // 默认实现不回调（真流式由实现方覆写）；仅保证签名一致
        self.chat(req).await
    }
}

/// v18.8-fix27：按句子边界（。！？；\n）从缓冲中切出分段文本并回调（供流式朗读）。
/// 缓冲中未到边界的内容保留；调用方在流结束后把剩余部分作为一段回调。
pub fn flush_segments(buf: &mut String, on_text: &mut dyn FnMut(&str)) {
    // v18.8-fix28d：改用 char_indices 按字符边界切分，避免 `buf[..=i]` 在
    // 多字节字符（中文标点）中间切出 panic（真机端 byte index not a char boundary）。
    loop {
        let cut = buf
            .char_indices()
            .find(|(_, c)| "。！？；\n".contains(*c))
            .map(|(i, c)| i + c.len_utf8());
        match cut {
            Some(end) => {
                let seg = buf[..end].trim().to_string();
                if !seg.is_empty() {
                    on_text(&seg);
                }
                buf.drain(..end);
            }
            None => break,
        }
    }
}

/// v45.9：无 LLM key 时的兜底 provider（返回 MissingApiKey，executor 会重试后落 failed）。
pub struct NullProvider;

#[async_trait]
impl LlmProvider for NullProvider {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::MissingApiKey)
    }
}

/// 豆包 Ark 实现（OpenAI 兼容）。
pub struct DoubaoProvider {
    endpoint: String,
    model: String,
    api_key: String,
    /// v18.8-fix24：备用服务商（欠费/失败自动切换；空=无备用）。
    backups: Vec<BackupEndpoint>,
    /// 问题7性能路由：共享 reqwest Client（连接池/TLS 上下文复用），
    /// 替代此前每请求 new Client 的反模式。
    client: reqwest::Client,
}

/// 备用端点（不同服务商/账号）。
#[derive(Debug, Clone)]
pub struct BackupEndpoint {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

/// OpenAI 兼容层 model id 发送前规范化。
///
/// v31.2：按用户要求不再自动补 `Qwen/` 前缀——模型名原样发送（如
/// `Qwen3.8-Flash-Next` 就发 `Qwen3.8-Flash-Next`）。带 `/` 的
/// 完整 id（如 `Qwen/Qwen3.6-35B-A3B`）同样原样保留。仅 trim 前后空白。
pub fn normalize_model_id(raw: &str) -> String {
    raw.trim().to_string()
}

/// v18.8-fix24：欠费/余额不足类错误识别（账户级问题，需换服务商或提示用户充值）。
pub fn is_balance_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("overdue balance")
        || m.contains("insufficient balance")
        || m.contains("overdue")
        || m.contains("arrears")
        || m.contains("欠费")
        || m.contains("余额不足")
}

impl DoubaoProvider {
    /// 从设置构造；密钥解析顺序：env `DOUBAO_API_KEY` → settings.llm.api_key。
    pub fn from_settings(s: &LlmSettings) -> Result<Self, LlmError> {
        let backups = s
            .backup_providers
            .iter()
            .filter(|b| {
                !b.base_url.trim().is_empty() && !b.model.trim().is_empty() && !b.api_key.trim().is_empty()
            })
            .map(|b| BackupEndpoint {
                name: b.name.clone(),
                endpoint: b.base_url.clone(),
                // 问题5：备用 provider model id 同样发送前规范化。
                model: normalize_model_id(&b.model),
                api_key: b.api_key.clone(),
            })
            .collect::<Vec<_>>();
        let mut p = Self::from_settings_inner(s)?;
        p.backups = backups;
        Ok(p)
    }

    fn from_settings_inner(s: &LlmSettings) -> Result<Self, LlmError> {
        // v18.8-fix27：key 优先级——settings 显式配置优先（如本地 Qwen key），env 兜底（如火山方舟）。
        let key = if !s.api_key.trim().is_empty() {
            Some(s.api_key.clone())
        } else {
            std::env::var(ENV_API_KEY).ok().filter(|k| !k.trim().is_empty())
        };
        let Some(api_key) = key else {
            return Err(LlmError::MissingApiKey);
        };
        // v18.8-fix27：端点智能拼接——base_url 为根（OpenAI 兼容）时自动补 /chat/completions；
        // 已是完整端点（火山方舟）则原样使用。
        let base = s.base_url.trim().trim_end_matches('/').to_string();
        let endpoint = if base.ends_with("/chat/completions") {
            base
        } else {
            format!("{base}/chat/completions")
        };
        // v31.2：model id 发送前规范化——原样保留（不再自动补 Qwen/ 前缀）。
        let model = normalize_model_id(&s.model);
        eprintln!(
            "[qibai] llm 配置: provider={} 端点={} 模型={} key={}",
            s.provider,
            endpoint,
            model,
            if s.api_key.trim().is_empty() { "未配置(将用env)" } else { "已配置" }
        );
        // 问题7性能路由：构造一次共享 Client（连接池复用）。总超时 120s 兜底；
        // 流式另有 chunk 级 30s/累计 120s 超时（见 chat_stream），非流式 post_json 沿用此上限。
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::Api(format!("HTTP 客户端构建失败: {e}")))?;
        Ok(Self {
            endpoint,
            model,
            api_key,
            backups: Vec::new(),
            client,
        })
    }
}

#[async_trait]
impl LlmProvider for DoubaoProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        // v18.8-fix26：单模型测试——只调用用户配置的模型（不做多模型回退链），
        // 失败原因（欠费/无权限/网络）原样返回，便于测试阶段精确定位。
        let mut last_err = match self.chat_with_model(&self.model, &req).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                eprintln!("[qibai] llm model '{}' 失败: {}", self.model, e);
                e
            }
        };
        // v18.8-fix24：配置了备用服务商时，主模型失败自动切换（欠费/无权限等）。
        for b in &self.backups {
            let alt = DoubaoProvider {
                endpoint: b.endpoint.clone(),
                model: b.model.clone(),
                api_key: b.api_key.clone(),
                backups: Vec::new(),
                client: self.client.clone(), // 问题7：复用共享 client
            };
            match alt.chat_with_model(&b.model, &req).await {
                Ok(r) => {
                    eprintln!("[qibai] llm failover -> {}", b.name);
                    return Ok(r);
                }
                Err(e) => {
                    eprintln!("[qibai] llm 备用 {} 也失败: {}", b.name, e);
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    /// v18.8-fix27：trait 流式实现——单模型（用户配置），SSE 逐段回调文本。
    async fn chat_stream(
        &self,
        req: ChatRequest,
        on_text: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<ChatResponse, LlmError> {
        let model = &self.model;
        let mut messages = vec![ChatMessage::system(req.system.clone())];
        messages.extend(req.messages.clone());
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });
        if let Some(tools) = &req.tools {
            body["tools"] = serde_json::json!(tools);
            body["tool_choice"] = serde_json::json!("auto");
        }
        if req.json_object {
            // v18.8-fix28e：流式与非流式保持一致——Qwen 思考型对 json_object 不兼容
            // （会输出空/{} 或干扰 tool_calls），同样跳过。
            let m = model.to_lowercase();
            let is_deepseek = m.contains("deepseek");
            let is_qwen = m.contains("qwen");
            if !is_deepseek && !is_qwen {
                body["response_format"] = serde_json::json!({"type": "json_object"});
            }
        }
        // v18.8-fix28e：max_tokens 上限（Qwen 思考型默认思考很长，防止请求长时间挂起）
        body["max_tokens"] = serde_json::json!(1000);

        // 问题7性能路由：复用共享 client（连接池/TLS 上下文），不再每请求新建。
        let client = &self.client;
        let resp = client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Api(format!("流式请求失败: {e}")))?;
        let status = resp.status();
        eprintln!("[qibai] llm 流式响应 status={status} 端点={}", self.endpoint);
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!(
                "流式 HTTP {status}: {}",
                err_body.chars().take(200).collect::<String>()
            )));
        }
        let stream = resp.bytes_stream();
        futures_util::pin_mut!(stream);

        let mut full_text = String::new();
        let mut seg_buf = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut line_buf: Vec<u8> = Vec::new();
        let mut usage: Option<u32> = None;
        let started = std::time::Instant::now();

        // v18.8-fix28：流式循环加超时——每 chunk 30s 无数据即中断返回 Err（触发降级非流式），
        // 累计 120s 硬限。防止服务流式响应异常（不结束/格式不同）时卡死监听循环。
        loop {
            // v18.8-fix28d：Qwen 思考型模型 reasoning 阶段可能长时间无 content chunk，
            // 超时放宽到 60s，避免思考中误判降级（累计 120s 硬限仍兜底）。
            let next = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                stream.next(),
            )
            .await;
            let chunk = match next {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => {
                    eprintln!("[qibai] llm 流式 60s 无数据，中断（服务流式异常？），降级非流式");
                    return Err(LlmError::Api("流式响应 60s 无数据".into()));
                }
            };
            if started.elapsed() > std::time::Duration::from_secs(120) {
                eprintln!("[qibai] llm 流式累计超 120s，中断");
                return Err(LlmError::Api("流式响应超 120s".into()));
            }
            let chunk = chunk.map_err(|e| LlmError::Api(format!("流式读取失败: {e}")))?;
            for &b in chunk.iter() {
                line_buf.push(b);
                if b == b'\n' {
                    let line = String::from_utf8_lossy(&line_buf).trim().to_string();
                    line_buf.clear();
                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim();
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                            if let Some(u) = parse_usage(&v) { usage = Some(u); }
                            if let Some(delta) = v.get("choices").and_then(|c| c.get(0)).and_then(|c| c.get("delta")) {
                                // v18.8-fix34：Qwen 等推理模型流式输出 content 恒为空（思考全在
                                // reasoning_content），等完整流式只会空耗时间；收到首个
                                // reasoning_content 即中断流式、返回 content=None，
                                // 由上层走「流式 content 无效 → 降级非流式」，一次调用拿全 JSON。
                                if let Some(rc) = delta.get("reasoning_content").and_then(|x| x.as_str()) {
                                    if !rc.trim().is_empty() && full_text.is_empty() {
                                        eprintln!("[qibai] 流式收到 reasoning_content（推理模型），跳过流式直接降级非流式");
                                        return Ok(ChatResponse {
                                            content: None,
                                            tool_calls, tokens_used: usage });
                                    }
                                }
                                if let Some(c) = delta.get("content").and_then(|x| x.as_str()) {
                                    full_text.push_str(c);
                                    seg_buf.push_str(c);
                                    flush_segments(&mut seg_buf, on_text);
                                }
                                if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                                    for tc in tcs {
                                        let idx = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                                        let id = tc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                        let name = tc.get("function").and_then(|f| f.get("name")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                                        let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                kind: "function".to_string(),
                                                function: ToolCallFunction { name: String::new(), arguments: String::new() },
                                            });
                                        }
                                        if !id.is_empty() { tool_calls[idx].id = id; }
                                        if !name.is_empty() { tool_calls[idx].function.name = name; }
                                        tool_calls[idx].function.arguments.push_str(&args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // 尾部未分段文本作为一段回调
        if !seg_buf.trim().is_empty() {
            on_text(seg_buf.trim());
        }
        Ok(ChatResponse {
            content: if full_text.is_empty() { None } else { Some(full_text) },
            tool_calls, tokens_used: usage })
    }
}

impl DoubaoProvider {
    async fn chat_with_model(
        &self,
        model: &str,
        req: &ChatRequest,
    ) -> Result<ChatResponse, LlmError> {
        let mut messages = vec![ChatMessage::system(req.system.clone())];
        messages.extend(req.messages.clone());

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
        });
        if let Some(tools) = &req.tools {
            body["tools"] = serde_json::json!(tools);
            body["tool_choice"] = serde_json::json!("auto");
        }
        if req.json_object {
            // v18.8-fix19：DeepSeek 系列（含经济版）对 response_format=json_object 不兼容，
            // 会返回 200 空 body（EOF 解析错）。system prompt 已有「必须输出 JSON」文本约束兜底，
            // 故对 DeepSeek 模型跳过该字段。
            // v18.8-fix28：本地 Qwen（vLLM/兼容层）同样跳过——可能返回 400 空 body。
            let m = model.to_lowercase();
            let is_deepseek = m.contains("deepseek");
            let is_qwen = m.contains("qwen");
            if !is_deepseek && !is_qwen {
                body["response_format"] = serde_json::json!({"type": "json_object"});
            }
        }
        // v18.8-fix28e：max_tokens 上限（与流式一致，防止思考型模型长时间挂起）
        body["max_tokens"] = serde_json::json!(1000);

        // v18.8-fix18：带工具请求失败/空响应 → 降级不带工具重试
        // （DeepSeek 经济版等模型可能不支持函数调用，返回空响应导致 EOF 解析错）。
        let value = match self.post_json(body.clone()).await {
            Ok(v) => v,
            Err(e) => {
                if req.tools.is_some() {
                    let mut b2 = body.clone();
                    if let Some(obj) = b2.as_object_mut() {
                        obj.remove("tools");
                        obj.remove("tool_choice");
                    }
                    match self.post_json(b2).await {
                        Ok(v2) => v2,
                        Err(_e2) => return Err(e),
                    }
                } else {
                    return Err(e);
                }
            }
        };

        let choice = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let content = choice.get("content").and_then(|c| c.as_str()).map(|s| s.to_string());
        let tool_calls = choice
            .get("tool_calls")
            .and_then(|t| serde_json::from_value::<Vec<ToolCall>>(t.clone()).ok())
            .unwrap_or_default();

        let tokens_used = parse_usage(&value);
        Ok(ChatResponse { content, tool_calls, tokens_used })
    }
}

/// v45.9：从 OpenAI 兼容响应的 `usage` 对象解析 total_tokens；没有则 None（上层估算）。
pub(crate) fn parse_usage(v: &serde_json::Value) -> Option<u32> {
    let u = v.get("usage")?;
    u.get("total_tokens")
        .and_then(|x| x.as_u64())
        .or_else(|| u.get("total_tokens").and_then(|x| x.as_f64()).map(|f| f as u64))
        .map(|n| n as u32)
}

impl DoubaoProvider {
    /// 发一次 Chat Completions 请求并解析 JSON；空响应返回 Api 错误（供上层降级）。
    async fn post_json(&self, body: serde_json::Value) -> Result<serde_json::Value, LlmError> {
        // 问题7性能路由：复用共享 client（120s 总超时兜底，防服务挂起卡死监听循环）。
        let client = &self.client;
        let resp = client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| LlmError::Http(e.to_string()))?;
        eprintln!(
            "[qibai] llm 非流式 status={status} body前200={}",
            text.chars().take(200).collect::<String>().replace("\n", " ")
        );
        if text.trim().is_empty() {
            return Err(LlmError::Api(format!("empty response (status {status})")));
        }
        let value: serde_json::Value = {
            let trimmed = text.trim();
            match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => {
                    // v26-fix43：截断 JSON 容错（fix33 扩展至非流式路径）。
                    // 1) 缺右花括号 → 补全重试；2) 非 JSON 纯文本回复 → 原样作为 reply 交给 persona 层解析
                    if let Ok(v) = serde_json::from_str(&format!("{trimmed}}}")) {
                        v
                    } else if status.is_success() {
                        let mut m = serde_json::Map::new();
                        m.insert("reply".to_string(), serde_json::Value::String(trimmed.to_string()));
                        serde_json::Value::Object(m)
                    } else {
                        return Err(LlmError::Api(format!("http {status}: {trimmed}")));
                    }
                }
            }
        };
        if !status.is_success() {
            let msg = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown api error");
            return Err(LlmError::Api(msg.to_string()));
        }
        Ok(value)
    }
}

/// 把 SkillOutcome 转成回填给 LLM 的 tool message 文本。
pub fn outcome_to_tool_text(outcome: &SkillOutcome) -> String {
    match outcome {
        SkillOutcome::Ok(msg) => format!("SUCCESS: {msg}"),
        SkillOutcome::PendingApproval { request_id, description } => {
            format!("PENDING_APPROVAL: request_id={request_id}, {description}. 请告知用户需要其确认后才能执行。")
        }
        // v45.8：路径类 Denied 自动附带工作区根，避免模型下一轮又猜同样路径。
        SkillOutcome::Denied(reason) => {
            let is_path_denial = reason.contains("不在工作区根")
                || reason.contains("越界")
                || reason.contains("白名单根")
                || reason.contains("拒绝：")
                || reason.contains("shell 元字符");
            if is_path_denial {
                let root = crate::engine::qibai_workspace_root_public();
                format!(
                    "DENIED: {reason}\n你当前可用的工作区根是 {}，请改用相对路径（如 demo_proj/src/main.py）；不要反复重试同一路径。",
                    root.display()
                )
            } else {
                format!("DENIED: {reason}")
            }
        }
    }
}

/// 工具执行抽象：run_agent_loop 通过它执行 LLM 发起的 tool_calls。
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn invoke(&mut self, name: &str, args: &serde_json::Value) -> SkillOutcome;
}

/// 工具执行留痕（给前端显示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecutionRecord {
    pub tool: String,
    pub args_summary: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// 承诺—落实校验：返回 Some(替换文案) 表示命中假完成。
/// 判据：① 含完成语 ② 含动作对象 ③ 本轮无成功工具调用。
pub fn false_completion_guard(reply: &str, tools_ok: u32) -> Option<String> {
    if tools_ok > 0 { return None; }
    const DONE_WORDS: &[&str] = &["已完成","已经建好","建好了","已创建","已经写好","写好了","已经生成","生成好了","搞定了","弄好了","已删除","已修改"];
    const OBJECT_WORDS: &[&str] = &["项目","文件","目录","文件夹","代码","网站","页面","文档","仓库","脚本","配置"];
    let has_done = DONE_WORDS.iter().any(|w| reply.contains(w));
    let has_obj = OBJECT_WORDS.iter().any(|w| reply.contains(w));
    if has_done && has_obj {
        let hit = DONE_WORDS.iter().find(|w| reply.contains(**w)).copied().unwrap_or("");
        let obj = OBJECT_WORDS.iter().find(|w| reply.contains(**w)).copied().unwrap_or("");
        eprintln!("[qibai] guard: false-completion blocked claims={hit} object={obj} tools_ok=0");
        Some(format!("我没有真正执行这一步——本轮没有成功的工具调用，所以{obj}还没有动过。要我现在按在 ~/Documents/qibai 下建骨架走一遍吗？"))
    } else {
        None
    }
}

/// 产物存在性原语（聊天路径与 task 路径共用，不复制两份）：
/// - 文件：必须 `is_file()` 且 `metadata.len() > 0`（0 字节不算真产物）；
/// - 目录：必须 `is_dir()` 且 `read_dir` 不报错（可读即存在）。
///
/// Err 文案统一以「声称完成但产物不存在：{p}」开头；聊天路径会在前面再缀
/// 「产物校验不通过：」前缀（见 [`assert_artifact_exists`]），task 路径直接用。
pub fn verify_artifact_at(p: &std::path::Path) -> Result<(), String> {
    let missing = |detail: &str| format!("声称完成但产物不存在：{} ({detail})", p.display());
    if p.is_dir() {
        match std::fs::read_dir(p) {
            Ok(_) => Ok(()),
            Err(e) => Err(missing(&format!("目录不可读: {e}"))),
        }
    } else if p.is_file() {
        match std::fs::metadata(p) {
            Ok(m) if m.len() > 0 => Ok(()),
            _ => Err(missing("文件为空")),
        }
    } else {
        Err(missing("路径不存在"))
    }
}

/// 产物断言：code_write_file / mkdir 成功后真检查产物存在（聊天路径）。
pub fn assert_artifact_exists(tool: &str, args: &serde_json::Value) -> Result<(), String> {
    use crate::engine::qibai_workspace_root;
    let root = qibai_workspace_root();
    if tool == "code_write_file" {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(p) = crate::engine::safe_resolve_pub(&root, path) {
            if let Err(e) = verify_artifact_at(&p) {
                return Err(format!("产物校验不通过：{e}"));
            }
        }
    }
    if tool == "code_run_command" {
        let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(rest) = cmd.strip_prefix("mkdir ") {
            let dir = rest.trim();
            if let Some(p) = crate::engine::safe_resolve_pub(&root, dir) {
                if let Err(e) = verify_artifact_at(&p) {
                    return Err(format!("产物校验不通过：{e}"));
                }
            }
        }
    }
    Ok(())
}

/// 执行 agent loop：返回最终解析回复、过程中执行过的最后一个技能名、留痕。
pub async fn run_agent_loop(
    provider: &dyn LlmProvider,
    system: &str,
    history: Vec<ChatMessage>,
    tools: Option<Vec<serde_json::Value>>,
    executor: &mut dyn ToolExecutor,
    on_text: &mut (dyn for<'a> FnMut(&'a str) + Send),
) -> Result<(ParsedReply, Option<String>, Vec<ExecutionRecord>), LlmError> {
    let mut messages = history;
    let mut last_skill: Option<String> = None;
    let mut executions: Vec<ExecutionRecord> = Vec::new();
    let mut tools_ok: u32 = 0;
    // v45.7：同一工具连续 Denied 计数（≥2 次直接终止，避免鬼打墙）。
    let mut last_denied_tool: Option<String> = None;
    let mut denied_streak: u32 = 0;

    // v18.8-fix28e 的 5 轮 / 120s 硬限，会把「真出片」这类长链路砍断：
    // animacode 一条 15s 片子要连跑 写 spec → schema → build → render(≈50s) → qa → mux，
    // 光工具耗时就要 60–80s，第 5 轮前必被截断，用户看到的是「跑一半没下文」。
    // 放宽到 8 轮 / 240s；单轮仍是流式 + 120s chunk 超时，不会无限挂。
    const LOOP_MAX_ROUNDS: usize = 8;
    const LOOP_MAX_SECS: u64 = 240;
    let loop_deadline = std::time::Instant::now();
    for _ in 0..LOOP_MAX_ROUNDS {
        if loop_deadline.elapsed() > std::time::Duration::from_secs(LOOP_MAX_SECS) {
            eprintln!(
                "[qibai] llm 工具循环累计超 {LOOP_MAX_SECS}s，终止（已执行工具见 executions）"
            );
            return Err(LlmError::Api(format!(
                "LLM 工具循环超时（累计 {LOOP_MAX_SECS}s）"
            )));
        }
        let req = ChatRequest {
            system: system.to_string(),
            messages: messages.clone(),
            tools: tools.clone(),
            json_object: true,
        };
        // v18.8-fix27：流式优先；服务不支持流式/流式失败 → 降级非流式（trait 默认实现走 chat）。
        let resp = match provider.chat_stream(req.clone(), on_text).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[qibai] llm 流式失败，降级非流式: {e}");
                provider.chat(req.clone()).await?
            }
        };

        if resp.tool_calls.is_empty() {
            // 最终回复
            let content = resp.content.unwrap_or_default();
            eprintln!(
                "[qibai] llm 原始content前60={}",
                content.chars().take(60).collect::<String>().replace("\n", " ")
            );
            // v18.8-fix28d：流式 content 为空或只是空 JSON（Qwen 思考型常见输出 {}）
            // 时视为无效 → 降级非流式重试一次（避免"有响应却无回复"的假成功）。
            let trimmed_c = content.trim();
            if trimmed_c.is_empty() || trimmed_c == "{}" || trimmed_c == "null" {
                eprintln!("[qibai] 流式 content 无效(空/{{}}),降级非流式重试");
                let resp2 = provider.chat(req.clone()).await?;
                if resp2.tool_calls.is_empty() {
                    let c2 = resp2.content.unwrap_or_default();
                    eprintln!(
                        "[qibai] llm 非流式 content前60={}",
                        c2.chars().take(60).collect::<String>().replace("\n", " ")
                    );
                    let mut parsed = crate::persona::parse_reply(&c2)
                        .map_err(|e| LlmError::Api(e.to_string()))?;
                    // 假完成闸门
                    if let Some(fixed) = false_completion_guard(&parsed.reply, tools_ok) {
                        parsed.reply = fixed;
                        parsed.false_completion = true;
                    }
                    eprintln!("[qibai] llm 解析回复前40={}", parsed.reply.chars().take(40).collect::<String>());
                    return Ok((parsed, last_skill, executions));
                }
                let content = resp2.content.unwrap_or_default();
                let mut parsed = crate::persona::parse_reply(&content)
                    .map_err(|e| LlmError::Api(e.to_string()))?;
                if let Some(fixed) = false_completion_guard(&parsed.reply, tools_ok) {
                    parsed.reply = fixed;
                    parsed.false_completion = true;
                }
                eprintln!("[qibai] llm 解析回复前40={}", parsed.reply.chars().take(40).collect::<String>());
                return Ok((parsed, last_skill, executions));
            }
            let mut parsed = crate::persona::parse_reply(&content)
                .map_err(|e| LlmError::Api(e.to_string()))?;
            if let Some(fixed) = false_completion_guard(&parsed.reply, tools_ok) {
                parsed.reply = fixed;
                parsed.false_completion = true;
            }
            eprintln!("[qibai] llm 解析回复前40={}", parsed.reply.chars().take(40).collect::<String>());
            return Ok((parsed, last_skill, executions));
        }

        // 有 tool_calls：先把 assistant 消息（含 tool_calls）加入对话
        messages.push(ChatMessage::assistant_tool_calls(resp.tool_calls.clone()));
        for call in &resp.tool_calls {
            let skill = call.function.name.clone();
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
            let mut outcome = executor.invoke(&skill, &args).await;
            // 产物断言：code_write_file / mkdir 成功后真检查产物
            if let SkillOutcome::Ok(_) = &outcome {
                if let Err(e) = assert_artifact_exists(&skill, &args) {
                    outcome = SkillOutcome::Denied(e);
                }
            }
            let args_summary = serde_json::to_string(&args).unwrap_or_default();
            let args_summary: String = args_summary.chars().take(40).collect();
            let is_ok = matches!(outcome, SkillOutcome::Ok(_));
            let err = match &outcome {
                SkillOutcome::Denied(r) => Some(r.clone()),
                SkillOutcome::PendingApproval { .. } => Some("pending_approval".into()),
                _ => None,
            };
            executions.push(ExecutionRecord {
                tool: skill.clone(), args_summary, ok: is_ok, error: err,
            });
            if is_ok { tools_ok += 1; }
            if skill != SKILL_EXECUTE_COMMAND {
                last_skill = Some(skill.clone());
            }
            // v45.7：PendingApproval 立刻结束本轮（等用户在控制台点确认）；
            // Denied 不立即中断——第一次喂回模型让它自己修，连续同一工具被拒 ≥2 次也仍回灌（不再直接终止）。
            match &outcome {
                SkillOutcome::PendingApproval { .. } => {
                    return Ok((
                        ParsedReply {
                            reply: "这个操作需要你在控制台点一下确认，我在这儿等你～".to_string(),
                            emotion: crate::enums::Emotion::Worried,
                            action: None,
                            ui_cmd: None,
                            false_completion: false,
                        },
                        last_skill,
                        executions,
                    ));
                }
                SkillOutcome::Denied(_reason) => {
                    if last_denied_tool.as_deref() == Some(skill.as_str()) {
                        denied_streak += 1;
                    } else {
                        last_denied_tool = Some(skill.clone());
                        denied_streak = 1;
                    }
                    // v45.10：≥2 次 Denied 也不直接终止，继续回灌让模型换路径/改口
                    if denied_streak >= 2 {
                        eprintln!("[qibai] tool: {skill} denied {denied_streak} times, still feeding back");
                    }
                }
                SkillOutcome::Ok(_) => {
                    last_denied_tool = None;
                    denied_streak = 0;
                }
            }
            messages.push(ChatMessage::tool(
                call.id.clone(),
                outcome_to_tool_text(&outcome),
            ));
        }
    }
    eprintln!("[qibai] agent-loop max iterations reached (5 rounds)");
    Err(LlmError::MaxIterations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SkillOutcome;
    use std::sync::Mutex;

    /// v18.8-fix24：欠费/余额不足类错误识别。
    #[test]
    fn balance_error_detection() {
        assert!(is_balance_error("The request failed because your account has an overdue balance."));
        assert!(is_balance_error("insufficient balance"));
        assert!(is_balance_error("账号欠费"));
        assert!(is_balance_error("余额不足，请充值"));
        assert!(!is_balance_error("connection reset"));
        assert!(!is_balance_error("rate limit exceeded"));
    }

    /// v18.8-fix24：settings 备用服务商配置反序列化 + remember_chat 默认 false。
    #[test]
    fn llm_settings_backup_and_remember_roundtrip() {
        use crate::settings::LlmSettings;
        let json = r#"{
            "provider":"doubao",
            "base_url":"https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            "model":"deepseek-v4-1-flash-261017",
            "backup_providers":[
                {"name":"deepseek官方","base_url":"https://api.deepseek.com/chat/completions","model":"deepseek-chat","api_key":"sk-abc"}
            ]
        }"#;
        let st: LlmSettings = serde_json::from_str(json).unwrap();
        assert_eq!(st.model, "deepseek-v4-1-flash-261017");
        // remember_chat 未提供 → 默认 false（测试阶段不带历史）
        assert!(!st.remember_chat);
        assert_eq!(st.backup_providers.len(), 1);
        let b = &st.backup_providers[0];
        assert_eq!(b.name, "deepseek官方");
        assert_eq!(b.base_url, "https://api.deepseek.com/chat/completions");
        assert_eq!(b.model, "deepseek-chat");
        assert_eq!(b.api_key, "sk-abc");
    }

    /// v31.2：model id 原样发送（不再自动补 Qwen/ 前缀）；仅 trim 前后空白。
    #[test]
    fn normalize_model_id_cases() {
        // 1) 无前缀模型名 → 原样（不再补 Qwen/）
        assert_eq!(normalize_model_id("Qwen3.8-Flash-Next"), "Qwen3.8-Flash-Next");
        assert_eq!(normalize_model_id("Qwen3.6-35B-A3B"), "Qwen3.6-35B-A3B");
        // 2) 已带前缀 → 原样不动
        assert_eq!(normalize_model_id("Qwen/Qwen3.6-35B-A3B"), "Qwen/Qwen3.6-35B-A3B");
        // 3) 非 qwen 模型 → 不动
        assert_eq!(normalize_model_id("deepseek-chat"), "deepseek-chat");
        // 4) 前后空白 trim
        assert_eq!(normalize_model_id("  qwen3-32b  "), "qwen3-32b");
        // 5) 用户自定义别名 → 原样
        assert_eq!(normalize_model_id("my-local-llm-v2"), "my-local-llm-v2");
    }

    /// Mock provider：脚本化返回若干轮。
    struct MockProvider {
        scripted: Mutex<std::collections::VecDeque<Result<ChatResponse, LlmError>>>,
        seen_bodies: Mutex<Vec<ChatRequest>>,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
            self.seen_bodies.lock().unwrap().push(req);
            self.scripted.lock().unwrap().pop_front().unwrap_or_else(|| {
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"好的","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                })
            })
        }
    }

    #[test]
    fn missing_key_is_structured_error() {
        let _g = crate::test_support::env_lock();
        let mut s = LlmSettings::default();
        s.api_key = String::new();
        // 确保环境变量不在
        unsafe { std::env::remove_var(ENV_API_KEY) };
        let result = DoubaoProvider::from_settings(&s);
        assert!(matches!(result.err(), Some(LlmError::MissingApiKey)));
    }

    #[test]
    fn config_key_takes_precedence() {
        // v18.8-fix27：key 优先级反转——settings 显式配置优先（本地 Qwen 场景），env 仅兜底。
        let _g = crate::test_support::env_lock();
        let mut s = LlmSettings::default();
        s.api_key = "from-config".to_string();
        unsafe { std::env::set_var(ENV_API_KEY, "from-env") };
        let p = DoubaoProvider::from_settings(&s).unwrap();
        assert_eq!(p.api_key, "from-config");
        unsafe { std::env::remove_var(ENV_API_KEY) };
    }

    #[tokio::test]
    async fn agent_loop_executes_tool_then_final_reply() {
        // 第一轮返回 tool_call；第二轮返回最终 JSON。
        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "launch_app".into(),
                            arguments: r#"{"target":"firefox"}"#.into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"已经帮你打开浏览器啦","emotion":"happy","action":"wave"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };

        struct Rec(std::sync::Mutex<Vec<(String, serde_json::Value)>>);
        #[async_trait]
        impl ToolExecutor for Rec {
            async fn invoke(&mut self, name: &str, args: &serde_json::Value) -> SkillOutcome {
                self.0.lock().unwrap().push((name.to_string(), args.clone()));
                SkillOutcome::Ok("launched firefox".into())
            }
        }
        let mut exec = Rec(std::sync::Mutex::new(vec![]));

        let (parsed, skill, _exec) = run_agent_loop(
            &mock,
            "system-prompt",
            vec![ChatMessage::user("打开浏览器")],
            Some(crate::skills::tool_schemas()),
            &mut exec,
            &mut |_seg: &str| {},
        )
        .await
        .unwrap();

        assert_eq!(parsed.emotion, crate::enums::Emotion::Happy);
        assert_eq!(parsed.action, Some(crate::enums::Action::Wave));
        assert_eq!(skill.as_deref(), Some("launch_app"));
        assert_eq!(exec.0.lock().unwrap().len(), 1);

        // 验证请求体带了 response_format=json_object（两轮都是）
        let seen = mock.seen_bodies.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].json_object);
        assert!(seen[1].json_object);
        // 第二轮 messages 里应包含 tool 回填
        let second_msgs = &seen[1].messages;
        assert!(second_msgs.iter().any(|m| m.role == "tool"));
    }

    #[tokio::test]
    async fn agent_loop_passes_denied_back_as_tool_message() {
        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call-2".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "hack_nasa".into(),
                            arguments: "{}".into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"这个技能我没有权限哦","emotion":"shy"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };

        struct Deny;
        #[async_trait]
        impl ToolExecutor for Deny {
            async fn invoke(&mut self, _name: &str, _args: &serde_json::Value) -> SkillOutcome {
                SkillOutcome::Denied("skill not in whitelist".into())
            }
        }
        let mut exec = Deny;

        let (parsed, _, _exec) = run_agent_loop(
            &mock,
            "sys",
            vec![ChatMessage::user("hack")],
            None,
            &mut exec,
            &mut |_seg: &str| {},
        )
        .await
        .unwrap();
        assert_eq!(parsed.emotion, crate::enums::Emotion::Shy);
    }

    /// 每个用例绑定独立临时端口，避免依赖外部 18081 服务或并发测试互相抢端口。
    async fn spawn_mock_llm_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let body_start = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 { return; }
                        data.extend_from_slice(&chunk[..n]);
                        let Some(header_pos) = data.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                        let start = header_pos + 4;
                        let headers = String::from_utf8_lossy(&data[..header_pos]);
                        let len = headers.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") { value.trim().parse::<usize>().ok() } else { None }
                        }).unwrap_or(0);
                        if data.len() >= start + len { break start; }
                    };
                    let body: serde_json::Value = serde_json::from_slice(&data[body_start..]).unwrap_or_default();
                    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
                    let streaming = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
                    let (content_type, payload) = if streaming {
                        let delta = if model.contains("reasoning") {
                            serde_json::json!({"reasoning_content":"先思考一下"})
                        } else {
                            serde_json::json!({"content":"{\"reply\":\"你好\",\"emotion\":\"happy\"}"})
                        };
                        let event = serde_json::json!({"choices":[{"delta":delta}]});
                        ("text/event-stream", format!("data: {event}\n\ndata: [DONE]\n\n"))
                    } else {
                        let content = if model.contains("truncated") {
                            r#"{"reply":"今天天气不错","emotion":"happy""#
                        } else {
                            r#"{"reply":"元气小小收到","emotion":"happy"}"#
                        };
                        ("application/json", serde_json::json!({"choices":[{"message":{"content":content}}]}).to_string())
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn mock_llm_chain_reasoning_fallback() {
        // 复现用户场景：reasoning 模型流式 content 空 → 降级非流式 → 解析干净回复
        let s = crate::settings::LlmSettings {
            provider: "custom".into(),
            base_url: spawn_mock_llm_server().await,
            model: "Qwen/Qwen3.6-35B-A3B-reasoning".into(),
            api_key: "test".into(),
            remember_chat: false,
            backup_providers: vec![],
        };
        let provider = DoubaoProvider::from_settings(&s).unwrap();
        let req = ChatRequest {
            system: "你是一个陪伴助手".into(),
            messages: vec![ChatMessage::user("hi")],
            tools: None,
            json_object: true,
        };
        let mut on_text = |_: &str| {};
        let r1 = provider.chat_stream(req.clone(), &mut on_text).await.unwrap();
        let c1 = r1.content.unwrap_or_default();
        assert!(
            c1.trim().is_empty() || c1.trim() == "{}",
            "流式 content 应为空(复现 reasoning 模型行为), 实际={c1:?}"
        );
        let r2 = provider.chat(req.clone()).await.unwrap();
        let parsed = crate::persona::parse_reply(r2.content.as_deref().unwrap_or("")).unwrap();
        assert!(parsed.reply.contains("元气小小"), "非流式解析失败: {:?}", parsed.reply);
    }

    #[tokio::test]
    async fn mock_llm_chain_truncated_json() {
        // 截断 JSON（缺右花括号）→ parse_reply 提取干净 reply
        let s = crate::settings::LlmSettings {
            provider: "custom".into(),
            base_url: spawn_mock_llm_server().await,
            model: "Qwen/Qwen3.6-35B-A3B-truncated".into(),
            api_key: "test".into(),
            remember_chat: false,
            backup_providers: vec![],
        };
        let provider = DoubaoProvider::from_settings(&s).unwrap();
        let req = ChatRequest {
            system: String::new(),
            messages: vec![ChatMessage::user("天气")],
            tools: None,
            json_object: true,
        };
        let r = provider.chat(req).await.unwrap();
        let parsed = crate::persona::parse_reply(r.content.as_deref().unwrap_or("")).unwrap();
        assert!(parsed.reply.contains("天气"), "截断JSON提取失败: {:?}", parsed.reply);
    }

    #[tokio::test]
    async fn mock_llm_chain_stream_ok() {
        // 正常流式：content 有值 → 流式解析成功（不降级）
        let s = crate::settings::LlmSettings {
            provider: "custom".into(),
            base_url: spawn_mock_llm_server().await,
            model: "Qwen/Qwen3.6-35B-A3B".into(),
            api_key: "test".into(),
            remember_chat: false,
            backup_providers: vec![],
        };
        let provider = DoubaoProvider::from_settings(&s).unwrap();
        let req = ChatRequest {
            system: String::new(),
            messages: vec![ChatMessage::user("hi")],
            tools: None,
            json_object: true,
        };
        let mut on_text = |_: &str| {};
        let r = provider.chat_stream(req, &mut on_text).await.unwrap();
        let content = r.content.unwrap_or_default();
        assert!(content.contains("reply"), "正常流式应拿到 content: {content:?}");
    }

    // ---- v45.7：agent loop 遇 PendingApproval / 连续 Denied 立即终止 ----

    /// executor 永远返回 PendingApproval → 只跑 1 轮、返回 Ok、reply 含"确认"。
    #[tokio::test]
    async fn agent_loop_pending_approval_breaks_immediately() {
        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "write_file".into(),
                            arguments: "{}".into(),
                        },
                    }], ..std::default::Default::default() }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };
        struct PendingExec;
        #[async_trait]
        impl ToolExecutor for PendingExec {
            async fn invoke(&mut self, _n: &str, _a: &serde_json::Value) -> SkillOutcome {
                SkillOutcome::PendingApproval {
                    request_id: "r1".into(),
                    description: "write file".into(),
                }
            }
        }
        let mut exec = PendingExec;
        let (parsed, _, _exec) = run_agent_loop(
            &mock, "sys", vec![ChatMessage::user("写文件")], None, &mut exec,
            &mut |_: &str| {},
        )
        .await
        .unwrap();
        assert!(parsed.reply.contains("确认"), "reply 应含提示确认，实际：{}", parsed.reply);
        // 只发了一轮请求
        assert_eq!(mock.seen_bodies.lock().unwrap().len(), 1);
    }

    /// executor 永远返回 Denied（同一工具）→ 第 2 次后 break、返回 Ok、文案含原因。
    #[tokio::test]
    async fn agent_loop_same_denied_twice_breaks() {
        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "hack".into(),
                            arguments: "{}".into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c2".into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: "hack".into(),
                            arguments: "{}".into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"好的我换个方式","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };
        struct AlwaysDeny;
        #[async_trait]
        impl ToolExecutor for AlwaysDeny {
            async fn invoke(&mut self, _n: &str, _a: &serde_json::Value) -> SkillOutcome {
                SkillOutcome::Denied("no permission".into())
            }
        }
        let mut exec = AlwaysDeny;
        let (parsed, _, executions) = run_agent_loop(
            &mock, "sys", vec![ChatMessage::user("hack")], None, &mut exec,
            &mut |_: &str| {},
        )
        .await
        .unwrap();
        // v45.10：连续 Denied ≥2 次不再直接终止，仍回灌让模型换路径
        assert_eq!(executions.len(), 2, "两次 denied 都留痕");
        assert!(!executions[0].ok);
        assert!(executions[0].error.as_deref().unwrap().contains("no permission"));
        // 第三轮模型给出最终回复
        assert!(parsed.reply.contains("换个方式"));
    }

    /// MaxIterations Display 不含"大模型"。
    #[test]
    fn max_iterations_display_no_llm_word() {
        let msg = LlmError::MaxIterations.to_string();
        assert!(!msg.contains("大模型"), "MaxIterations 文案不得含'大模型'：{msg}");
        assert!(msg.contains("工具循环") || msg.contains("反复"));
    }

    #[test]
    fn parse_usage_extracts_total_tokens() {
        let v: serde_json::Value = serde_json::from_str(r#"{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#).unwrap();
        assert_eq!(super::parse_usage(&v), Some(15));
    }

    #[test]
    fn parse_usage_none_when_missing() {
        let v: serde_json::Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert_eq!(super::parse_usage(&v), None);
    }

    #[test]
    fn false_completion_blocks_with_action_object() {
        // 含完成语 + 动作对象 + 无工具调用 → 拦
        let fixed = super::false_completion_guard("已经帮你把项目建好了", 0).unwrap();
        assert!(fixed.contains("没有真正执行"));
        // 闲聊不拦（无动作对象）
        assert!(super::false_completion_guard("好啦我搞定啦", 0).is_none());
        // 有成功工具调用不拦
        assert!(super::false_completion_guard("已经建好项目", 1).is_none());
    }

    // ===== v45.10 假完成闸门：4 条必测 =====

    /// 必测①：工具失败的原文 reason 必须回填进下一轮上下文（tool 消息），
    /// 不能被吞成笼统失败。
    #[tokio::test]
    async fn tool_failure_propagates_to_next_turn() {
        let marker = "BOOM_越界_marker_42";
        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".into(), kind: "function".into(),
                        function: ToolCallFunction {
                            name: "code_write_file".into(),
                            arguments: r#"{"path":"/etc/forbidden/x.txt","content":"hi"}"#.into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"好的我换个路径","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };
        struct DenyExec(String);
        #[async_trait]
        impl ToolExecutor for DenyExec {
            async fn invoke(&mut self, _n: &str, _a: &serde_json::Value) -> SkillOutcome {
                SkillOutcome::Denied(self.0.clone())
            }
        }
        let mut exec = DenyExec(marker.to_string());
        let _ = run_agent_loop(
            &mock, "sys", vec![ChatMessage::user("写文件")], None, &mut exec,
            &mut |_: &str| {},
        ).await.unwrap();
        let seen = mock.seen_bodies.lock().unwrap();
        // 第二轮请求必须带上 tool 回填
        assert!(seen.len() >= 2, "应发两轮请求，实际 {}", seen.len());
        let tool_msgs: Vec<&ChatMessage> = seen[1].messages.iter().filter(|m| m.role == "tool").collect();
        assert!(!tool_msgs.is_empty(), "第二轮上下文没有 tool 消息");
        let joined: String = tool_msgs.iter().map(|m| m.content.as_deref().unwrap_or("")).collect::<Vec<_>>().join("\n");
        assert!(joined.contains(marker), "tool 消息未含原文错误 reason：{joined}");
    }

    /// 必测②：executor 返回 Ok 却没真写盘 → 产物断言把 outcome 翻成失败，
    /// executions[0].ok=false，错误含「产物校验不通过」。
    #[tokio::test]
    async fn artifact_assertion_fails_when_missing() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let fname = format!("qc_missing_{nanos}.txt");
        let path_arg = format!(r#"{{"path":"{fname}","content":"x"}}"#);
        // 清理残留（确保一开始不存在）
        let root = crate::engine::qibai_workspace_root();
        let _ = std::fs::remove_file(root.join(&fname));

        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".into(), kind: "function".into(),
                        function: ToolCallFunction {
                            name: "code_write_file".into(),
                            arguments: path_arg.into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"好的","emotion":"smiling"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };
        // 假 executor：嘴上 Ok，不写盘
        struct FakeOk;
        #[async_trait]
        impl ToolExecutor for FakeOk {
            async fn invoke(&mut self, _n: &str, _a: &serde_json::Value) -> SkillOutcome {
                SkillOutcome::Ok("pretended written".into())
            }
        }
        let mut exec = FakeOk;
        let (_parsed, _skill, executions) = run_agent_loop(
            &mock, "sys", vec![ChatMessage::user("写文件")], None, &mut exec,
            &mut |_: &str| {},
        ).await.unwrap();
        assert_eq!(executions.len(), 1, "应有一条留痕");
        assert!(!executions[0].ok, "假写盘必须被产物断言判为失败: {:?}", executions[0]);
        let err = executions[0].error.as_deref().unwrap_or("");
        assert!(err.contains("产物校验不通过"), "错误应含「产物校验不通过」：{err}");
        assert!(err.contains("声称完成但产物不存在"), "错误应定位到缺失产物：{err}");
        // 清理
        let _ = std::fs::remove_file(root.join(&fname));
    }

    /// 必测④（正向）：工具真成功 + 产物真存在 + 文本含「已完成」→ 回复不被替换，
    /// executions[0].ok=true。
    #[tokio::test]
    async fn real_success_not_blocked() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let fname = format!("qc_real_ok_{nanos}.txt");
        let root = crate::engine::qibai_workspace_root();
        let target = root.join(&fname);
        // 预创建父目录
        let _ = std::fs::create_dir_all(&root);
        let path_arg = format!(r#"{{"path":"{fname}","content":"real-data-{nanos}"}}"#);

        let mock = MockProvider {
            scripted: Mutex::new(std::collections::VecDeque::from([
                Ok(ChatResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".into(), kind: "function".into(),
                        function: ToolCallFunction {
                            name: "code_write_file".into(),
                            arguments: path_arg.into(),
                        },
                    }], ..std::default::Default::default() }),
                Ok(ChatResponse {
                    content: Some(r#"{"reply":"已完成，文件写好了","emotion":"happy"}"#.into()),
                    tool_calls: vec![], tokens_used: None,
                }),
            ])),
            seen_bodies: Mutex::new(vec![]),
        };
        // 真 executor：真写盘
        struct RealWrite(std::path::PathBuf);
        #[async_trait]
        impl ToolExecutor for RealWrite {
            async fn invoke(&mut self, _n: &str, _a: &serde_json::Value) -> SkillOutcome {
                std::fs::write(&self.0, "real-data").unwrap();
                SkillOutcome::Ok("written".into())
            }
        }
        let mut exec = RealWrite(target.clone());
        // 前置确保不存在
        let _ = std::fs::remove_file(&target);
        let (parsed, _skill, executions) = run_agent_loop(
            &mock, "sys", vec![ChatMessage::user("写文件")], None, &mut exec,
            &mut |_: &str| {},
        ).await.unwrap();
        // 产物真存在且非空
        assert!(target.exists() && std::fs::metadata(&target).map(|m| m.len() > 0).unwrap_or(false), "真写盘产物应存在");
        assert_eq!(executions.len(), 1);
        assert!(executions[0].ok, "真成功不应被拦: {:?}", executions[0]);
        // 回复不被替换（tools_ok=1，假完成闸门放行）
        assert!(parsed.reply.contains("已完成"), "回复不应被改写：{}", parsed.reply);
        let _ = std::fs::remove_file(&target);
    }
}
