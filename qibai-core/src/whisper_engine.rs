//! whisper.cpp 本地离线语音识别引擎（OpenAI 开源，全本地处理不联网）。
//!
//! 依赖：
//! - whisper-cli 二进制（随包分发，或环境变量 QIBAI_WHISPER_BIN 指定）
//! - ggml 中文模型（随包分发 src-tauri/models/ggml-base.bin，
//!   或环境变量 QIBAI_WHISPER_MODEL 指定；可用 small.bin 换更高精度）
//!
//! 隐私：音频只在本地处理，绝不外发。

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// 候选定位顺序：环境变量 → exe 同目录 → 当前目录 → 上级目录（dev 模式）。
fn resolve_candidate(candidates: &[PathBuf]) -> Option<PathBuf> {
    for c in candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    None
}

/// v26-fix43：在候选 bin 目录下递归（最多 depth 层）查找目标二进制——
/// whisper.cpp release zip 解压可能带子目录（whisper-bin-x64/whisper-cli），
/// 直接路径全部 miss 会报「未找到 whisper-cli」。
fn find_in_dir_recursive(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = find_in_dir_recursive(&p, name, depth - 1) {
                    return Some(f);
                }
            }
        }
    }
    None
}

/// v26-fix43：在系统 PATH 中查找可执行文件（用户把二进制放进 PATH 的兜底）。
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for d in std::env::split_paths(&path) {
        let c = d.join(name);
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// v26-fix43：全部可能放置 whisper-bin 的目录（供递归兜底扫描）。
fn possible_bin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_dir() {
        dirs.push(d.join("whisper-bin"));
        dirs.push(d.join("..").join("whisper-bin"));
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            dirs.push(r2.join("whisper-bin"));
        }
    }
    dirs.push(PathBuf::from("src-tauri").join("whisper-bin"));
    dirs.push(PathBuf::from("whisper-bin"));
    dirs.push(PathBuf::from("..").join("whisper-bin"));
    dirs
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// whisper-cli 二进制路径。
pub fn whisper_bin() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("QIBAI_WHISPER_BIN") {
        if !v.is_empty() {
            cands.push(PathBuf::from(v));
        }
    }
    if let Some(d) = exe_dir() {
        cands.push(d.join("whisper-cli"));
        cands.push(d.join("whisper-bin").join("whisper-cli"));
        cands.push(d.join("..").join("whisper-bin").join("whisper-cli"));
        // exe = src-tauri/target/{debug,release}/qibai-tauri → 上两级 = src-tauri/
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            cands.push(r2.join("whisper-bin").join("whisper-cli"));
        }
    }
    // cargo run 场景：cwd = 工程根
    cands.push(PathBuf::from("src-tauri").join("whisper-bin").join("whisper-cli"));
    cands.push(PathBuf::from("whisper-bin").join("whisper-cli"));
    cands.push(PathBuf::from("..").join("whisper-bin").join("whisper-cli"));
    // v18.7：cargo run cwd=工程根时命中 src-tauri/whisper-bin/whisper-cli
    cands.push(PathBuf::from("src-tauri").join("whisper-bin").join("whisper-cli"));
    cands.push(PathBuf::from("src-tauri").join("..").join("src-tauri").join("whisper-bin").join("whisper-cli"));
    // v26-fix43：release zip 解压可能带子目录 → 递归兜底；用户放 PATH → PATH 兜底
    for base in possible_bin_dirs() {
        if let Some(f) = find_in_dir_recursive(&base, "whisper-cli", 2) {
            cands.push(f);
        }
    }
    if let Some(p) = find_in_path("whisper-cli") {
        cands.push(p);
    }
    resolve_candidate(&cands)
}

/// v18.8-fix27：whisper-server 常驻二进制路径（与 whisper-cli 同目录分发；
/// 可用环境变量 QIBAI_WHISPER_SERVER_BIN 指定）。
pub fn whisper_server_bin() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("QIBAI_WHISPER_SERVER_BIN") {
        if !v.is_empty() {
            cands.push(PathBuf::from(v));
        }
    }
    if let Some(d) = exe_dir() {
        cands.push(d.join("whisper-server"));
        cands.push(d.join("whisper-bin").join("whisper-server"));
        cands.push(d.join("..").join("whisper-bin").join("whisper-server"));
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            cands.push(r2.join("whisper-bin").join("whisper-server"));
        }
    }
    cands.push(PathBuf::from("src-tauri").join("whisper-bin").join("whisper-server"));
    cands.push(PathBuf::from("whisper-bin").join("whisper-server"));
    cands.push(PathBuf::from("..").join("whisper-bin").join("whisper-server"));
    // v26-fix43：递归兜底 + PATH 兜底（同 whisper-cli）
    for base in possible_bin_dirs() {
        if let Some(f) = find_in_dir_recursive(&base, "whisper-server", 2) {
            cands.push(f);
        }
    }
    if let Some(p) = find_in_path("whisper-server") {
        cands.push(p);
    }
    resolve_candidate(&cands)
}

