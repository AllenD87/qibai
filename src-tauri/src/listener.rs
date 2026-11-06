//! 免提聆听模式（语音唤醒 + 免提对话）——元气小小 v14
//!
//! 状态机：OFF（关闭）→ WAKE（麦克风常开监听唤醒词「元气小小」）→ ACTIVE（免提对话）
//! ACTIVE 下说「结束 / 拜拜 / 再见 / 退出 / 先这样 / 不聊了」退回 WAKE。
//!
//! 隐私：音频仅在本地 whisper.cpp 识别（不联网）；对话回复走后端 Engine（本地意图直达 / LLM）。
//!
//! 架构：
//! - cpal 常开采集（默认输入设备，单声道，20ms 帧能量 VAD 断句）
//! - 句末音频 → tokio 状态机 task → 重采样 16k → wav → whisper 转写 → 状态机处理
//! - 状态/事件广播：`qibai://listen-state`（off/wake/active）、`qibai://pet-say`（回复演出）

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use qibai_core::engine::Engine;
use qibai_core::realtime_voice_frontend::{PipecatClient, ServerEvent, VoiceEngineKind};
use qibai_core::skills::SkillOutcome;
use qibai_core::tts::TTS_SPEAKING;
use qibai_core::voice_ui_cmd::{match_ui_command, UiCmd, UiCmdMatch};
use qibai_core::whisper_engine;

/// B 线双工句柄：`VOICE_ENGINE=pipecat` 且 ws 连接成功后填充；
/// `None` = legacy 链路（P0-B 回声门 + whisper/sherpa），feed_audio 走原逻辑。
static PIPECAT: std::sync::OnceLock<Arc<tokio::sync::Mutex<PipecatClient>>> =
    std::sync::OnceLock::new();

pub const LISTEN_OFF: u8 = 0;
pub const LISTEN_WAKE: u8 = 1;
pub const LISTEN_ACTIVE: u8 = 2;

/// v45.8：免提采集流错误标志（设备拔下/流失败时置位，主循环据此暂停并清 pending）。
static LISTENER_DEVICE_LOST: AtomicBool = AtomicBool::new(false);

/// v20 barge-in：朗读期间采集回调检测到用户语音，置位此 flag；
/// 状态机 task 轮询到后调 stop_playback() 并广播 tts-interrupted。
pub static BARGE_IN_REQUEST: AtomicBool = AtomicBool::new(false);

/// 最近一次 barge 打断发生的时刻（误打断诊断用：打断后短时间内收到回声转写 = 疑似误打断）。
pub static LAST_BARGE_AT: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// 跨命令共享的监听状态。
///
/// 注意：cpal::Stream 内部含 `PhantomData<*mut ()>`（非 Send），不能放入 AppState；
/// 采集流由 start_listener 创建的独立线程持有，这里只存 Send 的原子与停止信号通道。
/// v18.8-fix8 语音诊断实时数据（前端 voice_diag 每 1s 轮询展示）。
#[derive(Default)]
pub struct VoiceDiag {
    /// 最近一帧 RMS 能量（静音≈0.001-0.005，说话≈0.01+）
    pub rms: Arc<StdMutex<f32>>,
    /// VAD 断句累计次数
    pub vad_count: Arc<AtomicU32>,
    /// 最近一次转写文本（空=还没转出来）
    pub last_text: Arc<StdMutex<String>>,
    /// 当前采集设备名
    pub device: Arc<StdMutex<String>>,
    /// 采集采样率
    pub rate: Arc<StdMutex<u32>>,
}

pub struct ListenerShared {
    /// 0=off 1=待唤醒 2=聆听对话中（AtomicU8，供采集回调零锁读取）
    pub state: Arc<AtomicU8>,
    /// 监听总开关（镜像 settings.voice.wake_enabled）
    pub enabled: Arc<AtomicBool>,
    /// 停止信号（Send）：stop_listener 发送后，采集线程 drop 流并退出
    pub stop_tx: StdMutex<Option<std::sync::mpsc::Sender<()>>>,
    /// 语音诊断实时数据
    pub diag: VoiceDiag,
}

impl ListenerShared {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(LISTEN_OFF)),
            enabled: Arc::new(AtomicBool::new(false)),
            stop_tx: StdMutex::new(None),
            diag: VoiceDiag::default(),
        }
    }
}

/// 启动麦克风常开监听。无麦克风设备/权限时返回 Err（调用方打印提示，不崩溃）。
///
/// 采集流（cpal::Stream 非 Send）由本函数创建的独立线程持有；
/// 线程在收到 stop 信号后 drop 流退出，麦克风随之释放。
pub fn start_listener(
    app: tauri::AppHandle,
    engine: Arc<Mutex<Engine>>,
    shared: Arc<ListenerShared>,
) -> Result<(), String> {
    eprintln!("[qibai] listener v18.8-fix7 (排除hw独占 + Pulse共享采集 + 双唤醒词 + mic rms 诊断)");
    if shared.enabled.load(Ordering::SeqCst) {
        return Ok(()); // 已在监听
    }
    // 先停掉可能残留的旧采集线程
    if let Some(tx) = shared.stop_tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = tx.send(());
    }
    shared.enabled.store(true, Ordering::SeqCst);
    shared.state.store(LISTEN_WAKE, Ordering::SeqCst);

    let (tx, rx) = tokio::sync::mpsc::channel::<(Vec<f32>, u32)>(8);

    // v18.8：用户指定的麦克风（设置页可选；空=自动打分选择）。
    // start_listener 非 async，Engine 锁是 tokio 锁——由异步任务读取后经 channel 交给采集线程。
    let (pref_tx, pref_rx) = std::sync::mpsc::channel::<String>();
    let eng2 = engine.clone();
    let app0 = app.clone(); // v18.8-fix17b：启动广播"待唤醒"状态用
    tauri::async_runtime::spawn(async move {
        let v = eng2.lock().await.settings.voice.clone();
        // 启动自检：打印当前语音配置，方便核对版本与设置是否生效。
        eprintln!(
            "[qibai] config: wake_word={} wake_enabled={} stt_input_device={:?} stt_engine={}",
            v.wake_word, v.wake_enabled, v.stt_input_device, v.stt_engine
        );
        // v18.8-fix17b：启动即广播"待唤醒"——此前启动不广播，前端监听标签永远不显示。
        // P0 回声根治：AEC 启用状态启动即打（env QIBAI_AEC=0 → false；延迟估计待核心模块暴露后补）。
        eprintln!("[qibai-aec] enabled={}", qibai_core::aec::aec_enabled());

        // B 线：语音引擎链路选择（VOICE_ENGINE=pipecat → 外接双工语音服务；
        // 缺省/legacy → 本函数下方原 P0-B+sherpa/whisper 链路，行为零变化）。
        let vk = qibai_core::realtime_voice_frontend::resolve_voice_engine();
        eprintln!("[voice-engine] kind={vk:?}");
        if vk == VoiceEngineKind::Pipecat {
            let ep = std::env::var("QIBAI_REALTIME_URL").unwrap_or_else(|_| {
                qibai_core::realtime_voice_frontend::DEFAULT_REALTIME_ENDPOINT.to_string()
            });
            match PipecatClient::connect(&ep).await {
                Ok(client) => {
                    eprintln!("[pipecat] ws connected {ep}");
                    let handle = Arc::new(tokio::sync::Mutex::new(client));
                    let ev_app = app0.clone();
                    let ev_handle = handle.clone();
                    let ev_engine = eng2.clone(); // 语音落库修复：事件循环需写 conversation
                    tauri::async_runtime::spawn(async move {
                        pipecat_event_loop(ev_handle, ev_app, ev_engine).await;
                    });
                    let _ = PIPECAT.set(handle);
                }
                Err(e) => {
                    eprintln!("[pipecat] connect failed ({e}) → 回退 legacy 链路");
                }
            }
        }
        let _ = app0.emit(
            "qibai://listen-state",
            serde_json::json!({ "state": "wake", "wake_word": v.wake_word }),
        );
        let _ = pref_tx.send(v.stt_input_device);
    });

    // ---- 状态机 task：句末音频 → 转写 → 处理 ----
    let st = shared.state.clone();
    let st2 = st.clone(); // 采集线程用（async task 独占 st）
    let en = shared.enabled.clone();
    let eng = engine.clone();
    let app2 = app.clone();
    let shared2 = shared.clone(); // v11：闭包内语音控制免提开关用（try_hf_command）
    let diag_text = shared.diag.last_text.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = rx;
        while en.load(Ordering::SeqCst) {
            // v20 barge-in：朗读期间用户打断 → 停播放 + 广播 tts-interrupted
            if BARGE_IN_REQUEST.load(Ordering::SeqCst) {
                BARGE_IN_REQUEST.store(false, Ordering::SeqCst);
                *LAST_BARGE_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                qibai_core::tts::stop_playback(); // 内含 clear_speaking（静音门立即解除）
                // v22-fix41（半双工·打断后记录）：stop 停的是播放队列，扬声器硬件缓冲里
                // 的 TTS 余音仍会被麦克风捕获——打断后保留 400ms 尾音丢弃窗口，
                // 再恢复监听记录用户打断后的内容。行业做法：半双工切换需留设备余音窗口。
                qibai_core::tts::mark_speaking_for(std::time::Duration::from_millis(400));
                let _ = app2.emit(
                    "tts-interrupted",
                    serde_json::json!({"reason":"user-barge-in"}),
                );
                eprintln!("[qibai] barge-in: 朗读已打断（尾音窗口 400ms 后恢复监听）");
            }
            let msg = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
            match msg {
                Ok(Some((samples, rate))) => {
                    if let Some(text) = transcribe(samples, rate).await {
                        *diag_text.lock().unwrap_or_else(|e| e.into_inner()) = text.clone(); // 诊断面板：最近转写
                        handle_text(&app2, &eng, &shared2, text).await;
                    }
                }
                _ => continue,
            }
        }
        st.store(LISTEN_OFF, Ordering::SeqCst);
    });

    // ---- 采集线程：创建流并 play，阻塞等待 stop 信号后 drop 退出 ----
    let (res_tx, res_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    *shared.stop_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(stop_tx);
    // v45.8：提前克隆 enabled 句柄，避免下方 move 闭包把 shared 移走
    let enabled_probe = shared.enabled.clone();
    std::thread::spawn(move || {
        // 最多等 300ms 拿用户指定设备名；拿不到则空=自动打分选择
        let pref_input = pref_rx.recv_timeout(std::time::Duration::from_millis(300)).unwrap_or_default();
        // v18.8-fix：stop_rx 移入 build_and_play —— 流在函数内阻塞保持活跃，
        // 收到 stop 信号才 drop 释放麦克风（修复"流立即 drop→听不到"的根因 bug）。
        let outcome = build_and_play(&tx, st2, pref_input, stop_rx, res_tx.clone(), &shared.diag);
        let _ = res_tx.send(outcome);
    });

    // v45.8：热插拔重探线程——设备丢失后每 3s 探测默认输入设备，回来自动恢复
    {
        std::thread::spawn(move || {
            let mut was_lost = false;
            while enabled_probe.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_secs(3));
                if !LISTENER_DEVICE_LOST.load(Ordering::SeqCst) {
                    was_lost = false;
                    continue;
                }
                // 设备丢失状态：探测是否恢复
                let host = cpal::default_host();
                match host.default_input_device() {
                    Some(dev) => {
                        let name = dev.name().unwrap_or_else(|_| "unknown".to_string());
                        LISTENER_DEVICE_LOST.store(false, Ordering::SeqCst);
                        if !was_lost {
                            eprintln!("[qibai] mic: device restored (<{name}>)");
                            was_lost = true;
                        }
                        // 注意：实际重新起采集由 build_and_play 外层重连逻辑负责；
                        // 这里只复位标志并打日志，避免在监控线程里直接操作 cpal stream。
                    }
                    None => {
                        was_lost = true; // 还没回来，保持
                    }
                }
            }
        });
    }

    // v18.8-fix3：recv 加 25s 超时——设备全失败/重试卡顿时，免提启用命令也必须
    // 及时返回（否则前端 invoke 永久等待 → 界面卡死、显示/交互全部失效）。
    let outcome = res_rx
        .recv_timeout(std::time::Duration::from_secs(25))
        .map_err(|e| format!("监听启动超时（25s）或线程退出: {e}"))?;
    outcome.map_err(|e| format!("{e}"))?;
    Ok(())
}

