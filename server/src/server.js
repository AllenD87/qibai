// server.js — 入口 + HTTP 路由（规格 §3）。WS 升级转给 WSHub（§4）。
'use strict';

const http = require('node:http');
const path = require('node:path');
const { Store, normalizeCity } = require('./store');
const { rankCandidates } = require('./match');
const { moderate } = require('./moderation');
const { WSHub } = require('./ws');

const SERVER_NAME = 'qibai-net';
const VERSION = '0.1.0';

function httpLog(req, res, started) {
  const ms = Date.now() - started;
  console.log(`[HTTP ${new Date().toISOString()}] ${req.method} ${req.url} -> ${res.statusCode} ${ms}ms`);
}

// 问题7隐私路由：CORS 默认 '*' 兼容本地 file:// 页面 / Tauri webview 联调；
// 生产部署到公网时应设环境变量 QIBAI_CORS_ORIGIN=https://your.domain 收窄到具体 origin，
// 避免任意第三方站点跨域读社交接口。
const CORS_ORIGIN = process.env.QIBAI_CORS_ORIGIN || '*';
const CORS_HEADERS = {
  'Access-Control-Allow-Origin': CORS_ORIGIN,
  'Access-Control-Allow-Methods': 'GET,POST,PUT,DELETE,OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
};

function sendJson(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', ...CORS_HEADERS });
  res.end(body);
}

/** 响应一个空状态码（带 CORS 头），用于 OPTIONS 预检等 */
function sendStatus(res, status) {
  res.writeHead(status, CORS_HEADERS);
  res.end();
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = '';
    req.on('data', (c) => {
      data += c;
      if (data.length > 1e6) { reject(new Error('body too large')); req.destroy(); }
    });
    req.on('end', () => {
      if (!data) return resolve({});
      try { resolve(JSON.parse(data)); } catch { reject(new Error('bad json')); }
    });
    req.on('error', reject);
  });
}

// 占位数据（§3：topics 至少 3 条，events 至少 2 条）
const TOPICS = [
  { id: 't1', title: '深夜电台：你最近在听什么歌', sticky: true, ts: 1729000000 },
  { id: 't2', title: '同城散步地图：推荐你常去的角落', sticky: false, ts: 1729100000 },
  { id: 't3', title: '一个人住的第 N 年', sticky: false, ts: 1729200000 },
];
const EVENTS = [
  { id: 'e1', title: '秋日城市漫步（同城线下 · 占位）', when: '2026-11-08 15:00', where: '城市公园', ts: 1730000000 },
  { id: 'e2', title: '线上音乐分享夜（占位）', when: '2026-11-15 20:00', where: '线上语音房', ts: 1730600000 },
];

/**
 * 创建请求处理器。
 * @param {Store} store
 * @param {WSHub} hub
 */
