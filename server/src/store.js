// store.js — SQLite 数据层（规格 §2）。node:sqlite（Node ≥22.5 内置实验模块）。
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { DatabaseSync } = require('node:sqlite');

// ---------- 规范化工具（规格 §3 注册规则 / §5 标签比较） ----------

function sha256(s) {
  return crypto.createHash('sha256').update(String(s), 'utf8').digest('hex');
}

function normalizeCity(city) {
  return String(city ?? '').trim();
}

function cleanNickname(nickname) {
  // 去控制字符、trim、截 12 字符；空则默认「星客」
  let s = String(nickname ?? '')
    .replace(/[\x00-\x1F\x7F]/g, '')
    .trim()
    .slice(0, 12);
  if (!s) s = '星客';
  return s;
}

function clampInt(v, lo, hi) {
  const n = Math.floor(Number(v));
  if (!Number.isFinite(n)) return lo;
  return Math.max(lo, Math.min(hi, n));
}

const TAG_GROUPS = ['personality', 'interests', 'status'];

function normalizeTags(tags) {
  // 三组各最多 5 个标签、单标签 ≤8 字符；trim / 去重 / 忽略大小写比较用由 match 负责
  const out = { personality: [], interests: [], status: [] };
  const src = (tags && typeof tags === 'object') ? tags : {};
  for (const g of TAG_GROUPS) {
    const arr = Array.isArray(src[g]) ? src[g] : [];
    const seen = new Set();
    for (const raw of arr) {
      const t = String(raw ?? '').trim().slice(0, 8);
      if (!t) continue;
      const key = t.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      out[g].push(t);
      if (out[g].length >= 5) break;
    }
  }
  return out;
}

function randomToken() {
  return crypto.randomBytes(16).toString('hex'); // 32 hex
}

// ---------- Store ----------

const SCHEMA = `
CREATE TABLE IF NOT EXISTS users (
  anon_id    TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL,
  nickname   TEXT NOT NULL,
  color_idx  INTEGER NOT NULL DEFAULT 0,
  soul_tags  TEXT NOT NULL DEFAULT '{}',
  city       TEXT NOT NULL DEFAULT '',
  age_declared TEXT NOT NULL DEFAULT '',
  pure_mode  INTEGER NOT NULL DEFAULT 0,
  avatar     INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  status     TEXT NOT NULL DEFAULT 'offline'
);
CREATE TABLE IF NOT EXISTS messages (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  sender    TEXT NOT NULL,
  receiver  TEXT NOT NULL,
  body      TEXT NOT NULL,
  ts        INTEGER NOT NULL,
  delivered INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS likes (
  liker  TEXT NOT NULL,
  liked  TEXT NOT NULL,
  ts     INTEGER NOT NULL,
  PRIMARY KEY (liker, liked)
);
CREATE TABLE IF NOT EXISTS reports (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  reporter TEXT NOT NULL,
  target   TEXT NOT NULL,
  reason   TEXT NOT NULL,
  ts       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS blocks (
  blocker TEXT NOT NULL,
  blocked TEXT NOT NULL,
  ts      INTEGER NOT NULL,
  PRIMARY KEY (blocker, blocked)
);
`;

class Store {
  constructor(dataDir) {
    fs.mkdirSync(dataDir, { recursive: true });
    this.dataDir = dataDir;
    this.db = new DatabaseSync(path.join(dataDir, 'qibai.db'));
    this.db.exec('PRAGMA journal_mode = WAL;');
    this.db.exec(SCHEMA);
  }

  close() {
    this.db.close();
  }

  // ---------- 用户 ----------

