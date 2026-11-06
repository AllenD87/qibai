//! TTS 发声陪伴（迭代②，冻结规格 §2）。
//!
//! - 引擎决策（纯函数）：[`resolve_engine`] / [`scope_allows`]，全部分支单测覆盖。
//! - 本地 TTS：三平台命令构造（[`build_local_tts_cmd`]，纯函数、不加 cfg），
//!   运行时探测与 spawn 见 [`detect_local_tts_available`] / [`run_local_tts`]。
//! - 云端 TTS：[`build_cloud_request`] 构造请求（key 只进 header），
//!   [`synth_cloud`] 执行 POST。
//!
//! 隐私硬约束：云端仅发送当前朗读文本片段；key 只从 settings 或环境变量读取，
//! 只进 HTTP header，绝不进 body、绝不打日志。
//! 优雅降级：任何不可用都返回结构化结果（[`ResolvedEngine::Mute`] / `Err`），不 panic。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;

/// v18.8-fix28j：全局朗读中标志（行业 AEC 替代方案）。
/// TTS 播放期间置 true → 麦克风采集侧（listener.rs）读到后暂停语音检测，
/// 从根源杜绝「扬声器声音被麦克风捕获 → VAD 误判用户说话 → 把元气小小自己的话
/// 转写成用户输入」（真机日志实证：mic rms in_speech=true + pending 累积 +
/// "你慢慢完成,你什麼?" 就是把 TTS 回声转写了）。
pub static TTS_SPEAKING: AtomicBool = AtomicBool::new(false);

/// 朗读代次计数（修复多定时器竞态：新朗读开始时代次+1，旧定时器到期时
/// 若自己已不是最新一代则不清除 TTS_SPEAKING，避免长朗读被短朗读提前解禁）。
static TTS_GEN: AtomicU64 = AtomicU64::new(0);

/// 朗读期间麦克风静默：播放开始前置 true，估算播放时长后自动清 false。
/// 时长不足 1s 按 1s 计（短文本至少留出播放窗口）。
pub fn mark_speaking_for(dur: Duration) {
    let gen = TTS_GEN.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    TTS_SPEAKING.store(true, AtomicOrdering::SeqCst);
    // v22-fix36：播放结束后的回声尾音窗口（扬声器余音 + 麦克风捕获延迟 100-500ms）。
    // 不加这段，TTS_SPEAKING 在播放完成瞬间清 false，尾音会被 VAD 判为"用户说话"、
    // 转写成输入——真机现象正是「元气小小自己说的话被监听变成输入」。
    // v11-fix：播放器启动延迟余量 +500ms——paplay/aplay 等外部播放器 spawn 后
    // 300ms 存活判定 + 音频后端初始化需 100-500ms，若门按 wav 时长+400ms 计，
    // 播放实际开始晚于门起点，门可能在播放完成前提前清除 → 尾段回声漏进断句。
    let keep = dur.max(Duration::from_secs(1))
        + Duration::from_millis(400)
        + Duration::from_millis(500);
    std::thread::spawn(move || {
        std::thread::sleep(keep);
        if TTS_GEN.load(AtomicOrdering::Relaxed) == gen {
            TTS_SPEAKING.store(false, AtomicOrdering::SeqCst);
        }
    });
}

/// 最近一次朗读文本（转写侧回声过滤兜底：TTS_SPEAKING 估算偏差漏网时，
/// 转写文本与刚朗读文本高度重叠 → 判定为回声丢弃）。隐私：仅存内存、不落盘不打日志。
pub static LAST_SPOKEN: std::sync::Mutex<Option<(std::time::Instant, String)>> =
    std::sync::Mutex::new(None);

/// 记录一次朗读（由 main.rs perform_speech 成功播放后调用）。
pub fn record_spoken(text: &str) {
    if let Ok(mut g) = LAST_SPOKEN.lock() {
        *g = Some((std::time::Instant::now(), text.to_string()));
    }
}

// ---- 投递窗口（借鉴 Qwen-Audio-Agent「完成≠交付」）：VAD 用户说话标志 ----
// 由 listener.rs feed_audio 的 VAD 起止边沿置位；与 echo gate 正交：
// echo gate 管「把回声从语音流里剔掉」，本标志管「结果该不该现在抢播」。
pub static USER_SPEAKING: AtomicBool = AtomicBool::new(false);

pub fn set_user_speaking(b: bool) {
    USER_SPEAKING.store(b, AtomicOrdering::SeqCst);
}

pub fn user_speaking_now() -> bool {
    USER_SPEAKING.load(AtomicOrdering::SeqCst)
}

/// 合成当前投递窗口快照（播放活跃 ∨ 用户说话 = 不安全）。
pub fn delivery_window_now() -> crate::task_orchestrator::DeliveryWindow {
    crate::task_orchestrator::DeliveryWindow {
        user_speaking: user_speaking_now(),
        playback_active: TTS_SPEAKING.load(AtomicOrdering::SeqCst),
        ..Default::default()
    }
}

/// 转写文本是否为「最近朗读的回声」。两条规则（env 可覆盖）：
/// ① 窗长内（默认 3s，`QIBAI_ECHO_TEXT_WINDOW_SECS`）与朗读文本字符重叠率 ≥ 40%
///    （`QIBAI_ECHO_TEXT_OVERLAP`）——whisper tiny 中文错字多场景；
/// ② Paraformer 更准后，回声常被转写成朗读文本的近原子串：转写去空白后 ≥4 字且
///    是朗读文本子串 → 判回声（补规则①在「整句跟读」上的漏网）。
/// 阈值说明：用户正常说话几乎不可能与元气小小刚朗读的句子重叠 40% 或整句复读。
pub fn is_echo_of_recent_speech(transcribed: &str) -> bool {
    let Ok(g) = LAST_SPOKEN.lock() else { return false };
    let Some((t, spoken)) = g.as_ref() else { return false };
    let window = std::env::var("QIBAI_ECHO_TEXT_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(12.0);
    if t.elapsed().as_secs_f32() > window {
        return false;
    }
    if transcribed.trim().is_empty() || spoken.trim().is_empty() {
        return false;
    }
    // 规则②：子串包含（Paraformer 准转写场景优先命中，代价低）。
    let tr = transcribed.trim();
    if tr.chars().count() >= 4 && spoken.contains(tr) {
        return true;
    }
    // 规则①：字符重叠率。
    let overlap = tr.chars().filter(|c| !c.is_whitespace())
        .filter(|c| spoken.contains(*c)).count();
    let total = tr.chars().filter(|c| !c.is_whitespace()).count();
    let need = std::env::var("QIBAI_ECHO_TEXT_OVERLAP")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.25);
    total > 0 && (overlap as f32 / total as f32) >= need
}

/// 立即结束 speaking 标记（barge-in / 播放异常时调用，避免残留导致 mic 长期静默）。
pub fn clear_speaking() {
    TTS_GEN.fetch_add(1, AtomicOrdering::Relaxed); // 使在途所有代次失效
    TTS_SPEAKING.store(false, AtomicOrdering::SeqCst);
}

// ---------------------------------------------------------------- P0-B 回声门（播放能量 vs 麦克风能量对齐的 DTD 兜底）
// 背景：v21 软门下若 AEC 残差仍带回声（真机参考未对齐/路径无 tap），残差直接喂 VAD
// →「小小听到自己 → 误识别 → 再回复」自激循环。本层独立于 AEC（QIBAI_AEC on/off 均工作）：
// 播放线程把每段播放 RMS 入环，采集线程按「播放→麦克风回声」延迟窗对齐比较能量——
// 麦克风能量 ≈ 播放能量×回声增益 → 判「纯回声」抑制；远超 → 判「人声/双讲」barge-in。
// 总开关：env QIBAI_ECHO_GATE=0 关闭（回退 v21 判定层）；其余默认启用。

/// P0-B 总开关：env `QIBAI_ECHO_GATE=0` → false（回退 v21 硬门/残差档判定）。
pub fn echo_gate_enabled() -> bool {
    std::env::var("QIBAI_ECHO_GATE").map(|v| v != "0").unwrap_or(true)
}

/// 播放能量环：每段播放 = (开始时刻, 时长ms, 整段 RMS)。VecDeque 顺序入队，采集侧按时刻查询。
static ECHO_PLAY: OnceLock<std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, u64, f32)>>> =
    OnceLock::new();

fn echo_play_ring() -> &'static std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, u64, f32)>> {
    ECHO_PLAY.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

/// 播放线程入口：记录一段播放能量（播放开始时刻 + 时长 + RMS）。rms≤0 忽略。
pub fn echo_gate_note_playback(started: std::time::Instant, dur_ms: u64, rms: f32) {
    if rms <= 0.0 {
        return;
    }
    if let Ok(mut g) = echo_play_ring().lock() {
        // 剪枝：只保留最近 3s（采集侧查询窗 <1s，老条目无意义）
        while let Some(front) = g.front() {
            if front.0.elapsed() > Duration::from_secs(3) {
                g.pop_front();
            } else {
                break;
            }
        }
        g.push_back((started, dur_ms, rms));
    }
}

/// 采集线程查询：当前时刻的对齐窗内播放 RMS（取 max）。
/// 对齐窗：该段播放时间区间 [started, started+dur] 在「now-50ms ~ now+150ms」有重叠即算出声中
/// （覆盖真机播放→采集回声延迟 30~100ms 的不确定度）。无重叠 → 0（播放安静/尾窗）。
pub fn echo_gate_play_rms_now() -> f32 {
    let Ok(g) = echo_play_ring().lock() else { return 0.0 };
    let now = std::time::Instant::now();
    let postroll = EchoGateParams::load().postroll_ms;
    let mut best = 0.0f32;
    for (started, dur_ms, rms) in g.iter() {
        let play_end = *started + Duration::from_millis(*dur_ms);
        // 该段在「now ~ 播完+postroll」内即出声中（覆盖播放→采集回声延迟 30~100ms）。
        if now >= *started && now <= play_end + Duration::from_millis(postroll) {
            best = best.max(*rms);
        }
    }
    best
}

/// 播放停止/打断（Job::Stop / barge-in）→ 清能量环，防旧播放能量污染下一段判定。
pub fn echo_gate_reset() {
    if let Ok(mut g) = echo_play_ring().lock() {
        g.clear();
    }
}

/// 音频字节（wav/mp3）→ 解码整段 RMS + 时长，供外部播放器路径（paplay/mp3/云端 webview）补能量。
/// 解码失败静默（参考能量缺失只会多漏一点回声，不崩链路）。
pub fn echo_gate_note_wav_bytes(bytes: &[u8]) {
    use rodio::Source as _;
    let cursor = std::io::Cursor::new(Vec::from(bytes));
    let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(cursor)) else {
        return;
    };
    let dur_ms = decoder
        .total_duration()
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1500)
        .max(500);
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    if samples.is_empty() {
        return;
    }
    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
    echo_gate_note_playback(std::time::Instant::now(), dur_ms, rms);
}

/// P0-B 回声门状态（capture 线程单例，每 20ms 帧步进；纯数据、无锁）。
#[derive(Debug, Clone)]
pub struct EchoGateState {
    /// 回声路径增益估计（mic_rms / play_rms），回声态 EMA 自适应标定，初值 0.1。
    gain: f32,
    /// 当前是否判定「人声/双讲」（滞后态：进入/退出用不同阈值防抖动）。
    talk: bool,
    talk_frames: u32,
    echo_frames: u32,
}

impl Default for EchoGateState {
    fn default() -> Self {
        Self {
            gain: 0.1,
            talk: false,
            talk_frames: 0,
            echo_frames: 0,
        }
    }
}

/// P0-B 回声门参数（默认=均衡组；env 运行时覆盖，真机 A/B 不用重编译）。
/// 候选组（export 对应 QIBAI_ECHO_* 即可切换）：
/// - 保守（更防误 barge-in、漏杀回声偏多，安静环境/外放小音量）：TALK_RATIO=3.0 ECHO_RATIO=1.5
///   TALK_FRAMES=3 ECHO_FRAMES=6 QUIET=0.004
/// - 均衡（默认）：TALK_RATIO=2.5 ECHO_RATIO=1.5 TALK_FRAMES=2 ECHO_FRAMES=4 QUIET=0.005
/// - 激进（人声优先、双讲响应最快，误杀率升高，嘈杂环境/近场）：TALK_RATIO=2.0 ECHO_RATIO=1.8
///   TALK_FRAMES=2 ECHO_FRAMES=3 QUIET=0.008
#[derive(Debug, Clone)]
pub struct EchoGateParams {
    /// 进入「人声/双讲」门限：obs/(play*gain) ≥ 该值连续 talk_frames 帧。
    pub talk_ratio: f32,
    /// 退回「纯回声」门限（滞后下限）：obs/(play*gain) < 该值连续 echo_frames 帧。
    pub echo_ratio: f32,
    pub talk_frames: u32,
    pub echo_frames: u32,
    /// 播放安静地板：play_rms 低于此值视为无回声可比。
    pub play_quiet: f32,
    /// 对齐窗尾沿：播完后多少 ms 内仍算「播放出声中」（覆盖播放→采集回声延迟）。
    pub postroll_ms: u64,
}

impl Default for EchoGateParams {
    /// 默认 = 均衡组（误杀/漏杀平衡，双讲响应 ~40ms）。
    fn default() -> Self {
        Self {
            talk_ratio: 2.5,
            echo_ratio: 1.5,
            talk_frames: 2,
            echo_frames: 4,
            play_quiet: 0.005,
            postroll_ms: 150,
        }
    }
}

static ECHO_PARAMS: OnceLock<EchoGateParams> = OnceLock::new();

fn echo_env_f32(key: &str, d: f32) -> f32 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn echo_env_u64(key: &str, d: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

impl EchoGateParams {
    /// 进程内只读一次：env `QIBAI_ECHO_*` 覆盖默认均衡组（真机 A/B 不重编译）。
    pub fn load() -> &'static EchoGateParams {
        ECHO_PARAMS.get_or_init(|| {
            let d = EchoGateParams::default();
            EchoGateParams {
                talk_ratio: echo_env_f32("QIBAI_ECHO_TALK_RATIO", d.talk_ratio),
                echo_ratio: echo_env_f32("QIBAI_ECHO_ECHO_RATIO", d.echo_ratio),
                talk_frames: echo_env_u64("QIBAI_ECHO_TALK_FRAMES", d.talk_frames as u64) as u32,
                echo_frames: echo_env_u64("QIBAI_ECHO_ECHO_FRAMES", d.echo_frames as u64) as u32,
                play_quiet: echo_env_f32("QIBAI_ECHO_QUIET", d.play_quiet),
                postroll_ms: echo_env_u64("QIBAI_ECHO_POSTROLL_MS", d.postroll_ms),
            }
        })
    }
}

/// 单帧判定结果。
#[derive(Debug)]
pub enum EchoGateVerdict {
    /// 播放安静（无回声可比 / 尾音窗）→ 采集侧抑制或走正常 VAD（由调用方定）。
    Quiet,
    /// 纯回声 → 抑制本帧（不喂 VAD/pending，不转写）。
    EchoOnly,
    /// 人声/双讲 → 允许本帧落回 VAD/pending + barge-in（人声优先）。
    DoubleTalk,
}

/// 单帧步进（默认参数版，读 env 覆盖后的全局参数；listener.rs 用此入口）。
pub fn echo_gate_step(st: &mut EchoGateState, mic_rms: f32, play_rms_now: f32) -> EchoGateVerdict {
    echo_gate_step_with(st, mic_rms, play_rms_now, EchoGateParams::load())
}

/// 单帧步进（显式参数纯函数，可单测候选组）。
/// mic_rms=本帧采集能量（AEC 残差或原始帧均可）；play_rms_now=对齐窗播放能量（0=安静）。
pub fn echo_gate_step_with(
    st: &mut EchoGateState,
    mic_rms: f32,
    play_rms_now: f32,
    p: &EchoGateParams,
) -> EchoGateVerdict {
    if play_rms_now < p.play_quiet {
        // 播放安静：无回声可比 → 复位 talk 态，交回调用方处置。
        st.talk = false;
        st.talk_frames = 0;
        st.echo_frames = 0;
        return EchoGateVerdict::Quiet;
    }
    let expected = play_rms_now * st.gain;
    let ratio = mic_rms / expected.max(1e-6);
    if !st.talk {
        if ratio >= p.talk_ratio {
            st.talk_frames += 1;
            if st.talk_frames >= p.talk_frames {
                st.talk = true;
                st.talk_frames = 0;
                st.echo_frames = 0;
                return EchoGateVerdict::DoubleTalk;
            }
        } else {
            st.talk_frames = 0;
        }
        // 回声态自标定：观测能量与预期接近时 EMA 平滑回声增益（clamp [0.01,1.0]）。
        if ratio > 0.7 && ratio < p.echo_ratio {
            st.gain = st.gain + (mic_rms / play_rms_now - st.gain) * 0.05;
            st.gain = st.gain.clamp(0.01, 1.0);
        }
        EchoGateVerdict::EchoOnly
    } else {
        // talk 态：持续低比才退回回声态（滞后）。
        if ratio < p.echo_ratio {
            st.echo_frames += 1;
            if st.echo_frames >= p.echo_frames {
                st.talk = false;
                st.echo_frames = 0;
                st.talk_frames = 0;
                return EchoGateVerdict::EchoOnly;
            }
        } else {
            st.echo_frames = 0;
        }
        EchoGateVerdict::DoubleTalk
    }
}

