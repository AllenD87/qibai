'use strict';
/* v45.12 MCP 服务器面板 QA
 * 覆盖：列表渲染 / 状态色映射 / 禁用后不在工具表 / 测试调用结果展示。
 * 全部 invoke 打桩，不依赖真实子进程。
 */
const { chromium } = require('playwright');
const path = require('path');
const ROOT = path.resolve(__dirname, '..');
const CONSOLE_URL = 'file://' + path.join(ROOT, 'src/console.html');
const results = [];
function report(name, pass, detail) {
  results.push({ name, pass });
  console.log((pass ? 'PASS' : 'FAIL') + ' | ' + name + (detail ? ' | ' + detail : ''));
}

const FAKE_LIST = [
  {
    name: 'fs', command: 'npx fake-fs', transport: 'stdio',
    args: ['/tmp/dir'], env: {}, enabled: true, trust: 'readonly',
    state: 'ready', state_detail: null, tools_count: 1,
    tools: [{ name: 'read_file', description: '读文件' }],
  },
  {
    name: 'broken', command: 'npx fake-broken', transport: 'stdio',
    args: [], env: {}, enabled: true, trust: 'confirm',
    state: 'unavailable', state_detail: 'initialize 失败：exit code=1', tools_count: 0, tools: [],
  },
  {
    name: 'off', command: 'npx fake-off', transport: 'stdio',
    args: [], env: {}, enabled: false, trust: 'confirm',
    state: 'disabled', state_detail: null, tools_count: 0, tools: [],
  },
];

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--allow-file-access-from-files'],
  });
  try {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    await page.addInitScript((fakeList) => {
      window.__calls = [];
      const callbacks = new Map();
      let cbId = 1;
      window.__TAURI_INTERNALS__ = {
        transformCallback: (fn) => { const id = cbId++; callbacks.set(id, fn); return id; },
        invoke: async (cmd, args) => {
          window.__calls.push({ cmd, args });
          if (cmd === 'mcp_list') return JSON.parse(JSON.stringify(fakeList));
          if (cmd === 'mcp_test_call') {
            return { ok: true, is_error: false, content: 'TEST_OK: hello from ' + args.server + '/' + args.tool };
          }
          if (cmd === 'mcp_set_enabled') return null;
          if (cmd === 'mcp_restart') return null;
          if (cmd === 'mcp_upsert') return null;
          if (cmd === 'mcp_remove') return null;
          if (cmd === 'get_skills_status') return { total_enabled: true, skills: {} };
          if (cmd === 'plugin_list') return [];
          if (cmd === 'expert_list') return [];
          if (cmd === 'connector_list') return [];
          return {};
        },
      };
    }, FAKE_LIST);
    await page.goto(CONSOLE_URL);
    await page.waitForTimeout(500);

    // 切到设置 tab（触发 initMcpPanel）
    await page.click('.ctab[data-tab="settings"]');
    await page.waitForTimeout(600);

    // 1) 列表渲染：三个 server 名都出现在 #mcpList
    const listText = await page.evaluate(() => document.getElementById('mcpList').textContent);
    report('S1 列表渲染 fs/broken/off',
      listText.includes('fs') && listText.includes('broken') && listText.includes('off'));

    // 2) 状态色映射：ready 行绿点 (#3aa655)，unavailable 行红点 + 原因文案
    const colors = await page.evaluate(() => {
      const dots = Array.from(document.querySelectorAll('#mcpList .mcp-state-dot'));
      return dots.map((d) => d.style.background);
    });
    report('S2a 状态色 ready=绿', colors.some((c) => c.indexOf('58, 166, 85') >= 0 || c === '#3aa655'),
      'dots=' + JSON.stringify(colors));
    const reasonText = await page.evaluate(() => document.getElementById('mcpList').textContent);
    report('S2b unavailable 显示原因', reasonText.includes('initialize 失败'));

    // 3) 禁用后不在工具表：off 行工具数=0，测试调用下拉无可用工具
    const offRow = await page.evaluate(() => {
      const rows = Array.from(document.querySelectorAll('#mcpList > div'));
      const off = rows.find((r) => r.textContent.includes('npx fake-off'));
      if (!off) return null;
      off.querySelector('button:last-of-type') && off.querySelector('button:last-of-type').click(); // 展开测试区
      return {
        hasZeroTools: off.textContent.includes('0 个工具'),
        selectOptions: Array.from(off.querySelectorAll('select option')).map((o) => o.textContent),
      };
    });
    report('S3a 禁用行 0 个工具', !!(offRow && offRow.hasZeroTools));
    report('S3b 禁用行测试下拉无可用工具',
      !!(offRow && offRow.selectOptions.some((t) => t.indexOf('服务器未就绪') >= 0)),
      'opts=' + JSON.stringify(offRow && offRow.selectOptions));

    // 4) 测试调用结果展示：fs 行展开 → 运行 → 结果出现 TEST_OK
    await page.evaluate(() => {
      const rows = Array.from(document.querySelectorAll('#mcpList > div'));
      const fsRow = rows.find((r) => r.textContent.includes('npx fake-fs'));
      const btn = Array.from(fsRow.querySelectorAll('button')).find((b) => b.textContent === '测试调用');
      btn.click();
    });
    await page.waitForTimeout(100);
    await page.evaluate(() => {
      const rows = Array.from(document.querySelectorAll('#mcpList > div'));
      const fsRow = rows.find((r) => r.textContent.includes('npx fake-fs'));
      const run = Array.from(fsRow.querySelectorAll('button')).find((b) => b.textContent === '运行');
      run.click();
    });
    await page.waitForTimeout(300);
    const fsResult = await page.evaluate(() => {
      const rows = Array.from(document.querySelectorAll('#mcpList > div'));
      const fsRow = rows.find((r) => r.textContent.includes('npx fake-fs'));
      return fsRow.textContent.includes('TEST_OK: hello from fs/read_file');
    });
    const testCallMade = await page.evaluate(() =>
      window.__calls.some((c) => c.cmd === 'mcp_test_call' && c.args && c.args.server === 'fs'));
    report('S4a 测试调用结果展示', fsResult);
    report('S4b 测试调用真走了 mcp_test_call', testCallMade);
  } finally {
    await browser.close();
  }
  const pass = results.filter((r) => r.pass).length;
  const fail = results.length - pass;
  console.log('\n===== MCP 面板 QA: ' + pass + ' PASS / ' + fail + ' FAIL =====');
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error('QA runner 异常:', e); process.exit(2); });
