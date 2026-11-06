'use strict';
const { chromium } = require('playwright');
const path = require('path');
const ROOT = path.resolve(__dirname, '..');
const CONSOLE_URL = 'file://' + path.join(ROOT, 'src/console.html');
const results = [];
function report(name, pass, detail) {
  results.push({ name, pass });
  console.log((pass ? 'PASS' : 'FAIL') + ' | ' + name + (detail ? ' | ' + detail : ''));
}
(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--allow-file-access-from-files'],
  });
  try {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    await page.addInitScript(() => {
      window.__calls = [];
      const callbacks = new Map();
      const listeners = {};
      let cbId = 1;
      let evtId = 1;
      window.__fireEvent = (event, payload) => {
        (listeners[event] || []).forEach((id) => {
          const fn = callbacks.get(id);
          if (fn) try { fn({ payload }); } catch (e) {}
        });
      };
      // 复刻 Tauri v2 internals（bridge 走的是这条路径）
      window.__TAURI_INTERNALS__ = {
        transformCallback: (fn) => { const id = cbId++; callbacks.set(id, fn); return id; },
        unregisterCallback: (id) => { callbacks.delete(id); },
        invoke: async (cmd, args) => {
          window.__calls.push({ cmd, args });
          if (cmd === 'plugin:event|listen') {
            listeners[args.event] = listeners[args.event] || [];
            listeners[args.event].push(args.handler);
            return evtId++;
          }
          if (cmd === 'plugin:event|unlisten') return null;
          if (cmd === 'approve_skill') {
            throw new Error('这个操作需要亲密度 L4 才能批准（当前 L1）');
          }
          return {};
        },
      };
    });
    await page.goto(CONSOLE_URL);
    await page.waitForTimeout(600);
    await page.evaluate(() => {
      window.__fireEvent('qibai://skill-approval', {
        request_id: 'test-1', skill: 'write_file', description: '写文件到工作区',
      });
    });
    await page.waitForTimeout(300);
    const modalShown = await page.isVisible('#approvalModal').catch(() => false);
    report('A1 审批弹窗弹出', modalShown);
    await page.click('#approvalAllow');
    await page.waitForTimeout(300);
    const modalStillShown = await page.isVisible('#approvalModal').catch(() => false);
    report('A2 审批失败后弹窗不关', modalStillShown);
    const errText = await page.evaluate(() => {
      const el = document.getElementById('approvalErr');
      return el ? el.textContent : '';
    });
    report('A3 err 文案显示在弹窗内（含亲密度）', errText.includes('亲密度'), 'err=' + errText);
  } finally { await browser.close(); }
  const pass = results.filter(r => r.pass).length;
  const fail = results.length - pass;
  console.log('\n===== 审批失败 QA: ' + pass + ' PASS / ' + fail + ' FAIL =====');
  process.exit(fail === 0 ? 0 : 1);
})().catch(e => { console.error('QA runner 异常:', e); process.exit(2); });
