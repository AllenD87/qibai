/* v45.21「可打断」链路契约测试（静态契约，不启 DOM）
 * 目的：后端命令名 ↔ 前端 invoke 名 ↔ UI 元素三者必须一致，
 * 且「运行中不能把唯一的停止入口禁用/丢掉」。任何一条被改坏，用户就又回到
 * 「不知道能不能打断、按了没反应」的状态，这里直接红灯。
 */
const fs = require('fs');
const path = require('path');
const root = path.join(__dirname, '..');
const read = (p) => fs.readFileSync(path.join(root, p), 'utf8');

let pass = 0, fail = 0;
function check(name, cond, extra) {
  if (cond) { pass++; console.log('  PASS ' + name); }
  else { fail++; console.log('  FAIL ' + name + (extra ? ' → ' + extra : '')); }
}

const main = read('src-tauri/src/main.rs');
const consoleJs = read('src/console.js');
const consoleHtml = read('src/console.html');
const petJs = read('src/pet.js');
const petCss = read('src/pet.css');
const llm = read('qibai-core/src/llm_client.rs');
const engine = read('qibai-core/src/engine.rs');

console.log('== 1. 命令存在且已注册 ==');
check('main.rs 定义 stop_message', /fn stop_message\(/.test(main));
check('main.rs 定义 stop_state', /fn stop_state\(/.test(main));
check('generate_handler 注册 stop_message', /stop_message,/.test(main));
check('generate_handler 注册 stop_state', /stop_state,/.test(main));
check('AppState 持锁外取消标志', /cancel: Arc<AtomicBool>/.test(main));
check('send_message 开轮清零标志', /state\.cancel\.store\(false/.test(main));

console.log('== 2. 引擎侧取消点 ==');
check('LlmError 有 Cancelled 变体', /enum LlmError/.test(llm) && /Cancelled/.test(llm));
check('ToolExecutor 有 cancelled() 默认方法', /fn cancelled\(&self\) -> bool/.test(llm));
check('轮间检查点存在', /if executor\.cancelled\(\)/.test(llm));
check('engine 有可取消入口', /fn send_message_cancellable/.test(engine));
check('取消按正常收尾（不是报错）', /LlmError::Cancelled\) =>/.test(engine));

console.log('== 3. 控制台停止入口 ==');
check('console.html 有 stopBtn', /id="stopBtn"/.test(consoleHtml));
check('console.js 取到 stopBtn', /getElementById\('stopBtn'\)/.test(consoleJs));
check('console.js 点击调用 stop_message', /invoke\('stop_message'\)/.test(consoleJs));
check('发送时显示停止按钮', /setRunning\(true\)/.test(consoleJs));
check('结束后隐藏停止按钮', /setRunning\(false\)/.test(consoleJs));
check('运行中输「停」也走叫停', /sendBusy && \/[\s\S]{0,80}停止/.test(consoleJs));
check('停止后卡片标题标「已停止」', /已停止 · 停在第/.test(consoleJs));
check('工具名语义化映射', /TOOL_LABELS/.test(consoleJs));

console.log('== 4. 桌宠停止入口 ==');
check('pet.js 发送中调用 stop_message', /invoke\('stop_message'\)/.test(petJs));
check('按钮两副面孔（点击按 state.sending 分流）', /if \(state\.sending\) \{ stopCurrentTurn\(\); return; \}/.test(petJs));
check('运行中不禁用按钮（否则断掉唯一打断路径）', /if \(state\.sending\) \{ btnSend\.disabled = false/.test(petJs));
check('运行中图标换停止方块', /'■'/.test(petJs));
check('CSS 有 .is-stop 红色态', /\.send-btn\.is-stop/.test(petCss));
check('输入「停」拦截为叫停', /停一下\|停止\|停下/.test(petJs));
check('气泡有 完成/失败/待确认 三态', /✓ 第|✗ 第|❓/.test(petJs));

console.log('\n结果: ' + pass + ' passed, ' + fail + ' failed');
process.exit(fail ? 1 : 0);