/// v18.8-fix：返回输入设备候选列表（按打分排序，供 build_and_play 逐个尝试打开）。
/// - 用户指定设备名 → 精确匹配（大小写不敏感）单设备；
/// - 否则枚举打分：mic/麦克优先，pulse/内置/analog 等加权，NVIDIA HDMI 输出降权；
/// - 无高分设备 → default_input_device 保底。
fn list_candidate_devices(host: &cpal::Host, preferred: &str) -> Vec<cpal::Device> {
    let mut out: Vec<cpal::Device> = Vec::new();
    if !preferred.trim().is_empty() {
        if let Ok(devs) = host.input_devices() {
            for d in devs {
                if let Ok(n) = d.name() {
                    if n.trim().eq_ignore_ascii_case(preferred.trim()) {
                        eprintln!("[qibai] audio input preferred: {}", n);
                        out.push(d.clone());
                        return out;
                    }
                }
            }
        }
        eprintln!(
            "[qibai] audio input preferred '{}' not found, fallback to scoring",
            preferred
        );
    }
    let mut scored: Vec<(i32, cpal::Device)> = Vec::new();
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            let name = d.name().unwrap_or_default().to_lowercase();
            let mut sc = 0;
            if name.contains("mic") || name.contains("麦克") {
                sc += 3;
            }
            // v18.8-fix7：pulse/default = PulseAudio 共享输入 = 系统当前激活的麦克风
            // （插耳机自动切换），不独占、不锁死其他程序——自动匹配的正解，恢复优先。
            if name.contains("pulse") || name == "default" || name.contains("default") {
                sc += 3;
            }
            // v18.8-fix4：耳机/耳麦/头戴优先（用户实际常用耳机麦克风），USB 设备加权
            if name.contains("headset") || name.contains("headphone") || name.contains("耳机")
                || name.contains("耳麦") || name.contains("handsfree") || name.contains("hands-free")
            {
                sc += 4;
            }
            if name.contains("usb") {
                sc += 2;
            }
            if name.contains("analog") || name.contains("内置") || name.contains("input")
                || name.contains("front") || name.contains("microphone")
            {
                sc += 2;
            }
            // v18.8-fix4：default/pulse 只是别名——可能指向输出监视器或内置 mic，
            // 不再盲目加权（此前 +2 导致耳机插着却选了 default → 听不到人声）。
            // monitor/loopback/echo 是回环采集（采电脑输出声），必须排除。
            // v18.8-fix7：ALSA hw:/plughw: 直连是独占模式——会锁死麦克风导致其他程序
            // 无法使用；且常指向主板声卡的错误输入（采不到耳机）。严重降权排除。
            if name.contains("hw:") || name.contains("plughw:") {
                sc -= 20;
            }
            if name.contains("monitor") || name.contains("loopback") || name.contains("echo")
                || name.contains("feedback")
            {
                sc -= 5;
            }
            if name.contains("hdmi") || name.contains("display") || name.contains("nvidia")
                || name.contains("dp ")
            {
                sc -= 2;
            }
            scored.push((sc, d.clone()));
            eprintln!("[qibai] audio input device: score={} name={}", sc, name);
        }
    }
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    // v18.8-fix3：高分候选优先（至多 6 个）——覆盖真麦克风排后位的问题，
    // 同时避免失败场景下对全部设备做重试导致监听启动卡死。
    for (_, d) in scored.into_iter().take(6) {
        out.push(d);
    }
    if let Some(d) = host.default_input_device() {
        let n = d.name().unwrap_or_default();
        let mut dup = false;
        for x in &out {
            if x.name().map(|a| a == n).unwrap_or(false) {
                dup = true;
                break;
            }
        }
        if !dup {
            eprintln!("[qibai] audio input fallback default: {}", n);
            out.push(d);
        }
    }
    out
}

/// v18.8-fix：在采集线程内打开输入流并保持活跃直到 stop 信号。
/// - 候选设备逐个尝试打开（第一个成功者生效），避免单设备打开失败导致"听不到"；
/// - 流在本函数内阻塞保持活跃——修复此前"函数返回即 drop 流→麦克风从未持续采集"的根因 bug。
fn build_and_play(
    tx: &tokio::sync::mpsc::Sender<(Vec<f32>, u32)>,
    st: Arc<AtomicU8>,
    preferred_input: String,
    stop_rx: std::sync::mpsc::Receiver<()>,
    started_tx: std::sync::mpsc::Sender<Result<(), String>>,
    diag: &VoiceDiag,
) -> Result<(), String> {
    let host = cpal::default_host();
    let devices = list_candidate_devices(&host, &preferred_input);
    if devices.is_empty() {
        return Err("未检测到麦克风设备".to_string());
    }
    let mut last_err = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    for device in devices {
        // v18.8-fix3：总时间预算 15s——防止无可用设备时逐设备重试把监听启动拖死
        if std::time::Instant::now() >= deadline {
            last_err = format!("{last_err}（尝试超时 15s）");
            break;
        }
        let dev_name = device.name().unwrap_or_default();
        match build_input_stream_for(&tx, &st, &device, &diag.rms, &diag.vad_count) {
            Ok((stream, rate, channels)) => {
                stream
                    .play()
                    .map_err(|e| format!("麦克风启动失败: {e}"))?;
                eprintln!(
                    "[qibai] audio capture started: {} @ {}Hz ch={} (stop to release)",
                    dev_name, rate, channels
                );
                *diag.device.lock().unwrap_or_else(|e| e.into_inner()) = dev_name.clone();
                *diag.rate.lock().unwrap_or_else(|e| e.into_inner()) = rate;
                // v18.8-fix4：成功启动必须**立即**通知调用方（此前等流结束才返回，
                // 调用方 25s 收不到 → 前端误报"监听未启动"——流其实已在采集）。
                let _ = started_tx.send(Ok(()));
                let _ = stop_rx.recv(); // 阻塞保持流活跃；stop → 流 drop → 麦克风释放
                return Ok(());
            }
            Err(e) => {
                last_err = format!("{dev_name}: {e}");
                eprintln!("[qibai] audio input open failed: {dev_name}: {e}");
            }
        }
    }
    Err(format!(
        "所有麦克风设备打开失败（{last_err}）。请自查：① pactl info 确认音频服务在运行（没有就 pulseaudio --start）；② arecord -l 确认系统有输入设备；③ 检查 ~/.asoundrc /etc/asound.conf 是否残留损坏配置；④ 设置页「麦克风选择」手动指定设备"
    ))
}

/// 为指定输入设备创建采集流。
/// v18.8-fix3：不再"默认配置失败即放弃"——先按设备默认配置尝试，
/// 失败后依次用常见采集格式重试（16k/44.1k/48k × mono/stereo × F32/I16），
/// 覆盖"设备默认配置与实际能力不符"导致的打开失败，让自动匹配真正生效。
fn build_input_stream_for(
    tx: &tokio::sync::mpsc::Sender<(Vec<f32>, u32)>,
    st: &Arc<AtomicU8>,
    device: &cpal::Device,
    diag_rms: &Arc<StdMutex<f32>>,
    diag_vad: &Arc<AtomicU32>,
) -> Result<(cpal::Stream, u32, usize), String> {
    // 组装候选配置：设备默认配置优先，其后为常见采集格式（去重）。
    let mut cfgs: Vec<(cpal::StreamConfig, cpal::SampleFormat)> = Vec::new();
    if let Ok(cfg) = device.default_input_config() {
        cfgs.push((
            cpal::StreamConfig {
                channels: cfg.channels(),
                sample_rate: cfg.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            },
            cfg.sample_format(),
        ));
    }
    // 常见采集格式：16k/44.1k/48k × 单声道 × F32/I16（麦克风以单声道为主，
    // 收敛重试组数——设备全失败时监听启动不能被重试拖死）。
    for rate in [16000u32, 44100, 48000] {
        for fmt in [cpal::SampleFormat::F32, cpal::SampleFormat::I16] {
            let sc = cpal::StreamConfig {
                channels: 1,
                sample_rate: cpal::SampleRate(rate),
                buffer_size: cpal::BufferSize::Default,
            };
            let dup = cfgs
                .iter()
                .any(|(c, f)| c.channels == 1 && c.sample_rate.0 == rate && *f == fmt);
            if !dup {
                cfgs.push((sc.clone(), fmt));
            }
        }
    }
    let mut last_err = String::new();
    for (sc, fmt) in cfgs {
        let rate = sc.sample_rate.0;
        let channels = sc.channels as usize;
        let capture = StdMutex::new(CaptureState::new(rate));
        let err_fn = |e| {
            // v45.8：采集流报错 → 设备丢失，置位标志（外层主循环检测到后暂停采集并清 pending）
            eprintln!("[qibai] listen: device lost: {e}");
            LISTENER_DEVICE_LOST.store(true, Ordering::SeqCst);
        };
        let dr = diag_rms.clone();
        let dv = diag_vad.clone();
        let r = match fmt {
            cpal::SampleFormat::F32 => {
                let tx2 = tx.clone();
                let st2 = st.clone();
                device
                    .build_input_stream(
                        &sc,
                        move |data: &[f32], _| {
                            feed_audio(data, channels, rate, &capture, &tx2, st2.as_ref(), &dr, &dv)
                        },
                        err_fn,
                        None,
                    )
                    .map(|s| (s, rate, channels))
            }
            cpal::SampleFormat::I16 => {
                let tx2 = tx.clone();
                let st2 = st.clone();
                let dr2 = diag_rms.clone();
                let dv2 = diag_vad.clone();
                device
                    .build_input_stream(
                        &sc,
                        move |data: &[i16], _| {
                            let buf: Vec<f32> =
                                data.iter().map(|&v| v as f32 / 32768.0).collect();
                            feed_audio(&buf, channels, rate, &capture, &tx2, st2.as_ref(), &dr2, &dv2)
                        },
                        err_fn,
                        None,
                    )
                    .map(|s| (s, rate, channels))
            }
            cpal::SampleFormat::U16 => {
                let tx2 = tx.clone();
                let st2 = st.clone();
                let dr3 = diag_rms.clone();
                let dv3 = diag_vad.clone();
                device
                    .build_input_stream(
                        &sc,
                        move |data: &[u16], _| {
                            let buf: Vec<f32> = data
                                .iter()
                                .map(|&v| v as f32 / 65535.0 * 2.0 - 1.0)
                                .collect();
                            feed_audio(&buf, channels, rate, &capture, &tx2, st2.as_ref(), &dr3, &dv3)
                        },
                        err_fn,
                        None,
                    )
                    .map(|s| (s, rate, channels))
            }
            other => return Err(format!("不支持的采样格式: {other:?}")),
        };
        match r {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = format!("{}Hz/{}ch/{:?}: {e}", sc.sample_rate.0, sc.channels, fmt);
            }
        }
    }
    Err(format!("该设备所有采集配置均失败（{last_err}）"))
}

