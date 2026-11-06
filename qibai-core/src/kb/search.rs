//! v45.10 P1：纯 BM25 检索。
//!
//! - k1=1.2, b=0.75（标准 BM25）
//! - 中文切分：字符二元组（bigram）；英文按词
//! - 返回 top_k 命中，snippet 截 ~200 字

use super::{chunk::Chunk, KbHit};
use std::collections::HashMap;

const K1: f32 = 1.2;
const B: f32 = 0.75;

/// 中文 bigram + 英文按词分词。
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut english = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_alphabetic() || c.is_ascii_digit() {
            english.push(c.to_ascii_lowercase());
            i += 1;
            continue;
        }
        if !english.is_empty() {
            out.push(std::mem::take(&mut english));
        }
        // 中文：bigram
        if i + 1 < chars.len() && is_cjk(c) && is_cjk(chars[i+1]) {
            out.push(format!("{}{}", c, chars[i+1]));
        } else if !c.is_whitespace() && !c.is_ascii_punctuation() {
            out.push(c.to_string());
        }
        i += 1;
    }
    if !english.is_empty() { out.push(english); }
    out
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// BM25 原始分（对全部块，长度 = 块数；0 表示没命中任何查询词）。
fn bm25_scores(chunks: &[Chunk], query: &str) -> Vec<f32> {
    let mut scores = vec![0f32; chunks.len()];
    let qtokens = tokenize(query);
    if chunks.is_empty() || qtokens.is_empty() {
        return scores;
    }

    let mut df: HashMap<String, usize> = HashMap::new();
    let mut doc_tokens: Vec<HashMap<String, usize>> = Vec::with_capacity(chunks.len());
    let mut doc_len: Vec<usize> = Vec::with_capacity(chunks.len());
    for c in chunks {
        let toks = tokenize(&c.text);
        let mut tf: HashMap<String, usize> = HashMap::new();
        for t in &toks { *tf.entry(t.clone()).or_insert(0) += 1; }
        for t in tf.keys() { *df.entry(t.clone()).or_insert(0) += 1; }
        doc_len.push(toks.len());
        doc_tokens.push(tf);
    }
    let n = chunks.len() as f32;
    let avgdl = doc_len.iter().sum::<usize>() as f32 / n.max(1.0);

    for (i, tf) in doc_tokens.iter().enumerate() {
        let mut s = 0.0f32;
        for q in &qtokens {
            let Some(&f) = tf.get(q) else { continue };
            let f = f as f32;
            let d = df.get(q).copied().unwrap_or(0) as f32;
            let idf = ((n - d + 0.5) / (d + 0.5) + 1.0).ln();
            let dl = doc_len[i] as f32;
            let denom = f32::mul_add(K1, 1.0 - B + B * dl / avgdl.max(1.0), f);
            s += idf * f32::mul_add(K1, f, 0.0) / denom.max(1.0);
        }
        scores[i] = s;
    }
    scores
}

fn hit_of(c: &Chunk, score: f32, query: &str) -> KbHit {
    KbHit {
        file_path: c.file_path.clone(),
        char_offset: c.char_offset,
        heading_path: c.heading_path.clone(),
        score: (score * 1000.0).round() / 1000.0,
        snippet: snippet(&c.text, query, 200),
    }
}

pub fn search(chunks: &[Chunk], query: &str, top_k: usize) -> Vec<KbHit> {
    if chunks.is_empty() || query.trim().is_empty() {
        return Vec::new();
    }
    let raw = bm25_scores(chunks, query);
    let mut scored: Vec<(usize, f32)> = raw.iter().copied().enumerate().filter(|(_, s)| *s > 0.0).collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored
        .into_iter()
        .take(top_k)
        .map(|(i, s)| hit_of(&chunks[i], s, query))
        .collect()
}

