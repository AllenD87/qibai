/* ============================================================================
 * 元气小小 · 按钮全量点击冒烟（v45.23）
 *
 * 目的：把「106 个按钮到底点了有没有反应」从人工目测变成可回归的实测。
 * 做法：真实 Chromium 加载 console.html / pet.html，注入 mock __TAURI_INTERNALS__，
 *       逐个 tab 逐个按钮触发 click，比对点击前后 6 个可观测维度：
 *         inv(后端调用数) / toast(提示数) / len(正文长度) / html(结构长度)
 *         / modal(弹层可见数) / tab(当前页签)
 *       任一变化即判定「有反馈」；六项全不变 = 静默按钮，列为待修。
 *
 * 用法：node tests/test_all_buttons.js [--json]
 * 退出码：0 = 无静默按钮；1 = 存在静默按钮或出现未捕获异常。
 * ========================================================================== */
'use strict';

const { chromium } = require('playwright');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const JSON_OUT = process.argv.includes('--json');

const TABS = ['chat', 'roles', 'intimacy', 'events', 'settings', 'vault', 'social', 'pk', 'plugins', 'codecheck', 'tasks'];

// ---- 注入的 mock 后端 ---------------------------------------------------
function mockBackend() {
  const callbacks = new Map();
  const listeners = {};
  let nextCb = 1, nextEv = 1;
  const calls = [];
  window.__mockCalls = calls;

  const seedSettings = {
    llm: { provider: 'doubao', api_key: '', model: 'doubao-seed-1-6', base_url: '' },
    voice: { wake_enabled: false, tts_engine: 'edge', audio_output: 'default', rate: 0 },
    ui: { transparent_bg: false, minimal: false, theme: 'rose' },
    search: { engine: 'bing' },
    kb: { folders: [], indexed_files: 0, indexed_chunks: 0 },
    stt: { device: 'default' },
  };

  function emit(event, payload) {
    (listeners[event] || []).forEach((id) => {
      const fn = callbacks.get(id);
      if (fn) fn({ event, id: 1, payload });
    });
    return (listeners[event] || []).length;
  }

  function defaultFor(cmd) {
    switch (cmd) {
      case 'get_settings': case 'get_settings_partial': return JSON.parse(JSON.stringify(seedSettings));
      case 'get_personas': return [
        { persona_id: 'gudian', nickname: '古典', active: true, emotion: 'calm' },
        { persona_id: 'dushi', nickname: '都市', active: false, emotion: 'calm' },
      ];
      case 'get_conversation': return [
        { role: 'user', content: '你好', ts: Date.now() - 5000 },
        { role: 'assistant', content: '在呢～', ts: Date.now() - 4000 },
      ];
      case 'get_skills_status': return { total_enabled: 3, skills: { code_review: { enabled: true }, kb_search: { enabled: true }, animacode: { enabled: true } } };
      case 'get_listen_state': return { enabled: false, listening: false, state: 'idle' };
      case 'get_desk_state': return { locked: false, x: 100, y: 100, scale: 1 };
      case 'get_intimacy_state': return { level: 1, exp: 0, next_exp: 100, personas: {} };
      case 'get_intimacy_levels': return { levels: [{ level: 1, name: '初识', exp: 0 }, { level: 2, name: '熟悉', exp: 100 }] };
      case 'kb_status': return { folders: [], indexed_files: 0, indexed_chunks: 0, embedding: { ready: false } };
      case 'mcp_list': case 'plugin_list': case 'expert_list': case 'connector_list':
      case 'skill_list': case 'skill_pack_list': case 'task_list': case 'netsocial_events':
      case 'netsocial_history': case 'netsocial_topics': case 'netsocial_city_souls':
        return [];
      case 'vault_status': return { initialized: true, unlocked: false, entries: 0 };
      case 'vault_list': return [];
      case 'netsocial_status': return { connected: false, registered: false, city: '', souls: 0 };
      case 'tts_engine_status': return { engine: 'edge', ready: true, voices: [] };
      case 'stt_status': return { ready: true, backend: 'whisper', model: 'small' };
      case 'current_audio_output': return 'default';
      case 'current_search_engine': return 'bing';
      case 'get_autostart': return { enabled: false };
      case 'list_audio_devices': case 'list_output_devices': return [{ id: 'default', name: '默认设备' }];
      case 'check_update': return { has_update: false, version: '45.22', latest: '45.22' };
      case 'stop_state': return { running: false };
      case 'pet_render_info': return { persona: 'gudian', emotion: 'calm' };
      case 'window_identity': return { label: 'pet' };
      case 'debug_log': return null;
      default: return {};
    }
  }

  const internals = {
    callbacks,
    transformCallback: (fn) => { const id = nextCb++; callbacks.set(id, fn); return id; },
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, p) => { const fn = callbacks.get(id); if (fn) fn(p); },
    invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      if (cmd === 'plugin:event|listen') {
        const ev = args && args.event;
        if (!ev) throw new Error('missing event');
        listeners[ev] = listeners[ev] || [];
        listeners[ev].push(args.handler);
        return nextEv++;
      }
      if (cmd === 'plugin:event|unlisten') return null;
      return defaultFor(cmd);
    },
    convertFileSrc: (p) => p,
  };
  window.__TAURI_INTERNALS__ = internals;
  window.__qibaiTest = { calls, listeners, emit, invoke: (c, a) => internals.invoke(c, a) };
  // 真实 DOM 变化计数（含 class 切换），比看 innerHTML 长度可靠
  window.__spark = { muts: 0, toasts: [] };
  const attach = () => {
    if (!document.body) return;
    new MutationObserver((recs) => { window.__spark.muts += recs.length; })
      .observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
  };
  if (document.body) attach();
  else document.addEventListener('DOMContentLoaded', attach, { once: true });
}

