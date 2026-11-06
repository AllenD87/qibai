// CORS：file:// 页面 fetch /api/* 直连本地服务端所需（联调补充项）
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('OPTIONS 预检：204 + CORS 头（Origin/Allow-Methods/Allow-Headers）', async () => {
  const res = await fetch(srv.baseUrl + '/api/me?token=x', { method: 'OPTIONS' });
  assert.equal(res.status, 204);
  assert.equal(res.headers.get('Access-Control-Allow-Origin'), '*');
  assert.equal(res.headers.get('Access-Control-Allow-Methods'), 'GET,POST,PUT,DELETE,OPTIONS');
  assert.equal(res.headers.get('Access-Control-Allow-Headers'), 'Content-Type');
});

test('普通 /api/* 响应：带 Access-Control-Allow-Origin: *', async () => {
  // health
  const h = await fetch(srv.baseUrl + '/api/health');
  assert.equal(h.headers.get('Access-Control-Allow-Origin'), '*');
  // 401 错误响应同样带头
  const bad = await fetch(srv.baseUrl + '/api/me?token=nope');
  assert.equal(bad.status, 401);
  assert.equal(bad.headers.get('Access-Control-Allow-Origin'), '*');
  // 正常注册响应带头
  const U = await register(srv.baseUrl, { nickname: 'CORS用户' });
  assert.ok(U.anon_id);
});
