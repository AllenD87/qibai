//! 事实卡（FactCard）抽取与淘汰。
//!
//! 规格：方案 6.2 / 6.3——数字、日期、文件路径、人名、接口名、结论性判断，
//! 每条带来源 message id；**不参与压缩**，超上限按最近优先淘汰进 archive 可回查。
//!
//! 抽取规则要"可解释、可单测"：每条事实都能回溯到原文中的一段 token；
//! 不做语义级抽取（那是 P1/P2 的事）。

use serde::{Deserialize, Serialize};

/// 与 layers 模块共享的消息结构——为避免循环依赖，这里再定义一次最小结构，
/// 由 layers::Message 通过 From/Into 适配。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub ts: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FactKind {
    Number,
    Date,
    FilePath,
    Identifier,
    Conclusion,
}

impl FactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FactKind::Number => "number",
            FactKind::Date => "date",
            FactKind::FilePath => "path",
            FactKind::Identifier => "ident",
            FactKind::Conclusion => "conclusion",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactCard {
    pub id: String,
    pub kind: FactKind,
    pub value: String,
    pub source_msg_id: String,
    pub ts: u64,
    /// 是否已经被淘汰进 archive（true 时不再注入 L3，只可回查）。
    pub archived: bool,
}

/// 从消息列表抽取事实卡。同一 message 内同一 value 只取一次；
/// 返回按 (ts 升序, 先到先得) 排列，便于"最近优先"淘汰。
pub fn extract_facts(messages: &[FactMessage]) -> Vec<FactCard> {
    let mut out: Vec<FactCard> = Vec::new();
    for msg in messages {
        if msg.role == "system" {
            // 系统人设不进事实卡，避免污染。
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        for tok in extract_tokens(&msg.content) {
            if !seen.insert((tok.kind, tok.value.clone())) {
                continue;
            }
            out.push(FactCard {
                id: format!("fact::{}::{}", &msg.id, out.len()),
                kind: tok.kind,
                value: tok.value,
                source_msg_id: msg.id.clone(),
                ts: msg.ts,
                archived: false,
            });
        }
    }
    out
}

struct RawToken {
    kind: FactKind,
    value: String,
}

/// 简单的线性扫描 + 规则匹配，不引入 regex 依赖（保持 crate 零新增依赖）。
fn extract_tokens(text: &str) -> Vec<RawToken> {
    let mut v = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];

        // 1) 路径：以 ~ / . / / 开头，或中间含 / 的连续 token
        if c == '~' || c == '/' || c == '.' || c == '\\' {
            if let Some(tok) = scan_path(&chars, i) {
                if tok.value.len() >= 2 {
                    let len = tok.value.len();
                    v.push(tok);
                    i += len;
                    continue;
                }
            }
        }

        // 2) 日期：YYYY-MM-DD / YYYY/MM/DD / YYYY年M月D日
        if c.is_ascii_digit() && i + 3 < n && chars[i + 1].is_ascii_digit() && chars[i + 2].is_ascii_digit() && chars[i + 3].is_ascii_digit() {
            if let Some(tok) = scan_date(&chars, i) {
                let len = tok.value.len();
                v.push(tok);
                i += len;
                continue;
            }
        }

        // 3) 数字（含小数/百分比）
        if c.is_ascii_digit() {
            if let Some(tok) = scan_number(&chars, i) {
                let len = tok.value.len();
                v.push(tok);
                i += len;
                continue;
            }
        }

        // 4) 结论性判断：以"决定"/"确认"/"结论"开头，取到行尾或 80 字截断
        if (c == '决' || c == '确' || c == '结') && i + 1 < n {
            let head: String = chars[i..(i + 2).min(n)].iter().collect();
            if head == "决定" || head == "确认" || head == "结论" {
                let end = (i + 80).min(n);
                let body: String = chars[i..end].iter().collect();
                // 截到首个句末
                let cut = body.find(|ch: char| ch == '\n' || ch == '。' || ch == '；').unwrap_or(body.len());
                let body = body[..cut].trim().to_string();
                if !body.is_empty() {
                    v.push(RawToken { kind: FactKind::Conclusion, value: body });
                }
                i += end;
                continue;
            }
        }

        // 5) 标识符：snake_case / kebab-case / CamelCase 后紧跟 ( 或 ::
        if c.is_ascii_alphabetic() || c == '_' {
            if let Some(tok) = scan_ident(&chars, i) {
                let len = tok.value.len();
                v.push(tok);
                i += len;
                continue;
            }
        }

        i += 1;
    }
    v
}

