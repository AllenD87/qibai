//! 四层上下文组装（方案 5.1）。
//!
//! L0 全局（人设/风格/安全底线）→ L1 技能（目录清单 + 按需全文）→
//! L2 任务简报（目标/约束/验收/产物/当前第几步）→ L3 上下文（最近原文 + 摘要 + 事实卡 + KB 片段）。
//!
//! 约束：
//! - 本模块不依赖 `qibai_core::task`（任务模块是另一条线）；
//! - L2 通过 [`TaskBrief`] 这个最小只读结构注入，由引擎接线时从 task.json 读出来填进来；
//! - 每层都按预算截断；截断时**不能把数字/路径拦腰切断**（见 [`smart_truncate`]）。

use serde::{Deserialize, Serialize};

use crate::context::budget::{estimate_tokens, LayerBudgets};
use crate::context::facts::FactCard;

/// 会话消息（L3 原文层用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    /// "user" | "assistant" | "system"（system 一般不进 L3）。
    pub role: String,
    pub content: String,
    pub ts: u64,
}

impl From<Message> for crate::context::facts::FactMessage {
    fn from(m: Message) -> Self {
        Self { id: m.id, role: m.role, content: m.content, ts: m.ts }
    }
}

/// 技能目录条目（L1 用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillBrief {
    pub name: String,
    pub description: String,
}

/// 本步需要全文注入的技能（L1 用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFull {
    pub name: String,
    pub body: String,
}

/// 任务简报（L2 用；与 task 模块解耦的最小只读结构）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBrief {
    pub goal: String,
    pub constraints: Vec<String>,
    pub acceptance: Vec<String>,
    pub artifacts_required: Vec<String>,
    pub current_step: usize,
    pub total_steps: usize,
    pub step_title: Option<String>,
}

/// KB 检索命中片段（L3 末尾注入，带出处）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbHit {
    pub chunk_id: String,
    pub source: String,
    pub snippet: String,
}

/// 已注入过的技能摘要（6.4：后续只留"名字 + 3 条要点"）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PriorSkillSummary {
    pub name: String,
    pub bullets: Vec<String>,
}

/// 组装四层所需的全部输入。
#[derive(Debug, Clone, Default)]
pub struct LayerInput {
    /// L0：完整人设 system prompt（通常来自 `persona::build_system_prompt`）。
    pub persona: String,
    /// L1：技能目录清单（名字 + 一句话）。
    pub skill_catalog: Vec<SkillBrief>,
    /// L1：本步需要全文注入的技能。
    pub skills_full: Vec<SkillFull>,
    /// L2：任务简报（无任务时为 None，L2 渲染为空段）。
    pub task_brief: Option<TaskBrief>,
    /// L3：最近若干轮原文（调用方已决定保留哪些）。
    pub messages: Vec<Message>,
    /// L3：结构化摘要（来自 compress 模块）。
    pub summary_block: Option<String>,
    /// L3：事实卡（活跃卡，不参与压缩）。
    pub facts: Vec<FactCard>,
    /// L3：KB 命中片段。
    pub kb_hits: Vec<KbHit>,
    /// L1 压缩：本步骤之前已经注入过全文的技能（按 6.4 只留名字 + 3 要点）。
    pub prior_skills: Vec<PriorSkillSummary>,
    /// 本步工具收窄提示（6.4：未使用工具 schema 不注入）。
    pub tool_hint: Option<String>,
}

/// 渲染后的四层文本 + token 统计。
#[derive(Debug, Clone, Default)]
pub struct RenderedLayers {
    pub l0: String,
    pub l1: String,
    pub l2: String,
    pub l3: String,
    /// 拼装后总 token（不含 reserve）。
    pub total_tokens: usize,
    /// 实际各层 token，便于快照与单测断言。
    pub tokens_l0: usize,
    pub tokens_l1: usize,
    pub tokens_l2: usize,
    pub tokens_l3: usize,
}

/// 按预算组装四层。每层独立截断，保证单层不超其预算；
/// 总 token = 各层实际 token 之和，≤ 总预算 - reserve。
pub fn assemble(input: &LayerInput, budgets: &LayerBudgets) -> RenderedLayers {
    let l0 = render_l0(&input.persona, budgets.l0);
    let l1 = render_l1(input, budgets.l1);
    let l2 = render_l2(input.task_brief.as_ref(), budgets.l2);
    let l3 = render_l3(input, budgets.l3);

    let tokens_l0 = estimate_tokens(&l0);
    let tokens_l1 = estimate_tokens(&l1);
    let tokens_l2 = estimate_tokens(&l2);
    let tokens_l3 = estimate_tokens(&l3);

    RenderedLayers {
        l0, l1, l2, l3,
        total_tokens: tokens_l0 + tokens_l1 + tokens_l2 + tokens_l3,
        tokens_l0, tokens_l1, tokens_l2, tokens_l3,
    }
}

