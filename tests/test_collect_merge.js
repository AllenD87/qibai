// 真跑 collect()：用 stub 替换 $() / currentSettings，断言产物字段
const fs = require('fs');
const path = require('path');
const src = fs.readFileSync(path.join(__dirname, '../src/console.js'), 'utf8');

// 提取 function collect() { ... } 块
const m = src.match(/function collect\(\) \{[\s\S]*?\n  \}/);
if (!m) { console.error('FAIL: collect() not found'); process.exit(1); }

// 构造 stub
const state = {
  currentSettings: {
    audio_output: 'alsa_output.usb-x',
    search_engine: 'baidu',
    context_budget_tokens: 8000,
    llm: { provider: 'doubao', base_url: 'https://example.com', model: 'x', api_key: 'key-旧' },
    voice: { wake_word: '小小旧' },
    skills: { check_code: true },
  },
};

// stub $()：返回 { value: ..., dataset: {on:'true'} }
const fakeValues = {
  setProvider: 'doubao', setBaseUrl: '', setModel: 'new-model', setApiKey: 'new-key',
  setSitMin: '60', setWaterMin: '40', setEyeMin: '45',
  setGuardSitMin: '120', setGuardCountdown: '120',
  setVoiceVolume: '80', setVoiceRate: '1.0',
  setSttDevice: 'hw:0', setWakeWord: '小小旧',
  audioOutputSelect: 'alsa_output.usb-x',  // 保持原设备
  searchEngineSelect: 'baidu',
  ctxBudgetTokens: '12000',
  ctxRecentRounds: '15',
  taskMaxSteps: '30',
  taskMaxMinutes: '45',
};
global.$ = (id) => ({
  get value() { return fakeValues[id] !== undefined ? fakeValues[id] : ''; },
  dataset: { on: 'true' },
});
global.currentSettings = state.currentSettings;

// eval collect
eval(m[0]);
const out = collect();

let pass = 0, fail = 0;
const check = (name, cond) => { console.log((cond?'PASS':'FAIL')+' | '+name); cond?pass++:fail++; };

// 1. 未在 UI 改的字段保留
check('audio_output 保留原值 alsa_output.usb-x', out.audio_output === 'alsa_output.usb-x');
check('search_engine 保留 baidu', out.search_engine === 'baidu');
check('context_budget_tokens 从 UI 覆盖为 12000', out.context_budget_tokens === 12000);
check('task_max_steps 从 UI 覆盖为 30', out.task_max_steps === 30);
check('voice.wake_word 原样保留（UI 未改）', out.voice.wake_word === '小小旧');
check('skills.check_code 原样保留', out.skills.check_code === true);
check('llm.api_key 从 UI 覆盖为 new-key', out.llm.api_key === 'new-key');

// 2. 无 null 值（RFC 7386 null = 删除键）
function findNull(obj, path) {
  for (const k of Object.keys(obj)) {
    if (obj[k] === null) return path + '.' + k;
    if (typeof obj[k] === 'object' && obj[k] !== null) {
      const r = findNull(obj[k], path + '.' + k);
      if (r) return r;
    }
  }
  return null;
}
const nullPath = findNull(out, 'out');
check('产物里没有 null 值（RFC 7386 不会误删键）', nullPath === null);

// 3. audio_output 永远是字符串（"系统默认" = "default"，不是 null）
check('audio_output 是字符串', typeof out.audio_output === 'string');

// 4. 夹取
fakeValues.ctxBudgetTokens = '999999';
const out2 = collect();
check('context_budget_tokens 夹取到 64000', out2.context_budget_tokens === 64000);

console.log(`\n===== collect 真跑 QA: ${pass} PASS / ${fail} FAIL =====`);
process.exit(fail ? 1 : 0);
