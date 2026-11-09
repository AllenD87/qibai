/* ============================================================
 * 元气小小 Qibai — 桌宠窗逻辑（pet.js）v21.11.14
 * 单角色独立窗（pet-<persona_id>）：
 *   形象注册表 / 表情系统 / 动作系统 / 眨眼 / 说话气泡 /
 *   单击摸头·双击戳脸 / 右键菜单 / 事件广播订阅（按 persona 过滤）
 * 不承载任何功能页面 UI（聊天/设置/PK overlay/事件面板/技能弹窗均在控制台窗）
 * ============================================================ */

(function () {
  'use strict';

  const Q = window.QibaiBridge;
  const { invoke, listen, isTauri: bridgeTauri } = Q;
  // v16 兜底：旧版 tauri-bridge.js 可能不导出 isTauri——一旦缺失，下方所有 isTauri 分支
  // （Tauri 禁动画、tauri-mode 类、PTT 路径）全是死代码，重影修复/PTT 修复全部失效。
  // 直接探测 window 上的 Tauri 运行时，不依赖桥版本。
  const isTauri = !!bridgeTauri ||
    !!(window.__TAURI__ && window.__TAURI__.invoke) ||
    !!window.__TAURI_INTERNALS__ ||
    /tauri=1/.test(location.search); // 仅本地冒烟注入（验证 canvas 整帧渲染路径）

  /* ---------- DOM 引用 ---------- */
  const $ = (id) => document.getElementById(id);
  const stage = $('stage');
  const wrap = $('characterWrap');
  const img = $('characterImg');
  const face = $('placeholderFace');
  const bubble = $('speechBubble');
  const speechText = $('speechText');
  // v45 气泡单实例硬约束：DOM 里 #speechBubble 必须恰好 1 个（重复创建直接报错，
  // 从结构上排除「两层气泡」的多实例成因）。
  if (document.querySelectorAll('#speechBubble').length !== 1) {
    console.error('[qibai] FATAL: #speechBubble 实例数 != 1 =', document.querySelectorAll('#speechBubble').length);
  }
  const btnSpeak = $('btnSpeak');
  const dragBar = $('dragBar');
  const lockBadge = $('lockBadge');
  const pkCard = $('pkCard');
  const pkHandIcon = $('pkHandIcon');
  const pkResultTag = $('pkResultTag');
  const reminderOverlay = $('reminderOverlay');
  const reminderMsg = $('reminderMsg');
  const reminderTag = $('reminderTag');
  const reminderClose = $('reminderClose');
  const toastBox = $('toastBox');
  const fxLayer = $('fxLayer');
  const ctxMenu = $('ctxMenu');
  const ctxConsole = $('ctxConsole');
  const ctxLock = $('ctxLock');
  const ctxLockText = $('ctxLockText');
  const ctxGuard = $('ctxGuard');
  const ctxExit = $('ctxExit');
  const ctxMinimal = $('ctxMinimal');
  const ctxMinimalText = $('ctxMinimalText');
  const voiceTagChip = $('voiceTagChip'); // 迭代⑤：本窗角色音色标签
  // v18 UI 面板·气泡卡片
  const uiPanel = $('uiPanel');
  const uiPanelTitle = $('uiPanelTitle');
  const uiPanelBody = $('uiPanelBody');
  const uiPanelExpand = $('uiPanelExpand');
  const uiPanelClose = $('uiPanelClose');
  // F3：气泡时间戳 / Thinking 态
  const speechTime = $('speechTime');
  // F4：心情/关系徽章
  const moodBadge = $('moodBadge');
  const moodEmoji = $('moodEmoji');
  const moodText = $('moodText');
  // D3：聊天输入条三件套（显式声明，避免依赖 id 全局量；btnMic 与 pet.html:84 的 id 对齐）
  const chatInput = $('chatInput');
  const btnSend = $('btnSend');
  const btnMic = $('btnMic');
  // D8：右侧快捷图标簇
  const btnUser = $('btnUser');
  const btnMute = $('btnMute');
  const btnMore = $('btnMore');

  /* ---------- 常量 ---------- */
  const EMOTIONS = ['smiling', 'happy', 'worried', 'sleepy', 'focused', 'sad', 'shy', 'surprised', 'angry', 'laugh', 'thinking', 'blush'];
  const ACTIONS = ['idle', 'wave', 'nod', 'shake', 'stretch', 'hands_on_hips', 'touch', 'pet', 'celebrate', 'bounce', 'tilt', 'wiggle', 'lean', 'point'];
  const REMINDER_TAGS = { sit: '久坐提醒', water: '喝水提醒', eye: '护眼提醒' };
  const REMINDER_MESSAGES = {
    sit: '坐久啦，起来走两步活动一下吧～',
    water: '该喝水啦，记得补充水分哦～',
    eye: '看看远处的绿色，让眼睛休息一下吧～',
  };

  // 形象注册表（5 个形象；资产位于 assets/personas/<key>/，默认古典佳人 gudian）
  const PERSONAS = {
    gudian: {
      name: '古典佳人',
      desc: '墨黑长发·月白汉服·温婉灵动',
      speak_actions: ['nod', 'wave', 'touch'],  // 温婉：点头/招手/轻触

      base: 'assets/personas/gudian/character.png',
      closed: 'assets/personas/gudian/character_closed.png',
      expr: (e) => `assets/personas/gudian/expressions/${e}.png`,
      poses: {
        wave: 'assets/personas/gudian/poses/wave.png',
        nod: 'assets/personas/gudian/poses/nod.png',
        shake: 'assets/personas/gudian/poses/shake.png',
        stretch: 'assets/personas/gudian/poses/stretch.png',
        hands_on_hips: 'assets/personas/gudian/poses/hands_on_hips.png',
        celebrate: 'assets/personas/gudian/poses/celebrate.png',
        touch: 'assets/personas/gudian/poses/touch.png',
        point: 'assets/personas/gudian/poses/point.png',
      },
    },
    dushi: {
      name: '都市女性',
      desc: '藏青西装·知性自信',
      speak_actions: ['nod', 'hands_on_hips', 'stretch'],  // 知性：点头/叉腰/伸展

      base: 'assets/personas/dushi/character.png',
      closed: 'assets/personas/dushi/character_closed.png',
      expr: (e) => `assets/personas/dushi/expressions/${e}.png`,
      poses: {
        wave: 'assets/personas/dushi/poses/wave.png',
        nod: 'assets/personas/dushi/poses/nod.png',
        shake: 'assets/personas/dushi/poses/shake.png',
        stretch: 'assets/personas/dushi/poses/stretch.png',
        hands_on_hips: 'assets/personas/dushi/poses/hands_on_hips.png',
        celebrate: 'assets/personas/dushi/poses/celebrate.png',
        touch: 'assets/personas/dushi/poses/touch.png',
        point: 'assets/personas/dushi/poses/point.png',
      },
    },
    wenrou: {
      name: '温柔',
      desc: '米白针织·温暖恬静',
      speak_actions: ['nod', 'pet', 'touch'],  // 恬静：点头/轻抚/轻触

      base: 'assets/personas/wenrou/character.png',
      closed: 'assets/personas/wenrou/character_closed.png',
      expr: (e) => `assets/personas/wenrou/expressions/${e}.png`,
      poses: {
        wave: 'assets/personas/wenrou/poses/wave.png',
        nod: 'assets/personas/wenrou/poses/nod.png',
        shake: 'assets/personas/wenrou/poses/shake.png',
        stretch: 'assets/personas/wenrou/poses/stretch.png',
        hands_on_hips: 'assets/personas/wenrou/poses/hands_on_hips.png',
        celebrate: 'assets/personas/wenrou/poses/celebrate.png',
        touch: 'assets/personas/wenrou/poses/touch.png',
        point: 'assets/personas/wenrou/poses/point.png',
      },
    },
    huopo: {
      name: '活泼',
      desc: '双马尾·元气灵动',
      speak_actions: ['wave', 'celebrate', 'shake'],  // 元气：招手/庆祝/摆动

      base: 'assets/personas/huopo/character.png',
      closed: 'assets/personas/huopo/character_closed.png',
      expr: (e) => `assets/personas/huopo/expressions/${e}.png`,
      poses: {
        wave: 'assets/personas/huopo/poses/wave.png',
        nod: 'assets/personas/huopo/poses/nod.png',
        shake: 'assets/personas/huopo/poses/shake.png',
        stretch: 'assets/personas/huopo/poses/stretch.png',
        hands_on_hips: 'assets/personas/huopo/poses/hands_on_hips.png',
        celebrate: 'assets/personas/huopo/poses/celebrate.png',
        touch: 'assets/personas/huopo/poses/touch.png',
        point: 'assets/personas/huopo/poses/point.png',
      },
    },
    reqing: {
      name: '热情时尚',
      desc: '驼色风衣·优雅名媛',
      speak_actions: ['wave', 'celebrate', 'touch'],  // 热情：招手/庆祝/轻触

      base: 'assets/personas/reqing/character.png',
      closed: 'assets/personas/reqing/character_closed.png',
      expr: (e) => `assets/personas/reqing/expressions/${e}.png`,
      poses: {
        wave: 'assets/personas/reqing/poses/wave.png',
        nod: 'assets/personas/reqing/poses/nod.png',
        shake: 'assets/personas/reqing/poses/shake.png',
        stretch: 'assets/personas/reqing/poses/stretch.png',
        hands_on_hips: 'assets/personas/reqing/poses/hands_on_hips.png',
        celebrate: 'assets/personas/reqing/poses/celebrate.png',
        touch: 'assets/personas/reqing/poses/touch.png',
        point: 'assets/personas/reqing/poses/point.png',
      },
    },
  };
  const DEFAULT_PERSONA = 'gudian';

  // 撒娇语（单击摸头互动）
  const PET_LINES = ['诶嘿嘿，好舒服～', '再摸一下也可以哦。', '唔……有点害羞啦。', '你的手暖暖的呢。'];

  // PK 手势 SVG（复用既有视觉语言：粉/紫）
  const MOVE_SVG = {
    rock: '<svg viewBox="0 0 24 24"><rect x="6" y="7" width="12" height="12" rx="5" fill="#D17397"/></svg>',
    scissors: '<svg viewBox="0 0 24 24"><rect x="7" y="5" width="4" height="12" rx="2" fill="#9B7FB8"/><rect x="13" y="5" width="4" height="12" rx="2" fill="#9B7FB8"/></svg>',
    paper: '<svg viewBox="0 0 24 24"><rect x="5" y="4" width="14" height="16" rx="4" fill="#D17397"/></svg>',
  };

  /* ---------- 运行时状态 ---------- */
  const state = {
    persona: DEFAULT_PERSONA, // 当前形象 key（由窗口身份 pet-<id> 决定）
    emotion: null,
    assetReady: false,
    locked: false,           // 位置锁定（get_desk_state 的 desk.locked[本窗 persona]）
    transparentBg: false,    // v31.1：立绘背景透明模式（settings.ui.transparent_bg，默认关）
    transparencySupported: true, // v42：X11 软渲染无合成路径自动降级，避免假开启与叠影
    pkPerforming: false,     // PK 演出态
    actionTimer: null,
    bubbleState: 'idle',      // v45 气泡状态机：idle/showing/visible，同一时刻只一个可见态
    speechTimer: null,
    bubbleSticky: false,   // v45：长句气泡不自动收，留到下一句或手动收
    speakingTimer: null,     // 迭代②：tts-speak 脉冲 class 自动移除定时器
    lastSpokenText: '',      // 迭代②：最近一次气泡/提醒文本（喇叭按钮重读用）
    blinkTimer: null,
    blinkPhase: false,
    prevEmotionOnThinking: null,
    recording: false,        // 语音输入：按住说话录音态
    mediaRec: null,
    recChunks: [],
    quietMode: false,        // D8：全局静音开关（前端本地，关闭朗读）
    showVoiceTag: false,    // 二次修复①：音色状态 chip 默认隐藏，仅设置开启才显示
    frames: null,           // 2D 帧动画引擎（备用；QibaiFrames.create 后挂载）
    use3d: false,           // Three.js 层保留作回退/未来启用，默认关
    renderMode: '2d',       // 任务八：默认 2D 原图立绘（生成式 GLB 未就绪前不展示低模）；'3d' 为预览
    windowFocused: true,     // F5：窗口聚焦态（聚焦时静音、失焦才响）
    sending: false,          // 任务八：发送中防重入
    staticArt: false,        // 任务八改道：当前是否为 3D 风格/原图静态立绘（不切表情帧）
    artSrc: '',              // 任务八改道：实际命中的立绘路径
    lastInteractionTs: 0,    // F2：最近一次交互时间
    recentClicks: [],        // F2：高频点击时间戳
    envPhase: 'idle',        // F2：环境表情阶段
    awaitExitConfirm: false,  // v21：退出二次确认态（先「退出」→ 待确认；再「确认退出」才真退）
    swapToken: 0,             // v21.11.10：透明窗安全换图令牌——连续换图（快速切换动作/眨眼/恢复帧）
                              // 时旧换图链若晚于新链完成，会把窗口 src 覆盖回旧帧 → 无合成器下残留双像。
                              // 每次 setCharacterImg 自增令牌，只有最新令牌才允许真正落图，旧链直接丢弃。
    phase: 'idle',             // v29 角色渲染状态机：'idle' | 'acting' | 'recovering'。
                              // idle：显示 base/表情帧；acting：显示动作帧；recovering：动作结束恢复 base 过渡态。
                              // 任何帧切换（base/表情/动作/启动/恢复）都必须经 showCharacterFrame()，
                              // 从结构上保证「任意时刻透明窗内只存在一张形象」。
    bootWaveTries: 0,         // v21.11.14：开场欢迎动作轮询计数——立绘未就绪前不播动作，
                              // 避免与立绘加载链并发换图（启动「至少两个动作重合」根因之一）。
  };

  // v29 日志锚点：前端 → console + Tauri debug_log（真机 journalctl 可查）。
  // 真机排查时 grep "[qibai] frame:" 或 "[qibai] action:" 即可还原换帧时序。
  function logFrame(tag, detail) {
    const line = '[qibai] ' + tag + ' ' + (typeof detail === 'string' ? detail : JSON.stringify(detail || {}));
    try { console.log(line); } catch (e) { /* ignore */ }
    try { invoke('debug_log', { msg: line }).catch(() => {}); } catch (e) { /* bridge 缺失 */ }
  }

  /* F2：环境表情状态机阈值常量（便于调参） */
  const ENV = {
    lazyAfterMs: 3 * 60 * 1000,   // 久置 N 分钟无交互 → 慵懒
    exciteClicks: 5,              // 短时多次点击 → 兴奋
    exciteWindowMs: 8000,
    checkMs: 5000,
  };
  const REL_NAMES = ['陌生', '眼熟', '合拍', '默契', '亲密', '知己'];

  function hhmm() {
    const d = new Date();
    return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
  }

  /* ============================================================
   * 渲染显示策略（任务八）：
   *  默认 = 高质量 2D 原图立绘（#characterImg）——生成式 GLB 未就绪前不展示低模占位。
   *  优先级：真 GLB 3D（renderMode='3d'）> 2D 原图立绘（默认）> 程序化占位/低模。
   *  3D 层（Qibai3D）与 2D 帧（QibaiFrames）代码保留，经 __qibaiDebug.setRenderMode 预览。
   * ============================================================ */
  function bootRenderEngine() {
    if (state.engine) return;
    if (state.renderMode !== '3d') return; // 默认 2D 立绘：不挂载任何引擎，#characterImg 原生显示
    try {
      if (window.Qibai3D && Qibai3D.detect()) {
        const gl = document.getElementById('glCanvas');
        if (gl) {
          const eng = Qibai3D.create(gl);
          if (eng) {
            eng.applyPalette(state.persona || DEFAULT_PERSONA);
            state.engine = eng;
            state.use3d = true;
            gl.classList.remove('hidden');
            document.body.classList.add('gl-on');
            console.log('[qibai] 3D 预览引擎已挂载 →', state.persona);
          }
        }
      }
    } catch (e) { console.warn('[qibai] 3D 初始化失败，回退 2D 立绘：', e && e.message); }
  }
  window.addEventListener('qibai-3d-ready', bootRenderEngine);
  window.addEventListener('qibai-frames-ready', bootRenderEngine);
  setTimeout(bootRenderEngine, 0);

  // 运行时切换渲染模式（2d=原生立绘 / 3d=三渲二预览）
  function setRenderMode(mode) {
    state.renderMode = (mode === '3d') ? '3d' : '2d';
    if (state.renderMode === '3d') {
      bootRenderEngine();
    } else {
      if (state.engine && state.engine.dispose) state.engine.dispose();
      state.engine = null; state.use3d = false;
      const gl = document.getElementById('glCanvas');
      if (gl) gl.classList.add('hidden');
      document.body.classList.remove('gl-on');
    }
  }

  // pk-round 本地逐条演出的台词去重表（text → 过期时间戳）：
  // 后端/mock 会同时把同一批 lines 逐条广播 qibai://pet-say，
  // 本地已计划演出的文本在过期前忽略重复广播，避免气泡双演。
  const plannedLines = new Map();

  /* ============================================================
   * 立绘加载 / 占位脸
   * ============================================================ */

  img.addEventListener('error', () => {
    state.assetReady = false;
    img.classList.add('img-missing');
    stage.classList.remove('asset-ready');
  });
  img.addEventListener('load', () => {
    state.assetReady = true;
    img.classList.remove('img-missing');
    stage.classList.add('asset-ready');
  });
  // 缓存命中时 load 事件可能已错过：立即自检
  if (img.complete && img.naturalWidth > 0) {
    state.assetReady = true;
    img.classList.remove('img-missing');
    stage.classList.add('asset-ready');
  }

  function personaAssets() {
    return PERSONAS[state.persona] || PERSONAS[DEFAULT_PERSONA];
  }

  // ============================================================
  // v30 渲染层重构：canvas 整帧重绘器（Tauri 模式根治残留）
  // ------------------------------------------------------------
  // 根因（v29.1/v29.2 真机定界）：无合成器 X11 下 WebKit 增量渲染只在
  // alpha>0 像素处落笔，普通重绘请求不重写透明区旧像素 → DOM img 换帧/
  // 气泡显隐/窗口拖动 都会残留旧帧（「动作重合/重影/拖动残留色块/语音提示
  // 身后重影」全部同源）。拖动时残留消失 = 真实交互触发整页重渲染。
  // v30 方案（行业 Desktop Pet 标准：整帧替换 vs 逐帧累加——选整帧）：
  //   全窗 canvas 每帧画「不透明背景 + 当前角色帧」整块覆盖窗口旧像素；
  //   任何换帧 / DOM 变化（MutationObserver）都触发整帧重绘。
  // ============================================================
  // v31：canvas 背景改为极浅近白（用户反馈「仍有有颜色的背景」）。
  // 无合成器 X11 透明窗必须不透明覆盖才能根治重影（已确认有效），
  // 因此把颜色压到肉眼接近无感的暖白，窗口不再是明显色块。
  const WINDOW_BG_TOP = '#FFFDFB';   // 渐变端点①（暖白）
  const WINDOW_BG_BOTTOM = '#F7F3FA'; // 渐变端点②（微紫白）
  // v42：舞台顶部已经由 CSS 完整避开气泡区，不再从角色高度里重复扣除 62px。
  const BUBBLE_RESERVE = 0;
  const frameCache = new Map(); // src → Image（v30 帧缓存）
  let bgCanvas = null, bgCtx = null, moTimer = null, moObs = null;
  let petLayoutObserver = null;

  function syncPetLayout() {
    if (!isTauri) return;
    const chips = document.getElementById('quickChips');
    const bar = document.getElementById('chatBar');
    const H = window.innerHeight || 330;
    const chipsRect = chips ? chips.getBoundingClientRect() : null;
    const barRect = bar ? bar.getBoundingClientRect() : null;
    const controlsTop = chipsRect && chipsRect.height > 0
      ? chipsRect.top
      : (barRect ? barRect.top : H - 48);
    const stageBottom = Math.max(52, Math.ceil(H - controlsTop + 6));
    // v43：默认极简态已隐藏快捷胶囊，舞台下缘直接贴输入条；上缘固定留出
    // 78px 气泡区，角色按剩余高度最大化（不再被两行胶囊压成小图）。
    const baseTop = H <= 280 ? 58 : 90;
    const stageTop = Math.max(46, Math.min(baseTop, H - stageBottom - 56));
    document.body.style.setProperty('--stage-bottom', stageBottom + 'px');
    document.body.style.setProperty('--stage-top', stageTop + 'px');
    repaintFrame();
  }

  function armResponsiveLayout() {
    if (!isTauri) return;
    syncPetLayout();
    window.addEventListener('resize', syncPetLayout);
    if (typeof ResizeObserver !== 'undefined' && !petLayoutObserver) {
      petLayoutObserver = new ResizeObserver(syncPetLayout);
      ['quickChips', 'chatBar'].forEach((id) => {
        const el = document.getElementById(id);
        if (el) petLayoutObserver.observe(el);
      });
    }
  }

  function ensureCanvas() {
    if (bgCanvas && bgCanvas.isConnected) return;
    bgCanvas = document.getElementById('bgCanvas');
    bgCtx = bgCanvas ? bgCanvas.getContext('2d') : null;
  }

  // v41：整窗「硬」重绘——v45.6 起只走 web 侧 canvas 整帧重绘（原生 X 表面操作已整条删除）。
  // 透明模式下 canvas 的 clearRect 只把像素变透明，WebKit 不会重写窗口透明区，
  // 旧动作帧/旧气泡就留在窗口表面形成「重影/一圈圈」。防抖 60ms，避免一帧多呼。
  let hardRepaintTimer = null;
  let hardRepaintWaiters = [];
  function hardRepaint(delay = 60) {
    if (!isTauri) return Promise.resolve();
    const p = new Promise((res) => hardRepaintWaiters.push(res));
    if (hardRepaintTimer) clearTimeout(hardRepaintTimer);
    hardRepaintTimer = setTimeout(() => {
      hardRepaintTimer = null;
      // v45.6：不再 invoke 原生表面命令（已整条删除，避免 BadDrawable）。
      // 直接走 web 侧整帧重绘即可。
      try { repaintFrame(); } catch (e) {}
      const waiters = hardRepaintWaiters;
      hardRepaintWaiters = [];
      waiters.forEach((r) => r());
    }, delay);
    return p;
  }

  // 整帧重绘：清空 → 不透明背景 → 角色帧（对齐 #characterWrap 定位/尺寸）
  let lastPaintedArt = null;
  function repaintFrame() {
    ensureCanvas();
    if (!bgCanvas || !bgCtx) return;
    const dpr = window.devicePixelRatio || 1;
    const W = bgCanvas.clientWidth || window.innerWidth || 340;
    const H = bgCanvas.clientHeight || window.innerHeight || 330;
    if (bgCanvas.width !== Math.round(W * dpr)) {
      bgCanvas.width = Math.round(W * dpr);
      bgCanvas.height = Math.round(H * dpr);
    }
    bgCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
    // 1) 整帧清空：物理覆盖透明区/旧帧/残框像素（根治关键）
    bgCtx.clearRect(0, 0, W, H);
    // v31.1：透明背景模式（settings.ui.transparent_bg 开启）时不填充底色——
    // 立绘外完全透明（仅建议带合成器的系统开启；无合成器 X11 下透明区会
    // 显示黑底/回退残留，此时请保持关闭，默认关闭即不透明覆盖防重影）。
    if (!state.transparentBg) {
      const bgGrd = bgCtx.createLinearGradient(0, H, W, 0); // 左下→右上 ≈ body 135deg
      bgGrd.addColorStop(0, WINDOW_BG_TOP);
      bgGrd.addColorStop(1, WINDOW_BG_BOTTOM);
      bgCtx.fillStyle = bgGrd;
      bgCtx.fillRect(0, 0, W, H);
    }
    // 2) 画当前角色帧（绘制区顶部让出气泡区 BUBBLE_RESERVE，头部不遮）
    const art = state.artSrc ? frameCache.get(state.artSrc) : null;
    const wrap = document.getElementById('characterWrap');
    if (art && art.complete && art.naturalWidth > 0 && wrap) {
      const r = wrap.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) {
        const availH = Math.max(10, r.height - BUBBLE_RESERVE);
        const scale = Math.min(r.width / art.naturalWidth, availH / art.naturalHeight);
        const dw = art.naturalWidth * scale;
        const dh = art.naturalHeight * scale;
        const top = r.top + BUBBLE_RESERVE + (availH - dh) / 2;
        bgCtx.drawImage(art, r.left + (r.width - dw) / 2, top, dw, dh);
      }
    }
    // v41：整帧重绘只管 canvas 像素；切动作/切表情后旧帧仍留在窗口表面（重影），
    // 这里补一次整窗硬重绘，把上一帧的透明区从 X11 窗口层抹掉。
    if (state.artSrc && state.artSrc !== lastPaintedArt) {
      lastPaintedArt = state.artSrc;
      hardRepaint(40);
    }
  }

  // v30：任何 DOM 内容变化（气泡/提示/右键菜单/输入条状态）→ debounce 整帧
  // 重绘，根治「语音提示叠在人物身后慢慢形成重影」一类残留。browser 模式不启用。
  function armCanvasResidueGuard() {
    if (!isTauri || typeof MutationObserver === 'undefined') return;
    ensureCanvas();
    if (!bgCanvas) return;
    moObs = new MutationObserver(() => {
      if (moTimer) clearTimeout(moTimer);
      moTimer = setTimeout(() => { moTimer = null; repaintFrame(); }, 60);
    });
    moObs.observe(document.body, {
      childList: true, subtree: true,
      attributes: true,
      attributeFilter: ['class', 'style', 'hidden'],
    });
    window.addEventListener('resize', () => repaintFrame());
  }

  // ============================================================
  // v30 角色渲染层统一入口：showCharacterFrame(src, reason)
  // ------------------------------------------------------------
  // 任何帧切换（base/表情/动作/启动/恢复/眨眼）都必须经此函数。
  // Tauri：Image 缓存 → state.artSrc → repaintFrame()（canvas 整帧覆盖旧像素）
  // browser：维持 img 显示（沙盒冒烟/无残留环境）。
  // ============================================================
  function showCharacterFrame(src, reason) {
    if (!src) return Promise.resolve();
    reason = reason || 'unknown';
    // 幂等：同图已就绪 → 跳过（避免无谓重绘与闪烁）
    if (state.artSrc === src &&
        frameCache.has(src) && frameCache.get(src).complete && frameCache.get(src).naturalWidth > 0) {
      logFrame('frame:skip', { reason, src, token: state.swapToken });
      return Promise.resolve();
    }
    const token = ++state.swapToken;
    logFrame('frame:start', { reason, src, token });

    return new Promise((resolve) => {
      const probe = new Image();
      probe.onload = () => {
        if (token !== state.swapToken) {
          logFrame('frame:drop', { reason, src, token, cur: state.swapToken });
          resolve(); return;
        }
        frameCache.set(src, probe);
        state.artSrc = src;
        state.assetReady = true;
        stage.classList.add('asset-ready');
        face.style.display = 'none'; // inline 强制隐藏占位脸
        logFrame('frame:shown', { reason, src, token });
        if (isTauri) {
          // v30：canvas 整帧重绘（背景+角色整块覆盖旧像素）+ X/GDK 辅助
          // repaintFrame 在帧源变化时统一安排硬重绘；不再叠加多次 X11 清屏。
          repaintFrame();
        } else {
          img.src = src;
          img.classList.remove('img-missing');
        }
        resolve();
      };
      probe.onerror = () => {
        if (token !== state.swapToken) { resolve(); return; }
        logFrame('frame:probe-fail', { reason, src, token });
        console.warn('[qibai] showCharacterFrame probe 加载失败:', src);
        resolve();
      };
      probe.src = src;
    });
  }
  // 兼容别名：旧代码全部走 setCharacterImg → 现统一收敛到 showCharacterFrame
  function setCharacterImg(src) { showCharacterFrame(src, 'legacy'); }


  // v29 冒烟断言工具（仅浏览器调试用，不影响真机）：
  // 返回当前可见角色 img 数量与关键状态——harness 据此断言「任意时刻可见角色 img ≤ 1」。
  // 可见判定：display != none && hidden != true && width/height > 0。
  window.__qibaiSingleImgAssert = function () {
    const imgs = Array.from(document.querySelectorAll('#characterWrap img'));
    const visible = imgs.filter((n) => {
      const cs = window.getComputedStyle(n);
      return cs.display !== 'none' && cs.visibility !== 'hidden' && !n.hidden &&
             n.offsetWidth > 0 && n.offsetHeight > 0;
    });
    return {
      visibleCount: visible.length,
      total: imgs.length,
      currentSrc: img.getAttribute('src') || '',
      actingFrame: state.actingFrame,
      phase: state.phase,
      assetReady: state.assetReady,
      swapToken: state.swapToken,
      staticArt: state.staticArt,
      // v30：canvas 整帧重绘信息（tauri 模式角色渲染在 bgCanvas 上）
      canvas: (() => {
        ensureCanvas();
        if (!bgCanvas || !bgCtx) return { active: false };
        return { active: true, w: bgCanvas.width, h: bgCanvas.height, artSrc: state.artSrc || '' };
      })(),
    };
  };
  // v30 调试钩子：强制整帧重绘（冒烟/真机均可 eval 调用）
  window.__qibaiRepaint = repaintFrame;

  /* 立绘加载优先级（v21 抠图修复版）：
   *   ai_char_3d/<persona>.png（3D 国漫 CG 风格，透明底·已白边清理）
   *   → ai_char/<persona>.png（白底 raw 已抠透明版；替代原 _raw.png 白底方块直链）
   *   → personas/<id>/character.png（旧立绘兜底·已白边清理）
   * 逐级 preload 探测，命中即显示；全失败露出占位脸，不黑屏不报错。 */
  function loadCharacterArt(key) {
    const cands = [
      'assets/ai_char_3d/' + key + '.png',
      'assets/ai_char/' + key + '.png',
      personaAssets().base,
    ];
    let i = 0;
    const next = () => {
      if (i >= cands.length) {
        img.classList.add('img-missing');
        stage.classList.remove('asset-ready');
        state.staticArt = false;
        return;
      }
      const src = cands[i++];
      const probe = new Image();
      probe.onload = () => {
        // v29：命中候选后，staticArt 标记先就位，再统一走 showCharacterFrame 落图
        // （与动作/表情/眨眼同序列：probe → 隐藏 → 清屏 → decode → 显示 → repaint）。
        state.staticArt = src.indexOf('ai_char') !== -1;
        wrap.classList.toggle('static-art', state.staticArt);
        state.initialBoot = false; // v20.6：首次立绘加载完成
        showCharacterFrame(src, 'boot:' + key).then(() => {
          // v20.4：立绘就绪后向后端报 ready——根治「前端从不报 pet_ready →
          // 12s 后后端强制 close+重建 → 旧窗未销毁 + 新窗 = 两个完整立绘重叠」。
          try { invoke('pet_ready').catch(() => {}); } catch (e) { /* bridge 缺失时静默 */ }
        });
      };
      probe.onerror = next;
      probe.src = src;
    };
    next();
  }

  function applyPersona(key) {
    if (!PERSONAS[key]) key = DEFAULT_PERSONA;
    state.persona = key;

    state.emotion = null;
    face.className = 'placeholder-face';
    state.assetReady = false;
    img.onerror = null;
    img.classList.add('img-missing');
    stage.classList.remove('asset-ready');
    loadCharacterArt(key); // 任务八改道：3D 风格立绘优先级加载
    if (state.engine) {
      if (state.engine.setPersona) state.engine.setPersona(key);   // 2D 帧
      else if (state.engine.applyPalette) state.engine.applyPalette(key); // 3D
    }
    console.log('[qibai] pet window persona →', key, '（' + PERSONAS[key].name + '）');
    refreshVoiceTag(); // 迭代⑤：角色切换后刷新本窗音色标签
    // v20.6：切角色后真正清窗口表面（hide/show）。首次启动（init 早期窗口未
    // 稳定）跳过——只在立绘加载完成且非首次时执行，避免 v20.2 启动闪退回归。
    setTimeout(() => {
      if (state.assetReady && !state.initialBoot) {
        hardRepaint();
      }
    }, 400);
    hardRepaint(); // v18.8-fix4：切角色后清重绘残影
  }

  /* ============================================================
   * 迭代⑤ · 本窗角色音色标签（与 bridge / console 同算法）
   *   get_personas 取本窗 persona 的 voice_profile → 计算中文标签渲染到顶部 chip。
   * ============================================================ */
  const VOICE_TONES = { gudian: '温婉', dushi: '知性', wenrou: '恬静', huopo: '元气', reqing: '热情' };
  function voiceRateDesc(r) { return r < 0.9 ? '慢速' : (r < 1.1 ? '中速' : '轻快'); }
  function voicePitchDesc(p) {
    if (p < -0.5) return '微低';
    if (p <= 0.5) return '中性';
    if (p <= 1.5) return '明亮';
    return '偏高';
  }
  function voiceTagFor(id, prof) {
    const rd = voiceRateDesc(Number(prof.rate) || 1);
    const pd = voicePitchDesc(Number(prof.pitch) || 0);
    if (prof.mode === 'custom') return '自定义 · ' + rd + ' · ' + pd;
    return (VOICE_TONES[id] || '元气小小') + ' · ' + rd + ' · ' + pd;
  }

  async function refreshVoiceTag() {
    if (!voiceTagChip) return;
    // 二次修复①：交付态默认隐藏调试 chip，仅当设置 ui.show_voice_tag 开启才显示
    if (!state.showVoiceTag) { voiceTagChip.classList.add('hidden'); return; }
    const id = state.persona;
    try {
      const data = await invoke('get_personas');
      const ent = (data && data.personas || []).find((p) => p.persona_id === id);
      const prof = (ent && ent.voice_profile) || { mode: 'auto', pitch: 0, rate: 1 };
      voiceTagChip.textContent = voiceTagFor(id, prof);
      voiceTagChip.classList.remove('hidden');
    } catch (e) {
      voiceTagChip.classList.add('hidden');
    }
  }

  /* ============================================================
   * 表情系统
   * ============================================================ */

  function setEmotion(emotion) {
    if (!EMOTIONS.includes(emotion)) emotion = 'smiling';
    state.emotion = emotion;

    if (state.engine && state.engine.setEmotion) state.engine.setEmotion(emotion); // 渲染引擎路由
    face.className = 'placeholder-face ph-' + emotion;

    // v22-fix42：立绘情绪滤镜强化——表情立绘差异在小窗口不可感知，
    // 叠加滤镜类 + 情绪脉冲微动放大情绪信号
    EMOTIONS.forEach((c) => img.classList.remove('img-' + c));
    img.classList.add('img-' + emotion, 'img-pulse');
    if (state.emotionPulseTimer) clearTimeout(state.emotionPulseTimer);
    state.emotionPulseTimer = setTimeout(() => {
      img.classList.remove('img-pulse');
    }, 700);

    if (state.assetReady && !state.staticArt) {
      // v29：统一入口换表情帧（Tauri/浏览器同序列）
      showCharacterFrame(personaAssets().expr(emotion), 'emotion:' + emotion);
    }
  }

  function resetEmotion() {
    state.emotion = null;
    if (state.engine && state.engine.resetEmotion) state.engine.resetEmotion(); // 渲染引擎回 idle
    face.className = 'placeholder-face';
    EMOTIONS.forEach((c) => img.classList.remove('img-' + c, 'img-pulse'));
    if (state.emotionPulseTimer) { clearTimeout(state.emotionPulseTimer); state.emotionPulseTimer = null; }
    if (state.assetReady && !state.staticArt) {
      // v29：统一入口恢复 base
      showCharacterFrame(personaAssets().base, 'emotion:reset');
    }
  }

  /* ============================================================
   * 动作系统（CSS 动画播放一次后回 idle）
   * ============================================================ */

  const ACTION_DURATION = {
    wave: 1900, nod: 1700, shake: 1700, stretch: 1500,
    hands_on_hips: 1900, touch: 1100, pet: 1200, celebrate: 2500,
    bounce: 900, tilt: 1300, wiggle: 1000, lean: 1200, point: 1400,
  };

  // v18.5 有真实姿态帧的动作（poses/<action>.png）：切帧显示，结束后回当前表情/默认
  const ACTION_FRAMES = { wave: 1, point: 1, pet: 1, celebrate: 1, touch: 1, hands_on_hips: 1, stretch: 1, nod: 1, shake: 1 };

  // v23 自然语言调度统一演出入口：表情→setEmotion，动作→playAction（供 INTENTS/后端事件/LLM 复用）。
  function performExpress(expr) {
    if (!expr) return;
    const EMOTION_NAMES = ['smiling', 'happy', 'worried', 'sleepy', 'focused', 'sad', 'shy', 'surprised', 'angry', 'laugh', 'thinking'];
    const ACTION_NAMES = ['idle', 'wave', 'nod', 'shake', 'stretch', 'hands_on_hips', 'touch', 'celebrate', 'point', 'pet'];
    if (EMOTION_NAMES.includes(expr)) {
      setEmotion(expr);
    } else if (ACTION_NAMES.includes(expr)) {
      playAction(expr);
    }
  }

  // v24 UI 开关：窗口隐藏/显示（真机 Tauri getCurrentWindow；浏览器演示隐藏立绘容器）。
  function togglePetWindow(show) {
    if (isTauri && window.__TAURI__ && window.__TAURI__.window) {
      try {
        const w = window.__TAURI__.window.getCurrentWindow();
        if (show) { w.show(); } else { w.hide(); }
        return;
      } catch (e) { console.log('[qibai] togglePetWindow tauri 调用失败，走本地隐藏', e && e.message); }
    }
    document.body.classList.toggle('pet-window-hidden', !show);
  }

  // ============================================================
  // v29 动作系统状态机
  // ------------------------------------------------------------
  // 状态：idle（base/表情帧） → acting（动作帧） → recovering（恢复 base 过渡） → idle
  // 入口：playAction(action)；恢复帧由内部 timer 自动走 showCharacterFrame。
  // 互斥：acting 期间禁止眨眼/表情换帧（见 startBlinkLoop/setEmotion 守卫）；
  //       新动作到来时旧 timer 被 clearTimeout，swapToken 自增使旧链丢弃。
  // 所有帧切换都经 showCharacterFrame(src, reason)，杜绝直接改 img.src。
  // ============================================================
  // v43：把「有情绪语义但无对应姿态帧」的动作收敛到已有帧，避免 Tauri 下静默无动作。
  const ACTION_ALIASES = { bounce: 'celebrate', tilt: 'touch', lean: 'touch', pet: 'touch', wiggle: 'shake' };
  function realActionFrame(action) {
    const a = ACTION_ALIASES[action] || action;
    const poses = (personaAssets() && personaAssets().poses) || {};
    return poses[a] ? a : '';
  }

  function playAction(action, holdMs) {
    if (!ACTIONS.includes(action) || action === 'idle') return;
    action = realActionFrame(action) || action;
    if (state.engine && state.engine.playAction) state.engine.playAction(action); // 渲染引擎动作路由
    hardRepaint(200); // v41：动作切换后整窗硬重绘，避免新旧姿态帧在窗口表面重叠
    ACTIONS.forEach((a) => wrap.classList.remove('action-' + a));
    if (state.actionTimer) { clearTimeout(state.actionTimer); state.actionTimer = null; }

    // 有真实姿态帧（poses/<action>.png）：统一走 showCharacterFrame 切动作帧
    // ——Tauri 与浏览器同序列，不再分叉；无动作帧时回退 CSS 动画（仅浏览器演示）。
    const frame = personaAssets().poses && personaAssets().poses[action];
    if (frame && state.assetReady) {
      // 进入 acting：记录 phase + actingFrame，切到动作帧
      state.phase = 'acting';
      state.actingFrame = action;
      logFrame('action:enter', { action, frame, token: state.swapToken });
      showCharacterFrame(frame, 'action:enter:' + action);
      state.actionTimer = setTimeout(() => {
        if (state.actingFrame === action) {
          // 进入 recovering：先把 actingFrame 清空（允许眨眼/表情恢复），再切回 base/表情
          state.phase = 'recovering';
          state.actingFrame = null;
          const back = state.emotion ? personaAssets().expr(state.emotion) : personaAssets().base;
          logFrame('action:recover', { action, back, token: state.swapToken });
          showCharacterFrame(back, 'action:recover:' + action).then(() => {
            state.phase = 'idle';
            logFrame('action:idle', { action });
          });
        }
      }, Math.max(ACTION_DURATION[action] || 1900, holdMs || 0));
      return;
    }

    // 无姿态帧：浏览器演示模式用 CSS transform 动画兜底（Tauri 端 v13.1 已禁 transform 残影）
    if (!isTauri) {
      void wrap.offsetWidth; // 强制重排，保证动画重放
      wrap.classList.add('action-' + action);
      state.actionTimer = setTimeout(() => {
        wrap.classList.remove('action-' + action);
      }, ACTION_DURATION[action] || 1500);
    }
    // Tauri 端无姿态帧 → 纯表情层兜底（v21.11.9 旧行为），不做任何帧替换
  }

  /* ============================================================
   * v18.4 语气 → 表情/动作 推断（兜底演出）
   * 说话内容不同 → 表情不同 + 动作不同。LLM 返回的 emotion/action 优先，
   * 缺失时按文本特征（关键词/标点/语气词/emoji）本地推断，保证每句都有演出。
   * 规则按优先级自上而下命中即返回。
   * ============================================================ */

  function inferMood(text, persona) {
    if (window.QibaiMood && typeof window.QibaiMood.inferMood === 'function') {
      return window.QibaiMood.inferMood(text, persona || state.persona);
    }
    // 模块缺失兜底：默认微笑（动作走形象专属说话动作）
    return { emotion: 'smiling', action: 'default' };
  }

  /* 说话演出队列：表情 + 动作一次播齐，连续台词依次演出不互相打断 */
  const speechQueue = [];
  let bubbleEpoch = 0;
  let reminderTimer = null;
  let reminderPulseTimer = null;
  let speechPlaying = false;

  // v43：气泡停留不再用硬编码时长，按「中文朗读语速」估算，保证读完才收起。
  function readingMs(text, floor = 3200) {
    const chars = Array.from(String(text || '')).filter((c) => !/\s/.test(c)).length;
    return Math.max(floor, Math.min(30000, chars * 240 + 1200));
  }

  function performSpeech(text, emotion, action, ms, skipTts = false) {
    speechQueue.push({ text: text || '', emotion, action, ms: ms || readingMs(text), skipTts });
    if (!speechPlaying) nextSpeech();
  }

  function nextSpeech() {
    if (speechQueue.length === 0) {
      speechPlaying = false;
      if (state.speechGestureTimer) { clearInterval(state.speechGestureTimer); state.speechGestureTimer = null; }
      resetEmotion();
      return;
    }
    speechPlaying = true;
    const item = speechQueue.shift();
    // v43：必须把 persona 传入推理，否则性格动作映射失效；且推断出的动作
    // 若没有对应姿态帧（tilt/lean/bounce 等），Tauri 下会「什么都不播」。
    // 统一收敛到有真实帧的动作，保证说话时一定有可见动作。
    const m = inferMood(item.text, state.persona);
    const e = (item.emotion && EMOTIONS.includes(item.emotion)) ? item.emotion : m.emotion;
    let a = (item.action && ACTIONS.includes(item.action)) ? item.action : m.action;
    if (!realActionFrame(a)) a = pickSpeakAction() || 'nod';
    setEmotion(e);
    // v43：整句只摆一个静止姿势会被误认为「没有动作」。每 2.6s 重播一次同语义
    // 姿态帧（动作帧 → 表情/基础帧 → 动作帧），说话期间保持可见肢体动作。
    if (state.speechGestureTimer) { clearInterval(state.speechGestureTimer); state.speechGestureTimer = null; }
    playAction(a, 2000);
    state.speechGestureTimer = setInterval(() => {
      if (!speechPlaying) { clearInterval(state.speechGestureTimer); state.speechGestureTimer = null; return; }
      playAction(a, 1800);
    }, 2600);
    if (state.emotionResetTimer) clearTimeout(state.emotionResetTimer);
    say(item.text, item.ms, item.skipTts === true);
    state.emotionResetTimer = setTimeout(() => {
      if (!speechPlaying || speechQueue.length === 0) resetEmotion();
      nextSpeech();
    }, item.ms + 400);
  }

  /* ============================================================
   * 眨眼
   * ============================================================ */

  function startBlinkLoop() {
    state.blinkTimer = setInterval(() => {
      if (stage.classList.contains('thinking')) return;
      if (state.emotion) return;
      // v21.11.14：动作播放中（actingFrame 非空）跳过眨眼换图——欢迎/动作帧期间
      // 眨眼会把动作帧覆盖成闭眼/睁眼，动作时长结束后又恢复 base，视觉上像
      // 「两个动作交替/叠帧」。动作结束（actingFrame 清空）后恢复正常眨眼。
      if (state.actingFrame) return;

      state.blinkPhase = !state.blinkPhase;
      if (state.assetReady && !state.staticArt) {
        const target = state.blinkPhase ? personaAssets().closed : personaAssets().base;
        // v29：统一入口换眨眼帧
        showCharacterFrame(target, 'blink:' + (state.blinkPhase ? 'closed' : 'open'));
      } else if (!state.assetReady) {
        face.classList.toggle('eyes-closed', state.blinkPhase);
      }
      setTimeout(() => {
        state.blinkPhase = false;
        if (state.assetReady && !state.staticArt && !state.emotion && !state.actingFrame) {
          // v29：统一入口恢复 base
          showCharacterFrame(personaAssets().base, 'blink:recover');
        }
        face.classList.remove('eyes-closed');
      }, 160);
    }, 4000);
  }

  /* ============================================================
   * 说话气泡 / toast
   * ============================================================ */

  function pickSpeakAction() {
    const p = PERSONAS[state.persona] || PERSONAS.gudian;
    const list = p.speak_actions || ['nod'];
    return list[Math.floor(Math.random() * list.length)];
  }

  /* ============================================================
   * 语音控制中枢（迭代⑥）：一切功能入口都是对话
   * - 本地意图命中（控制台/切形象/守护…）→ 直接执行命令
   * - 未命中 → send_message 走 LLM 聊天（回复带 emotion/action 演出）
   * - 语音输入：按住 mic 说话 → 录音 → stt_transcribe → 同一入口
   * ============================================================ */
  const INTENTS = [
    /* ---- v21 语音调度本地兜底（权威路由在 Rust voice_ui_dispatch；浏览器/命令未就绪时走这里）---- */
    // 退出：需二次确认词「确认退出」
    { re: /确认退出|确定退出/i,
      fn: async () => {
        if (!state.awaitExitConfirm) return null; // 无待确认态 → 不命中，交后续
        state.awaitExitConfirm = false;
        try { await invoke('app_exit'); } catch (e) { toast('浏览器演示环境：退出操作由系统处理'); }
        return '再见啦，我会想你的～';
      } },
    { re: /退出(元气小小|这个软件|程序|应用|吧|啦|了)?\s*$|^退出/i,
      fn: () => {
        state.awaitExitConfirm = true;
        return '确定要退出元气小小吗？再说一次「确认退出」我就真的走啦～';
      } },
    // 位置锁定 / 解锁
    { re: /(锁定位置|锁定我|锁住我|位置锁定|固定位置)/,
      fn: async () => {
        try { await invoke('pet_set_locked', { persona_id: state.persona, locked: true }); } catch (e) {}
        setLocked(true);
        return '好，位置锁死啦，不随便乱跑～';
      } },
    { re: /(解锁|取消锁定|不要锁定|位置解锁)/,
      fn: async () => {
        try { await invoke('pet_set_locked', { persona_id: state.persona, locked: false }); } catch (e) {}
        setLocked(false);
        return '好，解锁啦，可以继续拖着我玩～';
      } },
    // 静音 / 取消静音（与按钮 toggleQuietMode 同一路径）
    { re: /(静音|别说话|不要朗读|关闭朗读|关掉声音|把声音关了)/,
      fn: () => { if (!state.quietMode) toggleQuietMode(); return '好，已静音，我不朗读了'; } },
    { re: /(取消静音|恢复朗读|朗读恢复|打开声音|别静音了|取消安静)/,
      fn: () => { if (state.quietMode) toggleQuietMode(); return '好，朗读恢复啦'; } },
    /* ---- v23 自然语言调度：免提 / 提醒 / 表情动作 / 窗口（本地兜底；Tauri 权威路由在 Rust）---- */
    // 免提双工
    { re: /(开启免提|打开免提|免提模式|免提聆听|双工模式|免提说话|免提对话|开始免提)/,
      fn: async () => {
        try { await invoke('set_wake_enabled', { enabled: true }); } catch (e) { toast('浏览器演示：免提由真机后端开启'); }
        return '好，免提开启啦，我随时听你说话～';
      } },
    { re: /(关闭免提|关掉免提|取消免提|不用免提|退出免提)/,
      fn: async () => {
        try { await invoke('set_wake_enabled', { enabled: false }); } catch (e) { toast('浏览器演示：免提由真机后端关闭'); }
        return '好，免提关掉啦，之后按住说话就行～';
      } },
    // 提醒（真机 Rust 权威 Remind 落盘；本地兜底给确认语）
    { re: /(提醒我|帮我提醒|设置提醒|定个提醒).{0,8}(喝水|饮水)|(喝水|饮水).{0,4}提醒/,
      fn: () => '好，我记着啦，到点提醒你喝水～' },
    { re: /(提醒我|帮我提醒|设置提醒|定个提醒).{0,8}(久坐|坐太久|起身|活动)|(久坐).{0,4}提醒/,
      fn: () => '好，我盯着呢，久坐到点提醒你起来活动～' },
    { re: /(提醒我|帮我提醒|设置提醒|定个提醒).{0,8}(护眼|眼睛|远眺)|(护眼).{0,4}提醒/,
      fn: () => '好，到点提醒你护眼休息～' },
    { re: /(取消提醒|关掉提醒|关闭提醒|不要提醒|停掉提醒)/,
      fn: () => '好，提醒都关掉啦～' },
    // 表情动作（本地演出）
    { re: /(开心一点|开心些|高兴一点|高兴些|笑一个|笑一笑|笑一下|开心起来)/,
      fn: () => { performExpress('happy'); return '来啦，笑给你看～'; } },
    { re: /(挥挥手|招招手|打个招呼|挥手|挥一下)/,
      fn: () => { performExpress('wave'); return '嗨，你好呀～'; } },
    { re: /(伸个懒腰|伸展一下|舒展一下|拉伸一下|伸懒腰)/,
      fn: () => { performExpress('stretch'); return '伸个懒腰，舒服～'; } },
    { re: /(做个动作|表演一下|秀一下|动一动|庆祝一下)/,
      fn: () => { performExpress('celebrate'); return '表演给你看～'; } },
    { re: /(生气一下|不高兴|哼一声)/,
      fn: () => { performExpress('angry'); return '哼，我生气了～'; } },
    { re: /(惊讶一下|吓一跳|哇一下)/,
      fn: () => { performExpress('surprised'); return '哇，吓我一跳～'; } },
    // 窗口位置移动（真机 Rust 权威 PetWindow 用 window API；本地兜底给确认语）
    { re: /(移到左边|移到左面|往左移|挪到左边|窗口靠左|移到最左)/,
      fn: () => '好，我挪到左边去啦～' },
    { re: /(移到右边|移到右面|往右移|挪到右边|窗口靠右|移到最右)/,
      fn: () => '好，我挪到右边去啦～' },
    { re: /(移到上面|移到上边|挪到上面|移到顶部|窗口靠上)/,
      fn: () => '好，我挪到上面去啦～' },
    { re: /(回到原位|回到原位置|窗口复位|位置复位|回到原来位置)/,
      fn: () => '好，我回到贴边位置啦～' },
    /* ---- v24 UI 开关兜底：记忆 / 贴边 / 窗口显隐 / 联网 ---- */
    { re: /(记住我们的对话|记住我们聊过|记住我说的话|记住对话|别忘记我们聊过|记住我们说的)/,
      fn: () => '好，我会记住我们聊过的话～' },
    { re: /(忘记我们的对话|忘记我们聊过|忘记对话|别记了|关掉记忆|不要记住|忘掉我们聊的)/,
      fn: () => '好，我不记啦，从这轮开始重新算～' },
    { re: /(打开贴边|开启贴边|贴边吸附|吸附到边缘|靠边吸附|吸附边缘|打开吸附)/,
      fn: () => '好，贴边吸附开啦，拖我到边缘会自动贴住～' },
    { re: /(关闭贴边|关掉贴边|取消贴边|取消吸附|不要贴边|不贴边|关闭吸附)/,
      fn: () => '好，贴边吸附关掉啦～' },
    { re: /(藏起来|先躲起来|隐身|窗口藏起来|躲一会|藏一下|藏一会|收起来)/,
      fn: () => { togglePetWindow(false); return '好，我先藏起来，需要的时候叫我～'; } },
    { re: /(出来吧|现形|显示出来|回来吧|窗口显示|出来一下|别藏了)/,
      fn: () => { togglePetWindow(true); return '我回来啦～'; } },
    { re: /(关闭联网|关掉联网|不要联网|关闭联网搜索|关掉联网搜索|关闭网络搜索|别联网了)/,
      fn: async () => {
        try { await invoke('set_skill_enabled', { skill: 'web_search', enabled: false }); } catch (e) { toast('浏览器演示：联网开关由真机设置'); }
        return '好，联网搜索关掉啦，只靠我自己的知识回答你～';
      } },
    { re: /(打开联网|开启联网|联网搜索|打开联网搜索|开启联网搜索|恢复联网)/,
      fn: async () => {
        try { await invoke('set_skill_enabled', { skill: 'web_search', enabled: true }); } catch (e) { toast('浏览器演示：联网开关由真机设置'); }
        return '好，联网搜索开啦，遇到新消息我能上网查～';
      } },
    // 极简 / 完整界面
    { re: /(极简界面|极简模式|只留角色|清爽模式|极简一下)/,
      fn: () => { setMinimal(true); return '好，切到极简模式，只留角色和输入框～'; } },
    { re: /(完整界面|全部界面|更多按钮|显示快捷按钮|恢复界面|完整一下)/,
      fn: () => { setMinimal(false); return '好，完整界面出来啦，快捷按钮都回来了～'; } },
    // 关闭控制台
    { re: /(关闭控制台|关掉控制台|收起控制台)/,
      fn: async () => {
        try { await invoke('console_close'); return '控制台收起啦'; }
        catch (e) { return '控制台暂时收不起来，你可以直接关掉它哦'; }
      } },
    // 屏幕面板（眼睛+手）
    { re: /(打开面板|屏幕面板|眼睛面板|操作面板)/,
      fn: async () => {
        try { await invoke('ui_panel_show', { payload: { mode: 'window', title: '屏幕面板', html: '<p>屏幕面板已打开</p>' } }); } catch (e) {}
        return '屏幕面板打开啦，我可以帮你操作屏幕～';
      } },
    // 隐藏 / 显示气泡
    { re: /(隐藏气泡|把气泡藏起来|气泡消失)/,
      fn: () => { dismissBubble('ctx-hide'); return '气泡藏起来啦'; } },
    { re: /(显示气泡|气泡回来)/,
      fn: () => { bubble.classList.remove('hidden'); return '气泡显示出来啦'; } },
    // tab 导航（本地兜底：打开控制台；Tauri 权威路由会额外精确切到目标 tab）
    { re: /(角色页|形象页|角色管理|切换形象页)/,
      fn: async () => { await invoke('console_open'); return '角色页在控制台里，已经帮你打开啦～'; } },
    { re: /(设置页|设置界面)/,
      fn: async () => { await invoke('console_open'); return '设置页在控制台里，已经帮你打开啦～'; } },
    { re: /(亲密页|关系页|亲密度页|亲密关系)/,
      fn: async () => { await invoke('console_open'); return '亲密度页在控制台里，已经帮你打开啦～'; } },
    { re: /(事件页|提醒页|事件中心)/,
      fn: async () => { await invoke('console_open'); return '事件页在控制台里，已经帮你打开啦～'; } },
    { re: /(诊断页|代码检查|查代码|检查代码)/,
      fn: async () => { await invoke('console_open'); return '代码诊断在控制台里，已经帮你打开啦～'; } },
    { re: /(pk页|猜拳页|pk房间|我要玩猜拳)/,
      fn: async () => { await invoke('console_open'); return '猜拳页在控制台里，已经帮你打开啦～'; } },
    // 搜索（Tauri 权威路由直接执行；本地兜底调 web_search 技能，失败则提示）
    { re: /(搜索|搜一下|查一下|查一查|百度一下|联网搜)/,
      fn: async (t) => {
        try {
          const out = await invoke('skill_invoke', { id: 'web_search', function: 'web_search', args: t });
          return out ? String(out).slice(0, 200) : '搜索完成啦～';
        } catch (e) { return '搜索已交给语音路由，联网技能就绪后就能帮你查啦～'; }
      } },
    { re: /(打开|调出|呼出|显示|看看)(控制台|面板|设置)/i,
      fn: async () => { await invoke('console_open'); return '控制台已经打开啦，想做什么直接告诉我～'; } },
    { re: /(打开|看看)(保险箱|密码箱|密码库)/i,
      fn: async () => { await invoke('console_open'); return '保险箱在控制台里，我帮你把控制台打开啦～'; } },
    { re: /(打开|看看)(社交|好友|消息|星球)/i,
      fn: async () => { await invoke('console_open'); return '社交在控制台里，已帮你打开～'; } },
    { re: /(打开|看看)(技能|插件|专家|能力)/i,
      fn: async () => { await invoke('console_open'); return '技能与专家在控制台里，已帮你打开～'; } },
    { re: /(守护我|开始守护|守护模式)/i,
      fn: async () => { await invoke('guard_trigger'); return '收到，我帮你守着屏幕，注意休息呀～'; } },
    { re: /(切换|换成|换到|变成|用)(古典佳人|古典|gudian)/i,
      fn: async () => { await invoke('set_active_persona', { persona_id: 'gudian' }); return '好呀，换上古典佳人～'; } },
    { re: /(切换|换成|换到|变成|用)(都市|dushi)/i,
      fn: async () => { await invoke('set_active_persona', { persona_id: 'dushi' }); return '好呀，换上都市女性～'; } },
    { re: /(切换|换成|换到|变成|用)(温柔|wenrou)/i,
      fn: async () => { await invoke('set_active_persona', { persona_id: 'wenrou' }); return '好呀，换成温柔形象～'; } },
    { re: /(切换|换成|换到|变成|用)(活泼|huopo)/i,
      fn: async () => { await invoke('set_active_persona', { persona_id: 'huopo' }); return '好呀，换上活泼形象～'; } },
    { re: /(切换|换成|换到|变成|用)(热情|时尚|reqing)/i,
      fn: async () => { await invoke('set_active_persona', { persona_id: 'reqing' }); return '好呀，换上热情时尚～'; } },
  ];

  async function detectIntent(text) {
    for (const it of INTENTS) {
      if (it.re.test(text)) {
        try {
          const r = await it.fn(text);
          if (r !== null && r !== undefined) return r; // null = 命中但不执行（如退出未二次确认），继续后续匹配
        }
        catch (e) { console.warn(e); return '这个操作暂时没成功，稍后再试试？'; }
      }
    }
    return null;
  }

  // v16：技能库调用（打字/按住说话共用）——「调用/使用/执行 + 技能名」→ skill_invoke → 朗读结果。
  async function trySkillInvoke(t) {
    try {
      const heads = ['调用', '使用', '执行', '帮我'];
      if (!heads.some((k) => t.includes(k))) return null;
      const skills = await invoke('skill_list');
      let best = null;
      for (const sk of skills || []) {
        const id = sk.id || '';
        const name = sk.name || '';
        const fns = (sk.functions || []).map((f) => f.name || '');
        const hit = (name && t.includes(name)) || fns.some((f) => t.includes(f)) || (id && t.includes(id));
        if (hit && (!best || id.length > best.id.length)) best = { id, function: fns[0] || '' };
      }
      if (!best) return null;
      const out = await invoke('skill_invoke', { id: best.id, function: best.function, args: t });
      if (!out || !String(out).trim()) return null;
      return String(out);
    } catch (e) { console.warn('skill invoke failed', e); return null; }
  }

  // v18.8-fix28e：invoke 前端超时兜底
  function invokeTimeout(promise, ms, label) {
    let timer;
    const guard = new Promise((_, rej) => {
      timer = setTimeout(() => rej(new Error((label || '请求') + '响应超时（' + ms / 1000 + 's）')), ms);
    });
    return Promise.race([promise, guard]).finally(() => clearTimeout(timer));
  }

  async function sendChat(rawText) {
    const t = (rawText || '').trim();
    if (!t || state.sending) return; // 空输入/发送中防重
    state.sending = true;
    chatInput.value = '';
    if (btnSend) { btnSend.disabled = true; btnSend.style.opacity = '.4'; }
    try {
      // v21：Tauri 权威语音路由 voice_ui_dispatch（并行 Rust 子代理提供）。
      // 命中 → say(reply) 并 return；命令未就绪/未命中/浏览器 mock → 落到本地兜底链路。
      if (isTauri) {
        try {
          const vd = await invoke('voice_ui_dispatch', { text: t });
          if (vd && vd.matched) {
            setEmotion('happy');
            // v21.11.8：命令命中后端已直接播放（rust_played）→ 前端只显气泡不重复朗读（防双响）
            say(vd.reply || '好的～', 3500, !!vd.rust_played);
            return;
          }
        } catch (e) { /* 命令未就绪 → 走本地 INTENTS 兜底 */ }
      }
      const skillReply = await trySkillInvoke(t);
      if (skillReply) { setEmotion('happy'); say(skillReply, 4000); return; }
      const intentReply = await detectIntent(t);
      if (intentReply) { setEmotion('happy'); say(intentReply, 3500); return; }
      setEmotion('focused');
      bubble.classList.remove('hidden');
      bubble.classList.add('thinking');
      speechText.textContent = '正在想';
      if (speechTime) { speechTime.textContent = hhmm(); speechTime.classList.remove('hidden'); }
      hardRepaint(); // v21.11.10：thinking 态气泡出现清窗防残影
      if (state.speechTimer) clearTimeout(state.speechTimer);
      // v21.11.12：thinking 兜底隐藏——LLM 卡住/超时/出错时「正在想」气泡不再常驻不消失。
      // 正常流式第一段 tts-chunk 到达会被 say/流式分支覆盖并重置计时；此处仅兜底。
      state.speechTimer = setTimeout(() => {
        if (speechText.textContent === '正在想') {
          dismissBubble('thinking-timeout');
        }
      }, 15000);
      const r = await invokeTimeout(invoke('send_message', { text: t }), 130000, '大模型');
      const reply = (r && r.reply) || '';
      if (r && r.emotion) setEmotion(r.emotion);
      // v31：回复内容自动映射动作（不依赖 LLM 字段）——inferMood 按文本关键词
      // 推断情绪/动作，让每次回复都带动画演出，增强动态可玩性。
      const m = inferMood(reply);
      if (r && r.action) playAction(r.action);
      else if (m && m.action && ACTIONS.includes(m.action) && m.action !== 'idle') playAction(m.action);
      else if (m && m.emotion && EMOTIONS.includes(m.emotion)) setEmotion(m.emotion);
      else { const pa = pickSpeakAction(); if (pa) playAction(pa); }
      // v25：浏览器 mock/整段回复走打字机上屏（不朗读，防字未出完就发声）。
      // v21.11.11：真机不再由前端 say 触发朗读——Rust 端 engine 流式/非流式已统一
      // emit qibai://tts-chunk（rust_played=true，前端实时显示气泡）并在 Rust 端完成
      // 朗读（send_message 兜底 perform_speech）。前端若再 say(reply) 且窗口失焦，
      // allowTts=true 会再次触发 tts_speak = 打字/按住 mic「回答两次」的根因。
      // 这里只用 skipTts=true 把气泡补全为完整回复文本，朗读权一律归 Rust 端。
      if (isTauri) {
        say(reply || '嗯嗯，我听着呢～', 3500, true);
      } else {
        typewriterSay(reply || '嗯嗯，我听着呢～', 4000);
      }
    } catch (e) {
      console.warn(e);
      const msg = String((e && e.message) || e || '');
      setEmotion('worried');
      // v45.7：按错误内容分流，不再一律说"没听清"。
      if (!msg) {
        // 空错误 = STT 空转写（没听到说话）
        say('刚才没听清，再说一遍好吗？', 2600);
      } else if (msg.includes('超时')) {
        say('我这边响应太慢了，稍等一下再问我好吗？', 2600);
        toast('大模型超时：' + msg.slice(0, 120));
      } else if (msg.includes('反复试了') || msg.includes('工具循环') || msg.includes('MaxIterations')) {
        // v45.7：工具循环打满（MaxIterations）——不是网络问题，是卡在需要确认的操作上
        say('这一轮我卡在等确认上了，先停一下～', 2600);
        toast('这轮卡在需要你确认的操作上，不是网络问题');
      } else if (msg.includes('拒绝') || msg.includes('白名单') || msg.includes('越界') || msg.includes('不在工作区')) {
        say('我没法写到那个位置：' + msg.slice(0, 40), 3200);
        toast(msg.slice(0, 120));
      } else if (msg.includes('亲密度')) {
        // 原样转述门控文案，如"这个操作需要亲密度 L4 才能批准（当前 L1）"
        say(msg.slice(0, 60), 3200);
        toast(msg.slice(0, 120));
      } else {
        say('我这边出错了：' + msg.slice(0, 40), 3200);
        toast(msg.slice(0, 120));
      }
    } finally {
      state.sending = false;
      if (btnSend) { btnSend.disabled = !chatInput.value.trim(); btnSend.style.opacity = chatInput.value.trim() ? '1' : '.4'; }
    }
  }

  /* ---- 语音输入（按住 mic 说话）---- */
  // v11：PTT 改走 Rust 直采（ptt_start/ptt_stop）——根治 WebView getUserMedia
  // 需要系统麦克风权限的问题（Linux WebKitGTK 无公开放行 API）。
  async function startRecord() {
    if (state.recording) return;
    try {
      const r = await invoke('ptt_start');
      if (!r || r.ok !== true) { toast('麦克风启动失败，请检查系统麦克风权限/设备'); return; }
      state.recording = true;
      btnMic.classList.add('rec');
      say('嗯，我在听，你说～', 2000, true);
    } catch (e) {
      console.warn(e);
      // v45.8：启动失败必须清状态与样式，避免下一次录音带上次残留
      state.recording = false;
      btnMic.classList.remove('rec');
      toast('麦克风启动失败：' + (((e && e.message) || e) + '').slice(0, 60));
    }
  }

  async function stopRecord() {
    if (!state.recording) return;
    state.recording = false;
    btnMic.classList.remove('rec');
    try {
      const r = await invoke('ptt_stop');
      const b64 = (r && r.audio_b64) || '';
      if (!b64 || b64.length < 300) { toast('没听到声音，再说一次？'); return; }
      setEmotion('focused');
      const text = await invoke('stt_transcribe', { audio_b64: b64 });
      if (text && text.trim()) sendChat(text);
      else toast('没有识别到内容，试试文字输入？');
    } catch (e) {
      console.warn(e);
      const msg = String((e && e.message) || e || '');
      // v45.8：设备断开（拔麦）→ 只 toast 原文，不送转写
      if (msg.includes('麦克风已断开') || msg.includes('这次录音作废')) {
        toast(msg);
      } else {
        toast('语音识别失败：' + msg.slice(0, 60) + '（可在设置中配置 STT，或改用文字输入）');
      }
    }
  }

  // v18.8-fix27：LLM 流式朗读时间戳——流式逐段发声后，整段 say 不再重复朗读。
  let lastStreamTs = 0;

  // v25/v42：所有气泡分支共用同一份生命周期。新文案使旧计时器、旧打字机立即失效。
  let typeTimer = null;

  // v45：滚动条随气泡生命周期创建/销毁，不再靠 opacity class 切换。
  // 真机拍屏证据：静态 #speechScrollbar 在透明窗表面留下不同高度 thumb 的残影
  // （照片右缘两个粉色竖胶囊）。dismiss 时物理 remove，beginBubble 时按需重建。
  function ensureBubbleScrollbar() {
    if (document.getElementById('speechScrollbar')) return;
    const bar = document.createElement('span');
    bar.id = 'speechScrollbar';
    bar.setAttribute('aria-hidden', 'true');
    const thumb = document.createElement('i');
    thumb.id = 'speechScrollThumb';
    bar.appendChild(thumb);
    bubble.appendChild(bar);
  }
  function destroyBubbleScrollbar() {
    const bar = document.getElementById('speechScrollbar');
    if (bar) bar.remove();
  }

  function updateBubbleScrollbar() {
    try {
      const bar = document.getElementById('speechScrollbar');
      const thumb = document.getElementById('speechScrollThumb');
      if (!bar || !thumb || !speechText) return;
      const sh = speechText.scrollHeight, ch = speechText.clientHeight;
      if (sh <= ch + 2 || bubble.classList.contains('hidden')) {
        bubble.classList.remove('scrollable');
        return;
      }
      bubble.classList.add('scrollable');
      const trackH = bar.clientHeight || (ch + 14);
      const thumbH = Math.min(trackH, Math.max(14, Math.round(ch * ch / sh)));
      const maxTop = Math.max(0, trackH - thumbH);
      const ratio = sh > ch ? speechText.scrollTop / (sh - ch) : 0;
      thumb.style.height = thumbH + 'px';
      thumb.style.top = Math.round(maxTop * Math.min(1, Math.max(0, ratio))) + 'px';
    } catch (e) {}
  }

  function bubbleScrollToEnd() {
    const scroll = () => {
      try {
        if (speechText && bubble && !bubble.classList.contains('hidden')) {
          // 只滚动正文；外框、时间戳和喇叭保持在原位，短句不会被滚出顶部。
          speechText.scrollTop = speechText.scrollHeight;
          updateBubbleScrollbar();
        }
      } catch (e) {}
    };
    // 立即一次 + 排版后一次，覆盖流式文本在同一帧内换行、scrollHeight 后变化的情况。
    scroll();
    requestAnimationFrame(scroll);
    setTimeout(scroll, 0);
  }

  // v43：气泡停留定时器支持「悬停/滚轮暂停」，用户正在读或滚动时不自动消失。
  let bubbleHoverPaused = false;
  let bubbleResumeTimer = null;
  let bubbleHideRemain = 0;
  let bubbleHideAt = 0;
  // v45 显示节奏（用户定稿）：短句 ≤24 字，4s 自动收起；长句不自动收，
  // 一直留在屏上直到下一句覆盖、右键收掉或语音收起——朗读完毕也不强收。
  const BUBBLE_SHORT_CHARS = 24;
  const BUBBLE_SHORT_MS = 4000;
  function bubbleDwell(text, ms) {
    const n = Array.from(String(text || '')).length;
    // 短句：至少停 4s 再收（调用方给的 ms 只当"更久"的下限，不会提前收）
    if (n <= BUBBLE_SHORT_CHARS) return Math.max(Number(ms) || 0, BUBBLE_SHORT_MS);
    // 长句：一律常驻，不自动收（0 = 不挂定时器），留到下一句/右键/语音收
    return 0;
  }
  function armBubbleHide(epoch, ms) {
    if (state.speechTimer) { clearTimeout(state.speechTimer); state.speechTimer = null; }
    if (!(Number(ms) > 0)) {            // 0 = 长句常驻，不挂定时器
      state.bubbleSticky = true;
      bubbleHoverPaused = false;
      bubbleHideAt = 0; bubbleHideRemain = 0;
      return;
    }
    state.bubbleSticky = false;
    bubbleHoverPaused = false;
    bubbleHideRemain = Math.max(1200, Number(ms) || 0);
    bubbleHideAt = Date.now() + bubbleHideRemain;
    state.speechTimer = setTimeout(() => {
      bubbleHideAt = 0; bubbleHideRemain = 0;
      if (epoch === bubbleEpoch) hideSpeechBubble();
    }, bubbleHideRemain);
  }
  function pauseBubbleHide(restartMs) {
    if (!state.speechTimer && !bubbleHoverPaused) return;
    if (state.speechTimer) { clearTimeout(state.speechTimer); state.speechTimer = null; }
    bubbleHoverPaused = true;
    bubbleHideRemain = Math.max(bubbleHideRemain || 0, Number(restartMs) || 0,
      Math.max(1200, bubbleHideAt - Date.now()));
    bubbleHideAt = 0;
  }
  function resumeBubbleHide(delayMs) {
    if (!bubbleHoverPaused || bubble.classList.contains('hidden')) return;
    bubbleHoverPaused = false;
    const wait = Math.max(1800, Math.min(Number(delayMs) || 0 || bubbleHideRemain || 1800, 30000));
    const epoch = bubbleEpoch;
    if (state.speechTimer) clearTimeout(state.speechTimer);
    bubbleHideAt = Date.now() + wait;
    state.speechTimer = setTimeout(() => {
      bubbleHideAt = 0;
      if (epoch === bubbleEpoch) hideSpeechBubble();
    }, wait);
  }

  function beginBubble() {
    const epoch = ++bubbleEpoch;
    if (state.speechTimer) { clearTimeout(state.speechTimer); state.speechTimer = null; }
    if (typeTimer) { clearInterval(typeTimer); typeTimer = null; }
    if (reminderTimer) { clearTimeout(reminderTimer); reminderTimer = null; }
    if (reminderPulseTimer) { clearTimeout(reminderPulseTimer); reminderPulseTimer = null; }
    reminderOverlay.classList.remove('show');
    bubble.classList.remove('hidden', 'thinking');
    state.bubbleState = 'showing'; // 状态机：同一时刻只允许一个可见态
    bubble.scrollTop = 0;
    speechText.scrollTop = 0;
    ensureBubbleScrollbar();       // v45：每次开新气泡重建滚动条（不残留旧 thumb）
    if (speechTime) { speechTime.textContent = hhmm(); speechTime.classList.remove('hidden'); }
    return epoch;
  }

  function hideSpeechBubble() {
    dismissBubble('auto-hide');
  }

  // v45 气泡消失统一收尾（dismissBubble）：所有隐藏路径（自动计时/thinking 超时/
  // 右键菜单/语音 bubble-dismiss/菜单项藏气泡/切角色）都走这里，绝不各写各的。
  // 根因 B：单实例 #speechBubble，但透明 ARGB 无合成器 X11 下 DOM display:none 后，
  // 旧气泡文字像素留在窗口表面不被擦除，下一句画上去就「两层字、每句多一层不收敛」。
  // v45.6：原生表面操作（pet_invalidate_rect / pet_repaint / pet_surface_rebuild）已整条
  // 删除——真机证据 Gdk-ERROR BadDrawable RENDER ChangePicture（GTK 缓存的 Picture
  // 打在已销毁 drawable 上 → SIGTRAP）。现在只走 web 侧：canvas clearRect + 重绘立绘
  // + 一次 requestAnimationFrame。无合成器时走不透明窗，底色实心，旧像素天然被覆盖。
  function dismissBubble(reason) {
    if (state.bubbleState === 'idle') {
      // 已 idle：仅保证 hidden，不重复清屏（避免去抖/重绘风暴）
      bubble.classList.add('hidden');
      return;
    }
    ++bubbleEpoch;
    if (state.speechTimer) { clearTimeout(state.speechTimer); state.speechTimer = null; }
    if (typeTimer) { clearInterval(typeTimer); typeTimer = null; }
    if (reminderTimer) { clearTimeout(reminderTimer); reminderTimer = null; }
    if (reminderPulseTimer) { clearTimeout(reminderPulseTimer); reminderPulseTimer = null; }
    if (bubbleResumeTimer) { clearTimeout(bubbleResumeTimer); bubbleResumeTimer = null; }
    bubbleHoverPaused = false;
    bubble.classList.add('hidden');
    bubble.classList.remove('scrollable', 'thinking', 'speaking');
    destroyBubbleScrollbar();    // v45：物理移除滚动条 DOM，不留旧 thumb 残影
    if (speechTime) speechTime.classList.add('hidden');
    if (state.engine && state.engine.setTalk) state.engine.setTalk(false);
    state.bubbleState = 'idle';
    // v45.6：纯 web 侧清屏——canvas 整帧重绘 + 一帧 rAF。不再 invoke 任何原生表面操作。
    if (isTauri) {
      repaintFrame();
      requestAnimationFrame(() => repaintFrame());
    }
  }

  // v43：悬停/滚轮阅读时暂停自动收起；滚轮后 2.5s 继续。放在函数定义后注册，
  // 确保在 Tauri/browser 两种模式下都生效。
  bubble.addEventListener('pointerenter', () => pauseBubbleHide());
  bubble.addEventListener('pointerleave', () => resumeBubbleHide(2000));
  speechText.addEventListener('wheel', () => {
    pauseBubbleHide(2500);
    if (bubbleResumeTimer) clearTimeout(bubbleResumeTimer);
    bubbleResumeTimer = setTimeout(() => { bubbleResumeTimer = null; resumeBubbleHide(2500); }, 2500);
  }, { passive: true });
  speechText.addEventListener('touchstart', () => pauseBubbleHide(), { passive: true });
  speechText.addEventListener('scroll', updateBubbleScrollbar, { passive: true });
  window.addEventListener('resize', updateBubbleScrollbar);

  function typewriterSay(text, ms = 0) {
    const epoch = beginBubble();
    const dwell = bubbleDwell(text, ms);
    speechText.textContent = '';
    state.lastSpokenText = String(text || '');
    hardRepaint();
    const chars = Array.from(String(text || ''));
    if (!chars.length) { hideSpeechBubble(); return; }
    let i = 0;
    typeTimer = setInterval(() => {
      if (epoch !== bubbleEpoch) { clearInterval(typeTimer); typeTimer = null; return; }
      i += 2;
      speechText.textContent = chars.slice(0, i).join('');
      bubbleScrollToEnd();
      if (i >= chars.length) {
        clearInterval(typeTimer);
        typeTimer = null;
        armBubbleHide(epoch, dwell);
      }
    }, 26);
  }

  function say(text, ms = 0, skipTts = false) {
    const epoch = beginBubble();
    // v43：真实朗读时长未知（Rust 端没有结束事件），用字符数估算；
    // 传入值只作为下限，长句不会再 3 秒就消失。
    const dwell = bubbleDwell(text, ms);

    // v45.8：抽出来源 URL——若正文已有"1. http"形态则只 linkify，否则渲染来源块
    const { clean, sources, total } = extractSources(text);
    speechText.textContent = clean;
    renderSources(sources, total);

    // 带来源列表的气泡一律常驻，不自动收
    const sticky = sources.length > 0;
    if (sticky) state.bubbleSticky = true;

    bubbleScrollToEnd();
    state.lastSpokenText = text; // 迭代②：喇叭按钮重读最近一句
    hardRepaint();
    const focusedNow = (typeof document !== 'undefined' && typeof document.hasFocus === 'function')
      ? document.hasFocus() : state.windowFocused;
    const allowTts = !state.quietMode && !focusedNow;
    const streamedJustNow = Date.now() - lastStreamTs < 3000;
    if (state.engine && state.engine.setTalk) state.engine.setTalk(true);
    armBubbleHide(epoch, dwell);
    // 朗读真正结束后再补 1.2s 余量，避免云端音频比文本估计更长时提前收起。
    if (allowTts && text && String(text).trim() && !streamedJustNow && !skipTts) {
      invoke('tts_speak', { text: String(text), kind: 'chat', persona_id: state.persona })
        .then(() => {
          if (epoch !== bubbleEpoch) return;
          if (dwell === 0 || sticky) return;   // v45/v45.8：长句/带来源常驻，朗读完也不收
          armBubbleHide(epoch, 1800);
        }).catch(() => {});
    }
  }

  // v45.8：从回复文本抽 URL 来源
  function extractSources(text) {
    const urlRe = /(https?:\/\/[^\s)\]>"'<]+)/g;
    const all = text.match(urlRe) || [];
    const urls = all.slice(0, 5);
    // 若正文已有 "1. http" 形态，只 linkify 不重复列
    const alreadyListed = /\d+\.\s*https?:\/\//.test(text);
    if (alreadyListed) {
      // 去掉正文里的 URL 列表行，让气泡正文干净；来源块仍渲染
      const clean = text.replace(/\n*\d+\.\s*https?:\/\/[^\s)\]>"'<]+/g, '').trim();
      return { clean, sources: urls, total: all.length };
    }
    return { clean: text, sources: urls, total: all.length };
  }

  // v45.8：在气泡下方渲染来源列表（每行 数字. 域名，可点）
  function renderSources(urls, total) {
    let box = document.getElementById('speechSources');
    if (!box) {
      box = document.createElement('div');
      box.id = 'speechSources';
      box.style.cssText = 'margin-top:6px;font-size:11px;line-height:1.6;color:#888;';
      bubble.appendChild(box);
    }
    if (!urls || !urls.length) { box.remove(); return; }
    box.innerHTML = '';
    urls.forEach((u, i) => {
      let domain = u.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
      const a = document.createElement('a');
      a.href = u;
      a.target = '_blank';
      a.rel = 'noopener';
      a.textContent = `${i + 1}. ${domain}`;
      a.style.cssText = 'color:#6aa;padding:1px 4px;border-radius:3px;display:inline-block;';
      a.addEventListener('click', (e) => {
        e.preventDefault();
        invoke('open_external', { url: u }).catch(() => { window.open(u, '_blank'); });
      });
      box.appendChild(a);
      box.appendChild(document.createTextNode(' '));
    });
    // 超过 5 条：显示"还有 N 条"，点击展开全部
    if (total && total > urls.length) {
      const more = document.createElement('a');
      more.href = '#';
      more.textContent = `还有 ${total - urls.length} 条`;
      more.style.cssText = 'color:#888;font-size:11px;cursor:pointer;margin-left:4px;';
      more.addEventListener('click', (e) => {
        e.preventDefault();
        // 简单展开：直接把全部 URL 列出（不折叠）
        const urlRe = /(https?:\/\/[^\s)\]>"'<]+)/g;
        const all = (state.lastSpokenText || '').match(urlRe) || [];
        box.innerHTML = '';
        all.forEach((u, i) => {
          let domain = u.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
          const a = document.createElement('a');
          a.href = u; a.target = '_blank'; a.rel = 'noopener';
          a.textContent = `${i + 1}. ${domain}`;
          a.style.cssText = 'color:#6aa;padding:1px 4px;border-radius:3px;display:inline-block;';
          a.addEventListener('click', (ev) => { ev.preventDefault(); invoke('open_external', { url: u }).catch(() => window.open(u, '_blank')); });
          box.appendChild(a); box.appendChild(document.createTextNode(' '));
        });
      });
      box.appendChild(more);
    }
  }

  /* ---------- 迭代②·语音：喇叭按钮重读 + tts-speak 气泡脉冲 ---------- */
  function pulseSpeaking() {
    bubble.classList.add('speaking');
    if (state.speakingTimer) clearTimeout(state.speakingTimer);
    state.speakingTimer = setTimeout(() => {
      bubble.classList.remove('speaking');
    }, 1600);
  }

  btnSpeak.addEventListener('click', async () => {
    // 迭代③·保险箱：喇叭重读优先读 conversation 最后一条 assistant 消息——
    // core 保证写入 conversation 的只有脱敏文本（secret 回复落盘的是 speak_text，
    // 明文 reply 永不进 conversation/永不发声）。读取失败则降级用最近气泡文本。
    let text = '';
    try {
      const convo = await invoke('get_conversation');
      const last = (convo || []).slice().reverse().find((m) => m && m.role === 'assistant');
      if (last && last.content) text = last.content;
    } catch (e) { /* 桥接未就绪：走降级 */ }
    text = text || state.lastSpokenText || speechText.textContent || '';
    if (!text.trim()) {
      toast('还没有可朗读的内容哦');
      return;
    }
    try {
      // 迭代⑤：喇叭重读带本窗 persona_id，用该角色专属音色朗读
      const r = await invoke('tts_speak', { text, kind: 'chat', persona_id: state.persona });
      if (r && r.engine === 'mute') {
        // mute 态不报错：明确提示用户去设置开启
        toast((r && r.reason) || '语音朗读已关闭，可在设置中开启');
        return;
      }
      pulseSpeaking();
    } catch (e) {
      toast('朗读暂时不可用');
    }
  });

  // v18.8-fix27：LLM 流式输出 → 逐段朗读（第一句生成完即开口），并同步显示气泡。
  listen('qibai://tts-chunk', (event) => {
    const p = event.payload || {};
    const seg = p && p.text ? String(p.text) : '';
    // v20：前端接收断点诊断（只打元数据：段长/标志，绝不打印文本）——对照 Rust emit 日志
    console.log('[qibai][fe] tts-chunk recv: len=' + seg.length + ', rust_played=' + (p.rust_played === true));
    if (seg && seg.trim()) {
      lastStreamTs = Date.now();
      const pid = (p.persona) || state.persona;
      // v18.8-fix28f：rust_played=true 表示 Rust 端已直接播放——只显示气泡不重复朗读。
      if (p.rust_played === true) {
        // 与普通气泡共用状态机：取消旧计时/打字机、滚动到底、到时统一隐藏。
        say(seg, readingMs(seg, 3200), true);
        return;
      }
      say(seg, readingMs(seg, 3200));
      console.log('[qibai][fe] tts_speak trigger: kind=chat');
      invoke('tts_speak', { text: seg, kind: 'chat', persona_id: pid }).catch(() => {});
    }
  });

  // 朗读指示器：气泡加短暂 speaking 脉冲（Rust §3 payload 同形；mock 额外带 text 时顺手演出）
  // 迭代③：p.text 来自 tts_speak 的入参——控制台对 secret 回复一律传 speak_text
  //（脱敏文本），因此气泡展示与后续喇叭重读都不会出现密码明文。
  listen('qibai://tts-speak', (event) => {
    const p = event.payload || {};
    if (p.state === 'speaking') {
      if (p.text) say(p.text, 2400);
      pulseSpeaking();
    }
  });

  function toast(text, ms = 2600) {
    // v40：气泡正在说话时先不弹（提示条无论放顶部还是下移都会压住脸/头发）。
    // 等气泡收起再弹，最多等 3s 兜底，保证提示一定送达。
    if (isTauri) {
      const bb = document.getElementById('speechBubble');
      const rr = document.getElementById('reminderOverlay');
      const topVisible = (bb && !bb.classList.contains('hidden')) || (rr && rr.classList.contains('show'));
      if (topVisible) {
        let tries = 0;
        const timer = setInterval(() => {
          tries += 1;
          const bubbleGone = !bb || bb.classList.contains('hidden');
          const reminderGone = !rr || !rr.classList.contains('show');
          if ((bubbleGone && reminderGone) || tries > 20) {
            clearInterval(timer);
            toast(text, ms);
          }
        }, 150);
        return;
      }
    }
    const t = document.createElement('div');
    t.className = 'toast';
    t.textContent = text;
    toastBox.appendChild(t);
    if (isTauri) {
      // v21.11.14：无合成器 X11 下过渡动画逐帧残留 toast 文字（「已切回文字输入」
      // 「没听到声音」不消失的根因）——Tauri 端无动画直接显示，到时直接移除
      // （不留过渡期）并整窗重绘，把 toast 旧像素从屏幕层清掉。
      setTimeout(() => {
        t.remove();
        hardRepaint();
      }, ms);
      return;
    }
    requestAnimationFrame(() => t.classList.add('show'));
    setTimeout(() => {
      t.classList.remove('show');
      setTimeout(() => t.remove(), 350);
    }, ms);
  }

  /* ============================================================
   * 亲密度（规格 §8.2）：点按上报 + 爱心/星星飘浮 + 升级庆祝 + 陪伴上报
   * ============================================================ */

  // 上报一次亲密度事件（fire-and-forget；bridge/mock 未就绪时静默）
  async function reportIntimacy(kind, detail) {
    try {
      await invoke('intimacy_event', {
        persona_id: state.persona,
        kind,
        ...(detail ? { detail } : {}),
      });
    } catch (e) { /* mock 并行开发中；不影响既有互动 */ }
  }

  // 在 (x, y) 生成一颗飘浮粒子（♥ / ✦）
  function spawnParticle(x, y, glyph) {
    const s = document.createElement('span');
    s.className = 'click-fx ' + (glyph === '♥' ? 'fx-heart' : 'fx-star');
    s.textContent = glyph;
    s.style.left = x + 'px';
    s.style.top = y + 'px';
    s.style.setProperty('--drift', (Math.random() * 44 - 22).toFixed(1) + 'px');
    fxLayer.appendChild(s);
    setTimeout(() => {
      s.remove();
      // v21.11.14：无合成器 X11 下粒子移除后旧像素不清 → 特效残留；
      // Tauri 端移除粒子后整窗重绘清屏。
      if (isTauri) hardRepaint();
    }, 1300);
  }

  // 点按：中心处 5 颗粒子错落飘出
  function spawnClickFx(x, y) {
    for (let i = 0; i < 5; i++) {
      setTimeout(() => {
        spawnParticle(x + Math.random() * 20 - 10, y + Math.random() * 16 - 8,
          Math.random() < 0.6 ? '♥' : '✦');
      }, i * 90);
    }
  }

  // 点按互动统一入口：动作 + 撒娇气泡 + 粒子 + click 上报
  // evt 为 MouseEvent（真实点击）或 undefined（dev 按钮触发，取 wrap 中心）
  // F1：分区点击反馈表（头/脸颊/肚/手/脚 → 不同表情+台词+动作）
  const REGION_TABLE = [
    { key: 'head',   cond: (rx, ry) => ry < 0.28,           emotion: 'happy',  line: '摸摸头，好舒服～',       act: 'touch' },
    { key: 'cheek',  cond: (rx, ry) => ry >= 0.28 && ry < 0.45, emotion: 'blush', line: '哎呀，戳脸脸会害羞啦。',   act: 'shake' },
    { key: 'belly',  cond: (rx, ry) => ry >= 0.45 && ry < 0.68, emotion: 'laugh', line: '哈哈哈哈，好痒呀～',     act: 'bounce' },
    { key: 'foot',   cond: (rx, ry) => ry >= 0.82,           emotion: 'surprised', line: '戳脚脚！站不稳啦～',     act: 'point' },
    { key: 'hand',   cond: (rx, ry) => ry >= 0.68 && ry < 0.82 && (rx < 0.25 || rx > 0.75), emotion: 'happy', line: '击个掌！', act: 'wave' },
  ];

  function doPetClick(evt) {
    let x = window.innerWidth / 2;
    let y = window.innerHeight / 2;
    let rx = 0.5, ry = 0.5;
    if (evt && typeof evt.clientX === 'number') {
      x = evt.clientX;
      y = evt.clientY;
      const r = wrap.getBoundingClientRect();
      rx = (r.width > 0) ? (x - r.left) / r.width : 0.5;
      ry = (r.height > 0) ? (y - r.top) / r.height : 0.5;
    } else {
      const r = wrap.getBoundingClientRect();
      x = r.left + r.width / 2;
      y = r.top + r.height / 2;
    }

    // F2：记录交互时间（唤醒环境机）
    state.lastInteractionTs = Date.now();
    state.recentClicks.push(Date.now());
    while (state.recentClicks.length && Date.now() - state.recentClicks[0] > ENV.exciteWindowMs) state.recentClicks.shift();

    // 粗暴/连续狂点 → 微怒反馈
    if (state.recentClicks.length >= ENV.exciteClicks) {
      setEmotion('angry');
      say('别一直戳啦，我会生气的哦～', 1800);
      playAction('shake');
      spawnClickFx(x, y);
      reportIntimacy('click');
      return;
    }

    // F1：命中分区（默认落肚子中间区）
    const region = REGION_TABLE.find((r) => r.cond(rx, ry)) || { emotion: 'happy', line: PET_LINES[Math.floor(Math.random() * PET_LINES.length)], act: 'touch' };
    setEmotion(region.emotion);
    playAction(region.act || 'touch');
    say(region.line, 1800);
    spawnClickFx(x, y);
    reportIntimacy('click');
  }

  // 升级庆祝（leveled_up=true）：光晕 + 飘心 + 跳跃 + 升级气泡
  function celebrateLevelUp(p) {
    stage.classList.add('celebrating');
    playAction('celebrate');
    setEmotion('happy');
    setTimeout(resetEmotion, 3200);

    // 全窗飘心：12 颗从底部错落升起
    for (let i = 0; i < 12; i++) {
      setTimeout(() => {
        spawnParticle(
          16 + Math.random() * (window.innerWidth - 32),
          window.innerHeight - 12,
          Math.random() < 0.7 ? '♥' : '✦'
        );
      }, i * 160);
    }

    // 气泡：主文案 + 新解锁 1-2 条
    const unlocked = Array.isArray(p.unlocked)
      ? p.unlocked.map((u) => u && u.label).filter(Boolean)
      : [];
    let text = `我们更亲近啦，现在是 L${p.level || ''}「${p.level_name || ''}」！`;
    if (unlocked.length) text += ` 新解锁：${unlocked.slice(0, 2).join('、')}`;
    say(text, 5000);

    setTimeout(() => stage.classList.remove('celebrating'), 3400);
    updateMoodBadge(p);
  }

  // F4：心情/关系轻量徽章（亲密度驱动；分段关系名 + 表情）
  function updateMoodBadge(p) {
    if (!moodBadge) return;
    const level = (p && p.level) || 1;
    const relName = (p && p.level_name) || REL_NAMES[Math.min(REL_NAMES.length - 1, Math.floor((level - 1) / 2))];
    const emoji = level >= 8 ? '💗' : level >= 5 ? '😊' : level >= 3 ? '🙂' : '🐣';
    if (moodEmoji) moodEmoji.textContent = emoji;
    if (moodText) moodText.textContent = 'L' + level + ' ' + relName;
    moodBadge.classList.remove('hidden');
  }

  // D8：全局静音开关（前端本地；切换按钮态，不采集/不朗读）
  function toggleQuietMode() {
    state.quietMode = !state.quietMode;
    if (btnMute) {
      btnMute.classList.toggle('muted', state.quietMode);
      const sound = btnMute.querySelector('.ic-sound');
      const mute = btnMute.querySelector('.ic-mute');
      if (sound && mute) { sound.style.display = state.quietMode ? 'none' : ''; mute.style.display = state.quietMode ? '' : 'none'; }
      btnMute.setAttribute('aria-pressed', state.quietMode ? 'true' : 'false');
    }
    toast(state.quietMode ? '已静音（不再朗读）' : '朗读已恢复');
  }

  // F2：环境表情状态机——久置慵懒 / 高频兴奋；被交互唤醒
  function startEnvironmentMachine() {
    state.lastInteractionTs = Date.now();
    setInterval(() => {
      const idle = Date.now() - state.lastInteractionTs;
      // 正在说话/演出中不打断
      if (bubble && !bubble.classList.contains('hidden')) return;
      // 久置 → 慵懒（sleepy）
      if (idle > ENV.lazyAfterMs && state.envPhase !== 'lazy') {
        state.envPhase = 'lazy';
        setEmotion('sleepy');
        say('有点困啦……你忙你的，我在这儿陪着你。', 2600);
      }
      // 高频点击 → 兴奋
      if (state.recentClicks.length >= ENV.exciteClicks && state.envPhase !== 'excited') {
        state.envPhase = 'excited';
        setEmotion('laugh');
        playAction('celebrate');
      }
    }, ENV.checkMs);
    // 任意交互唤醒：复位环境阶段
    const wake = () => {
      state.lastInteractionTs = Date.now();
      if (state.envPhase !== 'idle') {
        state.envPhase = 'idle';
        // v45.3：从 sleepy/lazy 唤醒时主动清掉灰化滤镜——
        // 否则久置进入 img-sleepy(saturate(.78)) 后，点一下 envPhase 回了 idle，
        // 但 DOM 上的 img-sleepy class 还在，立绘一直灰着，用户以为"控件死了"。
        resetEmotion();
      }
    };
    wrap.addEventListener('pointerdown', wake);
    window.addEventListener('keydown', wake);
  }

  // ---- v45.4：后端 panic 兜底——弹 toast 提示降级，不让用户以为进程死了 ----
  listen('qibai://panic-fatal', (event) => {
    const p = event.payload || {};
    toast('音频/子系统崩溃已降级为纯文本模式（详情见 ~/.local/share/qibai/panic.log）');
    console.warn('[qibai] backend panic:', p.msg);
  });

  // ---- qibai://intimacy-updated：本窗 persona 升级才播庆祝；F4 每次刷新关系徽章 ----
  listen('qibai://intimacy-updated', (event) => {
    const p = event.payload || {};
    if (p.persona_id && p.persona_id !== state.persona) return;
    updateMoodBadge(p);
    if (p.leveled_up === true) celebrateLevelUp(p);
  });

  // ---- v21 语音调度事件（Rust voice_ui_dispatch 路由后广播）----
  // 全局静音切换：同步按钮态/图标，不弹气泡
  listen('qibai://mute-toggle', () => { toggleQuietMode(); });
  // 极简/完整界面切换：{on:true} → 极简；{on:false} → 完整
  listen('qibai://ui-minimal', (event) => {
    const p = event.payload || {};
    setMinimal(p.on !== false);
  });

  /* ============================================================
   * 角色互动：单击=摸头(touch)，双击=戳脸(pet) + 害羞表情
   * ============================================================ */

  wrap.addEventListener('click', (e) => {
    // 拖拽位移超过阈值（pointermove ≥5px 累计）时 pointerup 置 dragged=1，单击忽略
    if (wrap.dataset.dragged === '1') { wrap.dataset.dragged = ''; return; }
    doPetClick(e);
  });
  wrap.addEventListener('dblclick', () => {
    playAction('pet');
    setEmotion('shy');
    setTimeout(resetEmotion, 2000);
  });

  /* ============================================================
   * 舞台内拖动（可选；窗口级拖动靠 drag-bar + data-tauri-drag-region；
   * 位置锁定时禁用 wrap 位移）
   * ============================================================ */

  (function makeWrapDraggable() {
    // v16：窗口拖动——Rust 侧 pet_move_by(set_position)，全舞台可拖、交互区除外。
    // v18.7：鼠标优先系统级窗口拖动（pet_start_drag → GTK begin_move_drag）：
    //   系统一次接管整段拖拽，逐帧 set_position 在无合成器 X11 下会造成透明窗
    //   重影/橙色残框；系统拖拽失败时自动回退逐帧移动（rAF 限频，一帧最多一次）。
    // v19：统一指针采样模型（修三处位移丢失）——每个 pointermove 立即更新锚点并
    //   把增量累加进 pendingDx/pendingDy，任何位移都不丢；阈值用累计值判定
    //   （|pending| 曼哈顿距离 ≥6px 才 moved=true，保留点按/拖动区分，慢拖不漏
    //   启动）；moved 后 rAF 单帧批量 flush（快照→清零→invoke，回调内同步完成，
    //   与 pointermove 交错安全），挂起期间新位移只累加、由已挂起帧统一发出。
    let pid = null, lastX = 0, lastY = 0, moved = false;
    let sysDrag = false, fallbackDrag = false, rafId = null, dragArmed = false;
    let dragPending = false;
    let pendingDx = 0, pendingDy = 0;
    // v44 拖拽收尾根治状态：
    //  - settling：幂等门，settle() 一次拖拽只跑一次（防 .then 与 pointerup 双触发）。
    //  - settleTimer：看门狗。GTK begin_move_drag 在 X11 下会 nested loop 阻塞到松手，
    //    pet_start_drag 的 .then 到达即收尾；但若某些 WM 让 start_dragging 立即返回且
    //    webview 又收不到 pointerup，状态会卡死 → 3s 看门狗强制收尾。
    let settling = false;
    let settleTimer = null;
    const clearSettleTimer = () => {
      if (settleTimer) { clearTimeout(settleTimer); settleTimer = null; }
    };
    // 快照 pending → 清零 → 发送。JS 单线程下快照与清零在同一同步块内完成，
    // 不会与 pointermove 交错重复发送，也不会清零后丢新位移。
    const flushPending = () => {
      const dx = Math.round(pendingDx);
      const dy = Math.round(pendingDy);
      pendingDx = 0; pendingDy = 0;
      if (dx || dy) invoke('pet_move_by', { dx, dy }).catch(() => {});
    };
    // 统一收尾：清残影 + 吸附 + 复位。幂等。suppressClick=true 时压制下一次 click
    // （真拖过不能误触发摸头 doPetClick；纯点按放行）。
    const settle = (suppressClick) => {
      if (settling) return;
      settling = true;
      clearSettleTimer();
      if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
      pendingDx = 0; pendingDy = 0;
      if (suppressClick) wrap.dataset.dragged = '1';
      if (isTauri) repaintFrame(); // 整帧覆盖旧像素
      hardRepaint();               // X11 窗口层硬重绘清残影
      // 贴边吸附：snap 是「窗口跳变到边缘」，残影在旧位置，必须 snap 完成后再重绘一次。
      const repaintAfter = () => { if (isTauri) repaintFrame(); return hardRepaint(); };
      invoke('pet_snap_to_edge').then(repaintAfter, repaintAfter);
      sysDrag = false; fallbackDrag = false; dragPending = false;
      pid = null; // 忽略迟到的 pointerup/pointercancel
    };
    stage.addEventListener('pointerdown', (e) => {
      // v44.1 拖拽取证日志（一分三：没进 pointerdown / 进了但被前置条件挡 / IPC 没到）
      // console.log 只进 webview 控制台不进 app.log；必须走 debug_log 才能落进
      // 脚本收的进程 stdout/stderr（app.log）。
      const _dl = '[qibai] drag: pointerdown target=' + (e.target && e.target.id || e.target && e.target.className || '?')
        + ' inWrap=' + !!e.target.closest('#characterWrap')
        + ' locked=' + !!state.locked
        + ' pointerType=' + e.pointerType + ' button=' + e.button;
      console.log(_dl);
      try { invoke('debug_log', { msg: _dl }); } catch (e) {}
      // v18.8：只有点角色本身才允许拖动（CSS 已屏蔽空白区，双保险）
      if (!e.target.closest('#characterWrap')) { return; }
      if (state.locked) return;
      if (e.pointerType === 'mouse' && e.button !== 0) return;
      if (e.target.closest('.chat-bar, .mic-btn, .send-btn, #chatInput, .speech-bubble, .listen-tag, .quick-cluster')) return;
      pid = e.pointerId;
      moved = false; settling = false;
      lastX = e.clientX; lastY = e.clientY;
      pendingDx = 0; pendingDy = 0;
      sysDrag = false; fallbackDrag = false; dragArmed = false;
      // v44：恢复 GTK 系统级窗口拖动。v43 改成逐帧 set_position 后，在透明 X11 窗上
      // 每帧移动都会触发整窗重绘 → 闪烁；且窗口跟着指针移动会让 clientX 增量趋近 0，
      // 形成「拖不动」的反馈环。系统拖拽由 WM 一次接管，平滑且不重绘。
      dragArmed = (e.pointerType === 'mouse');
      if (dragArmed && isTauri) {
        dragPending = true;
        // 看门狗兜底：正常 .then 会清掉它；只有 promise 悬挂/无 pointerup 时才触发。
        clearSettleTimer();
        settleTimer = setTimeout(() => {
          if (dragPending || sysDrag) {
            // 无法确知是否位移：保守按「拖过」收尾（清残影 + 压制 click）。
            settle(true);
          }
        }, 3000);
        invoke('pet_start_drag').then((res) => {
          dragPending = false; sysDrag = true;
          // GTK begin_move_drag nested loop 阻塞到松手 → 本回调到达即拖拽结束。
          // res.moved 由 Rust 比对起止窗口坐标得出，是「真拖过」的确定性信号：
          // moved=true → 压制 click；moved=false（纯点按）→ 放行 click。
          settle(!!(res && res.moved));
        }).catch(() => {
          // 系统拖拽不可用（极少数 WM）：退回窗口内 capture + 增量移动，pointerup 正常到达。
          // 日志锚点：区分「IPC 到了但 WM 拒绝/命令报错」与「IPC 根本没到」。
          try { invoke('debug_log', { msg: '[qibai] drag: fallback armed (system drag unavailable, IPC reached but rejected)' }); } catch (_) {}
          clearSettleTimer();
          dragPending = false; sysDrag = false;
          fallbackDrag = true;
          try { stage.setPointerCapture(pid); } catch (_) {}
        });
        return;
      }
      fallbackDrag = true;
      try { stage.setPointerCapture(pid); } catch (_) {}
    });
    stage.addEventListener('pointermove', (e) => {
      if (e.pointerId !== pid) return;
      // 系统拖拽已接管或正在等待接管：绝不再叠加手动移动（双路移动是闪烁/抖动的根因）。
      if (sysDrag || dragPending) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      lastX = e.clientX; lastY = e.clientY;   // 锚点立即更新：增量先入队，位移不丢
      pendingDx += dx; pendingDy += dy;
      if (!moved) {
        // 阈值用累计位移判定（旧逻辑按单事件增量判定，慢拖每事件 <6px 永远漏启动）
        if (Math.abs(pendingDx) + Math.abs(pendingDy) < 6) return;
        moved = true;
        // 阈值前累计位移一次性发出（清零 pending，保证 pet 与指针零偏差）。
        flushPending();
        return;
      }
      // moved 之后：rAF 单帧批量（挂起期间新位移只累加，由已挂起帧统一发出）
      if (rafId === null) {
        rafId = requestAnimationFrame(() => {
          rafId = null;
          flushPending();
          // v44：拖动中不每帧硬重绘。透明 X11 下每次 full repaint 都先清空再重画，
          // 正是拖动闪烁的直接来源；残影统一在松手 settle 时清一次。
        });
      }
    });
    const up = (e) => {
      if (e.pointerId !== pid) return;
      // 系统拖拽路径：收尾由 pet_start_drag 的 .then 负责（GTK 阻塞到松手）。
      // 若 .then 已 settle，则 settling=true 且 pid=null，本函数不会被迟到事件触发。
      if (sysDrag || dragPending) return;
      pid = null;
      clearSettleTimer();
      if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
      if (moved) {
        flushPending(); // 尾段位移 flush：松手不丢最后几像素
        settle(true);   // fallback 真拖过：压制 click + 清残影 + 吸附
      } else {
        pendingDx = 0; pendingDy = 0; // 纯点按：pending 丢弃，不发任何 invoke
        fallbackDrag = false;
      }
    };
    stage.addEventListener('pointerup', up);
    stage.addEventListener('pointercancel', up);
  })();

  /* ============================================================
   * 身份解析：await QibaiBridge.getWindowLabel()（组织者扩展）
   * mock 返回 __qibaiWindowLabel；Tauri 返回 window_identity 的 label
   * ============================================================ */

  async function resolveWindowLabel() {
    if (typeof Q.getWindowLabel === 'function') {
      try {
        const label = await Q.getWindowLabel();
        if (label) return label;
      } catch (e) { /* 继续回退链 */ }
    }
    if (typeof Q.windowLabel === 'string' && Q.windowLabel && Q.windowLabel !== 'browser') {
      return Q.windowLabel;
    }
    try {
      const id = await invoke('window_identity');
      if (id && id.label) return id.label;
    } catch (e) { /* 继续回退 */ }
    return window.__qibaiWindowLabel || 'browser';
  }

  /* ============================================================
   * 事件订阅（全部广播，按本窗 persona 过滤）
   * ============================================================ */

  // ---- qibai://reminder：全部桌宠窗演出 overlay + action ----
  function showReminder(p) {
    // 同一时刻顶部只保留一张卡；新提醒替换旧提醒并重新计时，旧计时器不得关掉新卡。
    hideSpeechBubble();
    if (reminderTimer) { clearTimeout(reminderTimer); reminderTimer = null; }
    if (reminderPulseTimer) { clearTimeout(reminderPulseTimer); reminderPulseTimer = null; }
    toastBox.querySelectorAll('.toast').forEach((t) => t.remove());
    reminderTag.textContent = REMINDER_TAGS[p.type] || '小提醒';
    reminderMsg.textContent = p.message || REMINDER_MESSAGES[p.type] || '';
    reminderOverlay.classList.add('show');
    reminderOverlay.scrollTop = 0;
    hardRepaint(0);

    if (p.action) playAction(p.action);
    stage.classList.add('reminder-pulse');
    reminderPulseTimer = setTimeout(() => {
      stage.classList.remove('reminder-pulse');
      reminderPulseTimer = null;
    }, 1800);
    reminderTimer = setTimeout(() => hideReminder(), isTauri ? 9000 : 15000);
  }
  function hideReminder() {
    if (reminderTimer) { clearTimeout(reminderTimer); reminderTimer = null; }
    if (reminderPulseTimer) { clearTimeout(reminderPulseTimer); reminderPulseTimer = null; }
    stage.classList.remove('reminder-pulse');
    reminderOverlay.classList.remove('show');
    hardRepaint(0);
  }
  listen('qibai://reminder', (event) => showReminder(event.payload || {}));
  reminderClose.addEventListener('click', () => hideReminder());

  // ---- qibai://llm-status：thinking → focused 表情 ----
  listen('qibai://llm-status', (event) => {
    const p = event.payload || {};
    if (p.state === 'thinking') {
      stage.classList.add('thinking');
      state.prevEmotionOnThinking = state.emotion;
      setEmotion('focused');
    } else {
      stage.classList.remove('thinking');
      if (state.prevEmotionOnThinking) setEmotion(state.prevEmotionOnThinking);
      else resetEmotion();
      state.prevEmotionOnThinking = null;
    }
  });

  // ---- qibai://bubble-dismiss：语音「关掉气泡」→ 气泡立即消失 ----
  listen('qibai://bubble-dismiss', () => {
    hideSpeechBubble();
    hideReminder();
  });

  /* ================= v18 UI 面板·气泡卡片 ================= */
  // 白名单消毒（与 qibai-core::panel / panel.js 一致；DOM 级兜底）
  const PANEL_ALLOWED_TAGS = new Set([
    'table','thead','tbody','tfoot','tr','th','td','caption',
    'ul','ol','li','div','span','p','br','b','strong','i','em',
    'u','h1','h2','h3','h4','h5','small','code','pre','details',
    'summary','a'
  ]);
  const PANEL_ALLOWED_ATTRS = new Set(['class','id','colspan','rowspan','href','title','align']);
  function panelSafeHref(v) {
    v = (v || '').trim().toLowerCase();
    return v.indexOf('http://') === 0 || v.indexOf('https://') === 0 ||
      v.indexOf('#') === 0 || v.indexOf('/') === 0;
  }
  function panelSanitizeNode(node) {
    if (!node || node.nodeType !== 1) return null;
    const tag = node.tagName.toLowerCase();
    if (!PANEL_ALLOWED_TAGS.has(tag)) return null;
    Array.prototype.slice.call(node.attributes).forEach(function (attr) {
      const name = attr.name.toLowerCase();
      if (!PANEL_ALLOWED_ATTRS.has(name)) { node.removeAttribute(attr.name); return; }
      if (name === 'href' && !panelSafeHref(attr.value)) { node.removeAttribute(attr.name); return; }
      if (attr.value.indexOf('<') >= 0 || attr.value.indexOf('>') >= 0 || attr.value.indexOf('"') >= 0) {
        node.removeAttribute(attr.name);
      }
    });
    Array.prototype.slice.call(node.childNodes).forEach(function (child) {
      if (child.nodeType === 1) {
        if (!panelSanitizeNode(child)) node.removeChild(child);
      } else if (child.nodeType === 3) {
        child.nodeValue = child.nodeValue.replace(/<\//g, '&lt;/');
      } else {
        node.removeChild(child);
      }
    });
    return node;
  }
  function panelSanitize(html) {
    const doc = new DOMParser().parseFromString(html || '', 'text/html');
    const kids = Array.prototype.slice.call(doc.body.childNodes);
    const keep = [];
    kids.forEach(function (child) {
      if (child.nodeType === 1 && panelSanitizeNode(child)) keep.push(child);
    });
    const out = document.createElement('div');
    keep.forEach(function (n) { out.appendChild(n); });
    return out.innerHTML;
  }

  // 气泡卡片状态：最近一次 payload（展开用）
  let uiPanelPayload = null;

  function showUiPanel(title, html) {
    if (!uiPanel || !uiPanelBody) return;
    uiPanelPayload = { title: title, html: html };
    if (uiPanelTitle) uiPanelTitle.textContent = title || '元气小小';
    uiPanelBody.innerHTML = panelSanitize(html) ||
      '<p style="color:#A893A5;font-size:12px;padding:6px 2px;">暂无内容～</p>';
    uiPanel.classList.remove('hidden');
    if (state.uiPanelTimer) { clearTimeout(state.uiPanelTimer); state.uiPanelTimer = null; }
    // 30 秒无操作自动收起
    state.uiPanelTimer = setTimeout(function () {
      if (uiPanel) uiPanel.classList.add('hidden');
    }, 30000);
  }

  function hideUiPanel() {
    if (uiPanel) uiPanel.classList.add('hidden');
    if (state.uiPanelTimer) { clearTimeout(state.uiPanelTimer); state.uiPanelTimer = null; }
  }

  listen('qibai://ui-panel', (event) => {
    const p = event.payload || {};
    showUiPanel(p.title, p.html);
  });

  // 展开为独立面板窗（大内容）
  if (uiPanelExpand) {
    uiPanelExpand.addEventListener('click', function () {
      if (!uiPanelPayload) return;
      if (isTauri && invoke) {
        invoke('ui_panel_show', { payload: {
          mode: 'window',
          title: uiPanelPayload.title,
          html: uiPanelPayload.html
        } }).catch(function () {});
      }
      hideUiPanel();
    });
  }
  // 关闭气泡卡片
  if (uiPanelClose) {
    uiPanelClose.addEventListener('click', hideUiPanel);
  }

  // ---- qibai://listen-state：免提聆听状态（待唤醒/聆听中）提示 ----
  listen('qibai://listen-state', (event) => {
    const p = event.payload || {};
    const tag = document.getElementById('listenTag');
    if (!tag) return;
    if (p.state === 'active') {
      tag.textContent = '● 聆听中（说「结束」退出）';
      tag.classList.remove('hidden');
    } else if (p.state === 'wake') {
      tag.textContent = '● 待唤醒（说「' + (p.wake_word || '元气小小') + '」）';
      tag.classList.remove('hidden');
    } else {
      tag.classList.add('hidden');
    }
    // v21.4：自动消失兜底——状态条不常驻碍眼（8s 后前端自行隐藏，
    // 位置也已移到输入条上方，不再被立绘遮挡）
    if (tag.classList.contains('hidden')) {
      clearTimeout(tag._hideTimer);
    } else {
      clearTimeout(tag._hideTimer);
      tag._hideTimer = setTimeout(() => tag.classList.add('hidden'), 8000);
      // v21.5：气泡/状态条显示时强制重绘——无合成器下新出现的元素
      // 可能残留在旧合成顺序，重绘确保其绘制在立绘之上（配合 z-index 900）。
      if (isTauri) hardRepaint();
    }
  });

  // ---- B 线双工：Pipecat 语音链路事件 → 气泡联动（聊天面板随语音更新） ----
  // Rust B 线（listener.rs pipecat_event_loop）emit：
  //   qibai://pipecat-transcript  { text } 用户语音转写
  //   qibai://pipecat-reply      { delta } 小小回复文本
  listen('qibai://pipecat-transcript', (event) => {
    const text = String(((event.payload || {}).text) || '').trim();
    if (!text) return;
    // B 线转写也走统一气泡状态机；Rust 已负责音频，禁止前端重复朗读。
    say(text, 2500, true);
  });
  listen('qibai://pipecat-reply', (event) => {
    const delta = String(((event.payload || {}).delta) || '').trim();
    if (!delta) return;
    // 只显示不朗读：B 线音频已由 Rust 播放，skipTts 防双响
    say(delta, 4000, true);
  });

  // ---- qibai://pet-say：仅 persona_id 匹配本窗才演出 ----
  listen('qibai://pet-say', (event) => {
    const p = event.payload || {};
    if (p.persona_id !== state.persona) return;

    // 去重：pk-round 本地已计划逐条演出的台词，忽略总线重复广播
    const now = Date.now();
    for (const [text, exp] of plannedLines) {
      if (now > exp) { plannedLines.delete(text); continue; }
      if (text === p.text) return;
    }

    // v42：performSpeech → nextSpeech 是唯一气泡入口；rust_played 只决定是否跳过前端朗读。
    // 旧代码在这里再 say 一次，同一句会立即显示、排队后再显示并重置计时。
    performSpeech(p.text || '', p.emotion, p.action, p.ms || 3500, p.rust_played === true);
  });

  // ---- v23 qibai://pet-perform：自然语言/LLM 显式表情动作演出（后端 Rust 权威路由广播）----
  listen('qibai://pet-perform', (event) => {
    const payload = event.payload || {};
    const expr = payload.expr || '';
    if (!expr) return;
    console.log('[qibai] pet-perform 演出: ' + expr);
    performExpress(expr);
  });

  // ---- v24 qibai://pet-hide / pet-show：UI 开关调度窗口显隐 ----
  listen('qibai://pet-hide', () => { togglePetWindow(false); });
  listen('qibai://pet-show', () => { togglePetWindow(true); });

  // ---- qibai://toast：带 persona_id 时对应桌宠窗显示气泡 ----
  listen('qibai://toast', (event) => {
    const p = event.payload || {};
    if (p.persona_id && p.persona_id !== state.persona) return;
    if (!p.text) return;
    if (p.persona_id) say(p.text, 2500);
    else toast(p.text);
  });

  // 退出 PK 演出态（幂等：事件与本地定时器都会调）
  let pkEndTimer = null;
  function finishPk() {
    if (pkEndTimer) {
      clearTimeout(pkEndTimer);
      pkEndTimer = null;
    }
    state.pkPerforming = false;
    pkCard.classList.add('hidden');
    resetEmotion();
  }

  // ---- qibai://pk-round：双方桌宠窗手势动画 + 结果情绪 + 本窗台词演出 ----
  listen('qibai://pk-round', (event) => {
    const p = event.payload || {};
    if (p.a !== state.persona && p.b !== state.persona) return;

    state.pkPerforming = true;
    const myMove = p.a === state.persona ? p.move_a : p.move_b;

    // 手势卡：先摇拳，再亮出本窗手势
    pkCard.classList.remove('hidden');
    pkHandIcon.innerHTML = MOVE_SVG.rock;
    pkHandIcon.classList.add('shaking');
    pkResultTag.textContent = '出拳中…';

    setTimeout(() => {
      pkHandIcon.classList.remove('shaking');
      pkHandIcon.innerHTML = MOVE_SVG[myMove] || MOVE_SVG.rock;

      // result 情绪
      if (p.blocked) {
        setEmotion('worried');
        pkResultTag.textContent = '今天玩过啦';
      } else if (p.result === 'draw') {
        setEmotion('smiling');
        pkResultTag.textContent = '平局！';
      } else if (p.winner === state.persona) {
        setEmotion('happy');
        pkResultTag.textContent = '赢啦！';
      } else if (p.loser === state.persona) {
        setEmotion('worried');
        pkResultTag.textContent = '哎呀…';
      } else {
        pkResultTag.textContent = '对局结束';
      }

      // lines 中 speaker===本窗 persona 的台词逐条本地演出（带去重）
      const myLines = (p.lines || []).filter((l) => l.speaker === state.persona);
      myLines.forEach((l, i) => {
        plannedLines.set(l.text, Date.now() + 8000);
        setTimeout(() => say(l.text, 1500), 900 + i * 1400);
      });

      // 演出收尾：后端只发 pk-round，没有 pk-end。按本窗台词时长自行复位，
      // 否则 pkPerforming 会一直停在 true、手势卡永远挂在桌面上（点了也不消失）。
      const tail = myLines.length
        ? 900 + (myLines.length - 1) * 1400 + 1500
        : 0;
      if (pkEndTimer) clearTimeout(pkEndTimer);
      pkEndTimer = setTimeout(finishPk, tail + 400);
    }, 850);
  });

  // 注：后端只发 pk-round、不发 pk-end，演出收尾由上面的本地定时器负责，
  // 因此这里不再挂一个永远等不到事件的监听。

  // ---- qibai://desk-changed（仅浏览器 mock 生效：真机由后端直接关窗） ----
  listen('qibai://desk-changed', (event) => {
    if (isTauri) return; // 真机下窗口生命周期由后端管理
    const p = event.payload || {};
    if (Array.isArray(p.present) && !p.present.includes(state.persona)) {
      say('我先回去啦，需要我时再叫我～', 2500);
      setTimeout(() => {
        document.body.style.transition = 'opacity .6s ease';
        document.body.style.opacity = '0';
        setTimeout(() => { document.body.style.display = 'none'; }, 650);
      }, 2600);
    }
    // 同步锁定状态
    if (p.locked && typeof p.locked[state.persona] === 'boolean') {
      setLocked(p.locked[state.persona]);
    }
  });

  // v45 热切换：Rust 不重建窗，直接通知本窗换形象。
  // 清屏 → applyPersona（换立绘/姿态表/音色标签/2D-3D 引擎）→ 整帧重绘 → 播问候。
  listen('qibai://persona-hot-swap', (event) => {
    const p = event.payload || {};
    const id = p.persona_id;
    if (!id || !PERSONAS[id]) return;
    if (id === state.persona) return;
    console.log('[qibai] hot-swap persona →', id);
    hardRepaint();            // 先清窗口表面旧立绘残影
    applyPersona(id);         // 窗内热换（含 loadCharacterArt / 引擎 setPersona / 音色）
    repaintFrame();           // 整帧覆盖
    refreshLockState();
    refreshVoiceTag();
    // 热切换后播一次欢迎动作（首帧立绘就绪后）
    const greet = () => { if (state.assetReady && !state.actingFrame) { playAction('wave'); return; } if (state._greetTries < 12) { state._greetTries = (state._greetTries||0)+1; setTimeout(greet, 150); } };
    setTimeout(greet, 120);
  });

  /* ============================================================
   * 右键菜单（自定义 HTML，浏览器与 Tauri 均可用）
   * ============================================================ */

  function setLocked(locked) {
    state.locked = !!locked;
    ctxLockText.textContent = state.locked ? '解锁位置' : '锁定位置';
    lockBadge.classList.toggle('hidden', !state.locked);
    document.body.classList.toggle('locked', state.locked);
  }

  async function refreshLockState() {
    try {
      const desk = await invoke('get_desk_state');
      if (desk && typeof desk.locked === 'object' && desk.locked) {
        setLocked(!!desk.locked[state.persona]);
      }
    } catch (e) { /* mock 未就绪时保持默认 */ }
  }

  function openCtxMenu(x, y) {
    // v21.9（覆盖方案）：立绘**不隐藏**（视觉连续），菜单 z-index 1000 直接
    // 覆盖在立绘上方；立绘容器 pointer-events:none——即使合成顺序异常，点击
    // 也能穿透立绘命中菜单（点击必达）。气泡/状态条仍隐藏避免叠菜单。
    if (isTauri) {
      const wrap = document.getElementById('characterWrap');
      if (wrap) wrap.style.pointerEvents = 'none';
      dismissBubble('ctx-open'); // v45：统一清屏，不再只 add('hidden') 留旧字残影
      const lt = document.getElementById('listenTag');
      if (lt) lt.classList.add('hidden');
    }
    // v21.8（真机标准）：菜单打开即显示（inline display 恢复 + class 移除）。
    ctxMenu.classList.remove('hidden');
    ctxMenu.style.display = ''; // v21.1：恢复内联 display（closeCtxMenu 销毁式隐藏过）
    // v20.8：JS 动态限高（calc(100vh) 在透明窗 webview 可能异常）——菜单 7 项
    // 超出窗口时出现内部滚动，底部「退出元气小小」不再被截断。
    ctxMenu.style.maxHeight = Math.max(60, window.innerHeight - 8) + 'px';
    ctxMenu.style.overflowY = 'auto';
    // v20.5：立即强制重绘——否则立绘旧像素残留在窗口表面，菜单依旧被盖。
    if (isTauri) hardRepaint();
    // D6：tauri 小窗（220~230px）里菜单固定贴左缘，宽度恒 150，不跟光标 clamp——
    // 避免菜单实测宽度随布局时序漂移导致右缘越窗。浏览器宽窗仍按光标 clamp。
    const isTauriMode = document.body.classList.contains('tauri-mode');
    if (isTauriMode) ctxMenu.style.width = '150px';
    const rect = ctxMenu.getBoundingClientRect();
    const pw = rect.width || 168;
    const ph = rect.height || 132;
    const iw = window.innerWidth, ih = window.innerHeight;
    let px = x;
    if (isTauriMode) {
      // 贴左缘：菜单右缘 = 4 + pw，恒 < iw（pw≈150 < 230）
      px = 4;
    } else {
      if (px + pw > iw - 4) px = iw - pw - 4;
      if (pw >= iw) px = Math.max(0, (iw - pw) / 2);
      px = Math.max(4, Math.min(px, iw - pw - 4));
    }
    // v21.2：Tauri 下菜单**贴顶展开**（py=4）——从窗口顶部向下排，配合压缩后
    // 约 194px 的菜单高度，底部必然完整显示，杜绝「下面被截断显示不全」。
    // （跟随鼠标 y 时，若右键点在窗口下方，菜单向下溢出窗口下缘=截断。）
    let py = isTauriMode ? 4 : y;
    if (py + ph > ih - 4) py = ih - ph - 4;
    py = Math.max(4, py);
    ctxMenu.style.left = px + 'px';
    ctxMenu.style.top = py + 'px';
    refreshLockState();
    // v21.6：重置「进入过」标记；800ms 兜底——若鼠标从未进入菜单
    // （右键点在窗口另一侧），菜单不悬挂常驻，自动销毁；鼠标在菜单上则
    // 交给 mouseleave 关闭。两者保证「移开即关」且「不误关」。
    ctxMenu._everEntered = false;
    clearTimeout(ctxMenu._openCheck);
    ctxMenu._openCheck = setTimeout(() => {
      if (!ctxMenu.matches(':hover') && !ctxMenu.classList.contains('hidden')) {
        closeCtxMenu();
      }
    }, 800);
  }
  function closeCtxMenu() {
    // v44 幂等早退：菜单本就处于隐藏态（class hidden 且内联 display:none）时直接
    // 返回，不碰 #characterWrap、不 hardRepaint。根因：Tauri 右键走独立菜单窗，
    // 窗内 ctxMenu 永远 hidden；旧实现每次 document click 都无条件把 wrap
    // display:none → 异步恢复，~50ms 内命中测试漏 #characterWrap，吃掉连续拖拽/
    // 双击。现在只有「可见→隐藏」这一次过渡才执行 v21.10 二次清屏。
    const hiddenByClass = ctxMenu.classList.contains('hidden');
    const hiddenByInline = ctxMenu.style.display === 'none';
    if (hiddenByClass && hiddenByInline && !ctxMenu._clearing) {
      return;
    }
    // 二次清屏进行中：再次调用直接返回，避免 hardRepaint 的 Promise 交叠把 wrap 的
    // display/pointerEvents 状态踩乱。
    if (ctxMenu._clearing) return;
    // v21.8（真机标准）：标准浮层关闭——菜单隐藏（class+内联 display 双保险，
    // 确保任何样式表都盖不住）+ 立绘容器恢复 + 一次整窗重绘。不铺色、不
    // remove、不 hide/show——标准 WebView 下 display:none 即彻底不绘制。
    ctxMenu.classList.add('hidden');
    ctxMenu.style.display = 'none';
    if (isTauri) {
      // v21.10「二次清屏」：菜单藏在小女孩背后不消失 = display:none 的菜单
      // 旧像素残留在窗口表面，立绘恢复后被「夹」在中间层。解决：先隐藏立绘
      // → 整窗重绘（表面清空，菜单残留一并清掉）→ 恢复立绘+交互 → 再整窗
      // 重绘（立绘覆盖一切）。立绘隐藏仅 ~50ms，肉眼不可感知的瞬时空白。
      const wrap = document.getElementById('characterWrap');
      if (wrap) {
        ctxMenu._clearing = true;
        wrap.style.display = 'none';
        hardRepaint().then(() => {
          if (wrap) {
            wrap.style.display = '';        // 先恢复 display
            wrap.style.pointerEvents = '';  // 再恢复 pointerEvents
          }
          return hardRepaint();            // 最后第二次整窗重绘
        }).then(() => {
          ctxMenu._clearing = false;
        });
      }
    }
  }

  window.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (isTauri) {
      // v21.11.8：先整窗重绘清掉小窗表面历史残留（无合成器下旧浮层像素不清 =
      // 右键后「两个菜单叠着」：小窗内旧菜单残影 + 新独立菜单窗），再弹独立菜单窗。
      hardRepaint();
      // v21.11（架构根治）：真机右键 → 独立不透明菜单窗（关闭即销毁、无残留、
      // 不遮挡立绘）。不再使用窗内浮层（透明窗无合成器下浮层显隐残留清不掉）。
      invoke('ctx_menu_open', { x: Math.round(e.clientX), y: Math.round(e.clientY) }).catch(() => {});
      return;
    }
    openCtxMenu(e.clientX, e.clientY);
  });
  // v21.4（按用户方案）：**鼠标移开菜单区域 → 选项卡自动消失 + 人物重绘**
  // v21.5：120ms 延迟防手抖（鼠标快速划过菜单边缘不误关；真正移开才关）。
  ctxMenu.addEventListener('mouseleave', () => {
    clearTimeout(ctxMenu._leaveTimer);
    ctxMenu._leaveTimer = setTimeout(closeCtxMenu, 120);
  });
  ctxMenu.addEventListener('mouseenter', () => clearTimeout(ctxMenu._leaveTimer));
  // 触屏长按呼出
  let longPressTimer = null;
  window.addEventListener('touchstart', (e) => {
    if (isTauri) return; // v21.11.8：Tauri 用独立菜单窗，长按不再显示窗内浮层（防浮层+菜单窗双显示）
    const t = e.touches[0];
    longPressTimer = setTimeout(() => openCtxMenu(t.clientX, t.clientY), 500);
  }, { passive: true });
  window.addEventListener('touchend', () => { clearTimeout(longPressTimer); });
  window.addEventListener('touchmove', () => { clearTimeout(longPressTimer); });
  // 点空白处关闭（v44：仅当窗内浮层确实可见时才关，避免每次点角色都触发无谓清屏）
  document.addEventListener('click', (e) => {
    if (!ctxMenu.classList.contains('hidden') && !ctxMenu.contains(e.target)) closeCtxMenu();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') closeCtxMenu();
  });
  // v21.4（按用户方案）：鼠标移开菜单 → 选项卡自动消失 + 人物同步重绘。
  // 点击菜单项时鼠标不离开菜单（mousedown+mouseup 均在菜单内），click 正常
  // 触发；真正移开鼠标才触发本事件，两者互不干扰。
  // v21.6（修「点不出菜单」）：只有鼠标**进入过菜单再离开**才自动关闭——
  // 右键点完鼠标通常还在菜单外（如窗口右侧），若菜单一出现就按 mouseleave
  // 判「移开」销毁，用户根本来不及把鼠标移过去点选项（菜单闪一下就没）。
  ctxMenu._everEntered = false;
  ctxMenu.addEventListener('mouseenter', () => {
    ctxMenu._everEntered = true;
    clearTimeout(ctxMenu._leaveTimer);
  });
  ctxMenu.addEventListener('mouseleave', () => {
    if (!ctxMenu._everEntered) return; // 从未进入过菜单：不自动关（等点击/空白/兜底）
    clearTimeout(ctxMenu._leaveTimer);
    ctxMenu._leaveTimer = setTimeout(closeCtxMenu, 300);
  });

  // v21.11：动作执行器（浏览器浮层按钮 + 独立菜单窗 emit 共用同一套逻辑）
  function doConsoleAction() {
    closeCtxMenu();
    invoke('console_open').catch(() => { toast('控制台打开失败'); });
  }
  function doLockAction() {
    const next = !state.locked;
    closeCtxMenu();
    invoke('pet_set_locked', { persona_id: state.persona, locked: next })
      .then(() => { setLocked(next); toast(next ? '位置已锁定' : '位置已解锁'); })
      .catch(() => { toast('锁定操作失败'); });
  }
  // 极简/完整界面切换（默认极简：只角色+输入条；完整=按钮簇/状态标签等全显）
  function setMinimal(on) {
    document.body.classList.toggle('minimal', !!on);
    if (ctxMinimalText) ctxMinimalText.textContent = on ? '完整界面' : '极简界面';
  }
  function doMinimalAction() {
    closeCtxMenu();
    setMinimal(!document.body.classList.contains('minimal'));
  }
  function doExitAction() {
    closeCtxMenu();
    invoke('app_exit').catch(() => { toast('浏览器演示环境：退出操作由系统处理'); });
  }
  // 迭代①·全屏守护：右键「守护我」→ guard_trigger（含 enabled/冷却校验）
  function doGuardAction() {
    closeCtxMenu();
    invoke('guard_trigger', {}).then(() => { toast('全屏守护开始啦'); })
      .catch((e) => { toast((e && e.message) ? e.message : '守护触发失败'); });
  }

  // 浏览器/旧版浮层按钮绑定（Tauri 真机下菜单已迁到独立窗，此绑定保留作 fallback）
  ctxConsole.addEventListener('click', doConsoleAction);
  // v30：右键菜单「本地工作区」→ 打开 ~/.qibai_workspace（小小创建工程/写代码的落盘目录）
  const ctxWorkspace = $('ctxWorkspace');
  if (ctxWorkspace) {
    ctxWorkspace.addEventListener('click', () => {
      if (isTauri) {
        invoke('code_open_workspace').then((r) => toast(r)).catch((e) => toast('打开工作区失败：' + (((e && e.message) || e) + '').slice(0, 50)));
      } else {
        toast('浏览器模式无本地工作区（仅 Tauri 可用）');
      }
      closeCtxMenu();
    });
  }
  // v31：打开本地知识库目录（~/.qibai_kb，QIBAI_KB 可覆盖；放文档即自动检索）
  const ctxKB = $('ctxKB');
  if (ctxKB) {
    ctxKB.addEventListener('click', () => {
      if (isTauri) {
        invoke('code_open_kb').then((r) => toast(r)).catch((e) => toast('打开知识库失败：' + (((e && e.message) || e) + '').slice(0, 50)));
      } else {
        toast('浏览器模式无本地知识库（仅 Tauri 可用）');
      }
      closeCtxMenu();
    });
  }
  // v31.1：透明背景开关（立绘外无底色；无合成器 X11 下默认保持关闭防重影/黑底）
  const ctxTransBg = $('ctxTransBg');
  const ctxTransBgText = $('ctxTransBgText');
  function refreshTransBgText() {
    if (!ctxTransBgText) return;
    ctxTransBgText.textContent = state.transparencySupported
      ? '透明背景：' + (state.transparentBg ? '开' : '关')
      : '透明背景：兼容';
  }
  function toggleTransparentBg() {
    try { invoke('debug_log', { msg: '[transbg] handler-enter state=' + state.transparentBg }); } catch (e) {}
    if (isTauri && !state.transparencySupported) {
      toast('当前 X11 渲染环境已启用安全兼容窗，避免动作叠影', 2400);
      closeCtxMenu();
      return;
    }
    state.transparentBg = !state.transparentBg;
    try { invoke('debug_log', { msg: '[transbg] after-toggle state=' + state.transparentBg + ' repaint-done' }); } catch (e) {}
    refreshTransBgText();
    repaintFrame();
    if (isTauri) {
      invoke('get_settings').then((s) => {
        if (!s || !s.ui) return;
        s.ui.transparent_bg = state.transparentBg;
        invoke('save_settings', { settings: s })
          .then(() => toast(state.transparentBg ? '透明背景：开' : '透明背景：关', 1500))
          .catch((e) => toast('保存失败：' + (((e && e.message) || e) + '').slice(0, 50)));
      }).catch(() => {});
    }
    closeCtxMenu();
  }
  if (ctxTransBg) ctxTransBg.addEventListener('click', toggleTransparentBg);
  refreshTransBgText();
  ctxLock.addEventListener('click', doLockAction);
  ctxMinimal.addEventListener('click', doMinimalAction);
  ctxExit.addEventListener('click', doExitAction);
  ctxGuard.addEventListener('click', doGuardAction);

  // v21.11：独立菜单窗动作分发（后端 emit「qibai://ctx-action」→ 本窗执行）
  if (listen) {
    listen('qibai://ctx-action', (event) => {
      // 兼容 payload 为 JSON 字符串或已解析对象两种形态
      let payload = event && event.payload;
      if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch (e) {} }
      const action = payload && payload.action;
      if (action === 'lock') doLockAction();
      else if (action === 'guard') doGuardAction();
      else if (action === 'console') doConsoleAction();
      else if (action === 'minimal') doMinimalAction();
      else if (action === 'exit') doExitAction();
      else if (action === 'transbg') {
        // 独立菜单窗与内嵌菜单共用同一开关；兼容环境不允许假开启。
        toggleTransparentBg();
      }
    })
    .then(() => { try { invoke('debug_log', { msg: '[transbg] listen-registered' }); } catch (e) {} })
    .catch((e) => { try { invoke('debug_log', { msg: '[transbg] listen-failed ' + e }); } catch (e2) {} });
  }

  /* ============================================================
   * 浏览器 mock 调试面板（仅浏览器模式）
   * ============================================================ */

  function buildDevPanel() {
    if (isTauri) return;
    const emoBox = $('devEmotions');
    EMOTIONS.forEach((e) => {
      const b = document.createElement('button');
      b.textContent = e;
      b.addEventListener('click', () => { setEmotion(e); say(`表情：${e}`, 1500); });
      emoBox.appendChild(b);
    });

    const actBox = $('devActions');
    ['wave', 'nod', 'shake', 'stretch', 'hands_on_hips', 'touch', 'pet', 'celebrate', 'bounce', 'tilt', 'wiggle', 'lean', 'point'].forEach((a) => {
      const b = document.createElement('button');
      b.textContent = a;
      b.addEventListener('click', () => playAction(a));
      actBox.appendChild(b);
    });

    const evBox = $('devEvents');
    const mkBtn = (label, fn) => {
      const b = document.createElement('button');
      b.textContent = label;
      b.addEventListener('click', fn);
      evBox.appendChild(b);
    };
    mkBtn('提醒·久坐', () => showReminder({ type: 'sit', action: 'stretch' }));
    mkBtn('提醒·喝水', () => showReminder({ type: 'water' }));
    mkBtn('提醒·护眼', () => showReminder({ type: 'eye' }));
    mkBtn('LLM thinking', () => {
      stage.classList.add('thinking');
      state.prevEmotionOnThinking = state.emotion;
      setEmotion('focused');
    });
    mkBtn('LLM done', () => {
      stage.classList.remove('thinking');
      if (state.prevEmotionOnThinking) setEmotion(state.prevEmotionOnThinking);
      else resetEmotion();
      state.prevEmotionOnThinking = null;
    });
    mkBtn('打开控制台', async () => {
      try { await invoke('console_open'); } catch (e) { toast('控制台打开失败'); }
    });
    // v18：UI 面板冒烟（浏览器 mock）
    mkBtn('面板·气泡卡片', () => {
      showUiPanel('冒烟卡片', '<table class="qibai-table"><thead><tr><th>名称</th><th>状态</th></tr></thead><tbody><tr><td>聊天记录</td><td>就绪</td></tr><tr><td>技能库</td><td>就绪</td></tr></tbody></table>');
    });
    mkBtn('面板·独立窗口', async () => {
      try {
        await invoke('ui_panel_show', { payload: { mode: 'window', title: '冒烟面板', html: '<table class="qibai-table"><thead><tr><th>角色</th><th>内容</th></tr></thead><tbody><tr><td>我</td><td>你好</td></tr><tr><td>元气小小</td><td>嗨～</td></tr></tbody></table>' } });
      } catch (e) { toast('面板窗打开失败'); }
    });

    // ---- 亲密度 dev 按钮（浏览器 mock 冒烟） ----
    const intBox = $('devIntimacy');
    const imk = (label, fn) => {
      const b = document.createElement('button');
      b.textContent = label;
      b.addEventListener('click', fn);
      intBox.appendChild(b);
    };
    imk('点按互动', () => doPetClick());
    imk('陪伴上报', () => {
      reportIntimacy('companion');
      toast('已上报 companion（10 分钟间隔仅 dev 手动触发）');
    });
    imk('升级演示', async () => {
      // 优先用 mock 播种 seed(当前等级+1)（§8.2）；未就绪则循环 gift 兜底
      try {
        const mock = window.__qibaiMock && window.__qibaiMock.intimacy;
        if (mock && typeof mock.seed === 'function') {
          let level = 1;
          try {
            const st = await invoke('get_intimacy_state', { persona_id: state.persona });
            if (st) level = st.level || 1;
          } catch (e2) { /* 读不到就从 L1 种起 */ }
          // 显式指定本窗 persona，避免 mock 默认 active persona 与本窗不一致导致广播被过滤
          const ret = await mock.seed(level + 1, state.persona);
          toast(`seed → L${(ret && ret.level) || level + 1}，等待升级广播`);
        } else {
          let leveled = false;
          for (let i = 0; i < 40 && !leveled; i++) {
            const r = await invoke('intimacy_event', { persona_id: state.persona, kind: 'gift' });
            leveled = !!(r && r.leveled_up);
          }
          if (!leveled) toast('未触发升级（可能已达每日上限）');
        }
      } catch (e) {
        toast('升级演示未就绪：亲密度 mock 尚未接入');
      }
    });
  }

  /* ============================================================
   * 冒烟 hash 钩子（仅浏览器 mock；Tauri 无副作用）
   *   #fire=wave | happy | reminder-sit | menu
   * ============================================================ */

  function parseHash() {
    const h = location.hash.replace(/^#/, '');
    if (!h) return;
    const kv = h.split('&').reduce((o, p) => {
      const parts = p.split('=');
      o[parts[0]] = parts[1] || true;
      return o;
    }, {});

    if (kv.fire === 'wave') {
      setTimeout(() => { playAction('wave'); say('嗨～我在这儿陪你。', 3000); }, 350);
    } else if (kv.fire === 'happy') {
      setEmotion('happy');
      say('心情不错呢～', 2200);
    } else if (kv.fire === 'reminder-sit') {
      setTimeout(() => showReminder({ type: 'sit', action: 'stretch' }), 300);
    } else if (kv.fire === 'menu') {
      setTimeout(() => openCtxMenu(window.innerWidth / 2 - 70, window.innerHeight / 3), 300);
    } else if (kv.fire === 'click') {
      // 冒烟：直接走点按互动入口（动作+气泡+粒子+click 上报）
      setTimeout(() => doPetClick(), 350);
    } else if (kv.fire === 'celebrate') {
      // 冒烟：合成升级 payload 直走庆祝演出（不依赖 mock seed；§10 pet-celebrate.png）
      setTimeout(() => celebrateLevelUp({
        persona_id: state.persona, level: 7, level_name: '依赖', leveled_up: true,
        unlocked: [{ label: '息屏守护' }, { label: '语音陪伴模式' }],
      }), 400);
    }
    // 兼容旧钩子
    if (kv.emotion) {
      if (state.assetReady) setEmotion(kv.emotion);
      else img.addEventListener('load', function onBaseLoad() {
        img.removeEventListener('load', onBaseLoad);
        setEmotion(kv.emotion);
      });
    }
    if (kv.action) setTimeout(() => playAction(kv.action), 400);
  }

  /* ============================================================
   * 启动
   * ============================================================ */

  async function init() {
    // body 模式类（bridge 若已设置则幂等）
    document.body.classList.add(isTauri ? 'tauri-mode' : 'browser-mode');
    // 极简默认态：只显示角色 + 输入条；右键「完整界面」呼出其余 UI
    document.body.classList.add('minimal');

    // v13 残影根治·JS 兜底：透明窗（无合成器 X11）内，CSS transform 动画
    // （breathe/sway/img-pulse/action-*）逐帧累积残影 = 「两个人物/半透明轮廓重叠」。
    // pet.css v21 已禁 Tauri 端动画，但增量包曾漏发 pet.css——这里 JS 内联硬禁，
    // 不依赖 CSS 文件是否覆盖到真机。
    if (isTauri) {
      const killAnim = () => {
        const w = document.getElementById('characterWrap');
        if (w) { w.style.animation = 'none'; w.classList.add('paused'); }
        const ci = document.getElementById('characterImg');
        if (ci) ci.style.animation = 'none';
        const fc = document.getElementById('placeholderFace');
        if (fc) { fc.style.animation = 'none'; fc.style.display = 'none'; }
      };
      killAnim();
      window.addEventListener('load', killAnim);
    }

    // v20.6：首次启动标志——启动加载链跳过 hide/show 全清（窗口未稳定，
    // 防 v20.2 启动闪退回归）；onload 成功后清掉，此后切角色才全清。
    state.initialBoot = true;

    // 身份 → persona
    // v45 热切换：窗 label 固定 pet-stage。label 能推出合法 persona（浏览器/QA mock
    // 模式 label=pet-<id>）时用 label；否则（Tauri 真机 label=pet-stage）从引擎读
    // active persona。切角色不走重建窗，走 persona-hot-swap 事件在本窗内热换。
    const label = await resolveWindowLabel();
    let persona = (label || '').replace(/^pet-/, '');
    if (!PERSONAS[persona]) {
      try {
        const ap = await invoke('get_active_persona');
        if (ap && ap.persona_id && PERSONAS[ap.persona_id]) persona = ap.persona_id;
      } catch (e) { /* mock 无此命令时保持默认 */ }
    }
    applyPersona(PERSONAS[persona] ? persona : DEFAULT_PERSONA);
    armResponsiveLayout();

    // 后端在窗口创建前判定渲染路径；前端必须使用同一事实源，不能自行假开透明。
    if (isTauri) {
      try {
        const renderInfo = await invoke('pet_render_info');
        state.transparencySupported = !(renderInfo && renderInfo.transparent === false);
      } catch (e) { state.transparencySupported = true; }
      document.body.classList.toggle('transparency-unsupported', !state.transparencySupported);
    }

    // 二次修复①：读设置，决定是否显示音色调试 chip（默认隐藏）+ 透明背景模式
    try {
      const s = await invoke('get_settings');
      state.showVoiceTag = !!(s && s.ui && s.ui.show_voice_tag);
      state.transparentBg = state.transparencySupported && !!(s && s.ui && s.ui.transparent_bg);
      // 调试钩子只在后端支持原生透明时生效，兼容模式不制造“假开启”。
      if (state.transparencySupported && /[?&]transbg=1/.test(location.search)) { state.transparentBg = true; }
      refreshTransBgText && refreshTransBgText();
      // 设置已读出：立即按当前透明模式重绘一次（立绘首帧可能已用旧背景画完）
      repaintFrame();
    } catch (e) { state.showVoiceTag = false; }
    refreshVoiceTag();

    startBlinkLoop();
    refreshLockState();

    // v44：置顶重申。Linux WM 在 unminimize / resize / 切换形态后可能丢掉
    // _NET_WM_STATE_ABOVE；启动、回焦、页面可见和每 6 秒各重申一次，不抢焦点。
    const keepOnTop = () => {
      if (!isTauri) return;
      invoke('pet_keep_on_top').catch(() => {});
    };
    keepOnTop();
    setInterval(keepOnTop, 6000);
    window.addEventListener('focus', keepOnTop);
    document.addEventListener('visibilitychange', () => { if (!document.hidden) keepOnTop(); });

    // 陪伴上报：每 10 分钟一次（规格 §8.2 / §3.1，仅本窗 persona）
    setInterval(() => reportIntimacy('companion'), 10 * 60 * 1000);

    // 开场打招呼（极简默认态不弹气泡，保持 角色+输入条；呼出完整态后仍欢迎）
    // v21.11.14：立绘就绪后才播欢迎动作——轮询等待 assetReady（≤1.8s），
    // 杜绝「立绘加载链 vs 欢迎动作帧」并发换图导致启动双像/动作重合。
    const bootWave = () => {
      if (state.assetReady && !state.actingFrame) { playAction('wave'); return; }
      if (state.bootWaveTries++ < 12) setTimeout(bootWave, 150);
    };
    setTimeout(bootWave, 350);

    parseHash();
    buildDevPanel();

    // 对话输入条绑定：发送/回车/按住说话（D3：元素存在性保护，杜绝 ReferenceError 中断 init）
    if (btnSend && chatInput) {
      btnSend.addEventListener('click', () => sendChat(chatInput.value));
      chatInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); sendChat(chatInput.value); } });
      // 空输入禁用发送钮；发送中防重（sendChat 内已有空值守卫）
      const syncSendState = () => {
        const empty = !chatInput.value.trim();
        btnSend.disabled = empty || !!state.sending;
        btnSend.style.opacity = (empty || state.sending) ? '.4' : '1';
      };
      chatInput.addEventListener('input', syncSendState);
      syncSendState();
      // listen 态联动：用户输入=倾听（微前倾/点头），失焦回 idle
      chatInput.addEventListener('focus', () => { if (state.engine && state.engine.setListen) state.engine.setListen(true); });
      chatInput.addEventListener('blur', () => { if (state.engine && state.engine.setListen) state.engine.setListen(false); });
      // 输入框右键：呼出完整 UI / 设置菜单
      chatInput.addEventListener('contextmenu', (e) => {
        e.preventDefault(); e.stopPropagation();
        openCtxMenu(e.clientX - 70, e.clientY - 60);
      });
    }
    // v25 聊天功能完善：快捷提问 chips（点击即发送；浏览器演示也无需打字）
    const chips = document.querySelectorAll('.quick-chips .chip');
    chips.forEach((c) => c.addEventListener('click', () => {
      const t = (c.dataset && c.dataset.text) || '';
      if (t) sendChat(t);
    }));
    if (btnMic && chatInput) {
      // v8-PTT：免提开关（双工）——开=常开免提聆听，关=按住说话（PTT，默认）
      // 免提状态与 Rust listener 对齐（set_wake_enabled / get_listen_state）。
      let hfOn = false;
      const btnHF = $('btnHF');
      const setHfUI = (on) => { hfOn = !!on; if (btnHF) btnHF.classList.toggle('hf-on', hfOn); };
      const refreshHf = async () => {
        try {
          const s = await invoke('get_listen_state');
          setHfUI(s && s.state && s.state !== 'off');
        } catch (e) { /* 非 tauri 或命令不可用：保持默认 PTT */ }
      };
      if (btnHF) btnHF.addEventListener('click', async () => {
        const next = !hfOn;
        try {
          await invoke('set_wake_enabled', { enabled: next });
          setHfUI(next);
          toast(next ? '免提聆听已开启（直接说话即可）' : '免提已关闭，改为按住说话');
        } catch (e) { toast('免提切换失败：' + (((e && e.message) || e) + '').slice(0, 50)); }
      });
      refreshHf();

      // 点 mic 切语音/文字模式（STT 未配置时提示；按住仍可 startRecord）
      let voiceMode = false;
      btnMic.addEventListener('click', () => {
        if (hfOn) { toast('免提已开启，直接说话即可'); return; }
        voiceMode = !voiceMode;
        btnMic.classList.toggle('voice-mode', voiceMode);
        toast(voiceMode ? '语音模式：按住麦克风说话即可' : '已切回文字输入');
      });
      const micHold = (e) => { e.preventDefault(); if (hfOn) { toast('免提已开启，直接说话即可'); return; } startRecord(); };
      const micRelease = () => { if (!hfOn) stopRecord(); };
      btnMic.addEventListener('mousedown', micHold);
      window.addEventListener('mouseup', micRelease);
      btnMic.addEventListener('touchstart', micHold, { passive: false });
      window.addEventListener('touchend', micRelease);
    }

    // D8：右侧快捷图标簇
    if (btnUser) btnUser.addEventListener('click', () => { invoke('console_open').catch(() => toast('控制台打开失败')); });
    if (btnMute) btnMute.addEventListener('click', toggleQuietMode);
    if (btnMore) btnMore.addEventListener('click', () => openCtxMenu(window.innerWidth / 2 - 70, window.innerHeight / 3));

    // D2：窗口重新获得焦点/可见时强制重绘，清透明窗合成残框（橙色残框兜底）
    const repaintNow = () => hardRepaint();
    window.addEventListener('focus', repaintNow);
    document.addEventListener('visibilitychange', () => { if (!document.hidden) repaintNow(); });
    // 页面不可见时暂停立绘 CSS 动效（性能）
    document.addEventListener('visibilitychange', () => {
      wrap.classList.toggle('paused', document.hidden);
    });

    // F2：环境表情状态机启动（久置慵懒 / 高频兴奋 / 待机呼吸）
    startEnvironmentMachine();

    // v30：canvas 整帧重绘守卫挂载（Tauri 模式）——任何 DOM 变化后整帧覆盖，
    // 根治透明区旧像素残留（动作重合/重影/拖动残留色块/提示身后重影）。
    armCanvasResidueGuard();

    // v12：启动强制重绘——无合成器 X11 下，首帧/立绘加载的旧帧残留会呈「两个人物形象
    // 重叠」；补齐启动瞬间的一次主动擦除（拖拽/聚焦重绘已有，仅缺启动时机）。
    setTimeout(() => hardRepaint(), 600);
  }

  // v18.5 冒烟：浏览器 mock 下模拟连续台词，验证演出队列（表情+动作随轮次累积切换）
  // v30.1：放开到 tauri=1 也允许（仅 URL 带 smoke 参数时生效，用于无头截图验证气泡位置；真机不带此参数）
  if (/smoke=speech/.test(location.search)) {
    const smokeLines = [
      '哈哈哈，今天太开心啦！',
      '这是怎么回事呢？',
      '我有点难过了，呜呜',
      '哼，居然敢这样！',
      '晚安啦，你也早点休息',
      '嗯，我会一直陪着你的',
    ];
    let si = 0;
    const siv = setInterval(() => {
      if (si >= smokeLines.length) { clearInterval(siv); return; }
      performSpeech(smokeLines[si], '', '');
      si++;
    }, 2000);
  }

  // 冒烟/调试钩子（生产无害）：暴露渲染引擎与状态触发
  window.__qibaiDebug = {
    say: (t, ms) => say(t, ms || 2000, true),
    setEmotion, playAction, resetEmotion, realActionFrame, inferMood,
    getEngine: () => state.engine ? (state.use3d ? '3d' : '2d') : 'none',
    getState: () => (state.engine && state.engine.getState) ? state.engine.getState() : null,
    getPersona: () => state.persona,
    setRenderMode, getRenderMode: () => state.renderMode,
  };

  init();
})();
