# 元气小小 · Pipecat Realtime 语音服务（A 线）

**契约**：`ws://127.0.0.1:8765/v1/realtime`（OpenAI Realtime 兼容事件子集）
**定位**：独立 Python 语音服务进程，不碰 Rust 主进程；B 线按 OpenAI Realtime spec 写 Rust 客户端对接。

## 目录
```
src-tauri/pipecat/
  pipecat_voice/
    protocol.py   Realtime 事件子集（客户端上行/服务端下行）
    asr.py        sherpa-onnx paraformer(int8)+silero VAD 流式喂入
    llm.py        OpenAI 兼容 chat（可配置+超时兜底）
    tts.py        NullTTS（本地正弦波 PCM；真机切 edge-tts）
    session.py    会话状态机：barge-in 取消在途、一轮 response 编排
    server.py     ws 入口
  tests/          pytest（协议/状态机/ws 冒烟）
  requirements.txt
scripts/pipecat_voice.sh          启动脚本
bash fetch_resources.sh             资源补齐（sherpa 模型 ≈514MB，可选后端；不进增量包）
```

## 部署
```bash
pip install -r src-tauri/pipecat/requirements.txt
bash fetch_resources.sh                     # 首次补齐资源；已有则跳过（sherpa 可选）
bash scripts/pipecat_voice.sh              # ws://127.0.0.1:8765/v1/realtime
```
环境变量：`QIBAI_REALTIME_PORT`(8765) / `QIBAI_LLM_BASE_URL`(你的 OpenAI 兼容端点，如 http://127.0.0.1:8080/v1) / `QIBAI_LLM_MODEL`(Qwen3.6-35B-A3B)。

## 依赖与模型体积
| 项 | 体积 |
|---|---|
| sherpa-onnx wheel（含 onnxruntime） | ~150MB |
| paraformer.onnx (int8) | 233MB |
| silero_vad.onnx | 0.6MB |
| tokens.txt | 76KB |
| Python 运行时(3.11+) | 系统自带 |
| （可选）edge-tts 真音色 | pip 包，零模型下载 |

## 真机验证路径
1. 打断：播放中说话 → 服务下行 `input_audio_buffer.speech_started`，在途 LLM/TTS 取消；
2. 回声：Rust 侧 P0-B echo gate 仍在 mic 前级，服务只收干净语音；
3. 连续 10 轮对话无「自说自话」（小小听到自己→接话）。

## 联调 spec 对齐说明（2026-10-26，B 线联调后确认）
- **`response.audio.delta` 字段**：服务发送的 base64 音频增量字段为 `delta`（与 OpenAI Realtime 官方 spec 一致），未发送 `audio` 字段——B 线双字段兼容属冗余保护，无需改动。
- **话轮触发形态（设计取舍）**：当前为**客户端驱动话轮**——服务端 VAD 检测到静音出句后不自动起 response，需客户端显式发 `response.create`（B 线 `request_turn()`）。符合 OpenAI Realtime 客户端驱动语义。真机后续增强：服务端 VAD 出句后自动触发 response（服务端话轮）。

## 冒烟证据（沙盒 2026-10-26）
- 握手：`== handshake ok ==`
- 下行序列：`session.updated` → `response.created` → `response.audio_transcript.delta`（LLM 超时兜底台词）→ `response.audio.delta`（base64 PCM）→ `response.done`
- pytest：9/9 通过