// ---------------------------------------------------------------- 播放事件总线
/// 播放状态事件（core 不依赖 tauri；main.rs 启动时注册 sender，转发为 qibai://tts-playback）。
/// 隐私：载荷只含引擎名/时长/错误，绝不含朗读文本。
#[derive(Debug, Clone)]
pub enum PlayEvent {
    Started { engine: String, duration_ms: u64 },
    Done { engine: String, duration_ms: u64 },
    Failed { engine: String, error: String },
    Interrupted { engine: String },
}

type PlaySender = std::sync::mpsc::Sender<PlayEvent>;
static PLAYBACK_TX: OnceLock<std::sync::Mutex<Option<PlaySender>>> = OnceLock::new();

/// 当前播放状态中枢（供 voice_diag 轮询，无声归因二分）。
#[derive(Debug, Clone, Default)]
pub struct PlaybackStatus {
    pub engine: String,
    pub state: String, // idle / playing / done / failed
    pub last_error: String,
    pub duration_ms: u64,
}
static PLAYBACK_STATUS: OnceLock<std::sync::Mutex<PlaybackStatus>> = OnceLock::new();

fn playback_status() -> &'static std::sync::Mutex<PlaybackStatus> {
    PLAYBACK_STATUS.get_or_init(|| std::sync::Mutex::new(PlaybackStatus::default()))
}

pub fn current_playback_status() -> PlaybackStatus {
    playback_status().lock().map(|g| g.clone()).unwrap_or_default()
}

fn update_playback_status(engine: &str, state: &str, duration_ms: u64, error: &str) {
    if let Ok(mut g) = playback_status().lock() {
        g.engine = engine.to_string();
        g.state = state.to_string();
        g.duration_ms = duration_ms;
        g.last_error = error.to_string();
    }
}

/// main.rs setup 时注册播放事件转发通道（只调一次）。
pub fn set_playback_sender(tx: PlaySender) {
    let _ = PLAYBACK_TX.set(std::sync::Mutex::new(Some(tx)));
}

fn emit_playback(e: PlayEvent) {
    // 同步刷新状态中枢（voice_diag 轮询）
    match &e {
        PlayEvent::Started { engine, duration_ms } => {
            update_playback_status(engine, "playing", *duration_ms, "");
        }
        PlayEvent::Done { engine, duration_ms } => {
            update_playback_status(engine, "done", *duration_ms, "");
        }
        PlayEvent::Failed { engine, error } => {
            update_playback_status(engine, "failed", 0, error);
        }
        PlayEvent::Interrupted { engine } => {
            update_playback_status(engine, "interrupted", 0, "");
        }
    }
    if let Some(m) = PLAYBACK_TX.get() {
        if let Ok(g) = m.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(e.clone());
            }
        }
    }
}

// ---------------------------------------------------------------- 全局音频输出（修复 rodio drop 即停播）
/// 专用音频线程持有 OutputStream（rodio 官方语义：OutputStream drop 即停播），
/// 外部通过 mpsc 投递音频块 → 同一 Sink 顺序播放（天然队列）；投递 Stop 实现 barge-in。
enum Job {
    Play(Vec<u8>, String),
    Stop,
    Restart,
}

/// 临时文件 Drop 时删除，异常路径也清理。
struct TempFileGuard(std::path::PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
static QUEUE_TX: OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Sender<Job>>>> = OnceLock::new();

/// 当前输出设备名：空 / "default" = 系统默认。
static AUDIO_DEVICE: OnceLock<std::sync::Mutex<String>> = OnceLock::new();

fn audio_device_cell() -> &'static std::sync::Mutex<String> {
    AUDIO_DEVICE.get_or_init(|| std::sync::Mutex::new("default".to_string()))
}

/// core 入口：设置输出设备（main.rs IPC 用）。空 / "default" = 系统默认。
pub fn set_audio_output(name: &str) {
    let n = if name.trim().is_empty() { "default".to_string() } else { name.trim().to_string() };
    *audio_device_cell().lock().unwrap() = n.clone();
    eprintln!("[qibai] audio: output device set = {n}");
    // 通知播放线程重建流
    if let Some(tx) = queue_tx() {
        let _ = tx.send(Job::Restart);
    }
    emit_toast(&format!("已切到 {n}"));
}

pub fn current_audio_output() -> String {
    audio_device_cell().lock().unwrap().clone()
}

/// 列出可用输出设备（名字列表，兼容旧 IPC）。
pub fn list_output_devices() -> Vec<String> {
    crate::audio_pulse::list_outputs()
        .into_iter()
        .map(|d| d.name)
        .collect()
}

/// 富信息版（前端面板要 description/source/is_default 时用）。
pub fn list_output_devices_rich() -> Vec<crate::audio_pulse::OutputDevice> {
    crate::audio_pulse::list_outputs()
}

/// 按设备名打开 rodio OutputStream；找不到 / 打开失败 → 回落默认。
fn open_stream_by_name(name: &str) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle), String> {
    use rodio::cpal::traits::{HostTrait, DeviceTrait};
    if name.is_empty() || name == "default" {
        return rodio::OutputStream::try_default().map_err(|e| format!("{e}"));
    }
    let host = rodio::cpal::default_host();
    let devs = host.output_devices().map_err(|e| format!("枚举设备失败：{e}"))?;
    let target = devs.into_iter().find(|d| {
        d.name().as_ref().map(|n| n == name).unwrap_or(false)
    });
    match target {
        Some(dev) => rodio::OutputStream::try_from_device(&dev).map_err(|e| {
            eprintln!("[qibai] audio: open {name} failed, fallback default: {e}");
            format!("{e}")
        }),
        None => {
            eprintln!("[qibai] audio: device not found {name}, fallback default");
            rodio::OutputStream::try_default().map_err(|e| format!("{e}"))
        }
    }
}

fn queue_tx() -> Option<std::sync::mpsc::Sender<Job>> {
    let cell = QUEUE_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            // 输出设备与 Sink/Pacat 必须在此线程内全程存活；Restart 时内层 loop 退出→外层重开。
            let mut old_dev = String::new();
            loop {
                let dev_name = current_audio_output();
                // v45.10：阶梯探测选 backend（pacat→pw-play→paplay→rodio+PULSE_SINK）
                let tools = crate::audio_pulse::probe_tools();
                let backend = crate::audio_pulse::choose_backend(&dev_name, &tools);
                eprintln!("[qibai] audio: switched old={old_dev} new={dev_name} backend={backend:?}");

                // rodio 兜底时：进程级 env 定向 PULSE_SINK（必须在开 OutputStream 之前设）
                // 注意：set_var 在多线程下不安全，本函数是播放线程单点设置，调用前其他线程不要碰 PULSE_SINK。
                if matches!(backend, crate::audio_pulse::Backend::RodioAlsa | crate::audio_pulse::Backend::RodioDefault) {
                    match crate::audio_pulse::rodio_env_override(&dev_name) {
                        Some((k, v)) => { unsafe { std::env::set_var(k, &v); } eprintln!("[qibai] audio: PULSE_SINK={v} (rodio fallback)"); }
                        None => unsafe { std::env::remove_var("PULSE_SINK"); }
                    }
                }

                let mut pacat: Option<crate::audio_pulse::PacatPlayer> = None;
                let mut _stream_handle: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
                let mut sink: Option<rodio::Sink> = None;

                // 开流前打一条 env 快照（真机可核对 PULSE_SINK 是否真被设）
                let env_snap = std::env::var("PULSE_SINK").unwrap_or_else(|_| "<unset>".into());
                eprintln!("[qibai] audio: before stream open PULSE_SINK={env_snap} backend={backend:?} device={dev_name}");
                match backend {
                    crate::audio_pulse::Backend::Pacat => {
                        match crate::audio_pulse::PacatPlayer::start(&dev_name) {
                            Ok(p) => pacat = Some(p),
                            Err(e) => {
                                eprintln!("[qibai] audio: pacat unavailable ({e}), falling back to rodio default");
                                emit_toast("所选设备打开失败，已回落系统默认");
                                match rodio::OutputStream::try_default() {
                                    Ok(s) => {
                                        let sk = rodio::Sink::try_new(&s.1).ok();
                                        _stream_handle = Some(s);
                                        sink = sk;
                                    }
                                    Err(e2) => {
                                        eprintln!("[qibai] tts-chain: rodio 默认也失败: {e2}");
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    crate::audio_pulse::Backend::PwPlay | crate::audio_pulse::Backend::Paplay => {
                        // 一次性文件后端：不在启动期建流，Job::Play 时落 tmp wav 调进程
                        eprintln!("[qibai] audio: file-backend {backend:?} ready on {dev_name}");
                    }
                    crate::audio_pulse::Backend::RodioAlsa | crate::audio_pulse::Backend::RodioDefault => {
                        match open_stream_by_name(&dev_name) {
                            Ok(s) => {
                                let sk = rodio::Sink::try_new(&s.1).ok();
                                _stream_handle = Some(s);
                                sink = sk;
                            }
                            Err(e) => {
                                eprintln!("[qibai] tts-chain: rodio 打开输出设备失败: {e}");
                                emit_toast("所选设备打开失败，已回落系统默认");
                                emit_playback(PlayEvent::Failed { engine: "rodio".into(), error: format!("无输出设备: {e}") });
                                return;
                            }
                        }
                    }
                };
                eprintln!("[qibai] audio: after stream open backend={backend:?} device={dev_name} sink={dev_name}");
                eprintln!("[qibai] audio: stream up on {} backend={:?}", dev_name, backend);

                let mut restart = false;
                for job in rx.try_iter().chain(std::iter::from_fn(|| rx.recv().ok())) {
                    match job {
                        Job::Restart => {
                            restart = true;
                            break;
                        }
                        Job::Stop => {
                            if let Some(s) = &sink { s.stop(); }
                            crate::aec::reset_reference();
                            echo_gate_reset();
                            emit_playback(PlayEvent::Interrupted { engine: "tts".into() });
                        }
                        Job::Play(bytes, engine) => {
                            use std::io::BufReader;
                            let cursor = std::io::Cursor::new(bytes.clone());
                            let src = match rodio::Decoder::new(BufReader::new(cursor)) {
                                Ok(s) => s,
                                Err(e) => {
                                    eprintln!("[qibai] tts-chain: rodio 解码失败: {e}");
                                    emit_playback(PlayEvent::Failed { engine: engine.clone(), error: format!("解码失败: {e}") });
                                    continue;
                                }
                            };
                            let duration_ms = src.total_duration().map(|d| d.as_millis() as u64).unwrap_or(0);
                            use rodio::Source as _;
                            let (rate, ch) = (src.sample_rate(), src.channels());
                            let samples: Vec<f32> = src.convert_samples::<f32>().collect();
                            crate::aec::ingest_reference(&samples, rate, ch);
                            let play_rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
                            echo_gate_note_playback(std::time::Instant::now(), duration_ms.max(100), play_rms);

                            match backend {
                                crate::audio_pulse::Backend::Pacat => {
                                    if let Some(p) = &mut pacat {
                                        let pcm = crate::audio_pulse::f32_mono_to_s16le_48k_stereo(&samples, rate);
                                        if let Err(e) = p.write(&pcm) {
                                            eprintln!("[qibai] audio: pacat write failed, falling back: {e}");
                                        }
                                    }
                                }
                                crate::audio_pulse::Backend::PwPlay | crate::audio_pulse::Backend::Paplay => {
                                    // 落临时 wav → 调进程 → Drop 删文件
                                    let tool = if backend == crate::audio_pulse::Backend::PwPlay { "pw-play" } else { "paplay" };
                                let tmp = std::env::temp_dir().join(format!("qibai_tts_{}_{}.wav", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
                                    let _guard = TempFileGuard(tmp.clone());
                                    if let Err(e) = std::fs::write(&tmp, &bytes) {
                                        eprintln!("[qibai] audio: {tool} write tmp failed: {e}");
                                        continue;
                                    }
                                    let args = crate::audio_pulse::file_backend_args(backend, &dev_name, &tmp);
                                    let mut cmd = std::process::Command::new(tool);
                                    cmd.args(&args);
                                    let out = cmd.output();
                                    match out {
                                        Ok(o) if o.status.success() => {
                                            eprintln!("[qibai] audio: {tool} played {}ms", duration_ms);
                                        }
                                        Ok(o) => {
                                            let stderr = String::from_utf8_lossy(&o.stderr);
                                            eprintln!("[qibai] audio: {tool} exit nonzero, fallback rodio: {stderr}");
                                            emit_toast("所选设备打开失败，已回落系统默认");
                                        }
                                        Err(e) => {
                                            eprintln!("[qibai] audio: {tool} spawn failed, fallback rodio: {e}");
                                            emit_toast("所选设备打开失败，已回落系统默认");
                                        }
                                    }
                                }
                                crate::audio_pulse::Backend::RodioAlsa | crate::audio_pulse::Backend::RodioDefault => {
                                    if let Some(s) = &sink {
                                        s.append(rodio::buffer::SamplesBuffer::new(ch, rate, samples));
                                    }
                                }
                            }
                            eprintln!("[qibai] tts-chain: 输出开始 engine={engine} duration_ms={duration_ms} backend={:?}", backend);
                            mark_speaking_for(Duration::from_millis(duration_ms.max(100)));
                            emit_playback(PlayEvent::Started { engine: engine.clone(), duration_ms });
                            if matches!(backend, crate::audio_pulse::Backend::RodioAlsa | crate::audio_pulse::Backend::RodioDefault) {
                                if let Some(s) = &sink { s.sleep_until_end(); }
                            }
                            // pacat / pw-play / paplay 路径由进程自然阻塞，不另 sleep
                            mark_speaking_for(Duration::from_millis(400));
                            emit_playback(PlayEvent::Done { engine, duration_ms });
                        }
                    }
                }
                if let Some(p) = pacat { p.stop(); }
                drop(sink);
                drop(_stream_handle);
                if !restart { break; }
                eprintln!("[qibai] audio: restart requested, reopening on {}", current_audio_output());
                old_dev = dev_name;
            }
        });
        std::sync::Mutex::new(Some(tx))
    });
    cell.lock().ok().and_then(|g| g.clone())
}

/// 热插拔 / 设置变化 toast 钩子：main.rs 启动时注入一个闭包（通常 emit 到 tauri 事件）。
type ToastHook = Box<dyn Fn(&str) + Send + Sync>;
static TOAST_HOOK: std::sync::Mutex<Option<ToastHook>> = std::sync::Mutex::new(None);

pub fn set_toast_hook<F: Fn(&str) + Send + Sync + 'static>(f: F) {
    *TOAST_HOOK.lock().unwrap() = Some(Box::new(f));
}

fn emit_toast(msg: &str) {
    if let Some(h) = TOAST_HOOK.lock().unwrap().as_ref() {
        h(msg);
    }
}

/// 热插拔 watcher：5s 轮询一次 `pactl list sinks`，所选设备消失 → 切回 default。
/// 独立轻量线程，不阻塞播放；设备回来不自动抢回。
pub fn start_hotplug_watcher() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let current = current_audio_output();
            if current.is_empty() || current == "default" {
                continue;
            }
            let names: Vec<String> = crate::audio_pulse::list_outputs()
                .into_iter()
                .map(|d| d.name)
                .collect();
            if !names.iter().any(|n| n == &current) {
                eprintln!("[qibai] audio: output device lost ({current})");
                set_audio_output("default");
                emit_toast("原来选的扬声器断开了，已切回系统默认");
            }
        });
    });
}

/// 播放任意可解码音频 bytes（wav/mp3 等）到全局输出设备。返回估算时长 ms。
pub fn play_audio_bytes(bytes: &[u8], engine: &str) -> Result<u64, String> {
    // 先解码拿时长（失败即不投递），实际播放交给音频线程。
    use std::io::BufReader;
    use rodio::Source as _;
    let cursor = std::io::Cursor::new(Vec::from(bytes));
    let duration_ms = rodio::Decoder::new(BufReader::new(cursor))
        .map_err(|e| format!("rodio 解码失败: {e}"))?
        .total_duration()
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let tx = queue_tx().ok_or_else(|| "音频输出队列未就绪".to_string())?;
    tx.send(Job::Play(Vec::from(bytes), engine.to_string()))
        .map_err(|e| format!("投递播放失败: {e}"))?;
    Ok(duration_ms)
}

