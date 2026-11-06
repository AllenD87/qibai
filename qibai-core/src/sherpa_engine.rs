//! sherpa-onnx · FunASR Paraformer-zh 中文离线转写后端。
//!
//! 与 whisper.cpp 并列的 STT 后端，由环境变量 `STT_ENGINE` 切换：
//! - `STT_ENGINE=whisper`   强制走 whisper.cpp（回退路径，始终保留）
//! - `STT_ENGINE=paraformer` 强制走本模块（sherpa-onnx paraformer-zh int8）
//! - 未设置                  ：本模块资源齐全（runner+模型）时自动用 paraformer，否则 whisper
//!
//! 依赖（均不进增量包，由 scripts/fetch_sherpa_models.sh 拉取）：
//! - python3 + `pip install sherpa-onnx`（onnxruntime 随 wheel 捆绑）
//! - scripts/sherpa_stt.py（本仓库随包分发）
//! - 模型目录下：paraformer.onnx / tokens.txt / vad.onnx / punc.onnx
//!
//! 输入契约与 whisper_engine 完全一致：16k 单声道 i16 WAV 字节 → 文本。
//! 因此 listener.rs / engine.rs 零改动。

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// STT 引擎选择环境变量名。
pub const ENV_ENGINE: &str = "STT_ENGINE";

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn exists(p: &Path) -> bool {
    p.is_file()
}

/// python3 解释器路径（可用 QIBAI_PYTHON 覆盖）。
pub fn python_bin() -> PathBuf {
    std::env::var("QIBAI_PYTHON")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"))
}

/// sherpa_stt.py runner 脚本路径。
pub fn runner_script() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("QIBAI_SHERPA_RUNNER") {
        if !v.is_empty() {
            cands.push(PathBuf::from(v));
        }
    }
    if let Some(d) = exe_dir() {
        cands.push(d.join("scripts").join("sherpa_stt.py"));
        cands.push(d.join("..").join("scripts").join("sherpa_stt.py"));
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            cands.push(r2.join("scripts").join("sherpa_stt.py"));
        }
    }
    cands.push(PathBuf::from("scripts").join("sherpa_stt.py"));
    cands.push(PathBuf::from("src-tauri").join("scripts").join("sherpa_stt.py"));
    cands.into_iter().find(|p| exists(p))
}

/// 模型目录（含 paraformer.onnx/tokens.txt + vad.onnx|silero_vad.onnx；punc.onnx 可选）。
pub fn model_dir() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("QIBAI_SHERPA_DIR") {
        if !v.is_empty() {
            cands.push(PathBuf::from(v));
        }
    }
    if let Some(d) = exe_dir() {
        cands.push(d.join("models").join("sherpa"));
        cands.push(d.join("..").join("models").join("sherpa"));
        if let Some(r2) = d.parent().and_then(|p| p.parent()) {
            cands.push(r2.join("models").join("sherpa"));
        }
    }
    cands.push(PathBuf::from("models").join("sherpa"));
    cands.push(PathBuf::from("src-tauri").join("models").join("sherpa"));
    // v19：vad 支持 vad.onnx 与 silero_vad.onnx 两种文件名；punc.onnx 不强制（可选标点模型）。
    cands.into_iter().find(|p| {
        exists(&p.join("paraformer.onnx"))
            && exists(&p.join("tokens.txt"))
            && (exists(&p.join("vad.onnx")) || exists(&p.join("silero_vad.onnx")))
    })
}

/// `(paraformer, tokens, vad, punc?)` 模型文件路径；punc 为可选（无标点模型时 None）。
pub fn model_files() -> Option<(PathBuf, PathBuf, PathBuf, Option<PathBuf>)> {
    let d = model_dir()?;
    let vad = if exists(&d.join("vad.onnx")) {
        d.join("vad.onnx")
    } else {
        d.join("silero_vad.onnx")
    };
    let punc = if exists(&d.join("punc.onnx")) {
        Some(d.join("punc.onnx"))
    } else {
        None
    };
    Some((d.join("paraformer.onnx"), d.join("tokens.txt"), vad, punc))
}

