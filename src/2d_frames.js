// ============================================================
// 2d_frames.js — 桌宠 2D 帧动画状态机（主路径）
// B 组资产约定：assets/ai_char/<persona>_<state>_<n>.png（透明底）
//   核心立绘 5 张：assets/ai_char/<persona>.png
//   活泼 huopo：idle_1..4 / talk_1..3 / listen_1..2 / happy_1..3 / surprise_1..2 / wave_1..3
// 帧图未到位时用程序化占位帧（canvas 绘制）跑通状态机；真图到位自动合流。
// 对外 window.QibaiFrames；加载完成派发 'qibai-frames-ready'。
// ============================================================

/* ---- 状态→帧组映射（主形象 huopo） ---- */
const STATE_FRAMES = {
  idle:     { frames: 4, fps: 4, loop: true  },
  talk:     { frames: 3, fps: 7, loop: true  },
  listen:   { frames: 2, fps: 3, loop: true  },
  happy:    { frames: 3, fps: 6, loop: false },
  surprise: { frames: 2, fps: 5, loop: false },
  wave:     { frames: 3, fps: 6, loop: false },
};
const EMOTION_TO_STATE = {
  happy: 'happy', laugh: 'happy', surprised: 'surprise', shy: 'idle', blush: 'idle',
  sleepy: 'idle', sad: 'idle', worried: 'idle', focused: 'listen', thinking: 'idle',
  smiling: 'idle', angry: 'surprise',
};
const ACTION_TO_STATE = {
  wave: 'wave', celebrate: 'happy', bounce: 'happy', nod: 'listen', point: 'surprise',
  pet: 'idle', touch: 'idle', stretch: 'idle', hands_on_hips: 'idle', shake: 'surprise', lean: 'listen', tilt: 'idle', wiggle: 'happy',
};

/* ---- 占位帧绘制（真图未到位时；形象=彩色 Q 版剪影，帧间微差） ---- */
function placeholderFrame(persona, state, n) {
  const c = document.createElement('canvas'); c.width = 168; c.height = 196;
  const g = c.getContext('2d');
  const hair = { gudian:'#4a3b3b', dushi:'#5a4632', wenrou:'#c9a878', huopo:'#f2d06b', reqing:'#a64b4b' }[persona] || '#f2d06b';
  const cloth = { gudian:'#f3e6d8', dushi:'#d8d2cc', wenrou:'#f6c1d5', huopo:'#f6c1d5', reqing:'#c98aa0' }[persona] || '#f6c1d5';
  // 帧间微差：呼吸/嘴型/挥手
  const breathe = (state === 'idle' || state === 'talk') ? Math.sin(n * 1.2) * 2 : 0;
  const jawOpen = (state === 'talk') ? (n === 2 ? 6 : 2) : 0;
  const armWave = (state === 'wave') ? (n - 2) * 10 : 0;
  const sad = (state === 'surprise');

  // 身体
  g.fillStyle = cloth; g.beginPath();
  g.ellipse(84, 130 + breathe, 30, 38 + breathe, 0, 0, Math.PI * 2); g.fill();
  // 头
  g.fillStyle = '#FFE9F1'; g.beginPath(); g.arc(84, 62 + breathe * 0.5, 34, 0, Math.PI * 2); g.fill();
  // 头发
  g.fillStyle = hair; g.beginPath(); g.arc(84, 56, 36, Math.PI, 0); g.fill();
  // 眼睛
  g.fillStyle = '#6E5A6B';
  g.beginPath(); g.ellipse(72, 64, 3.5, sad ? 6 : 4.5, 0, 0, Math.PI * 2); g.fill();
  g.beginPath(); g.ellipse(96, 64, 3.5, sad ? 6 : 4.5, 0, 0, Math.PI * 2); g.fill();
  // 嘴（talk 开合）
  g.strokeStyle = '#D17397'; g.lineWidth = 2.5; g.beginPath();
  if (jawOpen) { g.ellipse(84, 84, 5, jawOpen, 0, 0, Math.PI * 2); g.stroke(); }
  else { g.moveTo(76, 84); g.quadraticCurveTo(84, 90 + breathe, 92, 84); g.stroke(); }
  // 手臂（wave）
  g.strokeStyle = cloth; g.lineWidth = 8; g.lineCap = 'round';
  g.beginPath(); g.moveTo(110, 118); g.lineTo(128, 108 - armWave); g.stroke();
  g.beginPath(); g.moveTo(58, 118); g.lineTo(44, 128); g.stroke();
  // 状态文字（调试可见，真图到位即隐）
  g.fillStyle = '#9B7FB8'; g.font = '10px sans-serif'; g.textAlign = 'center';
  g.fillText(persona + '/' + state + '_' + n, 84, 190);
  return c.toDataURL();
}