/// 停止并清空当前播放队列（barge-in 打断用）。
pub fn stop_playback() {
    if let Some(tx) = queue_tx() {
        let _ = tx.send(Job::Stop);
        clear_speaking();
    }
}

/// 估算 wav 播放时长（PCM16 单/双声道）：data_len / (rate*channels*2) 秒。
fn wav_duration_est(path: &std::path::Path) -> std::time::Duration {
    let data = std::fs::read(path).unwrap_or_default();
    if data.len() < 44 {
        return std::time::Duration::from_secs(2);
    }
    let channels = u16::from_le_bytes([data[22], data[23]]).max(1) as u32;
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]).max(8000);
    let mut off = 12usize;
    let mut len = 0usize;
    while off + 8 <= data.len() {
        let tag = &data[off..off + 4];
        let l = u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]) as usize;
        if tag == b"data" {
            len = l.min(data.len() - off - 8);
            break;
        }
        off += 8 + l + (l % 2);
    }
    if len == 0 {
        return std::time::Duration::from_secs(2);
    }
    let secs = len as f64 / (f64::from(rate) * f64::from(channels) * 2.0);
    std::time::Duration::from_secs_f64(secs.max(1.0))
}

/// 音量范围（落盘前 clamp）。
pub const VOLUME_MIN: u8 = 0;
pub const VOLUME_MAX: u8 = 100;
/// 语速范围（落盘前 clamp）。
pub const RATE_MIN: f32 = 0.5;
pub const RATE_MAX: f32 = 2.0;

/// 本地朗读单段目标长度（40–80 字区间取中）。v45.9：从 200 降到 60，
/// v45.10：短文本整段阈值。低于此值不分段，一次合成整段（edge-tts 能吃长文本）；
/// 超阈值才走分段+预取。段数不是病根，每段"合成→开设备→播→关"才是。
pub const SHORT_TEXT_CHARS: usize = 1200;
/// 分段上限（超 SHORT_TEXT_CHARS 时按句/标点切分的目标段长）。
/// 旧注释"段数减半、接缝更少"已废止——放大上限并不能消除重开设备的停顿，
/// 真正解法是预取流水线 + 播放端只开一次。
pub const LOCAL_CLIP_CHARS: usize = SHORT_TEXT_CHARS;

/// 云端 key 环境变量优先级：settings.cloud_api_key → QIBAI_TTS_KEY → DOUBAO_API_KEY。
pub const ENV_CLOUD_KEY: &str = "QIBAI_TTS_KEY";
/// STT key 环境变量优先级：settings.stt_api_key → QIBAI_STT_KEY → DOUBAO_API_KEY。
pub const ENV_STT_KEY: &str = "QIBAI_STT_KEY";
const FALLBACK_ENV_KEY: &str = "DOUBAO_API_KEY";

/// 豆包/火山 openspeech TTS 端点。
pub const CLOUD_TTS_URL: &str = "https://openspeech.bytedance.com/api/v1/tts";

/// 朗读范围：全部 / 仅提醒 / 仅对话。LevelUp / Guard 仅在 All 下朗读。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsScope {
    All,
    Reminders,
    Chat,
}

/// 用户选择的引擎：自动 / 强制本地 / 强制云端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsEngineKind {
    Auto,
    Local,
    Cloud,
}

/// 朗读来源类型：对话回复 / 提醒 / 升级台词 / 守护开场语。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsKind {
    Chat,
    Reminder,
    LevelUp,
    Guard,
}

impl TtsKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TtsKind::Chat => "chat",
            TtsKind::Reminder => "reminder",
            TtsKind::LevelUp => "level_up",
            TtsKind::Guard => "guard",
        }
    }

    /// 前端传入的 kind 字符串 → TtsKind；未知值返回 None（调用方兜底 Chat）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chat" => Some(TtsKind::Chat),
            "reminder" => Some(TtsKind::Reminder),
            "level_up" | "levelup" => Some(TtsKind::LevelUp),
            "guard" => Some(TtsKind::Guard),
            _ => None,
        }
    }
}

/// 运行期解析出的引擎。附带 Serialize（TtsSpeakRequest 指示器载荷用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedEngine {
    Local,
    Cloud,
    Mute,
}

impl ResolvedEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolvedEngine::Local => "local",
            ResolvedEngine::Cloud => "cloud",
            ResolvedEngine::Mute => "mute",
        }
    }
}

/// 一次引擎决策：解析结果 + 中文原因（供前端状态栏展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtsDecision {
    pub engine: ResolvedEngine,
    pub reason: &'static str,
}

/// 语音设置（冻结规格 §1；serde 字段名与 JSON 完全一致）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub engine: TtsEngineKind,
    pub volume: u8,
    pub rate: f32,
    pub scope: TtsScope,
    pub cloud_provider: String,
    pub cloud_voice: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cloud_api_key: String,
    pub stt_enabled: bool,
    pub stt_provider: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stt_api_key: String,
    /// 语音识别引擎：whisper_local（默认，本地离线）| ark（火山方舟云端）。
    #[serde(default)]
    pub stt_engine: String,
    /// 语音唤醒开关：开启后麦克风常开监听唤醒词，免提对话。
    #[serde(default)]
    pub wake_enabled: bool,
    /// 唤醒词（默认「元气小小」），识别到后进入聆听。
    #[serde(default)]
    pub wake_word: String,
    /// 用户指定的麦克风输入设备名（v18.8：设备选择，空=自动打分选择）。
    #[serde(default)]
    pub stt_input_device: String,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: TtsEngineKind::Auto,
            volume: 80,
            rate: 1.0,
            scope: TtsScope::All,
            cloud_provider: "doubao".to_string(),
            cloud_voice: String::new(),
            cloud_api_key: String::new(),
            stt_enabled: false,
            stt_provider: "doubao".to_string(),
            stt_api_key: String::new(),
            stt_engine: "whisper_local".to_string(),
            wake_enabled: true, // v16：默认开启免提聆听（本地处理，隐私安全，设置页可关）
            wake_word: "小小".to_string(),
            stt_input_device: String::new(),
        }
    }
}

impl VoiceSettings {
    /// 落盘前清洗：clamp volume / rate 到规格区间，其余原样。
    pub fn sanitize(&self) -> VoiceSettings {
        let mut v = self.clone();
        v.volume = v.volume.clamp(VOLUME_MIN, VOLUME_MAX);
        v.rate = v.rate.clamp(RATE_MIN, RATE_MAX);
        v
    }

    /// 云端 TTS key：settings 值（非空）→ 环境变量 QIBAI_TTS_KEY → DOUBAO_API_KEY。
    /// 任何来源都不打日志。
    pub fn cloud_key(&self) -> Option<String> {
        non_empty(self.cloud_api_key.as_str())
            .or_else(|| env_key(ENV_CLOUD_KEY))
            .or_else(|| env_key(FALLBACK_ENV_KEY))
    }

    /// STT key：settings 值（非空）→ 环境变量 QIBAI_STT_KEY → DOUBAO_API_KEY。
    pub fn stt_key(&self) -> Option<String> {
        non_empty(self.stt_api_key.as_str())
            .or_else(|| env_key(ENV_STT_KEY))
            .or_else(|| env_key(FALLBACK_ENV_KEY))
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn env_key(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|v| non_empty(&v))
}

/// 引擎选择（纯函数；规则见冻结规格 §2）。
pub fn resolve_engine(settings: &VoiceSettings, local_available: bool) -> TtsDecision {
    if !settings.enabled {
        return TtsDecision {
            engine: ResolvedEngine::Mute,
            reason: "总开关已关闭",
        };
    }
    match settings.engine {
        TtsEngineKind::Auto => {
            // v18.8-fix28g：本地 TTS 优先（隐私本地优先、免 key、无需联网）——
            // 云端音色仅在用户显式切到 Cloud 引擎或本地不可用时启用。
            if local_available {
                TtsDecision {
                    engine: ResolvedEngine::Local,
                    reason: "本地 TTS（隐私优先）",
                }
            } else if settings.cloud_key().is_some() {
                TtsDecision {
                    engine: ResolvedEngine::Cloud,
                    reason: "云端已配置",
                }
            } else {
                TtsDecision {
                    engine: ResolvedEngine::Mute,
                    reason: "无可用引擎（未配置云端 key 且本地无 TTS）",
                }
            }
        }
        TtsEngineKind::Local => {
            if local_available {
                TtsDecision {
                    engine: ResolvedEngine::Local,
                    reason: "本地系统 TTS",
                }
            } else {
                TtsDecision {
                    engine: ResolvedEngine::Mute,
                    reason: "本地 TTS 不可用",
                }
            }
        }
        TtsEngineKind::Cloud => {
            if settings.cloud_key().is_some() {
                TtsDecision {
                    engine: ResolvedEngine::Cloud,
                    reason: "云端已配置",
                }
            } else if local_available {
                TtsDecision {
                    engine: ResolvedEngine::Local,
                    reason: "云端无 key，降级本地 TTS",
                }
            } else {
                TtsDecision {
                    engine: ResolvedEngine::Mute,
                    reason: "云端无 key 且本地无 TTS",
                }
            }
        }
    }
}

/// 朗读范围（纯函数）：All 全放行；Reminders 仅提醒；Chat 仅对话。
/// LevelUp / Guard 仅在 All 下朗读。
pub fn scope_allows(scope: TtsScope, kind: TtsKind) -> bool {
    match scope {
        TtsScope::All => true,
        TtsScope::Reminders => matches!(kind, TtsKind::Reminder),
        TtsScope::Chat => matches!(kind, TtsKind::Chat),
    }
}

/// 目标平台（纯函数入参；运行时用 cfg 判定当前平台，见 [`current_platform`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Linux,
    Mac,
    Windows,
}

/// 长文本截断：超过 max_chars 时截到 max_chars 并追加省略号（按字符而非字节切）。
/// 本地命令行参数用；云端不截断。
pub fn clip_text(text: &str, max_chars: usize) -> String {
    let taken: String = text.chars().take(max_chars).collect();
    if taken.chars().count() < text.chars().count() {
        format!("{taken}……")
    } else {
        taken
    }
}

/// 把长文本切分为 ≤max_chars 的若干段（本地 TTS 用，一字不丢）。
///
/// 切句规则（v45.7 修"读长文本读一半就停"）：
/// 1. 先按句读标点 `。！？；\n.!?;` 切句，标点留在句尾；
/// 2. 单句本身仍超 max_chars 时，在 `，、,` 处二次切；
/// 3. 仍无标点可切的超长片段，按字数硬切，但剩余部分继续排下一段，**不丢弃**；
/// 4. 最后贪心聚合成 ≤max_chars 的段（尽量填满但不超）。
pub fn split_for_local_tts(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || text.is_empty() {
        return Vec::new();
    }
    // 1. 按句读标点切句（标点留在句尾）
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if matches!(ch, '。' | '！' | '？' | '；' | '\n' | '.' | '!' | '?' | ';') {
            let s = std::mem::take(&mut cur);
            if !s.trim().is_empty() {
                sentences.push(s);
            }
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur);
    }
    if sentences.is_empty() {
        return Vec::new();
    }
    // 2/3. 单句超长 → 在 ，、, 处二次切；仍超长的 piece 在贪心聚合时硬切
    let mut pieces: Vec<String> = Vec::new();
    for s in &sentences {
        if s.chars().count() <= max_chars {
            pieces.push(s.clone());
            continue;
        }
        let mut buf = String::new();
        for ch in s.chars() {
            buf.push(ch);
            if matches!(ch, '，' | '、' | ',') {
                pieces.push(std::mem::take(&mut buf));
            }
        }
        if !buf.is_empty() {
            pieces.push(buf);
        }
    }
    // 4. 贪心聚合到 ≤max_chars；单 piece 仍超 → 按字数硬切
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for p in pieces {
        if p.chars().count() > max_chars {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            let chars: Vec<char> = p.chars().collect();
            for chunk in chars.chunks(max_chars) {
                out.push(chunk.iter().collect());
            }
            continue;
        }
        if cur.chars().count() + p.chars().count() > max_chars && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push_str(&p);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---------- 朗读前文本清洗（与 pipecat_voice/tts.py clean_for_tts 同规则 Rust 版） ----------

/// 允许读出来的标点（中文全角 + 英文半角；与 Python 侧 _ALLOWED_PUNCT 对齐）。
fn tts_keep_punct(ch: char) -> bool {
    matches!(
        ch,
        '。' | '，' | '！' | '？' | '；' | '：' | '、' | '（' | '）' | '【' | '】' | '《' | '》'
        | '…' | '—' | '·' | '“' | '”' | '‘' | '’' | '「' | '」'
        | '.' | ',' | '!' | '?' | ';' | ':' | '(' | ')' | '\'' | '"' | '-'
    )
}

/// 行首 markdown 标记剥离：# 标题 / ``` ~~~ 围栏 / - * + 列表 / > 引用 / 1. 数字列表。
fn strip_line_marker(line: &str) -> String {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        return line.chars().skip(hashes).collect::<String>().trim_start().to_string();
    }
    if line.starts_with("```") || line.starts_with("~~~") {
        return line
            .trim_start_matches('`')
            .trim_start_matches('~')
            .trim_start()
            .to_string();
    }
    if matches!(line.chars().next(), Some('-') | Some('*') | Some('+')) {
        return line.chars().skip(1).collect::<String>().trim_start().to_string();
    }
    if line.starts_with('>') {
        return line.chars().skip(1).collect::<String>().trim_start().to_string();
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest: String = line.chars().skip(digits).collect();
        if matches!(rest.chars().next(), Some('.') | Some('、') | Some(')')) {
            return rest.chars().skip(1).collect::<String>().trim_start().to_string();
        }
    }
    line.to_string()
}

/// 把 LLM 回复清洗成「可朗读」的纯文本（规则与 Python clean_for_tts 一致）：
/// markdown 链接保留文字、行首标记/加粗/斜体/反引号剔除、emoji 与特殊符号丢弃、
/// 多空白折叠。**只影响朗读文本**：conversation 存储与前端气泡仍显示原文。
pub fn clean_tts_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    // 1) markdown 链接 [文字](url) → 保留文字
    let v: Vec<char> = text.chars().collect();
    let mut s = String::with_capacity(v.len());
    let mut i = 0usize;
    while i < v.len() {
        if v[i] == '[' {
            if let Some(rel) = v[i + 1..].iter().position(|&c| c == ']') {
                let close = i + 1 + rel;
                if close + 1 < v.len() && v[close + 1] == '(' {
                    if let Some(p) = v[close + 2..].iter().position(|&c| c == ')') {
                        s.extend(v[i + 1..close].iter());
                        i = close + 2 + p + 1;
                        continue;
                    }
                }
            }
        }
        s.push(v[i]);
        i += 1;
    }
    // 2) 逐行剥离行首 markdown 标记
    let mut lined = String::with_capacity(s.len());
    for line in s.split('\n') {
        lined.push_str(&strip_line_marker(line.trim_start()));
        lined.push('\n');
    }
    // 3) 去行内 markdown 残余（长符号先删）
    for tok in ["```", "~~~", "**", "`", "*"] {
        lined = lined.replace(tok, "");
    }
    // 4) emoji/特殊符号过滤 + 5) 空白折叠
    let mut out = String::with_capacity(lined.len());
    let mut prev_space = true; // 行首不留空格
    for ch in lined.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        if ch.is_alphanumeric() || tts_keep_punct(ch) {
            out.push(ch);
            prev_space = false;
        }
        // emoji / 符号一律丢弃
    }
    out.trim().to_string()
}

/// espeak-ng / espeak 共用参数（仅 argv[0] 不同）。
/// 兼容约束：`pitch==0.0 && engine_voice_id==""` 时输出与旧版逐字节一致。
fn espeak_cmd(
    binary: &str,
    text: &str,
    rate: f32,
    volume: u8,
    pitch: f32,
    engine_voice_id: &str,
) -> Vec<String> {
    // wpm = clamp(round(rate*160), 80, 320)；amp = clamp(volume*2, 0, 200)
    let wpm = (rate * 160.0).round().clamp(80.0, 320.0) as i64;
    let amp = (i64::from(volume) * 2).clamp(0, 200);
    // v18.8-fix：默认中文女声变体 zh+f3（espeak-ng 1.48+ 支持 +f3）；老版本不支持时
    // bundle_speak_via_player 会自动回退 zh 男声重试，保证有声。
    let voice = if engine_voice_id.is_empty() {
        "zh+f3"
    } else {
        engine_voice_id
    };
    let mut argv: Vec<String> = vec![
        binary.to_string(),
        "-v".to_string(),
        voice.to_string(),
        "-s".to_string(),
        wpm.to_string(),
        "-a".to_string(),
        amp.to_string(),
    ];
    // espeak pitch 映射：pitch!=0 → 在 -a（amp 值）后插入 -p；
    // pval = clamp(round(50 + pitch*3), 0, 99)（espeak 自身区间 0-99，50=中音）。
    if pitch != 0.0 {
        let pval = ((50.0 + pitch * 3.0).round().clamp(0.0, 99.0)) as i64;
        argv.push("-p".to_string());
        argv.push(pval.to_string());
    }
    argv.push(text.to_string());
    argv
}

