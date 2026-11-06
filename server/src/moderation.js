// moderation.js — 内容审核占位（规格 §6）
// 内置极小敏感词种子表；生产环境可在此集中接入第三方审核（见 server/README.md）。
'use strict';

const MAX_BODY_LEN = 500;

// 种子词表（示例性质，联调用；类别用于 reason 字段）
const SEED_WORDS = [
  { category: 'porn', words: ['色情', '裸体', 'av'] },
  { category: 'violence', words: ['暴力', '杀人', '枪支'] },
  { category: 'gambling', words: ['赌博', '赌场'] },
  { category: 'drugs', words: ['毒品', '贩毒'] },
  { category: 'fraud', words: ['诈骗', '刷单'] },
];

/**
 * 审核一段文本。
 * @param {string} text
 * @returns {{flagged: boolean, reason: string}}
 *   reason ∈ '' | 'too_long' | '<category>'
 */
function moderate(text) {
  const s = String(text ?? '').trim();
  if (s.length > MAX_BODY_LEN) {
    return { flagged: true, reason: 'too_long' };
  }
  // 空文本不算命中（由调用方决定是否拒收空 body）
  if (s.length === 0) {
    return { flagged: false, reason: '' };
  }
  const lower = s.toLowerCase();
  for (const { category, words } of SEED_WORDS) {
    for (const w of words) {
      if (lower.includes(w.toLowerCase())) {
        return { flagged: true, reason: category };
      }
    }
  }
  return { flagged: false, reason: '' };
}

module.exports = { moderate, MAX_BODY_LEN, SEED_WORDS };
