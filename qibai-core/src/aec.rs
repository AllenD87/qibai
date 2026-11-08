//! P0「回声根治工程」—— 信号级声学回声消除引擎。
//!
//! 背景：桌宠「元气小小」双工缺陷 —— 播放 TTS 时麦克风能听到自己回音。
//! 方案：WebRTC AEC3（[`sonora`] crate，纯 Rust 移植，Send+Sync），
//! 参考信号 = 播放的 TTS 音频（render），采集 = 麦克风帧（capture）。
//!
//! 接线契约（集成小组按此接线，本文件公开 API 逐字冻结）：
//! - 播放线程：[`ingest_reference`] / [`ingest_reference_wav`] / [`reset_reference`]
//! - 采集线程：[`process_frame`]（20ms 帧）
//! - A/B 开关：环境变量 `QIBAI_AEC=0` → [`aec_enabled`] 返回 false →
//!   [`process_frame`] 原样直通，集成侧据此回退旧半双工硬逻辑。
//!
//! 关键决策：
//! - **帧长**：sonora AEC3 固定 10ms 帧（16k 单声道 = 160 样本）。两端各自攒帧。
//! - **参考重采样**：参考率不固定（云端 24k / espeak-piper ≈22.05k / 未知 48k），
//!   用 [`rubato::FastFixedIn`]（异步多项式重采样，内部缓冲跨调用连续）统一降到 16k；
//!   参考率变化时重建实例并丢弃旧残留。16k 输入直通不经过重采样器。
//! - **采集重采样**：采集侧 rate≠16k 时做线性插值（与 listener.rs 同款思路），
//!   每帧独立插值、块边界可接受 1 样本跳变（对 AEC3 自适应时间尺度可忽略）。
//! - **延迟 hint**：构造后 `set_stream_delay_ms(40)`（播放→采集典型 30~50ms），
//!   之后完全交给 AEC3 内部估计（`statistics().delay_median_ms`），**不硬编码固定延迟线**。
//! - **回声检测**：`echo_detector(true)` 开启残余回声检测，供 barge-in 判定。
//! - **NS/AGC2**：开启 AEC3 + HPF + NS(Low)。AGC2 默认关闭 —— 它会改变信号能量，
//!   对打断判定与能量型测试不稳定；需要时在 `sonora::config::Config` 里开启。
//! - **锁**：播放/采集两线程经 `Mutex` 共享同一引擎（sonora 已实测 Send+Sync）；
//!   持锁失败（中毒）一律按未启用处理，绝不 panic 音频链路。

use std::sync::{Mutex, OnceLock};

use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use sonora::config::{EchoCanceller, HighPassFilter, NoiseSuppression, NoiseSuppressionLevel};
use sonora::{AudioProcessing, Config, StreamConfig};

/// AEC 内部固定处理参数：16k 单声道、10ms 帧。
const PROC_SAMPLE_RATE: u32 = 16_000;
/// sonora 每帧样本数 = 16000 / 100 = 160。
const FRAME: usize = 160;
/// 延迟 hint（播放→采集典型延时 30~50ms，取中值）。
const STREAM_DELAY_MS: i32 = 40;
/// 参考重采样器每次消费的输入帧数（≈10ms 参考音频，控制路径延迟）。
const REF_CHUNK: usize = 160;

// ---------------------------------------------------------------- 全局单例

/// 全局引擎单例。`None` 表示环境开关关闭或引擎初始化失败。
/// 写法对齐 tts.rs:158 的 `QUEUE_TX: OnceLock<Mutex<Option<...>>>` 模式。
static AEC: OnceLock<Mutex<Option<AecEngine>>> = OnceLock::new();

fn engine_cell() -> &'static Mutex<Option<AecEngine>> {
    AEC.get_or_init(|| Mutex::new(AecEngine::new()))
}