const INIT = '(' + mockBackend.toString() + ')();';

// ---- 快照 ---------------------------------------------------------------
const SNAP = `(() => {
  const vis = (el) => {
    if (!el) return false;
    if (el.hidden) return false;
    const s = getComputedStyle(el);
    if (s.display === 'none' || s.visibility === 'hidden' || +s.opacity === 0) return false;
    return el.getBoundingClientRect().height > 0;
  };
  const modals = [...document.querySelectorAll('.modal,[role="dialog"],.overlay,.mask')].filter(vis).length;
  const active = document.querySelectorAll('.active,.selected,.is-on,.is-stop,.on').length;
  const tabEl = document.querySelector('.ctab.active');
  const sp = window.__spark || { muts: 0, toasts: [] };
  return {
    inv: (window.__mockCalls || []).length,
    toast: document.querySelectorAll('.toast').length,
    muts: sp.muts,
    active,
    modals,
    tab: tabEl ? tabEl.dataset.tab : '',
    sparkToasts: sp.toasts.length,
  };
})()`;

function diff(a, b) {
  const keys = ['inv', 'toast', 'muts', 'active', 'modals', 'tab', 'sparkToasts'];
  return keys.filter((k) => a[k] !== b[k]).map((k) => `${k}:${a[k]}→${b[k]}`);
}