  createUser({ nickname, color_idx, soul_tags, city, age_declared, pure_mode, avatar }) {
    const anonId = crypto.randomUUID();
    const token = randomToken();
    const now = Math.floor(Date.now() / 1000);
    this.db.prepare(`
      INSERT INTO users (anon_id, token_hash, nickname, color_idx, soul_tags, city, age_declared, pure_mode, avatar, created_at, last_seen, status)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'offline')
    `).run(
      anonId,
      sha256(token),
      cleanNickname(nickname),
      clampInt(color_idx, 0, 11),
      JSON.stringify(normalizeTags(soul_tags)),
      normalizeCity(city),
      age_declared === 'minor' ? 'minor' : '18plus',
      pure_mode ? 1 : 0,
      clampInt(avatar, -1, 4),
      now, now,
    );
    return { anonId, token, row: this.getById(anonId) };
  }

  tokenToUser(token) {
    if (!token) return null;
    const row = this.db.prepare('SELECT * FROM users WHERE token_hash = ?').get(sha256(String(token)));
    return row || null;
  }

  getById(anonId) {
    if (!anonId) return null;
    return this.db.prepare('SELECT * FROM users WHERE anon_id = ?').get(anonId) || null;
  }

  updateProfile(anonId, fields) {
    const row = this.getById(anonId);
    if (!row) return null;
    const next = {
      nickname: fields.nickname !== undefined ? cleanNickname(fields.nickname) : row.nickname,
      color_idx: fields.color_idx !== undefined ? clampInt(fields.color_idx, 0, 11) : row.color_idx,
      soul_tags: fields.soul_tags !== undefined ? JSON.stringify(normalizeTags(fields.soul_tags)) : row.soul_tags,
      city: fields.city !== undefined ? normalizeCity(fields.city) : row.city,
      avatar: fields.avatar !== undefined ? clampInt(fields.avatar, -1, 4) : row.avatar,
    };
    this.db.prepare(`
      UPDATE users SET nickname=?, color_idx=?, soul_tags=?, city=?, avatar=? WHERE anon_id=?
    `).run(next.nickname, next.color_idx, next.soul_tags, next.city, next.avatar, anonId);
    return this.getById(anonId);
  }

  setStatus(anonId, status, tsSec) {
    const ts = tsSec ?? Math.floor(Date.now() / 1000);
    this.db.prepare(`UPDATE users SET status=?, last_seen=? WHERE anon_id=?`)
      .run(status === 'online' ? 'online' : 'offline', ts, anonId);
  }

  rowToProfile(row, viewerId) {
    if (!row) return null;
    let soulTags = { personality: [], interests: [], status: [] };
    try { soulTags = { ...soulTags, ...JSON.parse(row.soul_tags || '{}') }; } catch { /* 占位 */ }
    let mutualLiked = false;
    if (viewerId) {
      mutualLiked = !!this.db.prepare('SELECT 1 FROM likes WHERE liker=? AND liked=? LIMIT 1')
        .get(row.anon_id, viewerId);
    }
    return {
      anon_id: row.anon_id,
      nickname: row.nickname,
      color_idx: row.color_idx,
      soul_tags: soulTags,
      city: row.city,
      avatar: row.avatar,
      pure_mode: !!row.pure_mode,
      online: row.status === 'online',
      last_seen: row.last_seen,
      mutual_liked: mutualLiked,
    };
  }

  // ---------- 消息 ----------

  insertMessage(sender, receiver, body, delivered) {
    const ts = Math.floor(Date.now() / 1000);
    const info = this.db.prepare(`
      INSERT INTO messages (sender, receiver, body, ts, delivered) VALUES (?, ?, ?, ?, ?)
    `).run(sender, receiver, String(body), ts, delivered ? 1 : 0);
    return { mid: Number(info.lastInsertRowid), ts };
  }

  popInbox(receiver) {
    const rows = this.db.prepare(`
      SELECT * FROM messages WHERE receiver=? AND delivered=0 ORDER BY ts ASC, id ASC
    `).all(receiver);
    if (rows.length) {
      this.db.prepare(`UPDATE messages SET delivered=1 WHERE receiver=? AND delivered=0`).run(receiver);
    }
    return rows.map((r) => ({ mid: r.id, from: r.sender, body: r.body, ts: r.ts }));
  }

  // ---------- 点亮 / 举报 / 拉黑 ----------

