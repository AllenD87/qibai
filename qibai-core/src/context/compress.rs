//! 压缩 v1（方案 6.2–6.4）。
//!
//! - 原文层：最近 N 轮逐字保留；
//! - 摘要层：结构化（目标 / 已确认决策 / 关键结论 / 未决问题 / 产物清单），每条带来源 message id；
//! - 事实卡层：不参与压缩，超上限按最近优先淘汰进 archive；
//! - **防丢自校验**：摘要生成后自校验——摘要里出现的数字与路径必须在原文找得到，找不到就回填原文片段并标 `backfilled=true`；
//! - **不静默降级**：provider 报错/超时 → 保留原文 + 标 `compression=degraded`；
//! - `parent_snapshot` 链：每次压缩都返回 dropped_ids，快照层用它串链。

use serde::{Deserialize, Serialize};

use crate::context::budget::ContextConfig;
use crate::context::facts::{enforce_cap, extract_facts, FactCard, FactMessage};
use crate::context::layers::Message;

/// 结构化摘要。每个字段都是 Vec<(item, source_msg_id)>，便于回溯。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    pub goal: String,
    pub decisions: Vec<(String, String)>,
    pub conclusions: Vec<(String, String)>,
    pub open_questions: Vec<(String, String)>,
    pub artifacts: Vec<(String, String)>,
    /// 自校验回填项：摘要里引用了、但原文找不到的锚点 token，被回填到这里。
    pub backfilled: Vec<Backfill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Backfill {
    /// 摘要中出现的锚点 token（数字/路径/标识符）。
    pub extracted: String,
    /// 回填来源消息 id。
    pub source_msg_id: String,
    /// 从原文回填的片段（含该 token 的一小句）。
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressionState {
    /// 没触发压缩。
    Fresh,
    /// 正常压缩。
    Compressed,
    /// provider 失败，保留原文，不静默降级。
    Degraded,
}

impl CompressionState {
    pub fn as_str(self) -> &'static str {
        match self {
            CompressionState::Fresh => "fresh",
            CompressionState::Compressed => "compressed",
            CompressionState::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionOutcome {
    pub state: CompressionState,
    pub summary: Option<Summary>,
    /// 最近 N 轮逐字保留的消息。
    pub kept_messages: Vec<Message>,
    /// 被压成摘要的消息 id 列表（用于 parent_snapshot 链）。
    pub dropped_message_ids: Vec<String>,
    /// 活跃事实卡。
    pub facts: Vec<FactCard>,
    /// 被淘汰进 archive 的事实卡（可回查）。
    pub archived_facts: Vec<FactCard>,
    /// 本次是否发生过回填。
    pub backfilled: bool,
    /// provider 失败时的错误信息（Degraded 时非空）。
    pub error: Option<String>,
}

/// 摘要生成器 trait——测试里给假实现，生产里由 engine 接线到 LLM。
pub trait SummaryProvider {
    /// 给一批"被压的旧消息"，返回结构化摘要。
    /// 实现失败应返回 Err(String)，由上层转成 Degraded。
    fn summarize(&self, old_messages: &[FactMessage]) -> Result<Summary, String>;
}

/// 压缩入口。
///
/// 流程：
/// 1. 按 `cfg.recent_turns` 切出"最近保留"与"待压缩"两段；
/// 2. 待压缩段为空 → 直接 Fresh 返回；
/// 3. 调 provider.summarize；失败 → Degraded（保留全部原文）；
/// 4. 成功 → 自校验（[`verify_summary`]）+ 抽取/合并事实卡 + cap 淘汰；
/// 5. 返回 outcome。
pub fn compress(
    messages: &[Message],
    existing_facts: Vec<FactCard>,
    cfg: &ContextConfig,
    provider: &dyn SummaryProvider,
) -> CompressionOutcome {
    let c = cfg.clamped();
    if messages.is_empty() {
        return CompressionOutcome {
            state: CompressionState::Fresh,
            summary: None,
            kept_messages: Vec::new(),
            dropped_message_ids: Vec::new(),
            facts: existing_facts,
            archived_facts: Vec::new(),
            backfilled: false,
            error: None,
        };
    }

    // 切最近 N 轮。消息按 ts 升序由调用方保证，这里直接按数组末尾切。
    let split = messages.len().saturating_sub(c.recent_turns);
    let kept: Vec<Message> = messages[split..].to_vec();
    let old: Vec<Message> = messages[..split].to_vec();

    if old.is_empty() {
        // 消息总数 ≤ recent_turns：无需压缩。
        let old_fm: Vec<FactMessage> = old.clone().into_iter().map(Into::into).collect();
        let mut new_facts = extract_facts(&old_fm);
        new_facts.extend(existing_facts);
        let (facts, archived_facts) = enforce_cap(new_facts, c.fact_card_cap);
        return CompressionOutcome {
            state: CompressionState::Fresh,
            summary: None,
            kept_messages: kept,
            dropped_message_ids: Vec::new(),
            facts,
            archived_facts,
            backfilled: false,
            error: None,
        };
    }

    let old_fm: Vec<FactMessage> = old.clone().into_iter().map(Into::into).collect();

    match provider.summarize(&old_fm) {
        Ok(mut summary) => {
            let backfilled = verify_summary(&mut summary, &old_fm);
            // 抽取事实卡：从被压缩的旧消息里抽，合并到 existing_facts 里。
            let mut new_facts = extract_facts(&old_fm);
            new_facts.extend(existing_facts);
            let (facts, archived_facts) = enforce_cap(new_facts, c.fact_card_cap);
            CompressionOutcome {
                state: CompressionState::Compressed,
                summary: Some(summary),
                kept_messages: kept,
                dropped_message_ids: old.iter().map(|m| m.id.clone()).collect(),
                facts,
                archived_facts,
                backfilled,
                error: None,
            }
        }
        Err(e) => {
            // 不静默降级：保留全部原文，标 Degraded。
            let (facts, archived_facts) = enforce_cap(existing_facts, c.fact_card_cap);
            CompressionOutcome {
                state: CompressionState::Degraded,
                summary: None,
                kept_messages: messages.to_vec(),
                dropped_message_ids: Vec::new(),
                facts,
                archived_facts,
                backfilled: false,
                error: Some(e),
            }
        }
    }
}

/// 自校验：摘要里出现的数字/路径/标识符，必须在 old 原文里找得到。
/// 找不到的锚点 token → 回填（从最接近的一条旧消息里取一小段），并记到 `summary.backfilled`。
/// 返回是否发生过回填。
pub fn verify_summary(summary: &mut Summary, old: &[FactMessage]) -> bool {
    let old_blob: String = old.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n");
    let anchors = collect_anchors(summary);
    let mut backfilled = false;
    for tok in anchors {
        if old_blob.contains(&tok) {
            continue;
        }
        // 找不到 → 回填：在 old 里挑一条含最接近 token 的消息，截一小段。
        if let Some((mid, snippet)) = find_nearest_snippet(&tok, old) {
            summary.backfilled.push(Backfill {
                extracted: tok,
                source_msg_id: mid,
                snippet,
            });
            backfilled = true;
        }
    }
    backfilled
}

fn collect_anchors(summary: &Summary) -> Vec<String> {
    let mut v = Vec::new();
    let mut push_from = |s: &str| {
        for tok in scan_anchor_tokens(s) {
            if !v.contains(&tok) { v.push(tok); }
        }
    };
    push_from(&summary.goal);
    for (item, _) in &summary.decisions { push_from(item); }
    for (item, _) in &summary.conclusions { push_from(item); }
    for (item, _) in &summary.open_questions { push_from(item); }
    for (item, _) in &summary.artifacts { push_from(item); }
    v
}

/// 扫描一段文本里的"锚点 token"：纯数字（含小数/百分比）、路径、标识符。
fn scan_anchor_tokens(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < n && chars[i].is_ascii_digit() { i += 1; }
            if i < n && chars[i] == '.' && i + 1 < n && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < n && chars[i].is_ascii_digit() { i += 1; }
            }
            if i < n && chars[i] == '%' { i += 1; }
            let tok: String = chars[start..i].iter().collect();
            if tok.len() >= 2 { v.push(tok); }
            continue;
        }
        if c == '/' || c == '~' || c == '.' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '/' || chars[i] == '_' || chars[i] == '.' || chars[i] == '-' || chars[i] == '~') {
                i += 1;
            }
            let tok: String = chars[start..i].iter().collect();
            if tok.len() >= 3 && tok.contains('/') { v.push(tok); }
            continue;
        }
        i += 1;
    }
    v
}

