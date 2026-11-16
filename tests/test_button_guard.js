// v45.22 按钮守卫契约测试：防连点表与真实 HTML 必须对得上，否则按钮会「看起来有说明、其实没挂上」
const fs = require('fs');
const path = require('path');
const root = path.join(__dirname, '..');

let pass = 0, fail = 0;
function ok(name, cond) {
  if (cond) { pass++; console.log('  ✓ ' + name); }
  else { fail++; console.log('  ✗ ' + name); }
}
function read(p) { return fs.readFileSync(path.join(root, p), 'utf8'); }

const styles = read('src/styles.css');
const consoleJs = read('src/console.js');
const petJs = read('src/pet.js');
const consoleHtml = read('src/console.html');
const petHtml = read('src/pet.html');

console.log('== 1. 三态样式 ==');
ok('button:disabled 有专属视觉（不能和正常态一个样）', /button:disabled\s*\{[^}]*opacity/.test(styles));
ok('button:not(:disabled):active 有按下反馈', /button:not\(:disabled\):active/.test(styles));
ok('button[data-busy="1"] 有忙态', /button\[data-busy="1"\]/.test(styles));
ok('focus-visible 焦点环保留键盘可达', /button:focus-visible/.test(styles));

console.log('== 2. 守卫挂载 ==');
ok('console.js 有 initButtonGuard', /initButtonGuard/.test(consoleJs));
ok('console.js 拦截 data-async 连点', /dataset\.async !== '1'/.test(consoleJs) && /stopImmediatePropagation/.test(consoleJs));
ok('console.js 兜未处理 Promise', /unhandledrejection/.test(consoleJs));
ok('console.js 兜同步报错', /addEventListener\('error'/.test(consoleJs));
ok('pet.js 有 initPetButtonGuard', /initPetButtonGuard/.test(petJs));
ok('pet.js 拦截连点', /dataset\.async !== '1'/.test(petJs));
ok('pet.js 兜未处理 Promise', /unhandledrejection/.test(petJs));

console.log('== 3. 说明表与 HTML 对得上 ==');
function idsFromBlock(src, marker) {
  const i = src.indexOf(marker);
  if (i < 0) return [];
  const start = src.indexOf('{', i);
  const end = src.indexOf('\n  };', start);
  const block = src.slice(start, end < 0 ? src.length : end);
  return Array.from(block.matchAll(/^    ([A-Za-z][A-Za-z0-9_]*):/gm)).map(m => m[1]);
}
const cIds = idsFromBlock(consoleJs, 'const ASYNC_TIPS');
const pIds = idsFromBlock(petJs, 'const TIPS');
ok('控制台说明表非空（>=20 条）', cIds.length >= 20);
ok('桌宠说明表非空（>=10 条）', pIds.length >= 10);

const missingC = cIds.filter(id => !new RegExp('id="' + id + '"').test(consoleHtml));
const missingP = pIds.filter(id => !new RegExp('id="' + id + '"').test(petHtml));
ok('控制台说明表里的 id 全部存在于 console.html', missingC.length === 0);
if (missingC.length) console.log('    缺失: ' + missingC.join(', '));
ok('桌宠说明表里的 id 全部存在于 pet.html', missingP.length === 0);
if (missingP.length) console.log('    缺失: ' + missingP.join(', '));

console.log('== 3b. 防连点白名单 ==');
function listFromBlock(src, marker) {
  const i = src.indexOf(marker);
  const start = src.indexOf('[', i);
  const end = src.indexOf('];', start);
  const block = src.slice(start, end < 0 ? src.length : end);
  return Array.from(block.matchAll(/'([A-Za-z][A-Za-z0-9_]*)'/g)).map(m => m[1]);
}
const guardIds = listFromBlock(consoleJs, 'const GUARD_LIST');
ok('白名单非空', guardIds.length >= 20);
const missingG = guardIds.filter(id => !new RegExp('id="' + id + '"').test(consoleHtml));
ok('白名单 id 全部存在', missingG.length === 0);
if (missingG.length) console.log('    缺失: ' + missingG.join(', '));
// 打开弹窗/选文件这类幂等按钮不得进白名单，否则第二次点击会被吞掉
['btnSkillInstallUrl', 'btnSkillInstallFile', 'btnSkillOpenDir', 'btnPluginInstall', 'btnConnAdd', 'btnPackInstall']
  .forEach((id) => ok('幂等按钮 ' + id + ' 不防连点', !guardIds.includes(id)));

console.log('== 4. 每条说明都是人话（不是占位符）==');
function tipsValues(src, marker) {
  const i = src.indexOf(marker);
  const start = src.indexOf('{', i);
  const end = src.indexOf('\n  };', start);
  const block = src.slice(start, end < 0 ? src.length : end);
  return Array.from(block.matchAll(/^    [A-Za-z][A-Za-z0-9_]*: '([^']*)'/gm)).map(m => m[1]);
}
const cTips = tipsValues(consoleJs, 'const ASYNC_TIPS');
const pTips = tipsValues(petJs, 'const TIPS');
const bad = [...cTips, ...pTips].filter(t => t.length < 6 || /TODO|待补充|xxx/i.test(t));
ok('没有占位/空说明', bad.length === 0);
if (bad.length) console.log('    可疑: ' + JSON.stringify(bad));

console.log('== 5. 危险操作有明确后果说明 ==');
function tipMap(src, marker) {
  const i = src.indexOf(marker);
  const start = src.indexOf('{', i);
  const end = src.indexOf('\n  };', start);
  const block = src.slice(start, end < 0 ? src.length : end);
  const map = {};
  for (const m of block.matchAll(/^    ([A-Za-z][A-Za-z0-9_]*): '([^']*)'/gm)) map[m[1]] = m[2];
  return map;
}
const cMap = tipMap(consoleJs, 'const ASYNC_TIPS');
ok('btnClearAll 说明里写明不可恢复', /找不回来|不可撤销|删了/.test(cMap.btnClearAll || ''));
ok('btnClearOnly 说明里写明保留记忆（与全清区分开）', /角色记忆保留|记忆保留/.test(cMap.btnClearOnly || ''));
ok('btnNsBlock 说明里写明拉黑后果', /再也匹配不到|影响不可逆/.test(cMap.btnNsBlock || ''));

console.log('== 6. 兜底 toast 容器存在 ==');
ok('console.html 有 toastBox', /id="toastBox"/.test(consoleHtml));
ok('pet.html 有 toastBox', /id="toastBox"/.test(petHtml));

console.log('== 7. 失败不许无声无息 ==');
const cSilent = (consoleJs.match(/catch \(e\) \{\}/g) || []).length;
ok('console.js 没有完全静默的 catch', cSilent === 0);
if (cSilent) console.log('    仍有 ' + cSilent + ' 处');
// pet.js 只允许 debug_log 自报错这一种静默（它本身就是日志，失败再报错会递归）
const pSilentLines = (petJs.match(/.{0,220}catch \(e\) \{\}/g) || []);
const pSilentBad = pSilentLines.filter(l => !/debug_log/.test(l));
ok('pet.js 静默 catch 只剩 debug_log', pSilentBad.length === 0);
if (pSilentBad.length) console.log('    违规: ' + JSON.stringify(pSilentBad));

console.log('== 8. 语音路径不许假成功 ==');
ok('锁定失败不再谎报「锁死啦」', /okLock/.test(petJs) && /锁定没成功/.test(petJs));
ok('解锁失败会告知用户', /okUnlock/.test(petJs) && /解锁没成功/.test(petJs));
ok('面板打不开会说没打开', /okPanel/.test(petJs) && /没能打开/.test(petJs));

console.log('\n' + pass + ' passed, ' + fail + ' failed');
process.exit(fail ? 1 : 0);
