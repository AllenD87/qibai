/* 交付前视觉自检：把本轮新增的入口在真实 Chromium 里截出来看布局。
 * 不依赖完整 mock —— tauri-bridge.js 在浏览器环境自带 localStorage mock，
 * 页面结构（按钮/输入/提示条）本来就在 HTML 里，够看排版有没有被撑坏。
 * 运行：node tests/shot_ui.js
 */
'use strict';

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const ROOT = path.resolve(__dirname, '..');
const OUT = path.join(ROOT, 'tests/shots');
fs.mkdirSync(OUT, { recursive: true });

async function shot(locator, file, label) {
  try {
    await locator.scrollIntoViewIfNeeded();
    await locator.screenshot({ path: path.join(OUT, file) });
    const box = await locator.boundingBox();
    console.log(`OK ${label}: ${Math.round(box.width)}x${Math.round(box.height)}`);
    return true;
  } catch (e) {
    console.log(`!! ${label} 截图失败: ${e.message.split('\n')[0]}`);
    return false;
  }
}

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/local/bin/chromium',
    args: ['--no-sandbox'],
  });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errs = [];
  page.on('pageerror', (e) => errs.push('pageerror: ' + e.message));

  await page.goto('file://' + path.join(ROOT, 'src/console.html'), { waitUntil: 'load' });
  await page.waitForTimeout(900);

  // 1) 设置页：知识库搜索行
  await page.click('[data-tab="settings"]');
  await page.waitForTimeout(500);
  const kbField = page.locator('.field:has(#kbSearchInput)');
  if (await kbField.count()) await shot(kbField.first(), 'v45_24_kb_search.png', '知识库搜索行');
  else console.log('!! 找不到知识库搜索行');

  // 2) 设置页：音频诊断按钮所在行
  const audioRow = page.locator('div:has(#btnAudioTest):has(#btnAudioDiagnose)').last();
  if (await audioRow.count()) await shot(audioRow, 'v45_24_audio_diag.png', '音频诊断按钮行');
  else console.log('!! 找不到音频诊断按钮');

  // 3) 插件页：扫描目录行
  await page.click('[data-tab="plugins"]');
  await page.waitForTimeout(500);
  const scanBar = page.locator('.plg-toolbar:has(#btnSkillScan)');
  if (await scanBar.count()) await shot(scanBar.first(), 'v45_24_plugin_scan.png', '插件扫描行');
  else console.log('!! 找不到扫描行');

  // 4) 技能文档弹层
  await page.evaluate(() => {
    const m = document.getElementById('docModal');
    if (!m) return;
    document.getElementById('docModalTitle').textContent = '代码检查 · 技能说明';
    document.getElementById('docModalBody').textContent =
      '# 代码检查\n\n只读分析工作区代码，给出结构与风险建议，不修改任何文件。\n\n'
      + '## 参数\n- path：要检查的目录\n- depth：遍历深度\n\n## 输出\n按文件列出问题，标注严重程度。\n\n'
      + '## 边界\n不执行、不写入、不联网；发现可疑依赖只标注不升级。\n\n## 用法\n在控制台「代码检查」页填目录后点运行。\n\n'
      + '## 备注\n长文本换行是否撑破弹层，看这一屏就能判。';
    m.removeAttribute('hidden');
  });
  await page.waitForTimeout(300);
  const card = page.locator('#docModal .clear-modal-card');
  if (await card.count()) await shot(card, 'v45_24_doc_modal.png', '技能文档弹层');

  // 5) 确认弹层
  await page.evaluate(() => {
    document.getElementById('docModal').setAttribute('hidden', '');
    const m = document.getElementById('confirmActionModal');
    document.getElementById('confirmActionTitle').textContent = '卸载「代码检查」';
    document.getElementById('confirmActionDesc').textContent =
      '会删掉这个技能的本地目录（code-check），已授予的权限一并失效，且不可恢复。';
    document.getElementById('btnConfirmActionOk').textContent = '确认卸载';
    m.removeAttribute('hidden');
  });
  await page.waitForTimeout(300);
  const ccard = page.locator('#confirmActionModal .clear-modal-card');
  if (await ccard.count()) await shot(ccard, 'v45_24_confirm.png', '卸载确认弹层');

  await browser.close();
  console.log('页面异常:', errs.length ? errs.join(' | ') : '无');
})();