/// ggml 模型路径。
pub fn whisper_model() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("QIBAI_WHISPER_MODEL") {
        if !v.is_empty() {
            cands.push(PathBuf::from(v));
        }
    }
    // v18.8-fix25：tiny 模型优先（CPU 转写快约 2 倍，~75M），存在则用；否则回退 base（更准）。
    // 下载：curl -L -o src-tauri/models/ggml-tiny.bin https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
    if let Some(d) = exe_dir() {
        cands.push(d.join("models").join("ggml-small.bin"));
        cands.push(d.join("models").join("ggml-tiny.bin"));
        cands.push(d.join("models").join("ggml-base.bin"));
        cands.push(d.join("..").join("models").join("ggml-small.bin"));
        cands.push(d.join("..").join("models").join("ggml-tiny.bin"));
        cands.push(d.join("..").join("models").join("ggml-base.bin"));
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            cands.push(r2.join("models").join("ggml-small.bin"));
            cands.push(r2.join("models").join("ggml-tiny.bin"));
            cands.push(r2.join("models").join("ggml-base.bin"));
        }
    }
    cands.push(PathBuf::from("src-tauri").join("models").join("ggml-small.bin"));
    cands.push(PathBuf::from("src-tauri").join("models").join("ggml-tiny.bin"));
    cands.push(PathBuf::from("src-tauri").join("models").join("ggml-base.bin"));
    cands.push(PathBuf::from("models").join("ggml-tiny.bin"));
    cands.push(PathBuf::from("models").join("ggml-base.bin"));
    cands.push(PathBuf::from("..").join("models").join("ggml-small.bin"));
    cands.push(PathBuf::from("..").join("models").join("ggml-tiny.bin"));
    cands.push(PathBuf::from("..").join("models").join("ggml-base.bin"));
    // v18.7：cargo run cwd=工程根时命中 src-tauri/models/ggml-base.bin
    cands.push(PathBuf::from("src-tauri").join("models").join("ggml-base.bin"));
    resolve_candidate(&cands)
}

/// 探测引擎可用性：`(ok, reason)`。
pub fn probe() -> (bool, String) {
    match (whisper_bin(), whisper_model()) {
        (Some(_), Some(_)) => (true, "whisper 引擎就绪".to_string()),
        (None, _) => (
            false,
            "未找到 whisper-cli（需随包分发或设置 QIBAI_WHISPER_BIN）".to_string(),
        ),
        (_, None) => (
            false,
            "未找到 ggml 模型（需随包 models/ggml-base.bin 或设置 QIBAI_WHISPER_MODEL）".to_string(),
        ),
    }
}

// ---------------------------------------------------------------- P0-S 结构化探测 + 自愈指引
/// 获取方式（权威来源，不下载入包；均为真实 URL）。
pub const WHISPER_MODEL_URL: &str = "https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
pub const WHISPER_RELEASES_URL: &str = "https://github.com/ggml-org/whisper.cpp/releases";
pub const WHISPER_CONDA_URL: &str = "https://anaconda.org/channels/conda-forge/packages/whisper.cpp/files";
pub const WHISPER_LEGACY_BUNDLE_URL: &str = "https://aka.doubaocdn.com/s/WYP2IlfNNl";

/// STT 引擎状态快照（供日志与 voice_diag 共用；不含音频/转写文本）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SttEngineStatus {
    pub server_found: bool,
    pub server_path: String,
    pub cli_found: bool,
    pub cli_path: String,
    pub model_found: bool,
    pub model_path: String,
    pub probe_ok: bool,
    pub probe_reason: String,
    pub cwd: String,
    pub placement_guide: String,
    pub download_links: Vec<(String, String)>,
}

fn display(p: Option<&std::path::PathBuf>) -> String {
    p.map(|x| x.display().to_string()).unwrap_or_default()
}

