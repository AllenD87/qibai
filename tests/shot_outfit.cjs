/* v45.31 形象换装截图验证（浏览器 mock 模式）
 * 覆盖：右键菜单「换装」→ 服装列表渲染 → 未达 L6 拦截 → 达 L6 切换 → 画面滤镜生效
 */
const { chromium } = require('playwright');

const BASE = process.env.BASE || 'http://127.0.0.1:8099';
const SHOTS = '/home/user/Doubao/chats/38448289926008322/qibai-repo/tests/shots';

(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/local/bin/chromium' });
  const page = await browser.newPage({ viewport: { width: 520, height: 760 } });
  const log = [];
  page.on('console', (m) => log.push(m.type() + ': ' + m.text()));
  page.on('pageerror', (e) => log.push('PAGEERROR: ' + e.message));

  await page.goto(BASE + '/src/pet.html', { waitUntil: 'load' });
  await page.waitForTimeout(600);

  // 1) 低亲密度（默认）→ 点换装应被拦截
  await page.mouse.click(260, 400, { button: 'right' });
  await page.waitForTimeout(220);
  const menuOpen = await page.evaluate(() => !document.getElementById('ctxMenu').classList.contains('hidden'));
  console.log('菜单可见:', menuOpen);

  await page.click('#ctxOutfit');
  await page.waitForTimeout(320);
  const rows = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.ctx-outfit-row')).map((r) => r.dataset.outfit + '|' + r.textContent.trim())
  );
  console.log('列表项:', JSON.stringify(rows));
  await page.locator('#ctxMenu').screenshot({ path: SHOTS + '/v45_31_outfit_list.png' });

  // 低等级时点击应提示未解锁
  await page.click('.ctx-outfit-row[data-outfit="night"]');
  await page.waitForTimeout(260);
  const toast1 = await page.evaluate(() => {
    const t = document.getElementById('toast') || document.querySelector('.toast');
    return t ? t.textContent.trim() : '(无 toast 元素)';
  });
  console.log('低等级点击提示:', toast1);

  // 2) 抬到 L6（points=1200）后重载
  await page.evaluate(() => {
    const raw = JSON.parse(localStorage.getItem('qibai.mock.intimacy') || '{}');
    const pid = localStorage.getItem('qibai.mock.active') || 'gudian';
    if (raw.entries) Object.keys(raw.entries).forEach((k) => { raw.entries[k].points = 1200; });
    localStorage.setItem('qibai.mock.intimacy', JSON.stringify(raw));
    return pid;
  });
  await page.reload({ waitUntil: 'load' });
  await page.waitForTimeout(600);

  await page.mouse.click(260, 400, { button: 'right' });
  await page.waitForTimeout(220);
  await page.click('#ctxOutfit');
  await page.waitForTimeout(320);
  await page.click('.ctx-outfit-row[data-outfit="night"]');
  await page.waitForTimeout(500);

  const after = await page.evaluate(() => ({
    listHidden: document.getElementById('ctxOutfitList').classList.contains('hidden'),
    text: document.getElementById('ctxOutfitText').textContent.trim(),
    current: Array.from(document.querySelectorAll('.ctx-outfit-row.is-current')).map((r) => r.dataset.outfit),
    imgFilter: (document.querySelector('#characterImg') || {}).style ? document.querySelector('#characterImg').style.filter : '(无 petImg)',
  }));
  console.log('切换后:', JSON.stringify(after));

  // 重新打开菜单截「当前项带勾」+ 主画面
  await page.mouse.click(260, 400, { button: 'right' });
  await page.waitForTimeout(200);
  await page.click('#ctxOutfit');
  await page.waitForTimeout(300);
  await page.locator('#ctxMenu').screenshot({ path: SHOTS + '/v45_31_outfit_current.png' });
  await page.mouse.click(460, 700); // 关菜单
  await page.waitForTimeout(400);
  await page.screenshot({ path: SHOTS + '/v45_31_outfit_stage.png' });

  const errs = log.filter((l) => l.startsWith('PAGEERROR') || l.startsWith('error'));
  console.log('页面错误:', errs.length ? JSON.stringify(errs.slice(0, 6)) : '无');
  await browser.close();
})().catch((e) => { console.error('FAILED', e.message); process.exit(1); });
