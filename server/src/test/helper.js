// 测试共享 helper：起一个临时端口 + 临时数据目录的真实服务端
'use strict';

const os = require('node:os');
const fs = require('node:fs');
const path = require('node:path');
const { startServer } = require('../server');

async function startTestServer() {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'qibai-test-'));
  const app = startServer({ port: 0, host: '127.0.0.1', dataDir });
  await new Promise((resolve) => app.server.on('listening', resolve));
  const port = app.server.address().port;
  const baseUrl = `http://127.0.0.1:${port}`;
  return {
    app,
    port,
    baseUrl,
    wsUrl: `ws://127.0.0.1:${port}/ws`,
    dataDir,
    async close() {
      // 先关 WS 与 HTTP，再给 close 事件排空时间，最后关 DB，避免 teardown 竞态
      try { app.hub.closeAll(); } catch { /* 占位 */ }
      try { app.server.close(); } catch { /* 占位 */ }
      await new Promise((r) => setTimeout(r, 150));
      try { app.store.close(); } catch { /* 占位 */ }
      fs.rmSync(dataDir, { recursive: true, force: true });
    },
  };
}

async function api(baseUrl, method, urlPath, body, query) {
  let qs = '';
  if (query) {
    const sp = new URLSearchParams(query);
    qs = `?${sp.toString()}`;
  }
  const res = await fetch(baseUrl + urlPath + qs, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const json = await res.json().catch(() => ({}));
  return { status: res.status, json };
}

async function register(baseUrl, overrides = {}) {
  const body = {
    nickname: '星客',
    color_idx: 3,
    soul_tags: { personality: ['温柔'], interests: ['音乐'], status: ['想聊天'] },
    city: '石家庄',
    age_declared: '18plus',
    pure_mode: false,
    avatar: 0,
    ...overrides,
  };
  const r = await api(baseUrl, 'POST', '/api/register', body);
  return r.json; // {anon_id, token, profile}
}

module.exports = { startTestServer, api, register };
