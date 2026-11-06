/* ============================================================
 * 元气小小 Qibai — Tauri 桥接层（tauri-bridge.js）
 * 职责：
 *   1. 统一 invoke()：Tauri 环境走原生 IPC，浏览器环境走 mock
 *      （localStorage 假实现 + 手写事件总线），保证前端无 Tauri
 *      也能完整冒烟。
 *   2. 统一 listen()：Tauri v2 事件（qibai://reminder 等）订阅，
 *      浏览器环境下由 mock 事件总线驱动。
 *
 * 后端契约（invoke 命令）：
 *   get_settings()            -> SettingsJson
 *   save_settings(s)          -> ok
 *   send_message({text})      -> {reply, emotion, action?, skill?}
 *   get_conversation()        -> [{role, content, ts}]
 *   get_skills_status()        -> {total_enabled, skills:{...}}
 *   set_skill_enabled({skill, enabled}) -> ok
 *   approve_skill({request_id, approve}) -> ok
 *   插件与专家（PLUGIN_IMPL_CONTRACT §3.6）：
 *   plugin_list()              -> [{id,type,name,description,icon,version,author,
 *                                  permissions,granted,enabled,installed,entry_kind,
 *                                  functions,error,builtin}]
 *   plugin_install({dir})       -> ok（幂等）
 *   plugin_uninstall({id})     -> ok（幂等）
 *   plugin_set_enabled({id, enabled}) -> ok
 *   plugin_grant_permission({id, permission, approve}) -> ok（files:write 外部硬拒）
 *   expert_list()             -> [{id,name,description,version,author,system_prompt,
 *                                  skills,persona_binding,active}]
 *   expert_set_active({id})    -> ok（空串 = 停用）
 *
 * 事件：
 *   qibai://reminder      {type: sit|water|eye, message, action}
 *   qibai://llm-status    {state: thinking|done|error, message?}
 *   qibai://skill-approval {request_id, skill, description}
 * ============================================================ */