fn scan_path(chars: &[char], start: usize) -> Option<RawToken> {
    let n = chars.len();
    let mut i = start;
    // 必须有 "/" 才算路径（避免单个 "." 或 "~" 被收进来）
    let mut has_slash = false;
    while i < n {
        let c = chars[i];
        if c == '/' || c == '\\' {
            has_slash = true;
            i += 1;
        } else if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '~' || c == ' ' && i > start + 1 {
            // 允许空格仅在中段（"My Documents/foo"），但首字符不允许。
            i += 1;
        } else {
            break;
        }
    }
    if !has_slash {
        return None;
    }
    let value: String = chars[start..i].iter().collect();
    let value = value.trim_end_matches(['/', '\\', '.', ' ']).to_string();
    if value.len() < 3 {
        return None;
    }
    Some(RawToken { kind: FactKind::FilePath, value })
}

fn scan_date(chars: &[char], start: usize) -> Option<RawToken> {
    let n = chars.len();
    // YYYY-MM-DD or YYYY/MM/DD
    if start + 9 <= n {
        let y: String = chars[start..start + 4].iter().collect();
        let sep1 = chars[start + 4];
        let m_ok = chars[start + 5].is_ascii_digit() && chars[start + 6].is_ascii_digit();
        let sep2 = chars[start + 7];
        let d_ok = chars[start + 8].is_ascii_digit() && start + 9 <= n && chars[start + 9].is_ascii_digit();
        if y.chars().all(|c| c.is_ascii_digit()) && (sep1 == '-' || sep1 == '/') && m_ok && sep2 == sep1 && d_ok {
            let end = start + 10;
            let value: String = chars[start..end].iter().collect();
            return Some(RawToken { kind: FactKind::Date, value });
        }
    }
    // YYYY年M月D日
    if start + 5 <= n && chars[start + 4] == '年' {
        let mut end = start + 5;
        while end < n && chars[end].is_ascii_digit() { end += 1; }
        if end < n && chars[end] == '月' {
            end += 1;
            let ds = end;
            while end < n && chars[end].is_ascii_digit() { end += 1; }
            if end < n && chars[end] == '日' && end > ds {
                let value: String = chars[start..=end].iter().collect();
                return Some(RawToken { kind: FactKind::Date, value });
            }
        }
    }
    None
}

fn scan_number(chars: &[char], start: usize) -> Option<RawToken> {
    let n = chars.len();
    let mut i = start;
    while i < n && chars[i].is_ascii_digit() { i += 1; }
    // 小数
    if i < n && chars[i] == '.' && i + 1 < n && chars[i + 1].is_ascii_digit() {
        i += 1;
        while i < n && chars[i].is_ascii_digit() { i += 1; }
    }
    // 百分比
    if i < n && chars[i] == '%' { i += 1; }
    let value: String = chars[start..i].iter().collect();
    // 至少 1 位数字，且不要被日期的前 4 位重复收走（日期由 scan_date 先吃掉）
    if value.len() < 1 { return None; }
    Some(RawToken { kind: FactKind::Number, value })
}

fn scan_ident(chars: &[char], start: usize) -> Option<RawToken> {
    let n = chars.len();
    let mut i = start;
    while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '-') { i += 1; }
    let word: String = chars[start..i].iter().collect();
    if word.len() < 3 { return None; }
    // 必须后面紧跟 ( 或 :: 才算接口名
    if i + 1 < n && chars[i] == ':' && chars[i + 1] == ':' {
        return Some(RawToken { kind: FactKind::Identifier, value: word });
    }
    if i < n && chars[i] == '(' {
        return Some(RawToken { kind: FactKind::Identifier, value: word });
    }
    None
}