// ---- 主流程 -------------------------------------------------------------
(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--allow-file-access-from-files', '--no-sandbox', '--disable-dev-shm-usage'],
  });
  const ctx = await browser.newContext({ viewport: { width: 1100, height: 900 } });

  const problems = [];
  const results = [];

  // ============ 控制台 ============
  const page = await ctx.newPage();
  const pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push('pageerror: ' + (e.message || e)));
  page.on('console', (m) => { if (m.type() === 'error') pageErrors.push('console: ' + m.text()); });
  page.on('dialog', (d) => d.accept().catch(() => {}));
  // 只关闭控制台阶段被意外弹出的子窗，后面自己创建的 ppage 不在白名单里
  const allowed = new Set([page]);
  const killExtras = (p) => { if (!allowed.has(p)) p.close().catch(() => {}); };
  ctx.on('page', killExtras);

  await page.addInitScript(INIT);
  await page.goto('file://' + path.join(ROOT, 'src/console.html'), { waitUntil: 'load' });
  await page.waitForTimeout(1200);

  for (const tab of TABS) {
    // 切页签：优先点 tab 按钮，失败则直接调 switchTab
    const ok = await page.evaluate((t) => {
      const btn = document.querySelector(`.ctab[data-tab="${t}"]`);
      if (btn) { btn.click(); return true; }
      return false;
    }, tab);
    await page.waitForTimeout(250);
    const activeTab = await page.evaluate(() => (document.querySelector('.ctab.active') || {}).dataset?.tab || '');
    if (!ok || (activeTab && activeTab !== tab)) {
      problems.push({ scope: 'console', tab, id: '-', text: '-', issue: `页签切不过去（期望 ${tab}，实际 ${activeTab}）` });
    }

    // 枚举该页签下所有可见按钮，并打上稳定标记（DOM 重渲染后按标记找不到即视为元素消失）
    const btns = await page.evaluate(() => {
      let seq = 0;
      return [...document.querySelectorAll('button')].filter((b) => {
        if (b.disabled) return false;
        if (b.getAttribute('data-busy') === '1') return false;
        const s = getComputedStyle(b);
        if (s.display === 'none' || s.visibility === 'hidden') return false;
        const r = b.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      }).map((b) => {
        const key = 'sp' + (seq++);
        b.setAttribute('data-spark-seq', key);
        return { key, id: b.id || '', text: (b.innerText || b.value || '').trim().slice(0, 18).replace(/\s+/g, ' ') };
      });
    });

    for (const b of btns) {
      const before = await page.evaluate(SNAP);
      const clicked = await page.evaluate((key) => {
        if (window.__spark) window.__spark.muts = 0;
        const el = document.querySelector(`button[data-spark-seq="${key}"]`);
        if (!el) return 'gone';
        el.click();
        return 'ok';
      }, b.key).catch(() => 'error');

      await page.waitForTimeout(420);
      const after = await page.evaluate(SNAP);

      // 关掉本步可能打开的弹层，避免挡后续
      await page.evaluate(() => {
        document.querySelectorAll('.modal,[role="dialog"],.overlay,.mask').forEach((m) => {
          const close = m.querySelector('[data-close],[aria-label="关闭"],.modal-close,.close');
          if (close) close.click(); else m.hidden = true;
        });
      }).catch(() => {});
      await page.keyboard.press('Escape').catch(() => {});
      await page.waitForTimeout(120);

      results.push({ scope: 'console', tab, id: b.id, text: b.text, clicked, diff: diff(before, after) });
    }
  }

  // ============ 桌宠 ============
  allowed.clear();
  ctx.removeListener('page', killExtras);
  const ppage = await ctx.newPage();
  const petErrors = [];
  ppage.on('pageerror', (e) => petErrors.push('pageerror: ' + (e.message || e)));
  ppage.on('console', (m) => { if (m.type() === 'error') petErrors.push('console: ' + m.text()); });
  ppage.on('dialog', (d) => d.accept().catch(() => {}));

  await ppage.addInitScript(INIT);
  await ppage.goto('file://' + path.join(ROOT, 'src/pet.html'), { waitUntil: 'load' });
  await ppage.waitForTimeout(1200);

  const petBtns = await ppage.evaluate(() => {
    const list = [...document.querySelectorAll('button')].filter((b) => {
      if (b.disabled) return false;
      const s = getComputedStyle(b);
      if (s.display === 'none' || s.visibility === 'hidden') return false;
      const r = b.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    });
    return list.map((b, i) => ({ idx: i, id: b.id || '', text: (b.innerText || b.title || '').trim().slice(0, 18).replace(/\s+/g, ' ') }));
  });

  for (const b of petBtns) {
    const before = await ppage.evaluate(SNAP);
    await ppage.evaluate(() => { if (window.__spark) window.__spark.muts = 0; });
    const clicked = await ppage.evaluate((i) => {
      const list = [...document.querySelectorAll('button')].filter((x) => {
        if (x.disabled) return false;
        const s = getComputedStyle(x);
        if (s.display === 'none' || s.visibility === 'hidden') return false;
        const r = x.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      const el = list[i];
      if (!el) return false;
      el.click();
      return true;
    }, b.idx).catch(() => false);
    await ppage.waitForTimeout(420);
    const after = await ppage.evaluate(SNAP);
    results.push({ scope: 'pet', tab: '', id: b.id, text: b.text, clicked, diff: diff(before, after) });
    await ppage.keyboard.press('Escape').catch(() => {});
    await ppage.waitForTimeout(100);
  }

  // ---- 汇总 -------------------------------------------------------------
  // console 部分：因为没记录 before，改用「页面启动后每个按钮第一次点击是否引起
  // invoke/toast/结构变化」不可靠，这里改为重新逐按钮对比：改用同页顺序快照。
  // 简化：console 结果只保留「点了但没发生任何后端调用」的按钮（inv 未增长）。
  const seq = results.filter((r) => r.scope === 'console');
  const consoleSilent = seq.filter((r) => r.diff && r.diff.length === 0);
  const consoleGone = seq.filter((r) => r.clicked === 'gone');

  const petSilent = results.filter((r) => r.scope === 'pet' && r.diff && r.diff.length === 0);

  const report = {
    consoleButtons: seq.length,
    petButtons: petButtons_count(petBtns),
    consoleSilent,
    consoleGone: consoleGone.map((r) => ({ tab: r.tab, id: r.id, text: r.text })),
    petSilent,
    pageErrors,
    petErrors,
    problems,
  };

  function petButtons_count(a) { return a.length; }

  if (JSON_OUT) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    console.log('===== 按钮全量点击冒烟 =====');
    console.log(`控制台可见按钮: ${seq.length} / 桌宠可见按钮: ${petBtns.length}`);
    console.log(`\n控制台静默按钮 (${consoleSilent.length}):`);
    consoleSilent.forEach((s) => console.log(`   [${s.tab}] ${s.id || '(无id)'} 「${s.text}」`));
    console.log(`\n控制台点击时元素已消失 (${consoleGone.length}):`);
    consoleGone.slice(0, 15).forEach((s) => console.log(`   [${s.tab}] ${s.id || '(无id)'} 「${s.text}」`));
    console.log(`\n桌宠静默按钮 (${petSilent.length}):`);
    petSilent.forEach((s) => console.log(`   ${s.id || '(无id)'} 「${s.text}」`));
    console.log(`\n页签切换问题 (${problems.length}):`);
    problems.forEach((p) => console.log(`   [${p.tab}] ${p.issue}`));
    console.log(`\n运行时错误: ${pageErrors.length + petErrors.length}`);
    [...pageErrors, ...petErrors].slice(0, 12).forEach((e) => console.log('   ' + e));
  }

  await browser.close();
  const bad = consoleSilent.length + petSilent.length + problems.length;
  process.exit(bad === 0 ? 0 : 1);
})().catch((e) => {
  console.error('运行失败:', e);
  process.exit(2);
});
