//! v45.10 P1：Markdown 切块。
//!
//! 规则：
//! - 按 `#`/`##`/`###` 标题行切；同标题下内容超过 target_chars 再按段落兜底切
//! - 单块目标 `target_chars`（默认 700），重叠 `overlap` 字（默认 80）
//! - 每块记 `file_path` / `char_offset` / `heading_path`
//! - 不许空块；短文件（< target）整篇一块
//! - 只认 `.md`/`.txt`

use super::KbError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chunk {
    pub file_path: String,
    pub char_offset: usize,
    pub heading_path: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkConfig {
    pub target_chars: usize,
    pub overlap: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self { target_chars: 700, overlap: 80 }
    }
}

/// 切一个文件。`path` 用于记录 file_path（不读 mtime，那是 index 的事）。
pub fn chunk_file(path: &str, content: &str, cfg: ChunkConfig) -> Result<Vec<Chunk>, KbError> {
    // 短文件整篇一块
    if content.chars().count() <= cfg.target_chars {
        let hpath = first_heading(content);
        return Ok(vec![Chunk {
            file_path: path.to_string(),
            char_offset: 0,
            heading_path: hpath,
            text: content.trim().to_string(),
        }]);
    }

    // 按标题切段
    let sections = split_by_headings(content);
    let mut out = Vec::new();
    let mut char_off = 0usize;
    for sec in &sections {
        let chunks = split_long_section(path, sec, char_off, cfg);
        char_off += sec.body.chars().count();
        out.extend(chunks);
    }
    // 去空
    out.retain(|c| !c.text.trim().is_empty());
    Ok(out)
}

fn first_heading(content: &str) -> Vec<String> {
    for line in content.lines() {
        let t = line.trim_start();
        let h = heading_level(t);
        if h > 0 {
            return vec![t.trim_start_matches('#').trim().to_string()];
        }
    }
    Vec::new()
}

/// 返回 level（0=非标题）。
fn heading_level(line: &str) -> usize {
    let t = line.trim_start();
    if !t.starts_with('#') { return 0; }
    let n = t.bytes().take_while(|&b| b == b'#').count();
    if n > 6 { return 0; }
    // 标题后必须跟空格或行尾
    let rest = &t[n..];
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') { n } else { 0 }
}

struct Section {
    heading_path: Vec<String>,
    body: String,
}

fn split_by_headings(content: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut hstack: Vec<String> = Vec::new();
    let mut cur_heading: Vec<String> = Vec::new();
    let mut cur_body = String::new();

    for line in content.lines() {
        let lvl = heading_level(line);
        if lvl > 0 {
            // 先收掉上一段（take 清空 cur_body）
            if !cur_body.trim().is_empty() || !cur_heading.is_empty() {
                let body = std::mem::take(&mut cur_body);
                sections.push(Section { heading_path: cur_heading.clone(), body });
            }
            // 先更新标题栈，再让新段用新栈
            let title = line.trim_start_matches('#').trim().to_string();
            hstack.truncate(lvl - 1);
            if hstack.len() < lvl { hstack.push(title); }
            cur_heading = hstack.clone();
        } else {
            cur_body.push_str(line);
            cur_body.push('\n');
        }
    }
    if !cur_body.trim().is_empty() || !cur_heading.is_empty() {
        sections.push(Section { heading_path: cur_heading, body: cur_body });
    }
    sections
}