  like(liker, liked) {
    const ts = Math.floor(Date.now() / 1000);
    this.db.prepare(`INSERT OR IGNORE INTO likes (liker, liked, ts) VALUES (?, ?, ?)`)
      .run(liker, liked, ts);
    // matched = 对方已点亮过你
    const matched = !!this.db.prepare('SELECT 1 FROM likes WHERE liker=? AND liked=? LIMIT 1')
      .get(liked, liker);
    return { matched };
  }

  addReport(reporter, target, reason) {
    const ts = Math.floor(Date.now() / 1000);
    this.db.prepare(`INSERT INTO reports (reporter, target, reason, ts) VALUES (?, ?, ?, ?)`)
      .run(reporter, target, String(reason ?? '').slice(0, 200), ts);
  }

  addBlock(blocker, blocked) {
    const ts = Math.floor(Date.now() / 1000);
    this.db.prepare(`INSERT OR IGNORE INTO blocks (blocker, blocked, ts) VALUES (?, ?, ?)`)
      .run(blocker, blocked, ts);
  }

  blockedEither(a, b) {
    return !!this.db.prepare(`
      SELECT 1 FROM blocks WHERE (blocker=? AND blocked=?) OR (blocker=? AND blocked=?) LIMIT 1
    `).get(a, b, b, a);
  }

  /**
   * 候选池：排除自己 / 双向拉黑任一方向 / 被举报过（任意人举报）。
   */
  candidatePool(viewerId) {
    return this.db.prepare(`
      SELECT * FROM users u
      WHERE u.anon_id != ?
        AND NOT EXISTS (SELECT 1 FROM blocks b WHERE (b.blocker=? AND b.blocked=u.anon_id) OR (b.blocker=u.anon_id AND b.blocked=?))
        AND NOT EXISTS (SELECT 1 FROM reports r WHERE r.target = u.anon_id)
    `).all(viewerId, viewerId, viewerId);
  }

  /**
   * 漫游列表：排除自己 / 双向拉黑任一方向；支持 tag/city/online/q 过滤。
   */
  searchSouls(viewerId, { tag, city, online, q } = {}) {
    let sql = `SELECT * FROM users u WHERE u.anon_id != ?
      AND NOT EXISTS (SELECT 1 FROM blocks b WHERE (b.blocker=? AND b.blocked=u.anon_id) OR (b.blocker=u.anon_id AND b.blocked=?))`;
    const params = [viewerId, viewerId, viewerId];
    if (city) {
      sql += ` AND u.city = ?`; params.push(normalizeCity(city));
    }
    if (online === true) { sql += ` AND u.status = 'online'`; }
    if (online === false) { sql += ` AND u.status = 'offline'`; }
    if (q) {
      sql += ` AND u.nickname LIKE ?`; params.push(`%${String(q).replace(/[%_]/g, '')}%`);
    }
    sql += ` ORDER BY u.last_seen DESC`;
    let rows = this.db.prepare(sql).all(...params);
    if (tag) {
      const t = String(tag).trim().toLowerCase();
      rows = rows.filter((r) => {
        let tags = {};
        try { tags = JSON.parse(r.soul_tags || '{}'); } catch { /* 占位 */ }
        return TAG_GROUPS.some((g) =>
          Array.isArray(tags[g]) && tags[g].some((x) => String(x).toLowerCase().includes(t)));
      });
    }
    return rows;
  }

  citySouls(viewerId, city) {
    const c = normalizeCity(city);
    return this.db.prepare(`
      SELECT * FROM users u WHERE u.anon_id != ? AND u.city = ?
      AND NOT EXISTS (SELECT 1 FROM blocks b WHERE (b.blocker=? AND b.blocked=u.anon_id) OR (b.blocker=u.anon_id AND b.blocked=?))
      ORDER BY u.last_seen DESC
    `).all(viewerId, c, viewerId, viewerId);
  }
}

module.exports = {
  Store,
  sha256,
  normalizeCity,
  cleanNickname,
  normalizeTags,
  clampInt,
  TAG_GROUPS,
};
