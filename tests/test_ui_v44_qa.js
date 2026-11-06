/* ============================================================================
 * 元气小小 v44 独立 QA 终验测试（QA 线自建，不采信开发线自测结论）
 *
 * Node + 真实 Chromium，file:// 打开 src/pet.html，页面脚本前注入
 * mock window.__TAURI__/invoke/event.listen，用真实 PointerEvent 序列驱动。
 *
 * mock pet_start_drag 语义（__qaControl.startDragMode）：
 *   'moved'   → resolve {moved:true}（真拖过，应压制尾随 click）
 *   'idle'    → resolve {moved:false}（纯点按，应放行 click）
 *   'reject'  → reject（应降级 setPointerCapture + pet_move_by 逐帧）
 *   'hang'    → 永不 resolve（看门狗 3s 必须强制收尾）
 *   'delay'   → 150ms 后 resolve {moved:true}（拖管 pending 期 move 不得发出）
 *
 * 断言：
 *   S1 系统拖拽 moved=true：startDrag=1、move_by=0、尾随 click 被压制（无摸头气泡）
 *   S1b 系统拖拽 moved=false（纯点按）：尾随 click 正常触发摸头
 *   S1c delay 期（dragPending）move 不发出 move_by
 *   S2 reject 降级：setPointerCapture、move_by>0、sumDx≈100(±2px)
 *   S2b 看门狗：hang → 3.2s 内 settle（dragged=1）、状态复位可再拖、move_by=0
 *   S2c 快速连拖 5 次：startDrag 次数=5、move_by 全 0、无并发双路
 *   S3 置顶：启动/6s/focus/visibilitychange 四路
 *   S4 切角色：连续 persona 开新窗无错误、单角色节点
 *
 * 运行：node tests/test_ui_v44_qa.js
 * 退出码 0 = 全 PASS；1 = 有 FAIL。
 * ========================================================================== */
'use strict';

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const ROOT = path.resolve(__dirname, '..');
const PET_URL = 'file://' + path.join(ROOT, 'src/pet.html');
const SHOTS_DIR = path.join(ROOT, 'tests', 'shots');

const results = [];
function report(name, pass, detail) {
  results.push({ name, pass, detail: String(detail || '') });
  console.log((pass ? 'PASS' : 'FAIL') + ' | ' + name + (detail ? ' | ' + detail : ''));
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function makeInitScript(label, startDragMode) {
  return `(function () {
    window.__qaCalls = [];
    window.__qaErrors = [];
    window.__qaCaptures = 0;
    window.__qaControl = { startDragMode: ${JSON.stringify(startDragMode)} };
    window.__qibaiWindowLabel = ${JSON.stringify(label)};
    const listeners = {};
    window.__qaBus = {
      emit(event, payload) {
        (listeners[event] || []).slice().forEach((fn) => {
          try { fn({ payload }); } catch (e) { window.__qaErrors.push(String(e && e.message || e)); }
        });
      }
    };
    window.__TAURI__ = {
      invoke: async (cmd, args) => {
        window.__qaCalls.push({ cmd, args: args || null });
        if (cmd === 'pet_start_drag') {
          const m = window.__qaControl.startDragMode;
          if (m === 'reject') throw new Error('mock: WM begin_move_drag unavailable');
          if (m === 'hang') return new Promise(() => {});
          if (m === 'delay') await new Promise((r) => setTimeout(r, 150));
          return { moved: (m === 'idle') ? false : true, x: 0, y: 0 };
        }
        if (cmd === 'get_settings') return { ui: { transparent_bg: false, show_voice_tag: false } };
        if (cmd === 'pet_render_info') return { transparent: true };
        if (cmd === 'get_personas') return { personas: [] };
        return null;
      },
      event: {
        listen: async (event, handler) => {
          (listeners[event] = listeners[event] || []).push(handler);
          return () => {};
        }
      },
      window: { getCurrentWindow: () => ({ label: ${JSON.stringify(label)} }) }
    };
    const origSPC = Element.prototype.setPointerCapture;
    Element.prototype.setPointerCapture = function (pid) {
      window.__qaCaptures++;
      return origSPC.call(this, pid);
    };
    window.addEventListener('error', (e) => window.__qaErrors.push(String(e && e.message)));
  })();`;
}

async function newPage(browser, label, startDragMode) {
  const page = await browser.newPage({
    viewport: { width: 360, height: 380 },
    deviceScaleFactor: 1,
  });
  await page.addInitScript(makeInitScript(label, startDragMode));
  await page.goto(PET_URL, { waitUntil: 'load' });
  return page;
}

async function waitAssetReady(page, timeoutMs = 6000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    const ok = await page.evaluate(() => {
      const st = document.getElementById('stage');
      return st && st.classList.contains('asset-ready');
    });
    if (ok) return true;
    await sleep(120);
  }
  return false;
}
async function callsSince(page, sinceIdx) {
  return page.evaluate((since) => window.__qaCalls.slice(since), sinceIdx);
}
async function countSince(page, cmd, sinceIdx) {
  return page.evaluate(({ cmd, since }) =>
    window.__qaCalls.slice(since).filter((c) => c.cmd === cmd).length, { cmd, since: sinceIdx });
}
async function bubbleText(page) {
  return page.evaluate(() => {
    const t = document.getElementById('speechText');
    return t ? t.textContent.trim() : '';
  });
}
async function wrapBox(page) {
  const box = await page.locator('#characterWrap').boundingBox();
  return { cx: box.x + box.width / 2, cy: box.y + box.height / 2 };
}
async function draggedFlag(page) {
  return page.evaluate(() => document.getElementById('characterWrap').dataset.dragged);
}