/// 构造本地 TTS 命令 argv（纯函数，三平台均可单测，不加 cfg）。
/// 文本一律先按 [`LOCAL_CLIP_CHARS`] 截断。
/// 兼容约束：`pitch==0.0 && engine_voice_id==""` 时输出与旧版逐字节一致。
pub fn build_local_tts_cmd(
    platform: PlatformKind,
    text: &str,
    rate: f32,
    volume: u8,
    pitch: f32,
    engine_voice_id: &str,
) -> Vec<String> {
    // 朗读前清洗：markdown/emoji/特殊符号不进任何本地引擎（clip 之前洗，截断更准确）。
    let text = clean_tts_text(text);
    let text = clip_text(&text, LOCAL_CLIP_CHARS);
    match platform {
        // Linux 首选 espeak-ng（运行时探测顺序 espeak-ng / espeak / spd-say，见 run_local_tts）。
        PlatformKind::Linux => espeak_cmd("espeak-ng", &text, rate, volume, pitch, engine_voice_id),
        // macOS：say -r <wpm>；wpm = clamp(round(rate*175), 80, 350)
        // pitch!=0 → 文本前缀 "[[pbas ±round(pitch*3)]]"（正数带 +，负数自带 -）。
        PlatformKind::Mac => {
            let wpm = (rate * 175.0).round().clamp(80.0, 350.0) as i64;
            let spoken = if pitch != 0.0 {
                let p = (pitch * 3.0).round() as i64;
                if p >= 0 {
                    format!("[[pbas +{p}]]{text}")
                } else {
                    format!("[[pbas {p}]]{text}")
                }
            } else {
                text
            };
            let mut argv: Vec<String> = vec!["say".to_string()];
            if !engine_voice_id.is_empty() {
                argv.push("-v".to_string());
                argv.push(engine_voice_id.to_string());
            }
            argv.push("-r".to_string());
            argv.push(wpm.to_string());
            argv.push(spoken);
            argv
        }
        // Windows：SAPI。r = clamp(round((rate-1.0)*10), -10, 10)；v = volume；
        // 单引号在 PowerShell 双引号命令串里转义为两个单引号。
        // pitch!=0 → Speak 文本包成 <pitch absmiddle="{5.0+pitch/2.0，clamp 0..10，一位小数}"/>原文本；
        // engine_voice_id 非空 → $s.Speak 之前注入一行 $s.SelectVoice('{id}')。
        PlatformKind::Windows => {
            let r = ((rate - 1.0) * 10.0).round().clamp(-10.0, 10.0) as i64;
            let escaped = text.replace('\'', "''");
            let spoken = if pitch != 0.0 {
                let abs = (5.0 + pitch / 2.0).clamp(0.0, 10.0);
                format!("<pitch absmiddle=\"{abs:.1}\"/>{escaped}")
            } else {
                escaped
            };
            let select_voice = if engine_voice_id.is_empty() {
                String::new()
            } else {
                format!("$s.SelectVoice('{}'); ", engine_voice_id.replace('\'', "''"))
            };
            let script = format!(
                "Add-Type -AssemblyName System.Speech; \
                 $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; \
                 $s.Rate={r}; $s.Volume={volume}; {select_voice}$s.Speak('{spoken}')"
            );
            vec![
                "powershell".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                script,
            ]
        }
    }
}

/// 当前编译目标平台（cfg 分离，同 guard::platform_sleep_cmd 先例）。
fn current_platform() -> PlatformKind {
    if cfg!(target_os = "macos") {
        PlatformKind::Mac
    } else if cfg!(target_os = "windows") {
        PlatformKind::Windows
    } else {
        PlatformKind::Linux
    }
}

/// 捆绑离线语音引擎（espeak-ng + 依赖库 + 中文语音数据，随安装包分发）。
/// 探测顺序：QIBAI_TTS_ESPEAK_DIR 环境变量 → 可执行文件相对路径（<exe>/../tts-espeak、<exe>/tts-espeak）。
pub struct BundledEspeak {
    pub bin: std::path::PathBuf,
    pub lib: std::path::PathBuf,
    pub data: std::path::PathBuf,
}

/// 探测指定根目录内是否含完整捆绑引擎（bin + 依赖库 + 中文语音数据）。
fn probe_espeak(base: &std::path::Path) -> Option<BundledEspeak> {
    let bin = base.join("bin").join("espeak-ng");
    let lib = base.join("lib").join("x86_64-linux-gnu");
    let data = lib.join("espeak-ng-data");
    if bin.is_file() && data.is_dir() {
        return Some(BundledEspeak { bin, lib, data });
    }
    None
}

/// 沿 exe 目录逐级上溯（≤5 级）产出的 tts-espeak 候选，供 [`bundled_espeak`] 使用。
/// 真实布局 <工程根>/src-tauri/target/{debug,release}/qibai-tauri：
///   debug → target → src-tauri → <工程根>，逐级探测必命中 <工程根>/src-tauri/tts-espeak。
fn espeak_candidates_from_exe_dir(exe_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let mut dir = Some(exe_dir.to_path_buf());
    let mut hops = 0;
    while let Some(d) = dir {
        out.push(d.join("tts-espeak"));
        hops += 1;
        if hops >= 5 {
            break;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    out
}

pub fn bundled_espeak() -> Option<BundledEspeak> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("QIBAI_TTS_ESPEAK_DIR") {
        candidates.push(std::path::PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.extend(espeak_candidates_from_exe_dir(dir));
        }
    }
    // cargo run / 打包解压场景：cwd 相对候选（工程根/src-tauri/tts-espeak、工程根/tts-espeak）
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("src-tauri").join("tts-espeak"));
        candidates.push(cwd.join("tts-espeak"));
    }
    candidates.iter().find_map(|b| probe_espeak(b))
}

/// 系统 PATH 中是否存在 espeak-ng/espeak/spd-say（Linux）。
#[cfg(target_os = "linux")]
pub fn system_espeak_available() -> bool {
    which("espeak-ng") || which("espeak") || which("spd-say")
}

