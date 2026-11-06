// §9.1-2 鉴权：/api/me 有效/无效 token；WS 无效 token 拒绝并关闭 4001
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const WebSocket = require('ws');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('HTTP：有效 token 返回 profile；无效 token → 401 invalid_token', async () => {
  const u = await register(srv.baseUrl, { nickname: '鉴权A' });
  const ok = await api(srv.baseUrl, 'GET', '/api/me', undefined, { token: u.token });
  assert.equal(ok.status, 200);
  assert.equal(ok.json.profile.anon_id, u.anon_id);

  const bad = await api(srv.baseUrl, 'GET', '/api/me', undefined, { token: 'deadbeef' });
  assert.equal(bad.status, 401);
  assert.equal(bad.json.error, 'invalid_token');

  const none = await api(srv.baseUrl, 'GET', '/api/me');
  assert.equal(none.status, 401);
});

test('WS：无效 token → 收到 error 帧并以 4001 关闭', async () => {
  const ws = new WebSocket(`${srv.wsUrl}?token=invalid_token_xxx`);
  const frames = [];
  const code = await new Promise((resolve, reject) => {
    ws.on('message', (d) => frames.push(JSON.parse(String(d))));
    ws.on('close', (c) => resolve(c));
    ws.on('error', reject);
    setTimeout(() => reject(new Error('timeout')), 3000);
  });
  assert.equal(code, 4001, 'close code 4001');
  assert.ok(frames.some((f) => f.type === 'error' && f.code === 'invalid_token'), `收到 error 帧: ${JSON.stringify(frames)}`);
});

test('WS：有效 token → hello + 空 inbox', async () => {
  const u = await register(srv.baseUrl, { nickname: '鉴权B' });
  const ws = new WebSocket(`${srv.wsUrl}?token=${u.token}`);
  const frames = await new Promise((resolve, reject) => {
    const fs = [];
    ws.on('message', (d) => {
      fs.push(JSON.parse(String(d)));
      if (fs.some((f) => f.type === 'hello')) resolve(fs);
    });
    ws.on('open', () => {});
    ws.on('error', reject);
    setTimeout(() => reject(new Error('timeout')), 3000);
  });
  const hello = frames.find((f) => f.type === 'hello');
  assert.equal(hello.anon_id, u.anon_id);
  assert.equal(hello.profile.nickname, '鉴权B');
  ws.close();
});
