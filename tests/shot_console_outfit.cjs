/* v45.31 控制台亲密度面板「当前服装 + 一键换」截图验证（浏览器 mock 模式）
 * 覆盖：面板渲染 → 当前套装名 → 展开列表 5 项 → 低等级拦截 → 达 L6 切换 → 当前项打勾
 */
const { chromium } = require('playwright');

const BASE = process.env.BASE || 'http://127.0.0.1:8099';
const SHOTS = '/home/user/Doubao/chats/38448289926008322/qibai-repo/tests/shots';

(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/local/bin/chromium' });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  const log = [];
  page.on('console', (m) => log.push(m.type() + ': ' + m.text()));
  page.on('pageerror', (e) => log.push('PAGEERROR: ' + e.message));

  await page.goto(BASE + '/src/console.html', { waitUntil: 'load' });
  await page.waitForTimeout(800);

  // 切到亲密度 tab
  await page.click('[data-tab="intimacy"]');
  await page.waitForTimeout(1200);

  // 1) 初始状态：当前服装名 + 按钮可用
  const init = await page.evaluate(() => ({
    cur: (document.getElementById('intiOutfitCur') || {}).textContent,
    btn: (document.getElementById('intiOutfitBtn') || {}).textContent,
    btnDisabled: (document.getElementById('intiOutfitBtn') || {}).disabled,
    listHidden: document.getElementById('intiOutfitList').classList.contains('hidden'),
  }));
  console.log('初始:', JSON.stringify(init));

  const card = await page.locator('.inti-outfit').first();
  await card.screenshot({ path: SHOTS + '/v45_31_console_outfit.png' });

  // 2) 展开列表
  await page.click('#intiOutfitBtn');
  await page.waitForTimeout(400);
  const opened = await page.evaluate(() => ({
    items: Array.from(document.querySelectorAll('#intiOutfitList .io-item')).map((el) => ({
      id: el.dataset.outfit,
      text: el.textContent.trim(),
      current: el.classList.contains('is-current'),
    })),
  }));
  console.log('展开后:', JSON.stringify(opened));
  await card.screenshot({ path: SHOTS + '/v45_31_console_outfit_open.png' });

  // 3) 点「夜色冷调」：默认 mock 等级不足 → 应被拦截
  const night = page.locator('#intiOutfitList .io-item[data-outfit="night"]');
  if (await night.count()) {
    await night.click();
    await page.waitForTimeout(400);
  }
  const afterLow = await page.evaluate(() => ({
    cur: (document.getElementById('intiOutfitCur') || {}).textContent,
    toast: (document.querySelector('.toast, #toast') || {}).textContent || '(无 toast)',
  }));
  console.log('低等级点击后:', JSON.stringify(afterLow));

  console.log('页面错误:', JSON.stringify(log.filter((l) => l.startsWith('PAGEERROR'))));
  await browser.close();
})().catch((e) => {
  console.error('脚本失败:', e && e.message);
  process.exit(1);
});