/// 结构化探测：server / cli / model 分开判定，缺失项点名给绝对路径与获取方式。
pub fn stt_engine_status() -> SttEngineStatus {
    let server = whisper_server_bin();
    let cli = whisper_bin();
    let model = whisper_model();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());

    let mut missing: Vec<String> = Vec::new();
    if server.is_none() {
        missing.push(format!(
            "whisper-server → {}/src-tauri/whisper-bin/whisper-server",
            cwd
        ));
    }
    if cli.is_none() {
        missing.push(format!(
            "whisper-cli → {}/src-tauri/whisper-bin/whisper-cli",
            cwd
        ));
    }
    if model.is_none() {
        missing.push(format!(
            "ggml 模型 → {}/src-tauri/models/ggml-tiny.bin",
            cwd
        ));
    }

    let probe_ok = server.is_some() && cli.is_some() && model.is_some();
    let probe_reason = if probe_ok {
        "STT 引擎就绪".to_string()
    } else {
        format!("缺失: {}", missing.join("；"))
    };

    let placement_guide = if probe_ok {
        String::new()
    } else {
        format!(
            "STT 引擎缺失（免提聆听将不可用）。请放置：\n  {}\n获取：\n  模型: curl -L -o src-tauri/models/ggml-tiny.bin {}\n  二进制(Windows): {} → whisper-bin-x64.zip\n  二进制(Linux): {} 或源码编译（项目根 whisper/ 已克隆）\n  历史完整资源包: {}",
            missing.join("\n  "),
            WHISPER_MODEL_URL, WHISPER_RELEASES_URL, WHISPER_CONDA_URL, WHISPER_LEGACY_BUNDLE_URL
        )
    };

    SttEngineStatus {
        server_found: server.is_some(),
        server_path: display(server.as_ref()),
        cli_found: cli.is_some(),
        cli_path: display(cli.as_ref()),
        model_found: model.is_some(),
        model_path: display(model.as_ref()),
        probe_ok,
        probe_reason,
        cwd,
        placement_guide,
        download_links: vec![
            ("ggml-tiny 模型(hf-mirror)".into(), WHISPER_MODEL_URL.into()),
            ("whisper.cpp release(Windows 二进制)".into(), WHISPER_RELEASES_URL.into()),
            ("conda-forge linux-64 包".into(), WHISPER_CONDA_URL.into()),
            ("历史完整资源包 v17".into(), WHISPER_LEGACY_BUNDLE_URL.into()),
        ],
    }
}

/// 探测全部失败时打印自愈块（期望绝对路径 + 获取链接）。
pub fn log_stt_self_heal_if_broken() {
    let st = stt_engine_status();
    if !st.probe_ok {
        eprintln!("[qibai] {}", st.placement_guide.replace('\n', " | "));
    }
}

