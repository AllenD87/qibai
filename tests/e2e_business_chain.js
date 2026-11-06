/* 元气小小 全业务链路 E2E 测试（真实 Chromium + 仿真 Tauri 后端）
 *
 * 覆盖链路（不含 Rust/窗口层，其余全真）：
 *   右键菜单点击 → invoke('ctx_menu_pick') → 后端 emit('qibai://ctx-action')
 *   → 前端 listen 回调（transformCallback/runCallback 真机制）
 *   → state.transparentBg 切换 → repaintFrame 重绘
 *   → canvas 像素是否真的变透明（像素级断言，不看日志自证）
 *   → save_settings 持久化 → 重载后仍是开
 *   同时验证 CSS：body 背景透明、28px 圆角
 *
 * 运行：node tests/e2e_business_chain.js
 * 退出码 0 = 全绿
 */
const { chromium } = require('playwright');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const PET = 'file://' + path.join(ROOT, 'src/pet.html');
const MENU = 'file://' + path.join(ROOT, 'src/menu.html');

// 注入仿真 Tauri 后端（在页面脚本之前执行）
function backendInit() {
  const callbacks = new Map();
  let nextCallbackId = 1;
  let nextEventId = 1;
  const listeners = {};       // event -> [callbackId]
  const logs = [];            // debug_log 收集
  const calls = [];           // 所有 invoke 记录
  const errors = [];          // invoke 抛错记录
  const db = { settings: { ui: { transparent_bg: false } } };
  try { const savedRenderInfo = localStorage.getItem('__qibaiRenderInfo'); if (savedRenderInfo) db.renderInfo = JSON.parse(savedRenderInfo); } catch (e) {}

  const emitCalls = [];
  function emit(event, payload) {
    emitCalls.push(event);
    const ids = listeners[event] || [];
    for (const id of ids) {
      const fn = callbacks.get(id);
      if (fn) fn({ event, id: 1, payload });
    }
    return ids.length;
  }

  const internals = {
    callbacks,
    transformCallback: (fn) => { const id = nextCallbackId++; callbacks.set(id, fn); return id; },
    unregisterCallback: (id) => { callbacks.delete(id); },
    runCallback: (id, p) => { const fn = callbacks.get(id); if (fn) fn(p); },
    invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      // —— 事件插件（与 tauri-2.12.4 契约一致，校验必填项）——
      if (cmd === 'plugin:event|listen') {
        if (!args || args.event === undefined) { const e = new Error("missing required key event"); errors.push(cmd + ': ' + e.message); throw e; }
        if (args.target === undefined) { const e = new Error("missing required key target"); errors.push(cmd + ': ' + e.message); throw e; }
        if (args.handler === undefined) { const e = new Error("missing required key handler"); errors.push(cmd + ': ' + e.message); throw e; }
        if (typeof args.handler !== 'number' || !callbacks.has(args.handler)) { const e = new Error("handler is not a registered callback id"); errors.push(cmd + ': ' + e.message); throw e; }
        if (!args.target.kind) { const e = new Error("missing target kind"); errors.push(cmd + ': ' + e.message); throw e; }
        listeners[args.event] = listeners[args.event] || [];
        listeners[args.event].push(args.handler);
        return nextEventId++;
      }
      if (cmd === 'plugin:event|unlisten') {
        const arr = listeners[args.event] || [];
        listeners[args.event] = arr.filter((x) => x !== args.eventId);
        return null;
      }
      // —— 应用命令 ——
      if (cmd === 'debug_log') { logs.push(args && args.msg); return null; }
      if (cmd === 'get_settings') { return JSON.parse(JSON.stringify(db.settings)); }
      if (cmd === 'pet_render_info') { return db.renderInfo || {}; }
      if (cmd === 'save_settings') {
        db.settings = JSON.parse(JSON.stringify(args.settings || {}));
        return null;
      }
      // 复刻 main.rs：ctx_menu_pick 关菜单窗后向其它窗口 emit ctx-action
      if (cmd === 'ctx_menu_pick') { emit('qibai://ctx-action', { action: args && args.action }); return null; }
      if (cmd === 'ctx_menu_close') { return null; }
      // 其余命令（窗口/角色/记忆等）返回空对象，避免初始化链中断
      return {};
    },
  };

  window.__TAURI_INTERNALS__ = internals;
  window.__qibaiTest = { callbacks, listeners, logs, calls, errors, db, emit, emitCalls, invoke: (c, a) => internals.invoke(c, a) };
}
const INIT_SCRIPT = '(' + backendInit.toString() + ')();';