/// A/B 开关：环境变量 `QIBAI_AEC` 非 `"0"` 时为 true；
/// 引擎初始化失败（None）或锁中毒也一律返回 false（回退旧半双工逻辑）。
/// 每次调用实时读环境变量，不缓存 —— 允许运行期开关。
pub fn aec_enabled() -> bool {
    if std::env::var("QIBAI_AEC").as_deref() == Ok("0") {
        return false;
    }
    match engine_cell().lock() {
        Ok(g) => g.is_some(),
        Err(_) => false,
    }
}

/// 播放线程：喂入一段参考音频（播放的 TTS 音频）。
///
/// 入参为**交错** f32（[-1,1]）；内部 downmix 单声道 → 重采样 16k →
/// 切 160 样本帧喂 sonora render。任意块长均可。
pub fn ingest_reference(samples: &[f32], sample_rate: u32, channels: u16) {
    let Ok(mut g) = engine_cell().lock() else {
        return;
    };
    if let Some(eng) = g.as_mut() {
        eng.ingest(samples, sample_rate, channels);
    }
}

/// 播放线程：喂入一段 WAV 字节（rodio::Decoder 解码），解码失败静默丢弃。
/// 解码后转 [`ingest_reference`]。
pub fn ingest_reference_wav(bytes: &[u8]) {
    // rodio::Decoder::new 要求 reader: 'static（与 tts.rs 一致用 owned Vec）。
    // espeak-ng --stdout 产出的 WAV 把 RIFF/data 长度写成 ~2GB 占位值，
    // hound 会按声明长度无限读下去，先归一化 chunk 长度再解码。
    let mut owned = bytes.to_vec();
    normalize_wav_sizes(&mut owned);
    let cursor = std::io::Cursor::new(owned);
    let Ok(decoder) = rodio::Decoder::new(cursor) else {
        return;
    };
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    // rodio::Decoder 按源格式产出 i16 等样本，统一 convert 到 f32（与 tts.rs 同款）。
    use rodio::Source as _;
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    ingest_reference(&samples, rate, channels);
}

/// 修正 WAV 头部的 chunk 长度字段。
///
/// 背景：espeak-ng 写到管道/stdout 时无法回填长度，会把 RIFF size 和 data size
/// 写成 0x7fffff00（~2GB）占位值。rodio(hound) 忠实按声明长度迭代样本，
/// 到达文件真实 EOF 后仍不停（错误被 `.unwrap_or(0)` 吞成 0.0）→ 无限挂死。
/// 这里遍历 chunk，把 data chunk 的 size 钳到文件实际剩余长度，同时回填 RIFF size。
/// 非 WAV 或格式正常的文件原样返回，零副作用。
pub(crate) fn normalize_wav_sizes(bytes: &mut Vec<u8>) {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return;
    }
    let file_len = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&(file_len - 8).to_le_bytes());
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &bytes[pos..pos + 4] == b"data" {
            let actual = (bytes.len() - pos - 8) as u32;
            bytes[pos + 4..pos + 8].copy_from_slice(&actual.to_le_bytes());
            return;
        }
        let next = pos + 8 + size + (size % 2);
        if next > bytes.len() {
            break;
        }
        pos = next;
    }
}

/// 播放停止/打断时清参考缓冲（Job::Stop 场景）。
/// 只清缓冲、不重建引擎实例（重建代价大且会丢 AEC 收敛状态）。
pub fn reset_reference() {
    if let Ok(mut g) = engine_cell().lock() {
        if let Some(eng) = g.as_mut() {
            eng.reset();
        }
    }
}

/// 采集线程：处理一帧麦克风音频（单声道 f32，任意块长）。
///
/// 行为：
/// - `QIBAI_AEC=0`（或引擎未启用）→ **直通**：frame 原样不动，返回原 rate；
/// - 启用 → 内部攒 160 样本@16k 帧喂 sonora capture，残差 16k 覆盖 frame，返回 16000。
/// 输出长度 = floor(本帧重采样后总长 / 160) * 160，不足一整帧的尾巴留到下一帧。
pub fn process_frame(frame: &mut Vec<f32>, rate: u32) -> u32 {
    // A/B 直通：未启用时原样返回，不处理。
    if !aec_enabled() {
        return rate;
    }
    let Ok(mut g) = engine_cell().lock() else {
        return rate;
    };
    match g.as_mut() {
        Some(eng) => eng.process(frame, rate),
        None => rate,
    }
}

