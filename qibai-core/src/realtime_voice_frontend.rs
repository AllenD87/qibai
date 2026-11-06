//! 本地语音服务 OpenAI Realtime 兼容接口形态（轻量契约层）。
//!
//! 借鉴 Qwen-Audio-Agent：Gateway 只连 Realtime 接口
//! （`ws://127.0.0.1:8765/v1/realtime`），不关心 STT/LLM/TTS 内部配置。
//! 本模块把这一**接口范式**抽成引擎无关的契约：
//! - 不引入 Python（不打包 huggingface speech-to-speech 运行时）；
//! - 不改 whisper_engine.rs（STT 组领域）；
//! - 后续 sherpa-onnx 落地时，在其之上实现 `VoiceFrontend` trait 即可，
//!   STT_ENGINE 切换不受影响。
//!
//! B 线（Pipecat 双工语音层对接）：本模块已落地 Pipecat ws 客户端——
//! OpenAI Realtime 事件映射（session 初始化 / 音频帧上行 / 转写与响应事件下行 /
//! barge-in 中断事件处理），并以环境变量 `VOICE_ENGINE=pipecat|legacy` 切换：
//! - `legacy`（默认）：进程内 P0-B 回声门 + whisper/sherpa STT → LLM → tts 现链路；
//! - `pipecat`：整体切到 Pipecat 独立 Python 语音服务（barge-in/话轮/回声由服务端处理）。
//! 沙箱内单测以 loopback mock ws 服务验证事件映射，不依赖真实服务。

use serde::{Deserialize, Serialize};

// base64 Engine trait（encode/decode 方法在 trait 上）。
use base64::Engine as _;

/// 默认本地 Realtime 兼容端点（与 qwen-audio-agent speech-to-speech 前端一致）。
pub const DEFAULT_REALTIME_ENDPOINT: &str = "ws://127.0.0.1:8765/v1/realtime";

/// 语音前端提供方选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProviderKind {
    /// 进程内现有链路（whisper.cpp 子进程 → LLM → tts），当前默认。
    InProcess,
    /// 外挂 Realtime 兼容本地服务（sherpa-onnx 封装 / HF speech-to-speech）。
    LocalRealtime,
}

/// 语音前端配置（持久化进 Settings 时新增字段，本轮不改 settings.rs）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceFrontendConfig {
    pub provider: VoiceProviderKind,
    /// LocalRealtime 时的端点；缺省 DEFAULT_REALTIME_ENDPOINT。
    pub endpoint: Option<String>,
}

impl Default for VoiceFrontendConfig {
    fn default() -> Self {
        Self {
            provider: VoiceProviderKind::InProcess,
            endpoint: Some(DEFAULT_REALTIME_ENDPOINT.to_string()),
        }
    }
}

/// Realtime 协议消息子集（只元气小小需要的收敛面）。
/// 与 OpenAI Realtime 事件命名对齐，便于未来直连兼容服务。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RealtimeEvent {
    /// 客户端：提交音频块（base64 PCM 16k）。
    #[serde(rename = "input_audio_buffer.append")]
    InputAudioAppend { audio_b64: String },
    /// 客户端：提交结束（触发 VAD 出句）。
    #[serde(rename = "input_audio_buffer.commit")]
    InputAudioCommit,
    /// 服务端：转写完成。
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    TranscriptionCompleted { transcript: String },
    /// 服务端：用户打断（VAD 检测到人声起）。
    #[serde(rename = "input_audio_buffer.speech_started")]
    SpeechStarted,
    /// 服务端：人声结束。
    #[serde(rename = "input_audio_buffer.speech_stopped")]
    SpeechStopped,
    /// 服务端：TTS 文本回复。
    #[serde(rename = "response.audio_transcript.delta")]
    ResponseTextDelta { delta: String },
}

/// 引擎无关语音前端 trait：未来 whisper / sherpa-onnx 各实现一份。
///
/// 与 STT 组的关系：本 trait 不依赖 whisper_engine.rs；
/// sherpa-onnx 落地后由适配层在它之上实现，listener.rs 改走 WS 客户端。
pub trait VoiceFrontend: Send + Sync {
    fn kind(&self) -> VoiceProviderKind;
    /// 送一帧麦克风 PCM（16k i16 mono）。
    fn push_audio(&mut self, pcm_16k_mono: &[i16]);
    /// 拉取已就绪的转写结果（非阻塞）。
    fn drain_transcripts(&mut self) -> Vec<String>;
    /// 用户是否正在说话（VAD 上报，供投递窗口判定）。
    fn user_speaking(&self) -> bool;
}

