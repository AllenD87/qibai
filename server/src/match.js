// match.js — 匹配 + 同城算法（规格 §5）
// score = personality交集×3 + interests交集×2 + status交集×1 + 同城+5 + 在线+2
// 排序：score 降序；同分 last_seen 降序。limit 默认 20、上限 50。
'use strict';

const { TAG_GROUPS } = require('./store');

/** 标签集合：trim + 忽略大小写后去重 */
function tagSet(arr) {
  const s = new Set();
  for (const raw of arr || []) {
    const t = String(raw ?? '').trim().toLowerCase();
    if (t) s.add(t);
  }
  return s;
}

function parseTags(row) {
  let tags = {};
  try { tags = JSON.parse(row.soul_tags || '{}'); } catch { tags = {}; }
  return {
    personality: tagSet(tags.personality),
    interests: tagSet(tags.interests),
    status: tagSet(tags.status),
  };
}

const WEIGHTS = { personality: 3, interests: 2, status: 1 };

/**
 * 计算单个候选的分数（纯函数，便于单测）。
 * @param {object} viewer 用户行（含 soul_tags/city/status/last_seen）
 * @param {object} candidate 候选用户行
 * @returns {number} score
 */
function scoreFor(viewer, candidate) {
  const v = parseTags(viewer);
  const c = parseTags(candidate);
  let score = 0;
  for (const g of TAG_GROUPS) {
    let inter = 0;
    for (const t of v[g]) if (c[g].has(t)) inter += 1;
    score += inter * WEIGHTS[g];
  }
  // 同城：双方均填了城市且规范化相等才 +5（空城市不误匹配）
  if (viewer.city && candidate.city && viewer.city.trim() === candidate.city.trim()) score += 5;
  if (candidate.status === 'online') score += 2;
  return score;
}

const DEFAULT_LIMIT = 20;
const MAX_LIMIT = 50;

function normalizeLimit(limit) {
  const n = Math.floor(Number(limit));
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_LIMIT;
  return Math.min(n, MAX_LIMIT);
}

/**
 * 匹配排序（纯函数）。
 * @param {object} viewer 当前用户行
 * @param {object[]} candidates 已排除自己/拉黑/举报的候选行
 * @param {number} limit
 * @returns {object[]} [{ row, score }] 已排序、已截断
 */
function rankCandidates(viewer, candidates, limit) {
  const lim = normalizeLimit(limit);
  return candidates
    .map((row) => ({ row, score: scoreFor(viewer, row) }))
    .sort((a, b) => {
      if (b.score !== a.score) return b.score - a.score;
      return (b.row.last_seen || 0) - (a.row.last_seen || 0);
    })
    .slice(0, lim);
}

module.exports = { scoreFor, rankCandidates, normalizeLimit, tagSet, DEFAULT_LIMIT, MAX_LIMIT };