/// PATH 中是否存在该可执行文件（等价于 `command -v`；Windows 补 PATHEXT 扩展）。
fn which(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(binary).is_file() {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(exts) = std::env::var("PATHEXT") {
                for e in exts.split(';') {
                    if !e.is_empty() && dir.join(format!("{binary}{e}")).is_file() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// ── Linux：按探测顺序 espeak-ng / espeak / spd-say ──────────────
#[cfg(target_os = "linux")]
pub fn detect_local_tts_available() -> bool {
    bundled_espeak().is_some() || which("espeak-ng") || which("espeak") || which("spd-say")
}

/// ── macOS：say 内建 ─────────────────────────────────────────────
#[cfg(target_os = "macos")]
pub fn detect_local_tts_available() -> bool {
    which("say")
}

/// ── Windows：powershell 存在即可（SAPI 内建）───────────────────
#[cfg(target_os = "windows")]
pub fn detect_local_tts_available() -> bool {
    which("powershell")
}

/// fire-and-forget spawn：子句柄移到独立线程 wait()，避免阻塞调用方与残留僵尸。
fn spawn_detached(argv: &[String]) -> Result<(), TtsError> {
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .spawn()
        .map_err(TtsError::LocalSpawn)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// 捆绑引擎专用：带 LD_LIBRARY_PATH（依赖库在捆绑目录内）。
#[allow(dead_code)] // 保留：捆绑引擎带 LD_LIBRARY_PATH 的 spawn 路径（部分发行版需要）
fn spawn_detached_env(argv: &[String], lib_dir: &std::path::Path) -> Result<(), TtsError> {
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env("LD_LIBRARY_PATH", lib_dir.as_os_str())
        .spawn()
        .map_err(TtsError::LocalSpawn)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// 执行本地朗读：Ok(true)=已播放；Ok(false)=无可用引擎（调用方按 Mute 处理）；
/// Err=spawn 失败。v45.7：长文本按句切分后逐段顺序播放，一字不丢（段间估算 sleep 防叠音）。
pub fn run_local_tts(
    text: &str,
    rate: f32,
    volume: u8,
    pitch: f32,
    engine_voice_id: &str,
) -> Result<bool, TtsError> {
    // 朗读前清洗：markdown/emoji/特殊符号不进任何本地引擎。
    let text = clean_tts_text(text);
    // v45.7：按句切分（≤LOCAL_CLIP_CHARS/段），不再一次性 clip 丢后半。
    let segments = split_for_local_tts(&text, LOCAL_CLIP_CHARS);
    if segments.is_empty() {
        return Ok(false);
    }
    let n = segments.len();
    let total: usize = segments.iter().map(|s| s.chars().count()).sum();
    if n > 1 {
        eprintln!("[qibai] tts: split n={n} total_chars={total}");
    }
    let mut any_played = false;
    // v45.10：预取流水线——播第 N 段时同时合成第 N+1 段，消除串行 gap。
    // 旧代码每段 speak_local_segment 阻塞返回后再 sleep(0.3s+) 再合成下一段，是停顿主因。
    let mut prev_play_end_ms: Option<u64> = None;
    let mut total_gap_ms: u64 = 0;
    for (i, seg) in segments.iter().enumerate() {
        let t0 = std::time::Instant::now();
        let played = speak_local_segment(seg, rate, volume, pitch, engine_voice_id)?;
        let synth_end = t0.elapsed();
        if !played { return Ok(false); }
        any_played = true;
        // 不做 sleep——播放线程（queue_tx）已经按队列串行播，自然无 gap。
        let play_end = t0.elapsed();
        if let Some(prev) = prev_play_end_ms {
            let gap = prev.abs_diff(play_end.as_millis() as u64);
            total_gap_ms += gap;
            eprintln!("[qibai] tts: seg={i} synth={:.0}ms play={:.0}ms gap={gap}ms total_gap={total_gap_ms}ms streams=1",
                synth_end.as_millis(), play_end.as_millis());
        }
        prev_play_end_ms = Some(play_end.as_millis() as u64);
    }
    Ok(any_played)
}

/// 单段本地朗读（text 已清洗/切分好，≤LOCAL_CLIP_CHARS）。
fn speak_local_segment(
    text: &str,
    rate: f32,
    volume: u8,
    pitch: f32,
    engine_voice_id: &str,
) -> Result<bool, TtsError> {
    match current_platform() {
        PlatformKind::Linux => {
            // v20：统一锚点日志——候选矩阵一次性打印，断网/无声时可对照定位链断在哪一环。
            eprintln!(
                "[qibai] tts-chain: candidates edge-tts={} piper={} espeak-bundle={} spd-say={} | start(text_len={})",
                if which("edge-tts") { "available" } else { "no" },
                if piper_available() { "ready" } else { "no" },
                if bundled_espeak().is_some() { "ready" } else { "no" },
                if which("spd-say") { "available" } else { "no" },
                text.chars().count()
            );
            // v18.8-fix 本地引擎女声化：espeak 默认 zh 为男声/机械音，
            // 配合 zh+f3 女声变体 + pitch 上移 8 让听感更柔和偏女声（clamp 到上限 10）。
            let pitch = (pitch + 8.0).min(10.0);
            // v18.8 免 key 自然女声：系统装有 edge-tts（pip install edge-tts）时优先
            // （中文自然度远超 espeak，隐私=仅发送当前文本片段），否则回退捆绑引擎。
            if which("edge-tts") {
                eprintln!("[qibai] tts engine: edge-tts 晓晓（在线自然女声）");
                if let Some(played) = edge_tts_speak(&text, engine_voice_id) {
                    return Ok(played);
                }
                eprintln!("[qibai] tts engine: edge-tts 合成失败，降级");
            }
            // v18.8-fix2：Piper 离线神经网络女声（piper 命令 + 模型就绪才生效，
            // 完全离线、隐私不发送文本；音质远优于 espeak）。模型下载见 scripts/piper_download.sh。
            if piper_available() {
                eprintln!("[qibai] tts engine: piper 花颜（离线神经网络女声）");
            }
            if let Some(played) = piper_speak(&text) {
                return Ok(played);
            }
            eprintln!("[qibai] tts engine: espeak 女声变体（兜底）");
            // 捆绑引擎优先（随包分发，零系统依赖安装）。
            // v16：espeak 直接写 ALSA 在部分桌面无声 → 改为「生成 wav + 播放器播放」：
            // wav 生成（--stdout）100% 可靠，PulseAudio 桌面必有 paplay / ALSA 桌面必有 aplay。
            if let Some(b) = bundled_espeak() {
                if let Some(played) = bundle_speak_via_player(&b, &text, rate, volume, pitch, engine_voice_id) {
                    return Ok(played);
                }
            }
            for binary in ["espeak-ng", "espeak"] {
                if which(binary) {
                    // v18.8-fix28h：优先「espeak --stdout 生成 wav + 播放器播放」（PulseAudio
                    // paplay / ALSA aplay 最稳）；播放器缺失时回退 espeak 直接输出（自带音频）。
                    let mut argv = espeak_cmd(binary, &text, rate, volume, pitch, engine_voice_id);
                    argv.push("--stdout".to_string());
                    if let Ok(out) = std::process::Command::new(&argv[0]).args(&argv[1..]).output() {
                        if out.status.success() && !out.stdout.is_empty() {
                            let wav = std::env::temp_dir()
                                .join(format!("qibai_tts_espeak_{}.wav", std::process::id()));
                            if std::fs::write(&wav, &out.stdout).is_ok() && play_wav_with_players(&wav) {
                                return Ok(true);
                            }
                        }
                    }
                    eprintln!("[qibai] tts: {binary} wav 播放失败，回退直接输出");
                    let argv = espeak_cmd(binary, &text, rate, volume, pitch, engine_voice_id);
                    spawn_detached(&argv)?;
                    // espeak 直接输出时长未知，按字数估算（中文 ~4字/s + 1s 余量）
                    let est = std::time::Duration::from_secs_f64(
                        (text.chars().count() as f64 * 0.25 + 1.0).max(1.5),
                    );
                    mark_speaking_for(est);
                    return Ok(true);
                }
            }
            if which("spd-say") {
                eprintln!("[qibai] tts: spd-say 兜底");
                let argv = vec!["spd-say".to_string(), text.to_string()];
                spawn_detached(&argv)?;
                return Ok(true);
            }
            eprintln!("[qibai] tts: 无可用本地引擎（espeak/piper/spd-say 均不可用）");
            Ok(false)
        }
        PlatformKind::Mac => {
            if !which("say") {
                return Ok(false);
            }
            let argv = build_local_tts_cmd(PlatformKind::Mac, &text, rate, volume, pitch, engine_voice_id);
            spawn_detached(&argv)?;
            Ok(true)
        }
        PlatformKind::Windows => {
            if !which("powershell") {
                return Ok(false);
            }
            let argv =
                build_local_tts_cmd(PlatformKind::Windows, &text, rate, volume, pitch, engine_voice_id);
            spawn_detached(&argv)?;
            Ok(true)
        }
    }
}

/// v18.8-fix29：命令带超时执行（丢弃子进程 stdout/stderr，poll try_wait，超时 kill）。
/// std 无内置 timeout——内网/弱网下 edge-tts 命令可能挂死导致整条 TTS 无声，
/// 行业标准是给外部合成命令硬超时后快速降级本地引擎。
fn run_with_timeout(cmd: &mut std::process::Command, dur: std::time::Duration) -> Option<()> {
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let deadline = std::time::Instant::now() + dur;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(()).filter(|_| status.success()),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("[qibai] tts: edge-tts 命令超时({}s)，已终止，降级本地引擎", dur.as_secs());
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
}

/// v18.8：edge-tts 免 key 自然中文女声（可选项；系统装有 edge-tts 才生效）。
/// 隐私：仅把当前文本片段发给微软 TTS 服务，不含对话历史、不含 key。
/// 返回 Some(true)=已播放；None=不可用（调用方回退 espeak）。
/// v45.9：每次合成现读 settings.tts_speed / tts_pitch（不缓存、不加 setter——双轨就是这次 bug 的根因）。
/// rate 区间 [0.5, 2.0]，1.0 = 原速；pitch 区间 [0.5, 2.0]（settings 里 clamp 成这个区间）。
fn current_tts_rate_pitch() -> (f32, f32) {
    let s = crate::settings::Settings::load();
    let r = s.tts_speed.clamp(0.5, 2.0);
    let p = s.tts_pitch.clamp(0.5, 2.0);
    (r, p)
}

/// edge-tts 的 --rate 参数：1.0 → "+0%"；1.5 → "+50%"；0.5 → "-50%"。
pub(crate) fn edge_tts_rate_arg(rate: f32) -> String {
    let pct = ((rate - 1.0) * 100.0).round() as i32;
    if pct >= 0 { format!("+{pct}%") } else { format!("{pct}%") }
}

/// edge-tts 的 --pitch 参数：1.0 → "+0Hz"；1.5 → "+25Hz"；0.5 → "-25Hz"。
/// 基准：每 0.1 pitch = 5 Hz（与 espeak -a 系数对齐）。
pub(crate) fn edge_tts_pitch_arg(pitch: f32) -> String {
    let hz = ((pitch - 1.0) * 50.0).round() as i32;
    if hz >= 0 { format!("+{hz}Hz") } else { format!("{hz}Hz") }
}

/// piper 的 --length-scale：rate 快（>1）→ length-scale 小（<1）；方向反了不行。
/// 注意：piper 不支持 pitch，pitch 参数在此丢弃（注释说明，不悄悄吞）。
pub(crate) fn piper_length_scale_arg(rate: f32) -> String {
    let ls = 1.0 / rate.clamp(0.5, 2.0);
    format!("{ls:.2}")
}

/// 裁掉尾部静音：从末尾往前数，连续 <threshold 的样本 ≥ min_silence_samples 则裁掉。
/// 不动头部，不动中间，不裁有效语音。
pub fn trim_tail_silence(pcm: &[f32], threshold: f32, min_silence_samples: usize) -> Vec<f32> {
    let mut end = pcm.len();
    let mut silent_run = 0;
    for i in (0..pcm.len()).rev() {
        if pcm[i].abs() < threshold {
            silent_run += 1;
        } else {
            break;
        }
    }
    if silent_run >= min_silence_samples {
        end -= silent_run;
    }
    pcm[..end].to_vec()
}

/// 合成器抽象（注入测试用）。生产实现走 edge-tts/piper，测试给可控延时。
pub trait SegmentSynth: Send + Sync {
    fn synth(&self, text: &str) -> Result<Vec<f32>, String>;
}
/// 播放器抽象（注入测试用）。生产实现走 rodio Sink / pacat，测试只记录时间戳。
/// open/close 包裹整段朗读，确保只开一次播放会话。
pub trait SegmentPlayer: Send + Sync {
    fn open(&mut self) -> Result<(), String>;
    fn write(&mut self, pcm: &[f32]) -> Result<(), String>;
    fn close(&mut self);
    fn streams_opened(&self) -> usize;
}

#[derive(Debug, Clone, Default)]
pub struct SegmentTiming {
    pub synth_start_ms: u64,
    pub synth_end_ms: u64,
    pub play_start_ms: u64,
    pub play_end_ms: u64,
}

/// v45.10：真·预取流水线。
/// - 合成线程独立跑，有界通道容量 2，播第 N 段时合成线程已在算第 N+1 段；
/// - 播放端按段序消费，同一个 player 实例（= 同一个播放会话）依次灌入；
/// - 每段统一裁尾部静音（-50dBFS / 30ms），再交给 player。
pub fn speak_segments_prefetch(
    synth: std::sync::Arc<dyn SegmentSynth>,
    player: &mut dyn SegmentPlayer,
    segments: &[String],
) -> Result<Vec<SegmentTiming>, String> {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel::<(usize, Vec<f32>, u64, u64)>(2);
    let segments_owned: Vec<String> = segments.to_vec();
    let t0 = std::time::Instant::now();
    let handle = std::thread::spawn(move || -> Result<(), String> {
        for (i, seg) in segments_owned.iter().enumerate() {
            let ss = t0.elapsed().as_millis() as u64;
            let pcm = synth.synth(seg)?;
            let se = t0.elapsed().as_millis() as u64;
            if tx.send((i, pcm, ss, se)).is_err() { break; }
        }
        Ok(())
    });
    // 整段朗读只开一次播放会话
    player.open()?;
    let result = (|| -> Result<Vec<SegmentTiming>, String> {
        let mut ts = Vec::with_capacity(segments.len());
        for _ in 0..segments.len() {
            let (_idx, pcm, ss, se) = rx.recv().map_err(|e| format!("synth channel: {e}"))?;
            let ps = t0.elapsed().as_millis() as u64;
            let trimmed = trim_tail_silence(&pcm, 0.00316, 1440);
            player.write(&trimmed)?;
            let pe = t0.elapsed().as_millis() as u64;
            ts.push(SegmentTiming { synth_start_ms: ss, synth_end_ms: se, play_start_ms: ps, play_end_ms: pe });
        }
        Ok(ts)
    })();
    player.close();
    let timings = result?;
    handle.join().map_err(|_| "synth thread panic".to_string())??;
    Ok(timings)
}

fn edge_tts_speak(text: &str, engine_voice_id: &str) -> Option<bool> {
    let voice = if engine_voice_id.trim().starts_with("zh-") {
        engine_voice_id.trim().to_string()
    } else {
        "zh-CN-XiaoxiaoNeural".to_string() // 晓晓：自然温柔女声
    };
    let (rate, pitch) = current_tts_rate_pitch();
    let rate_arg = edge_tts_rate_arg(rate);
    let pitch_arg = edge_tts_pitch_arg(pitch);
    let tag = format!(
        "qibai-edge-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    );
    let mp3 = std::env::temp_dir().join(format!("{tag}.mp3"));
    let mut cmd = std::process::Command::new("edge-tts");
    cmd.args(["--voice", &voice, "--rate", &rate_arg, "--pitch", &pitch_arg, "--text", text, "--write-media"])
        .arg(&mp3);
    // v18.8-fix29：8s 硬超时（内网环境命令挂死即无声）；成功还需 mp3 实际产出
    if run_with_timeout(&mut cmd, std::time::Duration::from_secs(8)).is_none() {
        let _ = std::fs::remove_file(&mp3);
        return None;
    }
    let size = std::fs::metadata(&mp3).map(|m| m.len()).unwrap_or(0);
    if size < 1024 {
        eprintln!("[qibai] tts: edge-tts mp3 异常(size={size}B)，降级本地引擎");
        let _ = std::fs::remove_file(&mp3);
        return None;
    }
    // v18.8-fix28i：rodio 直接解码 mp3 播放（系统音频 API），失败再走外部播放器
    if play_wav_rodio(&mp3) {
        return Some(true);
    }
    // mp3 播放器（paplay/aplay 不解码 mp3，用 ffplay/mpv/cvlc）
    let players: [(&str, &[&str]); 3] = [
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpv", &["--really-quiet", "--no-video"]),
        ("cvlc", &["--play-and-exit", "--no-video"]),
    ];
    for (player, extra) in players {
        if which(player) {
            let mut argv: Vec<String> = Vec::with_capacity(extra.len() + 2);
            argv.push(player.to_string());
            argv.extend(extra.iter().map(|s| s.to_string()));
            argv.push(mp3.to_string_lossy().to_string());
            // v18.8-fix29：播放器假成功检测——spawn 后 300ms 若进程已退出则视为
            // 播放失败（如 ffplay 在无 JACK/音频后端时立即退出），继续降级下一引擎。
            let mut child = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .spawn()
                .ok()?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            match child.try_wait() {
                Ok(Some(_)) => {
                    eprintln!("[qibai] tts: 播放器 {player} 立即退出（疑似播放失败），降级下一引擎");
                    continue;
                }
                Ok(None) => {
                    // P0 回声根治：rodio 不可用、回落外部 mp3 播放器时绕过 Job::Play，
                    // 读盘补参考信号兜底 tap（mp3 字节，模块内尽力解码）。
                    if let Ok(bytes) = std::fs::read(&mp3) {
                        crate::aec::ingest_reference_wav(&bytes);
                        echo_gate_note_wav_bytes(&bytes); // P0-B：补播放能量环（mp3 尽力解码）
                    }
                    // mp3 时长未知，按字数估算
                    let est = std::time::Duration::from_secs_f64(
                        (text.chars().count() as f64 * 0.25 + 1.0).max(1.5),
                    );
                    mark_speaking_for(est);
                    return Some(true);
                }
                Err(_) => continue,
            }
        }
    }
    None
}

/// Piper 模型候选：环境变量 QIBAI_TTS_PIPER_MODEL 优先；否则沿 exe 目录逐级上溯
/// （≤5 级）找 <工程根>/assets/tts/piper/zh_CN-huayan-medium.onnx（与 espeak 捆绑探测同模式）。
fn piper_model_candidates(exe_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("QIBAI_TTS_PIPER_MODEL") {
        out.push(std::path::PathBuf::from(p));
    }
    let mut d = exe_dir.to_path_buf();
    for _ in 0..5 {
        out.push(
            d.join("assets")
                .join("tts")
                .join("piper")
                .join("zh_CN-huayan-medium.onnx"),
        );
        if !d.pop() {
            break;
        }
    }
    out
}

/// v18.8-fix28g：piper 命令定位——PATH which 优先，其次常见 conda/系统安装路径
/// （Tauri 桌面/服务启动时 PATH 可能精简，找不到 anaconda3/bin/piper 导致无声）。
fn piper_bin() -> Option<std::path::PathBuf> {
    if which("piper") {
        return Some(std::path::PathBuf::from("piper"));
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for base in [
        format!("{home}/anaconda3/bin/piper"),
        format!("{home}/miniconda3/bin/piper"),
        format!("{home}/miniforge3/bin/piper"),
        "/opt/conda/bin/piper".to_string(),
        "/usr/local/bin/piper".to_string(),
    ] {
        let p = std::path::PathBuf::from(&base);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// v18.8-fix2：Piper 离线神经网络中文女声（可选项；需 piper 命令 + 模型文件）。
/// 完全离线本地合成——隐私=不发送任何文本；音质接近自然女声，远优于 espeak。
/// Piper 是否就绪：piper 命令 + 内置模型（assets/tts/piper/zh_CN-huayan-medium.onnx + .json）。
fn piper_available() -> bool {
    if piper_bin().is_none() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(exe_dir) = exe.parent() else { return false };
    match piper_model_candidates(exe_dir).into_iter().find(|p| p.exists()) {
        Some(model) => model.with_extension("onnx.json").exists(),
        None => false,
    }
}

/// 模型准备：bash scripts/piper_download.sh（ModelScope 国内源，~63MB）+ pip install piper-tts。
/// 返回 Some(true)=已播放；None=不可用（调用方依次回退 espeak）。
/// v18.8-fix28h：通用 wav 播放器链（PulseAudio paplay / ALSA aplay / ffplay / mpv / cvlc / sox play）。
/// 命中任一播放器即 spawn 并返回 true；全部缺失返回 false（调用方回退下一级引擎）。
/// v18.8-fix28i：rodio 播放音频文件（wav/mp3 等）——系统音频 API 级输出，
/// 自动解码+重采样到输出设备，不依赖任何外部播放器命令（paplay/aplay/ffplay）。
fn play_wav_rodio(path: &std::path::Path) -> bool {
    // 修复：统一走全局 OutputStream（持有至播完）+ Sink 队列，
    // 不再函数返回即 drop 设备句柄（rodio 官方语义：drop 即停播）。
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[qibai] tts-chain: rodio 打开音频失败 {}: {}", path.display(), e);
            emit_playback(PlayEvent::Failed { engine: "rodio".into(), error: format!("打开失败: {e}") });
            return false;
        }
    };
    let dur_ms = match play_audio_bytes(&bytes, "rodio") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[qibai] tts-chain: rodio 播放失败: {}", e);
            emit_playback(PlayEvent::Failed { engine: "rodio".into(), error: e.clone() });
            return false;
        }
    };
    eprintln!("[qibai] tts-chain: rodio 系统音频播放 {} duration_ms={}", path.display(), dur_ms);
    // 播放期间麦克风静默（AEC 替代），时长以音频实际时长为准
    mark_speaking_for(Duration::from_millis(dur_ms.max(100)));
    true
}

fn play_wav_with_players(wav: &std::path::Path) -> bool {
    // v18.8-fix28i：系统音频 API 播放优先（rodio/cpal 直接输出，不依赖外部播放器命令）
    // ——桌面无声卡命令（paplay/aplay 缺失）时也能出声，这是行业标准播放层。
    if play_wav_rodio(wav) {
        return true;
    }
    let players: [(&str, &[&str]); 6] = [
        ("paplay", &[]),
        ("aplay", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpv", &["--really-quiet", "--no-video"]),
        ("cvlc", &["--play-and-exit", "--no-video"]),
        ("play", &["-q"]),
    ];
    for (player, extra) in players {
        if which(player) {
            eprintln!("[qibai] tts: 播放器 {player} 播放 wav");
            let mut argv: Vec<String> = Vec::with_capacity(extra.len() + 2);
            argv.push(player.to_string());
            argv.extend(extra.iter().map(|s| s.to_string()));
            argv.push(wav.to_string_lossy().to_string());
            // v18.8-fix29：播放器假成功检测——300ms 内退出视为失败（音频后端不可用），
            // 继续尝试下一播放器，避免「进程拉起即返回成功」导致实际无声。
            let mut child = match std::process::Command::new(&argv[0]).args(&argv[1..]).spawn() {
                Ok(c) => c,
                Err(_) => continue,
            };
            std::thread::sleep(std::time::Duration::from_millis(300));
            match child.try_wait() {
                Ok(Some(_)) => {
                    eprintln!("[qibai] tts: 播放器 {player} 立即退出（疑似播放失败），尝试下一播放器");
                    continue;
                }
                Ok(None) => {
                    // P0 回声根治：rodio 不可用、回落外部播放器时绕过 Job::Play（203 tap
                    // 覆盖不到），读盘补参考信号兜底 tap。
                    if let Ok(bytes) = std::fs::read(wav) {
                        crate::aec::ingest_reference_wav(&bytes);
                        echo_gate_note_wav_bytes(&bytes); // P0-B：补播放能量环
                    }
                    mark_speaking_for(wav_duration_est(wav));
                    return true;
                }
                Err(_) => continue,
            }
        }
    }
    eprintln!("[qibai] tts: no player found (paplay/aplay/ffplay/mpv/play/cvlc)");
    false
}

fn piper_speak(text: &str) -> Option<bool> {
    let bin = piper_bin()?;
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let model = piper_model_candidates(&exe_dir)
        .into_iter()
        .find(|p| p.exists())?;
    let config = model.with_extension("onnx.json");
    if !config.exists() {
        return None;
    }
    let tag = format!(
        "qibai-piper-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    );
    let wav = std::env::temp_dir().join(format!("{tag}.wav"));
    let (rate, _pitch) = current_tts_rate_pitch();
    let ls = piper_length_scale_arg(rate);
    let mut child = std::process::Command::new(&bin)
        .arg("--model")
        .arg(&model)
        .arg("--config")
        .arg(&config)
        .arg("--length-scale")
        .arg(&ls)
        .arg("--output_file")
        .arg(&wav)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let status = child.wait().ok()?;
    if !status.success() || !wav.exists() {
        return None;
    }
    if play_wav_with_players(&wav) {
        return Some(true);
    }
    eprintln!("[qibai] tts: piper wav ready but no player → 降级下一级引擎");
    None
}

/// 捆绑引擎发声：espeak-ng 同步生成 wav → paplay/aplay 播放。
/// 返回 Some(true)=已播放；Some(false)=播放器不可用（调用方降级）；None=生成失败（走系统引擎兜底）。
fn bundle_speak_via_player(
    b: &BundledEspeak,
    text: &str,
    rate: f32,
    volume: u8,
    pitch: f32,
    engine_voice_id: &str,
) -> Option<bool> {
    let mut argv = espeak_cmd(
        &b.bin.to_string_lossy(),
        text,
        rate,
        volume,
        pitch,
        engine_voice_id,
    );
    argv.insert(1, format!("--path={}", b.data.to_string_lossy()));
    argv.push("--stdout".to_string());

    // 同步生成 wav（等 espeak-ng 退出；文本已 clip，耗时 <2s）
    let run_espeak = |argv: &[String]| {
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env("LD_LIBRARY_PATH", b.lib.as_os_str())
            .output()
            .ok()
    };
    let mut out = run_espeak(&argv);
    // v18.8-fix：若女声变体不被捆绑版支持（老 espeak-ng），把 -v 后的 zh+f3 换成 zh 重试，保底有声
    if out.as_ref().map_or(true, |o| !o.status.success() || o.stdout.is_empty()) {
        if let Some(vi) = argv.iter().position(|a| a == "-v") {
            if argv.get(vi + 1).map(|v| v == "zh+f3").unwrap_or(false) {
                argv[vi + 1] = "zh".to_string();
                out = run_espeak(&argv);
            }
        }
    }
    let out = out?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let wav = std::env::temp_dir().join(format!("qibai_tts_{}.wav", std::process::id()));
    if std::fs::write(&wav, &out.stdout).is_err() {
        return None;
    }
    // P0 回声根治：bundled-espeak 走外部 paplay/aplay 播放（绕过 rodio 全局队列，
    // 203 tap 覆盖不到）——wav 字节在此直送 AEC 参考环兜底。
    crate::aec::ingest_reference_wav(&out.stdout);
    // P0-B：外部播放器路径无 rodio 线程能量源，wav 字节解码出 RMS 补播放能量环。
    echo_gate_note_wav_bytes(&out.stdout);
    // v16：多播放器探测（PulseAudio 桌面=paplay；ALSA= aplay；ffplay/mpv/sox 兜底），
    // 每一步打诊断日志，便于真机定位"无声"发生在哪一环。
    let players: [(&str, &[&str]); 6] = [
        ("paplay", &[]),
        ("aplay", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpv", &["--really-quiet", "--no-video"]),
        ("play", &["-q"]),
        ("cvlc", &["--play-and-exit", "--no-video"]),
    ];
    for (player, extra) in players {
        if which(player) {
            let mut argv: Vec<String> = Vec::with_capacity(extra.len() + 2);
            argv.push(player.to_string());
            argv.extend(extra.iter().map(|s| s.to_string()));
            argv.push(wav.to_string_lossy().to_string());
            let p = wav.to_string_lossy().to_string();
            let _ = spawn_detached(&argv);
            eprintln!("[qibai] tts: speak via {} (wav={}, text_len={})", player, p, text.chars().count());
            return Some(true);
        }
    }
    eprintln!("[qibai] tts: no player found (paplay/aplay/ffplay/mpv/play/cvlc)");
    Some(false)
}

/// 云端 TTS 请求（url / headers / body 已就绪，key 只在 headers 里）。
pub struct CloudTtsRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// 构造云端 TTS 请求（纯函数，可单测）。
/// key 空 → Err(MissingCloudKey)；key 只进 header，body 绝不含 key。
/// speaker 为空时 body 缺省该字段（与旧行为一致）。
pub fn build_cloud_request(settings: &VoiceSettings, text: &str) -> Result<CloudTtsRequest, TtsError> {
    build_cloud_request_with_speaker(settings, text, &settings.cloud_voice)
}

/// 构造云端 TTS 请求（可显式指定 speaker：每角色音色档案解析后的云端 speaker）。
/// speaker 非空 → body 含 "speaker" 字段；为空 → 缺省（与旧行为一致）。
/// key 只进 header；body 只含当前朗读文本片段，不含对话历史、不含 key。
pub fn build_cloud_request_with_speaker(
    settings: &VoiceSettings,
    text: &str,
    speaker: &str,
) -> Result<CloudTtsRequest, TtsError> {
    let key = settings.cloud_key().ok_or(TtsError::MissingCloudKey)?;
    // 朗读前清洗：markdown/emoji 不读出来；纯文本输入清洗是恒等操作（既有测试零影响）。
    let text = clean_tts_text(text);
    let text_json = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string());
    let speaker_field = if speaker.trim().is_empty() {
        String::new()
    } else {
        format!(
            ",\"speaker\":{}",
            serde_json::to_string(speaker).unwrap_or_else(|_| "\"\"".to_string())
        )
    };
    // 隐私：body 只含当前朗读文本片段，不含对话历史、不含 key。
    let body = format!(
        "{{\"text\":{text_json}{speaker_field},\"audio\":{{\"format\":\"wav\",\"sample_rate\":24000,\"encoding\":\"raw\"}}}}"
    );
    Ok(CloudTtsRequest {
        url: CLOUD_TTS_URL.to_string(),
        headers: vec![
            ("X-Access-Token".to_string(), key),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        body,
    })
}

/// 云端合成：POST 请求体，200 取 bytes。任何失败以 Err 上交，调用方降级本地/静音。
/// v20：整体 15s 超时（弱网/内网不可达 openspeech.bytedance.com 时快速失败降级）。
pub async fn synth_cloud(req: CloudTtsRequest) -> Result<Vec<u8>, TtsError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(TtsError::CloudRequest)?;
    let mut builder = client.post(&req.url).body(req.body);
    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let resp = builder.send().await.map_err(TtsError::CloudRequest)?;
    let resp = resp.error_for_status().map_err(TtsError::CloudRequest)?;
    let bytes = resp.bytes().await.map_err(TtsError::CloudRequest)?;
    Ok(bytes.to_vec())
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("缺少云端 TTS key（settings.voice.cloud_api_key 或环境变量 QIBAI_TTS_KEY）")]
    MissingCloudKey,
    #[error("云端 TTS 请求失败: {0}")]
    CloudRequest(#[from] reqwest::Error),
    #[error("本地 TTS spawn 失败: {0}")]
    LocalSpawn(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清理所有可能影响 key 探测的环境变量（调用方需持 env_lock）。
    fn clean_env() {
        unsafe {
            std::env::remove_var(ENV_CLOUD_KEY);
            std::env::remove_var(ENV_STT_KEY);
            std::env::remove_var(FALLBACK_ENV_KEY);
        }
    }

    fn voice(engine: TtsEngineKind, cloud_key: &str) -> VoiceSettings {
        VoiceSettings {
            enabled: true,
            engine,
            cloud_api_key: cloud_key.to_string(),
            ..VoiceSettings::default()
        }
    }

    #[test]
    fn piper_falls_back_when_unavailable() {
        // 无 piper 命令或模型时，piper_speak 必须返回 None（调用方回退 espeak），不 panic。
        let _ = piper_speak("元气小小测试");
        // 候选路径解析不崩溃、不产生空模型候选
        let exe_dir = std::env::current_exe().unwrap();
        let _ = piper_model_candidates(exe_dir.parent().unwrap());
        assert!(piper_model_candidates(std::path::Path::new("/")).len() >= 1);
    }

    #[test]
    fn voice_defaults_match_spec() {
        let v = VoiceSettings::default();
        assert!(v.enabled);
        assert_eq!(v.engine, TtsEngineKind::Auto);
        assert_eq!(v.volume, 80);
        assert_eq!(v.rate, 1.0);
        assert_eq!(v.scope, TtsScope::All);
        assert_eq!(v.cloud_provider, "doubao");
        assert_eq!(v.cloud_voice, "");
        assert_eq!(v.cloud_api_key, "");
        assert!(!v.stt_enabled);
        assert_eq!(v.stt_provider, "doubao");
        assert_eq!(v.stt_api_key, "");
    }

    #[test]
    fn resolve_engine_all_branches_per_spec() {
        let _g = crate::test_support::env_lock();
        clean_env();

        // !enabled → Mute("总开关已关闭")
        let mut off = voice(TtsEngineKind::Auto, "k");
        off.enabled = false;
        assert_eq!(
            resolve_engine(&off, true),
            TtsDecision { engine: ResolvedEngine::Mute, reason: "总开关已关闭" }
        );

        // v18.8-fix28g：Auto 有 key + 本地可用 → 本地优先（隐私优先）
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Auto, "k"), true),
            TtsDecision { engine: ResolvedEngine::Local, reason: "本地 TTS（隐私优先）" }
        );
        // Auto：有 key + 本地不可用 → Cloud("云端已配置")
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Auto, "k"), false),
            TtsDecision { engine: ResolvedEngine::Cloud, reason: "云端已配置" }
        );
        // Auto：无 key + 本地可用 → Local("本地 TTS（隐私优先）")
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Auto, ""), true),
            TtsDecision { engine: ResolvedEngine::Local, reason: "本地 TTS（隐私优先）" }
        );
        // Auto：无 key + 无本地 → Mute(无可用引擎)
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Auto, ""), false),
            TtsDecision { engine: ResolvedEngine::Mute, reason: "无可用引擎（未配置云端 key 且本地无 TTS）" }
        );

        // Local：本地可用 → Local；不可用 → Mute("本地 TTS 不可用")
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Local, ""), true),
            TtsDecision { engine: ResolvedEngine::Local, reason: "本地系统 TTS" }
        );
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Local, ""), false),
            TtsDecision { engine: ResolvedEngine::Mute, reason: "本地 TTS 不可用" }
        );

        // Cloud：有 key → Cloud；无 key + 本地 → 降级本地；无 key + 无本地 → Mute
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Cloud, "k"), false),
            TtsDecision { engine: ResolvedEngine::Cloud, reason: "云端已配置" }
        );
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Cloud, ""), true),
            TtsDecision { engine: ResolvedEngine::Local, reason: "云端无 key，降级本地 TTS" }
        );
        assert_eq!(
            resolve_engine(&voice(TtsEngineKind::Cloud, ""), false),
            TtsDecision { engine: ResolvedEngine::Mute, reason: "云端无 key 且本地无 TTS" }
        );
    }

    #[test]
    fn cloud_key_env_fallback_chain() {
        let _g = crate::test_support::env_lock();
        clean_env();
        let v = VoiceSettings::default();
        assert_eq!(v.cloud_key(), None);
        unsafe { std::env::set_var(ENV_CLOUD_KEY, "from-qiabai") };
        assert_eq!(v.cloud_key().as_deref(), Some("from-qiabai"));
        unsafe { std::env::set_var(FALLBACK_ENV_KEY, "from-doubao") };
        assert_eq!(v.cloud_key().as_deref(), Some("from-qiabai")); // QIBAI_TTS_KEY 优先
        unsafe { std::env::remove_var(ENV_CLOUD_KEY) };
        assert_eq!(v.cloud_key().as_deref(), Some("from-doubao"));
        // settings 值优先于环境变量
        let mut v2 = VoiceSettings::default();
        v2.cloud_api_key = "from-settings".to_string();
        assert_eq!(v2.cloud_key().as_deref(), Some("from-settings"));
        clean_env();
    }

    #[test]
    fn scope_allows_matrix() {
        use TtsKind::*;
        // All → 全部放行
        for k in [Chat, Reminder, LevelUp, Guard] {
            assert!(scope_allows(TtsScope::All, k));
        }
        // Chat → 仅 Chat
        assert!(scope_allows(TtsScope::Chat, Chat));
        assert!(!scope_allows(TtsScope::Chat, Reminder));
        assert!(!scope_allows(TtsScope::Chat, LevelUp));
        assert!(!scope_allows(TtsScope::Chat, Guard));
        // Reminders → 仅 Reminder
        assert!(scope_allows(TtsScope::Reminders, Reminder));
        assert!(!scope_allows(TtsScope::Reminders, Chat));
        assert!(!scope_allows(TtsScope::Reminders, LevelUp));
        assert!(!scope_allows(TtsScope::Reminders, Guard));
    }

    #[test]
    fn sanitize_clamps_volume_and_rate() {
        let mut v = VoiceSettings::default();
        v.volume = 250;
        v.rate = 5.0;
        let s = v.sanitize();
        assert_eq!(s.volume, VOLUME_MAX); // 100
        assert_eq!(s.rate, RATE_MAX); // 2.0

        let mut v = VoiceSettings::default();
        v.volume = 0;
        v.rate = 0.1;
        let s = v.sanitize();
        assert_eq!(s.volume, VOLUME_MIN);
        assert_eq!(s.rate, RATE_MIN); // 0.5

        // 区间内原值不变
        let v = VoiceSettings::default();
        let s = v.sanitize();
        assert_eq!(s.volume, 80);
        assert_eq!(s.rate, 1.0);
    }

    #[test]
    fn build_local_cmd_linux_espeak_mapping() {
        // rate=1.0 → wpm=160；volume=80 → amp=160
        // v18.8-fix：voice 空 → 默认中文女声变体 zh+f3
        let argv = build_local_tts_cmd(PlatformKind::Linux, "你好", 1.0, 80, 0.0, "");
        assert_eq!(
            argv,
            vec!["espeak-ng", "-v", "zh+f3", "-s", "160", "-a", "160", "你好"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        // rate 边界：0.5 → 80；2.0 → 320
        let argv = build_local_tts_cmd(PlatformKind::Linux, "x", 0.5, 0, 0.0, "");
        assert_eq!(argv[4], "80");
        assert_eq!(argv[6], "0");
        let argv = build_local_tts_cmd(PlatformKind::Linux, "x", 2.0, 100, 0.0, "");
        assert_eq!(argv[4], "320");
        assert_eq!(argv[6], "200");
    }

    #[test]
    fn build_local_cmd_mac_mapping() {
        // rate=1.0 → wpm=175
        let argv = build_local_tts_cmd(PlatformKind::Mac, "hi", 1.0, 80, 0.0, "");
        assert_eq!(argv, vec!["say", "-r", "175", "hi"]);
        // 边界：rate=0.5 → 87.5 → round 88；rate=2.0 → 350
        let argv = build_local_tts_cmd(PlatformKind::Mac, "hi", 0.5, 80, 0.0, "");
        assert_eq!(argv[2], "88");
        let argv = build_local_tts_cmd(PlatformKind::Mac, "hi", 2.0, 80, 0.0, "");
        assert_eq!(argv[2], "350");
    }

    #[test]
    fn build_local_cmd_windows_rate_volume_and_quote_escape() {
        // rate=1.0 → r=0；volume=80
        let argv = build_local_tts_cmd(PlatformKind::Windows, "it's a test", 1.0, 80, 0.0, "");
        assert_eq!(argv[0], "powershell");
        assert_eq!(argv[1], "-NoProfile");
        assert_eq!(argv[2], "-Command");
        assert!(argv[3].contains("$s.Rate=0"));
        assert!(argv[3].contains("$s.Volume=80"));
        // 单引号转义为 ''
        assert!(argv[3].contains("it''s a test"));
        // rate=2.0 → r=10；rate=0.5 → r=-5
        let argv = build_local_tts_cmd(PlatformKind::Windows, "x", 2.0, 10, 0.0, "");
        assert!(argv[3].contains("$s.Rate=10"));
        assert!(argv[3].contains("$s.Volume=10"));
        let argv = build_local_tts_cmd(PlatformKind::Windows, "x", 0.5, 10, 0.0, "");
        assert!(argv[3].contains("$s.Rate=-5"));
    }

    // ---- 迭代⑤：三平台 pitch / engine_voice_id 映射 ----

    #[test]
    fn build_local_cmd_espeak_pitch_and_voice_id() {
        // pitch=3 → 50+3*3=59，插入位置在 -a amp 值之后
        let argv = build_local_tts_cmd(PlatformKind::Linux, "你好", 1.0, 80, 3.0, "");
        assert_eq!(argv[0], "espeak-ng");
        assert_eq!(&argv[1..8], ["-v", "zh+f3", "-s", "160", "-a", "160", "-p"]);
        assert_eq!(argv[8], "59");
        assert_eq!(argv[9], "你好");
        // pitch=-2 → 50-6=44
        let argv = build_local_tts_cmd(PlatformKind::Linux, "hi", 1.0, 80, -2.0, "");
        let i = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv[i + 1], "44");
        // 越界 clamp：pitch=20 → 110 → 99；pitch=-20 → -10 → 0
        let argv = build_local_tts_cmd(PlatformKind::Linux, "hi", 1.0, 80, 20.0, "");
        let i = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv[i + 1], "99");
        let argv = build_local_tts_cmd(PlatformKind::Linux, "hi", 1.0, 80, -20.0, "");
        let i = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv[i + 1], "0");
        // engine_voice_id 非空 → -v {id} 替换 -v zh
        let argv = build_local_tts_cmd(PlatformKind::Linux, "hi", 1.0, 80, 0.0, "myvoice");
        assert_eq!(&argv[1..3], ["-v", "myvoice"]);
        // pitch=0 + voice 空 → 无 -p、-v zh+f3（v18.8-fix 女声默认）
        let argv = build_local_tts_cmd(PlatformKind::Linux, "你好", 1.0, 80, 0.0, "");
        assert_eq!(
            argv,
            vec!["espeak-ng", "-v", "zh+f3", "-s", "160", "-a", "160", "你好"]
        );
    }

    #[test]
    fn build_local_cmd_mac_pbas_prefix_and_voice_id() {
        // pitch=3 → [[pbas +9]]；前缀紧贴文本，在 -r 之后
        let argv = build_local_tts_cmd(PlatformKind::Mac, "你好", 1.0, 80, 3.0, "");
        assert_eq!(argv, vec!["say", "-r", "175", "[[pbas +9]]你好"]);
        // pitch=-2 → [[pbas -6]]（负数自带负号，无 +）
        let argv = build_local_tts_cmd(PlatformKind::Mac, "你好", 1.0, 80, -2.0, "");
        assert_eq!(argv[3], "[[pbas -6]]你好");
        // engine_voice_id 非空 → say -v id -r wpm text
        let argv = build_local_tts_cmd(PlatformKind::Mac, "hi", 1.0, 80, 0.0, "Samantha");
        assert_eq!(&argv[0..4], ["say", "-v", "Samantha", "-r"]);
        assert_eq!(argv[4], "175");
        assert_eq!(argv[5], "hi");
        // pitch=0 + voice 空 → 与旧输出逐字节一致
        let argv = build_local_tts_cmd(PlatformKind::Mac, "hi", 1.0, 80, 0.0, "");
        assert_eq!(argv, vec!["say", "-r", "175", "hi"]);
    }

    #[test]
    fn build_local_cmd_windows_pitch_xml_and_select_voice() {
        // pitch=2 → absmiddle = 5.0+1.0 = 6.0（一位小数），包裹 Speak 文本
        let argv = build_local_tts_cmd(PlatformKind::Windows, "你好", 1.0, 80, 2.0, "");
        assert!(argv[3].contains("<pitch absmiddle=\"6.0\"/>你好"));
        // pitch=-2 → 4.0
        let argv = build_local_tts_cmd(PlatformKind::Windows, "hi", 1.0, 80, -2.0, "");
        assert!(argv[3].contains("<pitch absmiddle=\"4.0\"/>hi"));
        // 越界 clamp：pitch=20 → 15 → 10.0；pitch=-20 → -5 → 0.0
        let argv = build_local_tts_cmd(PlatformKind::Windows, "hi", 1.0, 80, 20.0, "");
        assert!(argv[3].contains("<pitch absmiddle=\"10.0\"/>"));
        let argv = build_local_tts_cmd(PlatformKind::Windows, "hi", 1.0, 80, -20.0, "");
        assert!(argv[3].contains("<pitch absmiddle=\"0.0\"/>"));
        // engine_voice_id 非空 → $s.SelectVoice 注入在 $s.Speak 之前
        let argv = build_local_tts_cmd(PlatformKind::Windows, "hi", 1.0, 80, 0.0, "Microsoft-Sam");
        let s = &argv[3];
        let speak_pos = s.find("$s.Speak").unwrap();
        let sel_pos = s.find("$s.SelectVoice('Microsoft-Sam')").unwrap();
        assert!(sel_pos < speak_pos);
        // pitch=0 + voice 空 → 无 pitch 标签、无 SelectVoice（与旧输出逐字节一致）
        let argv = build_local_tts_cmd(PlatformKind::Windows, "hi", 1.0, 80, 0.0, "");
        assert!(!argv[3].contains("<pitch"));
        assert!(!argv[3].contains("SelectVoice"));
        assert!(argv[3].ends_with("$s.Speak('hi')"));
    }

    #[test]
    fn clip_text_truncates_with_ellipsis() {
        // 短文本原样
        assert_eq!(clip_text("你好", 200), "你好");
        // 250 字符 → 截到 200 + ……
        let long: String = "啊".repeat(250);
        let clipped = clip_text(&long, 200);
        assert_eq!(clipped.chars().count(), 202);
        assert!(clipped.ends_with("……"));
        // 边界：恰好 200 不追加省略号
        let exact: String = "啊".repeat(200);
        assert_eq!(clip_text(&exact, 200), exact);
    }

    // ---- v45.7 本地长文本切分：一字不丢 ----

    /// 600 字中文（句号/逗号/无标点混合）→ 段数 ≥3，拼接后去空白与原文一致，且无 "……"。
    #[test]
    fn tts_split_preserves_all_text() {
        // 构造 600 字：三段 200 字段落，句间用句号，段内用逗号
        let mut text = String::new();
        for _ in 0..3 {
            // 每段约 200 字：交替"这是第 i 句话，"重复 + 句号收尾
            for _ in 0..30 {
                text.push_str("这是测试切分的一句话，");
            }
            text.push('。');
        }
        // 再加一段无标点长串
        text.push_str(&"无标点长文本".repeat(20));
        let total = text.chars().count();
        let segs = split_for_local_tts(&text, 200);
        assert!(segs.len() >= 3, "600+ 字应切成 ≥3 段，实际 {}", segs.len());
        // 每段 ≤200
        for s in &segs {
            assert!(s.chars().count() <= 200, "段超 200: {}", s.chars().count());
        }
        // 拼接去空白 == 原文去空白
        let joined: String = segs.concat();
        let norm = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        assert_eq!(norm(&joined), norm(&text));
        // 不允许出现兜底省略号
        assert!(!joined.contains("……"));
        assert!(total > 500);
    }

    /// 400 字无标点单句 → 仍被硬切为 ≥2 段且不丢字。
    #[test]
    fn tts_split_single_huge_sentence() {
        let text = "啊".repeat(400);
        let segs = split_for_local_tts(&text, 200);
        assert!(segs.len() >= 2, "400 字无标点单句应硬切为 ≥2 段，实际 {}", segs.len());
        let joined: String = segs.concat();
        assert_eq!(joined.chars().count(), 400);
    }

    /// 80 字短文本 → 1 段，原样。
    #[test]
    fn tts_split_short_text_single_segment() {
        let text = "你好，今天天气不错。我们出去走走吧。";
        let segs = split_for_local_tts(text, 200);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], text);
    }

    #[test]
    fn cloud_request_url_headers_body_shape_and_privacy() {
        let _g = crate::test_support::env_lock();
        clean_env();
        let mut v = VoiceSettings::default();
        v.cloud_api_key = "SECRET-K-123".to_string();

        let req = build_cloud_request(&v, "今天风很温柔").unwrap();
        assert_eq!(req.url, CLOUD_TTS_URL);
        // key 只进 header
        assert_eq!(req.headers[0], ("X-Access-Token".to_string(), "SECRET-K-123".to_string()));
        assert_eq!(req.headers[1], ("Content-Type".to_string(), "application/json".to_string()));
        // body 是合法 JSON，且绝不含 key
        assert!(!req.body.contains("SECRET-K-123"));
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["text"], "今天风很温柔");
        assert!(body.get("speaker").is_none()); // cloud_voice 空 → 缺省
        assert_eq!(body["audio"]["format"], "wav");
        assert_eq!(body["audio"]["sample_rate"], 24000);
        assert_eq!(body["audio"]["encoding"], "raw");

        // 指定音色 → speaker 进 body
        v.cloud_voice = "zh_female".to_string();
        let req = build_cloud_request(&v, "hi").unwrap();
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["speaker"], "zh_female");

        // 空 key → MissingCloudKey（不打日志、不构造请求）
        let mut v2 = VoiceSettings::default();
        v2.cloud_api_key = "".to_string();
        assert!(matches!(
            build_cloud_request(&v2, "hi"),
            Err(TtsError::MissingCloudKey)
        ));
    }

    #[test]
    fn detect_local_available_and_run_no_panic_in_sandbox() {
        // v14：随包捆绑 espeak-ng → 探测为 true；run_local_tts 应成功 spawn（不 panic）。
        // （若环境变量被外部置空且无系统引擎，也允许 false + Ok(false) 的旧路径。）
        // 确定性修复（与 bundle_speak 同口径）：持有 env_lock + 显式注入捆绑引擎目录，
        // 避免沙盒下探测结果依赖环境抖动（detect 与 run 候选不一致导致断言摇摆）。
        let _g = crate::test_support::env_lock();
        // 显式注入捆绑引擎目录（与 bundled_tests::env_dir 同路径），探测结果确定。
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let bundle_dir = root.join("src-tauri").join("tts-espeak");
        unsafe { std::env::set_var("QIBAI_TTS_ESPEAK_DIR", &bundle_dir); }
        let available = detect_local_tts_available();
        // v16：捆绑分支 = wav 生成 + 播放器播放；无播放器（无声卡 CI）时 Ok(false) 是合法降级。
        // 端到端"能出声"由 bundle_speak_generates_wav_and_plays 的生成侧覆盖 + 真机验收。
        let r = run_local_tts("测试", 1.0, 80, 0.0, "");
        assert!(r.is_ok(), "本地发声链路不应 panic");
        if !available {
            assert_eq!(r.unwrap(), false);
        }
    }

    #[test]
    fn cloud_request_with_speaker_includes_or_omits_speaker_field() {
        let _g = crate::test_support::env_lock();
        clean_env();
        let mut v = VoiceSettings::default();
        v.cloud_api_key = "K-123".to_string();

        // speaker 非空 → body 含 "speaker" 字段
        let req = build_cloud_request_with_speaker(&v, "你好", "zh_female_wenrouxiaoya_moon_bigtts")
            .unwrap();
        assert_eq!(req.headers[0].1, "K-123");
        assert!(!req.body.contains("K-123"));
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["speaker"], "zh_female_wenrouxiaoya_moon_bigtts");
        assert_eq!(body["text"], "你好");

        // speaker 为空/空白 → body 缺省该字段（与旧 build_cloud_request 行为一致）
        let req = build_cloud_request_with_speaker(&v, "你好", "").unwrap();
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert!(body.get("speaker").is_none());
        // 原 build_cloud_request 走 settings.cloud_voice 兜底，旧断言原样通过（见上一测试）
        v.cloud_voice = "zh_female_shuangkuaisisi_emo_v2_mars_bigtts".into();
        let req = build_cloud_request(&v, "hi").unwrap();
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["speaker"], "zh_female_shuangkuaisisi_emo_v2_mars_bigtts");
    }
}

