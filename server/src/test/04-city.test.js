// §9.1-4 同城分组：规范化相等匹配；空城市不误匹配
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('同城：规范化后完全相等才可见；空城市互不匹配', async () => {
  const me = await register(srv.baseUrl, { nickname: '同城me', city: ' 石家庄 ' });
  const sj = await register(srv.baseUrl, { nickname: '石家庄用户', city: '石家庄' });
  const bj = await register(srv.baseUrl, { nickname: '北京用户', city: '北京' });
  const empty = await register(srv.baseUrl, { nickname: '无城市用户', city: '' });

  const r = await api(srv.baseUrl, 'GET', '/api/city/%E7%9F%B3%E5%AE%B6%E5%BA%84/souls', undefined, { token: me.token });
  assert.equal(r.status, 200);
  assert.equal(r.json.city, '石家庄', '回显规范化城市');
  const ids = r.json.souls.map((x) => x.anon_id);
  assert.ok(ids.includes(sj.anon_id), '同城市可见');
  assert.ok(!ids.includes(bj.anon_id), '不同城市不可见');
  assert.ok(!ids.includes(empty.anon_id), '空城市不误匹配');
  assert.ok(!ids.includes(me.anon_id), '不含自己');
});

test('同城列表按 last_seen 降序', async () => {
  const me = await register(srv.baseUrl, { nickname: '排序me', city: '成都' });
  const u1 = await register(srv.baseUrl, { nickname: '成都早', city: '成都' });
  // 手工改 last_seen，制造先后
  srv.app.store.setStatus(u1.anon_id, 'offline', 1000);
  srv.app.store.setStatus(me.anon_id, 'offline', 500);
  // 再注册一个更晚 last_seen 的
  const u2 = await register(srv.baseUrl, { nickname: '成都晚', city: '成都' });
  const r = await api(srv.baseUrl, 'GET', '/api/city/%E6%88%90%E9%83%BD/souls', undefined, { token: me.token });
  const arr = r.json.souls;
  assert.ok(arr[0].last_seen >= arr[1].last_seen, '按 last_seen 降序');
});