/// 停止监听（发 stop 信号 → 采集线程 drop 流；状态机 task 因 enabled=false 在 1s 内退出）。
pub fn stop_listener(shared: &ListenerShared) {
    shared.enabled.store(false, Ordering::SeqCst);
    shared.state.store(LISTEN_OFF, Ordering::SeqCst);
    if let Some(tx) = shared.stop_tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = tx.send(());
    }
}

// ---------------------------------------------------------------- VAD

const FRAME_MS: u32 = 20;
const RMS_THRESHOLD: f32 = 0.005; // v18.8-fix4：0.008 偏高，低电平麦克风永不触发 VAD
const SPEECH_START_FRAMES: u32 = 3;
const SILENCE_END_FRAMES: u32 = 45; // v18.8-fix23：0.9s 静默 → 句结束（原 1.2s 缩短，减少说完话后的等待）
const MAX_UTTERANCE_SECS: usize = 10;

struct CaptureState {
    frame_buf: Vec<f32>,
    pending: Vec<f32>,
    in_speech: bool,
    speech_frames: u32,
    silence_frames: u32,
    rms_log_frames: u64,
    barge_frames: u32,
    /// fix37: TTS_SPEAKING 静音门连续时长（watchdog 用，防残留致麦克风永久静默）
    speaking_since: Option<std::time::Instant>,
    /// P0-C2：残响抑制窗——TTS_SPEAKING true→false 边沿记朗读结束时刻，
    /// 结束后 2.5s 内断句按回声丢弃（覆盖播放完成后的扬声器残响 + 麦克风捕获延迟）。
    prev_speaking: bool,
    tts_ended_at: Option<std::time::Instant>,
    /// P0-B：回声门 DTD 判定状态（播放能量 vs 麦克风能量对齐；纯数据、采集线程单例）。
    echo_gate: qibai_core::tts::EchoGateState,
}

