// /api/souls 的 online 过滤参数：兼容 "true"/"false"/"1"/"0"（联调 D1 补充项）
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const WebSocket = require('ws');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('online=true / online=false 两种写法都生效（含 "true" 字符串）', async () => {
  const me = await register(srv.baseUrl, { nickname: '过滤me' });
  const onlineUser = await register(srv.baseUrl, { nickname: '在线者' });
  const offlineUser = await register(srv.baseUrl, { nickname: '离线者' });

  // 让 onlineUser 真正在线（连 WS）
  const ws = new WebSocket(`${srv.wsUrl}?token=${onlineUser.token}`);
  await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });

  // "true" 写法：只返回在线者
  const t = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: me.token, online: 'true' });
  const tIds = t.json.souls.map((s) => s.anon_id);
  assert.ok(tIds.includes(onlineUser.anon_id), 'online=true 返回在线用户');
  assert.ok(!tIds.includes(offlineUser.anon_id), 'online=true 不含离线用户');

  // "false" 写法：只返回离线者
  const f = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: me.token, online: 'false' });
  const fIds = f.json.souls.map((s) => s.anon_id);
  assert.ok(!fIds.includes(onlineUser.anon_id), 'online=false 不含在线用户');
  assert.ok(fIds.includes(offlineUser.anon_id), 'online=false 返回离线用户');

  // "1" 写法兼容
  const one = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: me.token, online: '1' });
  assert.ok(one.json.souls.some((s) => s.anon_id === onlineUser.anon_id));

  ws.close();
});
