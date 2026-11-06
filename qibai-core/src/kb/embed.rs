//! P2：本地向量化（bge-small-zh-v1.5，纯 Rust tract 推理）。
//!
//! 设计口径（A 方案）：
//! - 模型不进安装包，用户跑 `fetch_embed_model.sh` 按需下载到 `<data_dir>/kb/embed/bge-small-zh-v1.5/`
//! - 推理全程本机（tract-onnx，纯 Rust，无 C 运行时依赖），文本不出本机
//! - 模型缺失/加载失败一律返回错误但不 panic，上层自动退回纯 BM25
//!
//! 为什么按 token 截断：实测 64 tok ≈ 55ms、128 tok ≈ 143ms、512 tok ≈ 1.5s（注意力是平方级）。
//! 中文按字切词，700 字的块就近 700 token，整块送进去一次 1.5s，1000 块重建要 20 分钟以上。
//! 所以向量只取「块开头 CLS/SEP + 前 `MAX_LEN` 个 token」，检索原文仍用完整块文本。

use super::KbError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tract_onnx::prelude::*;

/// 模型标识（写进向量索引，模型换了就整体重建）。
pub const MODEL_NAME: &str = "bge-small-zh-v1.5";
/// 输出维度（bge-small-zh-v1.5 = 512）。
pub const MODEL_DIM: usize = 512;
/// 送进模型的 token 上限（性能与召回折中，见模块注释）。
pub const MAX_LEN: usize = 128;

const PAD_ID: i64 = 0;
const CLS_ID: i64 = 101;
const SEP_ID: i64 = 102;
const UNK_ID: i64 = 100;

/// 模型目录：`<data_dir>/kb/embed/bge-small-zh-v1.5/`（含 `model.onnx` + `vocab.txt`）。
pub fn default_model_dir() -> PathBuf {
    super::kb_dir().join("embed").join(MODEL_NAME)
}

/// 把一段文本压成向量。
pub trait TextEmbedder: Send + Sync {
    fn dim(&self) -> usize;
    fn model_name(&self, ) -> String;
    /// 逐条编码（返回条数与入参一致）。
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, KbError>;
}

/// 中文 BERT WordPiece 分词（只依赖 `vocab.txt`，不引 tokenizers crate）。
pub struct Tokenizer {
    vocab: HashMap<String, i64>,
}

impl Tokenizer {
    pub fn load(vocab_path: &Path) -> Result<Self, KbError> {
        let s = std::fs::read_to_string(vocab_path)
            .map_err(|e| KbError::Unreadable(format!("{}: {e}", vocab_path.display())))?;
        let vocab: HashMap<String, i64> = s
            .lines()
            .enumerate()
            .map(|(i, line)| (line.trim_end().to_string(), i as i64))
            .collect();
        if vocab.len() < 1000 {
            return Err(KbError::Unreadable(format!(
                "{}: 词表行数异常（{}），可能不是 BERT vocab.txt",
                vocab_path.display(),
                vocab.len()
            )));
        }
        Ok(Self { vocab })
    }

    fn id(&self, tok: &str, fallback: i64) -> i64 {
        self.vocab.get(tok).copied().unwrap_or(fallback)
    }

    /// BERT basic tokenizer：小写化 + CJK 逐字 + 标点单切 + 其余连词，再做 WordPiece 贪心最长匹配。
    pub fn wordpiece_ids(&self, text: &str) -> Vec<i64> {
        let mut out = Vec::new();
        for token in basic_tokens(text) {
            if let Some(&id) = self.vocab.get(&token) {
                out.push(id);
                continue;
            }
            let chars: Vec<char> = token.chars().collect();
            let mut start = 0usize;
            let mut pieces: Vec<i64> = Vec::new();
            while start < chars.len() {
                let mut end = chars.len();
                let mut found: Option<(i64, usize)> = None;
                while start < end {
                    let sub: String = chars[start..end].iter().collect();
                    let cand = if start > 0 { format!("##{sub}") } else { sub };
                    if let Some(&id) = self.vocab.get(&cand) {
                        found = Some((id, end));
                        break;
                    }
                    end -= 1;
                }
                match found {
                    Some((id, e)) => {
                        pieces.push(id);
                        start = e;
                    }
                    None => {
                        // 整词无法切分：回退 [UNK]
                        pieces.clear();
                        pieces.push(UNK_ID);
                        break;
                    }
                }
            }
            out.push(if pieces.is_empty() { UNK_ID } else { pieces[0] });
            out.extend(pieces.into_iter().skip(1));
        }
        out
    }

