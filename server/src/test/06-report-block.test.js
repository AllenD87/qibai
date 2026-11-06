// §9.1-6 举报/拉黑：记录落库、重复幂等、拉黑后双方不可见（souls 列表与详情 403）
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('举报：记录落库', async () => {
  const A = await register(srv.baseUrl, { nickname: '举报人' });
  const B = await register(srv.baseUrl, { nickname: '被举报人' });
  const r = await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/report`, { token: A.token, reason: '垃圾信息' });
  assert.equal(r.json.ok, true);
  const rows = srv.app.db || null;
  const rep = srv.app.store.db.prepare('SELECT * FROM reports WHERE reporter=? AND target=?').get(A.anon_id, B.anon_id);
  assert.ok(rep, 'reports 表有记录');
  assert.equal(rep.reason, '垃圾信息');
});

test('拉黑：重复幂等；souls 列表不再含对方；详情 403 blocked', async () => {
  const A = await register(srv.baseUrl, { nickname: '幂等A' });
  const B = await register(srv.baseUrl, { nickname: '幂等B' });

  // 拉黑前互相可见
  const before = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: A.token });
  assert.ok(before.json.souls.some((s) => s.anon_id === B.anon_id), '拉黑前可见');

  // 拉黑 + 重复拉黑（幂等）
  const r1 = await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/block`, { token: A.token });
  const r2 = await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/block`, { token: A.token });
  assert.equal(r1.json.ok, true);
  assert.equal(r2.json.ok, true);
  const cnt = srv.app.store.db.prepare('SELECT COUNT(*) c FROM blocks WHERE blocker=? AND blocked=?').get(A.anon_id, B.anon_id);
  assert.equal(cnt.c, 1, '主键幂等');

  // 拉黑后列表不含对方
  const after = await api(srv.baseUrl, 'GET', '/api/souls', undefined, { token: A.token });
  assert.ok(!after.json.souls.some((s) => s.anon_id === B.anon_id), '拉黑后列表不可见');

  // 详情 403
  const detail = await api(srv.baseUrl, 'GET', `/api/souls/${B.anon_id}`, undefined, { token: A.token });
  assert.equal(detail.status, 403);
  assert.equal(detail.json.error, 'blocked');

  // B 看 A 的详情也是 403（双向）
  const back = await api(srv.baseUrl, 'GET', `/api/souls/${A.anon_id}`, undefined, { token: B.token });
  assert.equal(back.status, 403);
});

test('拉黑后 match 候选池也排除对方', async () => {
  const A = await register(srv.baseUrl, { nickname: 'matchA' });
  const B = await register(srv.baseUrl, { nickname: 'matchB' });
  await api(srv.baseUrl, 'POST', `/api/souls/${B.anon_id}/block`, { token: A.token });
  const r = await api(srv.baseUrl, 'GET', '/api/match', undefined, { token: A.token });
  assert.ok(!r.json.souls.some((s) => s.profile.anon_id === B.anon_id), 'match 不含被拉黑者');
});
