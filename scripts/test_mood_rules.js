// 元气小小 v18.4 语气推断规则单测（node 直跑）
// 运行：node scripts/test_mood_rules.js
const assert = require('assert');
const mood = require('../src/mood-rules.js');
const infer = mood.inferMood;

const cases = [
  // [输入, 期望emotion, 期望action, 说明]
  ['哼，气死我了！', 'angry', 'shake', '生气'],
  ['你居然敢这样', 'angry', 'shake', '生气（居然敢）'],
  ['哈哈哈哈笑死我了', 'laugh', 'bounce', '大笑'],
  ['我今天好难过啊，呜呜', 'sad', 'pet', '难过'],
  ['抱抱，别怕有我在', 'sad', 'pet', '安慰'],
  ['哇！真的吗？', 'surprised', 'stretch', '惊讶'],
  ['这是怎么回事呢？', 'thinking', 'tilt', '疑问'],
  ['为什么呀？', 'thinking', 'tilt', '疑问'],
  ['我有点困了，想睡', 'sleepy', 'nod', '困倦'],
  ['晚安啦，你也早点休息', 'sleepy', 'nod', '晚安'],
  ['人家好害羞啦', 'blush', 'touch', '害羞'],
  ['今天好开心呀！', 'happy', 'wave', '开心'],
  ['你好呀～', 'happy', 'wave', '问候'],
  ['加油！你最棒了', 'happy', 'wave', '积极'],
  ['路上小心，注意安全', 'worried', 'lean', '关切'],
  ['这是一个比较长的陈述句，包含了很多信息，需要认真专注地讲清楚，所以应该用专注的表情和点头动作来配合', 'focused', 'nod', '长句专注'],
  ['嗯', 'smiling', 'default', '默认微笑'],
];

let pass = 0, fail = 0;
for (const [text, emo, act, note] of cases) {
  const r = infer(text);
  const ok = r.emotion === emo && r.action === act;
  if (ok) { pass++; }
  else {
    fail++;
    console.log('✗', note, '| 输入:', text, '| 期望', emo + '/' + act, '| 实际', r.emotion + '/' + r.action);
  }
}

// 枚举一致性：推断输出必须在 EMOTIONS/ACTIONS 内
for (const [text] of cases) {
  const r = infer(text);
  assert.ok(mood.EMOTIONS.includes(r.emotion), 'emotion 越界: ' + r.emotion);
  assert.ok(mood.ACTIONS.includes(r.action) || r.action === 'default', 'action 越界: ' + r.action);
}
pass++;

// v18.6 性格差异演出：同句话 → 不同形象不同表情/动作
const PERSONA_CASES = [
  ['gudian laugh弱化', '哈哈哈，真好笑', 'gudian', 'blush', 'touch'],
  ['gudian 强笑保留', '笑死我了哈哈哈哈', 'gudian', 'laugh', 'wave'],
  ['gudian 生气袖颤', '气死我了！', 'gudian', 'angry', 'shake'],
  ['dushi 弱开心收敛', '哦，这样啊', 'dushi', 'focused', 'nod'],
  ['dushi 强开心保留', '哈哈哈太棒了', 'dushi', 'laugh', 'nod'],
  ['dushi 惊讶克制', '天哪', 'dushi', 'surprised', 'tilt'],
  ['wenrou 难过安抚', '我好难过', 'wenrou', 'sad', 'pet'],
  ['wenrou 害羞轻触', '人家好害羞', 'wenrou', 'blush', 'touch'],
  ['huopo 生气跺脚', '气死了！', 'huopo', 'angry', 'bounce'],
  ['huopo 大笑蹦跳', '哈哈哈哈笑不活', 'huopo', 'laugh', 'bounce'],
  ['huopo 想你了', '我好想你', 'huopo', 'sad', 'pet'],
  ['reqing 开心庆祝', '耶！太好了', 'reqing', 'happy', 'celebrate'],
  ['reqing 思考前指', '这个问题怎么解决', 'reqing', 'thinking', 'point'],
  ['reqing 生气前指', '气死了！', 'reqing', 'angry', 'point'],
  ['gudian 晚安', '晚安', 'gudian', 'sleepy', 'nod'],
  ['无persona行为不变', '晚安', undefined, 'sleepy', 'nod'],
];
let personaPass = 0, personaFail = 0;
PERSONA_CASES.forEach(([name, text, persona, emo, act]) => {
  const r = mood.inferMood(text, persona);
  if (r.emotion === emo && r.action === act) personaPass++;
  else { personaFail++; console.log('✗ 性格·' + name + ' | 期望 ' + emo + '/' + act + ' | 实际 ' + r.emotion + '/' + r.action); }
});

console.log(`\n通过 ${pass}/${cases.length + 1}（语气基础）+ ${personaPass}/${PERSONA_CASES.length}（性格），失败 ${fail + personaFail}`);
process.exit(fail + personaFail === 0 ? 0 : 1);
