//! v45.9：PulseAudio/PipeWire sink 枚举与常驻播放。
//!
//! Ubuntu 桌面下 ALSA 只暴露 default/null，USB/HDMI/蓝牙都在 Pulse 层。
//! 本模块：
//! - `list_sinks()`：跑 `pactl list sinks` 解析 Name/Description/State；
//! - 与 cpal/ALSA 枚举合并（同名去重，`default` 永远第一条）；
//! - `pacat --playback --device=<sink>` 常驻子进程，喂 s16le/48k/2ch PCM。

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

/// 一个输出设备条目。
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputDevice {
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub source: &'static str, // "pulse" | "alsa" | "builtin_default"
}

/// 解析 `pactl list sinks` 输出。
pub fn parse_pactl_sinks(stdout: &str) -> Vec<OutputDevice> {
    let mut out = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_desc: Option<String> = None;
    for line in stdout.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Name: ") {
            cur_name = Some(rest.to_string());
            cur_desc = None;
        } else if let Some(rest) = l.strip_prefix("Description: ") {
            cur_desc = Some(rest.to_string());
        } else if l.starts_with("State: ") {
            if let (Some(n), Some(d)) = (cur_name.take(), cur_desc.take()) {
                out.push(OutputDevice {
                    description: d,
                    name: n,
                    is_default: false,
                    source: "pulse",
                });
            }
        }
    }
    // 处理结尾（最后一个 sink 没遇到新 State 行也保留）
    if let (Some(n), Some(d)) = (cur_name.take(), cur_desc.take()) {
        out.push(OutputDevice {
            name: n,
            description: d,
            is_default: false,
            source: "pulse",
        });
    }
    out
}

/// 跑 pactl，失败返回空 Vec（不 panic）。
fn run_pactl() -> Vec<OutputDevice> {
    let Ok(output) = Command::new("pactl").arg("list").arg("sinks").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_pactl_sinks(&String::from_utf8_lossy(&output.stdout))
}

/// 枚举 cpal/ALSA 设备（hw:CARD=... 这类）。
fn run_alsa() -> Vec<OutputDevice> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let Ok(devs) = host.output_devices() else { return Vec::new() };
    devs.filter_map(|d| {
        let name = d.name().ok()?;
        // 跳过默认/无效名，pactl 已覆盖
        if name.is_empty() || name == "default" || name == "default / dmix" || name.contains("Dummy") {
            return None;
        }
        Some(OutputDevice {
            description: name.clone(),
            name,
            is_default: false,
            source: "alsa",
        })
    })
    .collect()
}

/// 合并：default 第一条，pactl 优先，alsa 补 pactl 未覆盖的同名去重。
pub fn list_outputs() -> Vec<OutputDevice> {
    let pulse = run_pactl();
    let alsa = run_alsa();
    eprintln!(
        "[qibai] audio: sinks pactl={} alsa={}",
        pulse.len(),
        alsa.len()
    );
    let mut out = vec![OutputDevice {
        name: "default".to_string(),
        description: "系统默认".to_string(),
        is_default: true,
        source: "builtin_default",
    }];
    let mut seen = std::collections::HashSet::new();
    for d in pulse.into_iter().chain(alsa.into_iter()) {
        if seen.insert(d.name.clone()) {
            out.push(d);
        }
    }
    out
}

/// 线性重采样：任意输入 sample_rate → 48000（f32）。
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = ((samples.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let src_idx = (i as f64) / ratio;
        let i0 = src_idx.floor() as usize;
        let frac = src_idx - i0 as f64;
        let s0 = samples.get(i0).copied().unwrap_or(0.0);
        let s1 = samples.get(i0 + 1).copied().unwrap_or(s0);
        out.push((s0 * (1.0 - frac as f32)) + (s1 * frac as f32));
    }
    out
}