fn render_l0(persona: &str, budget: usize) -> String {
    if persona.is_empty() {
        return String::new();
    }
    smart_truncate(persona, budget)
}

fn render_l1(input: &LayerInput, budget: usize) -> String {
    let mut s = String::new();
    s.push_str("【L1 技能】\n");

    // 1) 目录清单（名字 + 一句话）
    if !input.skill_catalog.is_empty() {
        s.push_str("可用技能（按名调用 SKILL.md）：\n");
        for b in &input.skill_catalog {
            s.push_str(&format!("- {}：{}\n", b.name, b.description));
        }
    }

    // 2) 本步全文注入
    if !input.skills_full.is_empty() {
        s.push_str("本步加载的技能全文：\n");
        for sf in &input.skills_full {
            s.push_str(&format!("### {}\n{}\n", sf.name, sf.body));
        }
    }

    // 3) 已注入过的技能：只留名字 + 3 要点（6.4）
    if !input.prior_skills.is_empty() {
        s.push_str("已加载过的技能要点（无需重复读全文）：\n");
        for p in &input.prior_skills {
            s.push_str(&format!("- {}：\n", p.name));
            for (i, b) in p.bullets.iter().take(3).enumerate() {
                s.push_str(&format!("  {}. {}\n", i + 1, b));
            }
        }
    }

    // 4) 工具收窄
    if let Some(hint) = &input.tool_hint {
        s.push_str(&format!("本步工具范围：{}\n", hint));
    }

    if s == "【L1 技能】\n" {
        return String::new();
    }
    smart_truncate(&s, budget)
}

fn render_l2(brief: Option<&TaskBrief>, budget: usize) -> String {
    let Some(b) = brief else { return String::new() };
    if b.goal.is_empty() && b.constraints.is_empty() && b.acceptance.is_empty() && b.artifacts_required.is_empty() {
        return String::new();
    }
    let mut s = String::from("【L2 任务简报】\n");
    if !b.goal.is_empty() {
        s.push_str(&format!("目标：{}\n", b.goal));
    }
    if b.total_steps > 0 {
        let step = b.current_step.min(b.total_steps);
        s.push_str(&format!("进度：第 {}/{} 步", step, b.total_steps));
        if let Some(t) = &b.step_title {
            s.push_str(&format!("（{}）", t));
        }
        s.push('\n');
    }
    if !b.constraints.is_empty() {
        s.push_str("约束：\n");
        for c in &b.constraints { s.push_str(&format!("- {}\n", c)); }
    }
    if !b.acceptance.is_empty() {
        s.push_str("验收：\n");
        for a in &b.acceptance { s.push_str(&format!("- {}\n", a)); }
    }
    if !b.artifacts_required.is_empty() {
        s.push_str("产物：\n");
        for a in &b.artifacts_required { s.push_str(&format!("- {}\n", a)); }
    }
    smart_truncate(&s, budget)
}

fn render_l3(input: &LayerInput, budget: usize) -> String {
    let mut s = String::from("【L3 上下文】\n");

    // 1) 结构化摘要（如果有）
    if let Some(summary) = &input.summary_block {
        s.push_str("## 早期内容摘要\n");
        s.push_str(summary);
        if !summary.ends_with('\n') { s.push('\n'); }
    }

    // 2) 事实卡
    let active: Vec<&FactCard> = input.facts.iter().filter(|f| !f.archived).collect();
    if !active.is_empty() {
        s.push_str("## 事实卡（数字/路径/结论，不随压缩丢失）\n");
        for f in &active {
            s.push_str(&format!("- [{}] {}（来源 {}）\n", f.kind.as_str(), f.value, f.source_msg_id));
        }
    }

    // 3) 最近原文
    if !input.messages.is_empty() {
        s.push_str("## 最近对话\n");
        for m in &input.messages {
            if m.role == "system" { continue; }
            s.push_str(&format!("[{} {}] {}\n", m.role, m.id, m.content));
        }
    }

    // 4) KB 命中片段（带出处）
    if !input.kb_hits.is_empty() {
        s.push_str("## 知识片段\n");
        for h in &input.kb_hits {
            s.push_str(&format!("[来源: {} / {}] {}\n", h.source, h.chunk_id, h.snippet));
        }
    }

    if s == "【L3 上下文】\n" {
        return String::new();
    }
    smart_truncate(&s, budget)
}

