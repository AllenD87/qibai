/* v45.31 换装语音入口验证（浏览器 mock 模式，走前端 INTENTS 兜底）
 * 覆盖：「换成夜色」→ 真换并回话；「换装」→ 念衣柜不乱切；「换成白色」→ 不误命中换装
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

  // 抬到 L6：先让 mock 落一次 storage（否则 key 还不存在），再把所有角色 points 改 1200 后重载
  await page.evaluate(async () => {
    try { await window.__TAURI_MOCK__?.invoke?.('get_intimacy_state', {}); } catch (e) {}
    try { await window.invoke?.('get_intimacy_state', { persona_id: 'gudian' }); } catch (e) {}
  });
  await page.waitForTimeout(300);
  await page.evaluate(() => {
    const raw = JSON.parse(localStorage.getItem('qibai.mock.intimacy') || '{"entries":{}}');
    raw.entries = raw.entries || {};
    Object.keys(raw.entries).forEach((k) => { raw.entries[k].points = 1200; });
    // 没有任何 entry 时补一个 gudian，保证下一轮重载读得到
    if (!Object.keys(raw.entries).length) {
      raw.entries.gudian = { points: 1200 };
    }
    localStorage.setItem('qibai.mock.intimacy', JSON.stringify(raw));
  });
  await page.reload({ waitUntil: 'load' });
  await page.waitForTimeout(800);
  const lv = await page.evaluate(async () => {
    try {
      const st = await window.invoke('get_intimacy_state', { persona_id: 'gudian' });
      return { level: st && st.level, points: st && st.points };
    } catch (e) { return { err: String(e && e.message || e) }; }
  });
  console.log('抬升后等级:', JSON.stringify(lv));

  const ask = async (text) => {
    const input = page.locator('textarea, input[type="text"]').first();
    await input.fill(text);
    await page.keyboard.press('Enter');
    await page.waitForTimeout(900);
    return await page.evaluate(() => {
      const b = document.getElementById('speechText') || document.getElementById('bubbleText');
      return b ? b.textContent.trim() : '(无气泡文本)';
    });
  };

  console.log('说「换成夜色」→', JSON.stringify(await ask('换成夜色')));
  const st1 = await page.evaluate(() => ({
    filter: (document.querySelector('#characterImg') || {}).style.filter,
    outfit: (window.__state && window.__state.outfit) || '(未暴露)',
  }));
  console.log('  画面:', JSON.stringify(st1));

  console.log('说「换装」→', JSON.stringify(await ask('换装')));
  console.log('说「换成白色」→', JSON.stringify(await ask('换成白色')));

  await page.screenshot({ path: SHOTS + '/v45_31_voice_outfit.png' });
  console.log('页面错误:', JSON.stringify(log.filter((l) => l.startsWith('PAGEERROR'))));
  await browser.close();
})().catch((e) => {
  console.error('脚本失败:', e && e.message);
  process.exit(1);
});
