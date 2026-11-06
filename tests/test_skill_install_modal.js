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
      let cbId = 1;
      window.__TAURI_INTERNALS__ = {
        transformCallback: (fn) => { const id = cbId++; callbacks.set(id, fn); return id; },
        invoke: async (cmd, args) => {
          window.__calls.push({ cmd, args });
          if (cmd === 'skill_install_stage') {
            return {
              stage_id: 'stage-123', name: 'web-research', version: '1.0',
              author: 'test', source_url: (args && args.source_url) || '', description: '搜索技能',
              permissions: ['network'], file_count: 3, total_size: 1024,
            };
          }
          if (cmd === 'skill_install_cancel') return null;
          if (cmd === 'skill_install_commit') return { ok: true };
          if (cmd === 'get_skills_status') return { total_enabled: true, skills: {} };
          if (cmd === 'plugin_list') return [];
          if (cmd === 'expert_list') return [];
          if (cmd === 'connector_list') return [];
          if (cmd === 'skill_pack_list') return [];
          return {};
        },
      };
    });
    await page.goto(CONSOLE_URL);
    await page.waitForTimeout(600);

    // 切到插件 tab
    await page.click('.ctab[data-tab="plugins"]');
    await page.waitForTimeout(500);

    // 1) 点"从链接安装" → 模态出现且含权限表
    await page.click('#btnSkillInstallUrl');
    await page.waitForTimeout(100);
    await page.fill('#skillUrlInput', 'https://example.com/skill.zip');
    await page.press('#skillUrlInput', 'Enter');
    await page.waitForTimeout(400);

    const modalVisible = await page.evaluate(() => !document.getElementById('skillInstallModal').hasAttribute('hidden'));
    report('S1 模态出现', modalVisible);
    const bodyText = await page.evaluate(() => document.getElementById('skillInstallBody').textContent);
    report('S1b 含权限表 network', bodyText.includes('network'));

    // 2) 点取消 → cancel 被调、commit 未被调
    await page.click('#btnSkillInstallCancel');
    await page.waitForTimeout(200);
    const cancelCalled = await page.evaluate(() => window.__calls.some(c => c.cmd === 'skill_install_cancel'));
    const commitCalled = await page.evaluate(() => window.__calls.some(c => c.cmd === 'skill_install_commit'));
    report('S2 取消调了 cancel', cancelCalled);
    report('S2b 取消没调 commit', !commitCalled);

    // 3) 再走一次：点安装 → commit 带 stage token
    await page.evaluate(() => window.__calls.length = 0);
    await page.click('#btnSkillInstallUrl');
    await page.waitForTimeout(100);
    await page.fill('#skillUrlInput', 'https://example.com/skill2.zip');
    await page.press('#skillUrlInput', 'Enter');
    await page.waitForTimeout(400);
    await page.click('#btnSkillInstallConfirm');
    await page.waitForTimeout(200);
    const commit = await page.evaluate(() => window.__calls.find(c => c.cmd === 'skill_install_commit'));
    report('S3 commit 带 stage-123', !!(commit && commit.args && commit.args.stage_id === 'stage-123'),
      'commit=' + JSON.stringify(commit));
  } finally { await browser.close(); }
  const pass = results.filter(r => r.pass).length;
  const fail = results.length - pass;
  console.log('\n===== 技能安装 QA: ' + pass + ' PASS / ' + fail + ' FAIL =====');
  process.exit(fail === 0 ? 0 : 1);
})().catch(e => { console.error('QA runner 异常:', e); process.exit(2); });