// ============================================================================
// B 线：VOICE_ENGINE 回退开关
// ============================================================================

/// 语音引擎选择环境变量名。
/// - `VOICE_ENGINE=pipecat`  切到 Pipecat 独立语音服务（ws://127.0.0.1:8765/v1/realtime）
/// - `VOICE_ENGINE=legacy`   进程内现链路（P0-B 回声门 + whisper/sherpa → LLM → tts）
/// - 未设置 / 未知值           ：默认 `legacy`（宽松回退，与 STT_ENGINE 风格一致）
pub const ENV_VOICE_ENGINE: &str = "VOICE_ENGINE";

/// 语音引擎链路选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceEngineKind {
    /// Pipecat 独立语音服务：barge-in / 话轮 / 回声均由服务端处理。
    Pipecat,
    /// 进程内 legacy：P0-B 回声门（v1 冻结）+ whisper/sherpa STT → LLM → tts。
    Legacy,
}

impl Default for VoiceEngineKind {
    /// 默认 legacy：Pipecat 是进程外服务（8765），未就绪时回退开关必须
    /// 不破坏现网语音链路；切 pipecat 是显式 opt-in。
    fn default() -> Self {
        Self::Legacy
    }
}

/// 纯函数解析 VOICE_ENGINE 原始值（不读 env，单测友好）。
/// 空串 / 未知值一律回退 `Legacy`——宽松解析，与 STT_ENGINE 开关风格一致。
pub fn parse_voice_engine(raw: Option<&str>) -> VoiceEngineKind {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some("pipecat") => VoiceEngineKind::Pipecat,
        _ => VoiceEngineKind::Legacy,
    }
}

/// 读取进程环境变量 `VOICE_ENGINE`，得到当前链路选择。
pub fn resolve_voice_engine() -> VoiceEngineKind {
    parse_voice_engine(std::env::var(ENV_VOICE_ENGINE).ok().as_deref())
}

// ============================================================================
// B 线：Realtime 协议编解码（纯函数，与传输解耦，单测直测）
// ============================================================================

/// 麦克风上行采样率（OpenAI Realtime pcm16 约定，与 legacy 链路一致）。
pub const INPUT_SAMPLE_RATE: u32 = 16_000;

/// 默认上行帧长：20ms @ 16k i16 mono = 320 采样 = 640 字节。
pub const FRAME_MS: u32 = 20;

/// 一帧 20ms 对应的 i16 采样数（16k × 20ms）。
pub fn frame_samples() -> usize {
    (INPUT_SAMPLE_RATE as usize) * (FRAME_MS as usize) / 1000
}

/// i16 单声道 PCM → little-endian 字节 → base64（Realtime 上行编码约定）。
pub fn pcm_to_base64(pcm_16k_mono: &[i16]) -> String {
    let bytes: Vec<u8> = pcm_16k_mono
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}

/// 客户端→服务端：会话初始化（`session.update`）。
/// 固定契约：pcm16 上行 / server_vad 话轮检测 / 转写开。
pub fn encode_session_update() -> String {
    serde_json::json!({
        "type": "session.update",
        "session": {
            "type": "realtime",
            "input_audio_format": "pcm16",
            "output_audio_format": "pcm16",
            "turn_detection": {
                "type": "server_vad",
                "threshold": 0.5,
                "silence_duration_ms": 600,
            },
            "input_audio_transcription": { "model": "whisper-1" }
        }
    })
    .to_string()
}

/// 客户端→服务端：追加一帧麦克风音频（base64 pcm16）。
pub fn encode_input_audio_append(pcm_16k_mono: &[i16]) -> String {
    serde_json::json!({
        "type": "input_audio_buffer.append",
        "audio": pcm_to_base64(pcm_16k_mono),
    })
    .to_string()
}

/// 客户端→服务端：清空输入缓冲（barge-in 时按 Realtime 约定主动下发，
/// 服务端据此丢弃旧缓冲、立即收话）。
pub fn encode_input_audio_clear() -> String {
    serde_json::json!({ "type": "input_audio_buffer.clear" }).to_string()
}

