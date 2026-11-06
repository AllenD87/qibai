// §9.1-8 health / 占位接口（topics/events/moderation）形状正确
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const { startTestServer, api, register } = require('./helper');
const { moderate } = require('../moderation');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

test('health：形状 {ok, name, version, ts}', async () => {
  const r = await api(srv.baseUrl, 'GET', '/api/health');
  assert.equal(r.status, 200);
  assert.equal(r.json.ok, true);
  assert.equal(r.json.name, 'qibai-net');
  assert.equal(r.json.version, '0.1.0');
  assert.ok(Number.isInteger(r.json.ts));
});

test('topics ≥3 条；events ≥2 条', async () => {
  const t = await api(srv.baseUrl, 'GET', '/api/topics');
  assert.ok(t.json.topics.length >= 3);
  for (const x of t.json.topics) {
    for (const k of ['id', 'title', 'sticky', 'ts']) assert.ok(k in x, `topic 字段 ${k}`);
  }
  const e = await api(srv.baseUrl, 'GET', '/api/events');
  assert.ok(e.json.events.length >= 2);
  for (const x of e.json.events) {
    for (const k of ['id', 'title', 'when', 'where', 'ts']) assert.ok(k in x, `event 字段 ${k}`);
  }
});

test('moderation/check：未命中 / 命中 / 空文本 / 超长', async () => {
  const U = await register(srv.baseUrl, { nickname: '审核接口' });
  const clean = await api(srv.baseUrl, 'POST', '/api/moderation/check', { token: U.token, text: '今天天气不错' });
  assert.equal(clean.json.flagged, false);
  assert.equal(clean.json.reason, '');

  const hit = await api(srv.baseUrl, 'POST', '/api/moderation/check', { token: U.token, text: '色情内容' });
  assert.equal(hit.json.flagged, true);
  assert.ok(hit.json.reason);

  const empty = await api(srv.baseUrl, 'POST', '/api/moderation/check', { token: U.token, text: '   ' });
  assert.equal(empty.json.flagged, false, '空文本不命中');

  const tooLong = await api(srv.baseUrl, 'POST', '/api/moderation/check', { token: U.token, text: '好'.repeat(501) });
  assert.equal(tooLong.json.flagged, true);
  assert.equal(tooLong.json.reason, 'too_long');

  // 函数级直接单测覆盖
  assert.equal(moderate('正常文字').flagged, false);
  assert.equal(moderate('赌博广告').reason, 'gambling');
});

test('未注册路由 → 404', async () => {
  const r = await api(srv.baseUrl, 'GET', '/api/nope');
  assert.equal(r.status, 404);
});