// ---------------------------------------------------------------- 引擎本体

struct AecEngine {
    ap: AudioProcessing,
    /// 参考信号 16k 待切帧缓冲（不足 160 的尾巴留到下一次）。
    render_pending: Vec<f32>,
    /// 采集信号 16k 待切帧缓冲。
    capture_pending: Vec<f32>,
    /// sonora render 输出暂存（每次恰好 160）。
    render_out: [f32; FRAME],
    /// 当前参考重采样器及其输入采样率（rate 变化时重建）。
    ref_resampler: Option<(FastFixedIn<f32>, u32)>,
    /// 重采样器输入残留（native rate mono，不足 REF_CHUNK 不消费）。
    ref_in: Vec<f32>,
}

impl AecEngine {
    fn new() -> Option<Self> {
        // 配置：AEC3 + HPF + NS(Low)；AGC2 关闭（见模块注释）。
        let config = Config {
            echo_canceller: Some(EchoCanceller::default()),
            high_pass_filter: Some(HighPassFilter::default()),
            noise_suppression: Some(NoiseSuppression {
                level: NoiseSuppressionLevel::Low,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ap = AudioProcessing::builder()
            .config(config)
            .capture_config(StreamConfig::new(PROC_SAMPLE_RATE, 1))
            .render_config(StreamConfig::new(PROC_SAMPLE_RATE, 1))
            // 残余回声检测 → barge-in。
            .echo_detector(true)
            .build();
        // 延迟 hint：30~50ms 取中值 40ms；之后交给 AEC3 自估。
        let _ = ap.set_stream_delay_ms(STREAM_DELAY_MS);
        Some(Self {
            ap,
            render_pending: Vec::with_capacity(FRAME * 4),
            capture_pending: Vec::with_capacity(FRAME * 4),
            render_out: [0.0; FRAME],
            ref_resampler: None,
            ref_in: Vec::with_capacity(REF_CHUNK * 4),
        })
    }

    /// 播放线程入口：交错 f32 → mono → 16k → 160 帧喂 render。
    fn ingest(&mut self, samples: &[f32], sample_rate: u32, channels: u16) {
        if samples.is_empty() || sample_rate == 0 || channels == 0 {
            return;
        }
        // 1. 交错 downmix → mono。
        let channels = channels as usize;
        let mono: Vec<f32> = if channels == 1 {
            samples.to_vec()
        } else {
            samples
                .chunks(channels)
                .map(|c| c.iter().sum::<f32>() / channels as f32)
                .collect()
        };
        // 2. 重采样到 16k（16k 直通）。
        if sample_rate == PROC_SAMPLE_RATE {
            self.render_pending.extend_from_slice(&mono);
        } else {
            self.push_reference_resampled(&mono, sample_rate);
        }
        // 3. 切 160 样本帧喂 sonora render。
        while self.render_pending.len() >= FRAME {
            let frame: Vec<f32> = self.render_pending.drain(..FRAME).collect();
            let _ = self
                .ap
                .process_render_f32(&[&frame], &mut [&mut self.render_out[..]]);
        }
    }

    /// 参考信号异速率重采样：FastFixedIn 每次固定消费 REF_CHUNK 个输入帧。
    /// 参考率变化 → 重建重采样器，丢弃旧残留（避免速率串扰）。
    fn push_reference_resampled(&mut self, mono: &[f32], rate: u32) {
        let need_new = match &self.ref_resampler {
            None => true,
            Some((_, r)) => *r != rate,
        };
        if need_new {
            let ratio = PROC_SAMPLE_RATE as f64 / rate as f64;
            self.ref_resampler = FastFixedIn::<f32>::new(
                ratio,
                10.0, // max_resample_ratio_relative：余量，固定比率其实用不到
                PolynomialDegree::Linear,
                REF_CHUNK,
                1,
            )
            .ok()
            .map(|r| (r, rate));
            self.ref_in.clear();
            if self.ref_resampler.is_none() {
                // 重采样器构造失败：放弃本段参考（参考缺失只会多残留回声，不会崩链路）。
                return;
            }
        }
        self.ref_in.extend_from_slice(mono);
        let Some((resampler, _)) = self.ref_resampler.as_mut() else {
            return;
        };
        while self.ref_in.len() >= REF_CHUNK {
            let block: Vec<f32> = self.ref_in.drain(..REF_CHUNK).collect();
            if let Ok(out) = resampler.process(&[&block], None) {
                if let Some(chan) = out.into_iter().next() {
                    self.render_pending.extend(chan);
                }
            }
        }
    }

    fn reset(&mut self) {
        self.render_pending.clear();
        self.capture_pending.clear();
        self.ref_in.clear();
    }

    /// 采集线程入口：单声道 f32 任意块长 → 16k → 160 帧喂 capture → 残差覆盖 frame。
    fn process(&mut self, frame: &mut Vec<f32>, rate: u32) -> u32 {
        // 1. 采集侧重采样到 16k（rate==16k 直通）。
        let resampled: Vec<f32> = if rate == PROC_SAMPLE_RATE {
            frame.clone()
        } else {
            linear_resample_mono(frame, rate, PROC_SAMPLE_RATE)
        };
        self.capture_pending.extend_from_slice(&resampled);
        // 2. 攒 160 样本帧喂 sonora capture，残差累积为本帧输出。
        let mut out: Vec<f32> = Vec::with_capacity(self.capture_pending.len());
        while self.capture_pending.len() >= FRAME {
            let block: Vec<f32> = self.capture_pending.drain(..FRAME).collect();
            let mut clean = [0.0f32; FRAME];
            let _ = self
                .ap
                .process_capture_f32(&[&block], &mut [&mut clean[..]]);
            out.extend_from_slice(&clean);
        }
        // 3. 残差 16k 覆盖 frame。
        *frame = out;
        PROC_SAMPLE_RATE
    }
}

/// 线性插值重采样（与 listener.rs 同款思路）：任意块长、无状态、逐块独立。
fn linear_resample_mono(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if input.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    let step = from as f64 / to as f64;
    let out_len = (input.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * step;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let frac = pos - i0 as f64;
        out.push(input[i0] * (1.0 - frac as f32) + input[i1] * frac as f32);
    }
    out
}

// ---------------------------------------------------------------- 单元测试

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn energy(xs: &[f32]) -> f64 {
        xs.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>()
    }

    /// 简单可重复伪随机噪声（无需 rand 种子状态机）。
    fn noise_at(i: usize, seed: u64) -> f32 {
        let mut x = (i as u64)
            .wrapping_mul(2862933555777941757)
            .wrapping_add(seed)
            .wrapping_mul(3266489917);
        x ^= x >> 13;
        ((x as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// 构造 WAV(i16 PCM mono) 字节 —— 手写 RIFF 头，供 rodio::Decoder 读。
    fn make_wav_i16(samples: &[f32], rate: u32) -> Vec<u8> {
        let pcm: Vec<u8> = samples
            .iter()
            .flat_map(|s| {
                let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                v.to_le_bytes().to_vec()
            })
            .collect();
        let mut out = Vec::with_capacity(44 + pcm.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&((rate as u32) * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(&pcm);
        out
    }

    /// 1. 合成回声链路收敛：回声路径 = 延迟 D + 衰减 0.4；
    ///    断言 ERLE ≥ 10dB，且近端段能量衰减 < 3dB。
    #[test]
    fn echo_link_converges() {
        let sr = PROC_SAMPLE_RATE as usize;
        let mut eng = AecEngine::new().expect("引擎构造");

        // 参考：1.5s 伪语音（低频噪声 × 慢调制包络）。
        let total = sr * 3 / 2;
        let far: Vec<f32> = (0..total)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let envelope = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * 5.0 * t).sin();
                noise_at(i, 0xA11) * envelope * 0.5
            })
            .collect();
        // 近端：后 0.5s 才出现的双音信号（与 far 完全不同）。
        let near: Vec<f32> = (0..total)
            .map(|i| {
                if i < sr {
                    0.0
                } else {
                    let t = i as f32 / sr as f32;
                    0.3 * ((2.0 * std::f32::consts::PI * 440.0 * t).sin()
                        + (2.0 * std::f32::consts::PI * 880.0 * t).sin())
                }
            })
            .collect();
        // 回声路径：延时 D=320 样本(20ms) + 衰减 0.4 叠加为采集信号。
        let delay = 320usize;
        let mic_raw: Vec<f32> = (0..total)
            .map(|i| {
                let echo = if i >= delay {
                    far[i - delay] * 0.4
                } else {
                    0.0
                };
                echo + near[i]
            })
            .collect();

        // 逐 160 样本块：先喂参考，再处理采集（模拟双工时序）。
        let mut mic_out: Vec<f32> = Vec::with_capacity(total);
        for chunk in mic_raw.chunks(FRAME) {
            let start = mic_out.len();
            let end = start + chunk.len();
            // 找到 far 对应块
            let far_chunk = &far[start.min(total - 1)..end.min(total)];
            eng.ingest(far_chunk, PROC_SAMPLE_RATE, 1);
            let mut c = chunk.to_vec();
            eng.process(&mut c, PROC_SAMPLE_RATE);
            mic_out.extend(c);
        }

        // ERLE：前 1.0s 无近端，mic 全是回声。
        let warmup = sr / 2; // 前 0.5s 暖机不计入
        let erle_in = energy(&mic_raw[warmup..sr]);
        let erle_out = energy(&mic_out[warmup..sr]);
        assert!(erle_in > 1e-9, "回声信号能量不能为 0");
        let erle_db = 10.0 * (erle_in / erle_out).log10();
        assert!(
            erle_db >= 10.0,
            "ERLE 不足 10dB：实测 {erle_db:.1} dB（in={erle_in:.3} out={erle_out:.3}）"
        );

        // 近端保留：后 0.5s 近端段，输出能量 vs 近端能量，衰减 < 3dB。
        let near_in = energy(&near[sr..]);
        let near_out = energy(&mic_out[sr..]);
        let near_db = 10.0 * (near_out / near_in).log10();
        assert!(
            near_db > -3.0,
            "近端被过度抑制：{near_db:.1} dB（near_in={near_in:.3} near_out={near_out:.3}）"
        );
    }

    /// 2. 块长鲁棒性：任意块长连续喂不 panic，输出总长正确。
    #[test]
    fn chunk_length_robustness() {
        let mut eng = AecEngine::new().expect("引擎构造");
        let src: Vec<f32> = (0..PROC_SAMPLE_RATE as usize * 2)
            .map(|i| (i as f32 * 0.001).sin())
            .collect();
        for &chunk in &[137usize, 160, 320, 1000] {
            let mut total_out = 0usize;
            for c in src.chunks(chunk) {
                let mut v = c.to_vec();
                let rate = eng.process(&mut v, PROC_SAMPLE_RATE);
                assert_eq!(rate, PROC_SAMPLE_RATE);
                // 输出只包含完整 160 帧，总长 = floor(len/160)*160 累积。
                assert_eq!(v.len() % FRAME, 0);
                total_out += v.len();
            }
            let expect = (src.len() / FRAME) * FRAME;
            assert_eq!(total_out, expect, "chunk={chunk} 输出总长错误");
        }
        // 48k 采集侧：960 样本块，线性插值重采样到 16k 再处理。
        let src48: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.001).sin()).collect();
        let mut total_out = 0usize;
        for c in src48.chunks(960) {
            let mut v = c.to_vec();
            let rate = eng.process(&mut v, 48_000);
            assert_eq!(rate, PROC_SAMPLE_RATE);
            assert_eq!(v.len() % FRAME, 0);
            total_out += v.len();
        }
        // 48k→16k 重采样后约 16000 样本，输出只含整帧，误差 ±160 内。
        assert!(total_out > 15_000 && total_out < 16_160, "48k 输出总长={total_out}");
    }

    /// 3. 多线程冒烟：Arc<Mutex> 下播放/采集并发各数百帧不 panic。
    #[test]
    fn multithread_smoke() {
        let eng = Arc::new(Mutex::new(AecEngine::new().expect("引擎构造")));
        let e1 = eng.clone();
        let t1 = std::thread::spawn(move || {
            for k in 0..300 {
                let block: Vec<f32> = (0..160)
                    .map(|i| noise_to_f32(k * 160 + i, 0xBEEF))
                    .collect();
                e1.lock().unwrap().ingest(&block, PROC_SAMPLE_RATE, 1);
            }
        });
        let e2 = eng.clone();
        let t2 = std::thread::spawn(move || {
            for k in 0..300 {
                let mut block: Vec<f32> = (0..160)
                    .map(|i| noise_to_f32(k * 160 + i, 0xC0FE))
                    .collect();
                e2.lock().unwrap().process(&mut block, PROC_SAMPLE_RATE);
            }
        });
        t1.join().expect("播放线程");
        t2.join().expect("采集线程");
    }

    fn noise_to_f32(i: usize, seed: u64) -> f32 {
        noise_at(i, seed)
    }

    /// 4. A/B 直通：QIBAI_AEC=0 时 process_frame 原样直通、aec_enabled()==false。
    ///    env 是进程级的，主测试进程里 set_var 会污染并行用例，
    ///    因此用 current_exe 子进程重入做隔离断言。
    #[test]
    fn ab_pass_through() {
        if std::env::var("QIBAI_AEC_TEST_MARK").is_ok() {
            // 子进程内：父进程已注入 QIBAI_AEC=0。
            assert_eq!(std::env::var("QIBAI_AEC").as_deref(), Ok("0"));
            assert!(!aec_enabled());
            let mut frame = vec![0.123_456f32; 320];
            let expect = frame.clone();
            let rate = process_frame(&mut frame, 48_000);
            assert_eq!(rate, 48_000, "未启用时必须返回原 rate");
            assert_eq!(frame, expect, "未启用时 frame 必须原样不动");
            return;
        }
        let exe = std::env::current_exe().expect("current_exe");
        let status = std::process::Command::new(exe)
            .arg("--exact")
            .arg("aec::tests::ab_pass_through")
            .env("QIBAI_AEC_TEST_MARK", "1")
            .env("QIBAI_AEC", "0")
            .output()
            .expect("启动子进程");
        assert!(
            status.status.success(),
            "子进程断言失败: stdout={}\nstderr={}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// 5. ingest_reference_wav：构造 24k 单声道 wav 字节喂入，不 panic。
    #[test]
    fn ingest_reference_wav_smoke() {
        let samples: Vec<f32> = (0..24_000)
            .map(|i| (i as f32 / 24_000.0 * 440.0 * std::f32::consts::PI * 2.0).sin() * 0.3)
            .collect();
        let wav = make_wav_i16(&samples, 24_000);
        // 全局函数：解码失败/喂入均不得 panic。
        ingest_reference_wav(&wav);
        reset_reference();
        // 顺便确认坏字节静默。
        ingest_reference_wav(&[0u8; 16]);

        // 回归：espeak-ng --stdout 的 WAV 把 data/RIFF 长度写成 ~2GB 占位值，
        // hound 会按声明长度死读到天荒地老 —— normalize_wav_sizes 必须先修正。
        let mut bogus = make_wav_i16(&samples, 22_050);
        bogus[4..8].copy_from_slice(&0x7fff_fff0u32.to_le_bytes()); // RIFF size 占位
        if let Some(pos) = bogus.windows(4).position(|w| w == b"data".as_slice()) {
            bogus[pos + 4..pos + 8].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());
        }
        ingest_reference_wav(&bogus);
        reset_reference();
    }
}