/// 把 base64 解码后的 wav 字节转写为文本（调 whisper-cli 子进程）。
pub async fn transcribe_wav_bytes(wav: &[u8]) -> Result<String, String> {
    // STT 后端切换（v21）：STT_ENGINE=paraformer 走 sherpa-onnx Paraformer 中文模型；
    // 该路径失败/空结果时自动回退下方 whisper 路径（回退路径完整可用）。
    if crate::sherpa_engine::active_engine() == "paraformer" {
        match crate::sherpa_engine::transcribe_wav_bytes(wav).await {
            Ok(t) if !t.trim().is_empty() => return Ok(t),
            Ok(_) => eprintln!("[qibai] paraformer 返回空转写，回退 whisper"),
            Err(e) => eprintln!("[qibai] paraformer 转写失败，回退 whisper: {e}"),
        }
    }
    // v18.8-fix27：优先 whisper-server 常驻（一次加载模型，转写 ~1s）；失败回退子进程。
    let server_bin = whisper_server_bin();
    eprintln!(
        "[qibai] stt 引擎选择: server={} model={}",
        server_bin.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "未找到".into()),
        whisper_model().map(|p| p.display().to_string()).unwrap_or_else(|| "未找到".into())
    );
    // P0-S：任一缺失即打印自愈指引（不中断，错误文案后续仍上报）。
    if server_bin.is_none() || whisper_model().is_none() || whisper_bin().is_none() {
        log_stt_self_heal_if_broken();
    }
    if server_bin.is_some() {
        match server_transcribe(wav).await {
            Ok(t) => return Ok(t),
            Err(e) => eprintln!("[qibai] whisper-server 转写失败，回退 whisper-cli 子进程: {e}"),
        }
    }
    let bin = whisper_bin().ok_or_else(|| "未找到 whisper-cli".to_string())?;
    let model = whisper_model().ok_or_else(|| "未找到 ggml 模型".to_string())?;

    // v18.8-fix12：whisper-cli 动态链接 libwhisper.so.1 + libggml*.so——把
    // whisper-bin 目录加入 LD_LIBRARY_PATH（lib 随包分发；系统未安装时子进程才能加载）。
    let lib_path = bin.parent().map(|d| {
        let cur = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        format!("{}:{}", d.display(), cur)
    });

    // 临时 wav + 输出
    let tmp = std::env::temp_dir();
    let tag = format!(
        "qibai-stt-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let wav_path = tmp.join(format!("{tag}.wav"));
    let out_base = tmp.join(tag.clone());

    // 写 wav
    {
        let mut f = std::fs::File::create(&wav_path)
            .map_err(|e| format!("临时 wav 写入失败: {e}"))?;
        use std::io::Write as _;
        f.write_all(wav).map_err(|e| format!("临时 wav 写入失败: {e}"))?;
        f.flush().ok();
    }

    let lang = std::env::var("QIBAI_WHISPER_LANG").unwrap_or_else(|_| "zh".to_string());
    let mut cmd = tokio::process::Command::new(&bin);
    if let Some(lp) = &lib_path {
        cmd.env("LD_LIBRARY_PATH", lp);
    }
    // v18.8-fix25：线程数 = 可用 CPU 核数（工作站多核显著提速；默认 4 上限）
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(4)
        .min(32);
    let out = cmd
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(&wav_path)
        .arg("-l")
        .arg(&lang)
        .arg("-t")
        .arg(threads.to_string())
        .arg("--output-txt")
        .arg("--output-file")
        .arg(&out_base)
        .arg("--no-prints")
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("whisper-cli 执行失败: {e}"))?;

    // 清理 wav
    let _ = std::fs::remove_file(&wav_path);

    if !out.status.success() {
        return Err(format!(
            "whisper 识别失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // 读取输出 txt（whisper --output-txt 生成 <out_base>.txt）
    let txt_path = tmp.join(format!("{tag}.txt"));
    let text = std::fs::read_to_string(&txt_path)
        .unwrap_or_default();
    let _ = std::fs::remove_file(&txt_path);
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("whisper 未识别到语音内容".into());
    }
    Ok(text)
}

const SERVER_PORT: u16 = 9186;
static SERVER_PID: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

fn process_alive(pid: u32) -> bool {
    // 跨平台存活检测（去 /proc 依赖）：Unix 用 `kill -0`，Windows 用 tasklist 查 PID。
    match std::env::consts::OS {
        "windows" => std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false),
        _ => std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
    }
}

async fn port_open(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok()
}