/// f32 mono → s16le 48k 2ch 交织（每帧 = 4 字节：左 s16 + 右 s16）。
pub fn f32_mono_to_s16le_48k_stereo(pcm: &[f32], from_rate: u32) -> Vec<u8> {
    let resampled = resample_linear(pcm, from_rate, 48000);
    let mut out = Vec::with_capacity(resampled.len() * 4);
    for s in resampled {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes()); // left
        out.extend_from_slice(&v.to_le_bytes()); // right (mono → 双声道复制)
    }
    out
}

/// 后端选择：阶梯探测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Pacat,
    PwPlay,
    Paplay,
    RodioAlsa,
    RodioDefault,
}

pub fn pick_backend(device_name: &str) -> Backend {
    if device_name.is_empty() || device_name == "default" {
        return Backend::RodioDefault;
    }
    if device_name.starts_with("hw:") {
        return Backend::RodioAlsa;
    }
    if device_name.starts_with("alsa_output.")
        || device_name.starts_with("bluez_output.")
        || device_name.starts_with("bluez_sink.")
    {
        // 实际选哪个由 probe_backend 决定，这里先返回首选
        return Backend::Pacat;
    }
    Backend::RodioDefault
}

/// 探测单个工具是否可用：`Command::new(tool).arg("--version").status()` 判存在。
pub fn tool_exists(tool: &str) -> bool {
    Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct ProbeResult {
    pub pacat: bool,
    pub pw_play: bool,
    pub paplay: bool,
    pub rodio: bool,
}

pub fn probe_tools() -> ProbeResult {
    let r = ProbeResult {
        pacat: tool_exists("pacat"),
        pw_play: tool_exists("pw-play"),
        paplay: tool_exists("paplay"),
        rodio: true, // rodio 是库，永远 "可用"（开 stream 才可能失败）
    };
    eprintln!("[qibai] audio: probe pacat={} pw-play={} paplay={} rodio=ok",
        if r.pacat {"ok"} else {"missing"},
        if r.pw_play {"ok"} else {"missing"},
        if r.paplay {"ok"} else {"missing"},
    );
    r
}

/// 阶梯选后端：返回最终 backend + 选中的 device。
/// 调用方负责：选 Pacat 走 PacatPlayer；PwPlay/Paplay 落临时 wav 喂；Rodio 时 set_var PULSE_SINK。
pub fn choose_backend(device_name: &str, tools: &ProbeResult) -> Backend {
    if device_name.is_empty() || device_name == "default" {
        return Backend::RodioDefault;
    }
    if device_name.starts_with("hw:") {
        return Backend::RodioAlsa;
    }
    // pulse sink：按优先级探测
    if tools.pacat { return Backend::Pacat; }
    if tools.pw_play { return Backend::PwPlay; }
    if tools.paplay { return Backend::Paplay; }
    // 全无 → rodio + PULSE_SINK 定向
    Backend::RodioDefault
}

/// 解析 `pactl list sink-inputs`：找到 application.name 属于本进程的流，返回 Sink index。
pub fn parse_sink_landing(stdout: &str) -> Option<u32> {
    let mut cur_sink: Option<u32> = None;
    let mut matched = false;
    for line in stdout.lines() {
        let l = line.trim();
        if l.starts_with("Sink #") {
            if matched { return cur_sink; }
            cur_sink = None;
            matched = false;
        } else if let Some(rest) = l.strip_prefix("Sink: ") {
            cur_sink = rest.parse().ok();
        } else if let Some(rest) = l.strip_prefix("application.name = ") {
            let name = rest.trim_matches('"');
            if name.contains("qibai") || name.contains("pacat") || name.contains("pw-play") || name.contains("paplay") {
                matched = true;
            }
        }
    }
    if matched { cur_sink } else { None }
}

/// 跑 `pactl list sinks short` 把 index 映射成 name。
pub fn sink_index_to_name(index: u32) -> Option<String> {
    let out = Command::new("pactl").args(["list", "sinks", "short"]).output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let mut it = line.split('\t');
        let idx: u32 = it.next()?.parse().ok()?;
        if idx == index {
            return it.next().map(|x| x.to_string());
        }
    }
    None
}