(function (global) {
  'use strict';

  // ---- 运行环境判定 ----
  const isTauri = !!(
    (global.__TAURI__ && global.__TAURI__.invoke) ||
    global.__TAURI_INTERNALS__
  );

  // ---- mock 事件总线 ----
  const mockListeners = {}; // eventName -> Set<callback>

  function mockEmit(event, payload) {
    const set = mockListeners[event];
    if (set) set.forEach((cb) => {
      try { cb({ payload }); } catch (e) { console.error(e); }
    });
  }

  // ---- invoke 统一入口 ----
  async function invoke(cmd, args) {
    if (isTauri) {
      try {
        if (global.__TAURI__ && global.__TAURI__.invoke) {
          return await global.__TAURI__.invoke(cmd, args);
        }
        return await global.__TAURI_INTERNALS__.invoke(cmd, args);
      } catch (err) {
        console.error('[qibai] invoke failed:', cmd, err);
        // 诊断：转发前端错误到后端日志（失败静默，不影响主流程）
        try {
          if (global.__TAURI_INTERNALS__) {
            global.__TAURI_INTERNALS__.invoke('debug_log', { msg: 'invoke failed: ' + cmd + ' ' + String(err && err.message ? err.message : err) });
          }
        } catch (e2) { /* ignore */ }
        throw err;
      }
    }
    return mockInvoke(cmd, args || {});
  }

  // ---- listen 统一入口 ----
  // v20：unlisten 注册表——统一收集注销函数；当前窗口生命周期内无害（窗毁即毁），
  // 为「窗口复用重建」场景预留 disposeAll 清理路径。
  const unlistenFns = new Set();
  async function listen(event, handler) {
    if (isTauri) {
      // Tauri v2：优先 withGlobalTauri 暴露的 event.listen
      if (global.__TAURI__ && global.__TAURI__.event && global.__TAURI__.event.listen) {
        const un = await global.__TAURI__.event.listen(event, handler);
        if (typeof un === 'function') unlistenFns.add(un);
        return un;
      }
      // 兜底：v2 internals 的 plugin:event|listen
      // 对照 tauri-2.12.4 官方实现（scripts/bundle.global.js 的 event.listen）：
      //   invoke('plugin:event|listen', { event, target, handler: transformCallback(fn) })
      //     .then(id => async () => invoke('plugin:event|unlisten', { event, eventId: id }))
      // 三个必填参数：event / target / handler。
      // - target 必填，EventTarget 以 #[serde(tag="kind")] 序列化，{kind:'Any'} 监听所有目标，
      //   可接收后端 WebviewWindow::emit / AppHandle::emit 的全局广播。
      // - handler 必填，且必须是「IPC 回调 id」（数字），不能是裸函数：裸函数无法 JSON 序列化，
      //   序列化后丢失，后端报 missing required key handler。
      //   transformCallback 把函数注册进 __TAURI_INTERNALS__.callbacks 并返回数字 id。
      // - listen 返回的是 eventId（数字），注销须走 plugin:event|unlisten，不能当成函数调用。
      if (global.__TAURI_INTERNALS__ && global.__TAURI_INTERNALS__.invoke) {
        const internals = global.__TAURI_INTERNALS__;
        const wrapped = (ev) => {
          // 回调收到的是「完整事件对象」（含 payload 字段，见 event/emit_js_script）。
          // 兼容两种回调形态：完整事件对象 / 裸 payload。
          const normalized = ev && typeof ev === 'object' && 'payload' in ev ? ev : { payload: ev };
          handler(normalized);
        };
        const callbackId = typeof internals.transformCallback === 'function'
          ? internals.transformCallback(wrapped)
          : wrapped; // 理论上不会走到（Tauri v2 必有 transformCallback）
        const eventId = await internals.invoke('plugin:event|listen', {
          event,
          target: { kind: 'Any' },
          handler: callbackId,
        });
        const off = async () => {
          try { await internals.invoke('plugin:event|unlisten', { event, eventId }); } catch (e) { /* 忽略 */ }
          if (typeof internals.unregisterCallback === 'function') {
            try { internals.unregisterCallback(callbackId); } catch (e) { /* 忽略 */ }
          }
        };
        unlistenFns.add(off);
        return off;
      }
      console.warn('[qibai] no Tauri event API found for', event);
      return () => {};
    }
    // mock 总线
    if (!mockListeners[event]) mockListeners[event] = new Set();
    mockListeners[event].add(handler);
    const off = async () => {
      if (mockListeners[event]) mockListeners[event].delete(handler);
    };
    unlistenFns.add(off);
    return off;
  }

  // v20：注销本窗口全部事件监听（幂等）。窗口重建/卸载前调用，防监听累积。
  function disposeAllListeners() {
    unlistenFns.forEach((off) => { try { off(); } catch (e) { /* 忽略 */ } });
    unlistenFns.clear();
  }

  /* ============================================================
   * mock 适配层（浏览器独立模式）
   * ------------------------------------------------------------
   * - 所有设置/对话持久化到 localStorage
   * - send_message 返回固定剧本，emotion 按关键词/随机切换
   * - 提供 window.__qibaiMock 手动触发事件（dev 面板用）
   * ========================================================== */

  const LS_KEYS = {
    settings: 'qibai.mock.settings',
    conversation: 'qibai.mock.conversation',
    personas: 'qibai.mock.personas',
    social: 'qibai.mock.social',
    desk: 'qibai.mock.desk',
    bus: 'qibai.mock.bus',
    intimacy: 'qibai.mock.intimacy',
    guard: 'qibai.mock.guard', // 守护演出冷却状态 {last_end: ts}
    vault: 'qibai.mock.vault', // 迭代③·保险箱密封信封（qibai-vault JSON 信封）
  };

  const DEFAULT_SETTINGS = {
    llm: {
      provider: 'doubao',
      base_url: 'https://openai.app.msh.team/v1',
      model: 'doubao-pro-32k',
      api_key: '',
    },
    reminders: {
      sit_min: 60,
      water_min: 40,
      eye_min: 45,
      enabled: { sit: true, water: true, eye: true },
    },
    ui: { age_confirmed: false, pure_mode: true },
    skills: {
      total_enabled: true,
      launch_app: true,
      open_folder: false,  // v43：默认关闭，防止误触发文件夹打开
      set_reminder: true,
      check_code: true,
    },
    // 迭代①·全屏守护：久坐到期后展开全屏休息演出
    guard: {
      enabled: true,
      sit_min: 120,
      mode: 'strong', // light=轻提醒(仅通知) / strong=强提醒(演出可跳过) / lock=守护+锁屏
      end_action: 'countdown', // none / sleep / lock / countdown
      countdown_secs: 120,
    },
    // 迭代②·语音（冻结规格 §1：voice 块字段名不可变）
    voice: {
      enabled: true,             // 总开关（无可用引擎时自动静音，不发声）
      engine: 'auto',            // auto | local | cloud
      volume: 80,                // 0-100
      rate: 1.0,                 // 0.5-2.0
      scope: 'all',              // all | reminders | chat
      cloud_provider: 'doubao',  // doubao | volcano
      cloud_voice: '',           // 音色 ID，留空用引擎默认
      cloud_api_key: '',         // 空则运行时读环境变量
      stt_enabled: false,       // STT 总开关
      stt_provider: 'doubao',    // doubao | volcano
      stt_api_key: '',
    },
    intimacy_level: 1, // 预留
    // 迭代③·本地密码保险箱（冻结规格 §2：vault 段默认值）
    vault: {
      idle_lock_min: 5,        // 闲置自动锁定（分钟，1..=120）
      max_wrong_attempts: 5,    // 连续输错主密码次数
      cooldown_secs: 30,        // 冷却秒数
      clipboard_clear_secs: 20, // 剪贴板自动清除秒数
    },
    // 插件与专家（PLUGIN_IMPL_CONTRACT §2：PluginSettings 同形，增量迁移补齐）
    plugins: { enabled: {}, grants: {}, experts: {} },
  };

  function loadMockSettings() {
    try {
      const raw = localStorage.getItem(LS_KEYS.settings);
      if (raw) {
        const obj = JSON.parse(raw);
        // 增量迁移：旧版本存储补齐新默认块（skills.check_code / guard.* / voice）
        const dflt = JSON.parse(JSON.stringify(DEFAULT_SETTINGS));
        obj.skills = Object.assign({}, dflt.skills, obj.skills || {});
        obj.guard = Object.assign({}, dflt.guard, obj.guard || {});
        obj.voice = Object.assign({}, dflt.voice, obj.voice || {});
        obj.vault = Object.assign({}, dflt.vault, obj.vault || {}); // 迭代③：vault 段补齐
        // 插件段补齐（PLUGIN_IMPL_CONTRACT §2；enabled 空集合由插件层再播种）
        obj.plugins = Object.assign({}, dflt.plugins, obj.plugins || {});
        obj.plugins.enabled = Object.assign({}, dflt.plugins.enabled, (obj.plugins && obj.plugins.enabled) || {});
        obj.plugins.grants = Object.assign({}, dflt.plugins.grants, (obj.plugins && obj.plugins.grants) || {});
        obj.plugins.experts = Object.assign({}, dflt.plugins.experts, (obj.plugins && obj.plugins.experts) || {});
        return obj;
      }
    } catch (e) { /* ignore */ }
    return JSON.parse(JSON.stringify(DEFAULT_SETTINGS));
  }

  function saveMockSettings(s) {
    localStorage.setItem(LS_KEYS.settings, JSON.stringify(s));
    return 'ok';
  }

  // mock 回复剧本：按关键词匹配 emotion/action
  const MOCK_REPLIES = [
    { match: /(你好|hi|hello|在吗)/i, emotion: 'smiling', action: 'wave',
      reply: '你来啦！我一直在这儿等你呢～今天也要好好照顾自己哦。' },
    { match: /(累|疲惫|困|睡)/, emotion: 'sleepy', action: 'stretch',
      reply: '听起来你有点累了呢……要不先闭上眼睛休息五分钟？我陪着你。' },
    { match: /(生气|烦|讨厌|气)/, emotion: 'worried', action: 'nod',
      reply: '不气不气，深呼吸～我在呢。想说说发生了什么吗？' },
    { match: /(开心|高兴|太好了|耶)/, emotion: 'happy', action: 'celebrate',
      reply: '哇！替你开心！这份好心情要好好收起来呀～' },
    { match: /难过|伤心|哭/, emotion: 'sad', action: 'nod',
      reply: '抱抱你……难过的时候不用硬撑，我一直都在。' },
    { match: /(害羞|脸红)/, emotion: 'shy', action: 'shake',
      reply: '诶……突然这样说，人家会不好意思的啦。' },
    { match: /(什么|怎么|为什么)/, emotion: 'surprised', action: 'shake',
      reply: '嗯？让我想想……这个问题有点难到我了，但我愿意陪你一起琢磨。' },
  ];
  const MOCK_FALLBACK = [
    { emotion: 'focused', action: 'nod', reply: '我在认真听呢，继续说呀～' },
    { emotion: 'smiling', action: 'nod', reply: '原来是这样呀，我记下了。' },
    { emotion: 'happy', action: 'wave', reply: '和你聊天的时间总是过得好快。' },
  ];

  // 对话触发代码检查的命中意图（迭代①冒烟证据：聊天页显示检查结果摘要）
  const CHECK_TRIGGER_RE = /检查.*(代码|code)|帮我检查/i;

  // 内置坏 snippet：硬编码密钥(error/security) + 宽松相等(warning/logic) + eval(warning/security)
  const BUILTIN_BAD_SNIPPET = [
    'function login(user, pass) {',
    '  var api_key = "sk-ABCD1234EFGH";',
    '  if (user == "admin") {',
    '    eval("do_" + action);',
    '  }',
    '}',
  ].join('\n');

  function mockSendMessage(text) {
    // 先推入用户消息
    const convo = loadMockConversation();
    convo.push({ role: 'user', content: text, ts: Date.now() });
    localStorage.setItem(LS_KEYS.conversation, JSON.stringify(convo));

    // thinking 事件（跨窗口广播：桌宠窗也感知）
    broadcast('qibai://llm-status', { state: 'thinking', message: '正在思考…' });

    return new Promise((resolve) => {
      setTimeout(() => {
        (async () => {
          let reply, emotion, action;
          // 迭代③·保险箱拦截（LLM 之前，绝不进 LLM；命中才走 secret 分支）
          const hit = mockVaultIntercept(text);
          if (hit) {
            const convoS = loadMockConversation();
            // 写入 conversation 的是脱敏文本（speak_text），明文 reply 仅当次 UI 展示
            convoS.push({ role: 'assistant', content: hit.speak_text, ts: Date.now() });
            localStorage.setItem(LS_KEYS.conversation, JSON.stringify(convoS));
            broadcast('qibai://llm-status', { state: 'done' });
            resolve(hit.result);
            return;
          }
          // 迭代④·open_social 技能拦截（只读安全；命中回复 skill:'open_social'，前端自动切社交 tab）
          // 与 qibai-core skills.rs 语义一致：参数 view=planet|city
          if (/星球|灯塔|社交|同城|逛逛/.test(text)) {
            const view = /同城|灯塔/.test(text) ? 'city' : 'planet';
            const msg = view === 'city'
              ? '带你打开「同城灯塔」：看看同一座城市里的灵魂光点。'
              : '带你打开「星球广场」：去星河里漫游，遇见其他匿名光点。';
            broadcast('qibai://llm-status', { state: 'done' });
            const convoS = loadMockConversation();
            convoS.push({ role: 'assistant', content: msg, ts: Date.now() });
            localStorage.setItem(LS_KEYS.conversation, JSON.stringify(convoS));
            resolve({ reply: msg, emotion: 'smiling', action: 'nod', skill: 'open_social', view });
            return;
          }
          if (CHECK_TRIGGER_RE.test(text)) {
            // 对话触发检查：用内置坏 snippet 跑 mock 检查器（纯内存，绝不读写用户文件）
            const report = await mockCheckCode({ snippet: BUILTIN_BAD_SNIPPET, language: 'javascript' });
            const e = report.issues.filter((i) => i.severity === 'error').length;
            const w = report.issues.filter((i) => i.severity === 'warning').length;
            reply = `检查完成：发现 ${report.issues.length} 个问题（error ${e} / warning ${w}）。` +
                    `已生成只读报告，详见控制台「代码检查」页的修复建议。`;
            emotion = 'worried';
            action = 'nod';
          } else {
            let hit = MOCK_REPLIES.find((r) => r.match.test(text)) ||
                      MOCK_FALLBACK[Math.floor(Math.random() * MOCK_FALLBACK.length)];
            reply = hit.reply;
            emotion = hit.emotion;
            action = hit.action;
            // v25 聊天功能完善：多轮上下文呼应——最近 6 条历史里出现过同类话题时加记忆回指
            const prevUsers = loadMockConversation().filter((m) => m.role === 'user').slice(-6);
            if (prevUsers.length > 1) {
              const same = prevUsers.slice(0, -1).find((m) => hit.match && hit.match.test(m.content));
              if (same) {
                reply = '（上次你也聊过这个～）' + reply;
              } else if (text.length <= 8) {
                // 短句回复：回指上一轮话题，让对话连续而非对暗号
                const lastTopic = prevUsers[prevUsers.length - 2].content;
                if (lastTopic) reply = '嗯，接着你刚才说的「' + lastTopic.slice(0, 12) + '」——' + reply;
              }
            }
          }

          const convo2 = loadMockConversation();
          convo2.push({ role: 'assistant', content: reply, ts: Date.now() });
          localStorage.setItem(LS_KEYS.conversation, JSON.stringify(convo2));

          broadcast('qibai://llm-status', { state: 'done' });
          resolve({ reply, emotion, action });
        })();
      }, 900);
    });
  }

  function loadMockConversation() {
    try {
      const raw = localStorage.getItem(LS_KEYS.conversation);
      if (raw) return JSON.parse(raw);
    } catch (e) { /* ignore */ }
    return [
      { role: 'assistant', content: '嗨～我是元气小小，你的桌面小伙伴。有什么想聊的随时叫我呀。', ts: Date.now() - 60000 },
    ];
  }

  /* ============================================================
   * P0/P1 多角色社交 mock（qibai.mock.personas / qibai.mock.social）
   * 形状严格按 docs/P0P1_SOCIAL_PLAN.md §三、§四冻结契约：
   *   get_personas / set_active_persona / rename_persona
   *   get_social_state / social_greet / social_pk / get_social_events
   * ========================================================== */

  const ALL_PERSONA_IDS = ['gudian', 'dushi', 'wenrou', 'huopo', 'reqing'];

  const PERSONA_META = {
    gudian:  { display_name: '古典佳人', nickname: '元气小小', traits: { tone: '温婉', catchphrase: '……', interaction: '轻声细语，耐心聆听' }, intimacy: 5 },
    dushi:   { display_name: '都市女性', nickname: '小杜', traits: { tone: '干练', catchphrase: '嗯？', interaction: '理性直率，条理分明' }, intimacy: 3 },
    wenrou:  { display_name: '温柔',     nickname: '柔柔', traits: { tone: '柔和', catchphrase: '慢慢来', interaction: '体贴周到，治愈陪伴' }, intimacy: 4 },
    huopo:   { display_name: '活泼',     nickname: '火火', traits: { tone: '元气', catchphrase: '耶！', interaction: '热情主动，带动气氛' }, intimacy: 2 },
    reqing:  { display_name: '热情时尚', nickname: '晴晴', traits: { tone: '开朗', catchphrase: '大家一起呀', interaction: '大方得体，社交达人' }, intimacy: 4 },
  };

  /* ============================================================
   * 迭代⑤ · 每角色专属音色（冻结默认表，与 qibai-core voice_profile.rs 逐字段一致）
   *   - mode=auto 时按性格取下表；custom 用用户在控制台卡片里存的值。
   *   - tone 用于音色标签「{tone} · {rate_desc} · {pitch_desc}」。
   * ============================================================ */
  const PERSONA_VOICE_TONES = {
    gudian: '温婉', dushi: '知性', wenrou: '恬静', huopo: '元气', reqing: '热情',
  };
  const DEFAULT_VOICE_PROFILES = {
    gudian:  { mode: 'auto', pitch: -2, rate: 1.0,  engine_voice_id: '', cloud_speaker: 'zh_female_wenrouxiaoya_moon_bigtts' },
    dushi:   { mode: 'auto', pitch: 0,  rate: 1.1,  engine_voice_id: '', cloud_speaker: 'zh_female_shuangkuaisisi_emo_v2_mars_bigtts' },
    wenrou:  { mode: 'auto', pitch: -1, rate: 0.8,  engine_voice_id: '', cloud_speaker: 'ICL_zh_female_ruanmengtuanzi_cs_tob' },
    huopo:   { mode: 'auto', pitch: 3,  rate: 1.2,  engine_voice_id: '', cloud_speaker: 'ICL_zh_female_wuxi_tob' },
    reqing:  { mode: 'auto', pitch: 2,  rate: 1.05, engine_voice_id: '', cloud_speaker: 'ICL_zh_female_reqingaina_cs_tob' },
  };

  function defaultVoiceProfileFor(id) {
    const d = DEFAULT_VOICE_PROFILES[id] || DEFAULT_VOICE_PROFILES.gudian;
    return {
      mode: d.mode,
      pitch: d.pitch,
      rate: d.rate,
      engine_voice_id: d.engine_voice_id,
      cloud_speaker: d.cloud_speaker,
    };
  }

  // 关系等级表（冻结 §3.3）
  const LEVELS = [
    { level: 0, name: '初识',           title: '互称全名',       min: 0,   max: 99 },
    { level: 1, name: '朋友',           title: '昵称',           min: 100, max: 299 },
    { level: 2, name: '闺蜜/搭档',      title: '专属昵称',       min: 300, max: 599 },
    { level: 3, name: '羁绊',           title: '挚友宣言',       min: 600, max: Number.MAX_SAFE_INTEGER },
  ];

  const GREET_DAILY_CAP = 5;
  const PK_DAILY_CAP = 3;

  function todayStr() {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  function levelOf(affinity) {
    for (let i = LEVELS.length - 1; i >= 0; i--) {
      if (affinity >= LEVELS[i].min) return LEVELS[i];
    }
    return LEVELS[0];
  }

  function pairKey(a, b) { return [a, b].sort().join('|'); }

  function loadMockPersonas() {
    try {
      const raw = localStorage.getItem(LS_KEYS.personas);
      if (raw) {
        const obj = JSON.parse(raw);
        if (obj && Array.isArray(obj.personas) && obj.personas.length === 5) {
          // 迭代⑤：旧存储缺 voice_profile 时按角色补默认表（幂等迁移）
          obj.personas.forEach((p) => {
            if (!p.voice_profile) p.voice_profile = defaultVoiceProfileFor(p.persona_id);
          });
          return obj;
        }
      }
    } catch (e) { /* ignore */ }
    // 首次初始化：5 个角色实体
    const obj = {
      version: 1,
      active_persona: 'gudian',
      personas: ALL_PERSONA_IDS.map((id) => ({
        persona_id: id,
        display_name: PERSONA_META[id].display_name,
        nickname: PERSONA_META[id].nickname,
        traits: PERSONA_META[id].traits,
        intimacy: PERSONA_META[id].intimacy,
        memory: [],
        sync: { server_id: null, dirty: false, version: 0 },
        voice_profile: defaultVoiceProfileFor(id), // 迭代⑤：每角色音色档案
      })),
    };
    localStorage.setItem(LS_KEYS.personas, JSON.stringify(obj));
    return obj;
  }

  function saveMockPersonas(obj) {
    localStorage.setItem(LS_KEYS.personas, JSON.stringify(obj));
  }

  // 演示关系：gudian×huopo=95（一次 PK 胜利 +10 即破 100 触发 level_up）
  function buildSeedRelations() {
    const preset = {
      'gudian|huopo': 95,
      'reqing|wenrou': 350,
      'dushi|wenrou': 150,
      'gudian|dushi': 30,
    };
    const list = [];
    for (let i = 0; i < ALL_PERSONA_IDS.length; i++) {
      for (let j = i + 1; j < ALL_PERSONA_IDS.length; j++) {
        const a = ALL_PERSONA_IDS[i];
        const b = ALL_PERSONA_IDS[j];
        const aff = preset[pairKey(a, b)] || 0;
        const lv = levelOf(aff);
        list.push({
          a, b,
          affinity: aff,
          level: lv.level,
          title: lv.name,
          pk_count: { date: todayStr(), count: 0 },
          greet_today: 0,
          greet_date: todayStr(),
          summon_count: 0,
          greet_count: 0,
          pk_total: 0,
          last_interaction_ts: 0,
        });
      }
    }
    return list;
  }

  function loadMockSocial() {
    try {
      const raw = localStorage.getItem(LS_KEYS.social);
      if (raw) {
        const obj = JSON.parse(raw);
        if (obj && Array.isArray(obj.relations) && Array.isArray(obj.events)) return obj;
      }
    } catch (e) { /* ignore */ }
    const now = Date.now();
    const obj = {
      version: 1,
      relations: buildSeedRelations(),
      events: [
        { ts: now - 3 * 3600e3, type: 'summon', a: 'gudian', b: 'huopo',
          detail: { text: '活泼被召唤上桌作客' } },
        { ts: now - 2 * 3600e3, type: 'greet', a: 'gudian', b: 'huopo',
          detail: { affinity_delta: 1, text: '两人互相打了招呼' } },
        { ts: now - 1 * 3600e3, type: 'pk', a: 'gudian', b: 'huopo',
          detail: { winner: 'gudian', affinity_delta: 10, text: '古典佳人猜拳小胜一局' } },
        { ts: now - 5 * 86400e3, type: 'dismiss', a: 'gudian', b: 'wenrou',
          detail: { text: '温柔和大家道别，离开桌子' } },
      ],
    };
    localStorage.setItem(LS_KEYS.social, JSON.stringify(obj));
    return obj;
  }

  function saveMockSocial(obj) {
    // 事件环形裁剪 200 条
    if (obj.events.length > 200) obj.events = obj.events.slice(obj.events.length - 200);
    localStorage.setItem(LS_KEYS.social, JSON.stringify(obj));
  }

  /* ============================================================
   * 双窗口扩展（WINDOW_ARCH.md §7）：
   * 1. mock 跨窗口事件总线——共享 localStorage + storage 事件，
   *    broadcast() 同时投递本页（mockEmit）与其它页（storage 写入）。
   * 2. desk 状态（qibai.mock.desk）——多窗口在场/锁定唯一事实源，
   *    与后端 qibai-core desk 状态形状一致 {present:[...], locked:{...}}。
   * ========================================================== */

  const mockWindowLabel = global.__qibaiWindowLabel || 'browser';
  const BUS_KEY = LS_KEYS.bus;
  let busSeq = 0;
  const seenBusTs = new Set(); // 去重：storage 事件在跨 frame 下有复制风险

  function broadcast(event, payload) {
    mockEmit(event, payload); // 本页监听器
    try {
      const msg = {
        event,
        payload,
        ts: Date.now() + '_' + (busSeq++),
        from: mockWindowLabel,
      };
      localStorage.setItem(BUS_KEY, JSON.stringify(msg));
    } catch (e) { /* localStorage 满/禁用时降级为仅本页 */ }
  }

  // 全局 storage 监听：其它页写入总线后分发到本页监听器
  if (typeof window !== 'undefined') {
    window.addEventListener('storage', (e) => {
      if (e.key !== BUS_KEY) return;
      let msg = null;
      try { msg = JSON.parse(e.newValue); } catch (err) { return; }
      if (!msg || !msg.event || seenBusTs.has(msg.ts)) return;
      seenBusTs.add(msg.ts);
      if (seenBusTs.size > 500) seenBusTs.clear();
      const set = mockListeners[msg.event];
      if (set) set.forEach((cb) => {
        try { cb({ payload: msg.payload }); } catch (err) { console.error(err); }
      });
    });

    // 轮询兜底：个别浏览器/iframe 场景 storage 事件不可靠时，
    // 每 400ms 扫描总线增量，保证跨窗口事件必达（幂等：ts 去重）。
    const lastKnown = {};
    setInterval(() => {
      try {
        const raw = localStorage.getItem(BUS_KEY);
        if (!raw) return;
        const msg = JSON.parse(raw);
        if (msg && msg.event && !seenBusTs.has(msg.ts) && lastKnown.ts !== msg.ts) {
          lastKnown.ts = msg.ts;
          seenBusTs.add(msg.ts);
          if (seenBusTs.size > 500) seenBusTs.clear();
          const set = mockListeners[msg.event];
          if (set) set.forEach((cb) => {
            try { cb({ payload: msg.payload }); } catch (err) { console.error(err); }
          });
        }
      } catch (err) { /* ignore */ }
    }, 400);
  }

  // ---- mock desk 状态 ----
  function activePersonaFromStore() {
    try {
      const p = JSON.parse(localStorage.getItem(LS_KEYS.personas) || 'null');
      if (p && p.active_persona) return p.active_persona;
    } catch (e) { /* ignore */ }
    return 'gudian';
  }

  function loadMockDesk() {
    try {
      const raw = localStorage.getItem(LS_KEYS.desk);
      if (raw) {
        const d = JSON.parse(raw);
        if (d && Array.isArray(d.present) && d.locked && typeof d.locked === 'object') return d;
      }
    } catch (e) { /* ignore */ }
    const d = { present: [activePersonaFromStore()], locked: {} };
    localStorage.setItem(LS_KEYS.desk, JSON.stringify(d));
    return d;
  }

  function saveMockDesk(d) {
    localStorage.setItem(LS_KEYS.desk, JSON.stringify(d));
  }

  function ensureActiveOnDesk() {
    const d = loadMockDesk();
    const active = activePersonaFromStore();
    if (d.present.indexOf(active) === -1) {
      d.present = [active].concat(d.present.filter((x) => x !== active));
      saveMockDesk(d);
    }
    return d;
  }

  function mockGetDeskState() {
    const d = ensureActiveOnDesk();
    return Promise.resolve({ present: d.present.slice(), locked: Object.assign({}, d.locked) });
  }

  function mockPetSummon(args) {
    const d = ensureActiveOnDesk();
    const id = args.persona_id;
    if (d.present.indexOf(id) === -1) d.present.push(id);
    saveMockDesk(d);
    broadcast('qibai://desk-changed', { present: d.present.slice(), locked: Object.assign({}, d.locked) });
    return Promise.resolve('ok');
  }

  function mockPetDismiss(args) {
    const d = ensureActiveOnDesk();
    const id = args.persona_id;
    if (id === d.present[0] && activePersonaFromStore() === id) {
      return Promise.reject(new Error('[mock] 主角色不能被送走，请先切换主角色'));
    }
    d.present = d.present.filter((x) => x !== id);
    delete d.locked[id];
    saveMockDesk(d);
    broadcast('qibai://desk-changed', { present: d.present.slice(), locked: Object.assign({}, d.locked) });
    return Promise.resolve('ok');
  }

  function mockPetSetLocked(args) {
    const d = ensureActiveOnDesk();
    if (args.locked) d.locked[args.persona_id] = true;
    else delete d.locked[args.persona_id];
    saveMockDesk(d);
    broadcast('qibai://desk-changed', { present: d.present.slice(), locked: Object.assign({}, d.locked) });
    return Promise.resolve('ok');
  }

  function mockConsoleOpen() {
    broadcast('qibai://console-open', {});
    return Promise.resolve('ok');
  }

  function findRelation(social, a, b) {
    const [x, y] = [a, b].sort();
    return social.relations.find((r) => r.a === x && r.b === y) || null;
  }

  function pushEvent(social, ev) {
    social.events.push(Object.assign({ ts: Date.now() }, ev));
  }

  // 寒暄台词模板（按关系等级分档；无 LLM key 时自动降级模板）
  function greetLinesFor(rel, from, to) {
    const L = rel.level;
    const N = (id) => (PERSONA_META[id] || { display_name: id }).display_name;
    const bank = [
      [ // level 0 初识
        { speaker: from, text: `${N(to)}，你也在这里呀！我们第一次正式打招呼呢。` },
        { speaker: to, text: `是呀你好呀，${N(from)}。以后常来常往哦。` },
      ],
      [ // level 1 朋友
        { speaker: from, text: '嘿！最近忙什么呢，我可想你啦。' },
        { speaker: to, text: '没忙什么呀，就是想着什么时候和你聊聊天～' },
      ],
      [ // level 2 闺蜜/搭档
        { speaker: from, text: '亲爱的！今天也要贴贴贴贴贴～' },
        { speaker: to, text: '哈哈好呀好呀，和你待在一起最舒服啦。' },
      ],
      [ // level 3 羁绊
        { speaker: from, text: '无论什么时候，我都会站在你这边的。' },
        { speaker: to, text: '我们是永远的好搭档！这份交情谁也拆不散。' },
      ],
    ];
    return bank[Math.min(L, bank.length - 1)].slice();
  }

  const PK_WIN_LINES = {
    gudian: '哎呀承让了，赢了一点点而已～',
    dushi:  '预判到位，这局是我赢了。',
    wenrou: '嘻嘻，运气不错，侥幸赢啦～',
    huopo:  '耶！我赢啦！再来再来！',
    reqing: '哈哈，看来今天的幸运女神站在我这边哦。',
  };
  const PK_LOSE_LINES = {
    gudian: '没关系，下次我一定会赢回来的～',
    dushi:  '这局算你厉害，再来一局定胜负。',
    wenrou: '哎呀差一点点，不过陪你玩我很开心呀。',
    huopo:  '唔——下次我绝对不会输的！',
    reqing: '可以呀你，实力不容小觑哦。',
  };
  const LEVEL_UP_EGG = {
    1: '欸？我们好像越来越熟了！以后就是朋友啦～',
    2: '哇，关系升级！我们已经是闺蜜/搭档级别了！',
    3: '羁绊达成！你是我最重要的小伙伴之一！',
  };

  function mockGetPersonas() {
    const obj = loadMockPersonas();
    return Promise.resolve({
      active_persona: obj.active_persona,
      personas: obj.personas.map((p) => ({
        persona_id: p.persona_id,
        display_name: p.display_name,
        nickname: p.nickname,
        traits: p.traits,
        intimacy: p.intimacy,
        memory: (p.memory || []).slice(-10),
        sync: p.sync,
        voice_profile: p.voice_profile || defaultVoiceProfileFor(p.persona_id), // 迭代⑤
      })),
    });
  }

  // 迭代⑤：写入某角色 voice_profile（控制台卡片「保存」按钮 → invoke set_persona_voice）。
  function mockSetPersonaVoice(args) {
    args = args || {};
    const obj = loadMockPersonas();
    const p = obj.personas.find((x) => x.persona_id === args.persona_id);
    if (!p) return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    const vp = args.voice_profile || {};
    p.voice_profile = {
      mode: vp.mode === 'custom' ? 'custom' : 'auto',
      pitch: Math.max(-10, Math.min(10, Number(vp.pitch) || 0)),
      rate: Math.max(0.5, Math.min(2.0, Number(vp.rate) || 1)),
      engine_voice_id: String(vp.engine_voice_id || ''),
      cloud_speaker: String(vp.cloud_speaker || ''),
    };
    saveMockPersonas(obj);
    return Promise.resolve('ok');
  }

  function mockSetActivePersona(args) {
    const obj = loadMockPersonas();
    if (!obj.personas.find((p) => p.persona_id === args.persona_id)) {
      return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    }
    const old = obj.active_persona;
    obj.active_persona = args.persona_id;
    saveMockPersonas(obj);
    // 同步 desk：present = [new] ∪ (present \ {old})（契约 §4.2）
    const d = ensureActiveOnDesk();
    d.present = [args.persona_id].concat(d.present.filter((x) => x !== args.persona_id && x !== old));
    saveMockDesk(d);
    broadcast('qibai://desk-changed', { present: d.present.slice(), locked: Object.assign({}, d.locked) });
    return Promise.resolve('ok');
  }

  function mockRenamePersona(args) {
    const obj = loadMockPersonas();
    const p = obj.personas.find((x) => x.persona_id === args.persona_id);
    if (!p) return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    p.nickname = String(args.nickname || '').slice(0, 12) || p.nickname;
    saveMockPersonas(obj);
    return Promise.resolve('ok');
  }

  function mockGetSocialState() {
    const social = loadMockSocial();
    return Promise.resolve({
      relations: social.relations.map((r) => Object.assign({}, r, {
        pk_left: r.pk_count.date === todayStr()
          ? Math.max(0, PK_DAILY_CAP - r.pk_count.count)
          : PK_DAILY_CAP,
      })),
      levels: LEVELS.map((l) => ({ level: l.level, name: l.name, title: l.title, min: l.min, max: l.max })),
    });
  }

  function mockSocialGreet(args) {
    const social = loadMockSocial();
    const from = args.from;
    const to = args.to;
    if (!PERSONA_META[from] || !PERSONA_META[to]) {
      return Promise.reject(new Error('[mock] unknown persona in greet: ' + from + ' / ' + to));
    }
    const rel = findRelation(social, from, to);
    const today = todayStr();
    if (rel.greet_date !== today) { rel.greet_date = today; rel.greet_today = 0; }

    let lines;
    let delta = 0;
    if (rel.greet_today >= GREET_DAILY_CAP) {
      lines = [{ speaker: from, text: '今天已经聊过好多啦，聊太多会腻的，改天再玩～' }];
    } else {
      rel.greet_today += 1;
      rel.greet_count += 1;
      rel.affinity += 1;
      delta = 1;
      rel.last_interaction_ts = Date.now();
      lines = greetLinesFor(rel, from, to);
    }

    // 升级检测
    const oldLevel = rel.level;
    const lv = levelOf(rel.affinity);
    rel.level = lv.level;
    rel.title = lv.name;
    let event = null;
    if (lv.level > oldLevel) {
      event = {
        type: 'level_up', a: rel.a, b: rel.b,
        detail: { new_level: lv.level, text: `${rel.a} 与 ${rel.b} 关系升到「${lv.name}」` },
      };
      pushEvent(social, event);
      lines.push({ speaker: rel.a, text: LEVEL_UP_EGG[lv.level] || '我们的关系更进一步啦！' });
    }
    if (delta > 0) {
      pushEvent(social, {
        type: 'greet', a: rel.a, b: rel.b,
        detail: { affinity_delta: delta, text: `${PERSONA_META[from].display_name} 与 ${PERSONA_META[to].display_name} 寒暄了一番` },
      });
    }
    saveMockSocial(social);
    // 跨窗口广播（契约 §2.4/§5：后端对 lines 逐条广播 qibai://pet-say，mock 保持一致）
    (lines || []).forEach((l) => {
      broadcast('qibai://pet-say', { persona_id: l.speaker, text: l.text, ms: 2200 });
    });
    return Promise.resolve({ lines, affinity_delta: delta, event });
  }

  const WIN_MAP = { rock: 'scissors', scissors: 'paper', paper: 'rock' };

  function mockSocialPk(args) {
    const social = loadMockSocial();
    const a = args.a;
    const b = args.b;
    const ma = args.move_a;
    const mb = args.move_b;
    if (!PERSONA_META[a] || !PERSONA_META[b]) {
      return Promise.reject(new Error('[mock] unknown persona in pk: ' + a + ' / ' + b));
    }
    if (!WIN_MAP[ma] || !WIN_MAP[mb]) {
      return Promise.reject(new Error('[mock] bad move: ' + ma + ' / ' + mb));
    }
    const rel = findRelation(social, a, b);
    const today = todayStr();
    if (rel.pk_count.date !== today) rel.pk_count = { date: today, count: 0 };

    // 防刷：每日每对 3 次
    if (rel.pk_count.count >= PK_DAILY_CAP) {
      broadcast('qibai://pk-round', {
        a: rel.a, b: rel.b,
        move_a: args.move_a, move_b: args.move_b,
        result: 'blocked', winner: null,
        lines: [{ speaker: a, text: '今天的猜拳次数已经用完啦，休息一下，明天再来玩吧～' }],
      });
      return Promise.resolve({
        blocked: true,
        winner: null, loser: null, result: 'blocked',
        affinity_delta: {},
        lines: [{ speaker: a, text: '今天的猜拳次数已经用完啦，休息一下，明天再来玩吧～' }],
        remaining_today: 0,
      });
    }

    rel.pk_count.count += 1;
    rel.pk_total += 1;
    rel.last_interaction_ts = Date.now();

    let result, winner, loser;
    if (ma === mb) { result = 'draw'; winner = null; loser = null; }
    else if (WIN_MAP[ma] === mb) { result = 'win'; winner = a; loser = b; }
    else { result = 'lose'; winner = b; loser = a; }

    const delta = { };
    let lines = [];
    if (result === 'draw') {
      delta[a] = 4; delta[b] = 4;
      rel.affinity += 4;
      lines = [
        { speaker: a, text: '平局！这就是默契吧～' },
        { speaker: b, text: '平手也算心有灵犀，再来一局？' },
      ];
    } else {
      delta[winner] = 10;
      delta[loser] = 2;
      rel.affinity += 12;
      lines = [
        { speaker: winner, text: PK_WIN_LINES[winner] || '我赢啦！' },
        { speaker: loser, text: PK_LOSE_LINES[loser] || '哎呀，下次一定赢回来！' },
      ];
    }

    // 升级检测（胜利方带动两人共同好感度跨档时解锁彩蛋）
    const oldLevel = rel.level;
    const lv = levelOf(rel.affinity);
    rel.level = lv.level;
    rel.title = lv.name;
    let event = null;
    if (lv.level > oldLevel) {
      event = {
        type: 'level_up', a: rel.a, b: rel.b,
        detail: { new_level: lv.level, text: `${rel.a} 与 ${rel.b} 关系升到「${lv.name}」` },
      };
      pushEvent(social, event);
      lines.push({ speaker: rel.a, text: LEVEL_UP_EGG[lv.level] || '我们的关系更进一步啦！' });
    }

    pushEvent(social, {
      type: 'pk', a: rel.a, b: rel.b,
      detail: { winner: winner || 'draw', affinity_delta: delta, text: result === 'draw' ? '两人猜拳平局' : `${PERSONA_META[winner].display_name} 赢得猜拳` },
    });
    saveMockSocial(social);

    // 跨窗口广播（契约 §2.4/§5：pk-round + pet-say，mock 与后端行为一致）
    broadcast('qibai://pk-round', {
      a: rel.a, b: rel.b,
      move_a: args.move_a, move_b: args.move_b,
      result: result,
      winner: winner,
      lines: lines || [],
    });
    (lines || []).forEach((l) => {
      broadcast('qibai://pet-say', { persona_id: l.speaker, text: l.text, ms: 2200 });
    });

    return Promise.resolve({
      blocked: false,
      winner, loser, result,
      affinity_delta: delta,
      lines,
      remaining_today: PK_DAILY_CAP - rel.pk_count.count,
    });
  }

  function mockGetSocialEvents(args) {
    const social = loadMockSocial();
    const limit = Math.max(1, Math.min(args.limit || 50, 200));
    const events = social.events.slice().sort((x, y) => y.ts - x.ts).slice(0, limit);
    return Promise.resolve({ events });
  }

  /* ============================================================
   * 亲密度 L1-L10 成长系统 mock（INTIMACY_SYSTEM.md §6/§7/§8.3）
   * 存储 qibai.mock.intimacy，形状与 intimacy.json 冻结契约一致：
   *   get_intimacy_levels / get_intimacy_state / intimacy_event
   *   set_anniversary / clear_anniversary / check_gate
   * 事件：intimacy_event 成功后 broadcast qibai://intimacy-updated；
   * 升级写 social 事件流（type=intimacy_level_up，a=b=persona_id）。
   * ========================================================== */

  // 等级表（冻结 §2：阈值起点）
  const INTIMACY_LEVELS = [
    { level: 1, name: '初识', min: 0,    desc: '初见倾心，从一句你好开始。' },
    { level: 2, name: '熟悉', min: 120,  desc: '开始习惯彼此的存在。' },
    { level: 3, name: '习惯', min: 280,  desc: '你的作息与喜好，她悄悄记下。' },
    { level: 4, name: '贴心', min: 480,  desc: '她开始帮你打理生活小事。' },
    { level: 5, name: '默契', min: 720,  desc: '一个眼神，她就懂你的心情。' },
    { level: 6, name: '亲近', min: 1000, desc: '想为你准备更多惊喜。' },
    { level: 7, name: '羁绊', min: 1330, desc: '值得纪念的日子，她不会忘。' },
    { level: 8, name: '依赖', min: 1720, desc: '她总想多陪你一会儿。' },
    { level: 9, name: '灵魂', min: 2180, desc: '你们之间有了只有彼此懂的默契。' },
    { level: 10, name: '缔结', min: 2720, desc: '独一无二的约定，只属于你们。' },
  ];

  // 门控全集（冻结 §4：gate -> required_level / label / kind）
  const INTIMACY_GATES = [
    { gate: 'basic_chat', required_level: 1, label: '基础对话', kind: 'active' },
    { gate: 'idle_actions', required_level: 1, label: '待机动作', kind: 'active' },
    { gate: 'sit_reminder', required_level: 1, label: '久坐提醒', kind: 'active' },
    { gate: 'custom_nickname', required_level: 2, label: '自定义称呼', kind: 'active' },
    { gate: 'daily_greeting', required_level: 2, label: '每日问候', kind: 'active' },
    { gate: 'water_reminder', required_level: 2, label: '喝水提醒', kind: 'active' },
    { gate: 'preference_memory', required_level: 3, label: '记住偏好（作息/常用软件）', kind: 'active' },
    { gate: 'boot_greeting', required_level: 3, label: '开机问候', kind: 'active' },
    { gate: 'assistant_skills', required_level: 4, label: '助理技能（打开应用/文件夹、设提醒）', kind: 'active' },
    { gate: 'vault', required_level: 4, label: '本地密码保险箱', kind: 'active' },
    { gate: 'mood_empathy', required_level: 5, label: '心情识别增强', kind: 'active' },
    { gate: 'screen_guard', required_level: 5, label: '全屏守护', kind: 'placeholder' },
    { gate: 'performance', required_level: 6, label: '表演系统（节日彩蛋接口）', kind: 'active' },
    { gate: 'dressup', required_level: 6, label: '形象换装', kind: 'placeholder' },
    { gate: 'storyline_1', required_level: 7, label: '专属剧情线一（对话彩蛋模板）', kind: 'active' },
    { gate: 'anniversary', required_level: 7, label: '纪念日自动庆祝（纪念日设置）', kind: 'active' },
    { gate: 'sleep_guard', required_level: 8, label: '息屏守护', kind: 'placeholder' },
    { gate: 'voice_mode', required_level: 8, label: '语音陪伴模式', kind: 'placeholder' },
    { gate: 'storyline_2', required_level: 9, label: '专属剧情线二（双人彩蛋）', kind: 'active' },
    { gate: 'ceremony', required_level: 10, label: '缔结仪式结局演出（文字+CSS 动效）', kind: 'active' },
    { gate: 'batch_skills', required_level: 10, label: '高级助理批量技能', kind: 'placeholder' },
  ];

  // 成就（冻结 §5：判定时机=每次 intimacy_event 后重算）
  const INTIMACY_ACHIEVEMENTS = [
    { id: 'first_greeting', name: '初次问候',   test: (e, lv) => e.cumulative.greetings >= 1 },
    { id: 'chat_100', name: '聊得来',           test: (e, lv) => e.cumulative.chats >= 100 },
    { id: 'click_50', name: '指尖温情',        test: (e, lv) => e.cumulative.clicks >= 50 },
    { id: 'pk_win_5', name: '猜拳小能手',       test: (e, lv) => e.cumulative.pk_wins >= 5 },
    { id: 'companion_7', name: '常伴七日',       test: (e, lv) => e.cumulative.companion_days >= 7 },
    { id: 'level_3', name: '心有默契',           test: (e, lv) => lv >= 3 },
    { id: 'level_6', name: '亲密无间',           test: (e, lv) => lv >= 6 },
    { id: 'level_9', name: '灵魂共鸣',           test: (e, lv) => lv >= 9 },
    { id: 'level_10', name: '缔结之约',          test: (e, lv) => lv >= 10 },
  ];

  // 每日任务（冻结 §3.4：每任务每日仅首次 +5）
  const INTIMACY_TASKS = [
    { id: 'greeting', name: '和她说早安' },
    { id: 'click', name: '完成一次互动' },
    { id: 'pk', name: '赢一场猜拳' },
  ];

  // 数值常量（冻结 §3）
  const INTIMACY_DAILY_CAP = 60;       // 每角色每日总积分上限
  const CLICK_DAILY_CAP = 10;          // 每角色每日 click 次数上限
  const CHAT_POINTS = 2;
  const CLICK_POINTS = 1;
  const TASK_POINTS = 5;
  const GIFT_POINTS = 10;
  // 陪伴递减阈值（冻结 §3.2：累计分钟跨过即 +1）
  const COMPANION_THRESHOLDS = [10, 20, 30, 50, 70, 90, 120, 150, 180];
  const COMPANION_POINTS_DAILY_CAP = 10;

  function makeIntimacyEntry(personaId) {
    return {
      persona_id: personaId,
      points: 0,
      level: 1,
      daily: {
        date: todayStr(), gained: 0, chats: 0, clicks: 0,
        companion_minutes: 0, companion_points: 0,
        tasks: { greeting: false, click: false, pk: false },
      },
      cumulative: {
        chats: 0, clicks: 0, pk_wins: 0, companion_days: 0,
        greetings: 0, gifts: 0, last_gift_date: null,
      },
      achievements: [],
      anniversary: null,
      sync: { server_id: null, dirty: false, version: 0 },
    };
  }

  // 缺字段旧文件按 serde default 补齐，不报错不丢数据（§7.4）
  function normalizeIntimacyEntry(e) {
    if (!e || typeof e !== 'object') return e;
    e.points = typeof e.points === 'number' ? e.points : 0;
    const d = makeIntimacyEntry(e.persona_id);
    e.daily = Object.assign(d.daily, e.daily || {});
    e.daily.tasks = Object.assign({ greeting: false, click: false, pk: false }, (e.daily && e.daily.tasks) || {});
    e.cumulative = Object.assign(d.cumulative, e.cumulative || {});
    e.achievements = Array.isArray(e.achievements) ? e.achievements : [];
    e.anniversary = e.anniversary || null;
    e.sync = Object.assign({ server_id: null, dirty: false, version: 0 }, e.sync || {});
    return e;
  }

  function loadMockIntimacy() {
    let obj = null;
    try {
      const raw = localStorage.getItem(LS_KEYS.intimacy);
      if (raw) obj = JSON.parse(raw);
    } catch (e) { obj = null; }
    if (!obj || typeof obj !== 'object' || typeof obj.entries !== 'object') {
      // §7.1：文件不存在 → 首次初始化 5 角色实体，migrated=false
      obj = { version: 1, migrated: false, entries: {} };
      ALL_PERSONA_IDS.forEach((id) => { obj.entries[id] = makeIntimacyEntry(id); });
      localStorage.setItem(LS_KEYS.intimacy, JSON.stringify(obj));
    }
    // §7.2：migrated=false → 从 personas.json 的 intimacy 播种（幂等）
    if (!obj.migrated) {
      const personas = loadMockPersonas();
      ALL_PERSONA_IDS.forEach((id) => {
        if (!obj.entries[id]) obj.entries[id] = makeIntimacyEntry(id);
        const p = (personas.personas || []).find((x) => x.persona_id === id);
        const lv = Math.max(1, Math.min(10, parseInt(p && p.intimacy, 10) || 1));
        obj.entries[id].points = INTIMACY_LEVELS[lv - 1].min;
      });
      obj.migrated = true;
      localStorage.setItem(LS_KEYS.intimacy, JSON.stringify(obj));
    }
    ALL_PERSONA_IDS.forEach((id) => {
      if (!obj.entries[id]) obj.entries[id] = makeIntimacyEntry(id);
      normalizeIntimacyEntry(obj.entries[id]);
    });
    return obj;
  }

  function saveMockIntimacy(obj) {
    localStorage.setItem(LS_KEYS.intimacy, JSON.stringify(obj));
  }

  // 跨天重置（§3.3/§9.12：daily.date 过期则全部归零）
  function touchIntimacyDaily(entry) {
    const today = todayStr();
    if (entry.daily.date !== today) {
      entry.daily = {
        date: today, gained: 0, chats: 0, clicks: 0,
        companion_minutes: 0, companion_points: 0,
        tasks: { greeting: false, click: false, pk: false },
      };
    }
  }

  function intimacyLevelOf(points) {
    let lv = 1;
    for (let i = 0; i < INTIMACY_LEVELS.length; i++) {
      if (points >= INTIMACY_LEVELS[i].min) lv = INTIMACY_LEVELS[i].level;
    }
    return lv;
  }

  // 本次升级新跨过的门控（含 placeholder）
  function newUnlockedGates(oldLevel, newLevel) {
    return INTIMACY_GATES.filter((g) => oldLevel < g.required_level && newLevel >= g.required_level);
  }

  function buildIntimacyState(entry) {
    const lv = intimacyLevelOf(entry.points);
    const def = INTIMACY_LEVELS[lv - 1];
    const next = INTIMACY_LEVELS[lv] || null;
    const min = def.min;
    const max = next ? next.min : null;
    const ratio = max === null ? 1 : Math.max(0, Math.min(1, (entry.points - min) / (max - min)));
    return {
      persona_id: entry.persona_id,
      points: entry.points,
      level: lv,
      level_name: def.name,
      progress: { min, max, ratio },
      unlocked: INTIMACY_GATES.filter((g) => lv >= g.required_level).map((g) => g.gate),
      locked: INTIMACY_GATES.filter((g) => lv < g.required_level)
        .map((g) => ({ gate: g.gate, label: g.label, required_level: g.required_level, kind: g.kind })),
      daily: { date: entry.daily.date, gained: entry.daily.gained, cap: INTIMACY_DAILY_CAP },
      cumulative: {
        chats: entry.cumulative.chats, clicks: entry.cumulative.clicks,
        pk_wins: entry.cumulative.pk_wins, companion_days: entry.cumulative.companion_days,
        greetings: entry.cumulative.greetings, gifts: entry.cumulative.gifts,
      },
      tasks: INTIMACY_TASKS.map((t) => ({ id: t.id, name: t.name, done: !!entry.daily.tasks[t.id] })),
      achievements: entry.achievements.map((a) => Object.assign({}, a)),
      anniversary: entry.anniversary ? { month: entry.anniversary.month, day: entry.anniversary.day } : null,
      sync: Object.assign({}, entry.sync),
    };
  }

  function mockGetIntimacyLevels() {
    return Promise.resolve({
      levels: INTIMACY_LEVELS.map((l, i) => ({
        level: l.level, name: l.name, desc: l.desc,
        min: l.min, max: (INTIMACY_LEVELS[i + 1] || {}).min || null,
      })),
    });
  }

  function mockGetIntimacyState(args) {
    const obj = loadMockIntimacy();
    const entry = obj.entries[args.persona_id];
    if (!entry) return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    touchIntimacyDaily(entry);
    saveMockIntimacy(obj);
    return Promise.resolve(buildIntimacyState(entry));
  }

  function doIntimacyEvent(personaId, kind, detail) {
    detail = detail || {};
    const obj = loadMockIntimacy();
    const entry = obj.entries[personaId];
    if (!entry) return Promise.reject(new Error('[mock] unknown persona_id: ' + personaId));
    touchIntimacyDaily(entry);

    const oldLevel = intimacyLevelOf(entry.points);
    const tasksUpdated = [];
    let rawDelta = 0;

    if (kind === 'chat') {
      rawDelta = CHAT_POINTS;
      entry.daily.chats += 1;
      entry.cumulative.chats += 1;
    } else if (kind === 'click') {
      if (entry.daily.clicks < CLICK_DAILY_CAP) {
        rawDelta = CLICK_POINTS;
        entry.daily.clicks += 1;
        entry.cumulative.clicks += 1;
      }
      // §3.4：当日首次 click 事件自动完成「互动」任务
      if (!entry.daily.tasks.click) {
        entry.daily.tasks.click = true;
        rawDelta += TASK_POINTS;
        tasksUpdated.push({ id: 'click', done: true });
      }
    } else if (kind === 'task') {
      const taskId = detail.task_id;
      const known = INTIMACY_TASKS.find((t) => t.id === taskId);
      if (!known) return Promise.reject(new Error('[mock] unknown task_id: ' + taskId));
      if (!entry.daily.tasks[taskId]) {
        entry.daily.tasks[taskId] = true;
        rawDelta += TASK_POINTS;
        if (taskId === 'greeting') entry.cumulative.greetings += 1;
        tasksUpdated.push({ id: taskId, done: true });
      }
    } else if (kind === 'companion') {
      const minutes = Math.max(0, parseInt(detail.minutes, 10) || 10);
      const prev = entry.daily.companion_minutes;
      const nextM = prev + minutes;
      let crossed = 0;
      COMPANION_THRESHOLDS.forEach((t) => { if (prev < t && nextM >= t) crossed += 1; });
      entry.daily.companion_minutes = nextM;
      if (crossed > 0) {
        const room = COMPANION_POINTS_DAILY_CAP - entry.daily.companion_points;
        const pts = Math.max(0, Math.min(crossed, room));
        const wasFirstPointOfDay = entry.daily.companion_points === 0;
        entry.daily.companion_points += pts;
        rawDelta = pts;
        if (pts > 0 && wasFirstPointOfDay) entry.cumulative.companion_days += 1;
      }
    } else if (kind === 'gift') {
      const today = todayStr();
      if (entry.cumulative.last_gift_date !== today) {
        entry.cumulative.last_gift_date = today;
        entry.cumulative.gifts += 1;
        rawDelta = GIFT_POINTS;
      }
    } else if (kind === 'pk_win') {
      entry.cumulative.pk_wins += 1;
      // §3.4：当日首次猜拳获胜自动完成「pk」任务
      if (!entry.daily.tasks.pk) {
        entry.daily.tasks.pk = true;
        rawDelta += TASK_POINTS;
        tasksUpdated.push({ id: 'pk', done: true });
      }
    } else {
      return Promise.reject(new Error('[mock] unknown intimacy kind: ' + kind));
    }

    // §3.3：每日总上限——delta = min(raw, 60 - gained)；raw 被截即 capped=true
    const room = INTIMACY_DAILY_CAP - entry.daily.gained;
    const capped = rawDelta > room;
    const delta = Math.max(0, Math.min(rawDelta, room));
    entry.daily.gained += delta;
    entry.points += delta;

    const newLevel = intimacyLevelOf(entry.points);
    entry.level = newLevel;
    const leveledUp = newLevel > oldLevel;
    const unlocked = leveledUp ? newUnlockedGates(oldLevel, newLevel) : [];

    // §5：每次事件后统一重算成就
    const granted = [];
    INTIMACY_ACHIEVEMENTS.forEach((a) => {
      if (entry.achievements.find((x) => x.id === a.id)) return;
      if (a.test(entry, newLevel)) {
        const rec = { id: a.id, name: a.name, ts: Date.now() };
        entry.achievements.push(rec);
        granted.push(rec);
      }
    });

    // §6.1：升级写 social 事件流（kind=intimacy_level_up）
    if (leveledUp) {
      const social = loadMockSocial();
      pushEvent(social, {
        type: 'intimacy_level_up', a: personaId, b: personaId,
        detail: {
          new_level: newLevel,
          level_name: INTIMACY_LEVELS[newLevel - 1].name,
          unlocked: unlocked.map((g) => g.gate),
        },
      });
      saveMockSocial(social);
    }

    saveMockIntimacy(obj);

    // §6.2：每次成功事件后广播
    broadcast('qibai://intimacy-updated', {
      persona_id: personaId, points: entry.points, level: newLevel,
      level_name: INTIMACY_LEVELS[newLevel - 1].name,
      leveled_up: leveledUp, unlocked, achievements_granted: granted,
    });

    return Promise.resolve({
      delta, points: entry.points, level: newLevel, leveled_up: leveledUp,
      unlocked, achievements_granted: granted, tasks_updated: tasksUpdated, capped,
    });
  }

  function mockSetAnniversary(args) {
    const obj = loadMockIntimacy();
    const entry = obj.entries[args.persona_id];
    if (!entry) return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    const month = parseInt(args.month, 10);
    const day = parseInt(args.day, 10);
    if (!(month >= 1 && month <= 12 && day >= 1 && day <= 31)) {
      return Promise.reject(new Error('[mock] invalid anniversary date: ' + month + '-' + day));
    }
    entry.anniversary = { month, day };
    saveMockIntimacy(obj);
    return Promise.resolve('ok');
  }

  function mockClearAnniversary(args) {
    const obj = loadMockIntimacy();
    const entry = obj.entries[args.persona_id];
    if (!entry) return Promise.reject(new Error('[mock] unknown persona_id: ' + args.persona_id));
    entry.anniversary = null;
    saveMockIntimacy(obj);
    return Promise.resolve('ok');
  }

  function mockCheckGate(args) {
    const obj = loadMockIntimacy();
    const entry = obj.entries[args.persona_id];
    // §6.1.6：未知 persona 宽容返回 current_level=1，不报错
    let level = 1;
    if (entry) {
      touchIntimacyDaily(entry);
      level = intimacyLevelOf(entry.points);
    }
    const gate = INTIMACY_GATES.find((g) => g.gate === args.gate);
    const required = gate ? gate.required_level : 99;
    return Promise.resolve({ open: level >= required, required_level: required, current_level: level });
  }

  /* ============================================================
   * 迭代① · 全屏守护 mock（guard_trigger / guard_skip / guard_system_action）
   * 冻结契约（docs/GUARD.md / qibai-core guard.rs）：
   *   guard_trigger  → GuardStart{mode,hours,message,end_action,countdown_secs,
   *                                persona_id,screenshot,perform_fullscreen} | 结构化错误
   *   guard_skip     → "ok"；事件 guard-end {skipped, end_action}
   *   guard_system_action → {simulated, detail}；事件 guard-action {action,simulated,detail}
   * ========================================================== */

  const GUARD_COOLDOWN_MS = 1800 * 1000; // 30 分钟冷却

  function loadGuardState() {
    try {
      const o = JSON.parse(localStorage.getItem(LS_KEYS.guard) || '{}');
      if (o && typeof o === 'object') return o;
    } catch (e) { /* ignore */ }
    return {};
  }

  function mockGuardTrigger() {
    const s = loadMockSettings();
    const g = s.guard || {};
    if (!g.enabled) {
      return Promise.reject(Object.assign(
        new Error('全屏守护未启用：请到控制台「设置-全屏守护」打开开关'),
        { code: 'guard_disabled' }));
    }
    const st = loadGuardState();
    const now = Date.now();
    if (st.last_end && (now - st.last_end) < GUARD_COOLDOWN_MS) {
      const remainMin = Math.ceil((GUARD_COOLDOWN_MS - (now - st.last_end)) / 60000);
      return Promise.reject(Object.assign(
        new Error(`守护冷却中：距上次结束还剩约 ${remainMin} 分钟`),
        { code: 'guard_cooldown' }));
    }
    const sitMin = Math.max(5, parseInt(g.sit_min, 10) || 120);
    const hours = Math.max(1, Math.round(sitMin / 60));
    const payload = {
      mode: g.mode || 'strong',
      hours: hours,
      message: `你连续工作 ${hours} 小时啦，休息五分钟吧`,
      end_action: g.end_action || 'countdown',
      countdown_secs: Math.max(60, Math.min(300, parseInt(g.countdown_secs, 10) || 120)),
      persona_id: activePersonaFromStore(),
      screenshot: null, // mock 无真实截图，前端走 CSS 模拟桌面
      perform_fullscreen: true,
    };
    broadcast('qibai://guard-start', payload);
    return Promise.resolve(payload);
  }

  function mockGuardSkip(args) {
    const s = loadMockSettings();
    const endAction = (s.guard && s.guard.end_action) || 'countdown';
    broadcast('qibai://guard-end', {
      skipped: !!(args && args.reason === 'user'),
      end_action: endAction,
    });
    // 记录冷却起点
    localStorage.setItem(LS_KEYS.guard, JSON.stringify({ last_end: Date.now() }));
    return Promise.resolve('ok');
  }

  function mockGuardSystemAction(args) {
    const action = (args && args.action) === 'lock' ? 'lock' : 'sleep';
    const detail = `浏览器演示模式：已模拟${action === 'sleep' ? '息屏' : '锁屏'}动作`;
    broadcast('qibai://guard-action', { action, simulated: true, detail });
    return Promise.resolve({ simulated: true, detail });
  }

  /* ============================================================
   * 迭代② · 语音 mock（tts_speak / tts_engine_status / stt_status）
   * 冻结形状见 docs/_VOICE_ITER2_SPEC.md §3/§4.4：
   *   - 引擎解析规则逐分支对齐 core resolve_engine（local 用浏览器
   *     speechSynthesis 真发声；cloud 仅模拟，不调任何外部服务）
   *   - stt_status: {enabled, provider, key_ready, device_available,
   *                  available, reason}；沙箱无麦克风 → device_available=false
   * ========================================================== */

  function voiceLocalAvailable() {
    // v15：真机本地 TTS = Rust 侧 espeak-ng（启动日志 local-tts: available，perform_speech played=local）。
    // WebView 的 window.speechSynthesis 在 Linux WebKitGTK 基本不存在——用它探测必然误判
    // “本地无 TTS” → 误报「未配置」。真机一律按 Rust 侧为准（Rust 真不可用时 tts_speak 返回真实 reason）。
    if (isTauri) return true;
    return ('speechSynthesis' in window) && !!window.speechSynthesis;
  }

  function voiceCloudKey(v) {
    return String((v && v.cloud_api_key) || '').trim();
  }

  // 与 core resolve_engine 同规则（§2）：
  // !enabled → Mute("总开关已关闭")
  // auto:  cloud_key 有 → Cloud / local 可用 → Local / 否则 Mute("无可用引擎…")
  // local: 本地可用 → Local / 否则 Mute("本地 TTS 不可用")
  // cloud: 有 key → Cloud / 本地可用 → Local("云端无 key，降级本地 TTS") / 否则 Mute(…)
  function voiceResolveEngine(v, localAvailable) {
    if (!v.enabled) return { engine: 'mute', reason: '总开关已关闭' };
    const key = voiceCloudKey(v);
    if (v.engine === 'local') {
      if (localAvailable) return { engine: 'local', reason: '本地系统 TTS' };
      return { engine: 'mute', reason: '本地 TTS 不可用' };
    }
    if (v.engine === 'cloud') {
      if (key) return { engine: 'cloud', reason: '云端已配置' };
      if (localAvailable) return { engine: 'local', reason: '云端无 key，降级本地 TTS' };
      return { engine: 'mute', reason: '云端无 key 且本地无 TTS' };
    }
    // auto（默认）
    if (key) return { engine: 'cloud', reason: '云端已配置' };
    if (localAvailable) return { engine: 'local', reason: '本地系统 TTS' };
    return { engine: 'mute', reason: '无可用引擎（未配置云端 key 且本地无 TTS）' };
  }

  // 与 core scope_allows 同规则（§2）：all→全部；chat→仅 chat；reminders→仅 reminder
  function voiceScopeAllows(scope, kind) {
    if (scope === 'chat') return kind === 'chat';
    if (scope === 'reminders') return kind === 'reminder';
    return true; // all 及未知值兜底全放
  }

  /* ---- 迭代⑤：角色音色解析（与 core voice_profile.rs 同算法） ---- */

  // auto → 性格表默认；custom → 用户存值（clamp pitch/rate）
  function voiceResolveProfile(personaId, profile) {
    const p = profile || {};
    if (p.mode === 'custom') {
      return {
        mode: 'custom',
        pitch: Math.max(-10, Math.min(10, Number(p.pitch) || 0)),
        rate: Math.max(0.5, Math.min(2.0, Number(p.rate) || 1)),
        engine_voice_id: String(p.engine_voice_id || ''),
        cloud_speaker: String(p.cloud_speaker || ''),
      };
    }
    return defaultVoiceProfileFor(personaId);
  }

  // 语速描述（rate==1.1 归「轻快」，与冻结 5 标签一致）
  function voiceRateDesc(rate) {
    if (rate < 0.9) return '慢速';
    if (rate < 1.1) return '中速';
    return '轻快';
  }
  function voicePitchDesc(pitch) {
    if (pitch < -0.5) return '微低';
    if (pitch <= 0.5) return '中性';
    if (pitch <= 1.5) return '明亮';
    return '偏高';
  }
  function voiceTag(personaId, resolved) {
    const rd = voiceRateDesc(resolved.rate);
    const pd = voicePitchDesc(resolved.pitch);
    if (resolved.mode === 'custom') return '自定义 · ' + rd + ' · ' + pd;
    const tone = PERSONA_VOICE_TONES[personaId] || PERSONA_VOICE_TONES.gudian;
    return tone + ' · ' + rd + ' · ' + pd;
  }

  // 取某角色的生效音色：eff_rate = clamp(全局 voice.rate × profile.rate, 0.5, 2.0)；
  // pitch = profile.pitch；cloud_speaker = profile 非空 ? 它 : settings.voice.cloud_voice。
  function voiceResolveFor(personaId) {
    const obj = loadMockPersonas();
    const entity = (obj.personas || []).find((p) => p.persona_id === personaId);
    const resolved = voiceResolveProfile(personaId, entity && entity.voice_profile);
    const s = loadMockSettings();
    const gv = s.voice || {};
    const globalRate = Number(gv.rate) || 1;
    const effRate = Math.max(0.5, Math.min(2.0, globalRate * resolved.rate));
    const cloudSpeaker = resolved.cloud_speaker
      ? resolved.cloud_speaker
      : (gv.cloud_voice || '');
    return {
      persona_id: personaId,
      resolved,
      pitch: resolved.pitch,
      rate: effRate,            // 生效语速（用于 speechSynthesis / 广播 payload）
      cloud_speaker: cloudSpeaker,
      voice_tag: voiceTag(personaId, resolved),
    };
  }

  function mockTtsSpeak(args) {
    args = args || {};
    const text = String(args.text || '');
    const kind = args.kind || 'chat';
    const s = loadMockSettings();
    const v = s.voice || {};

    // 门控：总开关 / 范围 / 空文本 → mute（不广播指示器，与 Rust 一致）
    if (!v.enabled || !text.trim() || !voiceScopeAllows(v.scope, kind)) {
      return Promise.resolve({
        engine: 'mute', played: false, degraded: false,
        reason: '朗读未启用或范围未覆盖',
      });
    }

    const decision = voiceResolveEngine(v, voiceLocalAvailable());
    if (decision.engine === 'mute') {
      return Promise.resolve({
        engine: 'mute', played: false, degraded: false, reason: decision.reason,
      });
    }

    // 迭代⑤：解析本句角色音色（args.persona_id 缺省用 active_persona）
    const personaId = args.persona_id || activePersonaFromStore();
    const vres = voiceResolveFor(personaId);

    // 降级语义：决策为本地、但该角色指定了云端 speaker、且无 key → 明确提示
    let degraded = false;
    let detail = decision.reason;
    if (decision.engine === 'local' && vres.cloud_speaker && !voiceCloudKey(v)) {
      degraded = true;
      detail = '云端音色「' + vres.cloud_speaker + '」未配置 key，已用本地音色';
    }

    // local → 浏览器真发声；cloud → 模拟（无外部请求）
    // 迭代⑤：语速用 eff_rate；pitch 仅 mock 映射到 speechSynthesis 的 0..2（真机 pitch 由本地引擎参数负责）
    let played = false;
    if (decision.engine === 'local') {
      try {
        const u = new SpeechSynthesisUtterance(text);
        u.rate = Math.min(2, Math.max(0.5, vres.rate));
        u.pitch = Math.min(2, Math.max(0, 1 + vres.pitch / 5));
        u.volume = Math.min(1, Math.max(0, (Number(v.volume) || 80) / 100));
        window.speechSynthesis.speak(u);
        played = true;
      } catch (e) {
        console.warn('[mock] speechSynthesis failed', e);
      }
    }

    // 朗读指示器（Mute 不发；payload 与 Rust §3 同形，mock 额外带 text 供桌宠气泡演出）
    // 迭代⑤：payload 增 persona_id / pitch / voice_tag / cloud_speaker。
    broadcast('qibai://tts-speak', {
      kind,
      engine: decision.engine,
      volume: Number(v.volume) || 80,
      rate: vres.rate,
      state: 'speaking',
      text,
      persona_id: personaId,
      pitch: vres.pitch,
      voice_tag: vres.voice_tag,
      cloud_speaker: vres.cloud_speaker,
    });

    const res = {
      engine: decision.engine, played, degraded, detail,
      persona_id: personaId,
      rate: vres.rate,           // 生效语速（eff）
      pitch: vres.pitch,
      voice_tag: vres.voice_tag,
      cloud_speaker: vres.cloud_speaker,
    };
    if (decision.engine === 'cloud') res.audio_data_url = ''; // 模拟云端，未合成真实音频
    return Promise.resolve(res);
  }

  function mockTtsEngineStatus() {
    const s = loadMockSettings();
    const v = s.voice || {};
    const localAvailable = voiceLocalAvailable();
    const resolved = voiceResolveEngine(v, localAvailable);
    // 迭代⑤：附带当前主角色的生效音色（状态栏 / 降级提示用）
    const active = activePersonaFromStore();
    const vres = voiceResolveFor(active);
    return Promise.resolve({
      enabled: !!v.enabled,
      engine: v.engine || 'auto',
      volume: Number(v.volume) || 80,
      rate: vres.rate,           // 迭代⑤：当前主角色生效语速（eff）
      scope: v.scope || 'all',
      local_available: localAvailable,
      cloud_ready: !!voiceCloudKey(v),
      resolved: { engine: resolved.engine, reason: resolved.reason },
      active_persona: active,    // 迭代⑤
      voice_tag: vres.voice_tag, // 迭代⑤
      pitch: vres.pitch,         // 迭代⑤
      cloud_speaker: vres.cloud_speaker, // 迭代⑤
    });
  }

  function mockSttStatus() {
    const s = loadMockSettings();
    const v = s.voice || {};
    const key = String((v && v.stt_api_key) || '').trim();
    // 沙箱 chromium：file:// 非安全上下文，navigator.mediaDevices 通常 undefined
    const deviceAvailable = ('mediaDevices' in navigator) && !!navigator.mediaDevices;
    const enabled = !!v.stt_enabled;
    const available = enabled && !!key && deviceAvailable;
    let reason = 'STT 可用';
    if (!enabled) reason = 'STT 总开关未开启';
    else if (!key) reason = '未配置 STT Key（可通过环境变量 QIBAI_STT_KEY 提供）';
    else if (!deviceAvailable) reason = '麦克风设备不可用';
    return Promise.resolve({
      enabled,
      provider: v.stt_provider || 'doubao',
      key_ready: !!key,
      device_available: deviceAvailable,
      available,
      reason,
    });
  }

  /* ============================================================
   * 迭代① · 代码检查 mock（check_code）
   * 冻结契约（qibai-core/src/codecheck.rs）：
   *   CodeReport{source:"path"|"snippet", path, files_checked, language,
   *              summary, issues:[{severity,category,message,location,suggestion}], llm_note}
   * 只读红线：mock 绝不读写用户文件，仅按关键词/内存 snippet 构造演示报告。
   * ========================================================== */

  function buildCheckSummary(issues) {
    if (!issues || !issues.length) return '未发现问题';
    const e = issues.filter((i) => i.severity === 'error').length;
    const w = issues.filter((i) => i.severity === 'warning').length;
    const h = issues.filter((i) => i.severity === 'hint').length;
    return `发现 ${issues.length} 个问题：error ${e} / warning ${w} / hint ${h}`;
  }

  function checkLangOfPath(p) {
    const m = /\.([a-z0-9]+)$/i.exec(p || '');
    const ext = m ? m[1].toLowerCase() : '';
    return ({
      js: 'javascript', mjs: 'javascript', cjs: 'javascript',
      py: 'python', json: 'json', rs: 'rust',
      ts: 'typescript', tsx: 'typescript',
      html: 'html', htm: 'html', css: 'css', md: 'markdown', markdown: 'markdown', txt: 'text',
    })[ext] || 'unknown';
  }

  function sniffSnippetLang(snippet) {
    const t = String(snippet || '').trim();
    if (t.startsWith('{') || t.startsWith('[')) return 'json';
    if (/function|=>|\bvar\b|\blet\b|\bconst\b/.test(t)) return 'javascript';
    if (/\bdef\b|\bimport\b/.test(t)) return 'python';
    return 'unknown';
  }

  // snippet 启发式（codecheck.rs heuristic_scan 的最小对齐子集）
  function heuristicScanSnippet(text) {
    const issues = [];
    String(text || '').split('\n').forEach((raw, idx) => {
      const loc = `line ${idx + 1}`;
      if (raw.indexOf('eval(') !== -1) {
        issues.push({ severity: 'warning', category: 'security',
          message: '检测到 eval() 调用，可能导致注入风险', location: loc,
          suggestion: '避免使用 eval；可用 JSON.parse / 显式函数映射替代' });
      }
      if (/(api_?key|password|token|secret)/i.test(raw) && /[:=]\s*["'][^"']{4,}["']/.test(raw)) {
        issues.push({ severity: 'error', category: 'security',
          message: '检测到疑似硬编码的密钥/口令', location: loc,
          suggestion: '改为从环境变量或凭据管理读取，不要把密钥写进源码' });
      }
      if (/sk-[A-Za-z0-9]{8,}/.test(raw)) {
        issues.push({ severity: 'error', category: 'security',
          message: '检测到疑似硬编码的密钥/口令（sk- 前缀）', location: loc,
          suggestion: '改为从环境变量或凭据管理读取' });
      }
      // 宽松相等 == / !=（先剔除 === / !== 再判定）
      const stripped = raw.replace(/===/g, '').replace(/!==/g, '');
      if (stripped.indexOf('==') !== -1 || stripped.indexOf('!=') !== -1) {
        issues.push({ severity: 'warning', category: 'logic',
          message: '使用了宽松相等（== / !=），可能发生隐式类型转换', location: loc,
          suggestion: '改用严格相等 === / !==' });
      }
    });
    return issues;
  }

  function mockCheckCode(args) {
    args = args || {};
    const path = typeof args.path === 'string' && args.path.trim() ? args.path.trim() : null;
    const snippet = typeof args.snippet === 'string' && args.snippet ? args.snippet : null;
    if (!path && !snippet) {
      return Promise.reject(new Error('[mock] check_code 需要提供 path 或 snippet 之一'));
    }
    const llm_note = null; // 无 LLM key：纯静态报告（与后端降级路径一致）

    if (path) {
      const low = path.toLowerCase();
      const language = checkLangOfPath(path);
      // 坏报告：与 codecheck.rs 冻结形状一致（error/syntax 带 file:line + warning/security 带 suggestion）
      if (low.indexOf('bad') !== -1) {
        const issues = [
          { severity: 'error', category: 'syntax',
            message: 'SyntaxError: Missing semicolon.', location: `${path}:2`,
            suggestion: '根据报错行修正 JS 语法（括号/分号/引号）' },
          { severity: 'warning', category: 'security',
            message: '检测到 eval() 调用，可能导致注入风险', location: `${path}:4`,
            suggestion: '避免使用 eval；可用 JSON.parse / 显式函数映射替代' },
        ];
        return Promise.resolve({ source: 'path', path, files_checked: 1, language,
          summary: buildCheckSummary(issues), issues, llm_note });
      }
      if (low.indexOf('healthy') !== -1) {
        return Promise.resolve({ source: 'path', path, files_checked: 1, language,
          summary: '未发现问题', issues: [], llm_note });
      }
      // 其他路径：浏览器演示模式不读真实文件 → 干净 + hint 提示
      const issues = [
        { severity: 'hint', category: 'unknown',
          message: '浏览器演示模式：未读取真实文件（只读 mock），本报告为形状演示',
          location: path, suggestion: '建议本地使用真实检查器（真机由 qibai-core codecheck 执行）' },
      ];
      return Promise.resolve({ source: 'path', path, files_checked: 1, language,
        summary: buildCheckSummary(issues), issues, llm_note });
    }

    // snippet 模式：语言嗅探 + 启发式 + 真实语法校验（纯内存，不落盘）
    const language = (typeof args.language === 'string' && args.language && args.language !== 'auto')
      ? args.language : sniffSnippetLang(snippet);
    const issues = heuristicScanSnippet(snippet);
    if (language === 'json') {
      try { JSON.parse(snippet); } catch (e) {
        issues.push({ severity: 'error', category: 'syntax',
          message: `JSON 解析失败: ${e.message}`, location: 'line 1',
          suggestion: '检查逗号、引号、括号是否配对' });
      }
    }
    if (language === 'javascript') {
      try { new Function(String(snippet || '')); } catch (e) {
        const msg = (e && e.message) ? String(e.message) : String(e);
        issues.push({ severity: 'error', category: 'syntax',
          message: `SyntaxError: ${msg}`, location: 'line 1',
          suggestion: '修正 JS 语法（括号/分号/引号/变量声明）' });
      }
    }
    return Promise.resolve({ source: 'snippet', path: null, files_checked: 1, language,
      summary: buildCheckSummary(issues), issues, llm_note });
  }

  /* ============================================================
   * 迭代③ · 本地密码保险箱 mock（docs/VAULT_SPEC.md 冻结契约 v1）
   * 信封：{format:"qibai-vault", version:1, mode, kdf{PBKDF2-SHA256,210000,salt_b64},
   *       nonce_b64, aad, verifier_b64, ciphertext_b64} —— 与 core 同形。
   * 加密：优先 WebCrypto（PBKDF2-SHA256 + AES-256-GCM）；
   *       file:// 等非安全上下文 crypto.subtle 不可用时降级 base64 伪加密
   *       （仅流程演示，非安全存储，见下方 VAULT_WEBCRYPTO_OK 警告）。
   * 门控：active persona 亲密度 < 4 → vault_status.gated=true，写命令拒绝
   *       VaultGateLocked{required}；判定以后端为准，前端只展示。
   * ========================================================== */

  const VAULT_KDF_ITERATIONS = 210000;   // 冻结常量（≥210000）
  const VAULT_AAD = 'qibai-vault-v1';
  const VAULT_VERIFIER_PLAIN = 'qibai-vault-verifier-v1';
  const VAULT_REQUIRED_LEVEL = 4;
  const VAULT_KEYRING_KEY = 'qibai.mock.vault.keyring'; // sessionStorage：mock 无 OS 钥匙串，主密钥暂存于此（标签页内有效）

  // WebCrypto 可用性探测（file:// 非安全上下文 → subtle 缺失 → 走降级路径）
  const VAULT_WEBCRYPTO_OK = (function () {
    try {
      return !!(global.crypto && global.crypto.subtle && global.crypto.subtle.importKey);
    } catch (e) { return false; }
  })();
  if (!VAULT_WEBCRYPTO_OK) {
    console.warn('[qibai-mock] crypto.subtle 不可用（非安全上下文/file://），保险箱 mock 降级为 base64 伪加密：仅用于前端流程演示，不提供真实安全存储。真机/Tauri 由 qibai-core 做 AES-256-GCM。');
  }

  // 运行时状态（仅内存，不落盘；key/entries 在 locked 后即丢弃）
  const vaultRt = {
    initialized: false,
    state: 'unset',          // unset | locked | unlocked
    mode: 'none',            // none | keyring | master_password
    key: null,               // webcrypto: CryptoKey；降级: {fb:true, material:string}
    entries: [],
    failedAttempts: 0,
    cooldownUntil: 0,        // epoch ms
    lastActivity: 0,
    lockedReason: null,      // manual | idle | cooldown | null
    idleLockMin: 5,
    clipboardClearSecs: 20,
    maxWrongAttempts: 5,
    cooldownSecs: 30,
  };

  function vaultBufToB64(buf) {
    const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
    let bin = '';
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return btoa(bin);
  }
  function vaultB64ToBuf(b64) {
    const bin = atob(String(b64 || ''));
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  }
  function vaultRandomBytes(n) {
    const bytes = new Uint8Array(n);
    (global.crypto ? crypto : { getRandomValues: (x) => x }).getRandomValues(bytes);
    return bytes;
  }
  function vaultUuid() {
    try { if (global.crypto && crypto.randomUUID) return crypto.randomUUID(); } catch (e) { /* ignore */ }
    return 'entry-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 10);
  }
  function vaultErr(code, message, extra) {
    return Object.assign(new Error(message || code), { code }, extra || {});
  }

  function loadVaultEnvelope() {
    try {
      const raw = localStorage.getItem(LS_KEYS.vault);
      if (raw) {
        const env = JSON.parse(raw);
        if (env && env.format === 'qibai-vault') return env;
      }
    } catch (e) { /* ignore */ }
    return null;
  }
  function saveVaultEnvelope(env) {
    localStorage.setItem(LS_KEYS.vault, JSON.stringify(env));
  }

  function vaultActiveLevel() {
    try {
      const obj = loadMockIntimacy();
      const entry = obj.entries[activePersonaFromStore()];
      return entry ? intimacyLevelOf(entry.points) : 1;
    } catch (e) { return 1; }
  }
  function vaultGated() { return vaultActiveLevel() < VAULT_REQUIRED_LEVEL; }
  function vaultRequireGate() {
    if (vaultGated()) {
      throw vaultErr('VaultGateLocked', '亲密度等级不足，保险箱未解锁', { required: VAULT_REQUIRED_LEVEL });
    }
  }

  function vaultInit() {
    if (vaultRt.initialized) return;
    vaultRt.initialized = true;
    const s = (loadMockSettings().vault) || {};
    vaultRt.idleLockMin = s.idle_lock_min || 5;
    vaultRt.clipboardClearSecs = s.clipboard_clear_secs || 20;
    vaultRt.maxWrongAttempts = s.max_wrong_attempts || 5;
    vaultRt.cooldownSecs = s.cooldown_secs || 30;
    const env = loadVaultEnvelope();
    if (env) {
      vaultRt.state = 'locked';                       // 重启后一律保持锁定（key 不落盘）
      vaultRt.mode = env.mode === 'keyring' ? 'keyring' : 'master_password';
    }
  }
  function vaultTouch() { vaultRt.lastActivity = Date.now(); }
  function vaultTick() {
    if (vaultRt.state !== 'unlocked') return;
    if (Date.now() - vaultRt.lastActivity > Math.max(1, vaultRt.idleLockMin) * 60 * 1000) {
      vaultRt.state = 'locked';
      vaultRt.mode = (loadVaultEnvelope() || {}).mode || vaultRt.mode;
      vaultRt.entries = [];
      vaultRt.key = null;
      vaultRt.lockedReason = 'idle';
    }
  }

  // ---- 密钥派生 / 导入（webcrypto 或降级） ----
  async function vaultDeriveKey(password, saltBytes, iterations) {
    if (VAULT_WEBCRYPTO_OK) {
      const enc = new TextEncoder();
      const baseKey = await crypto.subtle.importKey('raw', enc.encode(String(password)), 'PBKDF2', false, ['deriveKey']);
      return crypto.subtle.deriveKey(
        { name: 'PBKDF2', salt: saltBytes, iterations: iterations, hash: 'SHA-256' },
        baseKey, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
    }
    // 降级：material 仅携带口令派生信息（伪），不提供真实安全
    return { fb: true, material: 'fb:pw:' + String(password) + ':' + vaultBufToB64(saltBytes) };
  }
  async function vaultImportRawKey(rawBytes) {
    if (VAULT_WEBCRYPTO_OK) {
      return crypto.subtle.importKey('raw', rawBytes, { name: 'AES-GCM' }, false, ['encrypt', 'decrypt']);
    }
    return { fb: true, material: 'fb:raw:' + vaultBufToB64(rawBytes) };
  }
  // 降级模式的"校验值"（非加密哈希，仅演示密码错误判定）
  function vaultFbVerifier(material) {
    let h = 5381;
    const s = VAULT_VERIFIER_PLAIN + '|' + material;
    for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
    return 'fbv1:' + (h >>> 0).toString(36);
  }

  // ---- 落盘密封（每次整体重加密，nonce 重新随机；salt 必须与 key 派生时一致） ----
  async function vaultSeal(key, mode, entries, saltBytes) {
    const enc = new TextEncoder();
    const salt = saltBytes || vaultRandomBytes(16);
    const nonce = vaultRandomBytes(12);
    const verifierNonce = vaultRandomBytes(12);
    if (VAULT_WEBCRYPTO_OK) {
      const ct = await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv: nonce, additionalData: enc.encode(VAULT_AAD) },
        key, enc.encode(JSON.stringify(entries)));
      const vf = await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv: verifierNonce, additionalData: enc.encode(VAULT_AAD) },
        key, enc.encode(VAULT_VERIFIER_PLAIN));
      // verifier_b64 = nonce(12B) || verifier密文（信封字段名保持冻结）
      const vfBlob = new Uint8Array(12 + vf.byteLength);
      vfBlob.set(verifierNonce, 0);
      vfBlob.set(new Uint8Array(vf), 12);
      return {
        format: 'qibai-vault', version: 1, mode: mode,
        kdf: { algo: 'PBKDF2-SHA256', iterations: VAULT_KDF_ITERATIONS, salt_b64: vaultBufToB64(salt) },
        nonce_b64: vaultBufToB64(nonce),
        aad: VAULT_AAD,
        verifier_b64: vaultBufToB64(vfBlob),
        ciphertext_b64: vaultBufToB64(new Uint8Array(ct)),
      };
    }
    // 降级伪加密：密文 = base64(JSON entries)（仅编码，非加密）
    return {
      format: 'qibai-vault', version: 1, mode: mode,
      kdf: { algo: 'PBKDF2-SHA256', iterations: VAULT_KDF_ITERATIONS, salt_b64: vaultBufToB64(salt) },
      nonce_b64: vaultBufToB64(nonce),
      aad: VAULT_AAD,
      verifier_b64: vaultFbVerifier(key.material),
      ciphertext_b64: vaultBufToB64(enc.encode(JSON.stringify(entries))),
    };
  }
  async function vaultCheckKey(key, env) {
    const enc = new TextEncoder();
    if (VAULT_WEBCRYPTO_OK) {
      try {
        const blob = vaultB64ToBuf(env.verifier_b64);
        const nonce = blob.slice(0, 12);
        const ct = blob.slice(12);
        const pt = await crypto.subtle.decrypt(
          { name: 'AES-GCM', iv: nonce, additionalData: enc.encode(env.aad || VAULT_AAD) },
          key, ct);
        return new TextDecoder().decode(pt) === VAULT_VERIFIER_PLAIN;
      } catch (e) { return false; }
    }
    return env.verifier_b64 === vaultFbVerifier(key.material);
  }
  async function vaultDecryptEntries(key, env) {
    if (VAULT_WEBCRYPTO_OK) {
      const enc = new TextEncoder();
      const pt = await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: vaultB64ToBuf(env.nonce_b64), additionalData: enc.encode(env.aad || VAULT_AAD) },
        key, vaultB64ToBuf(env.ciphertext_b64));
      return JSON.parse(new TextDecoder().decode(pt));
    }
    return JSON.parse(new TextDecoder().decode(vaultB64ToBuf(env.ciphertext_b64)));
  }
  // 变更后整体重加密落盘（nonce 重新随机；salt 沿用上一把 key 派生时的盐，
  // 否则下次解锁用新盐派生的 key 与信封 verifier 对不上）
  async function vaultRekeep() {
    const old = loadVaultEnvelope();
    const salt = old && old.kdf && old.kdf.salt_b64 ? vaultB64ToBuf(old.kdf.salt_b64) : vaultRandomBytes(16);
    const env = await vaultSeal(vaultRt.key, vaultRt.mode, vaultRt.entries, salt);
    saveVaultEnvelope(env);
  }

  // ---- IPC 命令（与冻结契约同名同形） ----
  async function mockVaultStatus() {
    vaultInit();
    vaultTick();
    return {
      state: vaultRt.state,
      mode: vaultRt.mode,
      keyring_available: true,                       // mock 恒 true（演示系统解锁路径）
      gated: vaultGated(),
      required_level: VAULT_REQUIRED_LEVEL,
      idle_lock_min: vaultRt.idleLockMin,
      max_wrong_attempts: vaultRt.maxWrongAttempts,
      cooldown_secs: vaultRt.cooldownSecs,
      clipboard_clear_secs: vaultRt.clipboardClearSecs,
      failed_attempts: vaultRt.failedAttempts,
      cooldown_remaining_secs: Math.max(0, Math.ceil((vaultRt.cooldownUntil - Date.now()) / 1000)),
      locked_reason: vaultRt.lockedReason,
      entries_count: vaultRt.state === 'unlocked' ? vaultRt.entries.length : 0,
    };
  }

  async function mockVaultSetup(args) {
    vaultInit();
    vaultRequireGate();
    if (vaultRt.state !== 'unset') throw vaultErr('VaultAlreadyUnlocked', '保险箱已初始化');
    const password = String((args && args.master_password) || '');
    if (!password) throw vaultErr('VaultWrongPassword', '主密码不能为空');
    const salt = vaultRandomBytes(16);
    const key = await vaultDeriveKey(password, salt, VAULT_KDF_ITERATIONS);
    saveVaultEnvelope(await vaultSeal(key, 'master_password', [], salt));
    vaultRt.state = 'locked';
    vaultRt.mode = 'master_password';
    vaultRt.key = null;
    vaultRt.entries = [];
    vaultRt.failedAttempts = 0;
    vaultRt.cooldownUntil = 0;
    vaultRt.lockedReason = null;
    return 'ok';
  }

  async function mockVaultSetupKeyring() {
    vaultInit();
    vaultRequireGate();
    if (vaultRt.state !== 'unset') throw vaultErr('VaultAlreadyUnlocked', '保险箱已初始化');
    const raw = vaultRandomBytes(32);
    const key = await vaultImportRawKey(raw);
    saveVaultEnvelope(await vaultSeal(key, 'keyring', [], vaultRandomBytes(16)));
    // mock 无系统钥匙串：主密钥暂存 sessionStorage（真机由 OS Keychain 托管）
    try { sessionStorage.setItem(VAULT_KEYRING_KEY, vaultBufToB64(raw)); } catch (e) { /* 隐私模式：仍落盘信封，演示系统解锁会失败提示 */ }
    vaultRt.state = 'locked';
    vaultRt.mode = 'keyring';
    vaultRt.key = null;
    vaultRt.entries = [];
    vaultRt.failedAttempts = 0;
    vaultRt.cooldownUntil = 0;
    vaultRt.lockedReason = null;
    return 'ok';
  }

  async function mockVaultUnlock(args) {
    vaultInit();
    vaultRequireGate();
    vaultTick();
    const env = loadVaultEnvelope();
    if (!env) throw vaultErr('VaultNotInitialized', '保险箱未初始化');
    if (vaultRt.state !== 'locked') throw vaultErr('VaultAlreadyUnlocked', '保险箱已解锁');
    if (vaultRt.mode !== 'master_password') throw vaultErr('VaultKeyringUnavailable', '当前为系统钥匙串模式，请使用「系统解锁」');
    const remain = Math.ceil((vaultRt.cooldownUntil - Date.now()) / 1000);
    if (remain > 0) throw vaultErr('VaultCooldown', `冷却中，请 ${remain} 秒后再试`, { remaining_secs: remain });
    const kdf = env.kdf || {};
    const key = await vaultDeriveKey(
      String((args && args.master_password) || ''),
      vaultB64ToBuf(kdf.salt_b64), kdf.iterations || VAULT_KDF_ITERATIONS);
    if (!(await vaultCheckKey(key, env))) {
      vaultRt.failedAttempts += 1;
      if (vaultRt.failedAttempts >= vaultRt.maxWrongAttempts) {
        vaultRt.cooldownUntil = Date.now() + vaultRt.cooldownSecs * 1000;
        vaultRt.failedAttempts = 0;
      }
      throw vaultErr('VaultWrongPassword', '主密码不正确');
    }
    vaultRt.entries = await vaultDecryptEntries(key, env);
    if (!Array.isArray(vaultRt.entries)) vaultRt.entries = [];
    vaultRt.key = key;
    vaultRt.state = 'unlocked';
    vaultRt.failedAttempts = 0;
    vaultRt.cooldownUntil = 0;
    vaultRt.lockedReason = null;
    vaultTouch();
    return 'ok';
  }

  async function mockVaultUnlockSystem() {
    vaultInit();
    vaultRequireGate();
    vaultTick();
    const env = loadVaultEnvelope();
    if (!env) throw vaultErr('VaultNotInitialized', '保险箱未初始化');
    if (vaultRt.state !== 'locked') throw vaultErr('VaultAlreadyUnlocked', '保险箱已解锁');
    if (vaultRt.mode !== 'keyring') throw vaultErr('VaultKeyringUnavailable', '当前为主密码模式，请输入主密码解锁');
    let rawB64 = null;
    try { rawB64 = sessionStorage.getItem(VAULT_KEYRING_KEY); } catch (e) { /* ignore */ }
    if (!rawB64) throw vaultErr('VaultKeyringUnavailable', '浏览器演示模式：系统钥匙串不可用（请重新初始化钥匙串模式）');
    const key = await vaultImportRawKey(vaultB64ToBuf(rawB64));
    if (!(await vaultCheckKey(key, env))) {
      throw vaultErr('VaultKeyringUnavailable', '系统钥匙串中的主密钥与本保险箱不匹配'); // 不计数失败（契约 §3）
    }
    vaultRt.entries = await vaultDecryptEntries(key, env);
    if (!Array.isArray(vaultRt.entries)) vaultRt.entries = [];
    vaultRt.key = key;
    vaultRt.state = 'unlocked';
    vaultRt.lockedReason = null;
    vaultTouch();
    return 'ok';
  }

  async function mockVaultLock() {
    vaultInit();
    vaultRt.state = 'locked';
    vaultRt.mode = (loadVaultEnvelope() || {}).mode || vaultRt.mode;
    vaultRt.entries = [];
    vaultRt.key = null;                 // 内存 key 立即清零（zeroize 同语义）
    vaultRt.lockedReason = 'manual';
    return 'ok';
  }

  function vaultRequireUnlocked() {
    vaultTick();
    vaultRequireGate();
    if (!loadVaultEnvelope()) throw vaultErr('VaultNotInitialized', '保险箱未初始化');
    if (vaultRt.state !== 'unlocked') throw vaultErr('VaultLocked', '保险箱已锁定');
  }

  async function mockVaultList() {
    vaultInit();
    vaultRequireUnlocked();
    vaultTouch();
    return vaultRt.entries.map((e) => Object.assign({}, e));
  }
  async function mockVaultSearch(args) {
    vaultInit();
    vaultRequireUnlocked();
    vaultTouch();
    const q = String((args && args.q) || '').trim().toLowerCase();
    if (!q) return vaultRt.entries.map((e) => Object.assign({}, e));
    return vaultRt.entries.filter((e) =>
      (e.site && e.site.toLowerCase().indexOf(q) !== -1) ||
      (e.account && e.account.toLowerCase().indexOf(q) !== -1) ||
      (e.note && e.note.toLowerCase().indexOf(q) !== -1)
    ).map((e) => Object.assign({}, e));
  }
  async function mockVaultAdd(args) {
    vaultInit();
    vaultRequireUnlocked();
    const entry = {
      id: vaultUuid(),
      site: String((args && args.site) || '').trim(),
      account: String((args && args.account) || ''),
      password: String((args && args.password) || ''),
      note: String((args && args.note) || ''),
      icon: String((args && args.icon) || ''),
      updated_at: Math.floor(Date.now() / 1000),
    };
    vaultRt.entries.push(entry);
    await vaultRekeep();
    vaultTouch();
    return { id: entry.id };
  }
  async function mockVaultUpdate(args) {
    vaultInit();
    vaultRequireUnlocked();
    const entry = vaultRt.entries.find((e) => e.id === (args && args.id));
    if (!entry) throw vaultErr('VaultNotFound', '条目不存在');
    if (args.site !== undefined) entry.site = String(args.site);
    if (args.account !== undefined) entry.account = String(args.account);
    if (args.note !== undefined) entry.note = String(args.note);
    if (args.icon !== undefined) entry.icon = String(args.icon);
    if (args.password) entry.password = String(args.password);   // 空串表示不改
    entry.updated_at = Math.floor(Date.now() / 1000);
    await vaultRekeep();
    vaultTouch();
    return 'ok';
  }
  async function mockVaultDelete(args) {
    vaultInit();
    vaultRequireUnlocked();
    const idx = vaultRt.entries.findIndex((e) => e.id === (args && args.id));
    if (idx < 0) throw vaultErr('VaultNotFound', '条目不存在');
    vaultRt.entries.splice(idx, 1);
    await vaultRekeep();
    vaultTouch();
    return 'ok';
  }
  async function mockVaultCopy(args) {
    vaultInit();
    vaultRequireUnlocked();
    const entry = vaultRt.entries.find((e) => e.id === (args && args.id));
    if (!entry) throw vaultErr('VaultNotFound', '条目不存在');
    vaultTouch();
    const clearSecs = vaultRt.clipboardClearSecs;
    let copied = false;
    // OS 剪贴板写操作（mock 层职责）；file:// 非安全上下文 navigator.clipboard 可能不可用/
    // 权限挂起 → 800ms 超时竞速，超时即降级内存 mock 剪贴板，绝不阻塞调用方
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await Promise.race([
          navigator.clipboard.writeText(entry.password),
          new Promise((_, rej) => setTimeout(() => rej(new Error('clipboard write timeout (file:// permission)')), 800)),
        ]);
        copied = true;
      }
    } catch (e) {
      copied = false;
      console.warn('[qibai-mock] navigator.clipboard 写入失败/超时（file:// 限制），降级为内存 mock 剪贴板', e);
    }
    if (!copied) global.__qibaiMockClipboard = entry.password;
    // 调度自动清除（与 core clear_after_secs 同语义）
    setTimeout(() => {
      if (!copied) { global.__qibaiMockClipboard = ''; return; }
      try { if (navigator.clipboard) navigator.clipboard.writeText(''); } catch (e) { /* ignore */ }
    }, clearSecs * 1000);
    return { copied: true, clear_after_secs: clearSecs }; // 载荷不含明文（契约 §4）
  }
  async function mockVaultExport() {
    vaultInit();
    vaultRequireUnlocked();
    vaultTouch();
    const env = loadVaultEnvelope();
    const payload = JSON.stringify(env, null, 2);
    // 测试/调试钩子：保留最近一次导出的信封对象（不影响下载行为）
    try { window.__qibaiMock.__lastExportPayload = JSON.parse(payload); } catch (e) {}
    const ts = new Date();
    const pad = (n) => String(n).padStart(2, '0');
    const name = `vault-backup-${ts.getFullYear()}${pad(ts.getMonth() + 1)}${pad(ts.getDate())}-${pad(ts.getHours())}${pad(ts.getMinutes())}.qbkv`;
    try {
      const blob = new Blob([payload], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = name;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
    } catch (e) { console.warn('[qibai-mock] 导出下载失败', e); }
    return { path: name };
  }
  async function mockVaultImportPayload(args) {
    vaultInit();
    vaultRequireGate();
    let payload;
    try { payload = JSON.parse(String((args && args.payload) || '')); }
    catch (e) { throw vaultErr('VaultImportFailed', '备份文件不是合法 JSON'); }
    if (!payload || payload.format !== 'qibai-vault' || !payload.ciphertext_b64) {
      throw vaultErr('VaultImportFailed', '备份文件格式不正确（应为 qibai-vault 信封）');
    }
    // 用备份自带 kdf + 传入密码派生 key 并校验（钥匙串模式用本地暂存主密钥）
    let key;
    if (payload.mode === 'keyring') {
      let rawB64 = null;
      try { rawB64 = sessionStorage.getItem(VAULT_KEYRING_KEY); } catch (e) { /* ignore */ }
      if (!rawB64) throw vaultErr('VaultKeyringUnavailable', '系统钥匙串不可用，无法导入钥匙串模式备份');
      key = await vaultImportRawKey(vaultB64ToBuf(rawB64));
    } else {
      const kdf = payload.kdf || {};
      key = await vaultDeriveKey(
        String((args && args.master_password) || ''),
        vaultB64ToBuf(kdf.salt_b64), kdf.iterations || VAULT_KDF_ITERATIONS);
    }
    let entries;
    try {
      if (!(await vaultCheckKey(key, payload))) throw new Error('bad key');
      entries = await vaultDecryptEntries(key, payload);
    } catch (e) {
      throw vaultErr('VaultImportFailed', '备份解密失败：主密码不正确或备份已损坏');
    }
    const localEnv = loadVaultEnvelope();
    if (!localEnv) {
      // 本地 unset：以该备份重建保险箱（保持锁定态，解锁后可见）
      saveVaultEnvelope(payload);
      vaultRt.state = 'locked';
      vaultRt.mode = payload.mode;
      vaultRt.entries = [];
      vaultRt.key = null;
      vaultRt.failedAttempts = 0;
      vaultRt.cooldownUntil = 0;
      vaultRt.lockedReason = null;
      return 'ok';
    }
    // 本地已初始化：需已解锁（持本地 key），以本地 key 重加密落盘，备份按 id 覆盖合并
    if (vaultRt.state !== 'unlocked') throw vaultErr('VaultLocked', '请先解锁保险箱再导入备份');
    const byId = {};
    vaultRt.entries.forEach((e) => { byId[e.id] = e; });
    (Array.isArray(entries) ? entries : []).forEach((e) => { byId[e.id] = e; });
    vaultRt.entries = Object.keys(byId).map((id) => byId[id]);
    await vaultRekeep();
    // 冻结契约：导入成功后统一回到 locked 状态（重建与合并均如此），随后用户按本地模式重新解锁
    vaultRt.state = 'locked';
    vaultRt.key = null;
    vaultRt.lockedReason = null;
    return 'ok';
  }

  // ---- 对话触发拦截（契约 §5：LLM 调用之前，本地判定，绝不进 LLM） ----
  const VAULT_PASSWORD_RE = /密码/;
  const VAULT_INTENT_RE = /密码.{0,6}(是什么|是多少|忘了|多少|帮我查|给我)/;

  function mockVaultIntercept(text) {
    vaultInit();
    if (!VAULT_PASSWORD_RE.test(text)) return null;
    const hasVaultWord = text.indexOf('保险箱') !== -1;
    const intentHit = VAULT_INTENT_RE.test(text);
    let entry = null;
    if (vaultRt.state === 'unlocked' && Array.isArray(vaultRt.entries)) {
      entry = vaultRt.entries.find((e) =>
        (e.site && text.indexOf(e.site) !== -1) ||
        (e.account && text.indexOf(e.account) !== -1));
    }
    if (!hasVaultWord && !intentHit && !entry) return null;

    const gated = vaultGated();
    if (gated) {
      const msg = '亲密度达到 L4（贴心）后会解锁本地密码保险箱，继续和我聊天吧～';
      return { result: { reply: msg, emotion: 'smiling', action: 'nod', skill: 'vault' }, speak_text: msg };
    }
    const env = loadVaultEnvelope();
    if (!env) {
      const msg = '先在控制台·保险箱页初始化（设置主密码）吧，之后我才能帮你取密码哦。';
      return { result: { reply: msg, emotion: 'smiling', action: 'nod', skill: 'vault' }, speak_text: msg };
    }
    if (vaultRt.state !== 'unlocked') {
      const msg = '保险箱已锁定，先在控制台解锁（主密码或系统解锁）后再问我吧。';
      return { result: { reply: msg, emotion: 'focused', action: 'nod', skill: 'vault' }, speak_text: msg };
    }
    if (!entry) {
      const msg = `没找到和「${text}」相关的密码记录哦。`;
      return { result: { reply: msg, emotion: 'surprised', action: 'shake', skill: 'vault' }, speak_text: msg };
    }
    // 命中：reply 含明文（仅当次返回已解锁会话 UI）；conversation/TTS 只走脱敏文本
    vaultTouch();
    const reply = `找到了～「${entry.site}」（${entry.account}）的密码是：${entry.password}`;
    const speak = `已为你复制「${entry.site}」的密码，${vaultRt.clipboardClearSecs} 秒后自动清除。`;
    return {
      result: { reply, emotion: 'smiling', action: 'nod', skill: 'vault',
                secret: true, secret_entry_id: entry.id, speak_text: speak },
      speak_text: speak,
    };
  }

  /* ============================================================
   * 迭代④·联网社交（浏览器 mock 直连真实本地服务端）
   * 冻结契约：docs/NETSOCIAL_SPEC.md §3(HTTP) / §4(WebSocket) / §7.5(命令)
   * - Tauri 真机路径：不动（走 invoke，Rust core 实现）
   * - 浏览器 mock：直连可配置 server_url（默认 http://127.0.0.1:8787），
   *   REST=fetch、实时通道=WebSocket(/ws?token=)；本地状态 + localStorage
   *   简化缓存；事件队列供 netsocial_poll 消费（环形上限 200）。
   * ========================================================== */
  const NS_LS_KEY = 'qibai.mock.netsocial.v1';
  const NS_DEFAULT_SERVER = 'http://127.0.0.1:8787';
  const NS_EVENT_LIMIT = 200;

  // 运行时（不持久化）
  const nsRt = {
    state: 'unregistered', // unregistered|registered|online|offline
    ws: null,
    intentionalClose: false,
    reconnectTimer: null,
    reconnectDelay: 1000,
    pingTimer: null,
    queue: [], // 事件队列（§7.4：connected/disconnected/hello/inbox/msg/presence/error）
    contacts: new Map(), // peer_key -> {peer_key,peer_nick,peer_color,peer_online,last_seen}
    messages: [], // {peer_key,text,ts,dir}
    pendingByBody: {}, // body -> [to,...]（self 回执匹配 peer 用）
    lastError: '',
  };

  function nsLoad() {
    try {
      const raw = localStorage.getItem(NS_LS_KEY);
      if (raw) return JSON.parse(raw);
    } catch (e) { /* ignore */ }
    return {};
  }
  function nsSave(obj) {
    try { localStorage.setItem(NS_LS_KEY, JSON.stringify(obj)); } catch (e) { /* ignore */ }
  }
  // 本地身份：首次生成 UUIDv4 anon_id（§7.1：客户端本地生成）
  function nsIdentity() {
    const id = nsLoad();
    if (!id.anon_id) {
      id.anon_id = nsUuidv4();
      id.nickname = '';
      id.color_idx = 0;
      id.soul_tags = { personality: [], interests: [], status: [] };
      id.city = '';
      id.avatar = 0;
      id.age_declared = '18plus';
      id.pure_mode = false;
      id.server_url = NS_DEFAULT_SERVER;
      nsSave(id);
    }
    return id;
  }
  function nsUuidv4() {
    try {
      if (global.crypto && crypto.randomUUID) return crypto.randomUUID();
    } catch (e) { /* ignore */ }
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
    });
  }
  // peer_key = sha256(anon_id) 前 12 hex（§7.2 脱敏缓存，绝不存原始 anon_id 明文）
  async function nsPeerKey(anonId) {
    if (!anonId) return '';
    try {
      const buf = new TextEncoder().encode(anonId);
      const dig = await crypto.subtle.digest('SHA-256', buf);
      return Array.from(new Uint8Array(dig)).map((b) => b.toString(16).padStart(2, '0')).join('').slice(0, 12);
    } catch (e) {
      // 非安全上下文降级（仅演示路径）：FNV-1a 双哈希截断
      let h1 = 0x811c9dc5, h2 = 0x01000190;
      for (let i = 0; i < anonId.length; i++) {
        const c = anonId.charCodeAt(i);
        h1 = ((h1 ^ c) * 2654435761) >>> 0;
        h2 = ((h2 ^ c) * 2246822519) >>> 0;
      }
      return (h1.toString(16).padStart(8, '0') + h2.toString(16).padStart(8, '0')).slice(0, 12);
    }
  }
  function nsApi(path, opts) {
    opts = opts || {};
    const id = nsIdentity();
    const base = (id.server_url || NS_DEFAULT_SERVER).replace(/\/$/, '');
    let u = base + path;
    if (opts.method !== 'POST' && opts.method !== 'PUT' && id.token) {
      u += (u.indexOf('?') >= 0 ? '&' : '?') + 'token=' + encodeURIComponent(id.token);
    }
    return fetch(u, {
      method: opts.method || 'GET',
      headers: { 'Content-Type': 'application/json' },
      body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    }).then(async (res) => {
      let data = null;
      try { data = await res.json(); } catch (e) { data = null; }
      if (!res.ok) {
        const err = new Error((data && (data.message || data.error)) || ('HTTP ' + res.status));
        err.code = (data && data.error) || ('http_' + res.status);
        throw err;
      }
      return data;
    });
  }
  function nsRequireOnline() {
    if (nsRt.state !== 'online') {
      const unreg = nsRt.state === 'unregistered';
      const err = new Error(unreg ? '尚未完成匿名注册：请先在社交页确认隐私声明'
        : '未连接服务器：请先在社交页填写服务端地址并点「连接」');
      err.code = unreg ? 'unregistered' : 'not_online';
      throw err;
    }
  }
  function nsEnqueue(type, payload) {
    nsRt.queue.push({ type, payload: payload || {} });
    if (nsRt.queue.length > NS_EVENT_LIMIT) nsRt.queue.splice(0, nsRt.queue.length - NS_EVENT_LIMIT);
  }
  async function nsRememberContact(anonId, profile) {
    const key = await nsPeerKey(anonId);
    const cur = nsRt.contacts.get(key) || {};
    nsRt.contacts.set(key, {
      peer_key: key,
      peer_nick: (profile && profile.nickname) || cur.peer_nick || '星客',
      peer_color: (profile && typeof profile.color_idx === 'number') ? profile.color_idx : (cur.peer_color || 0),
      peer_online: !!(profile && profile.online),
      last_seen: (profile && profile.last_seen) || cur.last_seen || Math.floor(Date.now() / 1000),
    });
    return nsRt.contacts.get(key);
  }
  function nsProfilePayload(id) {
    return {
      nickname: id.nickname || '星客',
      color_idx: typeof id.color_idx === 'number' ? id.color_idx : 0,
      soul_tags: id.soul_tags || { personality: [], interests: [], status: [] },
      city: id.city || '',
      avatar: typeof id.avatar === 'number' ? id.avatar : 0,
    };
  }
  function wsSendJson(obj) {
    if (nsRt.ws && nsRt.ws.readyState === 1) {
      try { nsRt.ws.send(JSON.stringify(obj)); return true; } catch (e) { return false; }
    }
    return false;
  }
  // ---- WebSocket（§4：/ws?token=，连接即鉴权） ----
  function nsOpenWs() {
    const id = nsIdentity();
    if (!id.token) {
      const err = new Error('尚未注册：请先完成匿名注册（POST /api/register 换取 token）');
      err.code = 'unregistered';
      throw err;
    }
    nsRt.intentionalClose = false;
    const wsUrl = (id.server_url || NS_DEFAULT_SERVER).replace(/^http/, 'ws').replace(/\/$/, '')
      + '/ws?token=' + encodeURIComponent(id.token);
    const ws = new WebSocket(wsUrl);
    nsRt.ws = ws;
    ws.onopen = () => {
      nsRt.state = 'online';
      nsRt.reconnectDelay = 1000;
      nsRt.lastError = '';
      nsEnqueue('connected', { server_url: id.server_url });
      // 客户端心跳 30s（§4.4）
      nsRt.pingTimer = setInterval(() => {
        if (ws.readyState === 1) {
          try { ws.send(JSON.stringify({ type: 'ping', ts: Math.floor(Date.now() / 1000) })); } catch (e) { /* ignore */ }
        }
      }, 30000);
    };
    ws.onmessage = (ev) => {
      let msg = null;
      try { msg = JSON.parse(String(ev.data)); } catch (e) { return; }
      nsHandleWsMessage(msg);
    };
    ws.onclose = () => {
      if (nsRt.pingTimer) { clearInterval(nsRt.pingTimer); nsRt.pingTimer = null; }
      nsRt.ws = null;
      if (nsRt.intentionalClose) {
        nsRt.state = 'offline';
        return;
      }
      if (nsRt.state === 'online') {
        nsRt.state = 'offline';
        nsEnqueue('disconnected', { reason: 'connection_lost' });
      }
      // 断线自动重连：指数退避 1s/2s/4s/8s（§7.4）
      nsRt.reconnectDelay = Math.min(nsRt.reconnectDelay * 2, 8000);
      nsRt.reconnectTimer = setTimeout(() => {
        nsRt.reconnectTimer = null;
        try { nsOpenWs(); } catch (e) { /* 下一次 close 再退避 */ }
      }, nsRt.reconnectDelay);
    };
  }
  async function nsHandleWsMessage(msg) {
    const id = nsIdentity();
    switch (msg.type) {
      case 'hello': {
        if (msg.profile) {
          id.nickname = msg.profile.nickname;
          id.color_idx = msg.profile.color_idx;
          nsSave(id);
        }
        nsEnqueue('hello', { profile: msg.profile || {} });
        break;
      }
      case 'inbox': {
        const msgs = Array.isArray(msg.messages) ? msg.messages : [];
        for (const m of msgs) {
          await nsCacheIncoming(m.sender, m.body, m.ts, 'in');
        }
        nsEnqueue('inbox', { count: msgs.length });
        break;
      }
      case 'msg.deliver': {
        if (msg.from === id.anon_id) {
          // 自己发出的回执（§4.4：from==自己 → 方向为「我发出」）
          const buf = nsRt.pendingByBody;
          const list = buf[msg.body];
          let to = null;
          if (list && list.length) { to = list.shift(); if (!list.length) delete buf[msg.body]; }
          let c = to ? nsRt.contacts.get(await nsPeerKey(to)) : null;
          if (c) {
            nsEnqueue('msg', {
              peer_key: c.peer_key, peer_nick: c.peer_nick, peer_color: c.peer_color,
              text: String(msg.body), ts: msg.ts || Math.floor(Date.now() / 1000), dir: 'out',
            });
          }
        } else {
          await nsCacheIncoming(msg.from, msg.body, msg.ts, 'in');
        }
        break;
      }
      case 'presence': {
        const key = await nsPeerKey(msg.who);
        const c = nsRt.contacts.get(key);
        if (c) { c.peer_online = !!msg.online; c.last_seen = msg.ts || c.last_seen; }
        nsEnqueue('presence', { peer_key: key, online: !!msg.online });
        break;
      }
      case 'error': {
        nsEnqueue('error', { code: msg.code || 'error', message: msg.message || '' });
        break;
      }
      default: break; // pong 等无需处理
    }
  }
  async function nsCacheIncoming(from, body, ts, dir) {
    const text = String(body == null ? '' : body);
    const t = ts || Math.floor(Date.now() / 1000);
    let c = nsRt.contacts.get(await nsPeerKey(from));
    if (!c) {
      try {
        const soul = await nsApi('/api/souls/' + encodeURIComponent(from));
        c = await nsRememberContact(from, soul.soul || soul);
      } catch (e) {
        c = await nsRememberContact(from, { nickname: '星客', color_idx: 0 });
      }
    }
    nsRt.messages.push({ peer_key: c.peer_key, text, ts: t, dir });
    nsEnqueue('msg', {
      peer_key: c.peer_key, peer_nick: c.peer_nick, peer_color: c.peer_color,
      text, ts: t, dir,
    });
  }

  // ---- §7.5 命令实现 ----
  function mockNsStatus() {
    const id = nsIdentity();
    return Promise.resolve({
      state: nsRt.state,
      anon_id: id.anon_id,
      nickname: id.nickname || '星客',
      color_idx: typeof id.color_idx === 'number' ? id.color_idx : 0,
      city: id.city || '',
      server_url: id.server_url || NS_DEFAULT_SERVER,
      online: nsRt.state === 'online',
      registered: !!id.token,
      last_error: nsRt.lastError || '',
    });
  }
  function mockNsRegister(args) {
    args = args || {};
    const id = nsIdentity();
    if (args.nickname !== undefined) id.nickname = String(args.nickname).trim().slice(0, 12);
    if (args.color_idx !== undefined) id.color_idx = Math.max(0, Math.min(11, parseInt(args.color_idx, 10) || 0));
    if (args.soul_tags) id.soul_tags = args.soul_tags;
    if (args.city !== undefined) id.city = String(args.city).trim();
    if (args.avatar !== undefined) id.avatar = Math.max(0, Math.min(4, parseInt(args.avatar, 10) || 0));
    if (args.age_declared !== undefined) id.age_declared = args.age_declared;
    if (args.pure_mode !== undefined) id.pure_mode = !!args.pure_mode;
    nsSave(id);
    if (id.token) {
      // 已有身份：仅更新 profile（在线则 PUT /api/me）
      if (nsRt.state === 'online') {
        nsApi('/api/me', { method: 'PUT', body: Object.assign({ token: id.token }, nsProfilePayload(id)) })
          .catch(() => { /* 本地已保存，不阻断 */ });
      }
      nsRt.state = nsRt.state === 'online' ? 'online' : 'registered';
      return Promise.resolve({ ok: true, anon_id: id.anon_id, state: nsRt.state, registered: true });
    }
    // 首次注册：POST /api/register 换 token（§3 注册规则）
    return nsApi('/api/register', {
      method: 'POST',
      body: Object.assign(nsProfilePayload(id), { age_declared: id.age_declared, pure_mode: !!id.pure_mode }),
    }).then((res) => {
      id.token = res.token;
      if (res.profile) {
        id.nickname = res.profile.nickname || id.nickname;
        id.color_idx = typeof res.profile.color_idx === 'number' ? res.profile.color_idx : 0;
      }
      nsSave(id);
      nsRt.state = 'registered';
      return { ok: true, anon_id: id.anon_id, state: nsRt.state, registered: true };
    }).catch((e) => {
      // 服务端不可达：仅本地保存（§7.1 离线占位）
      nsRt.state = 'registered';
      nsRt.lastError = e.message;
      nsSave(id);
      return { ok: true, anon_id: id.anon_id, state: 'registered', registered: false, offline_only: true, error: e.message };
    });
  }
  function mockNsConnect(args) {
    args = args || {};
    const id = nsIdentity();
    if (args.server_url) id.server_url = String(args.server_url).trim();
    if (!id.server_url) {
      const err = new Error('missing_server_url：请先填写服务端地址');
      err.code = 'missing_server';
      return Promise.reject(err);
    }
    nsSave(id);
    const ensure = id.token
      ? Promise.resolve({ registered: true })
      : mockNsRegister({});
    return ensure.then((r) => {
      if (!r.registered) {
        const err = new Error('无法连接服务端（' + id.server_url + '）：请确认本地服务端已启动');
        err.code = 'connect_failed';
        throw err;
      }
      if (nsRt.reconnectTimer) { clearTimeout(nsRt.reconnectTimer); nsRt.reconnectTimer = null; }
      nsOpenWs();
      // 等待 open（最多 5s）
      return new Promise((resolve) => {
        const t0 = Date.now();
        (function poll() {
          if (nsRt.state === 'online') return resolve();
          if (Date.now() - t0 > 5000) return resolve();
          setTimeout(poll, 100);
        })();
      });
    }).then(() => {
      if (nsRt.state !== 'online') {
        const err = new Error('连接服务端失败：' + id.server_url);
        err.code = 'connect_failed';
        throw err;
      }
      return { ok: true, state: nsRt.state };
    });
  }
  function mockNsDisconnect() {
    nsRt.intentionalClose = true;
    if (nsRt.reconnectTimer) { clearTimeout(nsRt.reconnectTimer); nsRt.reconnectTimer = null; }
    if (nsRt.ws) { try { nsRt.ws.close(); } catch (e) { /* ignore */ } }
    nsRt.state = 'offline';
    return Promise.resolve({ ok: true, state: nsRt.state });
  }
  function mockNsSetProfile(args) {
    args = args || {};
    const id = nsIdentity();
    if (args.nickname !== undefined) id.nickname = String(args.nickname).trim().slice(0, 12);
    if (args.color_idx !== undefined) id.color_idx = Math.max(0, Math.min(11, parseInt(args.color_idx, 10) || 0));
    if (args.soul_tags) id.soul_tags = args.soul_tags;
    if (args.city !== undefined) id.city = String(args.city).trim();
    if (args.avatar !== undefined) id.avatar = Math.max(0, Math.min(4, parseInt(args.avatar, 10) || 0));
    nsSave(id);
    if (nsRt.state === 'online' && id.token) {
      nsApi('/api/me', { method: 'PUT', body: Object.assign({ token: id.token }, nsProfilePayload(id)) })
        .catch(() => { /* 本地已保存 */ });
    }
    return Promise.resolve({ ok: true });
  }
  function mockNsRoam(args) {
    nsRequireOnline();
    const qs = [];
    if (args.tag) qs.push('tag=' + encodeURIComponent(args.tag));
    if (args.city) qs.push('city=' + encodeURIComponent(args.city));
    if (args.online !== undefined && args.online !== null && args.online !== '') {
      qs.push('online=' + (args.online ? '1' : '0'));
    }
    if (args.q) qs.push('q=' + encodeURIComponent(args.q));
    return nsApi('/api/souls' + (qs.length ? '?' + qs.join('&') : ''))
      .then((res) => ({ souls: res.souls || [] }));
  }
  function mockNsMatch(args) {
    nsRequireOnline();
    const qs = [];
    if (args.limit) qs.push('limit=' + encodeURIComponent(args.limit));
    return nsApi('/api/match' + (qs.length ? '?' + qs.join('&') : ''))
      .then((res) => ({ souls: res.souls || [] }));
  }
  function mockNsCitySouls(args) {
    nsRequireOnline();
    if (!args.city) { const err = new Error('city 必填'); err.code = 'bad_request'; return Promise.reject(err); }
    return nsApi('/api/city/' + encodeURIComponent(args.city) + '/souls')
      .then((res) => ({ souls: res.souls || [], city: res.city || args.city }));
  }
  function mockNsLike(args) {
    nsRequireOnline();
    const id = nsIdentity();
    return nsApi('/api/souls/' + encodeURIComponent(args.id) + '/like', {
      method: 'POST', body: { token: id.token },
    }).then((res) => ({ liked: !!res.liked, matched: !!res.matched, mutual: !!res.mutual }));
  }
  function mockNsReport(args) {
    nsRequireOnline();
    const id = nsIdentity();
    return nsApi('/api/souls/' + encodeURIComponent(args.id) + '/report', {
      method: 'POST', body: { token: id.token, reason: String(args.reason || '') },
    }).then(() => ({ ok: true }));
  }
  function mockNsBlock(args) {
    nsRequireOnline();
    const id = nsIdentity();
    return nsApi('/api/souls/' + encodeURIComponent(args.id) + '/block', {
      method: 'POST', body: { token: id.token },
    }).then((res) => ({ ok: true, blocked: !!res.blocked }));
  }
  function mockNsSend(args) {
    nsRequireOnline();
    const id = nsIdentity();
    const to = String(args.id || '');
    const text = String(args.text || '').trim();
    if (!to || !text) { const err = new Error('id 与 text 必填'); err.code = 'bad_request'; return Promise.reject(err); }
    nsRt.pendingByBody[text] = nsRt.pendingByBody[text] || [];
    nsRt.pendingByBody[text].push(to);
    wsSendJson({ type: 'msg.send', to, body: text }); // §4.3 客户端→服务端
    // 缓存联系人（脱敏）：最佳努力拉取对方 profile，失败用占位昵称
    return nsApi('/api/souls/' + encodeURIComponent(to))
      .then((soul) => nsRememberContact(to, soul.soul || soul))
      .catch(() => nsRememberContact(to, null))
      .then((c) => {
        nsRt.messages.push({ peer_key: c.peer_key, text, ts: Math.floor(Date.now() / 1000), dir: 'out' });
        return { ok: true };
      });
  }
  function mockNsHistory(args) {
    args = args || {};
    let contacts = Array.from(nsRt.contacts.values()).map((c) => Object.assign({}, c));
    let messages = nsRt.messages.slice();
    if (args.id) {
      // id 兼容 anon_id 或 peer_key（12 hex）
      let p = String(args.id);
      const keyPromise = /^[0-9a-f]{12}$/.test(p) ? Promise.resolve(p) : nsPeerKey(p);
      return keyPromise.then((key) => {
        return { contacts: contacts.filter((c) => c.peer_key === key), messages: messages.filter((m) => m.peer_key === key) };
      });
    }
    return Promise.resolve({ contacts, messages });
  }
  function mockNsPoll() {
    const evs = nsRt.queue.splice(0, nsRt.queue.length);
    return Promise.resolve({ events: evs });
  }
  function mockNsTopics() {
    nsRequireOnline();
    return nsApi('/api/topics').then((res) => ({ topics: res.topics || [] }));
  }
  function mockNsEvents() {
    nsRequireOnline();
    return nsApi('/api/events').then((res) => ({ events: res.events || [] }));
  }
  function mockNsCacheClear() {
    nsRt.intentionalClose = true;
    if (nsRt.ws) { try { nsRt.ws.close(); } catch (e) { /* ignore */ } }
    localStorage.removeItem(NS_LS_KEY);
    nsRt.contacts.clear();
    nsRt.messages = [];
    nsRt.queue = [];
    nsRt.pendingByBody = {};
    nsRt.state = 'unregistered';
    return Promise.resolve({ ok: true });
  }

  /* ============================================================
   * 插件与专家（PLUGIN_IMPL_CONTRACT §1/§3.6/§6）mock 适配层
   * ------------------------------------------------------------
   * - plugin_list 输出形状与契约一致（builtin 6 个 + 已安装外部示例
   *   + 坏 manifest 扫描错误演示）。
   * - 示例插件（§6 fixtures 的前端 mock 版）：
   *   com.qibai.datetime（技能，command:run）/ com.qibai.scientist（专家）
   * - installed 集合仅存内存（mock 会话级）；enabled/grants/experts
   *   落 localStorage settings.plugins（与 Rust 侧 PluginSettings 同形）。
   * ========================================================== */

  // 内置 6 技能包（contract §1；legacy=null 表示无旧开关，启用默认 true）
  const BUILTIN_PLUGINS = [
    { id: 'com.qibai.launch_app',     fn: 'launch_app',      name: '打开应用',   description: '元气小小可帮你启动常用程序', permissions: ['system:launch'], legacy: 'launch_app' },
    { id: 'com.qibai.open_folder',    fn: 'open_folder',    name: '打开文件夹', description: '元气小小可帮你打开本地目录', permissions: ['system:launch'], legacy: 'open_folder' },
    { id: 'com.qibai.set_reminder',   fn: 'set_reminder',    name: '设置提醒',   description: '元气小小可创建新的定时提醒', permissions: ['reminder:set'],   legacy: 'set_reminder' },
    { id: 'com.qibai.check_code',     fn: 'check_code',      name: '代码检查',   description: '只读扫描本地代码片段',   permissions: ['files:read'],     legacy: 'check_code' },
    { id: 'com.qibai.open_social',    fn: 'open_social',     name: '星球广场',   description: '聊天指令触发的只读社交跳转', permissions: ['network:read'], legacy: null },
    { id: 'com.qibai.execute_command',fn: 'execute_command', name: '执行命令',   description: '危险命令，每次执行前逐次确认', permissions: ['command:run'], legacy: null },
  ];

  // 外部示例插件（contract §6 字段一致）
  const EXTERNAL_EXAMPLES = [
    {
      id: 'com.qibai.datetime', type: 'skill', name: '日期时间',
      description: '命令行技能：输出当前日期与时间（stdin JSON 入参）',
      version: '1.0.0', author: 'Qibai Labs',
      permissions: ['command:run'], entry_kind: 'command',
      functions: [{ name: 'get_datetime', description: '获取当前日期与时间', parameters: { type: 'object', properties: {}, required: [] } }],
    },
    {
      id: 'com.qibai.scientist', type: 'expert', name: '科学家助手',
      description: '严谨专业的科研对话伙伴',
      version: '1.0.0', author: 'Qibai Labs',
      system_prompt: '你是一位严谨、简洁的科研助手：回答基于证据，先给结论再给依据；对不确定的事明确标注边界；全程保持全龄友好。',
      skills: ['com.qibai.launch_app', 'com.qibai.open_folder'],
      persona_binding: { voice: 'gudian', mood: 'professional' },
    },
  ];

  // 坏 manifest 扫描演示（registry 拒绝该目录并记录 error，不中断整体扫描）
  const BROKEN_EXAMPLE = {
    id: 'com.qibai.broken', type: 'skill', name: '损坏示例',
    description: 'manifest 不合规，已被插件目录扫描拒绝',
    version: '—', author: '未知',
    error: 'manifest 解析失败：version 缺失（期望 x.y.z），该目录已被跳过',
  };

  // 会话级 installed 集合（mock 不落盘；enabled/grants/experts 落 settings.plugins）
  const pluginRt = { installed: [] };

  function pluginsSettings() {
    const s = loadMockSettings();
    // 增量迁移（contract §2，幂等）：plugins.enabled 为空 → 从旧 skills 播种内置启停
    if (s.plugins && Object.keys(s.plugins.enabled || {}).length === 0) {
      BUILTIN_PLUGINS.forEach((p) => {
        s.plugins.enabled[p.id] = p.legacy ? s.skills[p.legacy] !== false : true;
      });
      saveMockSettings(s);
    }
    return s;
  }

  function mockPluginList() {
    const s = pluginsSettings();
    const out = [];
    // 内置 6（trusted builtin：grants 默认全授，无需安装确认）
    BUILTIN_PLUGINS.forEach((p) => {
      out.push({
        id: p.id, type: 'skill', name: p.name, description: p.description,
        icon: null, version: '1.0.0', author: '元气小小内置',
        permissions: p.permissions, granted: p.permissions.slice(),
        enabled: s.plugins.enabled[p.id] !== false, installed: true,
        entry_kind: 'builtin',
        functions: [{ name: p.fn, description: p.description, parameters: {} }],
        error: null, builtin: true,
      });
    });
    // 已安装的外部示例（默认 enabled=false、grants=[]）
    EXTERNAL_EXAMPLES.forEach((ex) => {
      if (!pluginRt.installed.includes(ex.id)) return;
      const grants = (s.plugins.grants && s.plugins.grants[ex.id]) || [];
      out.push({
        id: ex.id, type: ex.type, name: ex.name, description: ex.description,
        icon: null, version: ex.version, author: ex.author,
        permissions: ex.permissions || [], granted: grants,
        enabled: s.plugins.enabled[ex.id] === true, installed: true,
        entry_kind: ex.entry_kind || null,
        functions: ex.functions || [],
        error: null, builtin: false,
      });
    });
    // 坏 manifest（扫描结果，不可安装）
    out.push({
      id: BROKEN_EXAMPLE.id, type: 'skill', name: BROKEN_EXAMPLE.name,
      description: BROKEN_EXAMPLE.description, icon: null, version: BROKEN_EXAMPLE.version,
      author: BROKEN_EXAMPLE.author, permissions: [], granted: [],
      enabled: false, installed: false, entry_kind: null, functions: [],
      error: BROKEN_EXAMPLE.error, builtin: false,
    });
    return Promise.resolve(out);
  }

  function mockPluginInstall() {
    // mock 模拟安装：把已存在于示例目录的插件追加到列表（幂等：已安装则跳过）
    const added = [];
    EXTERNAL_EXAMPLES.forEach((ex) => {
      if (!pluginRt.installed.includes(ex.id)) {
        pluginRt.installed.push(ex.id);
        added.push(ex.id);
      }
    });
    console.log('[mock] plugin_install, added:', added);
    return Promise.resolve('ok');
  }

  function mockPluginUninstall(id) {
    pluginRt.installed = pluginRt.installed.filter((x) => x !== id);
    const s = loadMockSettings();
    if (s.plugins) {
      delete s.plugins.enabled[id];
      delete s.plugins.grants[id];
      delete s.plugins.experts[id];
    }
    saveMockSettings(s);
    return Promise.resolve('ok');
  }

  function mockPluginSetEnabled(id, enabled) {
    const s = pluginsSettings();
    s.plugins.enabled[id] = !!enabled;
    saveMockSettings(s);
    return Promise.resolve('ok');
  }

  function mockGrantPermission(id, perm, approve) {
    // files:write 对外部插件硬拒（contract §4 红线）
    const isBuiltin = BUILTIN_PLUGINS.some((p) => p.id === id);
    if (perm === 'files:write' && !isBuiltin) {
      return Promise.reject(new Error('[mock] files:write 对外部插件不可授予（安全红线）'));
    }
    const s = pluginsSettings();
    const list = (s.plugins.grants && s.plugins.grants[id]) || [];
    const idx = list.indexOf(perm);
    if (approve && idx < 0) list.push(perm);
    if (!approve && idx >= 0) list.splice(idx, 1);
    s.plugins.grants[id] = list;
    saveMockSettings(s);
    return Promise.resolve('ok');
  }

  function mockExpertList() {
    const s = pluginsSettings();
    const out = [];
    EXTERNAL_EXAMPLES
      .filter((ex) => ex.type === 'expert' && pluginRt.installed.includes(ex.id))
      .forEach((ex) => {
        out.push({
          id: ex.id, name: ex.name, description: ex.description,
          version: ex.version, author: ex.author,
          system_prompt: ex.system_prompt, skills: ex.skills || [],
          persona_binding: ex.persona_binding || null,
          active: !!(s.plugins.experts && s.plugins.experts[ex.id]),
        });
      });
    return Promise.resolve(out);
  }

  function mockExpertSetActive(id) {
    const s = pluginsSettings();
    // 空串 = 停用（contract §3.6：同时仅一位专家）
    s.plugins.experts = {};
    if (id) s.plugins.experts[id] = true;
    saveMockSettings(s);
    return Promise.resolve('ok');
  }

  // ---- 连接器 / 技能包 mock（T4 扩展：浏览器独立模式） ----
  function mockConnectors() {
    try { return JSON.parse(localStorage.getItem('qibai.mock.connectors') || '[]'); } catch (e) { return []; }
  }
  function saveMockConnectors(list) {
    localStorage.setItem('qibai.mock.connectors', JSON.stringify(list));
  }
  function mockConnectorList() {
    const out = mockConnectors().map((c) => ({
      id: c.id, type: 'connector', name: c.name, description: c.description || '',
      icon: null, version: '1.0.0', author: '用户自定义',
      permissions: c.kind === 'mcp' ? ['command:run'] : ['network:read'],
      granted: c.kind === 'mcp' ? ['command:run'] : ['network:read'],
      enabled: c.enabled !== false, installed: true,
      entry_kind: c.kind,
      functions: c.kind === 'mcp'
        ? [{ name: '__mcp__', description: 'MCP stdio 工具' }]
        : [{ name: '__http__', description: 'HTTP 端点调用' }],
      error: null, builtin: false,
    }));
    return Promise.resolve(out);
  }
  function mockConnectorAdd(args) {
    const list = mockConnectors();
    if (list.some((c) => c.id === args.id)) {
      return Promise.reject(new Error('连接器已存在：' + args.id));
    }
    list.push({
      id: args.id, name: args.name, kind: args.kind,
      command: args.command || '', args: args.args || [],
      url: args.url || '', headers: args.headers || {},
      description: args.description || '', enabled: args.enabled !== false,
    });
    saveMockConnectors(list);
    return Promise.resolve('ok');
  }
  function mockConnectorRemove(args) {
    saveMockConnectors(mockConnectors().filter((c) => c.id !== args.id));
    return Promise.resolve('ok');
  }
  function mockSkillPackList() {
    return mockPluginList().then((list) =>
      list.filter((p) => !p.builtin && p.installed && !p.error)
    );
  }

  function mockInvoke(cmd, args) {
  try {
    switch (cmd) {
      case 'get_settings':
        return Promise.resolve(loadMockSettings());
      case 'save_settings':
        return Promise.resolve(saveMockSettings(args.settings || args));
      case 'send_message':
        return mockSendMessage(args.text || '');
      case 'get_conversation':
        return Promise.resolve(loadMockConversation());
      case 'get_skills_status': {
        const s = loadMockSettings().skills;
        return Promise.resolve({
          total_enabled: s.total_enabled,
          skills: {
            launch_app: s.launch_app,
            open_folder: s.open_folder,
            set_reminder: s.set_reminder,
            check_code: s.check_code,
          },
        });
      }
      case 'set_skill_enabled': {
        const s = loadMockSettings();
        s.skills[args.skill] = args.enabled;
        saveMockSettings(s);
        return Promise.resolve('ok');
      }
      case 'approve_skill':
        console.log('[mock] approve_skill', args);
        return Promise.resolve('ok');
      // ---- 插件与专家（PLUGIN_IMPL_CONTRACT §3.6） ----
      case 'plugin_list':
        return mockPluginList();
      case 'plugin_install':
        return mockPluginInstall(args.dir);
      case 'plugin_uninstall':
        return mockPluginUninstall(args.id);
      case 'plugin_set_enabled':
        return mockPluginSetEnabled(args.id, args.enabled);
      case 'plugin_grant_permission':
        return mockGrantPermission(args.id, args.permission, args.approve);
      case 'expert_list':
        return mockExpertList();
      case 'expert_set_active':
        return mockExpertSetActive(args.id);
      // ---- 连接器 / 技能包（T4 扩展） ----
      case 'connector_list':
        return mockConnectorList();
      case 'connector_add':
        return mockConnectorAdd(args);
      case 'connector_remove':
        return mockConnectorRemove(args);
      case 'skill_pack_list':
        return mockSkillPackList();
      case 'skill_pack_install':
        return Promise.resolve('ok');
      case 'skill_pack_export':
        return Promise.resolve('ok');
      // ---- P0/P1 多角色社交（冻结契约 §四） ----
      case 'get_personas':
        return mockGetPersonas();
      case 'set_active_persona':
        return mockSetActivePersona(args);
      case 'set_persona_voice':
        return mockSetPersonaVoice(args); // 迭代⑤：每角色专属音色
      case 'rename_persona':
        return mockRenamePersona(args);
      case 'get_social_state':
        return mockGetSocialState();
      case 'social_greet':
        return mockSocialGreet(args);
      case 'social_pk':
        return mockSocialPk(args);
      case 'get_social_events':
        return mockGetSocialEvents(args);
      // ---- 亲密度 L1-L10（冻结契约 §6） ----
      case 'get_intimacy_levels':
        return mockGetIntimacyLevels();
      case 'get_intimacy_state':
        return mockGetIntimacyState(args);
      case 'intimacy_event':
        return doIntimacyEvent(args.persona_id, args.kind, args.detail);
      case 'set_anniversary':
        return mockSetAnniversary(args);
      case 'clear_anniversary':
        return mockClearAnniversary(args);
      case 'check_gate':
        return mockCheckGate(args);
      // ---- 双窗口扩展（WINDOW_ARCH.md §2.3/§2.4/§7） ----
      case 'window_identity':
        return Promise.resolve({ label: mockWindowLabel });
      case 'get_desk_state':
        return mockGetDeskState();
      case 'pet_summon':
        return mockPetSummon(args);
      case 'pet_dismiss':
        return mockPetDismiss(args);
      case 'pet_set_locked':
        return mockPetSetLocked(args);
      case 'console_open':
        return mockConsoleOpen();
      // ---- v18 UI 面板（气泡卡片 + 独立面板窗） ----
      case 'ui_panel_show': {
        const p = args.payload || args || {};
        if ((p.mode || 'window') === 'bubble') {
          mockEmit('qibai://ui-panel', { title: p.title, html: p.html });
        } else {
          console.log('[mock] ui_panel_show → 面板窗内容', p.title);
          mockEmit('qibai://panel-content', { title: p.title, html: p.html });
        }
        return Promise.resolve('ok');
      }
      case 'ui_panel_close':
        console.log('[mock] ui_panel_close');
        return Promise.resolve('ok');
      case 'chat_log_panel': {
        const rows = (loadMockConversation() || [])
          .slice(-30)
          .map(function (m) {
            return '<tr><td>' + (m.role === 'user' ? '我' : '元气小小') +
              '</td><td>' + (new Date((m.ts || 0) * 1000).toLocaleString('zh-CN')) +
              '</td><td>' + String(m.content || '').replace(/[<>&]/g, function (c) {
                return { '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c];
              }) + '</td></tr>';
          }).join('');
        mockEmit('qibai://panel-content', {
          title: '最近聊天记录',
          html: '<table class="qibai-table"><thead><tr><th>角色</th><th>时间</th><th>内容</th></tr></thead><tbody>' + rows + '</tbody></table>',
        });
        return Promise.resolve('ok');
      }
      case 'skills_panel':
      case 'experts_panel': {
        const list = cmd === 'skills_panel' ? mockPluginList() : mockExpertList();
        let html;
        if (cmd === 'skills_panel') {
          html = '<table class="qibai-table"><thead><tr><th>名称</th><th>说明</th><th>ID</th></tr></thead><tbody>' +
            (list || []).filter(function (p) { return p.type === 'skill'; }).map(function (p) {
              return '<tr><td>' + String(p.name || '') + '</td><td>' + String(p.description || '') +
                '</td><td>' + String(p.id || '') + '</td></tr>';
            }).join('') + '</tbody></table>';
        } else {
          html = '<table class="qibai-table"><thead><tr><th>名称</th><th>说明</th><th>ID</th></tr></thead><tbody>' +
            (list || []).map(function (p) {
              return '<tr><td>' + String(p.name || '') + '</td><td>' + String(p.description || '') +
                '</td><td>' + String(p.id || '') + '</td></tr>';
            }).join('') + '</tbody></table>';
        }
        mockEmit('qibai://panel-content', { title: cmd === 'skills_panel' ? '技能库' : '专家库', html: html });
        return Promise.resolve('ok');
      }
      // ---- v18.3 屏幕「眼睛 + 手」（浏览器冒烟 mock） ----
      case 'screen_capture': {
        // 用 canvas 画一张示意"屏幕"（含可点击坐标网格），冒烟验证换算与交互
        const c = document.createElement('canvas');
        c.width = 960; c.height = 540;
        const ctx = c.getContext('2d');
        ctx.fillStyle = '#FFF9F4'; ctx.fillRect(0, 0, 960, 540);
        ctx.strokeStyle = '#E58FAD'; ctx.lineWidth = 2;
        for (let gx = 0; gx <= 960; gx += 120) { ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, 540); ctx.stroke(); }
        for (let gy = 0; gy <= 540; gy += 90) { ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(960, gy); ctx.stroke(); }
        ctx.fillStyle = '#9B7FB8'; ctx.font = '18px sans-serif';
        ctx.fillText('元气小小 · 屏幕冒烟（960×540）—— 点击网格交叉点测试坐标换算', 24, 40);
        ctx.fillStyle = '#E58FAD'; ctx.fillRect(420, 220, 120, 100); // 一个"按钮"
        ctx.fillStyle = '#fff'; ctx.font = '16px sans-serif'; ctx.fillText('点我', 460, 280);
        const b64 = c.toDataURL('image/png').split(',')[1];
        mockEmit('qibai://screen-shot', { image: b64, w: 960, h: 540 });
        return Promise.resolve({ ok: true, w: 960, h: 540 });
      }
      case 'screen_gui_state':
        return Promise.resolve({ enabled: window.__qibaiPermMock === true, granted: window.__qibaiPermMock === true });
      case 'screen_gui_set': {
        window.__qibaiPermMock = !!(args && args.enabled);
        return Promise.resolve('ok');
      }
      case 'screen_gui_ack':
        return Promise.resolve('ok');
      case 'screen_click':
        console.log('[mock] screen_click', args.x, args.y);
        return Promise.resolve('ok');
      case 'screen_type':
        console.log('[mock] screen_type', (args && args.text) || '');
        return Promise.resolve('ok');
      case 'screen_key':
        console.log('[mock] screen_key', (args && args.keys) || '');
        return Promise.resolve('ok');
      case 'screen_scroll':
        console.log('[mock] screen_scroll', args && args.dy);
        return Promise.resolve('ok');
      case 'screen_activate_window':
        console.log('[mock] screen_activate_window', (args && args.title) || '');
        return Promise.resolve('ok');
      case 'pets_show':
      case 'pets_hide':
        return Promise.resolve('ok');
      case 'app_exit':
        console.log('[mock] app_exit（浏览器演示模式不退出）');
        return Promise.resolve('ok');
      // ---- 迭代①·全屏守护（冻结契约：GUARD.md） ----
      case 'guard_trigger':
        return mockGuardTrigger(args);
      case 'guard_skip':
        return mockGuardSkip(args);
      case 'guard_system_action':
        return mockGuardSystemAction(args);
      // ---- 迭代①·代码检查（冻结契约：codecheck.rs CodeReport） ----
      case 'check_code':
        return mockCheckCode(args);
      // ---- 迭代②·语音（冻结契约：_VOICE_ITER2_SPEC.md §3/§4.4） ----
      case 'tts_speak':
        return mockTtsSpeak(args);
      case 'tts_engine_status':
        return mockTtsEngineStatus();
      case 'stt_status':
        return mockSttStatus(args);
      // ---- 迭代③·本地密码保险箱（冻结契约：VAULT_SPEC.md §4） ----
      case 'vault_status':
        return mockVaultStatus();
      case 'vault_setup':
        return mockVaultSetup(args);
      case 'vault_setup_keyring':
        return mockVaultSetupKeyring();
      case 'vault_unlock':
        return mockVaultUnlock(args);
      case 'vault_unlock_system':
        return mockVaultUnlockSystem();
      case 'vault_lock':
        return mockVaultLock();
      case 'vault_list':
        return mockVaultList();
      case 'vault_search':
        return mockVaultSearch(args);
      case 'vault_add':
        return mockVaultAdd(args);
      case 'vault_update':
        return mockVaultUpdate(args);
      case 'vault_delete':
        return mockVaultDelete(args);
      case 'vault_copy':
        return mockVaultCopy(args);
      case 'vault_export':
        return mockVaultExport();
      case 'vault_import_payload':
        return mockVaultImportPayload(args);
      // ---- 迭代④·联网社交（冻结契约：NETSOCIAL_SPEC.md §7.5） ----
      // 浏览器 mock：直连真实本地服务端；Tauri 真机走 invoke（Rust core 实现）
      case 'netsocial_status':
        return mockNsStatus();
      case 'netsocial_register':
        return mockNsRegister(args);
      case 'netsocial_connect':
        return mockNsConnect(args);
      case 'netsocial_disconnect':
        return mockNsDisconnect();
      case 'netsocial_set_profile':
        return mockNsSetProfile(args);
      case 'netsocial_roam':
        return mockNsRoam(args);
      case 'netsocial_match':
        return mockNsMatch(args);
      case 'netsocial_city_souls':
        return mockNsCitySouls(args);
      case 'netsocial_like':
        return mockNsLike(args);
      case 'netsocial_report':
        return mockNsReport(args);
      case 'netsocial_block':
        return mockNsBlock(args);
      case 'netsocial_send':
        return mockNsSend(args);
      case 'netsocial_history':
        return mockNsHistory(args);
      case 'netsocial_poll':
        return mockNsPoll();
      case 'netsocial_topics':
        return mockNsTopics();
      case 'netsocial_events':
        return mockNsEvents();
      case 'netsocial_cache_clear':
        return mockNsCacheClear();
      default:
        return Promise.reject(new Error('[mock] unknown command: ' + cmd));
    }
  } catch (e) {
    // D9：mock 分发异常保护——未捕获异常（如递归/序列化）不刷屏，统一 reject
    console.error('[qibai] mockInvoke error:', cmd, e && e.message);
    return Promise.reject(e);
  }
  }

  // dev 面板手动触发入口（仅浏览器模式；广播让所有窗口一致响应）
  global.__qibaiMock = {
    emitReminder(type, message) {
      const actions = { sit: 'hands_on_hips', water: 'stretch', eye: 'wave' };
      broadcast('qibai://reminder', {
        type,
        message: message || { sit: '坐久啦，站起来活动一下吧～', water: '该喝水啦，给身体补补水～', eye: '眼睛累了吧，望向远处20秒～' }[type],
        action: actions[type] || 'nod',
      });
    },
    emitSkillApproval() {
      broadcast('qibai://skill-approval', {
        request_id: 'mock-' + Date.now(),
        skill: 'launch_app',
        description: '元气小小想帮你打开「记事本」应用，这是一个外部操作，是否允许？',
      });
    },
    emitLlmStatus(state, message) {
      broadcast('qibai://llm-status', { state, message });
    },
    // 亲密度冒烟入口（§8.3：event/seed/reset）
    intimacy: {
      // event(kind, detail?) —— 默认打到 active persona；detail.persona_id 可覆盖
      event(kind, detail) {
        detail = detail || {};
        const pid = detail.persona_id || activePersonaFromStore();
        return doIntimacyEvent(pid, kind, detail);
      },
      // seed(level, personaId?) —— 直接把某角色 points 置到该等级阈值起点，
      // 便于冒烟演示升级动画（跨档时按升级广播 intimacy-updated）
      seed(level, personaId) {
        const lv = Math.max(1, Math.min(10, parseInt(level, 10) || 1));
        const pid = personaId || activePersonaFromStore();
        const obj = loadMockIntimacy();
        const entry = obj.entries[pid];
        if (!entry) return Promise.reject(new Error('[mock] unknown persona_id: ' + pid));
        touchIntimacyDaily(entry);
        const oldLevel = intimacyLevelOf(entry.points);
        entry.points = INTIMACY_LEVELS[lv - 1].min;
        entry.level = lv;
        saveMockIntimacy(obj);
        const leveledUp = lv > oldLevel;
        if (leveledUp) {
          const social = loadMockSocial();
          pushEvent(social, {
            type: 'intimacy_level_up', a: pid, b: pid,
            detail: {
              new_level: lv,
              level_name: INTIMACY_LEVELS[lv - 1].name,
              unlocked: newUnlockedGates(oldLevel, lv).map((g) => g.gate),
            },
          });
          saveMockSocial(social);
        }
        broadcast('qibai://intimacy-updated', {
          persona_id: pid, points: entry.points, level: lv,
          level_name: INTIMACY_LEVELS[lv - 1].name,
          leveled_up: leveledUp,
          unlocked: leveledUp ? newUnlockedGates(oldLevel, lv) : [],
          achievements_granted: [],
        });
        return Promise.resolve(buildIntimacyState(entry));
      },
      // reset() —— 清空 intimacy 存储，下次加载重新播种迁移
      reset() {
        localStorage.removeItem(LS_KEYS.intimacy);
      },
    },
    // 迭代①·全屏守护 dev 入口（冒烟/演示用）
    guard: {
      // trigger：走完整 invoke（含 enabled/冷却校验；失败 reject 结构化错误）
      trigger() { return invoke('guard_trigger', {}); },
      // skip：用户主动跳过（记冷却）
      skip() { return invoke('guard_skip', { reason: 'user' }); },
      // end：自然结束路径（auto）
      end() { return invoke('guard_skip', { reason: 'auto' }); },
      // reset：冷却清零（连跑冒烟用）
      reset() { localStorage.removeItem(LS_KEYS.guard); },
    },
    // 迭代①·代码检查 dev 入口（冒烟/演示用）
    codecheck: {
      badDemo() { return invoke('check_code', { path: 'qibai-core/tests/fixtures/bad.js' }); },
      healthyDemo() { return invoke('check_code', { path: 'qibai-core/tests/fixtures/healthy.js' }); },
      snippetBad() { return invoke('check_code', { snippet: BUILTIN_BAD_SNIPPET, language: 'javascript' }); },
      // chatDemo：模拟对话触发——llm-status thinking→done + 脚本化回复摘要广播
      chatDemo() {
        broadcast('qibai://llm-status', { state: 'thinking', message: '正在思考…' });
        return new Promise((resolve) => {
          setTimeout(() => {
            broadcast('qibai://llm-status', { state: 'done' });
            // 通知控制台切到聊天页并渲染一条脚本化回复摘要
            broadcast('qibai://codecheck-chat-demo', {});
            resolve({ reply: '已通过对话触发代码检查（演示）', language: 'javascript' });
          }, 400);
        });
      },
    },
    // 迭代②·语音 dev 入口（冒烟/演示用；真机同样有效，走标准 invoke）
    voice: {
      // speak：朗读一句（kind 可传 chat / reminder）
      speak(text, kind) {
        return invoke('tts_speak', {
          text: text || '今天的风很温柔，你要好好照顾自己呀～',
          kind: kind || 'chat',
        });
      },
      // status：TTS + STT 状态一起取（harness 状态行展示用）
      status() {
        return Promise.all([invoke('tts_engine_status'), invoke('stt_status')]);
      },
    },
    // 迭代③·保险箱 dev 入口（冒烟/演示用）
    vault: {
      status() { return invoke('vault_status'); },
      // clipboard：读 mock 降级剪贴板（navigator.clipboard 不可用时记录于此）
      clipboard() { return global.__qibaiMockClipboard || ''; },
      // reset：清空信封 + 钥匙串主密钥 + 运行时，回到 unset 态
      reset() {
        localStorage.removeItem(LS_KEYS.vault);
        try { sessionStorage.removeItem(VAULT_KEYRING_KEY); } catch (e) { /* ignore */ }
        global.__qibaiMockClipboard = '';
        vaultRt.initialized = false;
        vaultRt.state = 'unset';
        vaultRt.mode = 'none';
        vaultRt.key = null;
        vaultRt.entries = [];
        vaultRt.failedAttempts = 0;
        vaultRt.cooldownUntil = 0;
        vaultRt.lockedReason = null;
      },
    },
  };

  // body 标记
  if (!isTauri) {
    document.addEventListener('DOMContentLoaded', () => {
      document.body.classList.add('browser-mode');
    });
  } else {
    document.addEventListener('DOMContentLoaded', () => {
      document.body.classList.add('tauri-mode');
    });
  }

  // 窗口身份（WINDOW_ARCH.md §6/§7）：Tauri 下 invoke window_identity，
  // mock 下返回 __qibaiWindowLabel（pet.html 内联脚本设置）或 'browser'。
  let cachedWindowLabel = null;
  async function getWindowLabel() {
    if (cachedWindowLabel) return cachedWindowLabel;
    if (isTauri) {
      try {
        const r = await invoke('window_identity');
        cachedWindowLabel = (r && r.label) || null;
        return cachedWindowLabel;
      } catch (e) {
        console.warn('[qibai] window_identity failed', e);
        return null;
      }
    }
    cachedWindowLabel = mockWindowLabel;
    return cachedWindowLabel;
  }

  // 全局裸 invoke/listen：全项目历史代码直接调用 invoke(...)，
  // 必须挂到 window 上才能工作（否则 ReferenceError → 一切后端通信静默失败）。
  global.invoke = invoke;
  global.listen = listen;

  global.QibaiBridge = {
    invoke,
    listen,
    isTauri,
    getWindowLabel,
    // v20：窗口重建/卸载前统一注销全部事件监听
    disposeAllListeners,
    // mock 场景下的本页窗口 label（同步；Tauri 下为 null，请用 getWindowLabel）
    windowLabel: isTauri ? null : mockWindowLabel,
    // 跨窗口总线广播（浏览器 mock 专用；Tauri 下为 null）
    broadcast: isTauri ? null : broadcast,
  };
})(window);
