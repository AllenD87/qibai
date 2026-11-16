/* v45.31-e 控制台「模型与接口」设置页截图验证（浏览器 mock 模式）
 * 覆盖：协议下拉存在且默认 auto → 选 Anthropic 服务商自动填充官方端点 → 选中态截图
 */
const { chromium } = require('playwright');

const BASE = process.env.BASE || 'http://127.0.0.1:8099';
const SHOTS = '/home/user/Doubao/chats/38448289926008322/qibai-repo/tests/shots';

(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/local/bin/chromium' });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  const errs = [];
  page.on('pageerror', (e) => errs.push('PAGEERROR: ' + e.message));

  await page.goto(BASE + '/src/console.html', { waitUntil: 'load' });
  await page.waitForTimeout(600);
  await page.click('[data-tab="settings"]');
  await page.waitForTimeout(600);
  // 设置页默认停在「声音」分组，模型卡片在「通用」分组
  await page.click('.set-subtab[data-setstab="general"]');
  await page.waitForTimeout(600);

  const init = await page.evaluate(() => {
    const p = document.getElementById('setProtocol');
    return {
      exists: !!p,
      value: p && p.value,
      options: p ? Array.from(p.options).map((o) => o.value) : [],
      providerOptions: Array.from(document.getElementById('setProvider').options).map((o) => o.value),
      anthropicLabel: (document.querySelector('#setProvider option[value="anthropic"]') || {}).textContent,
    };
  });
  console.log('初始:', JSON.stringify(init));

  // 卡片可能被折叠分组包着，用整页截图保证看得见
  await page.locator('#setProtocol').scrollIntoViewIfNeeded().catch(() => {});
  await page.waitForTimeout(300);
  await page.screenshot({ path: SHOTS + '/v45_31_console_llm_auto.png' });

  // 切到 Anthropic：应自动填官方端点 + 模型提示，协议保持 auto（走原生）
  await page.selectOption('#setProvider', 'anthropic');
  await page.waitForTimeout(500);
  const after = await page.evaluate(() => ({
    base: document.getElementById('setBaseUrl').value,
    model: document.getElementById('setModel').value,
    protocol: document.getElementById('setProtocol').value,
  }));
  console.log('选 Anthropic 后:', JSON.stringify(after));
  await page.screenshot({ path: SHOTS + '/v45_31_console_llm_anthropic.png' });

  // 显式切到原生协议，确认下拉可用
  await page.selectOption('#setProtocol', 'anthropic');
  await page.waitForTimeout(200);
  const forced = await page.evaluate(() => document.getElementById('setProtocol').value);
  console.log('强制原生:', forced);

  console.log('ERRORS:', errs.length ? errs.join(' | ') : 'none');
  await browser.close();
  if (!init.exists || init.value !== 'auto') process.exit(1);
  if (errs.length) process.exit(2);
})();
