//! Pipecat 联调冒烟（手工执行，不进自动单测）。
//!
//! 用法：cargo run -p qibai-core --example pipecat_live_smoke
//! 前置：bash scripts/pipecat_voice.sh（ws://127.0.0.1:8765/v1/realtime）
//! 证据：本文件 stdout 即为一次往返日志——握手 / 音频上行 / 转写与响应事件下行。

use qibai_core::realtime_voice_frontend::{
    encode_input_audio_append, parse_server_event, PipecatClient, ServerEvent,
    DEFAULT_REALTIME_ENDPOINT,
};

#[tokio::main]
async fn main() {
    let endpoint = std::env::var("QIBAI_REALTIME_URL").unwrap_or_else(|_| {
        DEFAULT_REALTIME_ENDPOINT.to_string()
    });
    println!("[smoke] connect {endpoint}");

    let mut client = match PipecatClient::connect(&endpoint).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[smoke] 握手失败: {e}");
            std::process::exit(1);
        }
    };
    println!("[smoke] 握手 ok（session.update 已下发）");

    // 上行：8 帧 20ms 静音 PCM（16k i16 mono），验证 input_audio_buffer.append 通道。
    for i in 0..8 {
        client.push_audio(&vec![0i16; 320]);
        println!("[smoke] 上行 audio frame #{i} (320 samples=20ms)");
    }
    // 裸编码再发一帧，日志打印帧形态（base64 长度）
    let raw = encode_input_audio_append(&[1i16, 2, 3, 4]);
    println!("[smoke] append frame json 长度={} bytes", raw.len());

    // VAD 静音段不触发话轮 → 显式 response.create 请求一轮（OpenAI Realtime 约定）。
    client.request_turn();
    println!("[smoke] 上行 response.create（请求一轮对话）");

    // 下行：最多收 12 秒，观察服务端事件序列。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    let mut seen: Vec<ServerEvent> = Vec::new();
    while std::time::Instant::now() < deadline {
        for ev in client.drain_events() {
            match &ev {
                ServerEvent::SessionReady => println!("[smoke] ← session ready"),
                ServerEvent::BargeIn => println!("[smoke] ← barge-in (speech_started)"),
                ServerEvent::SpeechStopped => println!("[smoke] ← speech_stopped"),
                ServerEvent::Transcript(t) => println!("[smoke] ← transcript: {t}"),
                ServerEvent::ResponseTextDelta(d) => println!("[smoke] ← text delta: {d}"),
                ServerEvent::ResponseCreated => println!("[smoke] ← response.created"),
                ServerEvent::ResponseAudioDelta(b64) => {
                    println!("[smoke] ← audio delta: {} b64 chars", b64.len())
                }
                ServerEvent::ResponseDone => println!("[smoke] ← response.done"),
                ServerEvent::Error(m) => println!("[smoke] ← error: {m}"),
                ServerEvent::Unknown(t) => println!("[smoke] ← unknown event type: {t}"),
            }
            seen.push(ev);
        }
        if seen.iter().any(|e| matches!(e, ServerEvent::ResponseDone)) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    client.close();

    // 终判：必须看到握手就绪 + 至少一条响应下行。
    let has_ready = seen.iter().any(|e| matches!(e, ServerEvent::SessionReady));
    let has_response = seen.iter().any(|e| {
        matches!(
            e,
            ServerEvent::ResponseTextDelta(_) | ServerEvent::ResponseAudioDelta(_)
        )
    });
    println!("[smoke] 汇总: ready={has_ready} response_down={has_response} 共 {} 事件", seen.len());

    // 顺便验证解析器对一条典型下行 JSON 的映射（字段名差异排查用）。
    let probe = parse_server_event(
        r#"{"type":"response.audio_transcript.delta","delta":"你好，我是元气小小"}"#,
    );
    println!("[smoke] probe 解析: {probe:?}");

    if has_ready && has_response {
        println!("[smoke] PASS：握手 → 上行 → 下行一次往返完成");
    } else {
        println!("[smoke] FAIL：未拿到完整往返");
        std::process::exit(2);
    }
}
