/* 元气小小事件链 Node 仿真测试
 * 忠实复刻 tauri-2.12.4 的 IPC 事件机制（scripts/bundle.global.js + src/event/*）：
 *   listen(event, handler, opts) → invoke('plugin:event|listen',
 *        { event, target: {kind:'Any'}, handler: transformCallback(fn) })
 *      → 返回 eventId → 注销走 invoke('plugin:event|unlisten', {event, eventId})
 * 关键契约（真机实证过的三个必填项）：
 *   1) target 缺失 → "missing required key target"
 *   2) handler 缺失（裸函数无法 JSON 序列化）→ "missing required key handler"
 *   3) handler 必须是数字回调 id，由 transformCallback 生成
 * 目的：在沙盒无 Tauri 环境下实证前端事件链是否按真实契约工作。
 */
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const callbacks = new Map();   // id -> fn  （对应 __TAURI_INTERNALS__.callbacks）
const registered = {};         // event -> callbackId
let nextId = 1;
let registeredTarget = null;
let eventIdSeq = 1;
const unlistened = [];         // 记录 unlisten 调用参数
const backendLog = [];
let invokeCalls = [];

const sandbox = {
  console: { log: () => {}, error: () => {}, warn: () => {} },
  localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => {},
  setInterval: () => 0,
  clearInterval: () => {},
  setTimeout: (fn) => 0,
  clearTimeout: () => {},
  CustomEvent: function (name, opts) { return { name, detail: opts && opts.detail }; },
  document: { addEventListener: () => {}, removeEventListener: () => {}, hidden: false },
  navigator: { mediaDevices: undefined },
  location: { href: 'tauri://localhost', search: '' },
  __TAURI_INTERNALS__: {
    callbacks: callbacks,
    // 对应 scripts/core.js 的 transformCallback：注册函数 → 返回数字 id
    transformCallback: (fn) => { const id = nextId++; callbacks.set(id, fn); return id; },
    unregisterCallback: (id) => { callbacks.delete(id); },
    // 对应 runCallback：后端派发事件时按 id 调用前端回调
    runCallback: (id, payload) => { const fn = callbacks.get(id); if (fn) fn(payload); },
    invoke: async (cmd, args) => {
      invokeCalls.push(cmd);
      if (cmd === 'plugin:event|listen') {
        // 复刻 Tauri 命令反序列化时的必填校验（顺序与实测一致：event → target → handler）
        if (!args || !args.event) {
          throw new Error("invalid args `event` for command `listen`: command listen missing required key event");
        }
        if (args.target === undefined) {
          throw new Error("invalid args `target` for command `listen`: command listen missing required key target");
        }
        if (!args.target.kind) {
          throw new Error("invalid args `target` for command `listen`: invalid target: missing kind");
        }
        if (args.handler === undefined) {
          throw new Error("invalid args `handler` for command `listen`: command listen missing required key handler");
        }
        // CallbackFn 必须是 transformCallback 生成的数字 id（裸函数无法 JSON 序列化 → 丢失）
        if (typeof args.handler !== 'number' || !callbacks.has(args.handler)) {
          throw new Error("invalid args `handler` for command `listen`: expected a callback id");
        }
        registered[args.event] = args.handler;
        registeredTarget = args.target;
        return eventIdSeq++;
      }
      if (cmd === 'plugin:event|unlisten') {
        unlistened.push(args);
        if (typeof args.handler === 'number' && callbacks.has(args.handler)) callbacks.delete(args.handler);
        return null;
      }
      if (cmd === 'debug_log') { backendLog.push(args.msg); return null; }
      return {};
    },
  },
};
sandbox.window = sandbox;
vm.createContext(sandbox);

const code = fs.readFileSync(path.join(__dirname, '..', 'src', 'tauri-bridge.js'), 'utf8');
vm.runInContext(code, sandbox);

let pass = 0, fail = 0;
function check(name, cond) { if (cond) { pass++; console.log('  PASS ' + name); } else { fail++; console.log('  FAIL ' + name); } }
// 模拟后端 emit：按 callback id 派发（与 Tauri emit_js 一致）
function backendEmit(event, payload) {
  const id = registered[event];
  if (!id) throw new Error('no listener for ' + event);
  sandbox.__TAURI_INTERNALS__.runCallback(id, { event: event, id: 1, payload: payload });
}

