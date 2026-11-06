//! 按住说话（PTT）短采集：Rust cpal 直采麦克风 → 16k 单声道 WAV base64 → 复用 stt_transcribe。
//! 根治 WebView getUserMedia 需要系统麦克风权限的问题（Linux WebKitGTK 无公开放行 API）。
//!
//! v45.3 加固：所有 Mutex 锁点改用 `lock().unwrap_or_else(|e| e.into_inner())` 吞掉
//! poison——此前音频回调线程里一次 panic 会 poison 整个 Mutex，下次 lock().unwrap()
//! 在 ALSA 实时回调里再次 panic → SIGABRT，表现为「按住麦克风进程直接退出」。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;

/// v45.8：设备丢失标志（拔麦/采集流错误时置位，ptt_stop 清空旧录音返回 Err）。
static DEVICE_LOST: AtomicBool = AtomicBool::new(false);
/// v45.8：是否正在采集（err_fn 置 false，防止旧帧继续灌入 samples）。
static COLLECTING: AtomicBool = AtomicBool::new(false);

/// poison-tolerant lock：锁被毒化时取 poisoned guard 的内部值，绝不二次 panic。
#[inline]
fn lock_poison_tolerant<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct PttHandle {
    pub stop_tx: mpsc::Sender<()>,
    pub samples: Arc<Mutex<Vec<f32>>>,
    pub rate: Arc<Mutex<u32>>,
}

static PTT: Mutex<Option<PttHandle>> = Mutex::new(None);

/// 启动 PTT 短采集（幂等：已启动则直接 ok）。
pub fn ptt_start() -> Result<(), String> {
    // v45.8：每次启动前复位设备丢失标志
    DEVICE_LOST.store(false, Ordering::SeqCst);
    {
        let g = lock_poison_tolerant(&PTT);
        if g.is_some() {
            return Ok(()); // 已在采集
        }
    }
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "无默认输入设备（PTT）".to_string())?;
    let dev_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    eprintln!("[qibai] ptt: start on device <{dev_name}>");
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let rate = Arc::new(Mutex::new(0u32));
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let samples_in = samples.clone();
    let rate_in = rate.clone();
    let dev_name_in = dev_name.clone();

    // 线程内构建流并保持活跃，收到 stop 才 drop（否则 Stream drop 立即停止采集）。
    std::thread::spawn(move || {
        // 候选配置：设备默认配置优先，其后 16k/44.1k/48k 单声道 F32/I16。
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
        for rate_hz in [16000u32, 44100, 48000] {
            for fmt in [cpal::SampleFormat::F32, cpal::SampleFormat::I16] {
                let sc = cpal::StreamConfig {
                    channels: 1,
                    sample_rate: cpal::SampleRate(rate_hz),
                    buffer_size: cpal::BufferSize::Default,
                };
                let dup = cfgs
                    .iter()
                    .any(|(c, f)| c.channels == 1 && c.sample_rate.0 == rate_hz && *f == fmt);
                if !dup {
                    cfgs.push((sc, fmt));
                }
            }
        }
        for (sc, fmt) in cfgs {
            let smp = samples_in.clone();
            let rt = rate_in.clone();
            let dev_name_for_err = dev_name_in.clone();
            // v45.8：采集流报错 → 置 DEVICE_LOST，ptt_stop 据此清空旧录音并返回 Err
            let err_fn = move |e: cpal::StreamError| {
                eprintln!("[qibai] ptt: device lost (<{dev_name_for_err}>): {e}");
                DEVICE_LOST.store(true, Ordering::SeqCst);
                COLLECTING.store(false, Ordering::SeqCst);
            };
            let ch = sc.channels as usize;
            let hz = sc.sample_rate.0;
            let r = match fmt {
                cpal::SampleFormat::F32 => {
                    let smp2 = smp.clone();
                    device
                        .build_input_stream(
                            &sc,
                            move |data: &[f32], _| {
                                // v45.3：音频回调内绝不 panic——毒锁也吞掉。
                                let mut g = lock_poison_tolerant(&smp2);
                                if ch == 1 {
                                    g.extend_from_slice(data);
                                } else {
                                    for v in data.iter().step_by(ch) {
                                        g.push(*v);
                                    }
                                }
                                // 上限 20s，防长按内存膨胀
                                let max = 16000 * 20;
                                if g.len() > max {
                                    let drop_n = g.len() - max;
                                    g.drain(..drop_n);
                                }
                            },
                            err_fn,
                            None,
                        )
                        .map(|s| (s, hz))
                }
                cpal::SampleFormat::I16 => {
                    let smp2 = smp.clone();
                    device
                        .build_input_stream(
                            &sc,
                            move |data: &[i16], _| {
                                let mut g = lock_poison_tolerant(&smp2);
                                let f: Vec<f32> =
                                    data.iter().map(|&v| v as f32 / 32768.0).collect();
                                if ch == 1 {
                                    g.extend_from_slice(&f);
                                } else {
                                    for v in f.iter().step_by(ch) {
                                        g.push(*v);
                                    }
                                }
                                let max = 16000 * 20;
                                if g.len() > max {
                                    let drop_n = g.len() - max;
                                    g.drain(..drop_n);
                                }
                            },
                            err_fn,
                            None,
                        )
                        .map(|s| (s, hz))
                }
                other => {
                    eprintln!("[qibai] ptt: 跳过不支持的格式 {other:?}");
                    continue;
                }
            };
            match r {
                Ok((stream, hz)) => {
                    *lock_poison_tolerant(&rt) = hz;
                    COLLECTING.store(true, Ordering::SeqCst);
                    if let Err(e) = stream.play() {
                        eprintln!("[qibai] ptt: stream play 失败: {e}");
                        return;
                    }
                    let _ = stop_rx.recv();
                    drop(stream);
                    return;
                }
                Err(e) => eprintln!(
                    "[qibai] ptt: {}Hz/{}ch/{:?} 打开失败: {e}",
                    sc.sample_rate.0, sc.channels, fmt
                ),
            }
        }
        eprintln!("[qibai] ptt: 所有候选配置均失败");
    });

    *lock_poison_tolerant(&PTT) = Some(PttHandle {
        stop_tx,
        samples,
        rate,
    });
    Ok(())
}