impl CaptureState {
    #[allow(unused_variables)]
    fn new(rate: u32) -> Self {
        Self {
            frame_buf: Vec::new(),
            pending: Vec::new(),
            in_speech: false,
            speech_frames: 0,
            silence_frames: 0,
            rms_log_frames: 0,
            barge_frames: 0,
            speaking_since: None,
            prev_speaking: false,
            tts_ended_at: None,
            echo_gate: qibai_core::tts::EchoGateState::default(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn feed_audio(
    samples: &[f32],
    channels: usize,
    rate: u32,
    cap: &StdMutex<CaptureState>,
    tx: &tokio::sync::mpsc::Sender<(Vec<f32>, u32)>,
    st: &AtomicU8,
    diag_rms: &Arc<StdMutex<f32>>,
    diag_vad: &Arc<AtomicU32>,
) {
    if st.load(Ordering::SeqCst) == LISTEN_OFF {
        return;
    }
    // v45.8：设备丢失 → 当场清空 pending，旧帧一律不送转写/发送
    if LISTENER_DEVICE_LOST.load(Ordering::SeqCst) {
        let mut c = cap.lock().unwrap_or_else(|e| e.into_inner());
        if !c.pending.is_empty() {
            eprintln!("[qibai] mic: device lost, clearing {} pending frames", c.pending.len());
            c.pending.clear();
        }
        return;
    }
    let mut c = cap.lock().unwrap_or_else(|e| e.into_inner());
    let frame_len = (rate * FRAME_MS / 1000) as usize;
    let mut idx = 0;
    while idx + channels <= samples.len() {
        c.frame_buf.push(samples[idx]); // 取第一声道
        idx += channels;
        if c.frame_buf.len() >= frame_len {
            let mut frame: Vec<f32> = c.frame_buf.drain(..frame_len).collect();
            // P0 回声根治：AEC 残差替换本帧（启用时内部 16k 时钟、返回 16000；
            // 未启用时直通、new_rate==原 rate）。残差同时喂 barge-in 判定与 whisper——
            // 下游 RMS / pending / 断句 / 重采样全部吃残差，改动最小。
            let new_rate = qibai_core::aec::process_frame(&mut frame, rate);
            // B 线双工：pipecat 模式下残差帧直推语音服务（barge-in/话轮/回声由服务端
            // VAD+状态机处理），跳过 legacy 静音门/VAD/pending/whisper。legacy 时
            // PIPECAT 为 None，本分支不进入，行为与现状完全一致。
            if let Some(handle) = PIPECAT.get() {
                if let Ok(c) = handle.try_lock() {
                    // new_rate → 16k 抽稀（AEC 开时 new_rate 即 16000，ratio=1）。
                    let ratio = (new_rate / 16_000).max(1) as usize;
                    let pcm16: Vec<i16> = frame
                        .iter()
                        .step_by(ratio)
                        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                        .collect();
                    c.push_audio(&pcm16);
                }
                continue;
            }
            let rms = (frame.iter().map(|v| v * v).sum::<f32>() / frame.len() as f32).sqrt();
            *diag_rms.lock().unwrap_or_else(|e| e.into_inner()) = rms; // 诊断面板实时值
            // P0-C2 残响窗边沿：TTS_SPEAKING true→false 记朗读结束时刻（残响抑制起点）。
            if !qibai_core::tts::TTS_SPEAKING.load(std::sync::atomic::Ordering::SeqCst)
                && c.prev_speaking
            {
                c.tts_ended_at = Some(std::time::Instant::now());
            }
            c.prev_speaking = qibai_core::tts::TTS_SPEAKING.load(std::sync::atomic::Ordering::SeqCst);
            // v18.8-fix28j：TTS 朗读期间麦克风静默（行业 AEC 替代方案）——
            // 扬声器声音被麦克风捕获会触发 VAD 误判。
            // v20 barge-in：「打断检测模式」——连续高能量帧判定用户真说话，置 BARGE_IN_REQUEST。
            // P0 回声根治：硬静音门是否丢帧按 aec_enabled() 分支（见下）。
            if TTS_SPEAKING.load(std::sync::atomic::Ordering::SeqCst) {
                // v22-fix37: 静音门 watchdog——连续超过 90s 判定残留（正常朗读最长数十秒），
                // 强制解除，避免某次朗读异常后麦克风永久静默「听不到用户声音」。
                let now = std::time::Instant::now();
                match c.speaking_since {
                    Some(t) if now.duration_since(t) > std::time::Duration::from_secs(90) => {
                        qibai_core::tts::clear_speaking();
                        c.speaking_since = None;
                        c.barge_frames = 0;
                        eprintln!("[qibai] TTS_SPEAKING 残留超 90s，已强制解除静音门（watchdog）");
                    }
                    None => c.speaking_since = Some(now),
                    _ => {}
                }
                // P0-B 回声门（能量对齐 DTD 兜底，独立于 AEC；env QIBAI_ECHO_GATE=0 关闭）：
                // 播放能量 vs 麦克风能量对齐——纯回声→抑制不转写；人声/双讲→barge-in+放行。
                // 优先级：watchdog(上) > P0-B(本块) > v21 legacy 判定层(下方，P0-B 关时生效)。
                if qibai_core::tts::echo_gate_enabled() {
                    let play_rms_now = qibai_core::tts::echo_gate_play_rms_now();
                    use qibai_core::tts::{echo_gate_step, EchoGateVerdict};
                    match echo_gate_step(&mut c.echo_gate, rms, play_rms_now) {
                        EchoGateVerdict::Quiet => {
                            // 播放安静（尾音窗）：视为回声抑制，防尾音漏网自激。
                            c.in_speech = false;
                            c.speech_frames = 0;
                            c.silence_frames = 0;
                            continue;
                        }
                        EchoGateVerdict::EchoOnly => {
                            // 纯回声：抑制本帧，不喂 VAD/pending，不转写（切断自激闭环）。
                            continue;
                        }
                        EchoGateVerdict::DoubleTalk => {
                            // 人声优先：立即 barge-in；帧落回下方正常 VAD/pending 转写。
                            if !BARGE_IN_REQUEST.load(std::sync::atomic::Ordering::SeqCst) {
                                BARGE_IN_REQUEST.store(true, std::sync::atomic::Ordering::SeqCst);
                                eprintln!("[qibai] barge-in: echo-gate 双讲判定，请求打断朗读");
                            }
                        }
                    }
                }
                if !qibai_core::tts::echo_gate_enabled() {
                // P0 回声根治·三层降级：
                // ① barge-in 朗读中统一 0.04/12 帧（TTS 声音能量高，回声易误判打断）。
                // ② 朗读期间无条件硬丢帧（NLMS AEC 压不住回声，软提示=残响全进 VAD/whisper）。
                // ③ 90s watchdog 无条件保留（防 TTS_SPEAKING 残留致 mic 永久静默）。
                // v8：朗读中 barge 阈值自适应（行业 DTD 能量联合判定）——
                // 播放越响，回声能量越高，固定阈值易误判打断。基准 0.04/12 帧，
                // 叠加当前播放 RMS 的一半（封顶 0.08，防轻声真话漏打断）：TTS 回声
                // 增益通常 <0.5 够不着，真说话持续 240ms 高能量仍必触发。
                let play_now = qibai_core::tts::echo_gate_play_rms_now();
                let bar_thr = (0.04f32).max((play_now * 0.5).min(0.08));
                let bar_frames = 12u32;
                if rms > bar_thr {
                    c.barge_frames += 1;
                    if c.barge_frames >= bar_frames
                        && !BARGE_IN_REQUEST.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        BARGE_IN_REQUEST.store(true, std::sync::atomic::Ordering::SeqCst);
                        eprintln!("[qibai] barge-in: 检测到用户语音，请求打断朗读");
                    }
                } else {
                    c.barge_frames = 0;
                }
                // v8：无论 AEC 是否启用都硬丢帧——NLMS 简易 AEC 实测压不住回声，
                // 软提示=残响全流经 VAD/whisper（真机日志 5~8s 断句循环根因）。
                // v45 barge-in 危险项修复：一旦 BARGE_IN_REQUEST 已确认（用户真声
                // 打断），不再 clear pending、不再 continue 丢帧——保留用户这句话进
                // 断句/转写通道，否则「停/停下/别做了」被当场丢掉，停止意图识别不到、
                // 任务继续跑。仅在尚未确认 barge-in 时硬丢回声帧。
                c.in_speech = false;
                c.speech_frames = 0;
                c.silence_frames = 0;
                if BARGE_IN_REQUEST.load(std::sync::atomic::Ordering::SeqCst) {
                    // 用户已真声打断：保留 pending，落到下方 VAD 管道正常处理/转写。
                    eprintln!("[qibai] barge-in: 已确认用户语音，保留 pending 转写停止意图 (pending={} frames)", c.pending.len());
                } else {
                    c.pending.clear();
                    continue;
                }
                }
            } else {
                c.speaking_since = None;
            }
            // v20 降频：RMS 诊断日志默认 5s 一次（原 1s 一次=日志风暴）；
            // 设 QIBAI_VAD_LOG=0 可完全关闭。
            let _ = c.rms_log_frames.saturating_add(1);
            c.rms_log_frames += 1;
            let want_log = c.rms_log_frames % 250 == 0;
            let log_on = std::env::var("QIBAI_VAD_LOG")
                .map(|v| v != "0")
                .unwrap_or(true);
            if want_log && log_on {
                eprintln!(
                    "[qibai] mic rms={rms:.5} in_speech={} pending={} frames (wire_rate={new_rate})",
                    c.in_speech, c.pending.len()
                );
            }
            let active = rms > RMS_THRESHOLD;
            if active {
                c.speech_frames += 1;
                c.silence_frames = 0;
                if !c.in_speech && c.speech_frames >= SPEECH_START_FRAMES {
                    c.in_speech = true;
                    // 借鉴 Qwen-Audio-Agent 投递窗口：VAD 起句边沿上报「用户在说话」，
                    // 供 tts.rs delivery_window_now() 判定是否该挂起结果投递。
                    qibai_core::tts::set_user_speaking(true);
                }
            } else {
                c.silence_frames += 1;
                if c.in_speech && c.silence_frames >= SILENCE_END_FRAMES {
                    if !c.pending.is_empty() {
                        let secs = c.pending.len() as f64 / new_rate as f64;
                        // P0-C 回声根治·硬门：朗读期间（TTS_SPEAKING=true）或朗读结束后
                        // 2.5s 残响窗内（扬声器余音 + 麦克风捕获延迟，whisper 慢转写时 fix38
                        // 文本窗会过期漏网），除非用户已触发打断，断句一律丢弃。
                        // 打断（BARGE_IN_REQUEST）会停播放并解除静音，用户话不丢。
                        let tts_now = qibai_core::tts::TTS_SPEAKING.load(std::sync::atomic::Ordering::SeqCst);
                        let within_reverb = c
                            .tts_ended_at
                            .map(|t| t.elapsed().as_millis() < 4000)
                            .unwrap_or(false);
                        if (tts_now || within_reverb)
                            && !BARGE_IN_REQUEST.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            let _ = std::mem::take(&mut c.pending);
                            eprintln!("[qibai] echo-gate: 朗读期间/残响窗断句丢弃({secs:.2}s)");
                        } else {
                            let seg_len = c.pending.len();
                            let seg = std::mem::take(&mut c.pending);
                            eprintln!("[qibai] vad: 断句 {seg_len} samples ({secs:.2}s) 送入转写");
                            diag_vad.fetch_add(1, Ordering::Relaxed); // 诊断面板断句计数
                            let _ = tx.try_send((seg, new_rate));
                        }
                    }
                    c.in_speech = false;
                    c.speech_frames = 0;
                    c.silence_frames = 0;
                    qibai_core::tts::set_user_speaking(false);
                }
            }
            if c.in_speech {
                c.pending.append(&mut frame);
            }
            // P0：10s 上限按残差流速率换算（样本数阈值随 new_rate 缩放；帧数累计语义不变）。
            if c.pending.len() > new_rate as usize * MAX_UTTERANCE_SECS {
                // P0-C：朗读期间/残响窗同样丢弃（同断句硬门，防 10s 上限把朗读残响送转写）。
                let tts_now = qibai_core::tts::TTS_SPEAKING.load(std::sync::atomic::Ordering::SeqCst);
                let within_reverb = c
                    .tts_ended_at
                    .map(|t| t.elapsed().as_millis() < 4000)
                    .unwrap_or(false);
                if (tts_now || within_reverb)
                    && !BARGE_IN_REQUEST.load(std::sync::atomic::Ordering::SeqCst)
                {
                    let _ = std::mem::take(&mut c.pending);
                    eprintln!("[qibai] echo-gate: 朗读期间/残响窗超长断句丢弃");
                } else {
                    let seg = std::mem::take(&mut c.pending);
                    let _ = tx.try_send((seg, new_rate));
                }
                c.in_speech = false;
                c.speech_frames = 0;
                c.silence_frames = 0;
                qibai_core::tts::set_user_speaking(false);
            }
        }
    }
}

// ---------------------------------------------------------------- 转写

/// 重采样到 16k → wav（i16 PCM）→ whisper 本地转写。失败返回 None（不打断监听）。
async fn transcribe(samples: Vec<f32>, rate: u32) -> Option<String> {
    let wav = match resample_to_wav(&samples, rate, 16_000) {
        Some(w) => w,
        None => {
            eprintln!("[qibai] 转写: 重采样失败（rate={rate}）");
            return None;
        }
    };
    let t0 = std::time::Instant::now();
    // v8-sherpa：转写引擎接入 sherpa（阿里 FunASR Paraformer-zh int8，中文准、转写快），
    // whisper 自动回退。sherpa 子进程转写远快于 whisper-server，fix38 文本窗口不再过期。
    let engine = qibai_core::sherpa_engine::active_engine();
    eprintln!("[qibai] stt 引擎选择: {engine}");
    let out = match engine {
        "paraformer" => match qibai_core::sherpa_engine::transcribe_wav_bytes(&wav).await {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("[qibai] paraformer 转写失败: {e}；回退 whisper");
                match whisper_engine::transcribe_wav_bytes(&wav).await {
                    Ok(t) => Some(t),
                    Err(e2) => {
                        eprintln!("[qibai] 转写失败: {e2}");
                        None
                    }
                }
            }
        },
        _ => match whisper_engine::transcribe_wav_bytes(&wav).await {
            Ok(t) => Some(t),
            Err(e) => {
                // v18.8-fix11：转写失败可见化——之前 .ok() 把原因全吞了，听不到却无从查起。
                eprintln!("[qibai] 转写失败: {e}");
                None
            }
        },
    };
    // v22-fix38：转写侧回声过滤——TTS_SPEAKING 估算偏差漏网的 TTS 回声，
    // 按「与最近朗读文本高度重叠」丢弃，杜绝元气小小听到自己的话（行业 AEC 文本层兜底）。
    let out = out.and_then(|t| {
        if qibai_core::tts::is_echo_of_recent_speech(&t) {
            // v10 误打断诊断：若此回声转写出现在「用户打断朗读」之后极短时间窗内，
            // 说明那次打断极可能是 TTS 回声误触发（用户其实没说话）。
            if let Some(b) = *LAST_BARGE_AT.lock().unwrap_or_else(|e| e.into_inner()) {
                let since = b.elapsed().as_millis();
                if since < 2500 {
                    eprintln!(
                        "[qibai] echo-filter: 疑似误打断（打断后 {}ms 收到回声转写，那次打断可能误触发）",
                        since
                    );
                }
            }
            eprintln!("[qibai] echo-filter: 丢弃疑似回声转写({}字)", t.chars().count());
            None
        } else {
            Some(t)
        }
    });
    eprintln!("[qibai] stt took {}ms", t0.elapsed().as_millis());
    out
}

/// 线性插值重采样 + WAV 编码（RIFF + i16 PCM）。
fn resample_to_wav(samples: &[f32], from: u32, to: u32) -> Option<Vec<u8>> {
    if from == 0 || to == 0 || samples.is_empty() {
        return None;
    }
    // 重采样到 to
    let ratio = from as f64 / to as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut resampled: Vec<f32> = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = (i as f64 * ratio) as f64;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(samples.len() - 1);
        let frac = (pos - i0 as f64) as f32;
        let v = samples[i0] * (1.0 - frac) + samples[i1] * frac;
        resampled.push(v.clamp(-1.0, 1.0));
    }