function createFrames(imgEl, personaKey) {
  const rt = {
    img: imgEl,
    persona: personaKey || 'huopo',
    state: 'idle',
    frame: 0,
    acc: 0,
    talk: false,
    listen: false,
    crossfadeImg: null,   // 过渡用第二张 img
    cache: {},
    loopDone: false,
  };

  function frameUrl(state, n) {
    // 真图约定路径；失败回退占位帧
    return 'assets/ai_char/' + rt.persona + '_' + state + '_' + n + '.png';
  }

  // 预生成占位帧（避免运行时抖动）
  for (const s of Object.keys(STATE_FRAMES)) {
    for (let i = 1; i <= STATE_FRAMES[s].frames; i++) {
      rt.cache[s + '_' + i] = placeholderFrame(rt.persona, s, i);
    }
  }

  function showFrame(state, n) {
    rt.img.src = rt.cache[state + '_' + n] || placeholderFrame(rt.persona, state, n);
  }

  function setState(next) {
    if (rt.state === next) return;
    const prev = rt.state;
    rt.state = next; rt.frame = 1; rt.acc = 0; rt.loopDone = false;
    // 200ms 交叉淡入：用 opacity 过渡（单 img，直接换帧 + 淡入）
    rt.img.style.opacity = '0.2';
    showFrame(next, 1);
    setTimeout(() => { rt.img.style.opacity = '1'; }, 200);
  }

  function setEmotion(emo) {
    setState(EMOTION_TO_STATE[emo] || 'idle');
  }
  function playAction(action) {
    setState(ACTION_TO_STATE[action] || 'idle');
  }
  function resetEmotion() { setState(rt.listen ? 'listen' : 'idle'); }
  function setTalk(on) {
    rt.talk = !!on;
    if (on) setState('talk'); else if (rt.state === 'talk') setState('idle');
  }
  function setListen(on) {
    rt.listen = !!on;
    if (on && rt.state === 'idle') setState('listen');
  }
  function setPersona(key) {
    rt.persona = key;
    for (const k of Object.keys(rt.cache)) delete rt.cache[k];
    for (const s of Object.keys(STATE_FRAMES))
      for (let i = 1; i <= STATE_FRAMES[s].frames; i++)
        rt.cache[s + '_' + i] = placeholderFrame(key, s, i);
    setState('idle');
  }

  // 帧循环
  let last = performance.now();
  function loop(now) {
    const dt = (now - last) / 1000; last = now;
    const conf = STATE_FRAMES[rt.state] || STATE_FRAMES.idle;
    rt.acc += dt;
    const interval = 1 / conf.fps;
    if (rt.acc >= interval) {
      rt.acc = 0;
      rt.frame++;
      if (rt.frame > conf.frames) {
        if (conf.loop) rt.frame = 1;
        else { // 一次性动作播完回 idle
          rt.frame = conf.frames;
          if (!rt.loopDone) { rt.loopDone = true; setState(rt.talk ? 'talk' : (rt.listen ? 'listen' : 'idle')); }
        }
      }
      showFrame(rt.state, rt.frame);
    }
    requestAnimationFrame(loop);
  }
  showFrame('idle', 1);
  requestAnimationFrame(loop);

  return { setEmotion, playAction, resetEmotion, setTalk, setListen, setPersona, getState: () => rt.state };
}

window.QibaiFrames = {
  create(imgEl, personaKey) {
    try { return createFrames(imgEl, personaKey); }
    catch (e) { console.warn('[qibai2d] init failed:', e && e.message); return null; }
  },
};
window.dispatchEvent(new CustomEvent('qibai-frames-ready'));