(async () => {
  const Q = sandbox.window.QibaiBridge;
  console.log('== 1. 桥接层加载 ==');
  check('QibaiBridge 存在', !!Q);
  check('listen 是函数', typeof Q.listen === 'function');
  check('isTauri 判定为 true', Q.isTauri === true);

  console.log('== 2. listen 注册（模拟 pet.js 监听 ctx-action） ==');
  const received = [];
  const off = await Q.listen('qibai://ctx-action', (event) => {
    let payload = event && event.payload;
    if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch (e) {} }
    received.push(payload && payload.action);
  });
  check('plugin:event|listen 被调用', invokeCalls.includes('plugin:event|listen'));
  check('event 已注册', registered['qibai://ctx-action'] !== undefined);
  check('handler 为数字回调 id', typeof registered['qibai://ctx-action'] === 'number');
  check('回调 id 已在 callbacks 表内', callbacks.has(registered['qibai://ctx-action']));
  check('target 参数已传入（必填）', !!registeredTarget);
  check('target.kind 为 Any', registeredTarget && registeredTarget.kind === 'Any');
  check('listen 返回注销函数', typeof off === 'function');

  console.log('== 3. 后端 emit → 完整事件对象形态（Tauri 标准） ==');
  backendEmit('qibai://ctx-action', { action: 'transbg' });
  check('handler 收到事件', received.length === 1);
  check('action 解析为 transbg', received[0] === 'transbg');

  console.log('== 4. 后端 emit → 裸 payload 形态（兼容） ==');
  received.length = 0;
  backendEmit('qibai://ctx-action', { action: 'lock' });
  check('裸 payload 被 normalized 包装', received[0] === 'lock');

  console.log('== 5. 后端 emit → payload 为 JSON 字符串形态 ==');
  received.length = 0;
  backendEmit('qibai://ctx-action', '{"action":"transbg"}');
  check('字符串 payload 被 parse', received[0] === 'transbg');

  console.log('== 6. 多个 action 连续分发 ==');
  received.length = 0;
  backendEmit('qibai://ctx-action', { action: 'guard' });
  backendEmit('qibai://ctx-action', { action: 'minimal' });
  backendEmit('qibai://ctx-action', { action: 'transbg' });
  check('三个 action 依次正确', JSON.stringify(received) === '["guard","minimal","transbg"]');

  console.log('== 7. 30 种事件批量注册（模拟 pet.js 真实监听面） ==');
  const events = ['bubble-dismiss','codecheck-chat-demo','codecheck-run','conversation-updated',
    'ctx-action','desk-changed','guard-start','intimacy-updated','listen-state','llm-status',
    'mute-toggle','panel-content','pet-hide','pet-perform','pet-say','pet-show',
    'pipecat-reply','pipecat-transcript','pk-end','pk-round','reminder','screen-shot',
    'skill-approval','toast','tts-chunk','tts-interrupted','tts-playback','tts-speak',
    'ui-minimal','ui-nav','ui-panel','voice-chat-demo'];
  let okAll = true;
  for (const e of events) {
    try { await Q.listen('qibai://' + e, () => {}); } catch (err) { okAll = false; }
  }
  check('全部事件注册成功（无 ACL/参数错误）', okAll);
  check('注册数 ≥ 30', Object.keys(registered).length >= 30);
  check('每个 handler 都是数字 id', Object.values(registered).every((v) => typeof v === 'number'));

  console.log('== 8. 注销（plugin:event|unlisten + eventId） ==');
  await off();
  check('unlisten 被调用', unlistened.length >= 1);
  check('unlisten 用 eventId 注销', typeof unlistened[0].eventId === 'number');
  check('unlisten 事件名正确', unlistened[0].event === 'qibai://ctx-action');

  console.log('\n结果: ' + pass + ' passed, ' + fail + ' failed');
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error('测试异常:', e); process.exit(1); });