    // WAV 编码
    let data_len = resampled.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&to.to_le_bytes());
    out.extend_from_slice(&(to * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for v in resampled {
        out.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
    }
    Some(out)
}

// ---------------------------------------------------------------- 状态机

/// v11：语音控制免提（双工）开关——「打开/关闭/切换免提」等本地规则直达执行，
/// 不送 LLM。返回 Some(()) 表示已处理（调用方不应再走 reply）。
/// 语音开启免提 = 调 start_listener（唤醒词+VAD 持续聆听）；关闭 = stop_listener。
async fn try_hf_command(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    shared: &Arc<ListenerShared>,
    text: &str,
) -> Option<()> {
    let open = [
        "打开免提", "开启免提", "启动免提", "打开双工", "开启双工", "启动双工",
        "免提开启", "双工开启", "打开聆听", "开启聆听", "启动聆听",
    ];
    let close = [
        "关闭免提", "关掉免提", "停止免提", "关闭双工", "关掉双工", "停止双工",
        "免提关闭", "双工关闭", "停止聆听", "关闭聆听",
    ];
    let toggle = ["切换免提", "免提切换", "切换双工", "双工切换", "切换聆听", "聆听切换"];
    let on_now = shared.state.load(Ordering::SeqCst) != LISTEN_OFF;
    let want: Option<bool> = if toggle.iter().any(|w| text.contains(w)) {
        Some(!on_now)
    } else if open.iter().any(|w| text.contains(w)) {
        Some(true)
    } else if close.iter().any(|w| text.contains(w)) {
        Some(false)
    } else {
        None
    };
    let want = want?;
    if want {
        if on_now {
            speak_and_emit_pet_say(app, engine, "免提已经开着啦，直接说话就行", 3000, Some("happy"))
                .await;
        } else {
            match start_listener(app.clone(), engine.clone(), shared.clone()) {
                Ok(()) => {
                    speak_and_emit_pet_say(app, engine, "免提已开启，直接说话就行", 3000, Some("happy"))
                        .await
                }
                Err(e) => {
                    eprintln!("[qibai] 语音开启免提失败: {e}");
                    speak_and_emit_pet_say(app, engine, "免提开启失败，看看麦克风是不是被占用了", 3000, Some("sad"))
                        .await
                }
            }
        }
    } else if !on_now {
        speak_and_emit_pet_say(app, engine, "免提已经关着啦，按住按钮就能说话", 3000, Some("happy"))
            .await;
    } else {
        stop_listener(shared);
        speak_and_emit_pet_say(app, engine, "免提已关闭，按住按钮说话就行", 3000, Some("happy"))
            .await;
    }
    Some(())
}

async fn handle_text(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    shared: &Arc<ListenerShared>,
    text: String,
) {
    let st = &shared.state;
    if st.load(Ordering::SeqCst) == LISTEN_OFF {
        return;
    }
    let wake_word = engine.lock().await.settings.voice.wake_word.clone();
    let end_words = ["结束", "拜拜", "再见", "退出", "先这样", "不聊了"];

    match st.load(Ordering::SeqCst) {
        LISTEN_WAKE => {
            // v18.8-fix4：兼容双唤醒词——设置旧值可能仍是「元气小小」，无论设置为何，
            // 「小小」「元气小小」都能唤醒（提示语统一用当前设置值）。
            let hit = text.contains("小小") || text.contains("元气小小") || text.contains(&wake_word);
            if hit {
                let used: &str = if text.contains("小小") { "小小" } else { wake_word.as_str() };
                st.store(LISTEN_ACTIVE, Ordering::SeqCst);
                emit_listen(app, "active", used);
                pet_say(app, engine, "我在呢，想做什么直接说～").await;
                // 唤醒词后的剩余文本当作首条指令
                let rest = text.replace(used, "").trim().to_string();
                if !rest.is_empty() && !is_end(&rest, &end_words) {
                    // v11 语音控制免提开关优先（不发 LLM）
                    if try_hf_command(app, engine, shared, &rest).await.is_none() {
                        reply(app, engine, &rest).await;
                    }
                }
            }
        }
        LISTEN_ACTIVE => {
            if is_end(&text, &end_words) {
                st.store(LISTEN_WAKE, Ordering::SeqCst);
                emit_listen(app, "wake", &wake_word);
                pet_say(app, engine, "好的，有需要再叫我～").await;
            } else {
                // v11 语音控制免提开关优先（不发 LLM）
                if try_hf_command(app, engine, shared, &text).await.is_none() {
                    reply(app, engine, &text).await;
                }
            }
        }
        _ => {}
    }
}

fn is_end(text: &str, ends: &[&str]) -> bool {
    ends.iter().any(|w| text.contains(w))
}

/// 免提对话：走 Engine（本地意图直达 / LLM），回复经 qibai://pet-say 演出 + 按角色音色发声。
/// v16：语音技能调用——文本含「调用/使用/执行/帮我」且命中技能库（id/名称/函数名）时，
/// 执行技能并返回结果文本（不命中返回 None，走正常对话）。
async fn try_skill(app: &tauri::AppHandle, engine: &Arc<Mutex<Engine>>, text: &str) -> Option<String> {
    let head = ["调用", "使用", "执行", "帮我"];
    if !head.iter().any(|k| text.contains(k)) {
        return None;
    }
    let skills = engine.lock().await.skill_list();
    let mut best: Option<(String, String)> = None;
    for sk in &skills {
        let id = sk["id"].as_str().unwrap_or("").to_string();
        let name = sk["name"].as_str().unwrap_or("").to_string();
        let mut fns: Vec<String> = Vec::new();
        if let Some(a) = sk["functions"].as_array() {
            for f in a {
                if let Some(n) = f["name"].as_str() {
                    fns.push(n.to_string());
                }
            }
        }
        let hit = (!name.is_empty() && text.contains(&name))
            || fns.iter().any(|f| text.contains(f))
            || text.contains(&id);
        if hit && id.len() > best.as_ref().map(|(i, _)| i.len()).unwrap_or(0) {
            best = Some((id, fns.first().cloned().unwrap_or_default()));
        }
    }
    let (id, function) = best?;
    let out = engine.lock().await.skill_invoke(&id, &function, text).ok()?;
    if out.trim().is_empty() {
        return None;
    }
    // v18.1：技能输出面板化——解析 @qibai-panel 协议块，气泡/面板窗展示，其余文本朗读。
    // v18.6 落盘模式：技能输出 @qibai-save 块 → 安全校验 → 写盘 → 反馈路径（绝不覆盖）
    if let Some((path, b64)) = qibai_core::skills::parse_skill_save_block(&out) {
        match save_skill_output(&path, &b64) {
            Ok(p) => return Some(format!("代码已保存到 {p}，可以直接打开使用啦～")),
            Err(e) => return Some(e),
        }
    }

    let (payload, speak) = qibai_core::panel::parse_panel_annotation(&out);
    if let Some(p) = payload {
        if p.mode == "bubble" {
            let _ = app.emit(
                "qibai://ui-panel",
                serde_json::json!({ "title": p.title, "html": p.html }),
            );
        } else {
            if ensure_panel_window(app).is_some() {
                let _ = app.emit(
                    "qibai://panel-content",
                    serde_json::json!({ "title": p.title, "html": p.html }),
                );
            }
        }
    }
    Some(speak)
}

/// v18 面板意图：语音直达「把聊天记录/技能/专家 用 表格/面板/窗口 展示」。
/// 命中则生成白名单 HTML 并打开独立面板窗 + 气泡提示；不命中返回 None 走正常对话。
async fn try_panel_intent(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    text: &str,
) -> Option<String> {
    let action = ["展示", "查看", "打开", "给我看", "调出", "列出来", "汇总", "显示"];
    if !action.iter().any(|k| text.contains(k)) {
        return None;
    }
    let ui = ["表格", "面板", "窗口", "列表", "清单", "大屏", "界面"];
    if !ui.iter().any(|k| text.contains(k)) {
        return None;
    }
    // 内容词 → 内置面板路由
    let panel: Option<(&str, &str)> = if text.contains("聊天记录") || text.contains("对话记录") {
        Some(("chat_log_panel", "最近聊天记录"))
    } else if text.contains("技能") || text.contains("技能库") {
        Some(("skills_panel", "技能库"))
    } else if text.contains("专家") || text.contains("专家库") {
        Some(("experts_panel", "专家库"))
    } else {
        None
    };
    let (kind, title) = panel?;
    let msgs = engine.lock().await.get_conversation().to_vec();
    let skills = engine.lock().await.skill_list();
    let experts = engine.lock().await.expert_list();
    let html = match kind {
        "chat_log_panel" => qibai_core::panel::chat_log_table(&msgs, 30),
        "skills_panel" => qibai_core::panel::skill_table(&skills),
        _ => qibai_core::panel::expert_table(&experts),
    };
    // 确保独立面板窗存在（与 main.rs ensure_panel_window 同构；窗口加载 panel.html）
    let _w = ensure_panel_window(app);
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": title, "html": html }),
    );
    Some(format!("给你看～{title}已经整理好了，在面板窗口里。"))
}

/// 复用/创建独立面板窗（panel.html）。与 main.rs ensure_panel_window 保持同构。
fn ensure_panel_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(win) = app.get_webview_window("panel") {
        let _ = win.show();
        let _ = win.set_focus();
        return Some(win);
    }
    WebviewWindowBuilder::new(app, "panel", WebviewUrl::App("panel.html".into()))
        .inner_size(720.0, 540.0)
        .title("元气小小 · 面板")
        .decorations(true)
        .resizable(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .build()
        .ok()
}

/// v20 统一发声 helper（五处回复共用，消除复制漂移）：
/// Rust 朗读兜底（tts_decide→perform_speech）+ pet-say 演出，Rust 已发声则置
/// rust_played=true 防前端 say() 双响。persona 由 helper 内取 active_persona。
async fn speak_and_emit_pet_say(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    text: &str,
    ms: u64,
    emotion: Option<&str>,
) {
    let mut rust_played = false;
    if !text.trim().is_empty() {
        let req = {
            let eng = engine.lock().await;
            let pid = eng.personas.active_persona().to_string();
            eng.tts_decide(qibai_core::tts::TtsKind::Chat, text, Some(&pid))
        };
        if let Some(r) = req {
            super::perform_speech(app, engine, r).await;
            rust_played = true;
        }
    }
    let persona = engine.lock().await.personas.active_persona().to_string();
    let mut p = serde_json::json!({ "persona_id": persona, "text": text, "ms": ms });
    if let Some(e) = emotion {
        p["emotion"] = serde_json::json!(e);
    }
    if rust_played {
        p["rust_played"] = serde_json::json!(true);
    }
    let _ = app.emit("qibai://pet-say", p);
}

async fn reply(app: &tauri::AppHandle, engine: &Arc<Mutex<Engine>>, text: &str) {
    // v18.6 形象切换意图最优先（免提语音「切换到温柔形象」直达，避免被屏幕「切到窗口」抢占）。
    if let Some(persona_out) = try_persona_intent(app, engine, text).await {
        speak_and_emit_pet_say(app, engine, &persona_out, 4000, None).await;
        return;
    }
    // v18.3 屏幕意图优先：截图/键鼠/窗口操作（「眼睛 + 手」，防注入隔离：
    // 屏幕内容只作展示，操作仅由本条语音指令触发）。
    if let Some(screen_out) = try_screen_intent(app, engine, text).await {
        speak_and_emit_pet_say(app, engine, &screen_out, 4000, None).await;
        return;
    }
    // 面板意图优先：语音「展示聊天记录表格」直达内置面板，不走 LLM。
    if let Some(panel_out) = try_panel_intent(app, engine, text).await {
        speak_and_emit_pet_say(app, engine, &panel_out, 4000, None).await;
        return;
    }
    // 技能调用优先：命中技能库则执行并朗读结果，不消耗 LLM。
    if let Some(skill_out) = try_skill(app, engine, text).await {
        speak_and_emit_pet_say(app, engine, &skill_out, 4000, None).await;
        return;
    }
    let llm_t0 = std::time::Instant::now();
    let result = {
        let mut e = engine.lock().await;
        let persona = e.personas.active_persona().to_string();
        match e.send_message(text).await {
            Ok(o) => Some((persona, o)),
            Err(err) => {
                eprintln!("[qibai] listen reply error: {err}");
                // v18.8-fix13：明确区分"没配对话密钥"与其他失败，语音播报引导
                let err_s = err.to_string();
                let fb = if err_s.contains("API key") {
                    "我还没配置对话密钥，在控制台·设置·API Key 填好豆包密钥就能正常聊天了。现在可以先说「帮我打开设置」这类本地指令。"
                } else if qibai_core::llm_client::is_balance_error(&err_s) {
                    "大模型服务欠费了，到火山方舟控制台充值后我就能正常对话。"
                } else {
                    "抱歉，刚才这句没听懂，换个说法试试？"
                };
                let _ = speak_and_emit_pet_say(app, engine, &fb, 5000, Some("confused"));
                None
            }
        }
    };
    eprintln!("[qibai] llm took {}ms", llm_t0.elapsed().as_millis());
    if let Some((persona, o)) = result {
        // v20：secret 回复（含密钥）不朗读，仅前端演出；其余走统一 helper 发声。
        if o.secret == Some(true) || o.reply.trim().is_empty() {
            let mut p = serde_json::to_value(&o).unwrap_or_else(|_| serde_json::json!({}));
            p["persona_id"] = serde_json::json!(persona);
            p["ms"] = serde_json::json!(3500);
            let _ = app.emit("qibai://pet-say", p);
        } else {
            speak_and_emit_pet_say(app, engine, &o.reply, 3500, None).await;
        }
    }
}

