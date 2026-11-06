/* ============================================================================
 * v45.7 清空聊天按钮最小 QA（console.html）
 * 用例：
 *   C1 点「清空聊天」→ 模态打开；点取消 → 关闭、不发请求
 *   C2 点「只清聊天记录」→ invoke clear_conversation_scoped('chat')
 *   C3 点「连角色记忆一起清」→ invoke clear_conversation_scoped('all')
 * ========================================================================== */
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
      window.__TAURI__ = {
        invoke: async (cmd, args) => {
          window.__calls.push({ cmd, args });
          if (cmd === 'clear_conversation_scoped') {
            return { cleared: 5, backup: '/tmp/conversation.json.bak-test', kept_memory: args.scope === 'chat' };
          }
          return null;
        },
      };
      // mock event listen
      window.__tauriEvents = {};
    });
    await page.goto(CONSOLE_URL);
    await page.waitForTimeout(400);

    // C1: 打开模态 → 取消
    await page.click('#btnClearChat');
    const modalVisible = await page.isVisible('#confirmClearChat');
    report('C1a 点清空按钮 → 模态打开', modalVisible, `modal visible=${modalVisible}`);
    await page.click('#btnClearCancel');
    await page.waitForTimeout(100);
    const modalHidden = await page.isHidden('#confirmClearChat');
    const callsAfterCancel = await page.evaluate(() => window.__calls.filter(c => c.cmd === 'clear_conversation_scoped').length);
    report('C1b 取消 → 模态关闭且不发请求', modalHidden && callsAfterCancel === 0,
      `hidden=${modalHidden} calls=${callsAfterCancel}`);

    // C2: 只清聊天
    await page.click('#btnClearChat');
    await page.waitForTimeout(100);
    await page.click('#btnClearOnly');
    await page.waitForTimeout(200);
    const chatCall = await page.evaluate(() => window.__calls.filter(c => c.cmd === 'clear_conversation_scoped' && c.args.scope === 'chat').length);
    const modalClosedAfterChat = await page.isHidden('#confirmClearChat');
    report('C2 只清聊天 → scope=chat', chatCall === 1 && modalClosedAfterChat,
      `chatCalls=${chatCall} modalClosed=${modalClosedAfterChat}`);

    // C3: 连记忆一起清
    await page.click('#btnClearChat');
    await page.waitForTimeout(100);
    await page.click('#btnClearAll');
    await page.waitForTimeout(200);
    const allCall = await page.evaluate(() => window.__calls.filter(c => c.cmd === 'clear_conversation_scoped' && c.args.scope === 'all').length);
    report('C3 连记忆一起清 → scope=all', allCall === 1, `allCalls=${allCall}`);

  } finally { await browser.close(); }

  const pass = results.filter(r => r.pass).length;
  const fail = results.length - pass;
  console.log(`\n===== 清空聊天 QA: ${pass} PASS / ${fail} FAIL =====`);
  process.exit(fail === 0 ? 0 : 1);
})().catch(e => { console.error('QA runner 异常:', e); process.exit(2); });
