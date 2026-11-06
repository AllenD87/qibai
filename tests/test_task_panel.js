const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/local/bin/chromium', args: ['--no-sandbox','--disable-dev-shm-usage'] });
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  const calls = [];
  await page.exposeFunction('invoke', async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'task_list') return [];
    if (cmd === 'current_audio_output') return 'default';
    if (cmd === 'list_output_devices') return [];
    if (cmd === 'set_audio_output') return {};
    if (cmd === 'set_search_engine') return {};
    if (cmd === 'current_search_engine') return 'auto';
    throw new Error('mock:' + cmd);
  });
  await page.exposeFunction('listen', async () => () => {});
  await page.goto('file://' + require('path').resolve(__dirname, '../src/console.html'));
  await page.waitForTimeout(300);

  // 切到任务 tab
  await page.click('button[data-tab="tasks"]');
  await page.waitForTimeout(200);
  const empty = await page.textContent('#tasksEmpty');
  console.log(empty && empty.includes('还没有任务') ? 'PASS 空态文案' : 'FAIL 空态: ' + empty);

  // 开始按钮在无任务时应该是 disabled 或不存在
  const startDisabled = await page.$eval('#btnTaskStart', el => el.disabled).catch(() => null);
  console.log(startDisabled === true ? 'PASS 开始按钮 disabled（无任务/未确认）' : 'INFO start disabled=' + startDisabled);

  console.log('\n===== 任务面板 QA: 2 checks =====');
  await browser.close();
})();