/// 按角色音色发声：tts_decide 已按 persona voice_profile 解析 pitch/rate/voice。
/// 本地用捆绑 espeak-ng（零依赖安装）；云端需 key（无 key 静默，气泡已演出）。

/// 云端 wav bytes → 临时文件 + 探测播放器（aplay/paplay）。失败静默。

fn emit_listen(app: &tauri::AppHandle, state: &str, wake_word: &str) {
    let _ = app.emit(
        "qibai://listen-state",
        serde_json::json!({ "state": state, "wake_word": wake_word }),
    );
}

async fn pet_say(app: &tauri::AppHandle, engine: &Arc<Mutex<Engine>>, text: &str) {
    let persona = engine.lock().await.personas.active_persona().to_string();
    let _ = app.emit(
        "qibai://pet-say",
        serde_json::json!({
            "persona_id": persona,
            "text": text,
            "emotion": "happy",
            "ms": 3000,
        }),
    );
}

/// v18.6 技能产物落盘：路径安全校验（HOME 内 / 防穿越 / 扩展名白名单 / 不覆盖）→ base64 解码 → 写入。
fn save_skill_output(path: &str, b64: &str) -> Result<String, String> {
    use base64::Engine as _;
    let home = std::env::var("HOME").unwrap_or_default();
    if home.trim().is_empty() {
        return Err("无法确定用户目录，请直接说想保存到哪里".to_string());
    }
    let target = qibai_core::skills::validate_save_target(path, &home)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("代码解码失败：{e}"))?;
    if let Some(parent) = std::path::Path::new(&target).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    std::fs::write(&target, &bytes).map_err(|e| format!("保存失败：{e}"))?;
    Ok(target)
}

/// v18.3 屏幕「眼睛 + 手」语音意图：截图（低危直接做）、键鼠/窗口操作（需 GUI 授权）。
/// 命中返回要朗读的提示文本；不命中返回 None 走正常对话。
/// 防注入隔离：屏幕内容只作展示/理解，操作仅由本语音指令显式触发。
/// v18.6 免提语音切形象：说「切换到温柔形象」「换成活泼的」「换上都市女性」直达切换。
/// 匹配形象关键词（含英文 key 别名）；命中则复用命令级窗口同步逻辑并返回反馈文本。
async fn try_persona_intent(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    text: &str,
) -> Option<String> {
    let heads = ["切换到", "切换成", "换到", "换成", "换上", "变到", "变成", "用"];
    if !heads.iter().any(|h| text.contains(h)) {
        return None;
    }
    let map: [(&str, &[&str]); 5] = [
        ("gudian", &["古典佳人", "古典", "gudian", "古风"]),
        ("dushi", &["都市女性", "都市", "知性", "dushi"]),
        ("wenrou", &["温柔形象", "温柔", "wenrou"]),
        ("huopo", &["活泼形象", "活泼", "元气", "huopo"]),
        ("reqing", &["热情时尚", "热情", "时尚", "reqing", "热情款"]),
    ];
    for (id, keys) in &map {
        if keys.iter().any(|k| text.contains(k)) {
            let current = engine.lock().await.personas.active_persona().to_string();
            if current == *id {
                return Some("我现在就是这个形象呀，换换别的心情？可以让我换身装扮～".to_string());
            }
            match crate::apply_persona_switch(app, engine, id).await {
                Ok(()) => {
                    let names = ["古典佳人", "都市女性", "温柔形象", "活泼形象", "热情时尚"];
                    let idx = map.iter().position(|(i, _)| i == id).unwrap_or(0);
                    return Some(format!("好呀，换上{}～", names[idx]));
                }
                Err(e) => return Some(format!("切换失败：{e}")),
            }
        }
    }
    None
}

async fn try_screen_intent(
    app: &tauri::AppHandle,
    _engine: &Arc<Mutex<Engine>>,
    text: &str,
) -> Option<String> {
    let low = text.to_ascii_lowercase();

    // 0) 授权开关
    if low.contains("打开操作权限") || low.contains("允许操作") || low.contains("允许点击") || low.contains("开启操作") {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "gui_set", "enabled": true }));
        return Some("好的，操作权限已打开。现在可以语音指挥我点击、输入、按键啦。用完记得说「关闭操作权限」更安全哦～".to_string());
    }
    if low.contains("关闭操作权限") || low.contains("禁止操作") || low.contains("停止操作") {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "gui_set", "enabled": false }));
        return Some("好，操作权限已关闭。我已恢复「只看不动」模式。".to_string());
    }

    // 1) 截图（低危，直接执行；主进程 screen-cmd 处理）
    if ["截图", "截屏", "看屏幕", "看看屏幕", "看下屏幕", "看看现在", "看下现在"].iter().any(|k| text.contains(k)) {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "capture" }));
        return Some("好，我来看一眼屏幕。画面已经放到面板上了，你可以在上面直接点想让我点的地方，也可以语音说「点一下 300 500」「输入……」「按回车」。「打开操作权限」后我就能动手操作啦。".to_string());
    }

    // 2) 窗口激活：「切到 xx 窗口 / 激活 xx 窗口」
    for kw in ["切到", "切换到", "激活", "打开窗口"] {
        if text.contains(kw) {
            let rest = text.split(kw).nth(1).unwrap_or("").trim();
            let rest = rest.trim_end_matches(['的', '窗', '口', '。', '，']).trim();
            // v18.6：形象/桌宠/元气小小 相关词不当作窗口标题（留给 try_persona_intent）
            let persona_words = ["形象", "元气小小", "桌宠", "宠物", "佳人", "都市", "温柔", "活泼", "热情", "时尚", "古典"];
            if !rest.is_empty() && rest.chars().count() <= 40
                && !persona_words.iter().any(|k| rest.contains(k)) {
                let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "activate_window", "title": rest }));
                return Some(format!("好，正在切到「{rest}」窗口。"));
            }
        }
    }

    // 3) 滚动
    if text.contains("向下滚") || text.contains("往下滚") {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "scroll", "dy": 3 }));
        return Some("好，向下滚了三格。".to_string());
    }
    if text.contains("向上滚") || text.contains("往上滚") {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "scroll", "dy": -3 }));
        return Some("好，向上滚了三格。".to_string());
    }

    // 4) 坐标点击：「点一下 300 500」「点击 (100,200)」
    let coord_re = regex_lite_like_coord(low.as_str());
    if let Some((x, y)) = coord_re {
        let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "click", "x": x, "y": y }));
        return Some(format!("好，点了一下 ({x}, {y})。"));
    }

    // 5) 按键：「按回车 / 按ctrl+s / 按一下 esc」
    if text.contains("按") {
        let rest = text.split_once("按").map(|(_, r)| r).unwrap_or("");
        let rest = rest.trim_start_matches("一下").trim().trim_end_matches(['。', '，', '键']);
        if !rest.is_empty() && rest.chars().count() <= 20 {
            let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "key", "keys": rest }));
            return Some(format!("好，按了一下「{rest}」。"));
        }
    }

    // 6) 输入：「输入…… / 打一下…… / 键入……」
    for kw in ["输入", "打一下", "键入"] {
        if text.contains(kw) {
            let rest = text.split_once(kw).map(|(_, r)| r).unwrap_or("").trim();
            let rest = rest.trim_end_matches(['。', '，']);
            if !rest.is_empty() {
                let _ = app.emit("qibai://screen-cmd", serde_json::json!({ "cmd": "type", "text": rest }));
                return Some("好，输入完成。".to_string());
            }
        }
    }

    None
}

/// 极简坐标解析（不引入正则依赖）：识别「x,y」「(x,y)」「x y」。
fn regex_lite_like_coord(low: &str) -> Option<(u32, u32)> {
    // 去掉括号与中文标点，找两个连续数字组
    let cleaned = low.replace('（', "(").replace('）', ")");
    for part in cleaned.split(|c: char| c == '(' || c == ')') {
        let nums: Vec<u32> = part
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<u32>().ok())
            .collect();
        if nums.len() >= 2 && nums[0] <= 20000 && nums[1] <= 20000 {
            return Some((nums[0], nums[1]));
        }
    }
    None
}

// ---------------------------------------------------------------- B 线：Pipecat 双工下行