/// 客户端→服务端：显式开启一轮 response（OpenAI Realtime `response.create`）。
/// Pipecat 服务在 VAD 未触发（静音段）时，由客户端显式请求一轮对话。
pub fn encode_response_create() -> String {
    serde_json::json!({ "type": "response.create" }).to_string()
}

/// 服务端→客户端：解析后的下行事件（干净 Rust 形态，listener 直接消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// `session.created` / `session.updated`：握手完成、会话就绪。
    SessionReady,
    /// `input_audio_buffer.speech_started`：VAD 检测到用户人声起 = barge-in，
    /// listener 应停当前 TTS 播放并下发 `input_audio_buffer.clear`。
    BargeIn,
    /// `input_audio_buffer.speech_stopped`：用户人声结束。
    SpeechStopped,
    /// `conversation.item.input_audio_transcription.completed`：最终转写文本。
    Transcript(String),
    /// `response.audio_transcript.delta`：TTS 文本流式增量（字幕/上下文用）。
    ResponseTextDelta(String),
    /// `response.created`：本轮 response 开始（LLM/TTS 在途）。
    ResponseCreated,
    /// `response.audio.delta`：TTS 音频增量（base64 pcm16，Pipecat 播放预留）。
    /// 注意：A 线服务当前把 base64 放在 `delta` 字段（OpenAI spec 为 `audio`），
    /// 解析时 `audio` 优先、`delta` 兜底（差异见交付说明）。
    ResponseAudioDelta(String),
    /// `response.done`：本轮响应结束。
    ResponseDone,
    /// `error`：服务端错误。
    Error(String),
    /// 未识别事件类型：前向兼容（服务端新增事件时不炸客户端）。
    Unknown(String),
}

/// 解析一条服务端下行 JSON 文本（OpenAI Realtime 事件映射）。
/// 非 JSON / 缺字段均不 panic，降级为 `Error` / 空串变体。
pub fn parse_server_event(text: &str) -> ServerEvent {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return ServerEvent::Error(format!("下行非 JSON: {}", truncate(text, 80))),
    };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "session.created" | "session.updated" => ServerEvent::SessionReady,
        "input_audio_buffer.speech_started" => ServerEvent::BargeIn,
        "input_audio_buffer.speech_stopped" => ServerEvent::SpeechStopped,
        "conversation.item.input_audio_transcription.completed" => {
            let t = v
                .get("transcript")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            ServerEvent::Transcript(t)
        }
        "response.audio_transcript.delta" => {
            let d = v
                .get("delta")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            ServerEvent::ResponseTextDelta(d)
        }
        "response.created" => ServerEvent::ResponseCreated,
        "response.audio.delta" => {
            // OpenAI spec 字段为 `audio`；A 线实现当前放在 `delta`——兼容两者。
            let a = v
                .get("audio")
                .and_then(|t| t.as_str())
                .or_else(|| v.get("delta").and_then(|t| t.as_str()))
                .unwrap_or("")
                .to_string();
            ServerEvent::ResponseAudioDelta(a)
        }
        "response.done" => ServerEvent::ResponseDone,
        "error" => {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            ServerEvent::Error(msg)
        }
        other => ServerEvent::Unknown(other.to_string()),
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ============================================================================
// B 线：Pipecat ws 客户端（tokio-tungstenite 传输，测试可用 loopback mock 服务）
// ============================================================================

/// Pipecat ws 客户端错误。
#[derive(Debug, thiserror::Error)]
pub enum PipecatError {
    #[error("ws 握手失败: {0}")]
    Handshake(String),
    #[error("ws 发送通道关闭: {0}")]
    Send(String),
}

/// Pipecat Realtime 客户端：握手后后台读写协程，主侧只做
/// `push_audio` 上行 + `drain_events` 非阻塞拉下行事件。
///
/// 生命周期：`connect`（握手 + session.update）→ `push_audio` 循环 →
/// `drain_events` 拿到 `Transcript` / `BargeIn` / `ResponseTextDelta` → `close`。
pub struct PipecatClient {
    writer: tokio::sync::mpsc::UnboundedSender<String>,
    events: tokio::sync::mpsc::UnboundedReceiver<ServerEvent>,
}