/// 在旧消息里找一条"最接近 token"的，返回 (msg_id, snippet)。
/// 接近度：先按数字探针；无数字时按路径中的字母段；再找不到就取第一条旧消息兜底。
/// 兜底也必须返回——防丢自校验的核心是"找不到就回填"，绝不允许静默跳过。
fn find_nearest_snippet(tok: &str, old: &[FactMessage]) -> Option<(String, String)> {
    if old.is_empty() { return None; }

    // 探针 1：tok 中的数字串。
    let digit_probe: String = tok.chars().filter(|c| c.is_ascii_digit()).collect();
    // 探针 2：tok 中最长的字母数字段（路径中段，如 "fake"、"config"）。
    let word_probe = longest_alpha_segment(tok);

    let mut best: Option<(usize, &FactMessage)> = None;
    // 探针 1：数字
    for m in old.iter() {
        if !digit_probe.is_empty() && m.content.contains(&digit_probe) {
            let score = m.content.matches(&digit_probe).count();
            match best {
                None => best = Some((score, m)),
                Some((s, _)) if score > s => best = Some((score, m)),
                _ => {}
            }
        }
    }
    // 探针 2：最长字母段（仅在数字探针没命中时）
    if best.is_none() {
        for m in old.iter() {
            if !word_probe.is_empty() && m.content.contains(&word_probe) {
                let score = m.content.matches(&word_probe).count();
                match best {
                    None => best = Some((score, m)),
                    Some((s, _)) if score > s => best = Some((score, m)),
                    _ => {}
                }
            }
        }
    }

    let (_, m) = match best {
        Some(b) => b,
        // 都找不到 → 兜底取第一条旧消息的前 80 字，保证回填一定发生。
        None => {
            let m = &old[0];
            let snippet: String = m.content.chars().take(80).collect();
            return Some((m.id.clone(), snippet));
        }
    };

    // 截一段含探针的句子（前后各 40 字）。
    let idx_probe = if !digit_probe.is_empty() { &digit_probe } else { &word_probe };
    let idx = m.content.find(idx_probe).unwrap_or(0);
    let start = idx.saturating_sub(40);
    let end = (idx + idx_probe.len() + 40).min(m.content.len());
    let snippet: String = m.content[start..end].chars().collect();
    Some((m.id.clone(), snippet))
}