function createHandler(store, hub) {
  return async (req, res) => {
    const started = Date.now();
    const url = new URL(req.url, 'http://localhost');
    const p = url.pathname;

    // CORS 预检（浏览器 file:// 直连 /api/* 时触发）
    if (req.method === 'OPTIONS') {
      sendStatus(res, 204);
      return httpLog(req, res, started);
    }

    let body = {};
    try {
      body = ['POST', 'PUT'].includes(req.method) ? await readBody(req) : {};
    } catch (e) {
      sendJson(res, 400, { error: 'bad_request', message: e.message });
      return httpLog(req, res, started);
    }

    const needToken = (token) => {
      const user = store.tokenToUser(token);
      if (!user) sendJson(res, 401, { error: 'invalid_token', message: 'token 无效或缺失' });
      return user;
    };

    try {
      // ---- health ----
      if (req.method === 'GET' && p === '/api/health') {
        return sendJson(res, 200, { ok: true, name: SERVER_NAME, version: VERSION, ts: Math.floor(Date.now() / 1000) });
      }

      // ---- register ----
      if (req.method === 'POST' && p === '/api/register') {
        const created = store.createUser(body);
        return sendJson(res, 200, {
          anon_id: created.anonId,
          token: created.token,
          profile: store.rowToProfile(created.row, created.anonId),
        });
      }

      // ---- me ----
      if (req.method === 'GET' && p === '/api/me') {
        const user = needToken(url.searchParams.get('token'));
        if (!user) return httpLog(req, res, started);
        return sendJson(res, 200, { profile: store.rowToProfile(user, user.anon_id) });
      }
      if (req.method === 'PUT' && p === '/api/me') {
        const user = needToken(body.token);
        if (!user) return httpLog(req, res, started);
        const updated = store.updateProfile(user.anon_id, body);
        return sendJson(res, 200, { profile: store.rowToProfile(updated, user.anon_id) });
      }

      // ---- souls 漫游列表 ----
      if (req.method === 'GET' && p === '/api/souls') {
        const viewer = needToken(url.searchParams.get('token'));
        if (!viewer) return httpLog(req, res, started);
        // online 过滤：兼容 "1"/"true"（在线）与 "0"/"false"（离线）；其他值忽略
        let online;
        const ov = url.searchParams.get('online');
        if (ov === '1' || ov === 'true') online = true;
        else if (ov === '0' || ov === 'false') online = false;
        const rows = store.searchSouls(viewer.anon_id, {
          tag: url.searchParams.get('tag'),
          city: url.searchParams.get('city'),
          online,
          q: url.searchParams.get('q'),
        });
        return sendJson(res, 200, { souls: rows.map((r) => store.rowToProfile(r, viewer.anon_id)) });
      }

      // ---- /api/souls/:id/action ----
      let m = p.match(/^\/api\/souls\/([^/]+)\/(like|report|block)$/);
      if (m && req.method === 'POST') {
        const viewer = needToken(body.token);
        if (!viewer) return httpLog(req, res, started);
        const target = store.getById(m[1]);
        if (!target) {
          return sendJson(res, 404, { error: 'not_found', message: '目标用户不存在' });
        }
        const action = m[2];
        if (action === 'like') {
          const { matched } = store.like(viewer.anon_id, target.anon_id);
          return sendJson(res, 200, { liked: true, matched, mutual: matched });
        }
        if (action === 'report') {
          store.addReport(viewer.anon_id, target.anon_id, body.reason);
          return sendJson(res, 200, { ok: true });
        }
        if (action === 'block') {
          store.addBlock(viewer.anon_id, target.anon_id);
          return sendJson(res, 200, { ok: true, blocked: true });
        }
      }

      // ---- /api/souls/:id 详情 ----
      m = p.match(/^\/api\/souls\/([^/]+)$/);
      if (m && req.method === 'GET') {
        const viewer = needToken(url.searchParams.get('token'));
        if (!viewer) return httpLog(req, res, started);
        const target = store.getById(m[1]);
        if (!target) {
          return sendJson(res, 404, { error: 'not_found', message: '目标用户不存在' });
        }
        if (store.blockedEither(viewer.anon_id, target.anon_id)) {
          return sendJson(res, 403, { error: 'blocked', message: '对方已将你拉黑' });
        }
        return sendJson(res, 200, { soul: store.rowToProfile(target, viewer.anon_id) });
      }

      // ---- match ----
      if (req.method === 'GET' && p === '/api/match') {
        const viewer = needToken(url.searchParams.get('token'));
        if (!viewer) return httpLog(req, res, started);
        const pool = store.candidatePool(viewer.anon_id);
        const ranked = rankCandidates(viewer, pool, url.searchParams.get('limit'));
        return sendJson(res, 200, {
          souls: ranked.map(({ row, score }) => ({ profile: store.rowToProfile(row, viewer.anon_id), score })),
        });
      }

      // ---- city/:city/souls ----
      m = p.match(/^\/api\/city\/([^/]+)\/souls$/);
      if (m && req.method === 'GET') {
        const viewer = needToken(url.searchParams.get('token'));
        if (!viewer) return httpLog(req, res, started);
        const city = normalizeCity(decodeURIComponent(m[1]));
        const rows = store.citySouls(viewer.anon_id, city);
        return sendJson(res, 200, { souls: rows.map((r) => store.rowToProfile(r, viewer.anon_id)), city });
      }

      // ---- 占位 ----
      if (req.method === 'GET' && p === '/api/topics') {
        return sendJson(res, 200, { topics: TOPICS });
      }
      if (req.method === 'GET' && p === '/api/events') {
        return sendJson(res, 200, { events: EVENTS });
      }
      if (req.method === 'POST' && p === '/api/moderation/check') {
        const viewer = needToken(body.token);
        if (!viewer) return httpLog(req, res, started);
        const r = moderate(body.text);
        return sendJson(res, 200, { ok: true, flagged: r.flagged, reason: r.reason });
      }

      sendJson(res, 404, { error: 'not_found', message: 'no such api' });
    } catch (e) {
      sendJson(res, 500, { error: 'internal', message: e.message });
    } finally {
      httpLog(req, res, started);
    }
  };
}

/**
 * 启动服务。
 * @param {{host?:string, port?:number, dataDir?:string}} opts
 */
function startServer(opts = {}) {
  const host = opts.host || process.env.QIBAI_SERVER_HOST || '0.0.0.0';
  const port = Number(opts.port ?? process.env.QIBAI_SERVER_PORT ?? 8787);
  const dataDir = opts.dataDir || process.env.QIBAI_DATA_DIR || path.join(__dirname, '..', 'data');

  const store = new Store(dataDir);
  const hub = new WSHub(store);
  const server = http.createServer(createHandler(store, hub));

  server.on('upgrade', (req, socket, head) => {
    const url = new URL(req.url, 'http://localhost');
    if (url.pathname === '/ws') {
      hub.handleUpgrade(req, socket, head);
    } else {
      socket.destroy();
    }
  });

  server.listen(port, host, () => {
    console.log(`[HTTP] ${SERVER_NAME}@${VERSION} listening on http://${host}:${port} data=${dataDir}`);
  });

  return {
    server, store, hub,
    close() {
      hub.closeAll();
      server.close();
      store.close();
    },
  };
}

// 直接运行（npm start）
if (require.main === module) {
  startServer();
}

module.exports = { startServer, createHandler, SERVER_NAME, VERSION };