/// 智能截断：按行累加 token；超出预算时，**含数字/路径特征的行整体保留**（不切关键 token）；
/// 若整段所有行都含关键 token 导致超支，则按字符兜底截断，但保证关键 token 完整不被拦腰切断。
pub fn smart_truncate(text: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    if estimate_tokens(text) <= max_tokens {
        return text.to_string();
    }

    // 1) 按行累加
    let mut kept = String::new();
    let mut used = 0usize;
    for line in text.split_inclusive('\n') {
        let line_tokens = estimate_tokens(line);
        let line_has_key = line_has_anchor(line);
        if used + line_tokens <= max_tokens {
            kept.push_str(line);
            used += line_tokens;
            continue;
        }
        // 这行加进来会超预算
        if line_has_key && used < max_tokens * 9 / 10 {
            // 关键行：整体保留（允许小幅超支），但只收一行
            kept.push_str(line);
            // 后面还有关键行也不再收（避免无限膨胀）
            break;
        }
        // 非关键行超了就到此为止
        break;
    }

    // 2) 兜底：若 kept 仍然超预算（关键行占比过大），按字符截，但保留锚点 token 完整。
    if estimate_tokens(&kept) > max_tokens {
        kept = truncate_preserving_anchors(&kept, max_tokens);
    }
    if kept.is_empty() && !text.is_empty() {
        // 极端情况：第一行就超预算——至少保留整行里能放下的关键 token。
        let first_line = text.lines().next().unwrap_or("");
        kept = truncate_preserving_anchors(first_line, max_tokens);
    }
    // 3) v45.9：截断不许丢关键信息——把原文里没落进 kept 的路径/数字回填成一行脚注。
    //    排序：路径（含 '/'）优先 → 长数字（≥3 位）→ 其他短数字。避免被无关短数字淹没。
    if !text.is_empty() && kept.len() < text.len() {
        let mut missing: Vec<String> = collect_anchor_tokens(text)
            .into_iter()
            .filter(|a| !kept.contains(a.as_str()))
            .collect();
        missing.sort_by_key(|tok| {
            let is_path = tok.contains('/') || tok.contains('\\') || tok.starts_with('~');
            let is_long_num = tok.chars().all(|c| c.is_ascii_digit()) && tok.len() >= 3;
            // 排序键：路径(0) < 长数字(1) < 其他(2)，同组内按长度降序（用反转长度）
            let group = if is_path { 0u8 } else if is_long_num { 1 } else { 2 };
            (group, usize::MAX - tok.len())
        });
        missing.truncate(6);
        if !missing.is_empty() {
            kept.push_str("\n（关键信息：");
            kept.push_str(&missing.join("、"));
            kept.push_str("）");
        }
    }
    kept
}

