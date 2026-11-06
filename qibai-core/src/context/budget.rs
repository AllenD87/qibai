//! 上下文预算与 token 估算。
//!
//! 规格：`docs/v45.9_长任务与上下文管理方案.md` 第 5.3、6.1 节。
//! - 默认总预算 8000 tokens；
//! - 切分比例 L0/L1/L2/L3/余量 = 10/15/10/60/5；
//! - 触发压缩：估算 token > 预算 × 70% 或轮次 > 30（均可配）。
//!
//! Token 估算为可解释近似：
//! - 中文（CJK 统一表意、全角标点）按 1 字 ≈ 1 token；
//! - ASCII 字符按 4 字符 ≈ 1 token（含英文、数字、半角标点、空格）；
//! - 其他（emoji、假名、谚文等 BMP 内非 CJK 非 ASCII）按 2 字符 ≈ 1 token 兜底。

/// 上下文运行时配置（由 AppSettings 透传，避免直接依赖 settings）。
#[derive(Debug, Clone, Copy)]
pub struct ContextConfig {
    /// 总预算 tokens（默认 8000）。
    pub budget_tokens: usize,
    /// 原文层保留最近多少轮（默认 10）。
    pub recent_turns: usize,
    /// 触发压缩的预算占比（默认 0.7）。
    pub trigger_ratio: f64,
    /// 轮次超过该值强制压缩（默认 30）。
    pub max_turns_before_compress: usize,
    /// 事实卡上限，超了按最近优先淘汰进 archive（默认 60）。
    pub fact_card_cap: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            budget_tokens: 8000,
            recent_turns: 10,
            trigger_ratio: 0.7,
            max_turns_before_compress: 30,
            fact_card_cap: 60,
        }
    }
}

impl ContextConfig {
    /// 边界夹取，避免外部传进来的 0 / 负数炸掉。
    pub fn clamped(&self) -> Self {
        Self {
            budget_tokens: self.budget_tokens.clamp(512, 128_000),
            recent_turns: self.recent_turns.clamp(1, 60),
            trigger_ratio: self.trigger_ratio.clamp(0.3, 0.95),
            max_turns_before_compress: self.max_turns_before_compress.clamp(5, 500),
            fact_card_cap: self.fact_card_cap.clamp(1, 2000),
        }
    }
}

/// 各层预算切分结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerBudgets {
    pub l0: usize,
    pub l1: usize,
    pub l2: usize,
    pub l3: usize,
    /// 工具结果预留，不参与拼装。
    pub reserve: usize,
}

impl LayerBudgets {
    /// 除 reserve 外可用于拼装的总量。
    pub fn usable(self) -> usize {
        self.l0 + self.l1 + self.l2 + self.l3
    }
}

/// 按 10/15/10/60/5 切总预算，向下取整；余数补给 L3（主上下文层）。
pub fn split_budget(total: usize) -> LayerBudgets {
    let t = total.max(1);
    let l0 = t * 10 / 100;
    let l1 = t * 15 / 100;
    let l2 = t * 10 / 100;
    let reserve = t * 5 / 100;
    let l3 = t - l0 - l1 - l2 - reserve;
    LayerBudgets { l0, l1, l2, l3, reserve }
}

/// 中英混合近似 token 估算（可解释、确定性）。
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut ascii = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else if ch.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    // 中文 1:1；ASCII 4:1；其他 2:1；四舍五入兜底至少 1（空串为 0）。
    let ascii_tokens = (ascii + 3) / 4;
    let other_tokens = (other + 1) / 2;
    let total = cjk + ascii_tokens + other_tokens;
    if total == 0 && !text.is_empty() {
        1
    } else {
        total
    }
}

fn is_cjk(ch: char) -> bool {
    // CJK 统一表意 + 扩展 A + 兼容表意 + 全角标点/片假平假也按中文计。
    let cp = ch as u32;
    matches!(cp,
        0x4E00..=0x9FFF |      // CJK Unified
        0x3400..=0x4DBF |      // Ext A
        0xF900..=0xFAFF |      // Compatibility
        0x3000..=0x303F |      // CJK 标点
        0x3040..=0x30FF |      // 假名
        0xFF00..=0xFFEF         // 全角 ASCII / 标点
    )
}

/// 是否应当触发压缩（6.1）。任一满足即触发。
pub fn should_compress(tokens: usize, turns: usize, cfg: &ContextConfig) -> bool {
    let c = cfg.clamped();
    let threshold = (c.budget_tokens as f64 * c.trigger_ratio).floor() as usize;
    tokens > threshold || turns > c.max_turns_before_compress
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_budget_respects_ratios() {
        let b = split_budget(8000);
        assert_eq!(b.l0, 800);
        assert_eq!(b.l1, 1200);
        assert_eq!(b.l2, 800);
        assert_eq!(b.reserve, 400);
        // L3 拿余下的，且总和等于 8000
        assert_eq!(b.l3, 4800);
        assert_eq!(b.usable() + b.reserve, 8000);
    }

    #[test]
    fn split_budget_small_total_no_panic() {
        let b = split_budget(10);
        // 极小预算也不 panic，reserve 可被取整成 0
        assert!(b.usable() <= 10);
    }

    #[test]
    fn estimate_tokens_mixed() {
        // 规则：CJK 1 字 ≈ 1 token；ASCII 4 字符 ≈ 1 token（向上取整）；其他 2 字符 ≈ 1 token。
        // 纯中文 10 字 ≈ 10
        assert_eq!(estimate_tokens("你好世界这是一段中文"), 10);
        // 纯 ASCII 16 字符 ≈ 4
        assert_eq!(estimate_tokens("abcdefghijklmnop"), 4);
        // 混合："中文" 2 CJK + 16 ASCII = 2 + ceil(16/4) = 6
        let mixed = "中文abcdefghijklmnop";
        let t = estimate_tokens(mixed);
        assert_eq!(t, 6, "2 CJK + 16 ASCII → 2 + 4 = 6");
        // 单调性：更长的同结构文本 token 数不应更少
        let short_t = estimate_tokens("中文ab");   // 2 CJK + 2 ASCII = 2 + 1 = 3
        let long_t = estimate_tokens("中文abcdefghijklmnopqrstuvwxyz"); // 2 CJK + 26 ASCII = 2 + 7 = 9
        assert!(long_t > short_t, "单调性失败：{} <= {}", long_t, short_t);
        // 区间断言：CJK 占比越高，token 越接近字符数
        let cn = estimate_tokens("你好世界");
        assert!((4..=5).contains(&cn), "4 个 CJK 应在 4~5 token 之间，实际 {}", cn);
    }

    #[test]
    fn estimate_tokens_empty_and_short() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn should_compress_by_ratio_and_turns() {
        let cfg = ContextConfig::default();
        // 6000 > 8000*0.7=5600 → 触发
        assert!(should_compress(6000, 5, &cfg));
        // 4000 < 5600 且轮次 10 < 30 → 不触发
        assert!(!should_compress(4000, 10, &cfg));
        // 轮次 35 > 30 → 触发
        assert!(should_compress(1000, 35, &cfg));
    }

    #[test]
    fn config_clamped_sanitizes_garbage() {
        let cfg = ContextConfig {
            budget_tokens: 0,
            recent_turns: 0,
            trigger_ratio: 9.9,
            max_turns_before_compress: 0,
            fact_card_cap: 0,
        };
        let c = cfg.clamped();
        assert_eq!(c.budget_tokens, 512);
        assert_eq!(c.recent_turns, 1);
        assert_eq!(c.trigger_ratio, 0.95);
        assert_eq!(c.fact_card_cap, 1);
    }
}