/// 超上限淘汰：按"最近优先"——ts 越大越新，超过 cap 的旧卡标 archived=true 并返回到 archive 列表。
/// 返回 (kept_active, archived)。archive 中的卡 `archived=true`，可回查但不再注入 L3。
pub fn enforce_cap(mut facts: Vec<FactCard>, cap: usize) -> (Vec<FactCard>, Vec<FactCard>) {
    if facts.len() <= cap {
        return (facts, Vec::new());
    }
    // 按 ts 升序，越靠前越旧。
    facts.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.id.cmp(&b.id)));
    let overflow_count = facts.len() - cap;
    let mut archived = Vec::new();
    for f in facts.iter_mut().take(overflow_count) {
        f.archived = true;
        archived.push(f.clone());
    }
    let kept: Vec<FactCard> = facts.into_iter().filter(|f| !f.archived).collect();
    (kept, archived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: &str, content: &str, ts: u64) -> FactMessage {
        FactMessage { id: id.into(), role: role.into(), content: content.into(), ts }
    }

    #[test]
    fn extract_numbers_dates_paths_idents() {
        let m = msg(
            "m1",
            "user",
            "预算 8000 tokens，截止 2026-11-04，产物写到 /home/user/proj/out.zip，调用 compress() 完成。",
            1,
        );
        let facts = extract_facts(&[m]);
        let kinds: Vec<FactKind> = facts.iter().map(|f| f.kind).collect();
        // 至少出现 Number / Date / FilePath / Identifier
        assert!(kinds.contains(&FactKind::Number), "应抽到数字：{:?}", kinds);
        assert!(kinds.contains(&FactKind::Date), "应抽到日期：{:?}", kinds);
        assert!(kinds.contains(&FactKind::FilePath), "应抽到路径：{:?}", kinds);
        assert!(kinds.contains(&FactKind::Identifier), "应抽到标识符：{:?}", kinds);
        // 每条都带来源 message id
        assert!(facts.iter().all(|f| f.source_msg_id == "m1"));
        // 数字值包含 8000
        assert!(facts.iter().any(|f| f.kind == FactKind::Number && f.value.contains("8000")));
        // 日期值就是 2026-11-04
        assert!(facts.iter().any(|f| f.kind == FactKind::Date && f.value == "2026-11-04"));
        // 路径以 /home/user/proj 开头
        assert!(facts.iter().any(|f| f.kind == FactKind::FilePath && f.value.starts_with("/home/user/proj")));
    }

    #[test]
    fn conclusion_facts_collected() {
        let m = msg("m2", "assistant", "决定采用四层组装，预算 8000。", 2);
        let facts = extract_facts(&[m]);
        assert!(facts.iter().any(|f| f.kind == FactKind::Conclusion && f.value.starts_with("决定采用")));
    }

    #[test]
    fn system_messages_skipped() {
        let m = msg("m3", "system", "系统提示里有 9999 和 /etc/passwd", 3);
        let facts = extract_facts(&[m]);
        assert!(facts.is_empty(), "system 消息不抽事实卡：{:?}", facts);
    }

    #[test]
    fn enforce_cap_archives_oldest() {
        let mut facts = Vec::new();
        for i in 0..10 {
            facts.push(FactCard {
                id: format!("f{i}"),
                kind: FactKind::Number,
                value: format!("v{i}"),
                source_msg_id: format!("m{i}"),
                ts: i,
                archived: false,
            });
        }
        let (kept, archived) = enforce_cap(facts, 4);
        assert_eq!(kept.len(), 4);
        assert_eq!(archived.len(), 6);
        // 保留的应该是 ts 最大的 4 个（v6..v9）
        let kept_values: Vec<&str> = kept.iter().map(|f| f.value.as_str()).collect();
        assert_eq!(kept_values, vec!["v6", "v7", "v8", "v9"]);
        // archive 里的全标 archived
        assert!(archived.iter().all(|f| f.archived));
        // 被淘汰的 id 可回查
        let archived_ids: Vec<&str> = archived.iter().map(|f| f.id.as_str()).collect();
        assert!(archived_ids.contains(&"f0"));
    }

    #[test]
    fn cap_not_exceeded_no_archive() {
        let facts = vec![
            FactCard { id: "a".into(), kind: FactKind::Number, value: "1".into(), source_msg_id: "m1".into(), ts: 1, archived: false },
        ];
        let (kept, archived) = enforce_cap(facts.clone(), 10);
        assert_eq!(kept.len(), 1);
        assert!(archived.is_empty());
    }
}
