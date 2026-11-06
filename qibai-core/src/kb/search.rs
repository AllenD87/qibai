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

pub fn search(chunks: &[Chunk], query: &str, top_k: usize) -> Vec<KbHit> {
    if chunks.is_empty() || query.trim().is_empty() {
        return Vec::new();
    }
    let qtokens = tokenize(query);
    if qtokens.is_empty() { return Vec::new(); }

    // 文档频率
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

    let mut scores: Vec<(usize, f32)> = Vec::new();
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
        if s > 0.0 { scores.push((i, s)); }
    }
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores.into_iter().take(top_k).map(|(i, s)| {
        let c = &chunks[i];
        KbHit {
            file_path: c.file_path.clone(),
            char_offset: c.char_offset,
            heading_path: c.heading_path.clone(),
            score: (s * 1000.0).round() / 1000.0,
            snippet: snippet(&c.text, query, 200),
        }
    }).collect()
}

fn snippet(text: &str, query: &str, max: usize) -> String {
    let qfirst = query.chars().next().unwrap_or(' ');
    let idx = text.find(qfirst).unwrap_or(0);
    let start = idx.saturating_sub(50);
    let end = (start + max).min(text.chars().count());
    text.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(path: &str, text: &str) -> Chunk {
        Chunk { file_path: path.into(), char_offset: 0, heading_path: vec![], text: text.into() }
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
}