/// 从 tok 里找最长的连续字母数字段（用于无数字路径的回填探针）。
fn longest_alpha_segment(tok: &str) -> String {
    let mut best = String::new();
    let mut cur = String::new();
    for c in tok.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c);
        } else {
            if cur.len() > best.len() { best = cur.clone(); }
            cur.clear();
        }
    }
    if cur.len() > best.len() { best = cur; }
    best
}

/// 把结构化摘要渲染成一段给 L3 用的文本。
pub fn render_summary_block(s: &Summary) -> String {
    let mut out = String::new();
    if !s.goal.is_empty() {
        out.push_str(&format!("目标：{}\n", s.goal));
    }
    if !s.decisions.is_empty() {
        out.push_str("已确认决策：\n");
        for (item, src) in &s.decisions { out.push_str(&format!("- {}（来源 {}）\n", item, src)); }
    }
    if !s.conclusions.is_empty() {
        out.push_str("关键结论：\n");
        for (item, src) in &s.conclusions { out.push_str(&format!("- {}（来源 {}）\n", item, src)); }
    }
    if !s.open_questions.is_empty() {
        out.push_str("未决问题：\n");
        for (item, src) in &s.open_questions { out.push_str(&format!("- {}（来源 {}）\n", item, src)); }
    }
    if !s.artifacts.is_empty() {
        out.push_str("产物清单：\n");
        for (item, src) in &s.artifacts { out.push_str(&format!("- {}（来源 {}）\n", item, src)); }
    }
    if !s.backfilled.is_empty() {
        out.push_str("防丢回填（摘要引用但原文直搜未命中，已回填）：\n");
        for b in &s.backfilled {
            out.push_str(&format!("- [{}] 原文片段：{}（来源 {}）\n", b.extracted, b.snippet, b.source_msg_id));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: &str, content: &str, ts: u64) -> Message {
        Message { id: id.into(), role: role.into(), content: content.into(), ts }
    }

    /// 假 provider：返回固定摘要。
    struct FakeProvider {
        summary: Result<Summary, String>,
    }
    impl SummaryProvider for FakeProvider {
        fn summarize(&self, _old: &[FactMessage]) -> Result<Summary, String> {
            self.summary.clone()
        }
    }

    #[test]
    fn compress_short_messages_noop_fresh() {
        let cfg = ContextConfig::default(); // recent_turns=10
        let msgs: Vec<Message> = (0..5).map(|i| msg(&format!("m{}", i), "user", &format!("内容{}", i), i)).collect();
        let p = FakeProvider { summary: Ok(Summary::default()) };
        let out = compress(&msgs, Vec::new(), &cfg, &p);
        assert_eq!(out.state, CompressionState::Fresh);
        assert!(out.dropped_message_ids.is_empty());
        assert_eq!(out.kept_messages.len(), 5);
    }

    #[test]
    fn compress_triggers_on_long_session() {
        let cfg = ContextConfig::default(); // recent_turns=10
        let msgs: Vec<Message> = (0..30).map(|i| msg(&format!("m{}", i), "user", &format!("讨论第{}轮 预算 8000", i), i)).collect();
        let summary = Summary {
            goal: "做 demo".into(),
            decisions: vec![("采用四层".into(), "m5".into())],
            ..Default::default()
        };
        let p = FakeProvider { summary: Ok(summary) };
        let out = compress(&msgs, Vec::new(), &cfg, &p);
        assert_eq!(out.state, CompressionState::Compressed);
        // 保留最近 10 条
        assert_eq!(out.kept_messages.len(), 10);
        assert_eq!(out.kept_messages[0].id, "m20");
        // dropped 是前 20 条 id
        assert_eq!(out.dropped_message_ids.len(), 20);
        assert_eq!(out.dropped_message_ids[0], "m0");
        // 数字 8000 与路径在原文（kept+old）中——这里主要验证 dropped 链完整
        assert!(out.dropped_message_ids.contains(&"m0".to_string()));
    }

    #[test]
    fn verify_summary_backfills_missing_path() {
        // 旧消息里根本没有 /fake/path.txt
        let old = vec![
            FactMessage { id: "m1".into(), role: "user".into(), content: "预算 8000 已确认。".into(), ts: 1 },
            FactMessage { id: "m2".into(), role: "assistant".into(), content: "决定用四层组装。".into(), ts: 2 },
        ];
        let mut summary = Summary {
            goal: "交付到 /fake/path.txt".into(), // 这个路径不在原文
            ..Default::default()
        };
        let backfilled = verify_summary(&mut summary, &old);
        assert!(backfilled, "应触发回填");
        assert!(!summary.backfilled.is_empty());
        assert!(summary.backfilled[0].extracted.contains("/fake/path.txt"));
        // 回填了某条旧消息的片段
        assert!(!summary.backfilled[0].snippet.is_empty());
    }

    #[test]
    fn verify_summary_no_backfill_when_anchor_present() {
        let old = vec![FactMessage {
            id: "m1".into(), role: "user".into(),
            content: "预算 8000 写到 /home/user/out.zip。".into(), ts: 1,
        }];
        let mut summary = Summary {
            goal: "写到 /home/user/out.zip 预算 8000".into(),
            ..Default::default()
        };
        let backfilled = verify_summary(&mut summary, &old);
        assert!(!backfilled);
        assert!(summary.backfilled.is_empty());
    }

    #[test]
    fn provider_error_degrades_keeps_all_messages() {
        let cfg = ContextConfig::default();
        let msgs: Vec<Message> = (0..20).map(|i| msg(&format!("m{}", i), "user", &format!("内容{}", i), i)).collect();
        let p = FakeProvider { summary: Err("provider timeout".into()) };
        let out = compress(&msgs, Vec::new(), &cfg, &p);
        assert_eq!(out.state, CompressionState::Degraded);
        assert_eq!(out.kept_messages.len(), 20, "Degraded 必须保留全部原文");
        assert!(out.summary.is_none());
        assert!(out.error.as_ref().unwrap().contains("timeout"));
    }

    #[test]
    fn facts_extracted_and_archived_on_cap() {
        let cfg = ContextConfig { fact_card_cap: 5, ..ContextConfig::default() };
        // 30 条消息，每条含一个独特数字 → 抽出 30 张事实卡，cap=5 应淘汰 25 张进 archive
        let msgs: Vec<Message> = (0..30)
            .map(|i| msg(&format!("m{}", i), "user", &format!("第 {} 轮", i * 100 + 1), i))
            .collect();
        let p = FakeProvider { summary: Ok(Summary::default()) };
        let out = compress(&msgs, Vec::new(), &cfg, &p);
        assert_eq!(out.state, CompressionState::Compressed);
        // 活跃卡 ≤ cap
        assert!(out.facts.len() <= 5, "活跃事实卡超 cap：{}", out.facts.len());
        // archive 非空
        assert!(!out.archived_facts.is_empty(), "应淘汰进 archive");
        // archive 里的都标 archived
        assert!(out.archived_facts.iter().all(|f| f.archived));
        // 被淘汰的卡仍能通过 id 回查（source_msg_id 非空）
        assert!(out.archived_facts.iter().all(|f| !f.source_msg_id.is_empty()));
    }

    #[test]
    fn dropped_ids_chainable_for_parent_snapshot() {
        // 连续 3 次压缩，每次都返回 dropped ids，能一路回溯到原始 message id。
        let cfg = ContextConfig { recent_turns: 5, ..ContextConfig::default() };
        // 初始 30 条消息
        let mut msgs: Vec<Message> = (0..30).map(|i| msg(&format!("m{}", i), "user", &format!("内容{}", i), i)).collect();
        let p = FakeProvider { summary: Ok(Summary::default()) };
        let mut all_dropped: Vec<String> = Vec::new();

        for round in 0..3 {
            let out = compress(&msgs, Vec::new(), &cfg, &p);
            all_dropped.extend(out.dropped_message_ids.clone());
            // 下一轮的输入 = kept + 再追加几条新消息
            let mut next = out.kept_messages.clone();
            let new_start = 30 + round * 3;
            for i in 0..3 {
                next.push(msg(&format!("n{}", new_start + i), "user", &format!("新消息{}", i), 100 + round as u64 * 3 + i as u64));
            }
            msgs = next;
        }
        // 应该 dropped 过 m0..m9 这些早期消息
        assert!(all_dropped.contains(&"m0".to_string()), "应能回溯到原始 m0：{:?}", all_dropped);
        assert!(all_dropped.contains(&"m5".to_string()));
    }

    #[test]
    fn anchors_preserved_after_compression_in_rendered_l3() {
        // 长会话压缩后，关键数字与路径仍能在最终上下文里找到。
        let cfg = ContextConfig::default();
        let mut msgs: Vec<Message> = Vec::new();
        for i in 0..30 {
            if i == 3 {
                msgs.push(msg("m3", "user", "最终产物写到 /home/x/out.zip，预算 8000 tokens。", 3));
            } else {
                msgs.push(msg(&format!("m{}", i), "user", &format!("普通寒暄句 {}", i), i));
            }
        }
        let p = FakeProvider {
            summary: Ok(Summary {
                goal: "交付 /home/x/out.zip 预算 8000".into(),
                ..Default::default()
            }),
        };
        let out = compress(&msgs, Vec::new(), &cfg, &p);
        // 关键路径要么在 kept_messages 里（m3 是第 3 条，会被压），要么在 facts 里，要么在 summary 里
        let mut blob = String::new();
        for m in &out.kept_messages { blob.push_str(&m.content); }
        for f in &out.facts { blob.push_str(&f.value); }
        if let Some(s) = &out.summary {
            blob.push_str(&s.goal);
            for (i, _) in &s.decisions { blob.push_str(i); }
        }
        assert!(blob.contains("/home/x/out.zip"), "压缩后路径丢失：{}", blob);
        assert!(blob.contains("8000"), "压缩后数字丢失：{}", blob);
    }
}