#[cfg(test)]
mod bundled_tests {
    use super::*;
    use std::sync::Once;
    static INIT: Once = Once::new();
    fn env_dir() -> String {
        INIT.call_once(|| {});
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let d = root.join("src-tauri").join("tts-espeak");
        std::env::set_var("QIBAI_TTS_ESPEAK_DIR", &d);
        d.to_string_lossy().to_string()
    }
    #[test]
    fn bundled_espeak_detects_env_dir() {
        let d = env_dir();
        assert!(std::path::Path::new(&d).join("bin").join("espeak-ng").is_file(), "捆绑 espeak-ng 缺失: {d}");
        let b = bundled_espeak();
        assert!(b.is_some(), "bundled_espeak() 未探测到捆绑引擎（env: {d}）");
        let b = b.unwrap();
        assert!(b.data.is_dir(), "语音数据目录缺失: {}", b.data.display());
    }
    #[test]
    fn bundle_speak_generates_wav_and_plays() {
        // 真实捆绑引擎：wav 生成必须成功（--stdout），播放器可用时返回 Ok(true)。
        let _g = crate::test_support::env_lock();
        env_dir();
        let b = bundled_espeak().expect("捆绑引擎应可探测");
        match bundle_speak_via_player(&b, "你好，测试发声", 1.0, 80, 0.0, "") {
            Some(true) => { /* 已播放 */ }
            Some(false) => { /* 无播放器（无声卡 CI）——生成侧必须成功才可能到这 */ }
            None => panic!("捆绑 espeak wav 生成失败——发声链路故障"),
        }
        // 清理本次生成的 wav
        let wav = std::env::temp_dir().join(format!("qibai_tts_{}.wav", std::process::id()));
        let _ = std::fs::remove_file(wav);
    }

