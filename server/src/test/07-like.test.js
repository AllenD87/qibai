// §9.1-7 点亮：互亮判定 matched
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('A 点亮 B：matched=false（B 尚未点亮 A）', async () => {
  const A = await register(srv.baseUrl, { nickname: '点亮A' });
  const B = await register(srv.baseUrl, { nickname: '点亮B' });
  const r = await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/like`, { token: A.token });
  assert.equal(r.json.liked, true);
  assert.equal(r.json.matched, false);
  assert.equal(r.json.mutual, false);
});

test('B 反向点亮 A：matched=true（互亮）', async () => {
  const A = await register(srv.baseUrl, { nickname: '互亮A' });
  const B = await register(srv.baseUrl, { nickname: '互亮B' });
  await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/like`, { token: A.token });
  const r = await api(srv.baseUrl, 'POST', `/api/souls/${A.anon_id}/like`, { token: B.token });
  assert.equal(r.json.matched, true, '对方已点亮过你 → matched');
  assert.equal(r.json.mutual, true);
});

test('重复点亮幂等，不影响 mutual_liked 标记', async () => {
  const A = await register(srv.baseUrl, { nickname: '重复点亮A' });
  const B = await register(srv.baseUrl, { nickname: '重复点亮B' });
  await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/like`, { token: A.token });
  await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/like`, { token: A.token });
  // B 视角的 souls 列表里 A 应 mutual_liked=true（A 已点亮 B）
  const r = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: B.token });
  const a = r.json.souls.find((s) => s.anon_id === A.anon_id);
  assert.ok(a, 'A 出现在 B 的列表');
  assert.equal(a.mutual_liked, true, 'B 看到 A 是已点亮自己的人');
});
