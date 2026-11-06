// §9.1-1 注册：字段校验（昵称空/超长、color_idx clamp、soul_tags 上限）、token 生成与哈希存储
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('注册：昵称空 → 默认「星客」；超长昵称截 12 字符；color_idx clamp 0-11', async () => {
  const r1 = await register(srv.baseUrl, { nickname: '' });
  assert.equal(r1.profile.nickname, '星客');

  const r2 = await register(srv.baseUrl, { nickname: '一'.repeat(20) });
  assert.equal(r2.profile.nickname.length, 12);

  const r3 = await register(srv.baseUrl, { color_idx: 99 });
  assert.equal(r3.profile.color_idx, 11);

  const r4 = await register(srv.baseUrl, { color_idx: -5 });
  assert.equal(r4.profile.color_idx, 0);
});

test('注册：soul_tags 每组 ≤5 个、单标签 ≤8 字符、trim/去重', async () => {
  const r = await register(srv.baseUrl, {
    soul_tags: {
      personality: [' 温柔 ', '温柔', 'A'.repeat(20), '内向', '安静', '第三个', '第四个', '第五个', '第六个'],
      interests: [],
      status: [],
    },
  });
  const p = r.profile.soul_tags;
  assert.ok(p.personality.length <= 5, '每组最多 5 个');
  for (const t of p.personality) assert.ok(t.length <= 8, `标签 ≤8 字符: ${t}`);
  assert.ok(p.personality.includes('温柔'));
  assert.equal(p.personality[0], '温柔', 'trim 生效');
});

test('注册：token 为 32 hex；库里只存 sha256(token)，不存明文', async () => {
  const r = await register(srv.baseUrl, { nickname: '凭证校验' });
  assert.match(r.token, /^[0-9a-f]{32}$/);
  // 直接查库：token_hash 存在且不等于明文 token
  const row = srv.app.store.getById(r.anon_id);
  assert.ok(row.token_hash.length === 64);
  assert.notEqual(row.token_hash, r.token);
  // 用 token 能换到用户
  const me = await api(srv.baseUrl, 'GET', '/api/me', undefined, { token: r.token });
  assert.equal(me.status, 200);
  assert.equal(me.json.profile.anon_id, r.anon_id);
});

test('注册：profile 形状逐字段对齐规格', async () => {
  const r = await register(srv.baseUrl, { city: ' 石家庄 ', avatar: 2 });
  const p = r.profile;
  for (const k of ['anon_id', 'nickname', 'color_idx', 'soul_tags', 'city', 'avatar', 'pure_mode', 'online', 'last_seen', 'mutual_liked']) {
    assert.ok(k in p, `字段 ${k} 存在`);
  }
  assert.equal(p.city, '石家庄', '城市 trim');
  assert.equal(p.avatar, 2);
  assert.equal(p.online, false);
  assert.equal(p.mutual_liked, false);
});