fn split_long_section(path: &str, sec: &Section, base_offset: usize, cfg: ChunkConfig) -> Vec<Chunk> {
    let body: &str = &sec.body;
    let chars: Vec<char> = body.chars().collect();
    if chars.len() <= cfg.target_chars {
        if body.trim().is_empty() { return Vec::new(); }
        return vec![Chunk {
            file_path: path.to_string(),
            char_offset: base_offset,
            heading_path: sec.heading_path.clone(),
            text: body.trim().to_string(),
        }];
    }
    // 按段落（空行）分块；累积到 target_chars 就切
    let mut out = Vec::new();
    let paras: Vec<&str> = body.split("\n\n").collect();
    let mut buf = String::new();
    let mut buf_start = 0usize; // 相对 body 的字符偏移
    for para in paras {
        // 单段过长：硬切
        if para.chars().count() > cfg.target_chars {
            if !buf.trim().is_empty() {
                out.push(Chunk { file_path: path.into(), char_offset: base_offset + buf_start, heading_path: sec.heading_path.clone(), text: buf.trim().to_string() });
                buf.clear();
            }
            let pchars: Vec<char> = para.chars().collect();
            let mut i = 0;
            while i < pchars.len() {
                let end = (i + cfg.target_chars).min(pchars.len());
                let piece: String = pchars[i..end].iter().collect();
                out.push(Chunk { file_path: path.into(), char_offset: base_offset + i, heading_path: sec.heading_path.clone(), text: piece.trim().to_string() });
                // 根因修复：尾段 end 被截到 pchars.len() 后，若仍按 overlap 回退 i，
                // 下一轮 end 仍等于 len、piece 仍是同一段、i 原地不动 → 死循环（挂死 60s）。
                // 已切到末尾即止，不再回退重叠（末尾没有下一块需要衔接）。
                if end == pchars.len() { break; }
                i = end.saturating_sub(cfg.overlap);
            }
            continue;
        }
        if buf.chars().count() + para.chars().count() > cfg.target_chars && !buf.is_empty() {
            out.push(Chunk {
                file_path: path.to_string(),
                char_offset: base_offset + buf_start,
                heading_path: sec.heading_path.clone(),
                text: buf.trim().to_string(),
            });
            // 重叠：保留末尾 overlap 字
            let keep = buf.chars().count().saturating_sub(cfg.overlap);
            let tail: String = buf.chars().skip(keep).collect();
            let consumed: usize = buf.chars().count();
            buf = tail;
            buf_start += consumed - cfg.overlap;
        }
        if !buf.is_empty() { buf.push('\n'); }
        buf.push_str(para);
    }
    if !buf.trim().is_empty() {
        out.push(Chunk {
            file_path: path.to_string(),
            char_offset: base_offset + buf_start,
            heading_path: sec.heading_path.clone(),
            text: buf.trim().to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_file_single_chunk() {
        let c = chunk_file("a.md", "hello world", ChunkConfig::default()).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "hello world");
    }

    #[test]
    fn heading_boundary_not_crossed() {
        let md = "# 部署\n安装步骤一安装步骤一安装步骤一安装步骤一。\n\n## 环境变量\n配置说明二配置说明二配置说明二配置说明二。\n";
        let c = chunk_file("b.md", md, ChunkConfig { target_chars: 30, overlap: 5 }).unwrap();
        // 两个二级段，不应跨标题合并
        let joined: String = c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join("||");
        assert!(!joined.contains("步骤一。## 环境变量"), "跨标题合并了: {joined}");
        assert!(c.iter().any(|x| x.heading_path == vec!["部署".to_string()]), "chunks: {:?}", c.iter().map(|x| x.heading_path.clone()).collect::<Vec<_>>());
        assert!(c.iter().any(|x| x.heading_path == vec!["部署".to_string(), "环境变量".to_string()]), "chunks: {:?}", c.iter().map(|x| x.heading_path.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn chunk_length_in_band() {
        let long: String = std::iter::repeat("这是一段测试文本。").take(200).collect();
        let c = chunk_file("long.md", &long, ChunkConfig { target_chars: 700, overlap: 80 }).unwrap();
        assert!(c.len() >= 2, "长文件应切多块: {}", c.len());
        for ch in &c {
            assert!(!ch.text.is_empty(), "空块");
        }
    }

    #[test]
    fn no_empty_chunk() {
        let md = "# 标题\n\n\n\n正文段落。\n";
        let c = chunk_file("e.md", md, ChunkConfig::default()).unwrap();
        assert!(c.iter().all(|x| !x.text.trim().is_empty()));
    }
}
