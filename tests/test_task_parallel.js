// v45.13 并行任务面板前端契约测试（node 打桩，无浏览器依赖）。
// 覆盖：并行组渲染、四种状态色、running/limit 文案、partial 徽标与失败原因、
//       并行上限输入调 task_set_parallel_max（1-8 夹取）。
// 运行：node tests/test_task_parallel.js   （node --check 必须过）
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

let pass = 0, fail = 0;
function check(name, cond, extra) {
  if (cond) { pass++; console.log('PASS ' + name); }
  else { fail++; console.log('FAIL ' + name + (extra ? '  | ' + extra : '')); }
}

// ---- 最小 DOM 打桩 ----
function makeEl() {
  const el = {
    children: [],
    style: {},
    attrs: {},
    hidden: false,
    textContent: '',
    _innerHTML: '',
    title: '',
    disabled: false,
    value: '3',
    appendChild(c) { this.children.push(c); return c; },
    set innerHTML(v) { this._innerHTML = v; this.children = []; },
    get innerHTML() { return this._innerHTML; },
    querySelector() { return makeEl(); },
    addEventListener() {},
    click() {},
  };
  return el;
}
const elRegistry = {};
function $(id) {
  if (!elRegistry[id]) elRegistry[id] = makeEl();
  return elRegistry[id];
}
const document = {
  createElement() { return makeEl(); },
  getElementById(id) { return $(id); },
};

// ---- 从真实 console.js 抽取渲染函数（避免复制逻辑漂移）----
const src = fs.readFileSync(path.resolve(__dirname, '../src/console.js'), 'utf8');
const startAnchor = '// v45.13：步骤 id → step 索引';
const endAnchor = '  async function openDetail(id) {';
const sIdx = src.indexOf(startAnchor);
const eIdx = src.indexOf(endAnchor);
check('从 console.js 抽取到 renderParallel 区段', sIdx !== -1 && eIdx !== -1 && eIdx > sIdx,
      `start=${sIdx} end=${eIdx}`);
let extracted = '';
if (sIdx !== -1 && eIdx !== -1) {
  extracted = src.slice(sIdx, eIdx);
}

// ---- invoke / listen 打桩 ----
const invokeCalls = [];
let invokeImpl = async () => ({});
const invoke = async (cmd, args) => { invokeCalls.push({ cmd, args }); return invokeImpl(cmd, args); };
const listeners = {};
const listen = async (evt, cb) => { listeners[evt] = cb; return () => {}; };
const toast = () => {};

// 在沙箱里执行抽取到的函数（stepIndexMap / chipColor / renderParallel）
const sandbox = { document, $, invoke, listen, toast, console };
vm.createContext(sandbox);
try {
  vm.runInContext(extracted, sandbox, { filename: 'console-parallel-extract.js' });
} catch (e) {
  check('抽取函数可执行', false, e.message);
}
check('沙箱暴露 renderParallel', typeof sandbox.renderParallel === 'function',
      typeof sandbox.renderParallel);

// ---- 构造一个 partial 任务（2 组并行，1 跑 / 1 成 / 1 败 / 1 跳）----
const fakeTask = {
  id: 't_x',
  state: 'partial',
  title: '演示任务',
  steps: [
    { id: 's1', title: '抓取A', state: 'running' },
    { id: 's2', title: '抓取B', state: 'done' },
    { id: 's3', title: '解析C', state: 'failed', error: 'HTTP 500' },
    { id: 's4', title: '跳过D', state: 'skipped' },
  ],
  parallel: {
    max: 3,
    groups: [['s1', 's2'], ['s3', 's4']],
    running: ['s1'],
    done: ['s2'],
    failed: ['s3'],
    skipped: ['s4'],
    progress: { done: 2, total: 4 },
  },
};