/// 停止 PTT 采集，返回 16k 单声道 WAV 的 base64（stt_transcribe 直接吃）。
/// 不足 0.1s 返回空串（前端提示再说一次）。
pub fn ptt_stop() -> Result<String, String> {
    let handle = lock_poison_tolerant(&PTT)
        .take()
        .ok_or_else(|| "PTT 未启动".to_string())?;
    let _ = handle.stop_tx.send(());
    // 等采集线程退出（150ms 足够，线程收到 stop 立即 drop stream）
    std::thread::sleep(std::time::Duration::from_millis(150));
    // v45.8：设备丢失或采集已停 → 清空旧录音，返回 Err（前端不送转写）
    if DEVICE_LOST.load(Ordering::SeqCst) || !COLLECTING.load(Ordering::SeqCst) {
        // 清空 samples，避免下一次复用旧帧
        lock_poison_tolerant(&handle.samples).clear();
        COLLECTING.store(false, Ordering::SeqCst);
        return Err("麦克风已断开（可能被拔下），这次录音作废".to_string());
    }
    let src = std::mem::take(&mut *lock_poison_tolerant(&handle.samples));
    let from = *lock_poison_tolerant(&handle.rate);
    if src.len() < 1600 || from == 0 {
        // <0.1s：几乎没采到
        return Ok(String::new());
    }
    let pcm = resample_16k(&src, from);
    if pcm.is_empty() {
        return Ok(String::new());
    }
    Ok(wav_b64(&pcm, 16000))
}

/// 线性插值重采样到 16k，输出 PCM16。
fn resample_16k(src: &[f32], from: u32) -> Vec<i16> {
    const TARGET: u32 = 16000;
    if from == TARGET {
        return src
            .iter()
            .map(|&v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
    }
    let out_len = (src.len() as f64 * TARGET as f64 / from as f64).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * from as f64 / TARGET as f64;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(src.len().saturating_sub(1));
        let frac = (pos - i0 as f64) as f32;
        let v = src[i0] * (1.0 - frac) + src[i1] * frac;
        out.push((v.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    out
}

/// PCM16 单声道 → WAV（44 字节头）→ base64。
fn wav_b64(pcm: &[i16], rate: u32) -> String {
    let data_len = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(wav)
}