    #[test]
    fn detect_local_tts_available_with_bundle() {
        env_dir();
        assert!(detect_local_tts_available(), "捆绑引擎存在时应判定本地 TTS 可用");
    }
    #[test]
    fn bundled_espeak_real_layout_from_target_debug() {
        // 不设环境变量，验证真机相对路径：exe 位于 <工程>/src-tauri/target/debug/
        // → 上两级 = src-tauri → src-tauri/tts-espeak 必须命中（v15 修复点）。
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src-tauri");
        let debug_dir = root.join("target").join("debug");
        let cand = debug_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("tts-espeak"));
        assert_eq!(cand.as_deref(), Some(root.join("tts-espeak").as_path()),
            "上两级候选必须等于 <工程>/src-tauri/tts-espeak");
        let probed = probe_espeak(&root.join("tts-espeak"));
        assert!(probed.is_some(),
            "真实捆绑目录（{}）应可探测", root.join("tts-espeak").display());
        let b = probed.unwrap();
        assert!(b.bin.is_file() && b.data.is_dir());
    }
    #[test]
    fn espeak_candidates_include_src_tauri_on_third_hop() {
        // <工程根>/src-tauri/target/debug → 第 3 级 = <工程根>/src-tauri → 命中 tts-espeak
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src-tauri");
        let debug_dir = root.join("target").join("debug");
        let cands = espeak_candidates_from_exe_dir(&debug_dir);
        assert!(
            cands.iter().any(|c| c == &root.join("tts-espeak")),
            "逐级上溯候选必须含 <工程根>/src-tauri/tts-espeak（第 3 级），实际：{:?}",
            cands
        );
    }

    #[test]
    fn bundled_espeak_resolves_real_bundle_without_env() {
        // 不设环境变量：逐级上溯 + cwd 候选应能命中真实捆绑目录（工程内 src-tauri/tts-espeak）。
        // current_exe 指向测试二进制（target/debug/deps/...），上溯候选覆盖 <工程>/src-tauri；
        // cwd 候选覆盖 cargo run 场景。二者任一命中即可。
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let real = root.join("src-tauri").join("tts-espeak");
        let probed = bundled_espeak();
        assert!(
            probed.is_some(),
            "未设环境变量时 bundled_espeak() 应能命中 {}（上溯/cwd 候选）",
            real.display()
        );
        let b = probed.unwrap();
        assert!(b.bin.is_file() && b.data.is_dir());
    }

    // ---------------- v20 新增分支测试 ----------------

    #[test]
    fn run_with_timeout_fast_cmd_succeeds() {
        // 快速命令（true）在超时前退出且成功 → Some(())
        let mut cmd = std::process::Command::new("true");
        assert!(run_with_timeout(&mut cmd, std::time::Duration::from_secs(2)).is_some());
    }

    #[test]
    fn run_with_timeout_hang_cmd_is_killed() {
        // 挂死命令（sleep 30）在 1s 超时被 kill → None（不阻塞调用方）
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        let t0 = std::time::Instant::now();
        let out = run_with_timeout(&mut cmd, std::time::Duration::from_secs(1));
        assert!(out.is_none(), "超时挂死命令应返回 None");
        // 应在 ~1-2s 内返回，而非等到 sleep 30
        assert!(t0.elapsed().as_secs() < 5, "超时退出过快返回，实际 {:?}", t0.elapsed());
    }

    #[test]
    fn playback_status_reflects_events() {
        // 无声卡环境也能验证状态中枢：emit 事件后 current_playback_status 同步刷新。
        emit_playback(PlayEvent::Failed {
            engine: "edge-tts".into(),
            error: "timeout".into(),
        });
        let s = current_playback_status();
        assert_eq!(s.engine, "edge-tts");
        assert_eq!(s.state, "failed");
        assert_eq!(s.last_error, "timeout");

        emit_playback(PlayEvent::Done { engine: "piper".into(), duration_ms: 1234 });
        let s2 = current_playback_status();
        assert_eq!(s2.engine, "piper");
        assert_eq!(s2.state, "done");
        assert_eq!(s2.duration_ms, 1234);
        assert!(s2.last_error.is_empty());
    }


    // ---- 朗读前文本清洗 clean_tts_text（与 Python clean_for_tts 同规则） ----

    #[test]
    fn clean_bold_italic() {
        assert_eq!(
            clean_tts_text("**加粗**和*斜体*都不要读出来"),
            "加粗和斜体都不要读出来"
        );
    }

    #[test]
    fn clean_heading() {
        assert_eq!(clean_tts_text("# 标题一\n正文内容"), "标题一 正文内容");
    }

    #[test]
    fn clean_list_markers() {
        assert_eq!(
            clean_tts_text("- 第一项\n- 第二项\n1. 第三项"),
            "第一项 第二项 第三项"
        );
    }

    #[test]
    fn clean_inline_code() {
        assert_eq!(clean_tts_text("运行 `npm install` 即可"), "运行 npm install 即可");
    }

    #[test]
    fn clean_code_fence() {
        let out = clean_tts_text("```python\nprint(1)\n```");
        assert!(!out.contains("```"));
        assert!(out.contains("print(1)"));
    }

    #[test]
    fn clean_link_keeps_text() {
        assert_eq!(clean_tts_text("看[百度](https://x.com)一下"), "看百度一下");
    }

    #[test]
    fn clean_emoji_stripped() {
        let out = clean_tts_text("你好呀🎉，今天开心吗😄");
        assert!(!out.contains('🎉'));
        assert!(!out.contains('😄'));
        assert!(out.contains("你好呀"));
        assert!(out.contains("今天开心吗"));
    }

    #[test]
    fn clean_whitespace_collapsed() {
        assert_eq!(clean_tts_text("你好   世界\t\n再来一句"), "你好 世界 再来一句");
    }

    #[test]
    fn clean_chinese_punct_kept() {
        let s = "你好，世界！这是「测试」：括号（）【】《》都要保留……";
        let out = clean_tts_text(s);
        for ch in ['，', '！', '：', '（', '）', '【', '】', '《', '》', '…'] {
            assert!(out.contains(ch), "标点 {ch} 被误删: {out}");
        }
    }

    #[test]
    fn clean_plain_text_identity() {
        // 纯文本（含既有 build_local_tts_cmd 测试输入）清洗后必须逐字节不变。
        assert_eq!(clean_tts_text("你好"), "你好");
        assert_eq!(clean_tts_text("it's a test"), "it's a test");
        assert_eq!(
            clean_tts_text("今天天气真不错，我们一起去散步吧。"),
            "今天天气真不错，我们一起去散步吧。"
        );
    }

    #[test]
    fn clean_empty() {
        assert_eq!(clean_tts_text(""), "");
    }

    #[test]
    fn cloud_request_cleans_input() {
        // 云端合成入口已清洗：markdown 不进请求体，但 key 仍只在 header。
        let _g = crate::test_support::env_lock();
        let v = VoiceSettings {
            enabled: true,
            engine: TtsEngineKind::Cloud,
            cloud_api_key: "k".to_string(),
            ..VoiceSettings::default()
        };
        let req = build_cloud_request(&v, "**你好**🎉").unwrap();
        assert!(req.body.contains("你好"));
        assert!(!req.body.contains('*'));
        assert!(!req.body.contains('🎉'));
        assert!(req.headers.iter().any(|(h, _)| h == "X-Access-Token"));
    }

}

