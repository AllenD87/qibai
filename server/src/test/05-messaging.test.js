// §9.1-5 消息路由：在线实时投递 / 离线 inbox 补投 / 双向拉黑拒收 / 超长拒收 / 审核命中拒收
'use strict';

const { test, after, before } = require('node:test');
const assert = require('node:assert');
const WebSocket = require('ws');
const { startTestServer, api, register } = require('./helper');

let srv;
before(async () => { srv = await startTestServer(); });
after(async () => { await srv.close(); });

function connect(token) {
  const ws = new WebSocket(`${srv.wsUrl}?token=${token}`);
  const waitOpen = new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
  const waitMsg = (pred, timeout = 3000) => new Promise((res, rej) => {
    const h = (d) => {
      const m = JSON.parse(String(d));
      if (pred(m)) { ws.off('message', h); res(m); }
    };
    ws.on('message', h);
    setTimeout(() => rej(new Error('waitMsg timeout: ' + pred.toString())), timeout);
  });
  return { ws, waitOpen, waitMsg };
}

test('在线投递：A→B 双方都实时收到（B 收 deliver；A 收回执 from=自己）', async () => {
  const A = await register(srv.baseUrl, { nickname: '在线A' });
  const B = await register(srv.baseUrl, { nickname: '在线B' });
  const ca = connect(A.token); const cb = connect(B.token);
  await ca.waitOpen; await cb.waitOpen;

  const bRecv = cb.waitMsg((m) => m.type === 'msg.deliver' && m.from === A.anon_id);
  const aReceipt = ca.waitMsg((m) => m.type === 'msg.deliver' && m.from === A.anon_id);
  ca.ws.send(JSON.stringify({ type: 'msg.send', to: B.anon_id, body: '你好呀' }));

  const [brec, arec] = await Promise.all([bRecv, aReceipt]);
  assert.equal(brec.body, '你好呀');
  assert.equal(arec.body, '你好呀');
  assert.equal(brec.mid, arec.mid);

  ca.ws.close(); cb.ws.close();
});

test('离线补投：B 离线时 A 发 → B 上线后经 inbox 收到', async () => {
  const A = await register(srv.baseUrl, { nickname: '离线A' });
  const B = await register(srv.baseUrl, { nickname: '离线B' });
  const ca = connect(A.token);
  await ca.waitOpen;
  // B 不连接（离线）
  ca.ws.send(JSON.stringify({ type: 'msg.send', to: B.anon_id, body: '离线时发的' }));
  await ca.waitMsg((m) => m.type === 'msg.deliver' && m.body === '离线时发的');
  ca.ws.close();

  // B 上线：hello 之后应收到 inbox（先挂监听再等 open，避免 hello 帧丢失）
  const cb = connect(B.token);
  const helloP = cb.waitMsg((m) => m.type === 'hello');
  const inboxP = cb.waitMsg((m) => m.type === 'inbox');
  await cb.waitOpen;
  const hello = await helloP;
  assert.equal(hello.anon_id, B.anon_id);
  const inbox = await inboxP;
  assert.equal(inbox.messages.length, 1);
  assert.equal(inbox.messages[0].body, '离线时发的');
  assert.equal(inbox.messages[0].from, A.anon_id);
  cb.ws.close();
});

test('双向拉黑：B 拉黑 A 后，A 发消息被拒 error=blocked', async () => {
  const A = await register(srv.baseUrl, { nickname: '拉黑A' });
  const B = await register(srv.baseUrl, { nickname: '拉黑B' });
  // B 拉黑 A
  const r = await api(srv.baseUrl, 'POST', `/api/souls/${A.anon_id}/block`, { token: B.token });
  assert.equal(r.json.ok, true);

  const ca = connect(A.token);
  await ca.waitOpen;
  ca.ws.send(JSON.stringify({ type: 'msg.send', to: B.anon_id, body: '还能发吗' }));
  const err = await ca.waitMsg((m) => m.type === 'error');
  assert.equal(err.code, 'blocked');
  ca.ws.close();
});

test('超长 body（>500 字符）被拒 error=censored', async () => {
  const A = await register(srv.baseUrl, { nickname: '超长A' });
  const B = await register(srv.baseUrl, { nickname: '超长B' });
  const ca = connect(A.token);
  await ca.waitOpen;
  ca.ws.send(JSON.stringify({ type: 'msg.send', to: B.anon_id, body: '字'.repeat(501) }));
  const err = await ca.waitMsg((m) => m.type === 'error');
  assert.equal(err.code, 'censored');
  ca.ws.close();
});

test('moderation 命中敏感词被拒 error=censored', async () => {
  const A = await register(srv.baseUrl, { nickname: '审核A' });
  const B = await register(srv.baseUrl, { nickname: '审核B' });
  const ca = connect(A.token);
  await ca.waitOpen;
  ca.ws.send(JSON.stringify({ type: 'msg.send', to: B.anon_id, body: '这里有违禁色情内容' }));
  const err = await ca.waitMsg((m) => m.type === 'error');
  assert.equal(err.code, 'censored');
  ca.ws.close();
});