/// 从全文里抽出"锚点 token"（路径与数字），供截断后回填使用。
fn collect_anchor_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() || c == '/' || c == '\\' || c == '~' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '/'
                    || chars[i] == '\\'
                    || chars[i] == '~'
                    || chars[i] == '.'
                    || chars[i] == '-'
                    || chars[i] == '_')
            {
                i += 1;
            }
            if i - start >= 2 {
                let tok: String = chars[start..i].iter().collect();
                if !out.iter().any(|x| x == &tok) {
                    out.push(tok);
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

/// 判断一行是否含"锚点 token"（数字串、路径、标识符）。
fn line_has_anchor(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c.is_ascii_digit() {
            // 连续数字至少 2 位才算锚点（避免单个 "a1" 这种）
            let start = i;
            while i < n && chars[i].is_ascii_digit() { i += 1; }
            if i - start >= 2 { return true; }
            continue;
        }
        if c == '/' || c == '\\' || c == '~' || c == '.' {
            // 路径特征
            if i + 1 < n && (chars[i + 1].is_ascii_alphanumeric() || chars[i + 1] == '/' || chars[i + 1] == '\\') {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 按字符截，保证锚点 token 不被切一半。
fn truncate_preserving_anchors(text: &str, max_tokens: usize) -> String {
    // v45.9 修复：原实现对每个片段都 estimate_tokens(整个前缀)，长文本下是 O(n²)，
    // 单测里 100k 字符的消息直接把测试挂住（跑过 60s 不结束）。
    // 改成"只对新增片段计数"，整体 O(n)。
    let chars: Vec<char> = text.chars().collect();
    let mut end = 0usize;
    let mut used = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let anchor_end = if c.is_ascii_digit() || c == '/' || c == '\\' || c == '~' || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '/'
                    || chars[i] == '\\'
                    || chars[i] == '~'
                    || chars[i] == '.'
                    || chars[i] == '-')
            {
                i += 1;
            }
            if i - start >= 2 { i } else { start + 1 }
        } else {
            i + 1
        };
        let seg: String = chars[end..anchor_end].iter().collect();
        let seg_tokens = estimate_tokens(&seg);
        if used + seg_tokens > max_tokens && end > 0 {
            break;
        }
        used += seg_tokens;
        end = anchor_end;
        if anchor_end > i { i = anchor_end; }
    }
    chars[..end].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::budget::split_budget;

    fn msg(id: &str, role: &str, content: &str) -> Message {
        Message { id: id.into(), role: role.into(), content: content.into(), ts: 1 }
    }

    #[test]
    fn assemble_empty_input_keeps_l0() {
        let input = LayerInput {
            persona: "你是小小".into(),
            ..Default::default()
        };
        let b = split_budget(8000);
        let r = assemble(&input, &b);
        assert!(r.l0.contains("你是小小"));
        assert!(r.l1.is_empty());
        assert!(r.l2.is_empty());
        // L3 空时应该是空串
        assert!(r.l3.is_empty());
        assert!(r.total_tokens <= 8000 - 400);
    }

    #[test]
    fn four_layers_rendered_each_respects_budget() {
        let input = LayerInput {
            persona: "你是小小，温柔陪伴。".repeat(50), // 长人设
            skill_catalog: vec![
                SkillBrief { name: "web".into(), description: "搜网页".into() },
                SkillBrief { name: "code".into(), description: "看代码".into() },
            ],
            skills_full: vec![SkillFull { name: "web".into(), body: "web 技能全文".into() }],
            task_brief: Some(TaskBrief {
                goal: "做一个 demo".into(),
                constraints: vec!["预算 8000".into()],
                acceptance: vec!["单测通过".into()],
                artifacts_required: vec!["/tmp/out.zip".into()],
                current_step: 1,
                total_steps: 3,
                step_title: Some("写代码".into()),
            }),
            messages: vec![msg("u1", "user", "帮我看看 /home/x/src/main.rs 第 42 行")],
            ..Default::default()
        };
        let b = split_budget(8000);
        let r = assemble(&input, &b);
        // 各层都渲染出来了
        assert!(r.l0.contains("你是小小"));
        assert!(r.l1.contains("web") && r.l1.contains("code"));
        assert!(r.l2.contains("做一个 demo") && r.l2.contains("第 1/3 步"));
        assert!(r.l3.contains("u1") && r.l3.contains("/home/x/src/main.rs"));
        // 各层不超各自预算
        assert!(r.tokens_l0 <= b.l0);
        assert!(r.tokens_l1 <= b.l1);
        assert!(r.tokens_l2 <= b.l2);
        assert!(r.tokens_l3 <= b.l3);
        // 总量在预算内（扣除 reserve）
        assert!(r.total_tokens <= b.usable());
    }

    #[test]
    fn smart_truncate_keeps_anchor_tokens() {
        // 构造：前面一堆非锚点文字，最后一行带关键路径/数字。
        let filler = "这是无关紧要的寒暄。".repeat(50); // 约 200 tokens
        let key_line = format!("关键路径 /home/user/proj/out.zip 数字 8000 必须保留。\n");
        let text = format!("{}{}", filler, key_line);
        let truncated = smart_truncate(&text, 250);
        // 关键路径与数字必须在
        assert!(truncated.contains("/home/user/proj/out.zip"), "截断后路径丢失：{}", truncated);
        assert!(truncated.contains("8000"), "截断后数字丢失");
    }

    #[test]
    fn smart_truncate_empty_and_short() {
        assert_eq!(smart_truncate("", 100), "");
        assert_eq!(smart_truncate("短文本", 0), "");
        assert_eq!(smart_truncate("短文本", 100), "短文本");
    }

    #[test]
    fn single_long_message_truncated_but_anchors_survive() {
        // 单条超长消息（预算边界用例）
        let mut content = String::from("开头寒暄 ");
        for i in 0..200 {
            content.push_str(&format!("无关句{} ", i));
        }
        content.push_str("最终路径 /etc/qibai/config.json 预算 8000 tokens。");
        let truncated = smart_truncate(&content, 200);
        assert!(truncated.contains("/etc/qibai/config.json"), "单条超长消息截断丢路径");
        assert!(truncated.contains("8000"), "单条超长消息截断丢数字");
    }
}