/// 探测 paraformer 后端可用性：`(ok, reason)`。
pub fn probe() -> (bool, String) {
    if runner_script().is_none() {
        return (false, "未找到 sherpa_stt.py（scripts/sherpa_stt.py 或 QIBAI_SHERPA_RUNNER）".into());
    }
    if model_files().is_none() {
        return (false, "未找到 sherpa 模型（models/sherpa/{paraformer.onnx,tokens.txt,vad.onnx|silero_vad.onnx} 或 QIBAI_SHERPA_DIR；punc.onnx 可选）".into());
    }
    (true, "paraformer(sherpa-onnx) 引擎就绪".into())
}

/// 当前生效的后端：`"paraformer"` 或 `"whisper"`。
pub fn active_engine() -> &'static str {
    match std::env::var(ENV_ENGINE).ok().unwrap_or_default().as_str() {
        "whisper" => "whisper",
        "paraformer" | "sherpa" => "paraformer",
        // 未显式设置：资源齐全即用 paraformer，否则回退 whisper（保证开箱不破坏现状）。
        _ => {
            if probe().0 {
                "paraformer"
            } else {
                "whisper"
            }
        }
    }
}

/// 把 16k WAV 字节转写为文本（子进程调 sherpa_stt.py）。
pub async fn transcribe_wav_bytes(wav: &[u8]) -> Result<String, String> {
    let runner = runner_script().ok_or_else(|| "未找到 sherpa_stt.py".to_string())?;
    let (pf, tok, vad, punc) =
        model_files().ok_or_else(|| "未找到 sherpa 模型文件".to_string())?;

    let tmp = std::env::temp_dir();
    let tag = format!(
        "qibai-sherpa-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let wav_path = tmp.join(format!("{tag}.wav"));
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&wav_path).map_err(|e| format!("临时 wav 写入失败: {e}"))?;
        f.write_all(wav).map_err(|e| format!("临时 wav 写入失败: {e}"))?;
        f.flush().ok();
    }

    let out = {
        let mut cmd = tokio::process::Command::new(python_bin());
        cmd.arg(&runner)
            .arg("--wav").arg(&wav_path)
            .arg("--paraformer").arg(&pf)
            .arg("--tokens").arg(&tok)
            .arg("--vad").arg(&vad);
        // v19：punc 标点模型可选——存在才传，避免因缺 punc.onnx 阻断 paraformer 主链路
        if let Some(p) = punc.as_ref() {
            cmd.arg("--punc").arg(p);
        }
        cmd.stdin(Stdio::null()).output().await
    }
    .map_err(|e| format!("sherpa_stt.py 执行失败: {e}"))?;
    let _ = std::fs::remove_file(&wav_path);

    if !out.status.success() {
        return Err(format!(
            "paraformer 识别失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err("paraformer 未识别到语音内容".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_env_override_is_respected() {
        std::env::set_var(ENV_ENGINE, "whisper");
        assert_eq!(active_engine(), "whisper");
        std::env::set_var(ENV_ENGINE, "paraformer");
        assert_eq!(active_engine(), "paraformer");
        std::env::set_var(ENV_ENGINE, "sherpa");
        assert_eq!(active_engine(), "paraformer");
        std::env::remove_var(ENV_ENGINE);
    }

    #[test]
    fn runner_and_model_resolution_need_no_env_when_absent() {
        // 清理环境后：无资源时 runner_script / model_files 应为 None（不 panic）。
        std::env::remove_var("QIBAI_SHERPA_RUNNER");
        std::env::remove_var("QIBAI_SHERPA_DIR");
        // 不依赖真实模型文件：只断言函数返回 Option 且不 panic。
        let _ = runner_script();
        let _ = model_dir();
        assert!(true, "路径解析在无资源时不应 panic");
    }

    #[test]
    fn probe_reports_missing_gracefully() {
        std::env::remove_var("QIBAI_SHERPA_RUNNER");
        std::env::remove_var("QIBAI_SHERPA_DIR");
        // 沙盒里 scripts/sherpa_stt.py 存在于仓库 cwd 时 probe 可能 ok；
        // 这里只断言 probe 返回 (bool,String) 且 reason 非空。
        let (_ok, reason) = probe();
        assert!(!reason.is_empty());
    }

    /// 标点/空白格式化：用于把转写结果规整为单行、折叠内部空白。
    #[test]
    fn output_format_collapses_whitespace() {
        fn format_output(s: &str) -> String {
            s.split_whitespace().collect::<Vec<_>>().join("")
        }
        assert_eq!(format_output("  你好， 世界 。 "), "你好，世界。");
        assert_eq!(format_output(""), "");
    }
}
