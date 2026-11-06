// §9.1-3 匹配算法：标签交集权重（personality×3 / interests×2 / status×1）、同城+5、在线+2、
//         排除自己/拉黑/举报、同分按 last_seen 降序、limit 默认20上限50
'use strict';

const { test } = require('node:test');
const assert = require('node:assert');
const { scoreFor, rankCandidates, normalizeLimit } = require('../match');

const row = (over = {}) => ({
  anon_id: 'x',
  soul_tags: '{"personality":[],"interests":[],"status":[]}',
  city: '',
  status: 'offline',
  last_seen: 100,
  ...over,
});
const tags = (p, i, s) => JSON.stringify({ personality: p, interests: i, status: s });

test('权重：personality×3 / interests×2 / status×1', () => {
  const viewer = row({ soul_tags: tags(['温柔', '内向'], ['音乐', '电影'], ['想聊天']) });
  // 同时命中：personality 2个×3=6, interests 2个×2=4, status 1个×1=1 → 11
  const c1 = row({ soul_tags: tags(['温柔', '内向'], ['音乐', '电影'], ['想聊天']) });
  assert.equal(scoreFor(viewer, c1), 11);
  // 只命中 status 1个 → 1
  const c2 = row({ soul_tags: tags([], [], ['想聊天']) });
  assert.equal(scoreFor(viewer, c2), 1);
});

test('标签比较：忽略大小写 + trim', () => {
  const viewer = row({ soul_tags: tags([' 温柔 '], [], []) });
  const c = row({ soul_tags: tags(['温柔'], [], []) });
  assert.equal(scoreFor(viewer, c), 3); // 1×3
});

test('同城 +5（双方非空且规范化相等）；在线 +2', () => {
  const viewer = row({ city: '石家庄', soul_tags: tags([], [], []) });
  const same = row({ city: '石家庄' });
  assert.equal(scoreFor(viewer, same), 5);
  const diff = row({ city: '北京' });
  assert.equal(scoreFor(viewer, diff), 0);
  const online = row({ city: '石家庄', status: 'online' });
  assert.equal(scoreFor(viewer, online), 7); // 5+2
});

test('排序：score 降序；同分按 last_seen 降序；limit 截断', () => {
  const viewer = row({ soul_tags: tags(['温柔'], [], []) });
  const c1 = row({ anon_id: 'a', last_seen: 100, soul_tags: tags(['温柔'], [], []) }); // 3分
  const c2 = row({ anon_id: 'b', last_seen: 999, soul_tags: tags(['温柔'], [], []) });  // 3分
  const c3 = row({ anon_id: 'c', last_seen: 1, soul_tags: tags([], [], []) });          // 0分
  const ranked = rankCandidates(viewer, [c1, c2, c3], 10);
  assert.deepEqual(ranked.map((r) => r.row.anon_id), ['b', 'a', 'c']); // 同分 b(999)>a(100)
});

test('limit：默认 20、上限 50', () => {
  assert.equal(normalizeLimit(undefined), 20);
  assert.equal(normalizeLimit(1000), 50);
  assert.equal(normalizeLimit(-3), 20);
  assert.equal(normalizeLimit(30), 30);
  const viewer = row();
  const pool = Array.from({ length: 60 }, (_, i) => row({ anon_id: `u${i}` }));
  assert.equal(rankCandidates(viewer, pool, 1000).length, 50);
});

test('候选池排除：自己 / 双向拉黑 / 被举报过（集成 Store）', async () => {
  const os = require('node:os');
  const fs = require('node:fs');
  const path = require('node:path');
  const { Store } = require('../store');
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'qibai-match-'));
  const store = new Store(dataDir);
  const mk = (nick) => store.createUser({ nickname: nick, soul_tags: {}, city: '', pure_mode: false, age_declared: '18plus', color_idx: 0, avatar: 0 }).row;

  const me = mk('me');
  const otherA = mk('正常');
  const blockedX = mk('拉黑对象');
  const reportedY = mk('被举报对象');

  store.addBlock(me.anon_id, blockedX.anon_id);          // 我拉黑他
  store.addReport(me.anon_id, reportedY.anon_id, 'spam'); // 他被举报

  const pool = store.candidatePool(me.anon_id).map((r) => r.anon_id);
  assert.ok(!pool.includes(me.anon_id), '排除自己');
  assert.ok(!pool.includes(blockedX.anon_id), '排除拉黑');
  assert.ok(!pool.includes(reportedY.anon_id), '排除被举报过');
  assert.ok(pool.includes(otherA.anon_id), '正常候选保留');

  store.close();
  fs.rmSync(dataDir, { recursive: true, force: true });
});