/// 确保 whisper-server 常驻（一次加载模型，多次转写复用）。
async fn ensure_server(model: &Path, threads: usize) -> Result<u16, String> {
    let held = SERVER_PID.lock().unwrap().clone();
    if let Some(pid) = held {
        if process_alive(pid) && port_open(SERVER_PORT).await {
            eprintln!("[qibai] whisper-server 复用(pid={pid} port=9186)");
            return Ok(SERVER_PORT);
        }
    }
    // 端口已被占用（可能是外部已启动的 whisper-server）→ 直接复用
    if port_open(SERVER_PORT).await {
        *SERVER_PID.lock().unwrap() = None;
        eprintln!("[qibai] whisper-server 复用外部实例(port=9186)");
        return Ok(SERVER_PORT);
    }
    let bin = whisper_server_bin().ok_or_else(|| "未找到 whisper-server".to_string())?;
    let child = std::process::Command::new(&bin)
        .arg("-m")
        .arg(model)
        .arg("-t")
        .arg(threads.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(SERVER_PORT.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("whisper-server 启动失败: {e}"))?;
    let pid = child.id();
    *SERVER_PID.lock().unwrap() = Some(pid);
    eprintln!("[qibai] whisper-server 启动(pid={pid} model={} port=9186)", model.display());
    // 等待就绪（首次加载模型需数秒；最多 10s）
    for _ in 0..50 {
        if port_open(SERVER_PORT).await {
            eprintln!("[qibai] whisper-server 就绪");
            return Ok(SERVER_PORT);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Err("whisper-server 启动超时（模型加载慢？）".into())
}

/// 停止常驻 whisper-server（应用退出时调用）。跨平台：Unix kill / Windows taskkill。
pub fn shutdown_server() {
    if let Some(pid) = SERVER_PID.lock().unwrap().take() {
        match std::env::consts::OS {
            "windows" => {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status();
            }
            _ => {
                let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
            }
        }
    }
}

/// 常驻服务转写：multipart POST /inference（文件字段名 file，返回 {"text": ...}）。
async fn server_transcribe(wav: &[u8]) -> Result<String, String> {
    let model = whisper_model().ok_or_else(|| "未找到 ggml 模型".to_string())?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(4)
        .min(32);
    let port = ensure_server(&model, threads).await?;
    let lang = std::env::var("QIBAI_WHISPER_LANG").unwrap_or_else(|_| "zh".to_string());
    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("multipart 构造失败: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("response_format", "json")
        .text("language", lang)
        // v18.8-fix30：STT 中文识别质量参数——whisper.cpp server 支持 beam 搜索与
        // 温度控制：beam_size/best_of 提升候选路径、temperature=0 降低随机错字
        // （用户反馈中文转写成功率低，tiny 模型容量有限，参数+换大模型双管齐下）。
        .text("beam_size", "5")
        .text("best_of", "5")
        .text("temperature", "0");
    let t0 = std::time::Instant::now();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/inference"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("whisper-server 请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("whisper-server 响应解析失败: {e}"))?;
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("whisper 未识别到语音内容".into());
    }
    eprintln!("[qibai] stt server took {}ms text_len={}", t0.elapsed().as_millis(), text.chars().count());
    Ok(text)
}

/// 供外部（免提监听）使用的文件转写入口。
pub async fn transcribe_wav_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("读取 wav 失败: {e}"))?;
    transcribe_wav_bytes(&bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实转写链路验证：whisper-cli + ggml 模型 + Rust 调用，全本地离线。
    #[test]
    fn transcribe_sample_wav() {
        // env 操作与 stt_engine_status 测试竞争：持 std 锁串行化，用 block_on 跑异步转写。
        let _g = crate::test_support::env_lock();
        // whisper.cpp 自带示例音频（jfk.wav，英文短演讲）
        let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../whisper/samples/jfk.wav");
        if !sample.is_file() {
            eprintln!("sample 缺失，跳过真实转写测试");
            return;
        }
        // 测试显式指向随包二进制与模型（cargo test 的 cwd 非 src-tauri）
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        std::env::set_var(
            "QIBAI_WHISPER_BIN",
            root.join("src-tauri/whisper-bin/whisper-cli"),
        );
        std::env::set_var(
            "QIBAI_WHISPER_MODEL",
            root.join("src-tauri/models/ggml-base.bin"),
        );
        let (ok, reason) = probe();
        assert!(ok, "whisper 引擎应就绪: {reason}");
        let bytes = std::fs::read(&sample).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let text = rt
            .block_on(transcribe_wav_bytes(&bytes))
            .expect("转写应成功");
        assert!(!text.trim().is_empty(), "识别文本不应为空");
        assert!(
            text.to_lowercase().contains("ask"),
            "应识别出 jfk 演讲关键词，实际: {text}"
        );
    }

    #[test]
    fn stt_engine_status_structured_and_links() {
        // env 操作与 transcribe_sample_wav 竞争，锁内串行化。
        let _g = crate::test_support::env_lock();
        // 清理资源 → 应三 false + 自愈指引 + 链接非空（cwd 为绝对路径）。
        std::env::remove_var("QIBAI_WHISPER_BIN");
        std::env::remove_var("QIBAI_WHISPER_SERVER_BIN");
        std::env::remove_var("QIBAI_WHISPER_MODEL");
        let st = stt_engine_status();
        assert!(!st.probe_ok, "无资源时 probe_ok 应为 false");
        assert!(!st.server_found && !st.cli_found && !st.model_found);
        assert!(st.cwd.starts_with('/') || st.cwd.contains(':'), "cwd 应为绝对路径");
        assert!(!st.placement_guide.is_empty(), "缺失时应给出放置指引");
        assert!(st.download_links.len() >= 4, "应含 4 个获取链接");
        assert!(st.probe_reason.contains("缺失"));

        // 显式指向模型 → model_found 为 true（server/cli 仍可能缺）。
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        std::env::set_var(
            "QIBAI_WHISPER_MODEL",
            root.join("src-tauri/models/ggml-base.bin"),
        );
        let st2 = stt_engine_status();
        assert!(st2.model_found, "设置模型路径后 model_found 应为 true");
        std::env::remove_var("QIBAI_WHISPER_MODEL");
    }
}