/* ------------------------------------------------- S1 系统拖拽 moved=true */
async function s1SystemDragMoved(browser) {
  console.log('\n== S1 系统拖拽（{moved:true}）==');
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    if (!await waitAssetReady(page)) { report('S1.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    const base = await page.evaluate(() => window.__qaCalls.length);

    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await sleep(250); // 等 promise resolve → settle(true)

    const nStart = await countSince(page, 'pet_start_drag', base);
    const nMove = await countSince(page, 'pet_move_by', base);
    report('S1.1 pet_start_drag 恰好 1 次', nStart === 1, '实际=' + nStart);
    report('S1.2 settle 后 pet_move_by 为 0', nMove === 0, '实际=' + nMove);
    const dragged = await draggedFlag(page);
    report('S1.3 settle(true) 立即置位 dragged=1', dragged === '1',
      '实际=' + JSON.stringify(dragged));

    await page.mouse.up();
    await sleep(150);
    const before = await bubbleText(page);
    await page.mouse.click(cx + 12, cy + 12);
    await sleep(400);
    const after = await bubbleText(page);
    report('S1.4 真拖过后尾随 click 被压制（不出现摸头气泡）',
      before === '' && after === '',
      'after=' + JSON.stringify(after.slice(0, 24)));
  } finally { await page.close(); }
}

/* ------------------------------------------------- S1b 纯点按 {moved:false} */
async function s1bSystemDragIdle(browser) {
  console.log('\n== S1b 纯点按（{moved:false}）==');
  const page = await newPage(browser, 'pet-huopo', 'idle');
  try {
    if (!await waitAssetReady(page)) { report('S1b.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await sleep(250);
    await page.mouse.up();
    await sleep(150);
    const dragged = await draggedFlag(page);
    report('S1b.1 纯点按不置位 dragged（放行 click）', dragged === undefined || dragged === '',
      '实际=' + JSON.stringify(dragged));
    await page.mouse.click(cx, cy);
    await sleep(450);
    const text = await bubbleText(page);
    report('S1b.2 纯点按 click 正常触发摸头气泡', text !== '',
      '气泡=' + JSON.stringify(text.slice(0, 24)));
  } finally { await page.close(); }
}

/* ------------------------------------------------- S1c pending 期 move 不发出 */
async function s1cPendingGate(browser) {
  console.log('\n== S1c dragPending 期 move 双门控 ==');
  const page = await newPage(browser, 'pet-huopo', 'delay');
  try {
    if (!await waitAssetReady(page)) { report('S1c.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    const base = await page.evaluate(() => window.__qaCalls.length);
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    // 150ms 内 promise 未 resolve → dragPending=true，期间 4 次 move
    for (let k = 1; k <= 4; k++) { await page.mouse.move(cx + 20 * k, cy + 20 * k); await sleep(20); }
    await sleep(250); // 等 resolve → settle
    await page.mouse.up();
    await sleep(150);
    const nMove = await countSince(page, 'pet_move_by', base);
    report('S1c.1 pending+sysDrag 期间全部 move 不发出 pet_move_by', nMove === 0,
      '实际=' + nMove + '（双门控失效则 >0）');
  } finally { await page.close(); }
}

/* ------------------------------------------------- S2 reject 降级 */
async function s2Fallback(browser) {
  console.log('\n== S2 降级路径（reject）==');
  const page = await newPage(browser, 'pet-huopo', 'reject');
  try {
    if (!await waitAssetReady(page)) { report('S2.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    const base = await page.evaluate(() => window.__qaCalls.length);
    const capBase = await page.evaluate(() => window.__qaCaptures);
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await sleep(250);
    const capDelta = await page.evaluate((b) => window.__qaCaptures - b, capBase);
    report('S2.1 reject 后 setPointerCapture 被调用', capDelta >= 1, '次数=' + capDelta);
    for (let k = 1; k <= 5; k++) { await page.mouse.move(cx + 20 * k, cy); await sleep(40); }
    await sleep(250);
    await page.mouse.up();
    await sleep(250);
    const calls = await callsSince(page, base);
    const moves = calls.filter((c) => c.cmd === 'pet_move_by');
    const sumDx = moves.reduce((s, c) => s + ((c.args && c.args.dx) || 0), 0);
    report('S2.2 降级拖拽 pet_move_by 被调用（窗口真跟手）', moves.length > 0, '次数=' + moves.length);
    report('S2.3 累计 dx≈+100px（±2px）', Math.abs(sumDx - 100) <= 2, 'sumDx=' + sumDx);
    report('S2.4 start_drag 恰 1 次', calls.filter((c) => c.cmd === 'pet_start_drag').length === 1, '');
  } finally { await page.close(); }
}

/* ------------------------------------------------- S2b 看门狗 */
async function s2bWatchdog(browser) {
  console.log('\n== S2b 看门狗（hang 永不 resolve）==');
  const page = await newPage(browser, 'pet-huopo', 'hang');
  try {
    if (!await waitAssetReady(page)) { report('S2b.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    const base = await page.evaluate(() => window.__qaCalls.length);
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await sleep(1200); // 1.2s：promise 仍悬挂，不应 settle
    const draggedMid = await draggedFlag(page);
    const nMoveMid = await countSince(page, 'pet_move_by', base);
    report('S2b.1 1.2s 时未 settle（看门狗未提前触发）且 move_by=0',
      (draggedMid === undefined || draggedMid === '') && nMoveMid === 0,
      'dragged=' + JSON.stringify(draggedMid) + ' move_by=' + nMoveMid);
    await sleep(2200); // 累计 3.4s > 3s 看门狗
    const draggedLate = await draggedFlag(page);
    report('S2b.2 3.4s 内看门狗强制 settle（dragged=1）', draggedLate === '1',
      '实际=' + JSON.stringify(draggedLate));
    // 状态复位：再 down 一次应重新发起 start_drag（不再卡死）
    await page.mouse.up();
    await page.mouse.down();
    await sleep(150);
    const nStart2 = await countSince(page, 'pet_start_drag', base);
    report('S2b.3 看门狗后状态复位、可重新发起拖拽', nStart2 === 2,
      '累计 start_drag=' + nStart2 + '（期望 2：hang 那次 + 重试那次）');
    await page.mouse.up();
  } finally { await page.close(); }
}

/* ------------------------------------------------- S2c 快速连拖 5 次 */
async function s2cRapidDrags(browser) {
  console.log('\n== S2c 快速连拖 5 次 ==');
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    if (!await waitAssetReady(page)) { report('S2c.0 立绘就绪', false, '超时'); return; }
    const { cx, cy } = await wrapBox(page);
    const base = await page.evaluate(() => window.__qaCalls.length);
    for (let i = 0; i < 5; i++) {
      await page.mouse.move(cx + 5 * i, cy + 5 * i);
      await page.mouse.down();
      await sleep(90);
      await page.mouse.up();
      await sleep(30);
    }
    await sleep(300);
    const calls = await callsSince(page, base);
    const nStart = calls.filter((c) => c.cmd === 'pet_start_drag').length;
    const nMove = calls.filter((c) => c.cmd === 'pet_move_by').length;
    report('S2c.1 5 次 down → start_drag 恰 5 次（无并发漏发/重发）', nStart === 5,
      '实际=' + nStart);
    report('S2c.2 全程 pet_move_by=0（无双路位移）', nMove === 0, '实际=' + nMove);
    const errs = await page.evaluate(() => window.__qaErrors.length);
    report('S2c.3 无页面错误', errs === 0, 'errors=' + errs);
  } finally { await page.close(); }
}

/* ------------------------------------------------- S3 置顶 */
async function s3KeepOnTop(browser) {
  console.log('\n== S3 置顶重申 ==');
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    if (!await waitAssetReady(page)) { report('S3.0 立绘就绪', false, '超时'); return; }
    const nBoot = await countSince(page, 'pet_keep_on_top', 0);
    report('S3.1 启动即调用 pet_keep_on_top', nBoot >= 1, '实际=' + nBoot);
    await sleep(6400);
    const nInt = await countSince(page, 'pet_keep_on_top', 0);
    report('S3.2 6s 轮询重申', nInt === nBoot + 1, 'boot=' + nBoot + ' now=' + nInt);
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await sleep(120);
    const nFocus = await countSince(page, 'pet_keep_on_top', 0);
    report('S3.3 window focus 触发重申', nFocus === nInt + 1, '');
    await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
    await sleep(120);
    const nVis = await countSince(page, 'pet_keep_on_top', 0);
    report('S3.4 visibilitychange 触发重申', nVis === nFocus + 1, '');
  } finally { await page.close(); }
}

/* ------------------------------------------------- S4 切角色 */
async function s4PersonaSwitch(browser) {
  console.log('\n== S4 切角色前端表现 ==');
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    if (!await waitAssetReady(page)) { report('S4.0 立绘就绪', false, '超时'); return; }
    const dom1 = await page.evaluate(() => ({
      imgs: document.querySelectorAll('#characterWrap img').length,
      phs: document.querySelectorAll('#characterWrap .placeholder-face').length,
      phHidden: getComputedStyle(document.getElementById('placeholderFace')).display,
    }));
    report('S4.1 稳态单角色节点（img=1/占位=1/占位隐藏）',
      dom1.imgs === 1 && dom1.phs === 1 && dom1.phHidden === 'none', JSON.stringify(dom1));
    for (let i = 0; i < 12; i++) {
      await page.evaluate((i) => {
        const evs = ['qibai://pet-perform', 'qibai://pet-say', 'qibai://llm-status'];
        window.__qaBus.emit(evs[i % evs.length], i % 2 ? { text: 's' + i } : { state: 'thinking' });
      }, i);
    }
    await sleep(1000);
    const dom2 = await page.evaluate(() => ({
      imgs: document.querySelectorAll('#characterWrap img').length,
      errs: window.__qaErrors.length,
    }));
    report('S4.2 并发事件后节点仍唯一、无错误', dom2.imgs === 1 && dom2.errs === 0, JSON.stringify(dom2));
    let ok = true, detail = [];
    for (const lb of ['pet-dushi', 'pet-gudian', 'pet-reqing']) {
      const p = await newPage(browser, lb, 'moved');
      try {
        const ready = await waitAssetReady(p, 5000);
        const c = await p.evaluate(() => ({
          imgs: document.querySelectorAll('#characterWrap img').length,
          errs: window.__qaErrors.length,
        }));
        detail.push(lb + ':ready=' + ready + ' imgs=' + c.imgs + ' errs=' + c.errs);
        if (!ready || c.imgs !== 1 || c.errs !== 0) ok = false;
      } finally { await p.close(); }
    }
    report('S4.3 连续换 3 persona 新窗均就绪、单角色、无错误', ok, detail.join(' ; '));
  } finally { await page.close(); }
}

/* ------------------------------------------------- 截图 */
/* v45 显示节奏：短句 ≤24 字 4s 自动收；长句常驻不自动收（用户 v45 定稿策略） */
async function s1dBubbleDwell(browser) {
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    const now = Date.now();
    await page.evaluate(() => window.__qibaiDebug.say('好的，收到。'));
    await sleep(3200);
    const shortStillOn = await page.evaluate(() =>
      !document.getElementById('speechBubble').classList.contains('hidden'));
    report('S1d.1 短句 3.2s 时仍在', shortStillOn === true, 'stillOn=' + shortStillOn);

    await sleep(1800); // 累计 5s，超过 4s 自动收起
    const shortHidden = await page.evaluate(() =>
      document.getElementById('speechBubble').classList.contains('hidden'));
    report('S1d.2 短句 5s 后已自动收起', shortHidden === true, 'hidden=' + shortHidden);

    await page.evaluate(() => window.__qibaiDebug.say(
      '这一句比较长，用来验证长句气泡不会自己消失：它应该一直留在屏幕上，直到我换下一句或者手动收掉。'));
    await sleep(6000);
    const longVisible = await page.evaluate(() => {
      const b = document.getElementById('speechBubble');
      return !b.classList.contains('hidden') && (document.getElementById('speechText') || {}).textContent.length > 0;
    });
    report('S1d.3 长句 6s 后仍常驻', longVisible === true, 'visible=' + longVisible);

    // 常驻后换下一句必须能覆盖，不能叠两层字
    await page.evaluate(() => window.__qibaiDebug.say('换一句短的。'));
    const oneBubble = await page.evaluate(() => document.querySelectorAll('#speechBubble').length);
    report('S1d.4 覆盖后气泡单实例', oneBubble === 1, 'count=' + oneBubble);
  } finally { await page.close(); }
}

async function screenshots(browser) {
  console.log('\n== 截图自验 ==');
  if (!fs.existsSync(SHOTS_DIR)) fs.mkdirSync(SHOTS_DIR, { recursive: true });
  const page = await newPage(browser, 'pet-huopo', 'moved');
  try {
    await waitAssetReady(page, 6000);
    await sleep(400);
    const p1 = path.join(SHOTS_DIR, 'v44_final_1_main_360x380.png');
    await page.screenshot({ path: p1 });
    report('SHOT.1 主窗', fs.existsSync(p1), p1);

    await page.evaluate(() => window.__qaBus.emit('qibai://ui-minimal', { on: true }));
    await sleep(500);
    const p2 = path.join(SHOTS_DIR, 'v44_final_2_minimal.png');
    await page.screenshot({ path: p2 });
    report('SHOT.2 极简态', fs.existsSync(p2), p2);

    await page.evaluate(() => window.__qaBus.emit('qibai://ui-minimal', { on: false }));
    await sleep(400);
    await page.evaluate(() => window.__qibaiDebug.say(
      '长气泡回归验证：在 360px 宽窗内换行完整、底部不截断、文字不溢出、立绘不被挤压裁脚。', 15000));
    await sleep(600);
    const p3 = path.join(SHOTS_DIR, 'v44_final_3_long_bubble.png');
    await page.screenshot({ path: p3 });
    report('SHOT.3 长气泡', fs.existsSync(p3), p3);
  } finally { await page.close(); }
}

/* ------------------------------------------------- main */
(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--allow-file-access-from-files'],
  });
  try {
    await s1SystemDragMoved(browser);
    await s1bSystemDragIdle(browser);
    await s1cPendingGate(browser);
    await s2Fallback(browser);
    await s2bWatchdog(browser);
    await s2cRapidDrags(browser);
    await s3KeepOnTop(browser);
    await s4PersonaSwitch(browser);
    await s1dBubbleDwell(browser);
    await screenshots(browser);
  } finally { await browser.close(); }

  const pass = results.filter((r) => r.pass).length;
  const fail = results.length - pass;
  console.log('\n===== 汇总: ' + pass + ' PASS / ' + fail + ' FAIL =====');
  if (fail > 0) {
    results.filter((r) => !r.pass).forEach((r) => console.log('  FAIL: ' + r.name + ' | ' + r.detail));
  }
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error('QA runner 异常:', e); process.exit(2); });