/// 反向：name → index（pactl list sinks short: 每行 `index\tname\t...`）。
pub fn sink_name_to_index(name: &str) -> Option<u32> {
    let out = Command::new("pactl").args(["list", "sinks", "short"]).output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    parse_sink_name_to_index(&s, name)
}

/// 纯函数：给 `pactl list sinks short` 输出 + name，返回 index（可单测）。
pub fn parse_sink_name_to_index(stdout: &str, name: &str) -> Option<u32> {
    for line in stdout.lines() {
        let mut it = line.split('\t');
        let idx: u32 = it.next()?.parse().ok()?;
        let n = it.next()?;
        if n == name { return Some(idx); }
    }
    None
}

/// 构造 pw-play / paplay 的命令参数（sink 含空格/中文/冒号都走 arg 数组不被拆）。
pub fn file_backend_args(backend: Backend, sink: &str, tmp_wav: &std::path::Path) -> Vec<std::ffi::OsString> {
    match backend {
        Backend::PwPlay => vec![
            "--target".into(), sink.into(), tmp_wav.as_os_str().to_owned(),
        ],
        Backend::Paplay => vec![
            "--device".into(), sink.into(), tmp_wav.as_os_str().to_owned(),
        ],
        _ => vec![],
    }
}

/// rodio 兜底时该设的 PULSE_SINK env（None 表示应 remove）。
pub fn rodio_env_override(device: &str) -> Option<(&'static str, String)> {
    if device.is_empty() || device == "default" {
        None
    } else {
        Some(("PULSE_SINK", device.to_string()))
    }
}