/// 执行一条语音 UI 指令（listener.rs 语音链路专用执行器；与 main.rs voice_ui_dispatch
/// 共用同一张匹配表 voice_ui_cmd，各自实现执行器）。
/// 返回最终要朗读/落 assistant 库的回复文本（技能类会把结果拼进 reply）。
pub(crate) async fn run_voice_ui_cmd(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    m: UiCmdMatch,
) -> String {
    match m.cmd {
        UiCmd::ConsoleOpen => {
            let _ = crate::ensure_console(app);
            m.reply
        }
        UiCmd::ConsoleClose => {
            let _ = crate::close_console(app.clone());
            m.reply
        }
        UiCmd::UiNav(tab) => {
            let _ = app.emit("qibai://ui-nav", serde_json::json!({ "tab": tab }));
            m.reply
        }
        UiCmd::GuardTrigger => {
            let start = {
                let mut e = engine.lock().await;
                e.guard_manual_trigger()
            };
            match start {
                None => "守护当前不可用（未启用或冷却中），稍后再试哦～".to_string(),
                Some(start) => {
                    let _ = crate::start_guard_performance(app, engine, start).await;
                    m.reply
                }
            }
        }
        UiCmd::SetPersona(id) => match crate::apply_persona_switch(app, engine, &id).await {
            Ok(()) => m.reply,
            Err(e) => format!("切换形象失败：{e}"),
        },
        UiCmd::Lock => {
            let pid = engine.lock().await.personas.active_persona().to_string();
            let ok = engine
                .lock()
                .await
                .desk_set_locked(&pid, true)
                .map_err(|e| e.to_string());
            match ok {
                Ok(()) => {
                    crate::broadcast_desk_changed(app, engine).await;
                    m.reply
                }
                Err(e) => format!("锁定失败：{e}"),
            }
        }
        UiCmd::Unlock => {
            let pid = engine.lock().await.personas.active_persona().to_string();
            let ok = engine
                .lock()
                .await
                .desk_set_locked(&pid, false)
                .map_err(|e| e.to_string());
            match ok {
                Ok(()) => {
                    crate::broadcast_desk_changed(app, engine).await;
                    m.reply
                }
                Err(e) => format!("解锁失败：{e}"),
            }
        }
        UiCmd::MuteOn => {
            let _ = app.emit("qibai://mute-toggle", serde_json::json!({ "on": true }));
            m.reply
        }
        UiCmd::MuteOff => {
            let _ = app.emit("qibai://mute-toggle", serde_json::json!({ "on": false }));
            m.reply
        }
        UiCmd::MinimalOn => {
            let _ = app.emit("qibai://ui-minimal", serde_json::json!({ "on": true }));
            m.reply
        }
        UiCmd::MinimalOff => {
            let _ = app.emit("qibai://ui-minimal", serde_json::json!({ "on": false }));
            m.reply
        }
        UiCmd::PetMode(mode) => match crate::apply_pet_mode(app, &mode) {
            Ok(()) => m.reply,
            Err(e) => format!("切换形态失败：{e}"),
        },
        UiCmd::HandsFreeOn | UiCmd::HandsFreeOff => {
            // v23：免提双工开关（与 set_wake_enabled 命令等价：写 settings + 启停 listener）。
            let enabled = matches!(m.cmd, UiCmd::HandsFreeOn);
            {
                let mut e = engine.lock().await;
                e.settings.voice.wake_enabled = enabled;
                if let Err(err) = e.settings.save() {
                    return format!("免提开关保存失败：{err:?}");
                }
            }
            let state = app.state::<crate::AppState>();
            if enabled {
                match crate::listener::start_listener(app.clone(), engine.clone(), state.listen.clone()) {
                    Ok(()) => m.reply,
                    Err(e) => format!("免提启动失败：{e}"),
                }
            } else {
                crate::listener::stop_listener(&state.listen);
                let _ = app.emit("qibai://wake-state", serde_json::json!({ "enabled": false }));
                m.reply
            }
        }
        UiCmd::Remind(kind) => {
            // v23：设置/取消提醒（sit/water/eye 或 off=全部取消）。
            let mut e = engine.lock().await;
            match kind.as_str() {
                "off" => {
                    e.reminder_cancel(&qibai_core::reminder::ReminderKind::Sit);
                    e.reminder_cancel(&qibai_core::reminder::ReminderKind::Water);
                    e.reminder_cancel(&qibai_core::reminder::ReminderKind::Eye);
                    m.reply
                }
                k => match qibai_core::reminder::ReminderKind::parse(k) {
                    Some(rk) => {
                        let mins = e.reminder_schedule_once(&rk, None);
                        let _ = app.emit(
                            "qibai://reminder-set",
                            serde_json::json!({ "kind": k, "minutes": mins }),
                        );
                        m.reply
                    }
                    None => "不懂这个提醒类型哦，试试『提醒我喝水』～".to_string(),
                },
            }
        }
        UiCmd::Express(expr) => {
            // v23：显式表情/动作演出——广播到所有桌宠窗，前端 setEmotion/playAction。
            let _ = app.emit("qibai://pet-perform", serde_json::json!({ "expr": expr }));
            m.reply
        }
        UiCmd::PetWindow(mode) => {
            // v23：窗口位置移动（left/right/up 贴边缘，reset 回贴边小窗位置）。
            let labels: Vec<String> = app
                .webview_windows()
                .keys()
                .filter(|l| l.starts_with("pet-"))
                .cloned()
                .collect();
            if labels.is_empty() {
                return "桌宠窗口还没准备好哦～".to_string();
            }
            let monitor = app
                .get_webview_window(&labels[0])
                .and_then(|w| w.current_monitor().ok().flatten())
                .or_else(|| app.primary_monitor().ok().flatten());
            let Some(monitor) = monitor else {
                return "窗口移动失败：找不到显示器".to_string();
            };
            let wa = monitor.work_area();
            let (wa_x, wa_y) = (wa.position.x as f64, wa.position.y as f64);
            let (wa_w, wa_h) = (wa.size.width as f64, wa.size.height as f64);
            for label in &labels {
                let Some(w) = app.get_webview_window(label) else { continue };
                let cur = w.outer_position().ok().map(|p| (p.x as f64, p.y as f64));
                let size = w.outer_size().ok().map(|s| (s.width as f64, s.height as f64));
                let (cx, cy) = cur.unwrap_or((wa_x, wa_y));
                let (ww, wh) = size.unwrap_or((220.0, 310.0));
                let (nx, ny) = match mode.as_str() {
                    "left" => (wa_x, cy),
                    "right" => (wa_x + wa_w - ww, cy),
                    "up" => (cx, wa_y),
                    _ => (wa_x + wa_w - ww, wa_y + wa_h - wh),
                };
                let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                    nx.max(wa_x) as i32,
                    ny.max(wa_y) as i32,
                )));
            }
            m.reply
        }
        UiCmd::RememberOn | UiCmd::RememberOff => {
            // v24 UI 开关：对话记忆（settings.llm.remember_chat）。
            let on = matches!(m.cmd, UiCmd::RememberOn);
            let mut e = engine.lock().await;
            e.settings.llm.remember_chat = on;
            if let Err(err) = e.settings.save() {
                return format!("记忆开关保存失败：{err:?}");
            }
            m.reply
        }
        UiCmd::SnapOn | UiCmd::SnapOff => {
            // v24 UI 开关：贴边吸附（settings.ui.snap_to_edge）。
            let on = matches!(m.cmd, UiCmd::SnapOn);
            let mut e = engine.lock().await;
            e.settings.ui.snap_to_edge = on;
            if let Err(err) = e.settings.save() {
                return format!("贴边开关保存失败：{err:?}");
            }
            m.reply
        }
        UiCmd::HidePet | UiCmd::ShowPet => {
            // v24 UI 开关：窗口隐藏/显示（广播事件，前端各窗隐藏/显示自己）。
            let evt = if matches!(m.cmd, UiCmd::HidePet) {
                "qibai://pet-hide"
            } else {
                "qibai://pet-show"
            };
            let _ = app.emit(evt, serde_json::json!({}));
            m.reply
        }
        UiCmd::WebSearchOn | UiCmd::WebSearchOff => {
            // v24 UI 开关：联网搜索技能启停（set_skill_enabled 等价路径）。
            let on = matches!(m.cmd, UiCmd::WebSearchOn);
            let mut e = engine.lock().await;
            if let Err(err) = e.set_skill_enabled(qibai_core::skills::SKILL_WEB_SEARCH, on) {
                return format!("联网搜索开关失败：{err:?}");
            }
            m.reply
        }
        UiCmd::Search(q) => {
            let outcome = engine
                .lock()
                .await
                .skills
                .invoke(qibai_core::skills::SKILL_WEB_SEARCH, &serde_json::json!({ "query": q }))
                .await;
            skill_reply(m.reply, outcome)
        }
        UiCmd::FileOp {
            action,
            path,
            dest,
            content,
        } => {
            // 语音写代码：先让 LLM 生成代码本体，再 file_ops write 落盘（需批准）。
            let reply = m.reply.clone();
            if action == "write" {
                let spec = content.unwrap_or_default();
                if spec.trim().is_empty() {
                    return format!("{reply} 想写个什么程序？告诉我要实现什么～");
                }
                let code = match engine.lock().await.generate_code(&spec).await {
                    Ok(c) => c,
                    Err(e) => return format!("{reply} 代码生成失败，稍后再试：{e}"),
                };
                let outcome = engine
                    .lock()
                    .await
                    .skills
                    .invoke(
                        qibai_core::skills::SKILL_FILE_OPS,
                        &serde_json::json!({ "action": "write", "path": path, "content": code }),
                    )
                    .await;
                pending_reply(m.reply, outcome)
            } else if action == "edit" {
                let mode = dest.unwrap_or_default();
                let edit_content = content.unwrap_or_default();
                let code = if mode == "append" {
                    // append：语音给的文本即追加内容。
                    edit_content
                } else {
                    // replace：先读原文件，拼修改要求让 LLM 重写全文。
                    match std::fs::read_to_string(&path) {
                        Ok(original) => {
                            let spec = format!("原文件内容：\n{original}\n\n修改要求：{edit_content}");
                            match engine.lock().await.generate_code(&spec).await {
                                Ok(c) => c,
                                Err(e) => return format!("{reply} 代码生成失败：{e}"),
                            }
                        }
                        Err(e) => return format!("{reply} 读不到 {path}：{e}，先写好文件再改吧～"),
                    }
                };
                let outcome = engine
                    .lock()
                    .await
                    .skills
                    .invoke(
                        qibai_core::skills::SKILL_FILE_OPS,
                        &serde_json::json!({ "action": "edit", "path": path, "mode": mode, "content": code }),
                    )
                    .await;
                pending_reply(m.reply, outcome)
            } else {
                let mut args = serde_json::json!({ "action": action, "path": path });
                if let Some(d) = dest {
                    if !d.is_empty() {
                        args["dest"] = serde_json::json!(d);
                    }
                }
                if let Some(c) = content {
                    if !c.is_empty() {
                        args["content"] = serde_json::json!(c);
                    }
                }
                let outcome = engine
                    .lock()
                    .await
                    .skills
                    .invoke(qibai_core::skills::SKILL_FILE_OPS, &args)
                    .await;
                skill_reply(m.reply, outcome)
            }
        }
        UiCmd::CreateProject {
            name,
            parent_dir,
            lang,
            goal,
        } => {
            // 建工程：技能先出方案（PendingApproval=方案文本），批准后才真建。
            let outcome = engine
                .lock()
                .await
                .skills
                .invoke(
                    "create_project",
                    &serde_json::json!({ "name": name, "parent_dir": parent_dir, "lang": lang, "goal": goal }),
                )
                .await;
            match outcome {
                SkillOutcome::PendingApproval { request_id, description } => {
                    remember_pending(request_id);
                    let plan: String = description.chars().take(200).collect();
                    format!("{plan}\n方案如上，说『确认』就开工，不满意告诉我怎么调整～")
                }
                SkillOutcome::Ok(out) => {
                    format!("好的，项目已经搭好了～ {}", out.chars().take(200).collect::<String>())
                }
                SkillOutcome::Denied(d) => format!("创建被拒绝：{d}"),
            }
        }
        UiCmd::ResumeProject { name, parent_dir } => {
            // 续工程：name 空 = 最近任务；朗读上下文摘要。
            let outcome = engine
                .lock()
                .await
                .skills
                .invoke("resume_project", &serde_json::json!({ "name": name, "parent_dir": parent_dir }))
                .await;
            skill_reply(m.reply, outcome)
        }
        UiCmd::ApproveLast => {
            let Some(id) = take_pending() else {
                return "当前没有待确认的操作哦".to_string();
            };
            match engine.lock().await.approve_skill(&id, true).await {
                Ok(out) => format!("好，已经批准啦：{}", out.chars().take(200).collect::<String>()),
                Err(e) => format!("批准失败：{e}"),
            }
        }
        UiCmd::DenyLast => {
            let Some(id) = take_pending() else {
                return "当前没有待确认的操作哦".to_string();
            };
            match engine.lock().await.approve_skill(&id, false).await {
                Ok(_) => "好的，请告诉我希望怎么调整，我再出方案～".to_string(),
                Err(e) => format!("取消失败：{e}"),
            }
        }
        UiCmd::Launch(target) => {
            let outcome = engine
                .lock()
                .await
                .skills
                .invoke(qibai_core::skills::SKILL_LAUNCH_APP, &serde_json::json!({ "target": target }))
                .await;
            skill_reply(m.reply, outcome)
        }
        UiCmd::Exit => m.reply, // 仅追问确认；真退在 ExitConfirm。
        UiCmd::ExitConfirm => {
            app.exit(0);
            m.reply
        }
    }
}