let pass = 0, fail = 0;
const failures = [];
function check(name, cond, extra) {
  if (cond) { pass++; console.log('  PASS ' + name); }
  else { fail++; failures.push(name + (extra !== undefined ? ' | ' + JSON.stringify(extra) : '')); console.log('  FAIL ' + name + (extra !== undefined ? ' | ' + JSON.stringify(extra) : '')); }
}

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--allow-file-access-from-files', '--no-sandbox', '--disable-dev-shm-usage'],
  });
  const ctx = await browser.newContext({ viewport: { width: 360, height: 360 }, deviceScaleFactor: 1 });

  // ---------- 场景 1：主窗（pet.html）透明链路 ----------
  const page = await ctx.newPage();
  const pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(String(e)));
  await page.addInitScript(INIT_SCRIPT);
  await page.goto(PET, { waitUntil: 'load' });
  await page.waitForTimeout(1200);

  console.log('== 1. 主窗初始化 / 监听注册 ==');
  const st1 = await page.evaluate(() => ({
    logged: !!window.__qibaiTest.logs.some((m) => /listen-registered/.test(m || '')),
    failed: window.__qibaiTest.logs.filter((m) => /listen-failed/.test(m || '')),
    ctxListeners: (window.__qibaiTest.listeners['qibai://ctx-action'] || []).length,
    totalEvents: Object.keys(window.__qibaiTest.listeners).length,
    errors: window.__qibaiTest.errors,
    hasCanvas: !!document.getElementById('bgCanvas'),
    bodyBg: getComputedStyle(document.body).backgroundColor,
    bodyRadius: getComputedStyle(document.body).borderRadius,
    eventNames: Object.keys(window.__qibaiTest.listeners),
  }));
  check('listen 注册成功（无 listen-failed）', st1.logged && st1.failed.length === 0, st1.failed);
  check('ctx-action 有监听者', st1.ctxListeners >= 1, st1.ctxListeners);
  check('事件监听总数 ≥ 18', st1.totalEvents >= 18, { total: st1.totalEvents, events: st1.eventNames });
  check('无 IPC 契约错误', st1.errors.length === 0, st1.errors);
  check('bgCanvas 存在', st1.hasCanvas);
  check('body 背景透明', /rgba\(0, 0, 0, 0\)|transparent/.test(st1.bodyBg), st1.bodyBg);
  check('body 28px 圆角', /28px/.test(st1.bodyRadius), st1.bodyRadius);

  // 采样画布像素（画布左上角区域，透明模式下 alpha 必须为 0）
  const samplePixel = () => page.evaluate(() => {
    const c = document.getElementById('bgCanvas');
    if (!c) return null;
    const g = c.getContext('2d');
    const d = g.getImageData(2, 2, 1, 1).data;
    return [d[0], d[1], d[2], d[3]];
  });

  console.log('== 2. 默认态（关闭）画布应不透明 ==');
  const pxOff = await samplePixel();
  check('关闭态 alpha=255（有底色覆盖）', pxOff && pxOff[3] === 255, pxOff);

  console.log('== 2b. 外观：输入条与画布对齐 / 提示条浅色贴顶 ==');
  const geo = await page.evaluate(() => {
    const c = document.getElementById('bgCanvas').getBoundingClientRect();
    const bar = document.querySelector('.chat-bar');
    const b = bar ? bar.getBoundingClientRect() : null;
    return { canvas: [c.left, c.right, c.width], bar: b ? [b.left, b.right, b.width] : null };
  });
  check('输入条左缘与画布对齐（≤1px）', geo.bar && Math.abs(geo.bar[0] - geo.canvas[0]) <= 1, geo);
  check('输入条右缘与画布对齐（≤1px）', geo.bar && Math.abs(geo.bar[1] - geo.canvas[1]) <= 1, geo);

  console.log('== 2c. 外观：快捷短语不裁字 / 气泡能容纳长文 ==');
  const chips = await page.evaluate(() => {
    const box = document.querySelector('.quick-chips');
    const items = [...document.querySelectorAll('.quick-chips .chip')].map((x) => {
      const r = x.getBoundingClientRect();
      return { t: x.textContent.trim(), left: r.left, right: r.right, top: r.top };
    });
    const r = box.getBoundingClientRect();
    return { innerW: innerWidth, boxH: r.height, items, clipped: items.filter((i) => i.right > innerWidth + 1).map((i) => i.t), scrollW: box.scrollWidth, clientW: box.clientWidth };
  });
  check('快捷短语无一项被窗口右缘裁掉', chips.clipped.length === 0, chips.clipped);
  check('快捷短语容器高度 ≤ 70px（两行内）', chips.boxH <= 70, chips.boxH);

  const bubble = await page.evaluate(() => {
    const b = document.getElementById('speechBubble');
    if (!b) return null;
    const cs = getComputedStyle(b);
    const txt = document.getElementById('speechText');
    b.classList.remove('hidden');
    if (txt) txt.textContent = '这是一段很长的测试文案，用来确认气泡在长回复时不会把后面的字截掉，'.repeat(4);
    const r = b.getBoundingClientRect();
    const scrollable = txt.scrollHeight > txt.clientHeight;
    txt.scrollTop = txt.scrollHeight;
    return { width: r.width, maxW: cs.maxWidth, maxH: cs.maxHeight, overflowY: getComputedStyle(txt).overflowY, scrollable, scrollTop: txt.scrollTop, scrollH: txt.scrollHeight, clientH: txt.clientHeight };
  });
  check('气泡宽度 ≥ 240px（旧值 170 太窄）', bubble && bubble.width >= 240, bubble && bubble.width);
  check('气泡超长时可内部滚动', bubble && bubble.overflowY === 'auto' && bubble.scrollable, bubble);
  check('气泡可滚到最新一行（scrollTop>0）', bubble && bubble.scrollTop > 0, bubble && bubble.scrollTop);

  // 行为断言：气泡在说话时点静音（该路径会 toast），提示条应延后到气泡收起才出现
  const avoid = await page.evaluate(async () => {
    const b = document.getElementById('speechBubble');
    b.classList.remove('hidden');
    document.getElementById('btnMute').click();
    await new Promise((r) => setTimeout(r, 260));
    const during = document.querySelectorAll('.toast-box .toast').length;
    b.classList.add('hidden');
    await new Promise((r) => setTimeout(r, 400));
    const appears = document.querySelectorAll('.toast-box .toast').length;
    const top = document.querySelector('.toast-box .toast')
      ? getComputedStyle(document.querySelector('.toast-box .toast')).backgroundColor : '';
    return { during, appears, top, toastTop: document.querySelector('.toast-box') ? getComputedStyle(document.querySelector('.toast-box')).top : '' };
  });
  check('气泡说话时不弹提示条（不盖脸）', avoid.during === 0, avoid.during);
  check('气泡收起后提示条补弹出来', avoid.appears >= 1, avoid.appears);
  check('提示条回到窗口顶部（≤20px）', parseFloat(avoid.toastTop) <= 20, avoid.toastTop);

  console.log('== 3. 全链路：菜单点「透明背景」→ 后端 emit → 前端切换 ==');
  // 等价于 menu.html 点击触发 invoke('ctx_menu_pick', {action:'transbg'})
  await page.evaluate(() => window.__qibaiTest.invoke('ctx_menu_pick', { action: 'transbg' }));
  await page.waitForTimeout(400);
  const st3 = await page.evaluate(() => ({
    logs: window.__qibaiTest.logs.filter((m) => /\[transbg\]/.test(m || '')),
    settings: window.__qibaiTest.db.settings,
    menuText: (document.getElementById('ctxTransBgText') || {}).textContent,
    saved: window.__qibaiTest.calls.filter((c) => c.cmd === 'save_settings').length,
  }));
  const pxOn = await samplePixel();
  check('handler 被执行（handler-enter 日志）', st3.logs.some((m) => /handler-enter/.test(m)), st3.logs);
  check('状态已切换（after-toggle state=true）', st3.logs.some((m) => /after-toggle state=true/.test(m)), st3.logs);
  check('画布真的透明了 alpha=0', pxOn && pxOn[3] === 0, pxOn);
  check('设置已持久化 transparent_bg=true', st3.settings && st3.settings.ui && st3.settings.ui.transparent_bg === true, st3.settings);
  check('save_settings 已调用', st3.saved >= 1, st3.saved);
  check('菜单文字同步为「开」', /开/.test(st3.menuText || ''), st3.menuText);
  const toastLook = await page.evaluate(() => {
    // 真实流程：右键菜单打开时 openCtxMenu 会先隐藏气泡，提示条回到窗口顶部
    const bb = document.getElementById('speechBubble');
    if (bb) bb.classList.add('hidden');
    const t = document.querySelector('.toast');
    if (!t) return null;
    const cs = getComputedStyle(t);
    const r = t.getBoundingClientRect();
    const m = cs.backgroundColor.match(/rgba?\(([^)]+)\)/);
    const parts = m ? m[1].split(',').map((x) => parseFloat(x)) : [];
    return { bg: cs.backgroundColor, rgb: parts.slice(0, 3), top: r.top, text: t.textContent };
  });
  check('提示条不再是黑色底（亮色 ≥200）', toastLook && toastLook.rgb.length === 3 && toastLook.rgb.every((v) => v >= 200), toastLook && toastLook.bg);
  check('提示条贴窗口顶部（不压角色脸）', toastLook && toastLook.top <= 20, toastLook && toastLook.top);

  console.log('== 4. 再点一次应关闭（可逆） ==');
  await page.evaluate(() => window.__qibaiTest.invoke('ctx_menu_pick', { action: 'transbg' }));
  await page.waitForTimeout(300);
  const pxBack = await samplePixel();
  const st4 = await page.evaluate(() => ({ settings: window.__qibaiTest.db.settings, menuText: (document.getElementById('ctxTransBgText') || {}).textContent }));
  check('关闭态 alpha 回到 255', pxBack && pxBack[3] === 255, pxBack);
  check('设置回写 false', st4.settings.ui.transparent_bg === false, st4.settings);
  check('菜单文字回「关」', /关/.test(st4.menuText || ''), st4.menuText);

  console.log('== 5. 持久化：设成开后重载，应保持透明 ==');
  await page.evaluate(() => window.__qibaiTest.invoke('ctx_menu_pick', { action: 'transbg' }));
  await page.waitForTimeout(300);
  const page2 = await ctx.newPage();
  page2.on('pageerror', (e) => pageErrors.push(String(e)));
  await page2.addInitScript(INIT_SCRIPT);
  await page2.addInitScript(() => {
    // 预置磁盘设置：透明背景已是开（模拟上次保存成功）
    window.__PRESET_SETTINGS__ = { ui: { transparent_bg: true } };
  });
  // 重载前把 db 预置：通过 addInitScript 顺序在 backendInit 之后执行
  await page2.addInitScript(() => {
    if (window.__qibaiTest && window.__PRESET_SETTINGS__) window.__qibaiTest.db.settings = window.__PRESET_SETTINGS__;
  });
  await page2.goto(PET, { waitUntil: 'load' });
  await page2.waitForTimeout(1200);
  const pxPersist = await page2.evaluate(() => {
    const c = document.getElementById('bgCanvas'); if (!c) return null;
    const d = c.getContext('2d').getImageData(2, 2, 1, 1).data; return [d[0], d[1], d[2], d[3]];
  });
  const st5 = await page2.evaluate(() => ({
    logsFailed: window.__qibaiTest.logs.filter((m) => /listen-failed/.test(m || '')),
    menuText: (document.getElementById('ctxTransBgText') || {}).textContent,
  }));
  check('重载后仍为透明 alpha=0', pxPersist && pxPersist[3] === 0, pxPersist);
  check('重载后菜单显示「开」', /开/.test(st5.menuText || ''), st5.menuText);
  check('重载后 listen 无失败', st5.logsFailed.length === 0, st5.logsFailed);

  // ---------- 场景 6：独立菜单窗（menu.html）点击链路 ----------
  console.log('== 6. menu.html 点击 → ctx_menu_pick(transbg) ==');
  const mpage = await ctx.newPage();
  mpage.on('pageerror', (e) => pageErrors.push('menu: ' + String(e)));
  await mpage.addInitScript(INIT_SCRIPT);
  await mpage.goto(MENU, { waitUntil: 'load' });
  await mpage.waitForTimeout(500);
  await mpage.click('#ctxTransBg');
  await mpage.waitForTimeout(300);
  const st6 = await mpage.evaluate(() => ({
    calls: window.__qibaiTest.calls.filter((c) => c.cmd === 'ctx_menu_pick'),
    emitted: window.__qibaiTest.emitCalls.filter((e) => e === 'qibai://ctx-action').length,
    text: (document.getElementById('ctxTransBgText') || {}).textContent,
    errors: window.__qibaiTest.errors,
  }));
  check('菜单点击发出 ctx_menu_pick(transbg)', st6.calls.some((c) => c.args && c.args.action === 'transbg'), st6.calls);
  check('菜单点击后后端 emit 了 ctx-action', st6.emitted >= 1, st6.emitted);
  check('菜单文字按设置显示（关）', /关/.test(st6.text || ''), st6.text);
  const menuLook = await mpage.evaluate(() => {
    const card = document.getElementById('ctxMenu');
    const cs = getComputedStyle(card);
    const bodyBg = getComputedStyle(document.body).backgroundColor;
    return { radius: cs.borderRadius, bodyBg, cardBg: cs.backgroundImage.slice(0, 24) };
  });
  const cornerShot = await mpage.screenshot({ clip: { x: 0, y: 0, width: 10, height: 10 } });
  check('菜单卡片 24px 圆角', /24px/.test(menuLook.radius), menuLook.radius);
  check('菜单页 body 透明（窗口透明后四角真透）', /rgba\(0, 0, 0, 0\)|transparent/.test(menuLook.bodyBg), menuLook.bodyBg);

  console.log('== 8. 护眼提醒：气泡同款外观 + 不叠层 + 收放都硬重绘 ==');
  const rem = await page.evaluate(async () => {
    const b = document.getElementById('speechBubble');
    b.classList.remove('hidden');
    const before = window.__qibaiTest.calls.filter((c) => c.cmd === 'pet_repaint' && c.args && c.args.full === true).length;
    window.__qibaiTest.emit('qibai://reminder', { type: 'eye', message: '看屏幕太久了，起来活动一下，看看远处。' });
    await new Promise((r) => setTimeout(r, 120));
    const el = document.getElementById('reminderOverlay');
    const cs = getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    const fullCalls = window.__qibaiTest.calls.filter((c) => c.cmd === 'pet_repaint' && c.args && c.args.full === true).length - before;
    const out = {
      visible: cs.display !== 'none',
      radius: parseFloat(cs.borderTopLeftRadius),
      bg: cs.backgroundColor,
      borderLeft: parseFloat(cs.borderLeftWidth),
      transition: cs.transitionDuration,
      top: rect.top,
      width: rect.width,
      bubbleHidden: b.classList.contains('hidden'),
      fullCalls,
      msg: document.getElementById('reminderMsg').textContent,
      tag: document.getElementById('reminderTag').textContent,
    };
    document.getElementById('reminderClose').click();
    await new Promise((r) => setTimeout(r, 120));
    out.hiddenAfter = getComputedStyle(el).display === 'none';
    out.fullCallsAfterHide = window.__qibaiTest.calls.filter((c) => c.cmd === 'pet_repaint' && c.args && c.args.full === true).length - before;
    return out;
  });
  check('护眼提醒卡片已弹出', rem.visible, rem);
  check('提醒卡片文案来自事件（护眼提醒）', rem.tag === '护眼提醒' && rem.msg.indexOf('看屏幕太久') === 0, rem);
  check('提醒卡是圆角气泡（≥14px 圆角）', rem.radius >= 14, rem.radius);
  check('提醒卡为浅色磨砂底（不再是深紫粗边卡片）', /rgba?\(255,\s*255,\s*255/.test(rem.bg) && rem.borderLeft <= 1.5, rem);
  check('提醒卡无位移过渡（X11 残影根源）', rem.transition === '0s', rem.transition);
  check('提醒卡贴窗口顶部（与气泡同一位置）', rem.top <= 20, rem.top);
  check('提醒出现时气泡自动收起（不叠两层）', rem.bubbleHidden === true, rem.bubbleHidden);
  // v45.6：原生表面操作（pet_repaint full=true）已整条删除，改为 web 侧 canvas 重绘。
  // 这里断言不再触发任何原生整窗重绘调用。
  check('提醒弹出不再触发原生整窗重绘（v45.6 起）', rem.fullCalls === 0, rem.fullCalls);
  check('提醒收起后彻底隐藏（不是半透明残留）', rem.hiddenAfter === true, rem.hiddenAfter);
  check('提醒收起也不触发原生整窗重绘（v45.6 起）', rem.fullCallsAfterHide === 0, rem.fullCallsAfterHide);

  console.log('== 9. v42 四种真实窗口尺寸：输入区/快捷区/舞台矩形不相交 ==');
  for (const size of [{ width: 340, height: 260 }, { width: 340, height: 330 }, { width: 220, height: 310 }, { width: 420, height: 520 }]) {
    await page.setViewportSize(size);
    await page.waitForTimeout(220);
    const geo = await page.evaluate(() => {
      const r = (id) => {
        const el = document.getElementById(id); const b = el.getBoundingClientRect();
        return { left: b.left, right: b.right, top: b.top, bottom: b.bottom, width: b.width, height: b.height };
      };
      return { chips: r('quickChips'), chat: r('chatBar'), stage: r('stage'), W: innerWidth, H: innerHeight,
        chipOverflow: Array.from(document.querySelectorAll('#quickChips .chip')).some((el) => {
          const b = el.getBoundingClientRect(); return b.left < -1 || b.right > innerWidth + 1;
        }) };
    });
    const label = size.width + '×' + size.height;
    check(label + ' 快捷区不压输入栏', geo.chips.bottom <= geo.chat.top + 1, geo);
    // v43：默认极简态隐藏快捷胶囊（chips 高 0），此时舞台只需不压输入栏；
    // 完整界面仍要求舞台不进入快捷区。
    const stageLimit = geo.chips.height > 0 ? geo.chips.top : geo.chat.top;
    check(label + ' 舞台不进入快捷/输入区', geo.stage.bottom <= stageLimit + 1 && geo.stage.height > 20, geo);
    check(label + ' 控件完整在窗口内', geo.chat.bottom <= geo.H + 1 && geo.chips.left >= -1 && geo.chips.right <= geo.W + 1 && !geo.chipOverflow, geo);
  }
  await page.setViewportSize({ width: 360, height: 360 });
  await page.waitForTimeout(180);

  console.log('== 10. v42 pet-say 单句只演出一次 + 流式长文自动滚动 ==');
  const single = await page.evaluate(async () => {
    const textNode = document.getElementById('speechText');
    textNode.textContent = '';
    let writes = 0;
    const obs = new MutationObserver(() => { if (textNode.textContent === '只演出一次的回归句') writes += 1; });
    obs.observe(textNode, { childList: true, characterData: true, subtree: true });
    const ttsBefore = window.__qibaiTest.calls.filter((c) => c.cmd === 'tts_speak').length;
    window.__qibaiTest.emit('qibai://pet-say', { persona_id: 'gudian', text: '只演出一次的回归句', rust_played: true, ms: 180 });
    // v45 显示策略（用户定稿）：短句（≤24字）至少停 4s 再自动收，事件里的 ms 只当更久的下限。
    await new Promise((r) => setTimeout(r, 4600));
    obs.disconnect();
    return { writes, text: textNode.textContent,
      hidden: document.getElementById('speechBubble').classList.contains('hidden'),
      ttsDelta: window.__qibaiTest.calls.filter((c) => c.cmd === 'tts_speak').length - ttsBefore };
  });
  check('pet-say 同一句只写入气泡一次', single.writes === 1, single);
  check('pet-say rust_played 不重复触发前端朗读', single.ttsDelta === 0, single);
  check('pet-say 到时自动隐藏', single.hidden === true, single);

  const shortBubble = await page.evaluate(async () => {
    window.__qibaiDebug.say('短句首行必须完整', 1000);
    await new Promise((r) => setTimeout(r, 100));
    const b = document.getElementById('speechBubble');
    const t = document.getElementById('speechText');
    const br = b.getBoundingClientRect();
    const tr = t.getBoundingClientRect();
    const out = { bubbleScroll: b.scrollTop, textScroll: t.scrollTop, textTop: tr.top, bubbleTop: br.top, visible: !b.classList.contains('hidden') };
    window.__qibaiTest.emit('qibai://bubble-dismiss', {});
    return out;
  });
  check('短气泡不自动滚动且首行在外框内', shortBubble.visible && shortBubble.bubbleScroll === 0 && shortBubble.textScroll === 0 && shortBubble.textTop >= shortBubble.bubbleTop, shortBubble);

  const stream = await page.evaluate(async () => {
    const seg = '流式长文自动滚动。'.repeat(32);
    window.__qibaiTest.emit('qibai://tts-chunk', { seq: 1, text: seg, rust_played: true });
    await new Promise((r) => setTimeout(r, 120));
    const b = document.getElementById('speechBubble');
    const t = document.getElementById('speechText');
    const out = { overflow: t.scrollHeight > t.clientHeight, atEnd: t.scrollTop > 0 && t.scrollTop + t.clientHeight >= t.scrollHeight - 3,
      visible: !b.classList.contains('hidden'), length: t.textContent.length, scrollH: t.scrollHeight, clientH: t.clientHeight,
      maxH: getComputedStyle(t).maxHeight, display: getComputedStyle(t).display };
    window.__qibaiTest.emit('qibai://bubble-dismiss', {});
    return out;
  });
  check('rust_played 长文统一自动滚动到底', stream.overflow && stream.atEnd, stream);
  check('bubble-dismiss 走统一隐藏状态机', await page.evaluate(() => document.getElementById('speechBubble').classList.contains('hidden')));

  console.log('== 11. v42 连续提醒：旧计时器不得关闭新提醒 ==');
  await page.evaluate(() => window.__qibaiTest.emit('qibai://reminder', { type: 'eye', message: '第一条提醒' }));
  await page.waitForTimeout(8800);
  await page.evaluate(() => window.__qibaiTest.emit('qibai://reminder', { type: 'water', message: '第二条提醒必须继续显示' }));
  await page.waitForTimeout(450);
  const reminderFresh = await page.evaluate(() => ({
    visible: document.getElementById('reminderOverlay').classList.contains('show'),
    text: document.getElementById('reminderMsg').textContent,
  }));
  check('跨过第一条旧计时点后第二条仍显示', reminderFresh.visible && /第二条/.test(reminderFresh.text), reminderFresh);
  await page.click('#reminderClose');

  console.log('== 12. v42 X11 安全兼容：设置与菜单不制造假透明 ==');
  await page.evaluate(() => { localStorage.setItem('__qibaiRenderInfo', JSON.stringify({ transparent: false, mode: 'x11-safe-opaque' })); });
  await page.reload({ waitUntil: 'load' });
  await page.waitForTimeout(500);
  const compat = await page.evaluate(() => {
    const before = window.__qibaiTest.calls.filter((c) => c.cmd === 'save_settings').length;
    document.getElementById('ctxTransBg').click();
    const c = document.getElementById('bgCanvas');
    const px = c.getContext('2d').getImageData(2, 2, 1, 1).data;
    return { bodyClass: document.body.classList.contains('transparency-unsupported'),
      text: document.getElementById('ctxTransBgText').textContent, alpha: px[3],
      saves: window.__qibaiTest.calls.filter((c2) => c2.cmd === 'save_settings').length - before };
  });
  check('后端不支持时前端进入兼容态', compat.bodyClass && /兼容/.test(compat.text), compat);
  check('兼容态点击不开启透明且不改设置', compat.alpha === 255 && compat.saves === 0, compat);

  console.log('== 13. v43 六条缺陷回归 ==');
  const v43 = await page.evaluate(async () => {
    const out = {};
    out.quickHidden = document.getElementById('quickChips').getBoundingClientRect().height === 0;
    out.bubblePage = document.getElementById('speechBubble').classList.contains('minimal');
    out.aliasBounce = window.__qibaiDebug.realActionFrame('bounce');
    out.aliasTilt = window.__qibaiDebug.realActionFrame('tilt');
    out.personaMood = (window.__qibaiDebug.inferMood('哈哈哈太好啦', 'huopo') || {}).action || '';
    window.__qibaiTest.calls.length = 0;
    window.__qibaiDebug.say('这是一段足够长的回归文本，用来验证气泡文字可以滚动，并且右侧滚动条真实可见。'.repeat(3), 2000);
    // 打字机把文本填到超过三行时才出现滚动条，等它越过阈值并完成 150ms 淡入。
    await new Promise((r) => setTimeout(r, 1400));
    const b = document.getElementById('speechBubble');
    const t = document.getElementById('speechText');
    const bar = document.getElementById('speechScrollbar');
    const thumb = document.getElementById('speechScrollThumb');
    out.scrollable = b.classList.contains('scrollable');
    out.overflow = t.scrollHeight > t.clientHeight;
    out.textMax = getComputedStyle(t).maxHeight;
    out.barOpacity = getComputedStyle(bar).opacity;
    out.barMatch = !!document.querySelector('#speechBubble.scrollable #speechScrollbar');
    out.thumbH = thumb.getBoundingClientRect().height;
    out.barVisible = getComputedStyle(bar).opacity !== '0' && thumb.getBoundingClientRect().height > 8;
    out.stillVisibleBeforeRead = !b.classList.contains('hidden');
    b.dispatchEvent(new PointerEvent('pointerenter', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 3600));
    out.stillVisibleAfterHover = !b.classList.contains('hidden');
    b.dispatchEvent(new PointerEvent('pointerleave', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 2400));
    out.hiddenAfterLeave = b.classList.contains('hidden');
    await new Promise((r) => setTimeout(r, 3000));   // 再等 3s，确认不是"还没到点"
    out.hiddenAfterHideCheck = b.classList.contains('hidden');
    // v44：真实模拟鼠标按下角色 → 必须走 GTK 系统拖拽，且不得再叠加逐帧移动。
    const cw = document.querySelector('#characterWrap');
    const before = window.__qibaiTest.calls.length;
    cw.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1, pointerType: 'mouse', button: 0, clientX: 120, clientY: 200 }));
    await new Promise((r) => setTimeout(r, 80));
    const after = window.__qibaiTest.calls.slice(before);
    out.sysDragCalls = after.filter((c) => c.cmd === 'pet_start_drag').length;
    out.moveCalls = after.filter((c) => c.cmd === 'pet_move_by').length;
    out.keepTopCalls = window.__qibaiTest.calls.filter((c) => c.cmd === 'pet_keep_on_top').length;
    return out;
  });
  check('v43 极简态隐藏快捷胶囊（角色区放大）', v43.quickHidden, v43);
  check('v43 无帧动作有别名（bounce/tilt 不再是空动作）', v43.aliasBounce === 'celebrate' && v43.aliasTilt === 'touch', v43);
  check('v43 inferMood 接收当前 persona', v43.personaMood.length > 0, v43);
  check('v43 长气泡溢出且自绘滚动条可见', v43.overflow && v43.scrollable && v43.barVisible && parseFloat(v43.textMax) <= 52, v43);
  check('v43 悬停阅读期间气泡不被定时器收走', v43.stillVisibleBeforeRead && v43.stillVisibleAfterHover, v43);
  // v45：长句改常驻，离开气泡也不再自动收（只能下一句覆盖或手动收）
  check('v45 长句离开气泡后仍常驻（不再按剩余时间收）', v43.hiddenAfterHideCheck === false, v43);
  check('v44 鼠标拖拽走 GTK 系统拖拽且不叠加逐帧移动', v43.sysDragCalls === 1 && v43.moveCalls === 0, v43);
  check('v44 置顶状态启动即主动重申', v43.keepTopCalls > 0, v43);

  console.log('== 7. 页面运行期无 JS 崩溃 ==');
  check('无 pageerror', pageErrors.length === 0, pageErrors.slice(0, 3));

  await browser.close();

  console.log('\n结果: ' + pass + ' passed, ' + fail + ' failed');
  if (fail) { console.log('失败项:\n - ' + failures.join('\n - ')); process.exit(1); }
})().catch((e) => { console.error('测试异常:', e); process.exit(1); });