impl PipecatClient {
    /// 连接 Pipecat Realtime 端点（默认 `ws://127.0.0.1:8765/v1/realtime`），
    /// 完成握手后立即下发 `session.update` 初始化会话。
    pub async fn connect(endpoint: &str) -> Result<Self, PipecatError> {
        use futures_util::{SinkExt, StreamExt};
        let (stream, _resp) = tokio_tungstenite::connect_async(endpoint)
            .await
            .map_err(|e| PipecatError::Handshake(e.to_string()))?;
        let (write_half, read_half) = stream.split();

        // 写入协程：把主侧序列化好的 JSON 帧灌进 ws。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut sink = write_half;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink
                    .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // 读取协程：ws 下行帧 → 解析 → 事件队列。
        let (etx, erx) = tokio::sync::mpsc::unbounded_channel::<ServerEvent>();
        tokio::spawn(async move {
            let mut source = read_half;
            while let Some(item) = source.next().await {
                match item {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => {
                        let _ = etx.send(parse_server_event(&t));
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
                    // Ping / Pong / Binary：传输层噪声，忽略继续读。
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            // 读循环退出 = 连接断了，让主侧感知。
            let _ = etx.send(ServerEvent::Error("ws 连接断开".to_string()));
        });

        // 会话初始化（在接收事件前先发，服务端才开始收音频）。
        tx.send(encode_session_update())
            .map_err(|e| PipecatError::Send(e.to_string()))?;

        Ok(Self { writer: tx, events: erx })
    }

    /// 上行一帧麦克风 PCM（16k i16 mono）。非阻塞：序列化后投递写入协程。
    pub fn push_audio(&self, pcm_16k_mono: &[i16]) {
        let _ = self.writer.send(encode_input_audio_append(pcm_16k_mono));
    }

    /// barge-in 中断处理：按 Realtime 约定下发 `input_audio_buffer.clear`，
    /// 让服务端丢弃旧缓冲、立即进入收话状态。
    pub fn notify_barge_in(&self) {
        let _ = self.writer.send(encode_input_audio_clear());
    }

    /// 显式开启一轮 response（`response.create`）。
    /// VAD 静音段服务端不会自动起话轮，由客户端请求一轮（如主动打招呼）。
    pub fn request_turn(&self) {
        let _ = self.writer.send(encode_response_create());
    }

    /// 非阻塞拉取已就绪的下行事件（转写 / barge-in / 响应文本等）。
    pub fn drain_events(&mut self) -> Vec<ServerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }

    /// 关闭发送通道（写入协程随最后一个 sender drop 自然退出；读协程随对端关闭退出）。
    pub fn close(&mut self) {
        // 用一个空 sender 替换，旧 sender 被 drop → 写入协程 rx.recv() 返回 None 后收尾。
        self.writer = tokio::sync::mpsc::unbounded_channel().0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_inprocess() {
        let c = VoiceFrontendConfig::default();
        assert_eq!(c.provider, VoiceProviderKind::InProcess);
        assert_eq!(c.endpoint.as_deref(), Some(DEFAULT_REALTIME_ENDPOINT));
    }

    #[test]
    fn event_serialization_openai_compatible_tags() {
        let ev = RealtimeEvent::TranscriptionCompleted {
            transcript: "今天天气".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains("conversation.item.input_audio_transcription.completed"));
        assert!(s.contains("今天天气"));
    }

    #[test]
    fn event_roundtrip_speech_stopped() {
        let ev = RealtimeEvent::SpeechStopped;
        let s = serde_json::to_string(&ev).unwrap();
        let back: RealtimeEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, RealtimeEvent::SpeechStopped));
    }

    #[test]
    fn provider_serde() {
        let c = VoiceFrontendConfig {
            provider: VoiceProviderKind::LocalRealtime,
            endpoint: None,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: VoiceFrontendConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.provider, VoiceProviderKind::LocalRealtime);
    }

    // ---- VOICE_ENGINE 回退开关 ----

    #[test]
    fn parse_voice_engine_default_legacy() {
        assert_eq!(parse_voice_engine(None), VoiceEngineKind::Legacy);
        assert_eq!(parse_voice_engine(Some("")), VoiceEngineKind::Legacy);
        assert_eq!(parse_voice_engine(Some("   ")), VoiceEngineKind::Legacy);
        assert_eq!(VoiceEngineKind::default(), VoiceEngineKind::Legacy);
    }

    #[test]
    fn parse_voice_engine_pipecat_explicit() {
        assert_eq!(parse_voice_engine(Some("pipecat")), VoiceEngineKind::Pipecat);
        // 容忍首尾空白
        assert_eq!(parse_voice_engine(Some("  pipecat ")), VoiceEngineKind::Pipecat);
    }

    #[test]
    fn parse_voice_engine_unknown_falls_back_legacy() {
        // 宽松回退：legacy / 任何拼写错误都落 legacy，不破坏现链路
        assert_eq!(parse_voice_engine(Some("legacy")), VoiceEngineKind::Legacy);
        assert_eq!(parse_voice_engine(Some("piper")), VoiceEngineKind::Legacy);
        assert_eq!(parse_voice_engine(Some("auto")), VoiceEngineKind::Legacy);
    }

    // ---- 协议编解码纯函数 ----

    #[test]
    fn session_update_encodes_realtime_contract() {
        let s = encode_session_update();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], "session.update");
        assert_eq!(v["session"]["input_audio_format"], "pcm16");
        assert_eq!(v["session"]["turn_detection"]["type"], "server_vad");
    }

    #[test]
    fn audio_append_roundtrip_base64() {
        // 320 采样 = 20ms@16k → 640 字节 → base64 可还原
        let pcm = vec![0x0102i16, -1, 300, -32768, 12345];
        let s = encode_input_audio_append(&pcm);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], "input_audio_buffer.append");
        let b64 = v["audio"].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(bytes.len(), pcm.len() * 2);
        let back: Vec<i16> = bytes
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(back, pcm);
    }

    #[test]
    fn frame_format_20ms_16k() {
        assert_eq!(INPUT_SAMPLE_RATE, 16_000);
        assert_eq!(frame_samples(), 320);
        assert_eq!(frame_samples() * 2, 640); // 640 字节/帧
    }

    #[test]
    fn input_audio_clear_encodes_barge_in() {
        let s = encode_input_audio_clear();
        assert!(s.contains("input_audio_buffer.clear"));
    }

    #[test]
    fn response_create_encodes_explicit_turn() {
        let v: serde_json::Value = serde_json::from_str(&encode_response_create()).unwrap();
        assert_eq!(v["type"], "response.create");
    }

    #[test]
    fn response_audio_delta_accepts_both_field_forms() {
        // OpenAI spec：base64 在 `audio` 字段
        assert_eq!(
            parse_server_event(r#"{"type":"response.audio.delta","audio":"QUJD"}"#),
            ServerEvent::ResponseAudioDelta("QUJD".into())
        );
        // A 线服务实现：base64 在 `delta` 字段（兼容兜底）
        assert_eq!(
            parse_server_event(r#"{"type":"response.audio.delta","delta":"REVG"}"#),
            ServerEvent::ResponseAudioDelta("REVG".into())
        );
        // response.created → 本轮开始
        assert_eq!(
            parse_server_event(r#"{"type":"response.created"}"#),
            ServerEvent::ResponseCreated
        );
    }

    #[test]
    fn parse_server_event_all_known_types() {
        assert_eq!(
            parse_server_event(r#"{"type":"session.created","session":{}}"#),
            ServerEvent::SessionReady
        );
        assert_eq!(
            parse_server_event(r#"{"type":"input_audio_buffer.speech_started"}"#),
            ServerEvent::BargeIn
        );
        assert_eq!(
            parse_server_event(r#"{"type":"input_audio_buffer.speech_stopped"}"#),
            ServerEvent::SpeechStopped
        );
        assert_eq!(
            parse_server_event(
                r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"你好呀"}"#
            ),
            ServerEvent::Transcript("你好呀".into())
        );
        assert_eq!(
            parse_server_event(r#"{"type":"response.audio_transcript.delta","delta":"哈"}"#),
            ServerEvent::ResponseTextDelta("哈".into())
        );
        assert_eq!(
            parse_server_event(r#"{"type":"response.audio.delta","audio":"AAAA"}"#),
            ServerEvent::ResponseAudioDelta("AAAA".into())
        );
        assert_eq!(
            parse_server_event(r#"{"type":"response.done"}"#),
            ServerEvent::ResponseDone
        );
        match parse_server_event(r#"{"type":"error","error":{"message":"boom"}}"#) {
            ServerEvent::Error(m) => assert!(m.contains("boom")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn parse_server_event_forward_compat() {
        // 服务端新增未知事件 → 不炸，降级 Unknown
        assert_eq!(
            parse_server_event(r#"{"type":"response.function_call_arguments.delta","x":1}"#),
            ServerEvent::Unknown("response.function_call_arguments.delta".into())
        );
        // 非 JSON → Error，不 panic
        match parse_server_event("not-json{") {
            ServerEvent::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
        // 缺字段 → 空串变体
        assert_eq!(
            parse_server_event(r#"{"type":"conversation.item.input_audio_transcription.completed"}"#),
            ServerEvent::Transcript(String::new())
        );
    }

    // ---- loopback mock ws 服务：客户端一次往返冒烟 ----

    #[tokio::test]
    async fn pipecat_client_loopback_roundtrip() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::TcpListener;

        // 起 loopback mock 服务（0 端口随机）：模拟 A 线 Pipecat 服务。
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();

            // ① 期望握手后第一条 = session.update
            let first = ws.next().await.unwrap().unwrap();
            let first_text = match first {
                tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
                other => panic!("expect Text session.update, got {other:?}"),
            };
            assert!(first_text.contains("session.update"), "首帧应为 session.update: {first_text}");

            // ② 期望第二条 = input_audio_buffer.append
            let second = ws.next().await.unwrap().unwrap();
            let second_text = match second {
                tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
                other => panic!("expect Text append, got {other:?}"),
            };
            assert!(second_text.contains("input_audio_buffer.append"));
            assert!(second_text.contains("audio"), "上行帧应带 audio base64 字段");

            // ③ 期望第三条 = response.create（客户端显式请求一轮对话）
            let third = ws.next().await.unwrap().unwrap();
            let third_text = match third {
                tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
                other => panic!("expect Text response.create, got {other:?}"),
            };
            assert!(third_text.contains("response.create"), "第三帧应为 response.create: {third_text}");

            // ④ 下行：与 A 线真实时序一致——session.updated → response.created →
            //    转写 → 文本增量 → audio delta（A 线字段在 delta）→ done
            for payload in [
                r#"{"type":"session.updated","session":{"model":"qibai-local","voice":"xiaoxiao"}}"#,
                r#"{"type":"response.created"}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"今天天气怎么样"}"#,
                r#"{"type":"response.audio_transcript.delta","delta":"晴天哦"}"#,
                r#"{"type":"response.audio.delta","delta":"REVGQw=="}"#,
                r#"{"type":"response.done","response":{"status":"completed"}}"#,
            ] {
                ws.send(tokio_tungstenite::tungstenite::Message::Text(
                    payload.to_string().into(),
                ))
                .await
                .unwrap();
            }
            // 挂起等客户端收尾
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });

        // 客户端连接 loopback（模拟连 A 线 8765）
        let mut client = PipecatClient::connect(&format!("ws://{addr}/v1/realtime"))
            .await
            .unwrap();
        // 上行一帧 20ms PCM + 显式请求一轮对话
        client.push_audio(&vec![0i16; 320]);
        client.request_turn();

        // 拉取下行事件（等待最多 ~2s）
        let mut evs = Vec::new();
        for _ in 0..200 {
            evs.extend(client.drain_events());
            if evs.iter().any(|e| matches!(e, ServerEvent::ResponseDone)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        client.close();
        let _ = server.await;

        // 事件序列断言：就绪 → 轮次开始 → 转写 → 文本增量 → 音频增量 → done
        assert!(evs.contains(&ServerEvent::SessionReady), "应收到 session ready: {evs:?}");
        assert!(evs.contains(&ServerEvent::ResponseCreated), "应收到 response.created: {evs:?}");
        assert!(
            evs.contains(&ServerEvent::Transcript("今天天气怎么样".into())),
            "应收到转写: {evs:?}"
        );
        assert!(
            evs.contains(&ServerEvent::ResponseTextDelta("晴天哦".into())),
            "应收到文本增量: {evs:?}"
        );
        assert!(
            evs.contains(&ServerEvent::ResponseAudioDelta("REVGQw==".into())),
            "应收到音频增量: {evs:?}"
        );
        assert!(evs.contains(&ServerEvent::ResponseDone), "应收到 done: {evs:?}");
    }

    #[tokio::test]
    async fn pipecat_client_handshake_failure_propagates() {
        // 没有服务监听的端口 → 握手错误上抛，不 panic
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // 立即释放，确保无人监听
        let res = PipecatClient::connect(&format!("ws://127.0.0.1:{port}/v1/realtime")).await;
        assert!(matches!(res, Err(PipecatError::Handshake(_))));
    }
}