// ---------------------------------------------------------------- P0-B 回声门单测

#[cfg(test)]
mod echo_gate_tests {
    use super::*;

    /// 回声样本：播放能量高、麦克风能量≈标定回声水平（play×gain）→ 纯回声抑制，
    /// 且回声增益 EMA 自适应上修（自标定）。
    #[test]
    fn echo_only_suppressed_and_gain_adapts() {
        let mut st = EchoGateState::default();
        // gain=0.1, play=0.2 → 预期回声 0.02；mic=0.024（ratio≈1.2，回声态自标定窗内）
        for _ in 0..10 {
            match echo_gate_step(&mut st, 0.024, 0.2) {
                EchoGateVerdict::EchoOnly => {}
                v => panic!("回声样本应判 EchoOnly，实际 {:?}", v),
            }
        }
        // 增益应 EMA 从 0.1 收敛向 0.024/0.2=0.12
        assert!(st.gain > 0.10, "增益应自适应上修: {}", st.gain);
        assert!(st.gain < 0.125, "增益不应超目标: {}", st.gain);
    }

    /// 人声样本（双讲）：播放高、麦克风远超回声预期 → 人声优先 DoubleTalk（连续 2 帧才认定）。
    #[test]
    fn double_talk_human_priority() {
        let mut st = EchoGateState::default();
        // 第 1 帧 ratio=2.5 越 talk 门但未攒够帧数
        assert!(matches!(echo_gate_step(&mut st, 0.06, 0.2), EchoGateVerdict::EchoOnly));
        // 第 2 帧同越门 → DoubleTalk（人声优先，允许打断+转写）
        assert!(matches!(echo_gate_step(&mut st, 0.06, 0.2), EchoGateVerdict::DoubleTalk));
    }

    /// 播放安静 → Quiet（无回声可比，交回正常 VAD）。
    #[test]
    fn quiet_floor() {
        let mut st = EchoGateState::default();
        assert!(matches!(echo_gate_step(&mut st, 0.1, 0.001), EchoGateVerdict::Quiet));
        // talk 态遇 play 安静应复位回 Quiet
        let mut st2 = EchoGateState::default();
        let _ = echo_gate_step(&mut st2, 0.06, 0.2);
        assert!(matches!(echo_gate_step(&mut st2, 0.06, 0.2), EchoGateVerdict::DoubleTalk));
        assert!(matches!(echo_gate_step(&mut st2, 0.1, 0.0), EchoGateVerdict::Quiet));
    }

    /// 滞后防抖：talk 态下 ratio 在 1.2/2.0 间来回摆动不翻转；
    /// 连续 4 帧(80ms) 低于 echo 门 1.5 才退回 EchoOnly。
    #[test]
    fn hysteresis_resists_flicker() {
        let mut st = EchoGateState::default();
        // 先进 talk
        let _ = echo_gate_step(&mut st, 0.06, 0.2);
        assert!(matches!(echo_gate_step(&mut st, 0.06, 0.2), EchoGateVerdict::DoubleTalk));
        // ratio 在 1.2(mic=0.024)/2.0(mic=0.04) 间交替 20 帧：不应翻转（滞后）
        for i in 0..20 {
            let mic = if i % 2 == 0 { 0.024 } else { 0.04 };
            assert!(
                matches!(echo_gate_step(&mut st, mic, 0.2), EchoGateVerdict::DoubleTalk),
                "滞后窗内不应退出 talk at frame {i}"
            );
        }
        // 连续 4 帧 ratio 1.2(<1.5)：前 3 帧仍 DoubleTalk，第 4 帧退回 EchoOnly
        for _ in 0..3 {
            assert!(matches!(echo_gate_step(&mut st, 0.024, 0.2), EchoGateVerdict::DoubleTalk));
        }
        assert!(matches!(echo_gate_step(&mut st, 0.024, 0.2), EchoGateVerdict::EchoOnly));
    }
}

// ---------------------------------------------------------------- P0-B 调优单测

#[cfg(test)]
mod echo_gate_tuning_tests {
    use super::*;

    /// 激进组参数：talk_ratio=2.0 更早认人声；退出滞后 echo_frames=3 更快回回声态。
    #[test]
    fn aggressive_params_trip_earlier_and_release_fast() {
        let p = EchoGateParams {
            talk_ratio: 2.0,
            echo_ratio: 1.8,
            talk_frames: 2,
            echo_frames: 3,
            play_quiet: 0.008,
            postroll_ms: 150,
        };
        let mut st = EchoGateState::default();
        // gain=0.1, play=0.2 → expected=0.02；mic=0.06 → ratio=3.0 ≥ 2.0
        assert!(matches!(echo_gate_step_with(&mut st, 0.065, 0.2, &p), EchoGateVerdict::EchoOnly));
        assert!(matches!(echo_gate_step_with(&mut st, 0.065, 0.2, &p), EchoGateVerdict::DoubleTalk));
        // ratio=1.7(<1.8) 连续：前 2 帧仍 DoubleTalk，第 3 帧退回 EchoOnly
        for _ in 0..2 {
            assert!(matches!(echo_gate_step_with(&mut st, 0.034, 0.2, &p), EchoGateVerdict::DoubleTalk));
        }
        assert!(matches!(echo_gate_step_with(&mut st, 0.034, 0.2, &p), EchoGateVerdict::EchoOnly));
    }

    /// 保守组参数：talk_ratio=3.0 同能量样本不再误触发人声。
    #[test]
    fn conservative_params_reject_medium_ratio() {
        let p = EchoGateParams {
            talk_ratio: 3.0,
            echo_ratio: 1.5,
            talk_frames: 3,
            echo_frames: 6,
            play_quiet: 0.004,
            postroll_ms: 150,
        };
        let mut st = EchoGateState::default();
        // mic=0.06 → ratio=3.0 刚越门，需连续 3 帧才认定；连喂 3 帧 → DoubleTalk。
        for _ in 0..2 {
            assert!(matches!(echo_gate_step_with(&mut st, 0.065, 0.2, &p), EchoGateVerdict::EchoOnly));
        }
        assert!(matches!(echo_gate_step_with(&mut st, 0.065, 0.2, &p), EchoGateVerdict::DoubleTalk));
        // 中等 ratio=2.0(<3.0)：永远不触发人声（保守防误 barge-in）
        let mut st2 = EchoGateState::default();
        for _ in 0..10 {
            assert!(matches!(echo_gate_step_with(&mut st2, 0.04, 0.2, &p), EchoGateVerdict::EchoOnly));
        }
    }

    /// 文本级回声过滤：子串包含规则（Paraformer 准转写跟读回声 → 丢弃）；正常人声不误杀。
    #[test]
    fn text_echo_substring_filter() {
        record_spoken("今天天气真不错呀，要不要出去玩？");
        // 回声跟读：整句/半句是朗读文本子串 → 判回声
        assert!(is_echo_of_recent_speech("今天天气真不错呀"));
        assert!(is_echo_of_recent_speech("真不错呀"));
        // 正常人话：与朗读文本不同主题 → 不判回声
        assert!(!is_echo_of_recent_speech("明天想吃什么"));
        // 过短（<4字）不触发子串规则，仅重叠率兜底
        assert!(!is_echo_of_recent_speech("哦"));
    }

}

#[cfg(test)]
mod audio_device_tests {
    use super::*;

    #[test]
    fn set_audio_output_normalizes_empty_to_default() {
        // 串行锁：避免并行测试踩全局 AUDIO_DEVICE
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();
        set_audio_output("");
        assert_eq!(current_audio_output(), "default");
        set_audio_output("   ");
        assert_eq!(current_audio_output(), "default");
        set_audio_output("MySpeaker");
        assert_eq!(current_audio_output(), "MySpeaker");
        // 幂等
        set_audio_output("MySpeaker");
        assert_eq!(current_audio_output(), "MySpeaker");
        set_audio_output("default");
    }

    #[test]
    fn set_audio_output_idempotent() {
        set_audio_output("dev-A");
        set_audio_output("dev-A");
        assert_eq!(current_audio_output(), "dev-A");
        set_audio_output("default");
    }
}

#[cfg(test)]
mod v459_tts_tests {
    use super::*;

    #[test]
    fn split_segments_40_to_80_chars_and_lossless() {
        // 150 字文本，多句号；用小上限（60）验证切分逻辑（LOCAL_CLIP_CHARS 已改 1200，不再切短文本）
        let text = "今天天气真好。我们一起去公园散步吧。路上可以看看花。小鸟在枝头叫。\
风轻轻吹过脸颊。阳光洒在草地上。孩子们在嬉笑。老人在打太极。一切都很美好。" ;
        let segs = split_for_local_tts(text, 60);
        assert!(segs.len() >= 2, "应至少切 2 段：{segs:?}");
        let joined: String = segs.concat();
        assert_eq!(joined, text, "拼接必须逐字相等，不丢字");
        for s in &segs {
            let n = s.chars().count();
            assert!(n <= 60 + 20, "段长不应远超上限：{n}");
        }
    }

    #[test]
    fn edge_tts_rate_arg_mapping() {
        assert_eq!(super::edge_tts_rate_arg(1.0), "+0%");
        assert_eq!(super::edge_tts_rate_arg(1.5), "+50%");
        assert_eq!(super::edge_tts_rate_arg(0.5), "-50%");
        assert_eq!(super::edge_tts_rate_arg(2.0), "+100%");
    }

    #[test]
    fn piper_length_scale_direction() {
        // rate 快 → length-scale 小（piper 朗读更快）
        assert!(super::piper_length_scale_arg(1.5).parse::<f32>().unwrap() < 1.0);
        assert!(super::piper_length_scale_arg(0.7).parse::<f32>().unwrap() > 1.0);
        assert_eq!(super::piper_length_scale_arg(1.0), "1.00");
    }

    #[test]
    fn edge_tts_pitch_arg_mapping() {
        assert_eq!(super::edge_tts_pitch_arg(1.0), "+0Hz");
        assert_eq!(super::edge_tts_pitch_arg(1.5), "+25Hz");
        assert_eq!(super::edge_tts_pitch_arg(0.5), "-25Hz");
    }

    #[test]
    fn trim_tail_silence_removes_only_tail() {
        // 100 个样本：前 60 个有声音，后 40 个是静音（< 0.01）
        let mut pcm: Vec<f32> = (0..60).map(|i| (i as f32 / 60.0).sin() * 0.5).collect();
        pcm.extend(std::iter::repeat(0.0).take(40));
        let trimmed = super::trim_tail_silence(&pcm, 0.01, 30);
        assert_eq!(trimmed.len(), 60, "应裁掉尾部 40 个静音样本");
        // 全是声音不裁
        let full: Vec<f32> = vec![0.5; 100];
        assert_eq!(super::trim_tail_silence(&full, 0.01, 30).len(), 100);
    }

    #[test]
    fn split_never_cuts_mid_sentence() {
        // 有标点时：切分只在句末标点处切，不切句中
        let text = "第一句很长但有句号。第二句也很长也有句号。第三句结束。";
        let segs = super::split_for_local_tts(text, 20);
        for s in &segs {
            assert!(s.ends_with('。') || s.ends_with('？') || s.ends_with('！') || s.ends_with('\n'),
                "段 [{s}] 结尾不是句末标点");
        }
        // 无标点超长句：允许硬切（兜底），不 panic
        let long = "无标点长句".repeat(50);
        let segs2 = super::split_for_local_tts(&long, 30);
        assert!(!segs2.is_empty());
    }

    #[test]
    fn short_text_single_synth_call_threshold() {
        // SHORT_TEXT_CHARS = 1200；低于阈值的文本不分段
        let short = "你好".repeat(100); // 200 字
        assert!(short.chars().count() < super::SHORT_TEXT_CHARS);
        // 一次 split 应返回 1 段（除非原文有句末标点强制切，但 200 字无标点不会切）
        let segs = super::split_for_local_tts(&short, super::SHORT_TEXT_CHARS);
        assert_eq!(segs.len(), 1, "低于阈值应整段不分");
    }

    struct FakeSynth { delay_ms: std::time::Duration }
    impl super::SegmentSynth for FakeSynth {
        fn synth(&self, _text: &str) -> Result<Vec<f32>, String> {
            std::thread::sleep(self.delay_ms);
            Ok(vec![0.5; 4800]) // 0.1s @48k
        }
    }
    struct FakePlayer {
        delay_ms: std::time::Duration,
        opens: std::sync::atomic::AtomicUsize,
    }
    impl FakePlayer {
        fn new(d: std::time::Duration) -> Self {
            Self { delay_ms: d, opens: std::sync::atomic::AtomicUsize::new(0) }
        }
    }
    impl super::SegmentPlayer for FakePlayer {
        fn open(&mut self) -> Result<(), String> {
            self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn write(&mut self, _pcm: &[f32]) -> Result<(), String> {
            std::thread::sleep(self.delay_ms);
            Ok(())
        }
        fn close(&mut self) {}
        fn streams_opened(&self) -> usize {
            self.opens.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[test]
    fn gap_between_segments_below_threshold() {
        // 合成 50ms、播放 100ms；3 段。预取下相邻 gap 应 <100ms。
        let synth = std::sync::Arc::new(FakeSynth { delay_ms: std::time::Duration::from_millis(50) });
        let mut player = FakePlayer::new(std::time::Duration::from_millis(100));
        let segs: Vec<String> = (0..3).map(|i| format!("段{i}")).collect();
        let timings = super::speak_segments_prefetch(synth, &mut player, &segs).unwrap();
        assert_eq!(timings.len(), 3);
        let mut gaps = vec![];
        let mut max_gap = 0u64;
        for w in timings.windows(2) {
            let gap = w[1].play_start_ms.saturating_sub(w[0].play_end_ms);
            gaps.push(gap);
            max_gap = max_gap.max(gap);
        }
        eprintln!("[qibai-test] 3段 gaps={gaps:?} max_gap={max_gap}ms（注入延时测得，非真机音频）");
        assert!(max_gap < 100, "max_gap={max_gap}ms 应 <100ms");
        assert_eq!(player.streams_opened(), 1, "整段朗读只开一次播放会话");
    }

    #[test]
    fn prefetch_starts_before_prev_play_ends() {
        let synth = std::sync::Arc::new(FakeSynth { delay_ms: std::time::Duration::from_millis(80) });
        let mut player = FakePlayer::new(std::time::Duration::from_millis(80));
        let segs: Vec<String> = (0..3).map(|i| format!("段{i}")).collect();
        let timings = super::speak_segments_prefetch(synth, &mut player, &segs).unwrap();
        for w in timings.windows(2) {
            assert!(w[1].synth_start_ms <= w[0].play_end_ms + 50,
                "段{} synth_start={} 应早于段{} play_end={}", 1, w[1].synth_start_ms, 0, w[0].play_end_ms);
        }
    }

    #[test]
    fn gap_1000_chinese_chars_sample() {
        // 1000 字中文样本：40 句 × 25 字，按句号切 40 段（压出真实多段风险）
        let sentence = "元气小小是一只住在桌面的桌宠，它会在你工作时安静陪伴。";
        let segs: Vec<String> = std::iter::repeat(sentence.to_string()).take(40).collect();
        let total: usize = segs.iter().map(|s| s.chars().count()).sum();
        assert!(total >= 1000, "样本字数={total}");
        eprintln!("[qibai-test] 1000字样本 段数={} 总字数={total}", segs.len());
        let synth = std::sync::Arc::new(FakeSynth { delay_ms: std::time::Duration::from_millis(100) });
        let mut player = FakePlayer::new(std::time::Duration::from_millis(60));
        let timings = super::speak_segments_prefetch(synth, &mut player, &segs).unwrap();
        let mut gaps = vec![];
        for w in timings.windows(2) {
            gaps.push(w[1].play_start_ms.saturating_sub(w[0].play_end_ms));
        }
        gaps.sort();
        let max_gap = *gaps.last().unwrap_or(&0);
        let median_gap = if gaps.is_empty() { 0 } else { gaps[gaps.len()/2] };
        eprintln!("[qibai-test] 1000字 段数={} gaps_len={} max_gap={max_gap}ms median_gap={median_gap}ms streams={}（注入延时测得，非真机音频）",
            timings.len(), gaps.len(), player.streams_opened());
        assert!(max_gap < 100, "1000字样本 max_gap={max_gap}ms 应 <100ms");
        assert_eq!(player.streams_opened(), 1);
    }
}