/// 1 秒 440Hz 正弦 wav（48k mono s16le，rodio 可直接播）。
pub fn make_test_tone_wav() -> Vec<u8> {
    let sample_rate: u32 = 48000;
    let duration_secs: u32 = 1;
    let n = sample_rate * duration_secs;
    let mut pcm: Vec<u8> = Vec::with_capacity((n as usize) * 2);
    for i in 0..n {
        let t = i as f32 / sample_rate as f32;
        let v = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3;
        let s = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    // WAV header
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

/// 常驻 pacat 子进程包装。
pub struct PacatPlayer {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    sink: String,
}

impl PacatPlayer {
    /// 启动 pacat --playback --device=<sink>。失败返回 Err（调用方回落 rodio）。
    pub fn start(sink: &str) -> Result<Self, String> {
        let mut child = Command::new("pacat")
            .arg("--playback")
            .arg(format!("--device={sink}"))
            .arg("--rate=48000")
            .arg("--channels=2")
            .arg("--format=s16le")
            .arg("--latency-msec=80")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("pacat 启动失败：{e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| "pacat stdin 不可用".to_string())?;
        eprintln!("[qibai] audio: pacat started sink={sink}");
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            sink: sink.to_string(),
        })
    }

    /// 喂一段已转 s16le/48k/2ch 的 PCM。管道断了就返回 Err。
    pub fn write(&mut self, pcm_s16le: &[u8]) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("pacat 已关闭")?;
        stdin
            .write_all(pcm_s16le)
            .map_err(|e| format!("pacat 写失败：{e}"))
    }

    /// 关进程。
    pub fn stop(mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn sink(&self) -> &str {
        &self.sink
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pactl_two_sinks() {
        let fixture = r#"Sink #0
        Name: alsa_output.pci-0000_00_1f.3.analog-stereo
        Description: 内置音频模拟立体声
        State: RUNNING

Sink #1
        Name: bluez_sink.AA_BB_CC_DD_EE_FF.a2dp_sink
        Description: Bluetooth Headphones
        State: SUSPENDED
"#;
        let sinks = parse_pactl_sinks(fixture);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].name, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(sinks[0].description, "内置音频模拟立体声");
        assert_eq!(sinks[1].name, "bluez_sink.AA_BB_CC_DD_EE_FF.a2dp_sink");
        assert_eq!(sinks[1].source, "pulse");
    }

    #[test]
    fn merge_default_first_and_dedup() {
        let pulse = vec![
            OutputDevice { name: "pulse-a".into(), description: "PA".into(), is_default: false, source: "pulse" },
            OutputDevice { name: "hw:CARD=USB".into(), description: "USB".into(), is_default: false, source: "pulse" },
        ];
        let alsa = vec![
            OutputDevice { name: "hw:CARD=USB".into(), description: "USB".into(), is_default: false, source: "alsa" },
            OutputDevice { name: "hw:CARD=HDMI".into(), description: "HDMI".into(), is_default: false, source: "alsa" },
        ];
        // 手动合并逻辑测试：default 第一条，hw:CARD=USB 去重
        let mut out = vec![OutputDevice { name: "default".into(), description: "系统默认".into(), is_default: true, source: "builtin_default" }];
        let mut seen = std::collections::HashSet::new();
        for d in pulse.into_iter().chain(alsa.into_iter()) {
            if seen.insert(d.name.clone()) { out.push(d); }
        }
        assert_eq!(out[0].name, "default");
        assert_eq!(out[1].name, "pulse-a");
        assert_eq!(out[2].name, "hw:CARD=USB");
        assert_eq!(out[3].name, "hw:CARD=HDMI");
        assert_eq!(out.len(), 4, "hw:CARD=USB 应去重");
    }

    #[test]
    fn f32_mono_to_s16le_48k_stereo_length() {
        // 100 帧 @48k 输入 → 100 帧输出，每帧 4 字节
        let pcm: Vec<f32> = vec![0.0; 100];
        let bytes = f32_mono_to_s16le_48k_stereo(&pcm, 48000);
        assert_eq!(bytes.len(), 100 * 4);
        // 交织：前 4 字节 = left(0) + right(0) 全 0
        assert_eq!(&bytes[..4], &[0, 0, 0, 0]);
        // 非零输入
        let pcm2 = vec![1.0];
        let b2 = f32_mono_to_s16le_48k_stereo(&pcm2, 48000);
        assert_eq!(b2.len(), 4);
        assert_eq!(&b2, &[0xff, 0x7f, 0xff, 0x7f]); // i16 32767 = 0x7fff
    }

    #[test]
    fn resample_44100_to_48000_proportional() {
        let pcm: Vec<f32> = (0..44100).map(|i| (i as f32 / 44100.0).sin()).collect();
        let up = resample_linear(&pcm, 44100, 48000);
        // 44100 → 48000，输出应 ≈ 48000 帧
        assert!((up.len() as i64 - 48000).abs() <= 2, "实际 {}", up.len());
    }

    #[test]
    fn pactl_missing_returns_empty_not_panic() {
        // 这里只测 parse 空 fixture，真实 pactl 不存在由 run_pactl 内部静默吞掉
        let sinks = parse_pactl_sinks("");
        assert!(sinks.is_empty());
    }
}

#[cfg(test)]
mod pick_backend_tests {
    use super::*;

    #[test]
    fn pick_backend_three_classes() {
        assert_eq!(pick_backend(""), Backend::RodioDefault);
        assert_eq!(pick_backend("default"), Backend::RodioDefault);
        assert_eq!(pick_backend("hw:CARD=USB,DEV=0"), Backend::RodioAlsa);
        assert_eq!(pick_backend("alsa_output.usb-Generic_USB_Audio-00.analog-stereo"), Backend::Pacat);
        assert_eq!(pick_backend("bluez_output.AA_BB_CC.a2dp-sink"), Backend::Pacat);
        assert_eq!(pick_backend("bluez_sink.XX_YY.a2dp"), Backend::Pacat);
        // 未知名按 default 处理，不 panic
        assert_eq!(pick_backend("some-unknown-name"), Backend::RodioDefault);
    }

    #[test]
    fn device_lost_detection() {
        // 模拟两份列表差集
        let current = "alsa_output.usb-headphones";
        let old: Vec<String> = vec!["default".into(), current.into(), "alsa_output.pci".into()];
        let new: Vec<String> = vec!["default".into(), "alsa_output.pci".into()];
        // 真做差集，而不是把 old 晾着不用：消失的设备应当只有 current 这一个
        let removed: Vec<String> = old
            .iter()
            .filter(|o| !new.iter().any(|n| n == *o))
            .cloned()
            .collect();
        assert_eq!(removed, vec![current.to_string()], "差集应只报出消失的那个设备");
        let still_present = new.iter().any(|n| n == current);
        assert!(!still_present, "设备消失判定应返回 false");
        // 若新列表里还有 current，则不触发
        let new2: Vec<String> = vec!["default".into(), current.into()];
        assert!(new2.iter().any(|n| n == current));
    }

    #[test]
    fn choose_backend_chain_pacat_first() {
        let tools = ProbeResult { pacat: true, pw_play: true, paplay: true, rodio: true };
        assert_eq!(choose_backend("alsa_output.usb-x", &tools), Backend::Pacat);
        let tools = ProbeResult { pacat: false, pw_play: true, paplay: true, rodio: true };
        assert_eq!(choose_backend("alsa_output.usb-x", &tools), Backend::PwPlay);
        let tools = ProbeResult { pacat: false, pw_play: false, paplay: true, rodio: true };
        assert_eq!(choose_backend("alsa_output.usb-x", &tools), Backend::Paplay);
        let tools = ProbeResult { pacat: false, pw_play: false, paplay: false, rodio: true };
        assert_eq!(choose_backend("alsa_output.usb-x", &tools), Backend::RodioDefault);
    }

    #[test]
    fn parse_sink_landing_fixture() {
        let fixture = r#"Sink Input #42
        sink: 0
        Sink: 2
        application.name = "pacat"
"#;
        assert_eq!(parse_sink_landing(fixture), Some(2));
        let fixture2 = r#"Sink Input #7
        Sink: 0
        application.name = "ALSA plug-in [other]"
"#;
        assert_eq!(parse_sink_landing(fixture2), None);
    }

    #[test]
    fn test_tone_wav_header() {
        let wav = make_test_tone_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn file_backend_args_tricky_sink_names() {
        let tmp = std::path::Path::new("/tmp/x.wav");
        for name in ["alsa_output.usb-Logitech.analog-stereo", "我的耳机", "Sink: 0: foo"] {
            let args = file_backend_args(Backend::PwPlay, name, tmp);
            assert_eq!(args[1], std::ffi::OsString::from(name));
            let args2 = file_backend_args(Backend::Paplay, name, tmp);
            assert_eq!(args2[1], std::ffi::OsString::from(name));
        }
        // pw-play 用 --target，paplay 用 --device
        assert_eq!(file_backend_args(Backend::PwPlay, "s", tmp)[0], std::ffi::OsString::from("--target"));
        assert_eq!(file_backend_args(Backend::Paplay, "s", tmp)[0], std::ffi::OsString::from("--device"));
    }

    #[test]
    fn rodio_env_override_logic() {
        assert!(rodio_env_override("").is_none());
        assert!(rodio_env_override("default").is_none());
        let (k, v) = rodio_env_override("alsa_output.usb-x").unwrap();
        assert_eq!(k, "PULSE_SINK");
        assert_eq!(v, "alsa_output.usb-x");
    }

    #[test]
    fn parse_sink_name_to_index_fixture() {
        // 真实 pactl list sinks short 样例（含 vol/中文 name）
        let fixture = "0\talsa_output.pci-0000_00_1f.3.analog-stereo\tauto\t2\t44100\n\
                      1\talsa_output.usb-Logitech_Headset-00.analog-stereo\tauto\t2\t48000\n\
                      2\tbluez_sink.AA_BB.a2dp-sink\tauto\t2\t48000\n";
        assert_eq!(parse_sink_name_to_index(fixture, "alsa_output.pci-0000_00_1f.3.analog-stereo"), Some(0));
        assert_eq!(parse_sink_name_to_index(fixture, "alsa_output.usb-Logitech_Headset-00.analog-stereo"), Some(1));
        assert_eq!(parse_sink_name_to_index(fixture, "bluez_sink.AA_BB.a2dp-sink"), Some(2));
        assert_eq!(parse_sink_name_to_index(fixture, "nonexistent"), None);
    }
}
