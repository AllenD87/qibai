/* v42 交付前视觉验证：真实 Chromium 渲染 + 多窗口尺寸几何检查
 * 1 不透明态  2 开启透明  3 右键菜单  4 长文气泡  5 护眼提醒
 * 6-9 真机启动尺寸/标准/紧凑/大窗适配
 * 运行：node tests/capture_screens.js
 */
const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const ROOT = path.resolve(__dirname, '..');
const OUT = path.join(ROOT, 'tests/shots');
fs.mkdirSync(OUT, { recursive: true });

function backendInit() {
  const callbacks = new Map();
  let nextCb = 1, nextEv = 1;
  const listeners = {};
  const emitCalls = [];
  const db = { settings: { ui: { transparent_bg: false } } };
  function emit(event, payload) {
    emitCalls.push(event);
    (listeners[event] || []).forEach((id) => { const fn = callbacks.get(id); if (fn) fn({ event, id: 1, payload }); });
    return (listeners[event] || []).length;
  }
  const internals = {
    callbacks,
    transformCallback: (fn) => { const id = nextCb++; callbacks.set(id, fn); return id; },
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, p) => { const fn = callbacks.get(id); if (fn) fn(p); },
    invoke: async (cmd, args) => {
      if (cmd === 'plugin:event|listen') {
        if (args.event === undefined || args.target === undefined || args.handler === undefined) throw new Error('missing key');
        listeners[args.event] = listeners[args.event] || [];
        listeners[args.event].push(args.handler);
        return nextEv++;
      }
      if (cmd === 'plugin:event|unlisten') { return null; }
      if (cmd === 'debug_log') return null;
      if (cmd === 'get_settings') return JSON.parse(JSON.stringify(db.settings));
      if (cmd === 'pet_render_info') return {};
      if (cmd === 'save_settings') { db.settings = JSON.parse(JSON.stringify(args.settings || {})); return null; }
      if (cmd === 'ctx_menu_pick') { emit('qibai://ctx-action', { action: args.action }); return null; }
      return {};
    },
  };
  window.__TAURI_INTERNALS__ = internals;
  window.__qibaiTest = { listeners, db, emit, emitCalls, invoke: (c, a) => internals.invoke(c, a) };
}
const INIT = '(' + backendInit.toString() + ')();';

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--allow-file-access-from-files', '--no-sandbox', '--disable-dev-shm-usage', '--force-color-profile=srgb'],
  });
  const ctx = await browser.newContext({ viewport: { width: 360, height: 380 }, deviceScaleFactor: 2 });

  const page = await ctx.newPage();
  await page.addInitScript(INIT);
  await page.goto('file://' + path.join(ROOT, 'src/pet.html'), { waitUntil: 'load' });
  await page.waitForTimeout(1500);
  // 棋盘格底，便于肉眼看出透明
  await page.addStyleTag({ content: 'html{background:repeating-conic-gradient(#cfd8dc 0 25%,#eceff1 0 50%) 0 0/16px 16px !important;} body{background:transparent !important;}' });
  await page.screenshot({ path: path.join(OUT, '1_不透明态.png') });

  await page.evaluate(() => window.__qibaiTest.invoke('ctx_menu_pick', { action: 'transbg' }));
  await page.waitForTimeout(220);
  await page.screenshot({ path: path.join(OUT, '2_透明开启含提示条.png') });

  // 长文本气泡：验证加宽 + 内部滚动不裁字
  await page.evaluate(() => {
    const b = document.getElementById('speechBubble');
    const t = document.getElementById('speechText');
    b.classList.remove('hidden');
    t.textContent = '今晚早点睡好不好？我把你这周的日程都看了一遍：周三有个评审，周五要交周报。' +
      '我已经把提醒都排好了，到点我会叫你。要是加班太晚，记得先喝口水再继续，别硬扛。';
    t.scrollTop = 0; // 首行完整 + 右侧滚动条可见
    t.dispatchEvent(new Event('scroll'));
  });
  await page.waitForTimeout(120);
  await page.screenshot({ path: path.join(OUT, '4_长文本气泡.png') });
  await page.evaluate(() => { const t = document.getElementById('speechText'); t.scrollTop = t.scrollHeight; });
  await page.waitForTimeout(60);
  await page.screenshot({ path: path.join(OUT, '4b_长文本气泡已滚到底.png') });

  // 护眼提醒：与气泡同款外观（白磨砂 + 圆角 + 粉细边），不再深紫粗边方块
  await page.evaluate(() => {
    document.getElementById('speechBubble').classList.add('hidden');
    document.querySelectorAll('.toast').forEach((t) => t.remove());
    window.__qibaiTest.emit('qibai://reminder', { type: 'eye', message: '看屏幕太久啦，抬头看看远处，让眼睛歇一会儿～' });
  });
  await page.waitForTimeout(150);
  await page.screenshot({ path: path.join(OUT, '5_护眼提醒气泡.png') });

  const mpage = await ctx.newPage();
  await mpage.addInitScript(INIT);
  await mpage.goto('file://' + path.join(ROOT, 'src/menu.html'), { waitUntil: 'load' });
  await mpage.waitForTimeout(300);
  await mpage.setViewportSize({ width: 180, height: 290 });
  await mpage.screenshot({ path: path.join(OUT, '3_右键菜单.png'), omitBackground: true });

  // v42：覆盖真机启动尺寸、标准、compact 与大窗，截图前做矩形相交硬断言。
  await page.evaluate(() => {
    window.__qibaiTest.emit('qibai://bubble-dismiss', {});
    document.getElementById('reminderClose').click();
    document.querySelectorAll('.toast').forEach((t) => t.remove());
  });
  const sizes = [
    { width: 360, height: 380, file: '6_真机启动360x380.png' },
    { width: 340, height: 260, file: '7_矮窗340x260.png' },
    { width: 220, height: 310, file: '8_紧凑220x310.png' },
    { width: 420, height: 520, file: '9_大窗420x520.png' },
  ];
  for (const size of sizes) {
    await page.setViewportSize({ width: size.width, height: size.height });
    await page.waitForTimeout(260);
    const geo = await page.evaluate(() => {
      const r = (id) => document.getElementById(id).getBoundingClientRect();
      const chips = r('quickChips'), chat = r('chatBar'), stage = r('stage');
      return { chipsBottom: chips.bottom, chipsTop: chips.top, chatTop: chat.top,
        stageBottom: stage.bottom, stageHeight: stage.height };
    });
    const stageLimit = geo.chipsTop > 1 ? geo.chipsTop : geo.chatTop;
    if (geo.chipsBottom > geo.chatTop + 1 || geo.stageBottom > stageLimit + 1 || geo.stageHeight <= 20) {
      throw new Error('布局几何失败 ' + size.width + 'x' + size.height + ' ' + JSON.stringify(geo));
    }
    await page.screenshot({ path: path.join(OUT, size.file) });
  }

  await browser.close();
  console.log('截图完成：', fs.readdirSync(OUT).join(', '));
})().catch((e) => { console.error(e); process.exit(1); });