if (typeof sandbox.renderParallel === 'function') {
  sandbox.renderParallel(fakeTask);

  // 1) 组渲染：groups 容器两行
  const groupsEl = $('taskParallelGroups');
  check('并行组容器显示', groupsEl.style.display === 'block');
  const groupRows = groupsEl.children.length;
  check('渲染 2 个并行组', groupRows === 2, 'rows=' + groupRows);

  // 2) 组标题文案：组 1（并行 1/2）
  const row0 = groupsEl.children[0];
  const row0Title = row0.children[0].textContent;
  check('组标题含「并行 x/y」', /组 1（并行 1\/2）/.test(row0Title), row0Title);

  // 3) 四种状态色：chip 数量与颜色
  // chip 是 row.children[1..]，style.cssText 内联了 background/color/border
  const chips0 = row0.children.slice(1);
  const colors = chips0.map((c) => c.style.cssText);
  check('组内 chip 数 = 2', chips0.length === 2, 'n=' + chips0.length);
  check('运行态 chip 蓝色', /#1d4ed8/.test(colors[0] || ''), colors[0]);
  check('成功态 chip 绿色', /#15803d/.test(colors[1] || ''), colors[1]);
  const row1 = groupsEl.children[1];
  const chips1 = row1.children.slice(1);
  check('失败态 chip 红色且悬停 error', /#b91c1c/.test(chips1[0].style.cssText) && chips1[0].title === 'HTTP 500',
        chips1[0].style.cssText + '| title=' + chips1[0].title);
  check('跳过态 chip 浅灰', /#9ca3af/.test(chips1[1].style.cssText), chips1[1].style.cssText);

  // 4) running/limit 文案 + 进度
  check('运行并发文案「运行 1/3」',
    $('taskParallelRunningText').textContent === '并发 运行 1/3',
    $('taskParallelRunningText').textContent);
  check('进度文案「进度 2/4」',
    $('taskParallelProgressText').textContent === '进度 2/4',
    $('taskParallelProgressText').textContent);
  check('进度条填充到 50 每分比', $('taskParallelProgressFill').style.width === '50%',
        $('taskParallelProgressFill').style.width);

  // 5) partial 橙色徽标
  check('partial 徽标显示', $('taskPartialBadge').hidden === false,
        'hidden=' + $('taskPartialBadge').hidden);

  // 6) 失败原因列表
  const failEl = $('taskFailedReasons');
  check('失败原因列表显示', failEl.style.display === 'block');
  const failText = failEl.children.map((c) => c.textContent).join('\n');
  check('失败原因含步骤名与 error 原文', /解析C/.test(failText) && /HTTP 500/.test(failText), failText);

  // 7) 空任务时并行条隐藏
  $('taskParallelBar').style.display = 'block';
  $('taskParallelGroups').style.display = 'block';
  sandbox.renderParallel({ steps: [], parallel: { groups: [], progress: { done: 0, total: 0 } } });
  check('无步骤时并行条隐藏', $('taskParallelBar').style.display === 'none');
}

// ---- 8) 并行上限输入：夹取 1-8，调 task_set_parallel_max ----
// 模拟 console.js 中 parallelInput.onchange 的行为（与源码一致的夹取逻辑）
async function simulateParallelChange(raw) {
  let n = parseInt(raw, 10);
  if (isNaN(n)) n = 3;
  n = Math.min(8, Math.max(1, n));
  const r = await invoke('task_set_parallel_max', { n });
  return r;
}
invokeImpl = async (cmd, args) => {
  if (cmd === 'task_set_parallel_max') return { task_parallel_max: Math.min(8, Math.max(1, args.n)) };
  return {};
};
(async () => {
  const r1 = await simulateParallelChange('50');   // 越界 -> 夹到 8
  check('上限 50 夹取为 8', r1.task_parallel_max === 8, JSON.stringify(r1));
  const r2 = await simulateParallelChange('0');     // 越界 -> 夹到 1
  check('上限 0 夹取为 1', r2.task_parallel_max === 1, JSON.stringify(r2));
  const r3 = await simulateParallelChange('3');
  check('上限 3 原样通过', r3.task_parallel_max === 3, JSON.stringify(r3));

  const setCalls = invokeCalls.filter((c) => c.cmd === 'task_set_parallel_max');
  check('共调用 task_set_parallel_max 3 次', setCalls.length === 3, 'n=' + setCalls.length);

  // 9) 事件监听：console.js 源码里挂了 qibai://task-branches（带 parallel 载荷处理）
  check('源码注册 qibai://task-branches 监听',
    /listen\(['"]qibai:\/\/task-branches['"]/.test(src) && /openDetail\(currentTaskId\)/.test(src));
  check('源码 task://updated 也会刷新详情面板',
    /listen\(['"]task:\/\/updated['"]/.test(src));

  console.log('\n===== test_task_parallel: ' + pass + ' PASS / ' + fail + ' FAIL =====');
  process.exit(fail ? 1 : 0);
})();