/// 把长回复切成 ≤ max_chars 的若干段依次朗读（执行中汇报：建目录/写文件/完成分次播报）。
/// v45.7：不再硬截断砍尾——按标点切分，剩余文本循环续排，保证
/// `chunks.concat() == 原文`（允许空白归一）。切分优先级：
///   1. 按换行 / 句末标点（。！？；）切；
///   2. 单句仍超长 → 在句中标点（，、）切；
///   3. 仍无标点可切 → 按字数硬切，剩余继续排，不放弃、不加截断省略号。
fn chunked_reply(reply: &str, max_chars: usize) -> Vec<String> {
    if reply.is_empty() {
        return vec![reply.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    // 先按换行粗切（保留换行作为段间分隔）
    for raw_line in reply.split('\n') {
        let line = raw_line.trim_end();
        if line.is_empty() {
            // 空行：段内换行占位，避免吞掉空行
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(String::new());
            continue;
        }
        // 对该行按句末标点切分（保留标点在段尾）
        for sentence in split_by(line, &['。', '！', '？', '；']) {
            // 单句仍超长 → 按句中标点再切
            for sub in split_by(&sentence, &['，', '、']) {
                // 子段仍超长 → 按字数硬切
                for piece in hard_split(&sub, max_chars) {
                    let pc = piece.chars().count();
                    if !cur.is_empty() && cur.chars().count() + pc + 1 > max_chars {
                        out.push(std::mem::take(&mut cur));
                    }
                    if !cur.is_empty() {
                        cur.push('\n');
                    }
                    cur.push_str(&piece);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(reply.to_string());
    }
    out
}

/// 在 chars 切分点处切分字符串，切分字符保留在前段末尾。
fn split_by(s: &str, delims: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        cur.push(ch);
        if delims.contains(&ch) {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(s.to_string());
    }
    out
}

/// 按字数硬切，无标点时兜底，保证不丢字。
fn hard_split(s: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return vec![s.to_string()];
    }
    chars
        .chunks(max_chars)
        .map(|c| c.iter().collect())
        .collect()
}

/// 最近一条待批准请求 id（语音确认词「确认/开工」收尾用）。
static LAST_PENDING_REQUEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn remember_pending(id: String) {
    *LAST_PENDING_REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
}

fn take_pending() -> Option<String> {
    LAST_PENDING_REQUEST.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// 技能执行结果 → 朗读文案：成功把结果摘要拼进 reply；待审批记录 id 并引导说「确认」。
fn pending_reply(base: String, outcome: SkillOutcome) -> String {
    match outcome {
        SkillOutcome::Ok(out) => skill_reply(base, SkillOutcome::Ok(out)),
        SkillOutcome::PendingApproval { request_id, .. } => {
            remember_pending(request_id);
            format!("{base} 这个操作需要确认，请说『确认』或到控制台授权～")
        }
        SkillOutcome::Denied(d) => format!("{base} 没能执行：{d}"),
    }
}

/// 技能执行结果 → 朗读文案：成功把结果摘要拼进 reply；待审批/被拒引导去控制台。
fn skill_reply(base: String, outcome: SkillOutcome) -> String {
    match outcome {
        SkillOutcome::Ok(mut out) => {
            if out.trim().is_empty() {
                base
            } else {
                // 结果摘要截断，防止朗读过长。
                let chars: String = out.chars().take(200).collect();
                out = chars;
                format!("{base} 结果：{out}")
            }
        }
        SkillOutcome::PendingApproval { .. } => {
            format!("{base} 这个操作需要授权，请在控制台点一下确认哦～")
        }
        SkillOutcome::Denied(d) => format!("{base} 没能执行：{d}"),
    }
}

/// Pipecat 下行事件消费循环：转写/回复文本 → UI 事件；audio delta → 全局播放；
/// barge-in → 停当前播放；连接断开 → 退出循环。
///
/// v20.1 落库修复：Transcript → append_chat(user)；ResponseTextDelta 累积、
/// ResponseDone → append_chat(assistant)；Transcript 先过 voice_ui_cmd 匹配表，
/// 命中则本地执行 UI 指令 + 朗读 reply + 落 assistant 消息，并把本轮 Python LLM
/// 回复标记为「不入库」（UI 指令即回复，避免双气泡）。
async fn pipecat_event_loop(
    client: Arc<tokio::sync::Mutex<PipecatClient>>,
    app: tauri::AppHandle,
    engine: Arc<Mutex<Engine>>,
) {
    eprintln!("[pipecat] event loop started");
    // 本轮 assistant 流式文本缓冲（ResponseCreated 清空）。
    let mut reply_buf = String::new();
    // true = 本轮已被 UI 指令消费，Python LLM 的文本回复不再落库。
    let mut suppress_python_reply = false;
    let mut alive = true;
    while alive {
        let evs = {
            let mut c = client.lock().await;
            c.drain_events()
        };
        for ev in evs {
            match ev {
                ServerEvent::SessionReady => eprintln!("[pipecat] session ready"),
                ServerEvent::ResponseCreated => {
                    eprintln!("[pipecat] turn created");
                    reply_buf.clear();
                }
                ServerEvent::BargeIn => {
                    eprintln!("[pipecat] barge-in (speech_started)");
                    qibai_core::tts::stop_playback();
                    let _ = app.emit(
                        "tts-interrupted",
                        serde_json::json!({ "reason": "pipecat-barge-in" }),
                    );
                    if let Ok(c) = client.try_lock() {
                        c.notify_barge_in();
                    }
                }
                ServerEvent::SpeechStopped => {}
                ServerEvent::Transcript(t) => {
                    eprintln!("[pipecat] transcript: {t}");
                    let _ = app
                        .emit("qibai://pipecat-transcript", serde_json::json!({ "text": t }));
                    // 修复根因①：语音转写此前不落 conversation，控制台看不到语音消息。
                    {
                        let mut e = engine.lock().await;
                        if let Err(err) = e.append_chat("user", &t) {
                            eprintln!("[pipecat] append_chat(user) 失败: {err}");
                        }
                    }
                    // 语音 UI 指令拦截（先精确后宽泛；未命中→走 Python LLM 对话链路）。
                    if let Some(m) = match_ui_command(&t) {
                        eprintln!("[pipecat] ui-cmd matched: {:?}", m.cmd);
                        let reply = run_voice_ui_cmd(&app, &engine, m).await;
                        // 砍 Python 侧可能正在播的音，避免指令回复与 LLM 语音叠音。
                        qibai_core::tts::stop_playback();
                        // 执行中汇报：长回复按换行分 2-3 段依次朗读，节奏自然。
                        for chunk in chunked_reply(&reply, 150) {
                            speak_and_emit_pet_say(&app, &engine, &chunk, 4000, None).await;
                            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                        }
                        {
                            let mut e = engine.lock().await;
                            if let Err(err) = e.append_chat("assistant", &reply) {
                                eprintln!("[pipecat] append_chat(assistant) 失败: {err}");
                            }
                        }
                        suppress_python_reply = true;
                    }
                }
                ServerEvent::ResponseTextDelta(d) => {
                    let _ = app
                        .emit("qibai://pipecat-reply", serde_json::json!({ "delta": d }));
                    reply_buf.push_str(&d);
                }
                ServerEvent::ResponseAudioDelta(b64) => {
                    use base64::Engine as _;
                    match base64::engine::general_purpose::STANDARD.decode(&b64) {
                        Ok(bytes) => match play_pcm16_mono_16k(&bytes) {
                            Ok(ms) => eprintln!("[pipecat] audio down {ms}ms ({} bytes)", bytes.len()),
                            Err(e) => eprintln!("[pipecat] audio down play failed: {e}"),
                        },
                        Err(e) => eprintln!("[pipecat] audio down base64 decode failed: {e}"),
                    }
                }
                ServerEvent::ResponseDone => {
                    eprintln!("[pipecat] turn done");
                    if suppress_python_reply {
                        // UI 指令回合：Python LLM 的兜底文本丢弃，不落库。
                        suppress_python_reply = false;
                        reply_buf.clear();
                    } else if !reply_buf.trim().is_empty() {
                        // 修复根因②：assistant 流式回复累积到一轮结束才落库一次。
                        let text = reply_buf.trim().to_string();
                        reply_buf.clear();
                        let mut e = engine.lock().await;
                        if let Err(err) = e.append_chat("assistant", &text) {
                            eprintln!("[pipecat] append_chat(assistant) 失败: {err}");
                        }
                    } else {
                        reply_buf.clear();
                    }
                }
                ServerEvent::Error(m) => {
                    eprintln!("[pipecat] error: {m}");
                    if m.contains("断开") {
                        alive = false;
                    }
                }
                ServerEvent::Unknown(t) => eprintln!("[pipecat] unknown event: {t}"),
            }
        }
        if !alive {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!("[pipecat] event loop exit (ws disconnected)");
}

/// 裸 PCM16 单声道 16k 字节 → 包 WAV 头 → 复用 qibai-core 全局 rodio 播放队列。
fn play_pcm16_mono_16k(pcm: &[u8]) -> Result<u64, String> {
    let data_len = pcm.len();
    let mut wav = Vec::with_capacity(44 + data_len);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36u32 + data_len as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&(16_000u32 * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    wav.extend_from_slice(pcm);
    qibai_core::tts::play_audio_bytes(&wav, "pipecat")
}

#[cfg(test)]
mod tests_chunked_reply {
    use super::chunked_reply;

    fn strip_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn long_text_no_newline_splits_and_reassembles() {
        // 600 字无换行、无标点（极端情况）→ 硬切 ≥2 段，拼接还原
        let raw: String = "啊".repeat(600);
        let chunks = chunked_reply(&raw, 150);
        assert!(chunks.len() >= 2, "600 字应切成 ≥2 段，实际 {} 段", chunks.len());
        let joined = strip_ws(&chunks.join(""));
        let original = strip_ws(&raw);
        assert_eq!(joined, original, "拼接后应还原原文（去空白后相等）");
        for c in &chunks {
            assert!(!c.contains("……"), "段尾不得加截断省略号: {:?}", c);
        }
    }

    #[test]
    fn short_text_returns_single_chunk_untouched() {
        let raw = "你好，我是小小～";
        let chunks = chunked_reply(raw, 150);
        assert_eq!(chunks.len(), 1, "短文本应 1 段原样返回");
        assert_eq!(chunks[0], raw);
    }

    #[test]
    fn multi_line_text_preserves_all_chars_in_order() {
        let raw = "第一行内容。\n第二行，带标点。\n\n第三行结尾！";
        let chunks = chunked_reply(raw, 150);
        let joined = strip_ws(&chunks.join(""));
        let original = strip_ws(raw);
        assert_eq!(joined, original, "多行文本每行不丢字、顺序不变");
    }

    #[test]
    fn splits_on_sentence_punctuation_naturally() {
        // 多句、每句 < 150 字 → 自然按句切，不硬切
        let raw = "第一句。第二句！第三句？第四句；";
        let chunks = chunked_reply(raw, 150);
        // 4 句都 < 150，应该合成 1-2 段（合并后 ≤150）
        let joined: String = chunks.join("");
        assert_eq!(strip_ws(&joined), strip_ws(raw));
        for c in &chunks {
            assert!(c.chars().count() <= 150, "段长 {} 超过 150", c.chars().count());
        }
    }
}