    /// 编码成 (input_ids, attention_mask)，两端加 `[CLS]`/`[SEP]`，截断到 `max_len`。
    pub fn encode(&self, text: &str, max_len: usize) -> (Vec<i64>, Vec<i64>) {
        let cls = self.id("[CLS]", CLS_ID);
        let sep = self.id("[SEP]", SEP_ID);
        let mut ids = vec![cls];
        let body = max_len.saturating_sub(2).max(1);
        ids.extend(self.wordpiece_ids(text).into_iter().take(body));
        ids.push(sep);
        let n = ids.len();
        (ids, vec![1i64; n])
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// BERT basic tokenization（小写、CJK 单字、标点/符号单切、其余字母数字连成词）。
fn basic_tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if is_cjk(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
            continue;
        }
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
            continue;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

type Plan = TypedSimplePlan<TypedModel>;

/// tract 上的 bge-small-zh-v1.5 推理器。
pub struct BertEmbedder {
    plan: Plan,
    tok: Tokenizer,
    dim: usize,
}

impl BertEmbedder {
    /// 从模型目录加载（需要 `model.onnx` + `vocab.txt`）。
    pub fn load(dir: &Path) -> Result<Self, KbError> {
        let model_path = dir.join("model.onnx");
        let vocab_path = dir.join("vocab.txt");
        if !model_path.exists() {
            return Err(KbError::Embed(format!(
                "缺少模型文件（应先跑 fetch_embed_model.sh）: {}",
                model_path.display()
            )));
        }
        if !vocab_path.exists() {
            return Err(KbError::Embed(format!(
                "缺少词表文件（应先跑 fetch_embed_model.sh）: {}",
                vocab_path.display()
            )));
        }
        let plan = tract_onnx::onnx()
            .model_for_path(&model_path)
            .map_err(|e| KbError::Embed(format!("读取 onnx 失败: {e}")))?
            .with_input_fact(0, i64::fact([1, MAX_LEN]).into())
            .and_then(|m| m.with_input_fact(1, i64::fact([1, MAX_LEN]).into()))
            .and_then(|m| m.with_input_fact(2, i64::fact([1, MAX_LEN]).into()))
            .map_err(|e| KbError::Embed(format!("声明输入形状失败: {e}")))?
            .into_optimized()
            .map_err(|e| KbError::Embed(format!("图优化失败: {e}")))?
            .into_runnable()
            .map_err(|e| KbError::Embed(format!("编译执行计划失败: {e}")))?;
        let tok = Tokenizer::load(&vocab_path)?;
        Ok(Self { plan, tok, dim: MODEL_DIM })
    }

    /// 单条推理：mask 平均池化 + L2 归一化（bge 系列的池化口径）。
    fn embed_one(&self, text: &str) -> Result<Vec<f32>, KbError> {
        let (mut ids, mut mask) = self.tok.encode(text, MAX_LEN);
        ids.truncate(MAX_LEN);
        mask.truncate(MAX_LEN);
        let real = ids.len().max(1);
        ids.resize(MAX_LEN, PAD_ID);
        mask.resize(MAX_LEN, 0);
        let ids_t = Tensor::from_shape(&[1usize, MAX_LEN], &ids[..])
            .map_err(|e| KbError::Embed(format!("input_ids 张量: {e}")))?;
        let mask_t = Tensor::from_shape(&[1usize, MAX_LEN], &mask[..])
            .map_err(|e| KbError::Embed(format!("attention_mask 张量: {e}")))?;
        let type_t = Tensor::from_shape(&[1usize, MAX_LEN], &vec![0i64; MAX_LEN][..])
            .map_err(|e| KbError::Embed(format!("token_type_ids 张量: {e}")))?;
        let out = self
            .plan
            .run(tvec!(ids_t.into(), mask_t.into(), type_t.into()))
            .map_err(|e| KbError::Embed(format!("推理失败: {e}")))?;
        let hs = out
            .first()
            .ok_or_else(|| KbError::Embed("推理无输出".into()))?
            .to_array_view::<f32>()
            .map_err(|e| KbError::Embed(format!("读取输出失败: {e}")))?;
        if hs.shape().len() != 3 || hs.shape()[2] != self.dim {
            return Err(KbError::Embed(format!(
                "输出形状异常: {:?}（期望 [1,{MAX_LEN},{}]）",
                hs.shape(),
                self.dim
            )));
        }
        let mut v = vec![0f32; self.dim];
        for s in 0..real {
            for d in 0..self.dim {
                v[d] += hs[[0, s, d]];
            }
        }
        let inv = 1.0 / real as f32;
        for x in v.iter_mut() {
            *x *= inv;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-12 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

impl TextEmbedder for BertEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> String {
        MODEL_NAME.to_string()
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, KbError> {
        texts.iter().map(|t| self.embed_one(t)).collect()
    }
}

/// 进程内单例：首次调用尝试加载，失败缓存为 None（避免每次检索都重试 91MB 模型）。
/// 用 Mutex 包一层是为了「应用内下载完模型后能立刻失效重载」。
static CACHE: std::sync::Mutex<Option<Option<Arc<BertEmbedder>>>> = std::sync::Mutex::new(None);

pub fn cached_default() -> Option<Arc<BertEmbedder>> {
    if let Ok(g) = CACHE.lock() {
        if let Some(v) = g.as_ref() {
            return v.clone();
        }
    }
    let loaded = if !default_model_dir().join("model.onnx").exists() {
        eprintln!(
            "[qibai] kb: 未安装向量模型（{}），检索走纯 BM25",
            default_model_dir().display()
        );
        None
    } else {
        let t0 = std::time::Instant::now();
        match BertEmbedder::load(&default_model_dir()) {
            Ok(e) => {
                eprintln!(
                    "[qibai] kb: 向量模型就绪 {}（{} 维，{:.0?}）",
                    MODEL_NAME,
                    e.dim,
                    t0.elapsed()
                );
                Some(Arc::new(e))
            }
            Err(err) => {
                eprintln!("[qibai] kb: 向量模型加载失败，退回纯 BM25: {err}");
                None
            }
        }
    };
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(loaded.clone());
    }
    loaded
}

/// 下载/删除模型后调用：丢掉缓存，下次检索按磁盘现状重新判定。
pub fn invalidate_cache() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

/// 应用内「一键下载模型」：拉 model.onnx + vocab.txt 到 `default_model_dir()`。
///
/// - 先写 `.tmp`，sha256 校验通过才 rename（半截文件不会顶替好文件）
/// - 主源是仓库自己的 Release（国内可达性更好），失败回退 HuggingFace 原地址
/// - `progress(bytes_done, bytes_total)` 用于前端进度条；total=0 表示未知
pub async fn fetch_model(
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<PathBuf, KbError> {
    const MODEL_SHA: &str = "69a0b846f4f116b5e6aabf9546ea6754d02264f3211a13a1bd69b31b8040749a";
    const VOCAB_SHA: &str = "45bbac6b341c319adc98a532532882e91a9cefc0329aa57bac9ae761c27b291c";
    let dir = default_model_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| KbError::Embed(format!("创建模型目录失败 {}: {e}", dir.display())))?;

    let files: [(&str, &str, &str); 2] = [
        (
            "model.onnx",
            "https://github.com/AllenD87/qibai/releases/download/kb-model-v1/model.onnx",
            MODEL_SHA,
        ),
        (
            "vocab.txt",
            "https://github.com/AllenD87/qibai/releases/download/kb-model-v1/vocab.txt",
            VOCAB_SHA,
        ),
    ];
    const HF_BASE: &str = "https://huggingface.co/Xenova/bge-small-zh-v1.5/resolve/main/";

    let client = reqwest::Client::builder()
        .user_agent("qibai-kb-model-fetch/1")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| KbError::Embed(format!("HTTP client: {e}")))?;

    for (name, url, sha) in files {
        let dest = dir.join(name);
        if dest.exists() {
            if let Ok(bytes) = std::fs::read(&dest) {
                if sha256_hex(&bytes) == sha {
                    continue;
                }
            }
        }
        let mut ok = false;
        let mut last_err = String::new();
        for attempt_url in [url.to_string(), format!("{HF_BASE}{}", if name == "model.onnx" { "onnx/model.onnx" } else { "vocab.txt" })] {
            match download_to(&client, &attempt_url, &dest, on_progress).await {
                Ok(bytes) => {
                    let got = sha256_hex(&bytes);
                    if got != sha {
                        last_err = format!("{attempt_url}: sha256 不符（got {got}）");
                        continue;
                    }
                    ok = true;
                    break;
                }
                Err(e) => last_err = format!("{attempt_url}: {e}"),
            }
        }
        if !ok {
            return Err(KbError::Embed(format!("下载 {name} 失败：{last_err}")));
        }
    }
    eprintln!("[qibai] kb: 向量模型已安装到 {}", dir.display());
    Ok(dir)
}

/// 下载到 `<dest>.tmp` 后原子替换，边下边报进度。
async fn download_to(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<Vec<u8>, KbError> {
    use futures_util::StreamExt;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| KbError::Embed(format!("请求失败: {e}")))?
        .error_for_status()
        .map_err(|e| KbError::Embed(format!("HTTP 状态异常: {e}")))?;
    let total = resp.content_length().unwrap_or(0);
    let tmp = dest.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)
        .map_err(|e| KbError::Embed(format!("写临时文件失败 {}: {e}", tmp.display())))?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut all: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| KbError::Embed(format!("下载中断: {e}")))?;
        std::io::Write::write_all(&mut f, &chunk)
            .map_err(|e| KbError::Embed(format!("写盘失败: {e}")))?;
        all.extend_from_slice(&chunk);
        done += chunk.len() as u64;
        if let Some(cb) = on_progress {
            cb(done, total);
        }
    }
    drop(f);
    std::fs::rename(&tmp, dest)
        .map_err(|e| KbError::Embed(format!("替换模型文件失败 {}: {e}", dest.display())))?;
    Ok(all)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// 尽力加载（测试与诊断用，失败返回错误而不是 None）。
pub fn try_load() -> Result<BertEmbedder, KbError> {
    BertEmbedder::load(&default_model_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok() -> Tokenizer {
        let p = std::env::var("QIBAI_KB_VOCAB").unwrap_or_default();
        if p.is_empty() || !Path::new(&p).exists() {
            // 没词表时用最小假词表，仅验证切词骨架不 panic
            let vocab: HashMap<String, i64> = ["[PAD]", "[CLS]", "[SEP]", "[UNK]"]
                .iter()
                .enumerate()
                .map(|(i, s)| (s.to_string(), i as i64))
                .collect();
            return Tokenizer { vocab };
        }
        Tokenizer::load(Path::new(&p)).unwrap()
    }

    #[test]
    fn basic_tokens_splits_cjk_per_char() {
        let t = basic_tokens("电脑很卡，CPU 100%");
        assert_eq!(t.first().map(|s| s.as_str()), Some("电"));
        assert!(t.iter().any(|s| s == "cpu"), "英文应小写连词: {t:?}");
        assert!(t.iter().any(|s| s == "，"), "中文标点应单切: {t:?}");
    }

    #[test]
    fn encode_has_cls_sep_and_respects_max_len() {
        let tk = tok();
        let (ids, mask) = tk.encode("电脑很卡怎么办", 8);
        assert!(ids.len() <= 8, "不得超过 max_len: {} 个", ids.len());
        assert_eq!(mask.len(), ids.len());
        assert_eq!(*ids.first().unwrap(), tk.id("[CLS]", CLS_ID));
        assert_eq!(*ids.last().unwrap(), tk.id("[SEP]", SEP_ID));
    }

    #[test]
    fn missing_model_returns_error_not_panic() {
        let r = BertEmbedder::load(Path::new("/tmp/qibai_no_such_model_dir"));
        assert!(r.is_err());
    }
}