/// 混合检索：BM25 与余弦相似度加权融合。
///
/// - `qvec` 为 None（模型缺席）时等价于纯 BM25
/// - `w_vec` 是向量权重（0..1），BM25 权重取 `1 - w_vec`
/// - BM25 先按本批最大值归一，余弦归一化向量上就是点积，两者落在同一量纲再融合
pub fn search_hybrid(
    chunks: &[Chunk],
    query: &str,
    top_k: usize,
    qvec: Option<&[f32]>,
    vectors: Option<&super::vectors::VectorIndex>,
    w_vec: f32,
) -> Vec<KbHit> {
    if chunks.is_empty() || query.trim().is_empty() {
        return Vec::new();
    }
    let (Some(q), Some(idx)) = (qvec, vectors) else {
        return search(chunks, query, top_k);
    };
    if idx.dim == 0 || q.len() != idx.dim {
        return search(chunks, query, top_k);
    }
    let w = w_vec.clamp(0.0, 1.0);
    let raw = bm25_scores(chunks, query);
    let max_bm = raw.iter().copied().fold(0f32, f32::max).max(1e-9);

    // key -> 行号（一次查表，避免每个块都线性扫 entries）
    let mut rows: HashMap<String, usize> = HashMap::with_capacity(idx.entries.len());
    for (i, e) in idx.entries.iter().enumerate() {
        rows.insert(e.key.clone(), i);
    }

    let mut scored: Vec<(usize, f32)> = Vec::with_capacity(chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        let bm = raw[i] / max_bm;
        let cos = rows
            .get(&super::vectors::key_of(c))
            .and_then(|&r| idx.row(r))
            .map(|row| super::vectors::cosine(q, row).max(0.0))
            .unwrap_or(0.0);
        let s = (1.0 - w) * bm + w * cos;
        if s > 0.0 {
            scored.push((i, s));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored
        .into_iter()
        .take(top_k)
        .map(|(i, s)| hit_of(&chunks[i], s, query))
        .collect()
}

fn snippet(text: &str, query: &str, max: usize) -> String {
    // v45.14：窗口统一在「字符」空间算。旧实现拿 str::find 的**字节**偏移当字符下标，
    // 中文文档里 end < start 会让 end - start 下溢（debug 直接 panic），
    // 表现为「检索一段中文就崩」；英文文本字节≈字符才碰巧不炸。
    let chars: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.chars().take(40).collect();
    let center = if q.is_empty() {
        0
    } else {
        chars.windows(q.len()).position(|w| w == q.as_slice()).unwrap_or(0)
    };
    let start = center.saturating_sub(50);
    let end = (start + max).min(chars.len());
    chars[start..end].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(path: &str, text: &str) -> Chunk {
        Chunk { file_path: path.into(), char_offset: 0, heading_path: vec![], text: text.into() }
    }

    #[test]
    fn snippet_chinese_near_tail_no_panic() {
        // 命中位置靠后 + 中文多字节：旧实现会在这里 subtract overflow panic。
        let text = format!("{}关键词{}", "前".repeat(80), "后".repeat(80));
        let s = snippet(&text, "关键词", 60);
        assert!(s.chars().count() <= 60, "截图不应超长，got {}", s.chars().count());
        assert!(s.contains("关键词"), "截图应包含命中词，got {}", s);
    }

    #[test]
    fn three_docs_relevant_ranking() {
        let chunks = vec![
            mk("deploy.md", "# 部署\n环境变量配置。设置 DOUBAO_API_KEY 后启动服务。"),
            mk("recipe.md", "# 烹饪\n红烧肉菜谱：五花肉切块，加糖酱油炖一小时。"),
            mk("legal.md", "# 法律条款\n本合同有效期一年，争议由仲裁解决。"),
        ];
        let hits = search(&chunks, "环境变量 部署", 3);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].file_path, "deploy.md", "Top1 应是部署文档，got {}", hits[0].file_path);
        // 至少 2 个命中才比较排序；只有 1 个也接受
        if hits.len() >= 2 {
            assert!(hits[0].score >= hits.last().unwrap().score);
        }
    }

    #[test]
    fn no_query_returns_empty() {
        let chunks = vec![mk("a.md", "hello")];
        assert!(search(&chunks, "", 3).is_empty());
    }

    #[test]
    fn english_tokenized() {
        let chunks = vec![
            mk("a.md", "docker compose up -d"),
            mk("b.md", "kubernetes pods service"),
        ];
        let hits = search(&chunks, "docker", 3);
        assert_eq!(hits[0].file_path, "a.md");
    }

    #[test]
    fn snippet_around_query() {
        let chunks = vec![mk("a.md", "前置内容。环境变量 DOUBAO_API_KEY 配置在此。后续内容。")];
        let hits = search(&chunks, "环境变量", 1);
        assert!(hits[0].snippet.contains("环境变量"));
    }

    /// 造一个「造得出来但语义明确」的索引：两行，第一行与 query 同向。
    fn fake_vectors(chunks: &[Chunk], q: &[f32], semantic_row: &[f32]) -> super::super::vectors::VectorIndex {
        let mut entries = Vec::new();
        let mut data = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            entries.push(super::super::vectors::VecEntry {
                key: super::super::vectors::key_of(c),
                hash: super::super::vectors::text_hash(&c.text),
            });
            let row: Vec<f32> = if i == 0 { semantic_row.to_vec() } else {
                // 与 query 正交的方向：语义上毫不相关
                let mut v = vec![0f32; q.len()];
                v[q.iter().position(|x| *x == 0.0).unwrap_or(1)] = 1.0;
                v
            };
            data.extend_from_slice(&row);
        }
        super::super::vectors::VectorIndex {
            model: super::super::embed::MODEL_NAME.to_string(),
            dim: q.len(),
            entries,
            data,
        }
    }

    /// 语义命中：查询词与目标块字面不重合时，混合检索必须把语义最接近的块顶上来。
    #[test]
    fn hybrid_lifts_semantic_hit_without_literal_overlap() {
        let chunks = vec![
            mk("a.md", "数据库连接池耗尽，请求排队等待超时"),
            mk("b.md", "今天天气晴朗，适合去公园散步"),
        ];
        // query 与两块都没有共同 token
        let query = "服务端应用程序无法作业";
        // 让第一块与 query 同向、第二块正交
        let mut qv = vec![0f32; 4];
        qv[0] = 1.0;
        let sem = vec![1.0f32, 0.0, 0.0, 0.0];
        let idx = fake_vectors(&chunks, &qv, &sem);
        let hits = super::search_hybrid(&chunks, query, 2, Some(&qv), Some(&idx), 0.45);
        assert!(!hits.is_empty(), "至少应返回语义命中的块");
        assert_eq!(hits[0].file_path, "a.md", "字面不重合也要靠语义排第一");
        // 纯 BM25 在这句查询下命中不了 a.md：说明这条排序确实来自向量分
        let bm_only = search(&chunks, query, 2);
        assert!(bm_only.is_empty() || bm_only[0].file_path != "a.md");
    }

    /// 权重 0 = 退化成纯 BM25：语义信息不得影响排序。
    #[test]
    fn hybrid_weight_zero_matches_bm25() {
        let chunks = vec![
            mk("a.md", "数据库连接池耗尽，请求排队等待超时"),
            mk("b.md", "今天天气晴朗，适合去公园散步"),
        ];
        let query = "数据库超时";
        let qv = vec![0f32, 1.0];
        let idx = fake_vectors(&chunks, &qv, &vec![0f32, 1.0]); // 语义指向 b.md
        let hy = super::search_hybrid(&chunks, query, 2, Some(&qv), Some(&idx), 0.0);
        let plain = search(&chunks, query, 2);
        assert_eq!(hy.len(), plain.len());
        for (a, b) in hy.iter().zip(plain.iter()) {
            assert_eq!(a.file_path, b.file_path, "w=0 时排序必须与纯 BM25 一致");
        }
    }

    /// 向量缺席（没装模型）时自动退回 BM25，不能报错也不能空结果。
    #[test]
    fn hybrid_falls_back_to_bm25_without_vectors() {
        let chunks = vec![mk("a.md", "数据库连接池耗尽")];
        let hits = super::search_hybrid(&chunks, "数据库", 3, None, None, 0.45);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_path, "a.md");
    }
}
