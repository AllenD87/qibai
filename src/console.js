/* ============================================================
 * 元气小小 Qibai — 控制台窗逻辑（console.js）
 * 合并迁移：settings.js（设置/角色管理/关系/事件）
 *           + app.js 控制台侧逻辑（聊天 UI / 猜拳对局 /
 *             事件摘要 / 技能审批弹窗）
 * 一切跨窗口交互经 QibaiBridge.invoke / listen，不做窗间直连。
 * ============================================================ */

(function () {
  'use strict';

  const B = window.QibaiBridge;
  const { invoke, listen, isTauri: bridgeTauri } = B;
  // v16 兜底：旧版 tauri-bridge.js 可能不导出 isTauri——一旦缺失，tauri-mode 类/窗口分支全是死代码。
  const isTauri = !!bridgeTauri ||
    !!(window.__TAURI__ && window.__TAURI__.invoke) ||
    !!window.__TAURI_INTERNALS__;
  const $ = (id) => document.getElementById(id);

  // body 模式类（styles.css 依此决定透明底 / 演示底）
  document.body.classList.add(isTauri ? 'tauri-mode' : 'browser-mode');

  /* ---------- 常量 ---------- */
  const PERSONA_NAMES = {
    gudian: '古典佳人', dushi: '都市女性', wenrou: '温柔', huopo: '活泼', reqing: '热情时尚',
  };
  const PERSONA_IMG = {
    gudian: 'assets/personas/gudian/character.png',
    dushi: 'assets/personas/dushi/character.png',
    wenrou: 'assets/personas/wenrou/character.png',
    huopo: 'assets/personas/huopo/character.png',
    reqing: 'assets/personas/reqing/character.png',
  };
  const TYPE_NAME = { greet: '串门', pk: '猜拳', level_up: '升级', summon: '召唤', dismiss: '送走' };
  const MOVE_LABEL = { rock: '石头', scissors: '剪刀', paper: '布' };
  const MOVE_SVG = {
    rock: '<svg viewBox="0 0 24 24"><rect x="6" y="7" width="12" height="12" rx="5" fill="#D17397"/></svg>',
    scissors: '<svg viewBox="0 0 24 24"><rect x="7" y="5" width="4" height="12" rx="2" fill="#9B7FB8"/><rect x="13" y="5" width="4" height="12" rx="2" fill="#9B7FB8"/></svg>',
    paper: '<svg viewBox="0 0 24 24"><rect x="5" y="4" width="14" height="16" rx="4" fill="#D17397"/></svg>',
  };
  const REMINDER_TAGS = { sit: '久坐提醒', water: '喝水提醒', eye: '护眼提醒' };

  const wait = (ms) => new Promise((r) => setTimeout(r, ms));

  /* ============================================================
   * toast 轻提示
   * ============================================================ */
  function toast(text, ms = 2600) {
    const box = $('toastBox');
    if (!box) return;
    const t = document.createElement('div');
    t.className = 'toast';
    t.textContent = text;
    box.appendChild(t);
    requestAnimationFrame(() => t.classList.add('show'));
    setTimeout(() => {
      t.classList.remove('show');
      setTimeout(() => t.remove(), 350);
    }, ms);
  }

  /* ============================================================
   * Tab 切换（单页多视图；hash 同步 #tab=xxx）
   * ============================================================ */
  // v45.14：补 tasks——控制台 11 个页签此前只有 10 个在允许表里，
  // 点「任务」会被 switchTab 兜底成 chat，任务页永远进不去。
  const TABS = ['chat', 'roles', 'intimacy', 'events', 'settings', 'vault', 'social', 'pk', 'plugins', 'codecheck', 'tasks'];
  let currentTab = 'chat';

  function switchTab(name) {
    if (!TABS.includes(name)) name = 'chat';
    currentTab = name;
    document.querySelectorAll('#tabBar .ctab').forEach((b) =>
      b.classList.toggle('active', b.dataset.tab === name));
    document.querySelectorAll('.console-pane').forEach((p) =>
      p.classList.toggle('active', p.id === 'pane-' + name));
    // hash 同步（保留既有 fire 参数）
    const kv = parseHashKv();
    kv.tab = name;
    history.replaceState(null, '', '#' + encodeKv(kv));
    onTabShown(name);
    // v20：无合成器 X11 下切 tab 时旧 pane 像素不自动清除，新 pane 叠在旧像素上
    // =「选项卡/内容互相覆盖」。与 pet_repaint 的 PET_REFLOW_JS 同一手段：强制
    // webview reflow（display:none → offsetHeight → 恢复），纯前端无 Rust 依赖。
    // 放 rAF：等 onTabShown 异步内容填充完再清，避免把刚渲染的 pane 又抖掉。
    requestAnimationFrame(() => {
      const b = document.body;
      const d = b.style.display;
      b.style.display = 'none';
      void b.offsetHeight;
      b.style.display = d;
    });
  }

  function parseHashKv() {
    const h = location.hash.replace(/^#/, '');
    if (!h) return {};
    return h.split('&').reduce((o, p) => {
      const i = p.indexOf('=');
      if (i < 0) { o[p] = true; return o; }
      o[p.slice(0, i)] = decodeURIComponent(p.slice(i + 1));
      return o;
    }, {});
  }
  function encodeKv(kv) {
    return Object.keys(kv)
      .filter((k) => kv[k] !== undefined && kv[k] !== false)
      .map((k) => `${k}=${encodeURIComponent(kv[k] === true ? '' : kv[k])}`)
      .join('&');
  }

  function onTabShown(name) {
    stopDiag(); // v18.8-fix8：离开设置页停止语音诊断轮询
    if (name === 'chat') loadConversation();
    else if (name === 'roles') loadRoles();
    else if (name === 'intimacy') loadIntimacyTab();
    else if (name === 'events') renderEvents();
    else if (name === 'settings') { refreshVoiceStatus(); refreshWakeStatus(); refreshHfQuick(); startDiag(); initMcpPanel(); } // 迭代②+免提：进入设置刷新语音/STT/唤醒状态；v45.12：MCP 面板
    else if (name === 'pk') loadPkTab();
    else if (name === 'vault') loadVaultTab();
    else if (name === 'social') loadSocialTab();
    else if (name === 'plugins') loadPluginsTab(); // 技能与专家（契约 §5）
    else if (name === 'tasks') { if (window.__tasksRefresh) window.__tasksRefresh(); } // v45.14：任务页在切进来时拉一次列表
  }

  $('tabBar').addEventListener('click', (e) => {
    const b = e.target.closest('.ctab');
    if (b) switchTab(b.dataset.tab);
  });

  /* F6：设置项子 Tab 分组切换（声音/行为/通用），按 data-setgroup 显隐卡片 */
  function switchSetSubTab(name) {
    const pane = $('pane-settings');
    if (!pane) return;
    ['voice', 'behavior', 'general'].forEach((g) => pane.classList.remove('set-group-' + g));
    pane.classList.add('set-group-' + name);
    const bar = document.getElementById('setSubBar');
    if (bar) bar.querySelectorAll('.set-subtab').forEach((b) =>
      b.classList.toggle('active', b.dataset.setstab === name));
  }
  (function bindSetSubBar() {
    const bar = document.getElementById('setSubBar');
    if (!bar) return;
    bar.addEventListener('click', (e) => {
      const b = e.target.closest('.set-subtab');
      if (b) switchSetSubTab(b.dataset.setstab);
    });
  })();

  /* ============================================================
   * 关闭控制台（Tauri window.close() / mock toast）
   * ============================================================ */
  $('btnCloseConsole').addEventListener('click', async () => {
    if (isTauri) {
      // v21.11.8：统一走后端 console_close（此前 close_console 未注册 → 静默失败）；失败再兜底原生 window.close()
      try {
        await invoke('console_close');
        return;
      } catch (e) {
        console.warn(e);
        try { window.close(); } catch (_) { /* 忽略 */ }
      }
    } else {
      toast('控制台已关闭（浏览器演示模式）');
    }
  });

  /* ============================================================
   * 聊天 UI（迁移自 app.js：appendBubble / loadConversation / doSend）
   * ============================================================ */
  // v26：聊天消息轻量 Markdown 白名单渲染（先整体转义 → 再白名单替换，杜绝注入）。
  // 支持：代码块 ```…```、行内 `code`、**粗体**、行首列表项、普通换行。
  // 代码块先占位摘出，完成行内替换后再还原，避免块内 ** / ` 被二次替换。
  function renderChatMarkdown(raw) {
    const esc = escapeHtml(raw);
    const blocks = [];
    const withPh = esc.replace(/```([^\n`]*)\n([\s\S]*?)```/g, (_, lang, body) => {
      const langCls = escapeHtml(lang.trim()).replace(/[^a-zA-Z0-9_-]/g, '');
      const cls = langCls ? ' class="chat-code chat-code-' + langCls + '"' : ' class="chat-code"';
      // 去掉首尾多余空行，内部换行转 <br>，块内内容不做行内样式替换
      const safeBody = body.replace(/^\n|\n$/g, '').replace(/\n/g, '<br>');
      const ph = '__QIBBLOCK' + blocks.length + '__';
      blocks.push('<pre' + cls + '>' + safeBody + '</pre>');
      return ph;
    });
    // 行内代码：`x` → <code>
    const withCode = withPh.replace(/`([^`\n]+)`/g, '<code>$1</code>');
    // 粗体：**x** → <strong>
    const withBold = withCode.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
    // 行首列表项：- x → • x
    const withList = withBold.replace(/^\s*[-•]\s+/gm, '<span class="chat-bullet">• </span>');
    // 剩余换行 → <br>
    const withBr = withList.replace(/\n/g, '<br>');
    // 还原代码块
    return withBr.replace(/__QIBBLOCK(\d+)__/g, (_, i) => blocks[Number(i)] || '');
  }

  // v45.14：执行留痕接线——core 每轮都会 emit qibai://executions，
  // 但此前前端没有任何监听，「本轮执行（N）」折叠块永远挂不出来。
  // 事件可能早于气泡到达（send_message 还没返回），也可能晚于气泡，
  // 所以两条路都接：先到 → 存 pendingExec，气泡落地时挂；后到 → 挂到最后一条 assistant 气泡。
  let pendingExec = [];
  let lastAssistantRow = null;
  let lastSendAt = 0;

  function attachExecutionFold(row, items) {
    if (!row || !Array.isArray(items) || !items.length) return;
    if (row.querySelector('details.bubble-exec')) return; // 同一气泡只挂一次
    const det = document.createElement('details');
    det.className = 'bubble-exec';
    det.style.cssText = 'margin-top:4px;font-size:11px;color:#6b7280;';
    det.innerHTML = `<summary style="cursor:pointer;">本轮执行（${items.length}）</summary>`;
    items.forEach((ex) => {
      ex = ex || {};
      const line = document.createElement('div');
      line.style.cssText = 'padding:2px 0;border-top:1px dashed #e5e7eb;';
      const args = (ex.args && typeof ex.args === 'object') ? JSON.stringify(ex.args) : String(ex.args || '');
      const short = args.length > 40 ? args.slice(0, 40) + '…' : args;
      const ok = ex.ok !== false;
      line.innerHTML = `<span style="color:${ok ? '#16a34a' : '#b91c1c'};">${ok ? '✓' : '✗'}</span> ` +
        `<b>${ex.tool || '?'}</b> <span style="color:#374151;">${short}</span>`;
      if (ex.source) {
        const sb = document.createElement('span');
        sb.style.cssText = 'margin-left:6px;padding:0 4px;background:#eef2ff;color:#3730a3;border-radius:3px;font-size:10px;';
        sb.textContent = ex.source;
        line.appendChild(sb);
      }
      if (args.length > 40) {
        const full = document.createElement('div');
        full.style.cssText = 'color:#6b7280;white-space:pre-wrap;word-break:break-all;';
        full.textContent = args;
        line.appendChild(full);
      }
      if (!ok && ex.error) {
        const err = document.createElement('div');
        err.style.cssText = 'color:#b91c1c;white-space:pre-wrap;word-break:break-all;';
        err.textContent = '错误：' + ex.error;
        line.appendChild(err);
      }
      det.appendChild(line);
    });
    row.appendChild(det);
  }

  function appendBubble(role, content, doScroll = true, meta = null) {
    const scroll = $('chatScroll');
    const row = document.createElement('div');
    row.className = 'bubble-row ' + (role === 'user' ? 'me' : role === 'assistant' ? 'role' : 'sys');
    // v18.8-fix28g：角色名标签（我 / 元气小小·角色名 / 系统）——一眼分清谁说
    const nm = document.createElement('div');
    nm.className = 'bubble-name';
    nm.textContent = role === 'user' ? '我'
      : role === 'assistant' ? (personaNameMap[activePersonaId] || '元气小小')
      : '系统';
    // v45.11：未执行却声称完成的黄色小标
    if (role === 'assistant' && meta && meta.markers && meta.markers.false_completion) {
      const warn = document.createElement('span');
      warn.className = 'bubble-warn';
      warn.textContent = '⚠ 未执行 · 声称已完成';
      warn.style.cssText = 'margin-left:8px;padding:1px 6px;background:#fef3c7;color:#92400e;border-radius:3px;font-size:11px;';
      nm.appendChild(warn);
    }
    const b = document.createElement('div');
    b.className = 'bubble';
    // v26：assistant 消息走轻量 Markdown 白名单渲染（先转义后替换）；
    // 用户消息/系统消息保持纯文本。渲染结果来自已转义文本的白名单替换，无注入面。
    if (role === 'assistant') {
      b.innerHTML = renderChatMarkdown(content);
    } else {
      b.textContent = content;
    }
    row.appendChild(nm);
    row.appendChild(b);

    // v45.11 / v45.14：本轮执行留痕——优先用调用方传入的 meta，
    // 否则消费 core 广播时缓存的 pendingExec（详见 attachExecutionFold）。
    if (role === 'assistant') {
      lastAssistantRow = row;
      let items = [];
      if (meta && Array.isArray(meta.executions) && meta.executions.length) {
        items = meta.executions;
      } else if (pendingExec.length) {
        items = pendingExec.splice(0, pendingExec.length);
      }
      if (items.length) attachExecutionFold(row, items);
    }

    // v45.8：assistant 消息带来源 URL → 在气泡下方渲染可点来源列表
    if (role === 'assistant') {
      const urlRe = /(https?:\/\/[^\s)\]>"'<]+)/g;
      const urls = (content.match(urlRe) || []).slice(0, 5);
      if (urls.length) {
        const src = document.createElement('div');
        src.className = 'bubble-sources';
        src.style.cssText = 'margin-top:4px;font-size:11px;color:#888;line-height:1.6;';
        urls.forEach((u, i) => {
          const domain = u.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
          const a = document.createElement('a');
          a.href = u;
          a.target = '_blank';
          a.rel = 'noopener';
          a.textContent = `${i + 1}. ${domain}`;
          a.style.cssText = 'color:#6aa;margin-right:8px;';
          src.appendChild(a);
        });
        row.appendChild(src);
      }
    }

    scroll.appendChild(row);
    // v20：批量渲染时仅最后一次滚底（调用方 render 完统一滚动），避免 N 次 scrollTop 抖动
    if (doScroll) scroll.scrollTop = scroll.scrollHeight;
  }

  // v20：聊天记录刷新——epoch 门防旧快照覆盖 + 150ms trailing 合并风暴
  let convSeq = 0;
  let convDebounceTimer = null;

  function requestConversationReload() {
    if (convDebounceTimer) clearTimeout(convDebounceTimer);
    convDebounceTimer = setTimeout(() => {
      convDebounceTimer = null;
      loadConversation().catch((e) => console.warn(e));
    }, 150);
  }

  // v20-fix35：变更检测签名（条数+末条ts）——无变化跳过重建，支撑 1s 轮询兜底且不闪烁
  let lastConvSig = '';
  let lastRenderedCount = 0; // v11：增量渲染游标（1386 条全量重建会卡 UI）
  async function loadConversation() {
    let list = [];
    try {
      list = await invoke('get_conversation') || [];
    } catch (e) { console.warn(e); return; }
    const tail = list.length ? (list[list.length - 1].ts || '') : '';
    const sig = list.length + ':' + tail;
    if (sig === lastConvSig) return;   // 数据无变化：不重建 DOM（轮询安静、不闪烁）
    lastConvSig = sig;
    // v11：尾部增量——条数变多且已渲染过 → 只 append 新增消息，不全量重建。
    // 根治长对话（千条级）每次变更全量 innerHTML 重建导致的卡顿与「不实时」观感。
    if (lastRenderedCount > 0 && list.length > lastRenderedCount) {
      const added = list.slice(lastRenderedCount);
      lastRenderedCount = list.length;
      added.forEach((m) => {
        try { appendBubble(m.role, m.content, false); }
        catch (e) { console.warn('[qibai] 增量渲染单条失败', e, m); }
      });
      $('chatScroll').scrollTop = $('chatScroll').scrollHeight;
      return;
    }
    const my = ++convSeq;            // epoch：在途旧请求返回时丢弃，防旧快照 innerHTML='' 覆盖新渲染
    if (my !== convSeq) return;
    console.log('[qibai] loadConversation:', list.length, '条');
    lastRenderedCount = list.length;
    $('chatScroll').innerHTML = '';
    // v18.8-fix28h：单条渲染异常不中断整屏（try/catch 逐条防护）；
    // v20：doScroll=false 批量渲染后统一滚底一次
    list.forEach((m) => {
      try { appendBubble(m.role, m.content, false); }
      catch (e) { console.warn('[qibai] 渲染单条失败', e, m); }
    });
    $('chatScroll').scrollTop = $('chatScroll').scrollHeight;
  }

  // v20-fix35：聊天记录实时性兜底——即使 conversation-updated 事件链丢失，
  // 窗口可见时 1s 轮询也能拉到新消息（变更检测保证有变化才重建、不闪烁）；
  // 彻底解决「聊天内容要重新打开/重启才更新」。
  setInterval(() => {
    try {
      // v22-fix36：去掉 visibilityState 跳过——控制台窗最小化/隐藏时也要轮询，
      // 否则还原前聊天记录一直不更新（还原瞬间才补刷）。invoke 读内存很轻，可接受。
      if ($('chatScroll')) loadConversation().catch((e) => console.warn(e));
    } catch (e) { /* 窗口未就绪忽略 */ }
  }, 1000);
  // v22-fix36：窗口从隐藏还原瞬间立即刷新一次（不等下个轮询周期）
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
      loadConversation().catch((e) => console.warn(e));
    }
  });

  // v18.8-fix28e：invoke 前端超时兜底——Rust 端 LLM 循环异常时不再无限空等
  function invokeTimeout(promise, ms, label) {
    let timer;
    const guard = new Promise((_, rej) => {
      timer = setTimeout(() => rej(new Error((label || '请求') + '响应超时（' + ms / 1000 + 's）')), ms);
    });
    return Promise.race([promise, guard]).finally(() => clearTimeout(timer));
  }

  async function doSend() {
    const input = $('chatInput');
    await doSendWithText(input.value);
  }
  // v20：发送中锁——防连按 Enter/双击发送按钮导致多轮并发 send_message 与事件风暴
  let sendBusy = false;
  async function doSendWithText(rawText) {
    const text = (rawText || '').trim();
    if (!text || sendBusy) return;
    sendBusy = true;
    $('sendBtn').disabled = true;
    $('chatInput').disabled = true;
    appendBubble('user', text);
    // v45.14：新一轮开始，清掉上轮残留的执行留痕缓存与气泡指针，
    // 防止 core 抢先 emit 的 executions 被挂到上一轮气泡上。
    pendingExec = [];
    lastAssistantRow = null;
    lastSendAt = Date.now();
    $('chatInput').value = '';
    try {
      // v21：Tauri 权威语音路由 voice_ui_dispatch——命中（切 tab/操作类指令）→ toast 提示并 return，不走 LLM。
      if (isTauri) {
        try {
          const vd = await invoke('voice_ui_dispatch', { text });
          if (vd && vd.matched) {
            toast(vd.reply || '已执行');
            return;
          }
        } catch (e) { /* 命令未就绪 → 走正常聊天 */ }
      }
      const res = await invokeTimeout(invoke('send_message', { text }), 130000, '大模型');
      const replyText = (res && res.reply) || '（…）';
      appendBubble('assistant', replyText);
      // v20：乐观追加后请求一次合并刷新（150ms debounce）——全量渲染以落库为准，
      // 消除「事件先到、乐观回复后补」造成的尾部重复气泡
      requestConversationReload();
      // 亲密度联动（§8.1）：发送并收到回复后上报 chat；命中早安正则补报 greeting 任务
      reportChatIntimacy(text);
      // 迭代③：secret 分支——明文 reply 已当次渲染；前端自动调 vault_copy 完成剪贴板写入
      //（载荷不含明文；mock 层写剪贴板并调度 clear_after_secs 清除）
      if (res && res.secret === true && res.secret_entry_id) {
        try {
          const r = await invoke('vault_copy', { id: res.secret_entry_id });
          toast(`已复制到剪贴板，${(r && r.clear_after_secs) || 20} 秒后自动清除`);
        } catch (e) {
          console.warn(e);
          toast('自动复制失败，可到「保险箱」页手动复制');
        }
      }
      // v18.8-fix28f：朗读兜底已移到 Rust 端（非流式直接播放、流式 tts-chunk 逐段）——
      // 前端仅对 secret 拦截场景朗读 speak_text（脱敏文本），避免与 Rust 播放重复。
      if (res && res.secret === true) {
        speakReply((res && res.speak_text) || replyText);
      }
      // 迭代④·open_social 技能跳转（§7.6）：回复带 skill:'open_social' → 切社交 tab 并切视图
      if (res && res.skill === 'open_social') {
        switchTab('social');
        setNsView(res.view === 'city' ? 'city' : 'planet');
      }
    } catch (e) {
      console.warn(e);
      // v20：错误反馈分级（不展示后端原始错误原文）
      const msg = String((e && e.message) || e || '');
      let friendly;
      if (/超时|timeout/i.test(msg)) {
        friendly = '（大模型响应超时（130s）：请缩短问题后重试）';
      } else if (/overdue balance|insufficient balance|arrears|欠费|余额不足/i.test(msg)) {
        friendly = '（大模型服务欠费：请到火山方舟充值，或在设置中配置备用 Key）';
      } else if (/401|403|unauthorized|invalid.*key|api.?key|鉴权|密钥/i.test(msg)) {
        friendly = '（API Key 无效或未配置：请到「设置」检查模型配置）';
      } else if (/429|rate.?limit|频率|too many/i.test(msg)) {
        friendly = '（请求过于频繁，请稍后再试）';
      } else {
        friendly = '（连接失败，请稍后再试）';
      }
      appendBubble('sys', friendly);
    } finally {
      sendBusy = false;
      $('sendBtn').disabled = false;
      $('chatInput').disabled = false;
    }
  }
  $('sendBtn').addEventListener('click', doSend);
  $('chatInput').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') doSend();
  });

  // v45.7：清空聊天记录（模态确认：只清聊天 / 连记忆一起清）
  (function setupClearChat() {
    const btn = document.getElementById('btnClearChat');
    const modal = document.getElementById('confirmClearChat');
    if (!btn || !modal) return;
    const btnOnly = document.getElementById('btnClearOnly');
    const btnAll = document.getElementById('btnClearAll');
    const btnCancel = document.getElementById('btnClearCancel');
    const errEl = document.getElementById('clearChatErr');

    function open() { errEl.hidden = true; modal.removeAttribute("hidden"); }
    function close() { modal.setAttribute("hidden", ""); errEl.hidden = true; }

    btn.addEventListener('click', open);
    btnCancel.addEventListener('click', close);
    modal.querySelector('.clear-modal-mask').addEventListener('click', close);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !modal.hidden) close();
    });

    async function doClear(scope) {
      errEl.hidden = true;
      try {
        const r = await invoke('clear_conversation_scoped', { scope });
        const backup = r.backup ? `，备份在 ${r.backup}` : '';
        const mem = r.kept_memory ? '（已保留角色记忆）' : '（角色记忆已一并清空）';
        toast(`已清空 ${r.cleared} 条${backup}${mem}`);
        close();
        requestConversationReload();
      } catch (e) {
        errEl.textContent = `清空失败：${(e && e.message) || e}`;
        errEl.hidden = false;
      }
    }
    btnOnly.addEventListener('click', () => doClear('chat'));
    btnAll.addEventListener('click', () => doClear('all'));
  })();

  /* ============================================================
   * 设置（迁移自 settings.js：开关渲染 / collect / apply / save）
   * ============================================================ */
  let currentSettings = null;

  function bindSwitch(el, initial, onChange) {
    el.classList.toggle('on', !!initial);
    el.dataset.on = initial ? 'true' : 'false';
    el.onclick = () => {
      const next = el.dataset.on !== 'true';
      el.dataset.on = next ? 'true' : 'false';
      el.classList.toggle('on', next);
      if (onChange) onChange(next);
    };
  }

  // v45.10：collect 改为"以 currentSettings 为底 + 只覆盖 UI 上能编辑的字段"。
  // 之前从零拼对象，导致 audio_output / search_engine / context_* / task_* 等未在
  // collect 里列的字段被后端 serde(default) 静默重置成默认值（用户选的 USB 音箱被切回）。
  function collect() {
    const num = (id, dflt) => {
      const v = parseInt($(id).value, 10);
      return isNaN(v) ? dflt : v;
    };
    // 从最近一次 get_settings 的完整对象深拷贝做底
    const out = JSON.parse(JSON.stringify(currentSettings || {}));
    out.llm = out.llm || {};
    out.reminders = out.reminders || {};
    out.reminders.enabled = out.reminders.enabled || {};
    out.ui = out.ui || {};
    out.skills = out.skills || {};
    out.guard = out.guard || {};
    out.voice = out.voice || {};
    // 以下只覆盖 UI 上有控件的字段；UI 没列的字段原样保留
    out.llm.provider = $('setProvider').value;
    out.llm.base_url = $('setBaseUrl').value.trim();
    out.llm.model = $('setModel').value.trim();
    out.llm.api_key = $('setApiKey').value;
    out.reminders.sit_min = num('setSitMin', 60);
    out.reminders.water_min = num('setWaterMin', 40);
    out.reminders.eye_min = num('setEyeMin', 45);
    out.reminders.enabled.sit = $('swSitEnabled').dataset.on === 'true';
    out.reminders.enabled.water = $('swWaterEnabled').dataset.on === 'true';
    out.reminders.enabled.eye = $('swEyeEnabled').dataset.on === 'true';
    out.ui.age_confirmed = $('swAgeConfirmed').dataset.on === 'true';
    out.ui.pure_mode = $('swPureMode').dataset.on === 'true';
    out.ui.snap_to_edge = $('swSnapEdge').dataset.on === 'true';
    if ($('swShowVoiceTag')) out.ui.show_voice_tag = $('swShowVoiceTag').dataset.on === 'true';
    out.skills.total_enabled = $('swSkillsTotal').dataset.on === 'true';
    out.skills.launch_app = $('swSkillLaunch').dataset.on === 'true';
    out.skills.open_folder = $('swSkillFolder').dataset.on === 'true';
    out.skills.set_reminder = $('swSkillReminder').dataset.on === 'true';
    // skills.check_code 无独立开关，原样保留（不覆盖）
    out.guard.enabled = $('swGuardEnabled').dataset.on === 'true';
    out.guard.sit_min = num('setGuardSitMin', 120);
    out.guard.mode = $('setGuardMode').value;
    out.guard.end_action = $('setGuardEnd').value;
    out.guard.countdown_secs = num('setGuardCountdown', 120);
    out.voice.enabled = $('swVoiceEnabled').dataset.on === 'true';
    out.voice.engine = $('selVoiceEngine').value;
    out.voice.volume = Math.min(100, Math.max(0, num('setVoiceVolume', 80)));
    out.voice.rate = Math.min(2.0, Math.max(0.5, parseFloat($('setVoiceRate').value) || 1.0));
    out.voice.scope = $('selVoiceScope').value;
    out.voice.cloud_provider = $('setCloudProvider').value;
    out.voice.cloud_voice = $('setCloudVoice').value.trim();
    out.voice.cloud_api_key = $('setCloudKey').value;
    out.voice.stt_enabled = $('swSttEnabled').dataset.on === 'true';
    out.voice.stt_provider = $('setSttProvider').value;
    out.voice.stt_api_key = $('setSttKey').value;
    out.voice.stt_input_device = $('setSttDevice').value;
    out.voice.wake_enabled = $('swWakeEnabled').dataset.on === 'true';
    out.voice.wake_word = $('setWakeWord').value.trim() || '小小';
    // v45.10：扬声器下拉——显式赋值 "default" 或具体 sink name
    if ($('audioOutputSelect')) out.audio_output = $('audioOutputSelect').value || 'default';
    // v45.10：搜索引擎下拉
    if ($('searchEngineSelect')) out.search_engine = $('searchEngineSelect').value || 'auto';
    // v45.10：上下文 / 任务设置（夹取）
    if ($('ctxBudgetTokens')) out.context_budget_tokens = Math.min(64000, Math.max(1000, num('ctxBudgetTokens', 8000)));
    if ($('ctxRecentRounds')) out.context_recent_turns = Math.min(200, Math.max(5, num('ctxRecentRounds', 10)));
    if ($('taskMaxSteps')) out.task_max_steps = Math.min(200, Math.max(1, num('taskMaxSteps', 20)));
    if ($('taskMaxMinutes')) out.task_max_minutes = Math.min(180, Math.max(1, num('taskMaxMinutes', 30)));
    // v45.11：知识库参数
    if ($('kbTopK')) out.kb_top_k = Math.min(20, Math.max(1, num('kbTopK', 6)));
    if ($('kbChunkChars')) out.kb_chunk_chars = Math.min(800, Math.max(500, num('kbChunkChars', 700)));
    if ($('kbMaxChunks')) out.kb_max_chunks = Math.min(100000, Math.max(1000, num('kbMaxChunks', 20000)));
    // P2：语义检索开关 / 融合权重（0..1，默认 0.45）
    if ($('kbEmbedEnabled')) out.kb_embed_enabled = !!$('kbEmbedEnabled').checked;
    if ($('kbHybridWeight')) out.kb_hybrid_weight = Math.min(1, Math.max(0, num('kbHybridWeight', 0.45)));
    return out;
  }

  function apply(s) {
    loadAudioDevices(); // v18.8 设备枚举
    currentSettings = s;
    $('setProvider').value = (s.llm && s.llm.provider) || 'doubao';
    $('setBaseUrl').value = (s.llm && s.llm.base_url) || '';
    $('setModel').value = (s.llm && s.llm.model) || '';
    $('setApiKey').value = (s.llm && s.llm.api_key) || '';
    // v45.11/P2：知识库参数回填
    if ($('kbTopK')) $('kbTopK').value = s.kb_top_k || 6;
    if ($('kbChunkChars')) $('kbChunkChars').value = s.kb_chunk_chars || 700;
    if ($('kbMaxChunks')) $('kbMaxChunks').value = s.kb_max_chunks || 20000;
    if ($('kbEmbedEnabled')) $('kbEmbedEnabled').checked = s.kb_embed_enabled !== false;
    if ($('kbHybridWeight')) $('kbHybridWeight').value = (s.kb_hybrid_weight === 0 ? 0 : s.kb_hybrid_weight) || 0.45;

    $('setSitMin').value = (s.reminders && s.reminders.sit_min) || 60;
    $('setWaterMin').value = (s.reminders && s.reminders.water_min) || 40;
    $('setEyeMin').value = (s.reminders && s.reminders.eye_min) || 45;

    const en = (s.reminders && s.reminders.enabled) || {};
    bindSwitch($('swSitEnabled'), !!en.sit);
    bindSwitch($('swWaterEnabled'), !!en.water);
    bindSwitch($('swEyeEnabled'), !!en.eye);

    bindSwitch($('swAgeConfirmed'), !!(s.ui && s.ui.age_confirmed));
    bindSwitch($('swPureMode'), !!(s.ui && s.ui.pure_mode));
    // v20：桌宠贴边吸附——默认开启（旧 settings 无此字段视为 true）
    bindSwitch($('swSnapEdge'), !(s.ui && s.ui.snap_to_edge === false));
    // 二次修复①：音色调试 chip——默认关闭
    if ($('swShowVoiceTag')) bindSwitch($('swShowVoiceTag'), !!(s.ui && s.ui.show_voice_tag));

    // 技能开关：即时写后端
    const sk = s.skills || {};
    bindSwitch($('swSkillsTotal'), !!sk.total_enabled,
      (v) => invoke('set_skill_enabled', { skill: 'total', enabled: v }).catch(console.warn));
    bindSwitch($('swSkillLaunch'), !!sk.launch_app,
      (v) => invoke('set_skill_enabled', { skill: 'launch_app', enabled: v }).catch(console.warn));
    bindSwitch($('swSkillFolder'), !!sk.open_folder,
      (v) => invoke('set_skill_enabled', { skill: 'open_folder', enabled: v }).catch(console.warn));
    bindSwitch($('swSkillReminder'), !!sk.set_reminder,
      (v) => invoke('set_skill_enabled', { skill: 'set_reminder', enabled: v }).catch(console.warn));

    // 迭代①·全屏守护回填
    const g = s.guard || {};
    bindSwitch($('swGuardEnabled'), !!g.enabled);
    $('setGuardSitMin').value = g.sit_min || 120;
    $('setGuardMode').value = g.mode || 'strong';
    $('setGuardEnd').value = g.end_action || 'countdown';
    $('setGuardCountdown').value = g.countdown_secs || 120;

    // 迭代②·语音回填（整表）
    const v = s.voice || {};
    bindSwitch($('swVoiceEnabled'), v.enabled !== false);
    $('selVoiceEngine').value = v.engine || 'auto';
    $('setVoiceVolume').value = Math.min(100, Math.max(0, Number(v.volume) || 80));
    $('setVoiceVolumeVal').textContent = $('setVoiceVolume').value;
    $('setVoiceRate').value = Math.min(2.0, Math.max(0.5, Number(v.rate) || 1.0));
    $('setVoiceRateVal').textContent = Number($('setVoiceRate').value).toFixed(2);
    $('selVoiceScope').value = v.scope || 'all';
    $('setCloudProvider').value = v.cloud_provider || 'doubao';
    $('setCloudVoice').value = v.cloud_voice || '';
    $('setCloudKey').value = v.cloud_api_key || '';
    bindSwitch($('swSttEnabled'), !!v.stt_enabled);
    $('setSttProvider').value = v.stt_provider || 'doubao';
    $('setSttKey').value = v.stt_api_key || '';
    $('setSttDevice').value = v.stt_input_device || '';
    bindSwitch($('swWakeEnabled'), !!v.wake_enabled);
    $('setWakeWord').value = v.wake_word || '小小';
  }

  /* ---------- 迭代②·语音滑块：拖动实时更新数值标签 ---------- */
  $('setVoiceVolume').addEventListener('input', () => {
    $('setVoiceVolumeVal').textContent = $('setVoiceVolume').value;
  });
  $('setVoiceRate').addEventListener('input', () => {
    $('setVoiceRateVal').textContent = Number($('setVoiceRate').value).toFixed(2);
  });

  /* ---------- 迭代②·语音状态（tts_engine_status / stt_status → hint） ---------- */
  const TTS_ENGINE_LABELS = { local: '本地系统 TTS', cloud: '云端 TTS', mute: '无可用引擎（静音）' };

  async function refreshWakeStatus() {
    try {
      const st = await invoke('get_listen_state');
      const label = st && st.enabled ? `● ${st.state === 'active' ? '聆听中' : '待唤醒'}（唤醒词：${st.wake_word || '元气小小'}）` : '○ 已关闭';
      $('wakeStatusHint').textContent = label;
    } catch (e) {
      $('wakeStatusHint').textContent = '免提聆听状态暂不可用';
    }
  }

  // v20：STT 引擎自愈卡片（契约：voice_diag.stt_engine，见语音官冻结稿）
  function renderSttEngineCard(se) {
    const card = $('sttEngineCard');
    if (!card) return;
    if (!se) { card.hidden = true; return; }
    card.hidden = false;
    const ok = se.probe_ok === true;
    card.classList.toggle('stt-bad', !ok);
    card.classList.toggle('stt-ok', ok);
    $('sttEngineDot').textContent = ok ? '● 就绪' : '● 缺失';
    $('sttEngineDot').style.color = ok ? '#4caf50' : '#d9534f';
    // 三行资源状态（textContent 赋值，路径为本地字符串，不走 innerHTML 注入）
    const setRow = (trId, found, path) => {
      const cell = document.getElementById(trId);
      if (!cell) return;
      cell.textContent = found ? ('✓ ' + (path || '已就位')) : '✗ 未找到';
    };
    setRow('sttRowServer', se.server_found, se.server_path);
    setRow('sttRowCli', se.cli_found, se.cli_path);
    setRow('sttRowModel', se.model_found, se.model_path);
    // 缺失指引区
    const guide = $('sttEngineGuide');
    if (ok) {
      guide.hidden = true;
    } else {
      guide.hidden = false;
      $('sttEngineReason').textContent = se.probe_reason || 'STT 引擎未就绪';
      $('sttEngineCwd').textContent = se.cwd ? `期望放置根目录：${se.cwd}` : '';
      const linksBox = $('sttEngineLinks');
      linksBox.textContent = '';
      (se.download_links || []).forEach((lk) => {
        const a = document.createElement('a');
        a.href = lk.url;
        a.target = '_blank';
        a.rel = 'noopener noreferrer';
        a.textContent = lk.label || lk.url;
        a.style.display = 'block';
        a.style.margin = '3px 0';
        linksBox.appendChild(a);
      });
    }
  }

  // v18.8-fix8：语音诊断轮询（每 1s）
  let diagTimer = null;
  function startDiag() {
    stopDiag();
    diagTimer = setInterval(async () => {
      try {
        const d = await invoke('voice_diag');
        const st = ['关闭', '待唤醒', '聆听中'][d.state] || d.state;
        $('diagDevice').textContent = d.device || '—';
        $('diagState').textContent = st;
        $('diagRms').textContent = d.rms > 0 ? d.rms.toFixed(4) : '—';
        $('diagVad').textContent = d.vad_count;
        $('diagText').textContent = d.last_text || '—';
        const dot = $('diagStateDot');
        dot.style.background = d.state === 2 ? '#4caf50' : (d.state === 1 ? '#ffc107' : '#aaa');
        // v20：朗读播放状态行 + STT 引擎卡片（缺字段时优雅隐藏）
        const tp = $('diagTtsPlayback');
        if (tp && d.tts_playback) {
          const tp2 = d.tts_playback;
          const stMap = { idle: '空闲', start: '播放中', end: '播放完成', fail: '播放失败' };
          tp.textContent = (stMap[tp2.state] || tp2.state)
            + ` · ${tp2.engine || '—'}`
            + (tp2.duration_ms ? ` · ${(tp2.duration_ms / 1000).toFixed(1)}s` : '')
            + (tp2.last_error ? ` · ${tp2.last_error}` : '');
        }
        renderSttEngineCard(d.stt_engine);
      } catch (e) { /* 面板未就绪忽略 */ }
    }, 1000);
  }
  function stopDiag() { if (diagTimer) { clearInterval(diagTimer); diagTimer = null; } }

  // v20：播放测试音按钮（tts_test_tone，定位输出链路是否出声）
  $('btnTtsTest').addEventListener('click', async () => {
    const res = $('ttsTestResult');
    res.textContent = '播放中…';
    try {
      const r = await invoke('tts_test_tone');
      // 契约：{played:true, duration_ms} | {played:false, error}
      res.textContent = (r && r.played)
        ? `已播放 ${(r.duration_ms / 1000).toFixed(1)}s——听到蜂鸣即输出正常`
        : ('失败：' + String((r && r.error) || '未知'));
    } catch (e) {
      res.textContent = '失败：' + String((e && e.message) || e || '').slice(0, 80);
    }
  });

  async function refreshVoiceStatus() {
    try {
      const st = await invoke('tts_engine_status');
      const r = (st && st.resolved) || { engine: 'mute', reason: '' };
      const label = TTS_ENGINE_LABELS[r.engine] || r.engine;
      // 迭代⑤：状态栏同步展示当前主角色的音色标签
      let tagPart = '';
      if (st && st.voice_tag) {
        const pidName = PERSONA_NAMES[st.active_persona] || st.active_persona || '';
        tagPart = `；当前角色「${pidName}」音色：${st.voice_tag}`;
      }
      let extra = '';
      if (r.engine === 'mute' || (r.reason && /未安装|未检测|缺少/.test(r.reason || ''))) {
        extra = '；要发声请安装本地语音引擎：Ubuntu 执行 <code>sudo apt install espeak-ng</code>（或配置云端 TTS Key）';
      }
      $('voiceStatusHint').innerHTML =
        `当前将使用：${label}` + (r.reason && r.reason !== label ? `（${r.reason}）` : '') + tagPart + extra;
    } catch (e) {
      $('voiceStatusHint').textContent = '语音状态暂不可用（稍后重试）';
    }
    try {
      const stt = await invoke('stt_status');
      if (!stt) return;
      const box = $('sttStatusHint');
      box.textContent = `STT：${stt.reason}`;
      // 优雅降级：设备硬不可用 → 禁用配置入口并标记 degraded；缺 key 时仅提示（输入保持可用）
      const deviceOff = stt.enabled && !stt.device_available;
      box.classList.toggle('degraded', deviceOff);
      $('setSttProvider').disabled = deviceOff;
      $('setSttKey').disabled = deviceOff;
    } catch (e) {
      $('sttStatusHint').textContent = 'STT 状态暂不可用（稍后重试）';
    }
    // 迭代⑤：角色音色卡片随状态刷新（cloud_ready 影响每卡降级提示）
    refreshPersonaVoiceCards();
  }

  /* ============================================================
   * v45.12 · MCP 服务器面板
   * 数据全部来自 invoke('mcp_list')（Rust registry 真实状态），
   * 不写死"已连接"：ready=绿 / unavailable=红+原因 / starting=黄 / disabled=灰。
   * ============================================================ */
  const MCP_STATE_META = {
    ready: { label: '就绪', color: '#3aa655' },
    unavailable: { label: '不可用', color: '#d9534f' },
    starting: { label: '启动中', color: '#d9a441' },
    disabled: { label: '已禁用', color: '#999' },
  };

  function mcpStateDot(state) {
    const meta = MCP_STATE_META[state] || MCP_STATE_META.disabled;
    const dot = document.createElement('span');
    dot.className = 'mcp-state-dot';
    dot.style.cssText =
      `display:inline-block;width:8px;height:8px;border-radius:50%;` +
      `background:${meta.color};margin-right:6px;`;
    dot.title = meta.label;
    return dot;
  }

  function mcpRowEl(s) {
    const row = document.createElement('div');
    row.style.cssText = 'border:1px solid #e5d9c8;border-radius:8px;padding:8px 10px;background:#fdf9f2;';

    const top = document.createElement('div');
    top.style.cssText = 'display:flex;align-items:center;gap:8px;flex-wrap:wrap;';
    top.appendChild(mcpStateDot(s.state));

    const name = document.createElement('b');
    name.textContent = s.name || '(未命名)';
    top.appendChild(name);

    const cmd = document.createElement('span');
    cmd.style.cssText = 'font-family:monospace;font-size:11px;color:#7a6a5a;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
    cmd.textContent = s.command || '';
    top.appendChild(cmd);

    // 工具数
    const toolCount = document.createElement('span');
    toolCount.style.cssText = 'font-size:12px;color:#5a4a3a;';
    toolCount.textContent = `${Number(s.tools_count || 0)} 个工具`;
    top.appendChild(toolCount);

    // enabled 开关
    const enableChk = document.createElement('input');
    enableChk.type = 'checkbox';
    enableChk.checked = !!s.enabled;
    enableChk.title = '启用/停用';
    enableChk.addEventListener('change', async () => {
      try {
        await invoke('mcp_set_enabled', { name: s.name, enabled: enableChk.checked });
        toast(enableChk.checked ? `已启用 ${s.name}` : `已停用 ${s.name}`);
      } catch (e) {
        toast('设置失败：' + e);
        enableChk.checked = !enableChk.checked;
      }
      loadMcpList();
    });
    top.appendChild(enableChk);

    // trust 下拉
    const trustSel = document.createElement('select');
    trustSel.style.cssText = 'font-size:12px;';
    const optR = document.createElement('option');
    optR.value = 'readonly'; optR.textContent = 'readonly';
    const optC = document.createElement('option');
    optC.value = 'confirm'; optC.textContent = 'confirm';
    trustSel.appendChild(optR); trustSel.appendChild(optC);
    trustSel.value = s.trust === 'readonly' ? 'readonly' : 'confirm';
    trustSel.title = '信任档位';
    trustSel.addEventListener('change', async () => {
      try {
        await invoke('mcp_upsert', {
          cfg: {
            name: s.name,
            transport: s.transport || 'stdio',
            command: s.command,
            args: Array.isArray(s.args) ? s.args : [],
            env: s.env && typeof s.env === 'object' ? s.env : {},
            enabled: !!s.enabled,
            trust: trustSel.value,
          },
        });
        toast(`已更新 ${s.name} 的信任档位`);
      } catch (e) {
        toast('更新失败：' + e);
      }
      loadMcpList();
    });
    top.appendChild(trustSel);

    // 重连
    const btnRestart = document.createElement('button');
    btnRestart.className = 'mini-btn';
    btnRestart.textContent = '重连';
    btnRestart.addEventListener('click', async () => {
      btnRestart.disabled = true;
      try {
        await invoke('mcp_restart', { name: s.name });
        toast(`已重连 ${s.name}`);
      } catch (e) {
        toast('重连失败：' + e);
      }
      btnRestart.disabled = false;
      loadMcpList();
    });
    top.appendChild(btnRestart);

    // 测试调用
    const btnTest = document.createElement('button');
    btnTest.className = 'mini-btn';
    btnTest.textContent = '测试调用';
    top.appendChild(btnTest);

    // 删除
    const btnDel = document.createElement('button');
    btnDel.className = 'mini-btn';
    btnDel.textContent = '删除';
    btnDel.style.color = '#d9534f';
    btnDel.addEventListener('click', async () => {
      if (!window.confirm(`确定删除 MCP server「${s.name}」？`)) return;
      try {
        await invoke('mcp_remove', { name: s.name });
        toast(`已删除 ${s.name}`);
      } catch (e) {
        toast('删除失败：' + e);
      }
      loadMcpList();
    });
    top.appendChild(btnDel);

    row.appendChild(top);

    // 状态原因（unavailable 时展示）
    if (s.state === 'unavailable' && s.state_detail) {
      const reason = document.createElement('div');
      reason.style.cssText = 'font-size:11px;color:#d9534f;margin-top:4px;';
      reason.textContent = '原因：' + s.state_detail;
      row.appendChild(reason);
    }

    // 测试调用展开区（默认收起）
    const testBox = document.createElement('div');
    testBox.style.cssText = 'display:none;margin-top:8px;border-top:1px dashed #e5d9c8;padding-top:6px;';
    const toolSel = document.createElement('select');
    toolSel.style.cssText = 'font-size:12px;margin-right:6px;';
    const tools = Array.isArray(s.tools) ? s.tools : [];
    if (tools.length === 0) {
      const o = document.createElement('option');
      o.value = ''; o.textContent = s.state === 'ready' ? '（暂无工具）' : '（服务器未就绪）';
      toolSel.appendChild(o);
    } else {
      for (const t of tools) {
        const o = document.createElement('option');
        o.value = t.name; o.textContent = t.name;
        toolSel.appendChild(o);
      }
    }
    testBox.appendChild(toolSel);

    const argsInput = document.createElement('input');
    argsInput.type = 'text';
    argsInput.placeholder = '参数 JSON，如 {"path":"."}';
    argsInput.style.cssText = 'width:220px;font-size:12px;margin-right:6px;';
    testBox.appendChild(argsInput);

    const btnRun = document.createElement('button');
    btnRun.className = 'mini-btn primary';
    btnRun.textContent = '运行';
    const testResult = document.createElement('div');
    testResult.style.cssText = 'font-family:monospace;font-size:11px;margin-top:6px;white-space:pre-wrap;max-height:160px;overflow:auto;';
    btnRun.addEventListener('click', async () => {
      const toolName = toolSel.value;
      if (!toolName) { toast('没有可调用的工具'); return; }
      let args = {};
      const raw = argsInput.value.trim();
      if (raw) {
        try { args = JSON.parse(raw); } catch (e) { toast('参数不是合法 JSON'); return; }
      }
      btnRun.disabled = true;
      testResult.textContent = '调用中…';
      try {
        const r = await invoke('mcp_test_call', { server: s.name, tool: toolName, args });
        const ok = r && r.ok !== false;
        testResult.style.color = ok ? '#3aa655' : '#d9534f';
        testResult.textContent = (ok ? '✔ ' : '✘ ') + (r && r.content != null ? String(r.content) : JSON.stringify(r));
      } catch (e) {
        testResult.style.color = '#d9534f';
        testResult.textContent = '✘ ' + String(e);
      }
      btnRun.disabled = false;
    });
    testBox.appendChild(btnRun);
    testBox.appendChild(testResult);

    btnTest.addEventListener('click', () => {
      testBox.style.display = testBox.style.display === 'none' ? 'block' : 'none';
    });
    row.appendChild(testBox);

    return row;
  }

  async function loadMcpList() {
    const box = $('mcpList');
    if (!box) return;
    let list = [];
    try {
      list = (await invoke('mcp_list')) || [];
    } catch (e) {
      list = [];
    }
    const empty = $('mcpEmpty');
    if (empty) empty.style.display = list.length ? 'none' : '';
    box.innerHTML = '';
    for (const s of list) box.appendChild(mcpRowEl(s));
  }

  async function initMcpPanel() {
    const btn = $('btnMcpAdd');
    if (btn && !btn.dataset.bound) {
      btn.dataset.bound = '1';
      btn.addEventListener('click', async () => {
        const name = ($('mcpNewName').value || '').trim();
        const command = ($('mcpNewCommand').value || '').trim();
        if (!name || !command) { toast('名称与启动命令必填'); return; }
        const argsRaw = ($('mcpNewArgs').value || '').trim();
        try {
          await invoke('mcp_upsert', {
            cfg: {
              name,
              transport: 'stdio',
              command,
              args: argsRaw ? argsRaw.split(/\s+/).filter(Boolean) : [],
              env: {},
              enabled: true,
              trust: $('mcpNewTrust').value || 'confirm',
            },
          });
          toast(`已添加 ${name}`);
          $('mcpNewName').value = '';
          $('mcpNewCommand').value = '';
          $('mcpNewArgs').value = '';
        } catch (e) {
          toast('添加失败：' + e);
        }
        loadMcpList();
      });
    }
    loadMcpList();
  }

  /* ============================================================
   * 迭代⑤ · 角色专属音色卡片（设置 tab 语音卡内）
   * get_personas 回填 voice_profile；保存 → invoke('set_persona_voice')；
   * cloud_ready==false 且该卡 cloud_speaker 非空 → 卡内降级提示。
   * ============================================================ */
  const PERSONA_VOICE_TONES = {
    gudian: '温婉', dushi: '知性', wenrou: '恬静', huopo: '元气', reqing: '热情',
  };
  function vpRateDesc(r) { return r < 0.9 ? '慢速' : (r < 1.1 ? '中速' : '轻快'); }
  function vpPitchDesc(p) {
    if (p < -0.5) return '微低';
    if (p <= 0.5) return '中性';
    if (p <= 1.5) return '明亮';
    return '偏高';
  }
  function vpVoiceTag(id, prof) {
    const rd = vpRateDesc(Number(prof.rate) || 1);
    const pd = vpPitchDesc(Number(prof.pitch) || 0);
    if (prof.mode === 'custom') return '自定义 · ' + rd + ' · ' + pd;
    return (PERSONA_VOICE_TONES[id] || '元气小小') + ' · ' + rd + ' · ' + pd;
  }

  function buildPersonaVoiceCard(p, cloudReady) {
    const prof = p.voice_profile || { mode: 'auto', pitch: 0, rate: 1, engine_voice_id: '', cloud_speaker: '' };
    const card = document.createElement('div');
    card.className = 'vp-card';
    card.dataset.pid = p.persona_id;
    card.innerHTML =
      '<div class="vp-head">' +
      '  <img src="' + (PERSONA_IMG[p.persona_id] || 'assets/personas/gudian/character.png') + '" alt="">' +
      '  <div>' +
      '    <div class="vp-name"></div>' +
      '    <span class="vp-chip"></span>' +
      '  </div>' +
      '</div>' +
      '<div class="vp-row">' +
      '  <div class="vp-field">' +
      '    <label>音色模式</label>' +
      '    <select class="vp-mode">' +
      '      <option value="auto">按性格自动</option>' +
      '      <option value="custom">自定义</option>' +
      '    </select>' +
      '  </div>' +
      '  <div class="vp-field">' +
      '    <label>音高 <span class="slider-val vp-pitch-val"></span></label>' +
      '    <input class="vp-pitch" type="range" min="-10" max="10" step="0.5">' +
      '  </div>' +
      '</div>' +
      '<div class="vp-field">' +
      '  <label>语速 <span class="slider-val vp-rate-val"></span></label>' +
      '  <input class="vp-rate" type="range" min="0.5" max="2.0" step="0.1">' +
      '</div>' +
      '<div class="vp-field vp-text-row">' +
      '  <label>本地引擎音色</label>' +
      '  <input class="vp-engine" type="text" placeholder="本地引擎音色，留空用默认">' +
      '</div>' +
      '<div class="vp-field vp-text-row">' +
      '  <label>云端 speaker id</label>' +
      '  <input class="vp-speaker" type="text" placeholder="云端 speaker id，留空按性格自动">' +
      '</div>' +
      '<div style="display:flex;align-items:center;gap:10px;">' +
      '  <button class="vp-save" type="button">保存</button>' +
      '  <span class="vp-status"></span>' +
      '</div>';

    card.querySelector('.vp-name').textContent = p.display_name || PERSONA_NAMES[p.persona_id] || p.persona_id;
    const modeSel = card.querySelector('.vp-mode');
    const pitchRange = card.querySelector('.vp-pitch');
    const pitchVal = card.querySelector('.vp-pitch-val');
    const rateRange = card.querySelector('.vp-rate');
    const rateVal = card.querySelector('.vp-rate-val');
    const engineInp = card.querySelector('.vp-engine');
    const speakerInp = card.querySelector('.vp-speaker');
    const chip = card.querySelector('.vp-chip');
    const status = card.querySelector('.vp-status');
    const saveBtn = card.querySelector('.vp-save');

    // 种子值（auto 模式切回时复位用）
    const seed = {
      mode: prof.mode === 'custom' ? 'custom' : 'auto',
      pitch: Number(prof.pitch) || 0,
      rate: Number(prof.rate) || 1,
      engine_voice_id: prof.engine_voice_id || '',
      cloud_speaker: prof.cloud_speaker || '',
    };

    function applyStatus() {
      // 生效 cloud_speaker：auto 用种子默认；custom 用当前输入
      const spk = modeSel.value === 'custom' ? speakerInp.value.trim() : seed.cloud_speaker;
      if (!cloudReady && spk) {
        status.textContent = `云端音色「${spk}」未配置 key，当前用本地音色`;
        status.classList.add('degraded');
        status.classList.remove('ok');
      } else {
        status.textContent = '';
        status.classList.remove('degraded');
      }
    }
    function refreshChip() {
      // auto 模式：生效音色取性格默认表（seed），不随滑块（滑块 step 0.1 会量化 1.05→1.1）；
      // custom 模式：取当前滑块值。保证 5 个冻结标签（如 reqing 1.05→中速）准确。
      const isCustom = modeSel.value === 'custom';
      const rate = isCustom ? Number(rateRange.value) : seed.rate;
      const pitch = isCustom ? Number(pitchRange.value) : seed.pitch;
      chip.textContent = vpVoiceTag(p.persona_id, { mode: modeSel.value, pitch, rate });
      applyStatus();
    }
    function fillFromSeed() {
      pitchRange.value = seed.pitch;
      rateRange.value = seed.rate;
      engineInp.value = seed.engine_voice_id;
      speakerInp.value = seed.cloud_speaker;
      pitchVal.textContent = Number(seed.pitch).toFixed(1);
      rateVal.textContent = Number(seed.rate).toFixed(2);
    }

    // 回填
    modeSel.value = seed.mode;
    fillFromSeed();
    refreshChip();

    modeSel.addEventListener('change', () => {
      if (modeSel.value === 'auto') fillFromSeed(); // 切回自动：复位性格默认
      refreshChip();
    });
    pitchRange.addEventListener('input', () => {
      pitchVal.textContent = Number(pitchRange.value).toFixed(1);
      refreshChip();
    });
    rateRange.addEventListener('input', () => {
      rateVal.textContent = Number(rateRange.value).toFixed(2);
      refreshChip();
    });
    speakerInp.addEventListener('input', refreshChip);

    saveBtn.addEventListener('click', async () => {
      const voice_profile = {
        mode: modeSel.value,
        pitch: Number(pitchRange.value) || 0,
        rate: Number(rateRange.value) || 1,
        engine_voice_id: engineInp.value.trim(),
        cloud_speaker: speakerInp.value.trim(),
      };
      saveBtn.disabled = true;
      try {
        await invoke('set_persona_voice', { persona_id: p.persona_id, voice_profile });
        // 同步种子，便于切回 auto 时复位到刚保存的值
        seed.mode = voice_profile.mode;
        seed.pitch = voice_profile.pitch;
        seed.rate = voice_profile.rate;
        seed.engine_voice_id = voice_profile.engine_voice_id;
        seed.cloud_speaker = voice_profile.cloud_speaker;
        status.textContent = '已保存 ✓';
        status.classList.add('ok');
        status.classList.remove('degraded');
        refreshChip();
        refreshVoiceStatus(); // 状态栏音色标签随主角色/音色变化刷新
      } catch (e) {
        console.warn(e);
        status.textContent = '保存失败';
        status.classList.add('degraded');
      } finally {
        saveBtn.disabled = false;
      }
    });

    return card;
  }

  async function refreshPersonaVoiceCards() {
    const box = $('personaVoiceCards');
    if (!box) return;
    let personas = [];
    let cloudReady = false;
    try {
      const data = await invoke('get_personas');
      personas = (data && data.personas) || [];
      if (data && data.active_persona) activePersonaId = data.active_persona;
    } catch (e) {
      box.innerHTML = '<div class="hint">角色音色加载失败</div>';
      return;
    }
    try {
      const st = await invoke('tts_engine_status');
      cloudReady = !!(st && st.cloud_ready);
    } catch (e) { cloudReady = false; }

    box.innerHTML = '';
    personas.forEach((p) => box.appendChild(buildPersonaVoiceCard(p, cloudReady)));
  }

  /* ---------- 迭代②·对话回复朗读指示器（§4.2） ---------- */
  let ttsIndicatorTimer = null;
  function showTtsIndicator(r) {
    const ind = $('ttsIndicator');
    $('ttsIndicatorText').textContent =
      '正在朗读（' + (r.engine === 'cloud' ? '云端 TTS' : '本地系统 TTS') + '）';
    ind.hidden = false;
    if (ttsIndicatorTimer) clearTimeout(ttsIndicatorTimer);
    ttsIndicatorTimer = setTimeout(() => { ind.hidden = true; }, 2600);
  }
  async function speakReply(replyText) {
    if (!replyText) return;
    // v18.8-fix27：LLM 流式已逐段朗读（3s 内）→ 整段不再重复朗读。
    if (Date.now() - (window.__lastStreamTs || 0) < 3000) return;
    try {
      // 迭代⑤：对话回复朗读带当前主角色 persona_id（缺省后端/mock 回落主角色）
      const r = await invoke('tts_speak', {
        text: replyText,
        kind: 'chat',
        persona_id: activePersonaId || undefined,
      });
      if (r && r.engine && r.engine !== 'mute') {
        showTtsIndicator(r);
      } else if (r && r.engine === 'mute' && !window.__qibaiMuteWarned) {
        // v18.8-fix23：无可用 TTS 引擎时给一次性指引，不再完全静默
        window.__qibaiMuteWarned = true;
        toast('听不到朗读？' + ((r && r.reason) || '语音朗读不可用') +
          '。装 edge-tts（pip install edge-tts）或下载 piper 模型即可发声');
      }
    } catch (e) { /* 朗读失败静默，不打扰 */ }
  }

  /* v18.8：枚举系统音频设备 → 麦克风下拉 + 输出设备展示 */
  async function loadAudioDevices() {
    try {
      const r = await invoke('list_audio_devices');
      const sel = $('setSttDevice');
      const cur = sel.value || '';
      sel.innerHTML = '';
      const autoOpt = document.createElement('option');
      autoOpt.value = '';
      autoOpt.textContent = '自动选择（推荐）';
      sel.appendChild(autoOpt);
      (r.inputs || []).forEach((d) => {
        const o = document.createElement('option');
        o.value = d.name;
        o.textContent = d.name + (d.score >= 0 ? '（推荐' + (d.score > 0 ? '，检测到麦克风特征' : '）') : '（可能无麦克风）');
        sel.appendChild(o);
      });
      sel.value = cur;

      // v45.8：扬声器下拉（输出设备）——用独立 IPC list_output_devices / current_audio_output
      const outSel = $('audioOutputSelect');
      if (outSel) {
        let curOut = 'default';
        try { curOut = (await invoke('current_audio_output')) || 'default'; } catch (e) {}
        let outDevs = [];
        try { outDevs = (await invoke('list_output_devices')) || []; } catch (e) {}
        outSel.innerHTML = '';
        const defOpt = document.createElement('option');
        defOpt.value = 'default';
        defOpt.textContent = '系统默认';
        outSel.appendChild(defOpt);
        outDevs.forEach((d) => {
          // 兼容旧结构：纯字符串 或 {name, description, is_default, source}
          const name = typeof d === 'string' ? d : (d.name || '');
          if (!name) return;
          const desc = (typeof d === 'object' && d.description) ? d.description : name;
          const isDef = typeof d === 'object' && d.is_default;
          const o = document.createElement('option');
          o.value = name;
          // v45.11：同时显示 description 与 sink Name，方便用户 `pactl list sink-inputs` 对账
          o.textContent = (isDef ? '✓ ' : '') + desc + '  [' + name + ']';
          o.title = name; // 悬停看原始设备名
          outSel.appendChild(o);
        });
        outSel.value = curOut;
        // 保存的设备不在列表里 → 回落 default 并提示一次
        if (outSel.value !== curOut) {
          outSel.value = 'default';
          $('audioOutputs').textContent = '原选定设备已断开，已回落到系统默认';
        } else {
          $('audioOutputs').textContent = '';
        }
        outSel.onchange = async () => {
          try {
            await invoke('set_audio_output', { device: outSel.value });
            toast('扬声器已切换：' + (outSel.value === 'default' ? '系统默认' : outSel.value));
          } catch (e) {
            toast('切换失败：' + ((e && e.message) || e));
          }
        };
      }
      // v45.8：搜索引擎下拉
      const seSel = $('searchEngineSelect');
      if (seSel) {
        let curSe = 'auto';
        try { curSe = (await invoke('current_search_engine')) || 'auto'; } catch (e) {}
        seSel.value = ['auto','baidu','360','sogou'].includes(curSe) ? curSe : 'auto';
        seSel.onchange = async () => {
          try {
            await invoke('set_search_engine', { engine: seSel.value });
            toast('搜索引擎已切换：' + seSel.options[seSel.selectedIndex].text);
          } catch (e) {
            toast('切换失败：' + ((e && e.message) || e));
          }
        };
      }
    } catch (e) {
      $('audioOutputs').textContent = '枚举失败（设备不可用或权限受限）';
    }
  }
  $('sttDeviceRefresh').addEventListener('click', loadAudioDevices);

  async function saveSettings() {
    const tip = $('savedTip');
    try {
      const collected = collect();
      // v45.10：用 JSON Merge Patch 语义，只传 UI 上能编辑的字段；后端递归合并、保留未出现字段
      const saved = await invoke('save_settings_partial', { patch: collected });
      // 用后端返回的完整 settings 刷新 currentSettings，保证"保存后重开还是填的"
      if (saved && typeof saved === 'object') currentSettings = saved;
      else currentSettings = collected;
      tip.textContent = '已保存 ✓';
      tip.classList.add('show');
      setTimeout(() => tip.classList.remove('show'), 1800);
      refreshSkillsGate(); // 保存后按最新亲密度重新校验助理技能门控
      refreshVoiceStatus(); // 迭代②：保存后刷新引擎/STT 状态提示
      // v14：免提聆听即时启停
      try {
        await invoke('set_wake_enabled', { enabled: !!(collected.voice && collected.voice.wake_enabled) });
      } catch (e) { console.warn('[wake] 免提聆听切换失败', e); }
      refreshWakeStatus();
    } catch (e) {
      console.warn(e);
      tip.textContent = '保存失败';
      tip.classList.add('show');
    }
  }
  $('btnSave').addEventListener('click', saveSettings);

  /* ============================================================
   * 关于与更新（迭代⑥）：invoke('check_update') → 结构化状态渲染
   * 冻结形状：{status:'available'|'latest'|'error', current_version,
   *           latest_version?, error?}。手动检查只回报状态，不自动安装；
   * 启动时的静默下载安装在 Rust 侧 spawn_updater_check 完成。
   * ============================================================ */
  async function runCheckUpdate() {
    const btn = $('btnCheckUpdate'), tip = $('updStatus');
    btn.disabled = true;
    btn.textContent = '检查中…';
    try {
      const r = await invoke('check_update');
      if (r.status === 'available') {
        tip.textContent = `发现新版本 ${r.latest_version || ''}（当前 ${r.current_version}）；启动时将自动下载安装。`;
      } else if (r.status === 'latest') {
        tip.textContent = `已是最新版本（${r.current_version}）。`;
      } else {
        tip.textContent = `检查失败：${r.error || '未知错误'}`;
      }
    } catch (e) {
      console.warn(e);
      tip.textContent = '检查失败：' + ((e && e.message) || '未知错误');
    }
    btn.disabled = false;
    btn.textContent = '检查更新';
  }
  $('btnCheckUpdate').addEventListener('click', runCheckUpdate);

  /* ============================================================
   * 代码检查 tab（迭代①）：invoke('check_code') → CodeReport 渲染
   * 冻结形状：{source,path,files_checked,language,summary,issues[],llm_note}
   * ============================================================ */
  const CC_SEVERITY_BADGE = { error: '错误', warning: '警告', hint: '提示' };
  const CC_CATEGORY_LABEL = { syntax: '语法', logic: '逻辑', security: '安全', style: '风格', unknown: '其他' };

  function escapeHtml(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, (c) =>
      ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  }

  function setCcBusy(busy) {
    $('ccRun').disabled = busy;
    $('ccRun').textContent = busy ? '检查中…' : '运行检查';
  }

  function renderCcReport(report, errorMsg) {
    const empty = $('ccEmpty'), summary = $('ccSummary'), meta = $('ccMeta'), box = $('ccIssues');
    box.innerHTML = '';
    if (!report) {
      empty.hidden = false;
      empty.className = 'cc-empty' + (errorMsg ? ' cc-error' : '');
      empty.textContent = errorMsg || '尚未运行检查。';
      summary.hidden = true;
      meta.hidden = true;
      return;
    }
    empty.hidden = true;
    summary.hidden = false;
    meta.hidden = false;
    const issues = Array.isArray(report.issues) ? report.issues : [];
    summary.textContent = report.summary;
    summary.className = 'cc-summary ' + (issues.length ? 'has-issues' : 'clean');
    meta.textContent = `来源：${report.source === 'path' ? '路径 ' + report.path : '代码片段'}　` +
      `语言：${report.language}　文件数：${report.files_checked}`;
    if (!issues.length) {
      box.innerHTML = '<div class="cc-clean-note">代码状态良好，可以放心提交。</div>';
      return;
    }
    issues.forEach((it) => {
      const card = document.createElement('div');
      card.className = 'cc-issue sev-' + (it.severity || 'hint');
      const badge = `<span class="cc-badge ${it.severity || 'hint'}">${CC_SEVERITY_BADGE[it.severity] || it.severity}</span>`;
      const cat = `<span class="cc-cat">${CC_CATEGORY_LABEL[it.category] || it.category}</span>`;
      const loc = it.location ? `<div class="cc-loc">${escapeHtml(it.location)}</div>` : '';
      const sug = it.suggestion ? `<div class="cc-suggestion">修复建议：${escapeHtml(it.suggestion)}</div>` : '';
      card.innerHTML =
        `<div class="cc-issue-head">${badge}${cat}</div>` +
        `<div class="cc-msg">${escapeHtml(it.message)}</div>${loc}${sug}`;
      box.appendChild(card);
    });
  }

  async function runCodecheck(overrides) {
    overrides = overrides || {};
    const path = (overrides.path !== undefined ? overrides.path : $('ccPath').value).trim();
    const snippet = overrides.snippet !== undefined ? overrides.snippet : $('ccSnippet').value;
    let language = overrides.language !== undefined ? overrides.language : $('ccLang').value;
    if (language === 'auto') language = '';
    const args = {};
    if (path && snippet) args.path = path; // 路径优先
    else if (path) args.path = path;
    else if (snippet) args.snippet = snippet;
    else { renderCcReport(null, '请输入文件路径，或粘贴代码片段后再运行。'); return; }
    if (language) args.language = language;
    setCcBusy(true);
    try {
      const report = await invoke('check_code', args);
      renderCcReport(report);
    } catch (e) {
      console.warn(e);
      renderCcReport(null, (e && e.message) || '检查失败');
    }
    setCcBusy(false);
  }

  $('ccRun').addEventListener('click', () => runCodecheck());
  $('ccRerun').addEventListener('click', () => runCodecheck());

  // 总线驱动：smoke harness / dev 面板的演示按钮经广播落到本窗渲染
  // v18.8-fix27：LLM 流式输出 → 逐段朗读（第一句生成完即开口）。
  listen('qibai://tts-chunk', (event) => {
    const p = event.payload || {};
    const seg = p && p.text ? String(p.text) : '';
    // v20：前端接收断点诊断（只打段长/标志，绝不打印文本）——对照 Rust emit 字符数日志
    console.log('[qibai][fe] tts-chunk recv: len=' + seg.length + ', rust_played=' + (p.rust_played === true));
    if (seg && seg.trim()) {
      window.__lastStreamTs = Date.now();
      // v18.8-fix28f：rust_played=true 表示 Rust 端已直接播放——前端只更新去重
      // 时间戳（挡 say/speakReply 重复朗读），不再重复调用 tts_speak。
      if (p.rust_played === true) return;
      console.log('[qibai][fe] tts_speak trigger: kind=chat');
      invoke('tts_speak', {
        text: seg,
        kind: 'chat',
        persona_id: (p.persona) || activePersonaId || undefined,
      }).then((r) => {
        if (r && r.engine && r.engine !== 'mute') showTtsIndicator(r);
      }).catch(() => {});
    }
  });

  // v18.8-fix28f：聊天记录落库广播 → 实时刷新（跨窗口同步：桌宠/控制台任一处发言）
  // v20：改走 requestConversationReload（150ms 合并 + epoch 门）
  // v20-fix35：加 1s 轮询兜底——语音路径仅靠事件刷新，事件一旦未达面板就「重开才更新」；
  // 变更检测保证无变化不重建 DOM，轮询开销可忽略，事件驱动仍保即时性。
  listen('qibai://conversation-updated', () => {
    requestConversationReload();
  });

  /* ---------- v11：顶部免提快捷开关（点击切换；语音「打开/关闭/切换免提」同步实时刷新） ---------- */
  const btnHfQuick = document.getElementById('btnHfQuick');
  async function refreshHfQuick() {
    if (!btnHfQuick) return;
    try {
      const s = await invoke('get_listen_state');
      const on = !!(s && s.state && s.state !== 'off');
      btnHfQuick.classList.toggle('hf-on', on);
      const lab = document.getElementById('hfQuickLabel');
      if (lab) lab.textContent = on ? '免提中' : '免提关';
    } catch (e) { /* 桥接未就绪忽略 */ }
  }
  if (btnHfQuick) {
    btnHfQuick.addEventListener('click', async () => {
      try {
        const s = await invoke('get_listen_state');
        const next = !(s && s.state && s.state !== 'off');
        const r = await invoke('set_wake_enabled', { enabled: next });
        await refreshHfQuick();
        if (r && r.ok === false) toast('免提启动失败：' + ((r.reason || '') + '').slice(0, 40));
      } catch (e) { console.warn('[qibai] 免提快捷切换失败', e); }
    });
  }
  // 语音控制（listener try_hf_command）/唤醒切换/设置页开关 → 实时同步顶栏状态
  listen('qibai://listen-state', () => { refreshHfQuick().catch(() => {}); });
  refreshHfQuick().catch(() => {}); // 启动即对齐

  // v21 语音调度：Rust voice_ui_dispatch 命中导航意图后广播 → 切到目标 tab（全部 tab 语音可达）
  listen('qibai://ui-nav', (event) => {
    const p = (event && event.payload) || {};
    const tab = p.tab || '';
    if (TABS.includes(tab)) {
      switchTab(tab);
      toast(`已切到「${tab}」页`);
    } else {
      console.warn('[qibai] ui-nav 未知 tab:', tab);
    }
  });
  setInterval(() => {
    if (document.hidden) return;      // 窗口不可见不轮询（省资源）
    loadConversation().catch((e) => console.warn(e));
    refreshHfQuick().catch(() => {}); // v11：免提顶栏状态 1s 兜底对齐（事件驱动保即时）
  }, 1000);

  // v20：朗读播放状态 / 打断事件——失败与打断给可见提示，start/end 由诊断面板轮询展示
  listen('qibai://tts-playback', (event) => {
    const p = (event && event.payload) || {};
    if (p.state === 'fail') {
      toast(`朗读播放失败（${p.engine || '引擎'}${p.error ? '：' + p.error : ''}），已切换降级链`);
    }
  });
  listen('qibai://tts-interrupted', () => {
    toast('已停止朗读（检测到你说话）');
  });

  // v45.14：core 的工具执行留痕（engine 每轮 emit）——接到气泡折叠块上。
  // 时序有两种：① 事件先到（send_message 还没返回）→ 缓存，等气泡落地时消费；
  // ② 事件后到（气泡已在）→ 挂到最近一条 assistant 气泡上（限制在本次发送后 60s 内，避免挂到陈年旧消息）。
  listen('qibai://executions', (ev) => {
    const p = (ev && ev.payload) || {};
    const items = Array.isArray(p.items) ? p.items : [];
    if (!items.length) return;
    let target = (lastAssistantRow && lastAssistantRow.isConnected) ? lastAssistantRow : null;
    if (!target && Date.now() - lastSendAt < 60000) {
      const rows = document.querySelectorAll('#chatScroll .bubble-row.role');
      target = rows.length ? rows[rows.length - 1] : null;
    }
    if (target) attachExecutionFold(target, items);
    else pendingExec = pendingExec.concat(items);
  });

  listen('qibai://codecheck-run', (ev) => {
    const p = (ev && ev.payload) || {};
    switchTab('codecheck');
    const opts = {};
    if (p.path !== undefined) { $('ccPath').value = p.path; opts.path = p.path; }
    if (p.snippet !== undefined) { $('ccSnippet').value = p.snippet; opts.snippet = p.snippet; }
    if (p.language) opts.language = p.language;
    runCodecheck(opts);
  });
  listen('qibai://codecheck-chat-demo', () => {
    switchTab('chat');
    doSendWithText('帮我检查这段代码');
  });

  // 迭代②·语音演示：harness 广播后走真实聊天链路（回复朗读 + 指示 + tts-speak 广播）
  listen('qibai://voice-chat-demo', () => {
    switchTab('chat');
    doSendWithText('在吗');
  });

  // 初始空态
  renderCcReport(null);

  /* ============================================================
   * 形象网格（角色 tab）：点选 = set_active_persona
   * ============================================================ */
  function markPersonaSel(key) {
    document.querySelectorAll('#personaGrid .persona-card').forEach((c) =>
      c.classList.toggle('sel', c.dataset.key === key));
  }

  function bindPersonaGrid() {
    $('personaGrid').addEventListener('click', async (e) => {
      const c = e.target.closest('.persona-card');
      if (!c) return;
      const key = c.dataset.key;
      try {
        await invoke('set_active_persona', { persona_id: key });
        toast(`主角色已切换为「${PERSONA_NAMES[key] || key}」`);
        loadRoles(); // desk-changed 也会触发，这里即时刷新双保险
      } catch (err) {
        console.warn(err);
        toast('切换主角色失败');
      }
    });
  }

  let activePersonaId = null; // 当前主角色（聊天/技能门控上报用）
  let personaNameMap = {}; // v18.8-fix28g：persona id → 名字（气泡角色名标签）
  async function refreshPersonaSel() {
    try {
      const data = await invoke('get_personas');
      if (data && data.personas) {
        (data.personas || []).forEach((p) => { if (p && p.id) personaNameMap[p.id] = p.name || p.id; });
      }
      if (data && data.active_persona) {
        activePersonaId = data.active_persona;
        markPersonaSel(data.active_persona);
      }
    } catch (e) { console.warn(e); }
  }

  /* ============================================================
   * 角色管理列表（get_personas + get_desk_state 判定在场状态）
   * ============================================================ */
  async function renderMgmt() {
    const list = $('mgmtList');
    list.innerHTML = '<div class="hint">加载中…</div>';
    let data, desk;
    try {
      [data, desk] = await Promise.all([
        invoke('get_personas'),
        invoke('get_desk_state'),
      ]);
    } catch (e) {
      console.warn(e);
      list.innerHTML = '<div class="hint">角色数据加载失败</div>';
      return;
    }

    const active = data.active_persona;
    const present = (desk && desk.present) || [];
    list.innerHTML = '';

    (data.personas || []).forEach((p) => {
      const isActive = p.persona_id === active;
      const isPresent = present.includes(p.persona_id);
      const pName = PERSONA_NAMES[p.persona_id] || p.display_name || p.persona_id;

      const card = document.createElement('div');
      card.className = 'mgmt-card' + (isActive ? ' active' : '');

      const img = document.createElement('img');
      img.src = PERSONA_IMG[p.persona_id] || PERSONA_IMG.gudian;
      img.alt = p.display_name;

      const info = document.createElement('div');
      info.className = 'mgmt-info';
      const name = document.createElement('div');
      name.className = 'm-name';
      name.textContent = pName + (isActive ? '（主角色）' : '') + (isPresent && !isActive ? '（在场）' : '');
      const meta = document.createElement('div');
      meta.className = 'm-meta';
      meta.textContent = `与你亲密度 Lv.${p.intimacy} · 性格：${(p.traits && p.traits.tone) || '—'}`;

      const nickRow = document.createElement('div');
      nickRow.className = 'mgmt-nick';
      const nickInput = document.createElement('input');
      nickInput.type = 'text';
      nickInput.value = p.nickname || '';
      nickInput.maxLength = 12;
      const nickSave = document.createElement('button');
      nickSave.type = 'button';
      nickSave.textContent = '重命名';
      nickSave.addEventListener('click', async () => {
        const v = nickInput.value.trim();
        if (!v) return;
        try {
          await invoke('rename_persona', { persona_id: p.persona_id, nickname: v });
          nickSave.textContent = '已保存';
          setTimeout(() => { nickSave.textContent = '重命名'; renderMgmt(); }, 900);
        } catch (e2) { console.warn(e2); toast('重命名失败'); }
      });
      nickRow.appendChild(nickInput);
      nickRow.appendChild(nickSave);
      info.appendChild(name);
      info.appendChild(meta);
      info.appendChild(nickRow);

      const actions = document.createElement('div');
      actions.className = 'mgmt-actions';

      // 设为主角色
      const btnActive = document.createElement('button');
      btnActive.type = 'button';
      btnActive.textContent = isActive ? '当前主角色' : '设为主角色';
      btnActive.disabled = isActive;
      if (!isActive) btnActive.classList.add('primary');
      btnActive.addEventListener('click', async () => {
        try {
          await invoke('set_active_persona', { persona_id: p.persona_id });
          toast(`主角色已切换为「${pName}」`);
          loadRoles();
        } catch (e2) { console.warn(e2); toast('切换主角色失败'); }
      });
      actions.appendChild(btnActive);

      // 召唤上桌 / 送走（按在场状态禁用）
      const btnDesk = document.createElement('button');
      btnDesk.type = 'button';
      if (isActive) {
        btnDesk.textContent = '主角色常驻';
        btnDesk.disabled = true;
      } else if (isPresent) {
        btnDesk.textContent = '送走（离桌）';
        btnDesk.classList.add('on-desk');
        btnDesk.addEventListener('click', async () => {
          try {
            await invoke('pet_dismiss', { persona_id: p.persona_id });
            toast(`${pName} 道别离开桌子啦～`);
            loadRoles();
          } catch (e2) { console.warn(e2); toast('送走失败（主角色不可送走）'); }
        });
      } else {
        btnDesk.textContent = '召唤上桌';
        btnDesk.addEventListener('click', async () => {
          try {
            await invoke('pet_summon', { persona_id: p.persona_id });
            toast(`${pName} 来桌上作客啦～`);
            loadRoles();
          } catch (e2) { console.warn(e2); toast('召唤失败'); }
        });
      }
      actions.appendChild(btnDesk);

      card.appendChild(img);
      card.appendChild(info);
      card.appendChild(actions);
      list.appendChild(card);
    });
  }

  /* ============================================================
   * 角色关系（get_social_state：好感度/等级/称谓/今日 PK 剩余）
   * ============================================================ */
  async function renderRelations() {
    const box = $('relationList');
    box.innerHTML = '<div class="hint">加载中…</div>';
    let st;
    try {
      st = await invoke('get_social_state');
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="hint">关系数据加载失败</div>';
      return;
    }
    box.innerHTML = '';
    (st.relations || []).forEach((r) => {
      const lv = (st.levels || []).find((l) => l.level === r.level) || {};
      const row = document.createElement('div');
      row.className = 'relation-row';
      const head = document.createElement('div');
      head.className = 'relation-head';
      const pair = document.createElement('span');
      pair.textContent = `${PERSONA_NAMES[r.a] || r.a} × ${PERSONA_NAMES[r.b] || r.b}`;
      const lvName = document.createElement('span');
      lvName.className = 'r-level';
      lvName.textContent = lv.name || '初识';
      head.appendChild(pair);
      head.appendChild(lvName);

      const bar = document.createElement('div');
      bar.className = 'relation-bar';
      const fill = document.createElement('i');
      const pct = Math.max(2, Math.min(100, Math.round((r.affinity / 600) * 100)));
      fill.style.width = pct + '%';
      bar.appendChild(fill);

      const foot = document.createElement('div');
      foot.className = 'relation-foot';
      const left = document.createElement('span');
      left.textContent = `好感度 ${r.affinity} · 称谓：${lv.title || '—'}`;
      const pkInfo = document.createElement('span');
      pkInfo.textContent = `今日 PK ${r.pk_left != null ? (3 - r.pk_left) : (r.pk_count ? r.pk_count.count : 0)}/3`;
      foot.appendChild(left);
      foot.appendChild(pkInfo);

      row.appendChild(head);
      row.appendChild(bar);
      row.appendChild(foot);
      box.appendChild(row);
    });
  }

  /* ============================================================
   * 事件日志（get_social_events 时间倒序）
   * ============================================================ */
  async function renderEvents() {
    const box = $('eventListBox');
    box.innerHTML = '<div class="hint">加载中…</div>';
    let res;
    try {
      res = await invoke('get_social_events', { limit: 40 });
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="hint">事件加载失败</div>';
      return;
    }
    box.innerHTML = '';
    (res.events || []).forEach((ev) => {
      const row = document.createElement('div');
      row.className = 'event-row';
      const tag = document.createElement('span');
      tag.className = 'ev-tag ev-' + ev.type;
      tag.textContent = TYPE_NAME[ev.type] || ev.type;
      const text = document.createElement('span');
      text.className = 'ev-text';
      const who = ev.a
        ? `${PERSONA_NAMES[ev.a] || ev.a}${ev.b ? ' × ' + (PERSONA_NAMES[ev.b] || ev.b) : ''}`
        : '';
      text.textContent = (who ? who + '：' : '') + ((ev.detail && ev.detail.text) || '');
      const time = document.createElement('span');
      time.className = 'ev-time';
      const d = new Date(ev.ts);
      time.textContent = `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ` +
        `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
      row.appendChild(tag); row.appendChild(text); row.appendChild(time);
      box.appendChild(row);
    });
  }

  async function loadRoles() {
    await Promise.all([renderMgmt(), renderRelations(), refreshPersonaSel()]);
  }

  /* ============================================================
   * 亲密度 tab（L1-L10 成长系统，冻结规格 §8.1）
   * 只读契约（§6.3）：开/关判定一律以后端 get_intimacy_state 的
   * unlocked/locked 与 check_gate 返回为准，前端不复制阈值做判断。
   * 下方 GATE_META / ACH_META 仅为展示用标签与分组镜像（§4/§5）。
   * ============================================================ */
  const INTIMACY_GREET_RE = /(早安|早上好|早呀|早啊|morning)/i;

  // 展示元数据：门控标签/类型/分组等级（镜像 §4，不参与开/关判定）
  const INTIMACY_GATE_META = [
    { gate: 'basic_chat', label: '基础对话', kind: 'active', level: 1 },
    { gate: 'idle_actions', label: '待机动作', kind: 'active', level: 1 },
    { gate: 'sit_reminder', label: '久坐提醒', kind: 'active', level: 1 },
    { gate: 'custom_nickname', label: '自定义称呼', kind: 'active', level: 2 },
    { gate: 'daily_greeting', label: '每日问候', kind: 'active', level: 2 },
    { gate: 'water_reminder', label: '喝水提醒', kind: 'active', level: 2 },
    { gate: 'preference_memory', label: '记住偏好（作息/常用软件）', kind: 'active', level: 3 },
    { gate: 'boot_greeting', label: '开机问候', kind: 'active', level: 3 },
    { gate: 'assistant_skills', label: '助理技能（打开应用/文件夹、设提醒）', kind: 'active', level: 4 },
    { gate: 'vault', label: '本地密码保险箱', kind: 'active', level: 4 },
    { gate: 'mood_empathy', label: '心情识别增强', kind: 'active', level: 5 },
    { gate: 'screen_guard', label: '全屏守护', kind: 'placeholder', level: 5 },
    { gate: 'performance', label: '表演系统（节日彩蛋接口）', kind: 'active', level: 6 },
    { gate: 'dressup', label: '形象换装', kind: 'placeholder', level: 6 },
    { gate: 'storyline_1', label: '专属剧情线一（对话彩蛋模板）', kind: 'active', level: 7 },
    { gate: 'anniversary', label: '纪念日自动庆祝（纪念日设置）', kind: 'active', level: 7 },
    { gate: 'sleep_guard', label: '息屏守护', kind: 'placeholder', level: 8 },
    { gate: 'voice_mode', label: '语音陪伴模式', kind: 'placeholder', level: 8 },
    { gate: 'storyline_2', label: '专属剧情线二（双人彩蛋）', kind: 'active', level: 9 },
    { gate: 'ceremony', label: '缔结仪式结局演出（文字+CSS 动效）', kind: 'active', level: 10 },
    { gate: 'batch_skills', label: '高级助理批量技能', kind: 'placeholder', level: 10 },
  ];

  // 展示元数据：9 个成就（镜像 §5）
  const INTIMACY_ACH_META = [
    { id: 'first_greeting', name: '初次问候', desc: '完成首次早安问候' },
    { id: 'chat_100', name: '聊得来', desc: '累计聊天 100 次' },
    { id: 'click_50', name: '指尖温情', desc: '累计点按 50 次' },
    { id: 'pk_win_5', name: '猜拳小能手', desc: '累计赢下 5 场猜拳' },
    { id: 'companion_7', name: '常伴七日', desc: '累计陪伴满 7 天' },
    { id: 'level_3', name: '心有默契', desc: '亲密度达到 L3' },
    { id: 'level_6', name: '亲密无间', desc: '亲密度达到 L6' },
    { id: 'level_9', name: '灵魂共鸣', desc: '亲密度达到 L9' },
    { id: 'level_10', name: '缔结之约', desc: '亲密度达到 L10' },
  ];

  let intiPersona = null;  // 亲密度页当前查看角色（默认主角色）
  let intiLevels = null;   // get_intimacy_levels 结果
  let intiState = null;    // get_intimacy_state 结果
  let intiAnnivOpen = null;

  async function loadIntimacyTab() {
    if (!intiPersona) {
      try {
        const data = await invoke('get_personas');
        intiPersona = (data && data.active_persona) || 'gudian';
        activePersonaId = intiPersona;
      } catch (e) { intiPersona = 'gudian'; }
    }
    renderIntiChips();
    await Promise.all([loadIntiLevels(), loadIntiState()]);
    refreshAnnivGate();
  }

  function renderIntiChips() {
    const box = $('intiChips');
    box.innerHTML = '';
    Object.keys(PERSONA_NAMES).forEach((id) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'guest-chip' + (id === intiPersona ? ' sel' : '');
      chip.textContent = PERSONA_NAMES[id];
      chip.addEventListener('click', async () => {
        if (intiPersona === id) return;
        intiPersona = id;
        renderIntiChips();
        await loadIntiState();
        refreshAnnivGate();
      });
      box.appendChild(chip);
    });
  }

  async function loadIntiLevels() {
    try {
      intiLevels = await invoke('get_intimacy_levels');
    } catch (e) {
      intiLevels = null;
    }
  }

  async function loadIntiState() {
    const box = $('intiUnlockList');
    try {
      intiState = await invoke('get_intimacy_state', { persona_id: intiPersona });
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="hint">亲密度数据加载中（桥接 mock 就绪后自动显示）…</div>';
      $('intiLevelDesc').textContent = '';
      return;
    }
    renderIntiLevel();
    renderIntiUnlock();
    renderIntiAch();
    renderIntiTasks();
    renderIntiAnniv();
  }

  function renderIntiLevel() {
    const s = intiState;
    $('intiBadgeLv').textContent = 'L' + s.level;
    $('intiLevelName').textContent = s.level_name || ('L' + s.level);
    const lv = (intiLevels && intiLevels.levels)
      ? (intiLevels.levels.find((l) => l.level === s.level) || {})
      : {};
    $('intiLevelDesc').textContent = lv.desc || '';
    const pr = s.progress || {};
    const maxTxt = pr.max == null ? '∞' : pr.max;
    let ratio = pr.ratio;
    if (ratio == null) {
      ratio = pr.max == null ? 1 : (s.points - (pr.min || 0)) / (pr.max - (pr.min || 0));
    }
    const pct = Math.max(0, Math.min(100, Math.round((ratio || 0) * 100)));
    $('intiProgFill').style.width = pct + '%';
    $('intiProgText').textContent = `当前档 ${pr.min != null ? pr.min : 0} – ${maxTxt} 积分（${pct}%）`;
    $('intiPoints').textContent = `总积分 ${s.points}`;
    const d = s.daily || {};
    $('intiDaily').textContent = `今日已得 ${d.gained != null ? d.gained : 0}/${d.cap != null ? d.cap : 60}`;
  }

  function renderIntiUnlock() {
    const box = $('intiUnlockList');
    box.innerHTML = '';
    const s = intiState || {};
    const openSet = new Set(s.unlocked || []);
    const lockedMap = {};
    (s.locked || []).forEach((l) => { lockedMap[l.gate] = l; });

    // 按等级分组（分组等级：未解锁项用后端给的 required_level，已解锁项用本地展示镜像）
    const groups = {};
    INTIMACY_GATE_META.forEach((g) => {
      const open = openSet.has(g.gate);
      const lock = lockedMap[g.gate];
      const level = open ? g.level : (lock ? lock.required_level : g.level);
      const label = lock ? lock.label : g.label;
      const kind = lock ? lock.kind : g.kind;
      (groups[level] = groups[level] || []).push({ label, kind, open });
    });

    Object.keys(groups).sort((a, b) => Number(a) - Number(b)).forEach((lv) => {
      const head = document.createElement('div');
      head.className = 'inti-group-head';
      head.textContent = 'L' + lv;
      box.appendChild(head);
      groups[lv].forEach((item) => {
        const row = document.createElement('div');
        row.className = 'inti-gate' + (item.open ? ' open' : ' locked');
        const mark = document.createElement('span');
        mark.className = 'ig-mark';
        mark.textContent = item.open ? '✓' : '🔒';
        const label = document.createElement('span');
        label.className = 'ig-label';
        label.textContent = item.label;
        const tag = document.createElement('span');
        tag.className = 'ig-tag' + (item.kind === 'placeholder' ? ' ph' : '');
        tag.textContent = item.open
          ? (item.kind === 'placeholder' ? '已解锁·占位' : '可用')
          : 'L' + lv + ' 解锁';
        row.appendChild(mark); row.appendChild(label); row.appendChild(tag);
        box.appendChild(row);
      });
    });
  }

  function renderIntiAch() {
    const box = $('intiAchGrid');
    box.innerHTML = '';
    const got = new Set(((intiState && intiState.achievements) || []).map((a) => a.id));
    INTIMACY_ACH_META.forEach((a) => {
      const d = document.createElement('div');
      d.className = 'inti-ach' + (got.has(a.id) ? ' got' : '');
      const badge = document.createElement('div');
      badge.className = 'ia-badge';
      badge.textContent = got.has(a.id) ? '🏅' : '🔒';
      const nm = document.createElement('div');
      nm.className = 'ia-name';
      nm.textContent = a.name;
      const ds = document.createElement('div');
      ds.className = 'ia-desc';
      ds.textContent = a.desc;
      d.appendChild(badge); d.appendChild(nm); d.appendChild(ds);
      box.appendChild(d);
    });
  }

  function renderIntiTasks() {
    const box = $('intiTasks');
    box.innerHTML = '';
    (intiState && intiState.tasks || []).forEach((t) => {
      const card = document.createElement('div');
      card.className = 'inti-task' + (t.done ? ' done' : '');
      const info = document.createElement('div');
      const nm = document.createElement('div');
      nm.className = 'it-name';
      nm.textContent = (t.done ? '✓ ' : '') + t.name;
      const ds = document.createElement('div');
      ds.className = 'it-desc';
      ds.textContent = t.done ? '今日已完成（+5）' : '今日未完成（+5）';
      info.appendChild(nm); info.appendChild(ds);
      card.appendChild(info);
      if (t.id === 'greeting') {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'primary';
        btn.textContent = '说早安';
        btn.disabled = !!t.done;
        btn.addEventListener('click', doSayGreeting);
        card.appendChild(btn);
      }
      box.appendChild(card);
    });
  }

  async function doSayGreeting() {
    try {
      const res = await invoke('intimacy_event', {
        persona_id: intiPersona, kind: 'task', detail: { task_id: 'greeting' },
      });
      toast(res && res.delta > 0 ? `早安任务完成，亲密度 +${res.delta}` : '早安任务今日已完成过了');
      applyIntiEventResult(res);
      await loadIntiState();
    } catch (e) {
      console.warn(e);
      toast('亲密度事件暂不可用（桥接 mock 未就绪）');
    }
  }

  function renderIntiAnniv() {
    const a = intiState && intiState.anniversary;
    if (a && a.month && a.day) {
      $('annivMonth').value = a.month;
      $('annivDay').value = a.day;
      $('annivNote').textContent = `当前纪念日：${a.month} 月 ${a.day} 日，当天她会记得为你庆祝。`;
    } else {
      $('annivNote').textContent = '尚未设置纪念日。';
    }
    if (intiAnnivOpen === false) {
      $('annivNote').textContent = '🔒 纪念日功能需亲密度 L7「羁绊」解锁。';
    }
  }

  async function refreshAnnivGate() {
    let open = true; // mock 未就绪时不锁死，避免无法冒烟
    try {
      const r = await invoke('check_gate', { persona_id: intiPersona, gate: 'anniversary' });
      open = !!(r && r.open);
    } catch (e) { open = true; }
    intiAnnivOpen = open;
    $('annivMonth').disabled = !open;
    $('annivDay').disabled = !open;
    $('btnAnnivSave').disabled = !open;
    $('btnAnnivClear').disabled = !open;
    $('annivCard').classList.toggle('gate-locked', !open);
    renderIntiAnniv();
  }

  $('btnAnnivSave').addEventListener('click', async () => {
    const m = parseInt($('annivMonth').value, 10);
    const d = parseInt($('annivDay').value, 10);
    if (!(m >= 1 && m <= 12) || !(d >= 1 && d <= 31)) {
      toast('请输入合法的月（1-12）和日（1-31）');
      return;
    }
    try {
      await invoke('set_anniversary', { persona_id: intiPersona, month: m, day: d });
      toast('纪念日已保存');
      await loadIntiState();
    } catch (e) { console.warn(e); toast('保存失败'); }
  });

  $('btnAnnivClear').addEventListener('click', async () => {
    try {
      await invoke('clear_anniversary', { persona_id: intiPersona });
      toast('纪念日已清除');
      await loadIntiState();
    } catch (e) { console.warn(e); toast('清除失败'); }
  });

  /* 升级动画 + 解锁通知 banner（§8.1） */
  function triggerUpgradeAnim() {
    const badge = $('intiBadge');
    badge.classList.remove('upgrading');
    void badge.offsetWidth; // 重启动画
    badge.classList.add('upgrading');
  }

  function showIntiBanner(p) {
    const b = $('intiBanner');
    const lines = [];
    if (p.leveled_up) lines.push(`🎉 升级至 L${p.level != null ? p.level : ''}「${p.level_name || ''}」`);
    const unlocked = Array.isArray(p.unlocked) ? p.unlocked : [];
    if (unlocked.length) lines.push('新解锁：' + unlocked.map((u) => u.label || u.gate).join('、'));
    const ach = Array.isArray(p.achievements_granted) ? p.achievements_granted : [];
    if (ach.length) lines.push('🏅 成就达成：' + ach.map((a) => a.name || a.id).join('、'));
    if (!lines.length) return;
    b.textContent = lines.join('　');
    b.hidden = false;
    b.classList.remove('show');
    void b.offsetWidth;
    b.classList.add('show');
    clearTimeout(showIntiBanner._t);
    showIntiBanner._t = setTimeout(() => {
      b.classList.remove('show');
      b.hidden = true;
    }, 6000);
  }

  // 本地 intimacy_event 返回结果的升级/解锁处理
  function applyIntiEventResult(res) {
    if (!res) return;
    const unlocked = Array.isArray(res.unlocked) ? res.unlocked : [];
    if (res.leveled_up) triggerUpgradeAnim();
    if (res.leveled_up || unlocked.length) showIntiBanner(res);
  }

  // 聊天亲密度上报（fire-and-forget，不影响聊天主流程）
  async function reportChatIntimacy(text) {
    try {
      const pid = activePersonaId || intiPersona || 'gudian';
      await invoke('intimacy_event', { persona_id: pid, kind: 'chat' });
      if (INTIMACY_GREET_RE.test(text || '')) {
        await invoke('intimacy_event', { persona_id: pid, kind: 'task', detail: { task_id: 'greeting' } });
      }
    } catch (e) { /* mock 未就绪时静默 */ }
  }

  /* ============================================================
   * 技能授权门控（§8.1）：check_gate('assistant_skills') 为 false
   * 时整区块锁定、开关禁用；open=true 恢复。
   * ============================================================ */
  async function refreshSkillsGate() {
    let pid = activePersonaId || intiPersona;
    if (!pid) {
      try {
        const d = await invoke('get_personas');
        pid = (d && d.active_persona) || 'gudian';
        activePersonaId = pid;
      } catch (e) { pid = 'gudian'; }
    }
    let open = true; // mock 未就绪时默认放行
    try {
      const r = await invoke('check_gate', { persona_id: pid, gate: 'assistant_skills' });
      open = !!(r && r.open);
    } catch (e) { open = true; }
    $('skillsCard').classList.toggle('gate-locked', !open);
    $('skillsLockBanner').hidden = open;
    ['swSkillsTotal', 'swSkillLaunch', 'swSkillFolder', 'swSkillReminder'].forEach((id) => {
      const el = $(id);
      el.style.pointerEvents = open ? '' : 'none';
      el.style.opacity = open ? '' : '.45';
    });
  }

  /* ============================================================
   * 猜拳 tab：客人选择器 / 串门寒暄 / 对局状态机
   * ============================================================ */
  const pkState = { activeId: null, guestId: null, busy: false, blocked: false, remaining: 3 };

  async function loadPkTab() {
    try {
      const [data, desk] = await Promise.all([
        invoke('get_personas'),
        invoke('get_desk_state'),
      ]);
      pkState.activeId = data.active_persona;
      $('kMainName').textContent = (PERSONA_NAMES[data.active_persona] || '元气小小') + '（你）';
      const present = (desk && desk.present) || [];
      const guests = present.filter((id) => id !== data.active_persona);
      renderGuestChips(guests);
    } catch (e) {
      console.warn(e);
      $('guestChips').innerHTML = '<div class="hint">桌面状态加载失败</div>';
    }
  }

  function renderGuestChips(guests) {
    const box = $('guestChips');
    box.innerHTML = '';
    if (!guests.length) {
      pkState.guestId = null;
      box.innerHTML = '<div class="hint">桌上没有客人哦。去「角色」页召唤一位小伙伴上桌吧。</div>';
      $('kGuestName').textContent = '客人';
      return;
    }
    if (!guests.includes(pkState.guestId)) pkState.guestId = guests[0];
    guests.forEach((id) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'guest-chip' + (pkState.guestId === id ? ' sel' : '');
      chip.textContent = PERSONA_NAMES[id] || id;
      chip.addEventListener('click', () => {
        pkState.guestId = id;
        renderGuestChips(guests);
        $('kResult').textContent = `已选择「${PERSONA_NAMES[id] || id}」当对手，点手势开始。`;
      });
      box.appendChild(chip);
    });
    $('kGuestName').textContent = PERSONA_NAMES[pkState.guestId] || '客人';
  }

  function renderLines(lines) {
    const box = $('kLines');
    box.innerHTML = '';
    (lines || []).forEach((l) => {
      const row = document.createElement('div');
      row.className = 'k-line';
      const sp = document.createElement('span');
      sp.className = 'k-speaker';
      sp.textContent = PERSONA_NAMES[l.speaker] || l.speaker || '—';
      const tx = document.createElement('span');
      tx.textContent = l.text || '';
      row.appendChild(sp);
      row.appendChild(tx);
      box.appendChild(row);
    });
  }

  // 串门寒暄：social_greet {from:客人, to:主角色}
  async function doGreet() {
    if (!pkState.guestId || !pkState.activeId) {
      toast('请先在「角色」页召唤一位客人上桌');
      return;
    }
    try {
      const res = await invoke('social_greet', { from: pkState.guestId, to: pkState.activeId });
      renderLines(res.lines || []);
      if (res.affinity_delta > 0) toast(`串门寒暄：好感度 +${res.affinity_delta}`);
      if (res.event && res.event.type === 'level_up' && res.event.detail) {
        toast(`关系升级彩蛋：${res.event.detail.text}！`);
      }
      loadRoles();
    } catch (e) {
      console.warn(e);
      toast('串门失败，请稍后再试');
    }
  }
  $('btnGreet').addEventListener('click', doGreet);

  // 进入对局：重置结果区，等待手势
  $('btnPkStart').addEventListener('click', () => {
    if (!pkState.guestId) {
      toast('请先选择在场客人（或去「角色」页召唤）');
      return;
    }
    pkState.busy = false;
    $('kResult').textContent = '出拳吧！';
    $('kLines').innerHTML = '';
    $('kMainHand').innerHTML = MOVE_SVG.rock;
    $('kGuestHand').innerHTML = MOVE_SVG.rock;
    setGestureBtnsDisabled(pkState.blocked);
  });

  function setGestureBtnsDisabled(v) {
    document.querySelectorAll('#pkMoves button[data-move]').forEach((b) => { b.disabled = v; });
  }

  async function doPkRound(ma, mb) {
    if (pkState.busy) return;
    if (!pkState.guestId || !pkState.activeId) {
      toast('请先选择在场客人');
      return;
    }
    if (pkState.blocked) {
      toast('今天猜拳次数用完啦，明天再来～');
      return;
    }
    pkState.busy = true;
    setGestureBtnsDisabled(true);
    $('kResult').textContent = '出拳中……';
    $('kMainHand').classList.add('shaking');
    $('kGuestHand').classList.add('shaking');
    await wait(800);
    $('kMainHand').classList.remove('shaking');
    $('kGuestHand').classList.remove('shaking');
    $('kMainHand').innerHTML = MOVE_SVG[ma] || MOVE_SVG.rock;
    $('kGuestHand').innerHTML = MOVE_SVG[mb] || MOVE_SVG.rock;

    try {
      const res = await invoke('social_pk', {
        a: pkState.activeId, b: pkState.guestId, move_a: ma, move_b: mb,
      });
      applyPkResult(res, ma, mb);
    } catch (e) {
      console.warn(e);
      $('kResult').textContent = '对局出错';
    }
    pkState.busy = false;
    setGestureBtnsDisabled(pkState.blocked);
  }

  function applyPkResult(res, ma, mb) {
    if (res.blocked) {
      pkState.blocked = true;
      $('kResult').textContent = (res.lines && res.lines[0] && res.lines[0].text) || '今日次数已用完';
      $('kRemaining').textContent = '今日剩余 0 次';
      renderLines(res.lines || []);
      toast('今日猜拳次数已用完');
      return;
    }
    if (res.result === 'draw') {
      $('kResult').textContent = `平局！心有灵犀～（${MOVE_LABEL[ma]} vs ${MOVE_LABEL[mb]}）`;
    } else {
      const wName = PERSONA_NAMES[res.winner] || res.winner || '—';
      $('kResult').textContent = `${wName} 赢啦！（${MOVE_LABEL[ma]} vs ${MOVE_LABEL[mb]}）`;
    }
    pkState.remaining = res.remaining_today;
    $('kRemaining').textContent = `今日剩余 ${res.remaining_today} 次`;
    const d = res.affinity_delta || {};
    const parts = Object.keys(d).map((k) => `${PERSONA_NAMES[k] || k}+${d[k]}`);
    if (parts.length) toast(`好感度变化：${parts.join('，')}`);
    renderLines(res.lines || []);
    // 亲密度联动（§3.4/§5）：主角色获胜 → pk_win 事件（+0 积分，累计成就/任务）
    if (!res.blocked && res.winner && res.winner === pkState.activeId) {
      invoke('intimacy_event', { persona_id: res.winner, kind: 'pk_win' }).catch(() => {});
    }
    if (res.remaining_today <= 0) {
      pkState.blocked = true;
      toast('今日 3 次猜拳已用尽，明天再来吧～');
    }
    loadRoles();
  }

  $('pkMoves').addEventListener('click', (e) => {
    const b = e.target.closest('button[data-move]');
    if (!b || b.disabled) return;
    const ma = b.dataset.move;
    const mb = ['rock', 'paper', 'scissors'][Math.floor(Math.random() * 3)];
    doPkRound(ma, mb);
  });

  /* ============================================================
   * 保险箱 tab（迭代③，冻结契约 VAULT_SPEC.md）
   * 5 态：gated（L4 前，横幅锁定）/ unset（初始化向导）/
   *       locked（解锁表单 + 冷却倒计时）/ unlocked（列表 CRUD）。
   * 开/关与状态一律以后端 vault_status 为准，前端只渲染与交互。
   * ============================================================ */
  let vaultStatus = null;
  let vaultCooldownTimer = null;
  let vaultPollTimer = null;

  async function loadVaultTab() {
    await refreshVaultStatus();
  }

  async function refreshVaultStatus() {
    try {
      vaultStatus = await invoke('vault_status');
    } catch (e) {
      console.warn(e);
      $('vaultBody').innerHTML = '<div class="hint">保险箱状态加载失败（桥接 mock 未就绪）…</div>';
      return;
    }
    renderVault();
  }

  function renderVault() {
    const st = vaultStatus;
    if (!st) return;
    $('vaultGatedBanner').hidden = !st.gated; // tab 始终可见；gated 时横幅常显
    if (st.gated) { renderVaultGated(); return; }
    if (st.state === 'unset') { renderVaultUnset(st); return; }
    if (st.state === 'locked') { renderVaultLocked(st); return; }
    renderVaultUnlocked(st);
  }

  function vaultErrText(e) {
    if (!e) return '操作失败';
    if (e.code === 'VaultWrongPassword') return '主密码不正确';
    if (e.code === 'VaultCooldown') return `冷却中，请 ${e.remaining_secs || ''} 秒后再试`;
    if (e.code === 'VaultGateLocked') return `亲密度达到 L${e.required || 4} 后解锁`;
    return e.message || '操作失败';
  }

  /* ---- gated：锁定横幅 + 说明（禁用全部操作） ---- */
  function renderVaultGated() {
    $('vaultBody').innerHTML =
      '<div class="settings-card">' +
      '<h2>本地密码保险箱</h2>' +
      '<p class="vault-desc">在这里加密存放各网站的账号密码：只存在本机，不进云、不喂给模型。</p>' +
      '<div class="vault-lock-hint">🔒 未开放：和元气小小多聊天提升亲密度，达到 L4「贴心」后自动解锁，届时可在本页初始化并管理条目。</div>' +
      '</div>';
  }

  /* ---- unset：初始化向导（主密码两次一致 / 系统钥匙串） ---- */
  function renderVaultUnset(st) {
    $('vaultBody').innerHTML =
      '<div class="settings-card">' +
      '<h2>初始化保险箱</h2>' +
      '<p class="vault-desc">选择解锁方式：主密码（自己记一个强密码），或系统钥匙串（系统托管，免记密码）。</p>' +
      '<div class="field"><label for="vSetupPwd">设置主密码</label>' +
      '<input id="vSetupPwd" type="password" placeholder="至少 6 位" autocomplete="new-password"></div>' +
      '<div class="field"><label for="vSetupPwd2">再次输入主密码</label>' +
      '<input id="vSetupPwd2" type="password" autocomplete="new-password"></div>' +
      '<div class="vault-btn-row">' +
      '<button id="vSetupBtn" class="primary" type="button">设置主密码并初始化</button>' +
      (st.keyring_available ? '<button id="vSetupKeyringBtn" type="button">使用系统钥匙串（免主密码）</button>' : '') +
      '</div>' +
      '<div class="hint" style="margin-top:8px;">保险箱数据加密后落盘；主密码一旦丢失，内部数据无法恢复。</div>' +
      '</div>';
    $('vSetupBtn').addEventListener('click', async () => {
      const p1 = $('vSetupPwd').value, p2 = $('vSetupPwd2').value;
      if (p1.length < 6) { toast('主密码至少 6 位'); return; }
      if (p1 !== p2) { toast('两次输入不一致'); return; }
      try {
        await invoke('vault_setup', { master_password: p1 });
        toast('保险箱已初始化');
        await refreshVaultStatus();
      } catch (e) { console.warn(e); toast(vaultErrText(e)); }
    });
    if (st.keyring_available) {
      $('vSetupKeyringBtn').addEventListener('click', async () => {
        try {
          await invoke('vault_setup_keyring', {});
          toast('已用系统钥匙串初始化');
          await refreshVaultStatus();
        } catch (e) { console.warn(e); toast(vaultErrText(e)); }
      });
    }
  }

  /* ---- locked：解锁表单（主密码 / 系统解锁 + 冷却倒计时） ---- */
  function renderVaultLocked(st) {
    const cd = st.cooldown_remaining_secs || 0;
    $('vaultBody').innerHTML =
      '<div class="settings-card">' +
      '<h2>保险箱已锁定' + (st.locked_reason === 'idle' ? '（闲置自动锁定）' : '') + '</h2>' +
      (cd > 0 ? `<div class="vault-cooldown-banner">⏳ 连续输错次数过多，冷却中，剩余 ${cd} 秒</div>` : '') +
      '<div class="field"><label for="vUnlockPwd">主密码</label>' +
      '<input id="vUnlockPwd" type="password" placeholder=' + '"输入主密码"' + ' autocomplete="current-password"></div>' +
      '<div class="vault-btn-row">' +
      `<button id="vUnlockBtn" class="primary" type="button"${cd > 0 ? ' disabled' : ''}>解锁</button>` +
      (st.keyring_available ? `<button id="vUnlockSysBtn" type="button"${cd > 0 ? ' disabled' : ''}>系统解锁</button>` : '') +
      '</div>' +
      (st.failed_attempts ? `<div class="hint" style="margin-top:6px;">已连续输错 ${st.failed_attempts}/${st.max_wrong_attempts} 次</div>` : '') +
      '</div>';
    $('vUnlockBtn').addEventListener('click', async () => {
      try {
        await invoke('vault_unlock', { master_password: $('vUnlockPwd').value });
        toast('已解锁');
        await refreshVaultStatus();
      } catch (e) { console.warn(e); toast(vaultErrText(e)); await refreshVaultStatus(); }
    });
    if (st.keyring_available) {
      $('vUnlockSysBtn').addEventListener('click', async () => {
        try {
          await invoke('vault_unlock_system', {});
          toast('已通过系统钥匙串解锁');
          await refreshVaultStatus();
        } catch (e) { console.warn(e); toast(vaultErrText(e)); }
      });
    }
    // 冷却倒计时：每秒轮询刷新；冷却结束或状态变化后停止
    if (vaultCooldownTimer) { clearInterval(vaultCooldownTimer); vaultCooldownTimer = null; }
    if (cd > 0) {
      vaultCooldownTimer = setInterval(async () => {
        const s = await invoke('vault_status').catch(() => null);
        if (!s) return;
        vaultStatus = s;
        if (s.cooldown_remaining_secs <= 0 || s.state !== 'locked') {
          clearInterval(vaultCooldownTimer); vaultCooldownTimer = null;
        }
        if (s.state === 'locked') renderVault();
      }, 1000);
    }
  }

  /* ---- unlocked：工具栏 + 条目列表 ---- */
  function renderVaultUnlocked(st) {
    $('vaultBody').innerHTML =
      '<div class="settings-card">' +
      '<h2>保险箱</h2>' +
      '<div class="vault-toolbar">' +
      '<input id="vSearch" type="text" placeholder="搜索站点 / 账号 / 备注…" autocomplete="off">' +
      '<button id="vAddBtn" class="primary" type="button">新增</button>' +
      '<button id="vExportBtn" type="button">导出</button>' +
      '<button id="vImportBtn" type="button">导入</button>' +
      '<button id="vLockBtn" type="button">立即锁定</button>' +
      '</div>' +
      '<div id="vEditForm"></div>' +
      '<div id="vEntryList"></div>' +
      `<div class="hint" style="margin-top:8px;">复制的密码 ${st.clipboard_clear_secs} 秒后自动清除；闲置 ${st.idle_lock_min} 分钟自动锁定。</div>` +
      '</div>';
    $('vAddBtn').addEventListener('click', () => vaultRenderEditForm(null));
    $('vSearch').addEventListener('input', () => vaultRenderEntries($('vSearch').value.trim()));
    $('vLockBtn').addEventListener('click', async () => {
      try {
        await invoke('vault_lock');
        toast('已锁定');
        await refreshVaultStatus();
      } catch (e) { console.warn(e); toast(vaultErrText(e)); }
    });
    $('vExportBtn').addEventListener('click', async () => {
      try {
        const r = await invoke('vault_export');
        toast('已导出备份：' + ((r && r.path) || 'vault-backup.qbkv'));
      } catch (e) { console.warn(e); toast('导出失败：' + (e.message || e)); }
    });
    $('vImportBtn').addEventListener('click', () => $('vaultFileInput').click());
    vaultRenderEntries('');
    // 轮询感知闲置自动锁定（后端 tick）；切走 tab 则暂停渲染
    if (vaultPollTimer) clearInterval(vaultPollTimer);
    vaultPollTimer = setInterval(async () => {
      if (currentTab !== 'vault') return;
      const s = await invoke('vault_status').catch(() => null);
      if (!s) return;
      if (s.state !== 'unlocked') { vaultStatus = s; renderVault(); }
    }, 4000);
  }

  async function vaultRenderEntries(q) {
    const box = $('vEntryList');
    if (!box) return;
    try {
      const list = q ? await invoke('vault_search', { q }) : await invoke('vault_list');
      if (!list.length) {
        box.innerHTML = '<div class="vault-empty">还没有密码条目，点「新增」存第一条吧。</div>';
        return;
      }
      box.innerHTML = '';
      list.forEach((e) => box.appendChild(vaultEntryCard(e)));
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="vault-empty">条目加载失败（' + escapeHtml(e.message || '') + '）</div>';
    }
  }

  function vaultIconText(icon, site) {
    if (icon && String(icon).trim()) return String(icon).trim().slice(0, 2);
    return (site ? String(site).trim().slice(0, 1) : '🔑');
  }

  function vaultEntryCard(e) {
    const row = document.createElement('div');
    row.className = 'vault-entry';
    const icon = document.createElement('div');
    icon.className = 'vault-entry-icon';
    icon.textContent = vaultIconText(e.icon, e.site);
    const info = document.createElement('div');
    info.className = 'vault-entry-info';
    const site = document.createElement('div');
    site.className = 've-site';
    site.textContent = e.site || '（未命名站点）';
    const acc = document.createElement('div');
    acc.className = 've-account';
    acc.textContent = e.account || '—';
    const pwd = document.createElement('div');
    pwd.className = 've-password';
    pwd.textContent = '••••••••'; // 掩码展示，列表载荷不含明文（契约 §4）
    info.appendChild(site); info.appendChild(acc); info.appendChild(pwd);
    if (e.note) {
      const note = document.createElement('div');
      note.className = 've-note';
      note.textContent = e.note;
      info.appendChild(note);
    }
    row.appendChild(icon); row.appendChild(info);

    const actions = document.createElement('div');
    actions.className = 'vault-entry-actions';
    const mkBtn = (txt, cls, fn) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = txt;
      if (cls) b.className = cls;
      b.addEventListener('click', fn);
      actions.appendChild(b);
      return b;
    };
    mkBtn('复制', 'primary', async () => {
      try {
        const r = await invoke('vault_copy', { id: e.id });
        toast(`已复制，${(r && r.clear_after_secs) || 20} 秒后自动清除`);
      } catch (err) { console.warn(err); toast('复制失败：' + (err.message || err)); }
    });
    mkBtn('编辑', '', () => vaultRenderEditForm(e));
    // 删除：两段式确认（首次点击 2.5s 内再点才真删）
    const del = mkBtn('删除', 'danger', async () => {
      if (!del.dataset.arm) {
        del.dataset.arm = '1';
        del.textContent = '确认删除？';
        setTimeout(() => { if (del.isConnected) { del.dataset.arm = ''; del.textContent = '删除'; } }, 2500);
        return;
      }
      try {
        await invoke('vault_delete', { id: e.id });
        toast('已删除');
        vaultRenderEntries($('vSearch') ? $('vSearch').value.trim() : '');
      } catch (err) { console.warn(err); toast('删除失败：' + (err.message || err)); }
    });
    row.appendChild(actions);
    return row;
  }

  /* ---- 新增/编辑内联表单 ---- */
  function vaultRenderEditForm(entry) {
    const box = $('vEditForm');
    if (!box) return;
    const isNew = !entry;
    const v = (x) => escapeHtml(x == null ? '' : String(x));
    box.innerHTML =
      '<div class="vault-edit-card">' +
      '<div class="vef-title">' + (isNew ? '新增密码条目' : '编辑条目') + '</div>' +
      '<div class="row-2">' +
      '<div class="field"><label for="veSite">站点</label>' +
      `<input id="veSite" type="text" placeholder="如：微博" value="${v(entry && entry.site)}"></div>` +
      '<div class="field"><label for="veAccount">账号</label>' +
      `<input id="veAccount" type="text" placeholder="邮箱 / 用户名" value="${v(entry && entry.account)}"></div>` +
      '</div>' +
      '<div class="field"><label for="vePassword">密码' + (isNew ? '' : '（留空不修改）') + '</label>' +
      '<input id="vePassword" type="text" placeholder="输入密码" autocomplete="new-password"></div>' +
      '<div class="field"><label for="veNote">备注</label>' +
      `<input id="veNote" type="text" placeholder="可选" value="${v(entry && entry.note)}"></div>` +
      '<div class="vault-btn-row">' +
      '<button id="veSave" class="primary" type="button">保存</button>' +
      '<button id="veCancel" type="button">取消</button>' +
      '</div></div>';
    $('veSave').addEventListener('click', async () => {
      const args = {
        site: $('veSite').value,
        account: $('veAccount').value,
        password: $('vePassword').value, // 空串 → 后端视为不修改
        note: $('veNote').value,
        icon: '',
      };
      try {
        if (isNew) {
          if (!args.site.trim()) { toast('站点名称不能为空'); return; }
          await invoke('vault_add', args);
          toast('已保存');
        } else {
          await invoke('vault_update', Object.assign({ id: entry.id }, args));
          toast('已更新');
        }
        box.innerHTML = '';
        vaultRenderEntries($('vSearch') ? $('vSearch').value.trim() : '');
      } catch (e) { console.warn(e); toast('保存失败：' + (e.message || e)); }
    });
    $('veCancel').addEventListener('click', () => { box.innerHTML = ''; });
  }

  /* ---- 导入：file input 读文本 → 内联面板要备份主密码 → vault_import_payload ---- */
  $('vaultFileInput').addEventListener('change', async () => {
    const fi = $('vaultFileInput');
    const f = fi.files && fi.files[0];
    fi.value = ''; // 允许重复选同一文件
    if (!f) return;
    let text = '';
    try { text = await f.text(); }
    catch (e) { toast('读取备份文件失败'); return; }
    const box = $('vEditForm');
    if (!box) return;
    box.innerHTML =
      '<div class="vault-edit-card">' +
      '<div class="vef-title">导入备份：' + escapeHtml(f.name) + '</div>' +
      '<div class="field"><label for="viPwd">备份主密码</label>' +
      '<input id="viPwd" type="password" placeholder="该备份加密时使用的主密码" autocomplete="current-password"></div>' +
      '<div class="vault-btn-row">' +
      '<button id="viOk" class="primary" type="button">确认导入</button>' +
      '<button id="viCancel" type="button">取消</button>' +
      '</div></div>';
    $('viOk').addEventListener('click', async () => {
      try {
        await invoke('vault_import_payload', { payload: text, master_password: $('viPwd').value });
        toast('导入完成');
        box.innerHTML = '';
        await refreshVaultStatus();
      } catch (e) { console.warn(e); toast('导入失败：' + (e.message || e)); }
    });
    $('viCancel').addEventListener('click', () => { box.innerHTML = ''; });
  });

  /* ============================================================
   * 迭代④·联网社交（NETSOCIAL_SPEC.md §8）
   * 调用约定：invoke('netsocial_xxx', args)，命令名/字段以 §7.5 为最终裁决；
   * 浏览器 mock 直连真实本地服务端（tauri-bridge.js 内 fetch + WebSocket），
   * Tauri 真机走 Rust core。
   * ============================================================ */
  const NS_COLORS = ['#E58FAD', '#D17397', '#9B7FB8', '#7FBF9E', '#7FB0D9', '#E8B86D',
    '#E09B8A', '#B8A9E8', '#6FC7B8', '#F2A0C0', '#D9C78A', '#C98AD9'];
  const NS_TAG_PRESETS = {
    personality: ['温柔', '开朗', '安静', '理性', '幽默', '慢热'],
    interests: ['音乐', '读书', '电影', '旅行', '美食', '运动', '画画', '编程'],
    status: ['想聊天', '想散步', '刚下班', '周末有空', '想安静待着'],
  };
  const NS_TAG_GROUPS = [['personality', 'nsTagsPersonality', 'nsCustomPersonality'],
    ['interests', 'nsTagsInterests', 'nsCustomInterests'],
    ['status', 'nsTagsStatus', 'nsCustomStatus']];
  const NS_CITIES = ['北京', '上海', '广州', '深圳', '成都', '杭州', '武汉', '西安', '石家庄'];
  const NS_PRIVACY_LS = 'qibai.console.ns.privacy-accepted';
  const NS_TAGS_LS = 'qibai.console.ns.tags';
  function nsGetLocalTags() {
    try {
      const t = JSON.parse(localStorage.getItem(NS_TAGS_LS) || '{}');
      return { personality: t.personality || [], interests: t.interests || [], status: t.status || [] };
    } catch (e) { return { personality: [], interests: [], status: [] }; }
  }
  function nsSaveLocalTags(tags) {
    try { localStorage.setItem(NS_TAGS_LS, JSON.stringify(tags)); } catch (e) { /* ignore */ }
  }

  const nsRt = {
    connected: false,
    pureMode: false,
    view: 'planet',
    my: null,            // netsocial_status 返回
    currentSoul: null,   // 详情浮层当前 soul（§3 profile 形状）
    chatPeer: null,      // {id, nick, color_idx}
    pollTimer: null,
    booted: false,
  };

  function nsColor(idx) { return NS_COLORS[Math.max(0, Math.min(11, parseInt(idx, 10) || 0))]; }

  function loadSocialTab() {
    if (!nsRt.booted) {
      nsRt.booted = true;
      bindNsStatic();
    }
    refreshNsStatus();
    // 首次进入 → 隐私声明弹窗（§8.1 合规元素）
    if (!localStorage.getItem(NS_PRIVACY_LS)) {
      $('nsPrivacyModal').hidden = false;
    }
    setNsView(nsRt.view); // 进入即按当前子视图加载（离线时列表显示不可用提示）
  }

  async function refreshNsStatus() {
    try {
      const st = await invoke('netsocial_status');
      nsRt.my = st;
      nsRt.connected = !!st.online;
      // 连接状态条
      $('nsConnDot').classList.toggle('online', nsRt.connected);
      $('nsConnText').textContent = nsRt.connected
        ? ('已连接 · ' + (st.server_url || ''))
        : '离线模式 · 配置服务器后启用联网';
      $('btnNsConnect').textContent = nsRt.connected ? '断开' : '连接';
      $('btnNsConnect').classList.toggle('off', nsRt.connected);
      $('nsOfflineHint').classList.toggle('hidden', nsRt.connected);
      // 我的光点
      const c = nsColor(st.color_idx);
      $('nsMyDot').style.background = c;
      $('nsMyDot').style.color = c;
      $('nsMyName').textContent = st.nickname || '星客';
      const tag = $('nsMyOnline');
      tag.textContent = nsRt.connected ? '在线' : '离线';
      tag.classList.toggle('off', !nsRt.connected);
      if (!$('nsServerUrl').value) $('nsServerUrl').value = st.server_url || 'http://127.0.0.1:8787';
      if (st.city) $('nsCityInput').value = st.city;
      renderMyTags();
    } catch (e) {
      console.warn('netsocial_status failed', e);
    }
  }

  // ---- 标签面板：预置多选 + 自定义输入 ----
  function renderMyTags() {
    // §7.5 status 形状不含 soul_tags：UI 层用 localStorage 缓存已选标签
    const selAll = nsGetLocalTags();
    NS_TAG_GROUPS.forEach(([g, boxId]) => {
      const box = $(boxId);
      box.innerHTML = '';
      const sel = selAll[g] || [];
      NS_TAG_PRESETS[g].forEach((t) => {
        box.appendChild(nsChip(t, sel.indexOf(t) >= 0));
      });
      sel.forEach((t) => {
        if (NS_TAG_PRESETS[g].indexOf(t) < 0) box.appendChild(nsChip(t, true));
      });
    });
  }
  function nsChip(tag, on) {
    const c = document.createElement('button');
    c.type = 'button';
    c.className = 'ns-chip' + (on ? ' on' : '');
    c.dataset.tag = tag;
    c.textContent = tag;
    c.onclick = () => c.classList.toggle('on');
    return c;
  }
  function nsCollectTags() {
    const out = { personality: [], interests: [], status: [] };
    NS_TAG_GROUPS.forEach(([g, boxId]) => {
      out[g] = Array.from($(boxId).querySelectorAll('.ns-chip.on'))
        .slice(0, 5).map((c) => c.dataset.tag);
    });
    return out;
  }

  // ---- 子视图切换 ----
  function setNsView(v, skipLoad) {
    nsRt.view = (v === 'city') ? 'city' : 'planet';
    document.querySelectorAll('#pane-social .ns-subtab').forEach((b) =>
      b.classList.toggle('active', b.dataset.view === nsRt.view));
    $('nsViewPlanet').hidden = nsRt.view !== 'planet';
    $('nsViewCity').hidden = nsRt.view !== 'city';
    if (!skipLoad) {
      if (nsRt.view === 'planet') loadRoam();
      else loadCity();
    }
  }

  // ---- 漫游浏览（§3 profile 形状渲染） ----
  async function loadRoam() {
    const box = $('nsRoamList');
    box.innerHTML = '<div class="ns-empty">正在连接星河…</div>';
    try {
      const res = await invoke('netsocial_roam', {});
      renderSoulCards(box, res.souls || []);
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="ns-empty">' + escapeHtml((e && e.message) || '暂不可用') + '</div>';
    }
  }
  async function loadMatch() {
    const box = $('nsRoamList');
    box.innerHTML = '<div class="ns-empty">正在匹配灵魂频率…</div>';
    try {
      const res = await invoke('netsocial_match', { limit: 20 });
      const souls = (res.souls || []).map((s) => s.profile || s);
      renderSoulCards(box, souls);
    } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="ns-empty">' + escapeHtml((e && e.message) || '暂不可用') + '</div>';
    }
  }
  function renderSoulCards(box, souls) {
    box.innerHTML = '';
    if (!souls.length) {
      box.innerHTML = '<div class="ns-empty">附近还没有光点，稍后再来漫游吧～</div>';
      return;
    }
    souls.forEach((s) => {
      const card = document.createElement('div');
      card.className = 'ns-soul-card';
      const dot = document.createElement('span');
      dot.className = 'ns-dot';
      dot.style.background = nsColor(s.color_idx);
      dot.style.color = nsColor(s.color_idx);
      const head = document.createElement('div');
      head.className = 'ns-soul-head';
      const nm = document.createElement('span');
      nm.className = 'ns-soul-name';
      nm.textContent = s.nickname || '星客';
      const onl = document.createElement('span');
      onl.className = s.online ? 'ns-soul-online' : 'ns-soul-offline';
      onl.textContent = s.online ? '在线' : '离线';
      head.appendChild(dot); head.appendChild(nm); head.appendChild(onl);
      card.appendChild(head);
      // 纯净模式：只显示昵称+色（隐藏标签与城市，§8.1）
      if (!nsRt.pureMode) {
        const tags = Object.keys(s.soul_tags || {}).reduce((a, k) => a.concat(s.soul_tags[k] || []), []);
        if (tags.length) {
          const tw = document.createElement('div');
          tw.className = 'ns-soul-tags';
          tags.slice(0, 6).forEach((t) => {
            const sp = document.createElement('span');
            sp.className = 'ns-soul-tag';
            sp.textContent = t;
            tw.appendChild(sp);
          });
          card.appendChild(tw);
        }
        if (s.city) {
          const ci = document.createElement('div');
          ci.className = 'ns-soul-city';
          ci.textContent = '城市：' + s.city;
          card.appendChild(ci);
        }
      }
      card.onclick = () => openNsDetail(s);
      box.appendChild(card);
    });
  }

  // ---- 详情浮层：点亮 / 举报 / 拉黑 / 发消息 ----
  function openNsDetail(soul) {
    nsRt.currentSoul = soul;
    const c = nsColor(soul.color_idx);
    $('nsDetailDot').style.background = c;
    $('nsDetailDot').style.color = c;
    $('nsDetailName').textContent = soul.nickname || '星客';
    const body = $('nsDetailBody');
    body.innerHTML = '';
    if (!nsRt.pureMode) {
      const tags = Object.keys(soul.soul_tags || {}).reduce((a, k) => a.concat(soul.soul_tags[k] || []), []);
      if (tags.length) {
        const tw = document.createElement('div');
        tw.className = 'ns-soul-tags';
        tags.forEach((t) => {
          const sp = document.createElement('span');
          sp.className = 'ns-soul-tag';
          sp.textContent = t;
          tw.appendChild(sp);
        });
        body.appendChild(tw);
      }
      if (soul.city) {
        const ci = document.createElement('div');
        ci.className = 'ns-soul-city';
        ci.textContent = '城市：' + soul.city;
        body.appendChild(ci);
      }
    }
    const st = document.createElement('div');
    st.className = 'ns-soul-city';
    st.textContent = soul.online ? '该光点现在在线' : '该光点暂时离线';
    body.appendChild(st);
    $('nsSoulModal').hidden = false;
  }

  async function nsDoLike() {
    const s = nsRt.currentSoul;
    if (!s) return;
    try {
      const r = await invoke('netsocial_like', { id: s.anon_id });
      if (r.matched || r.mutual) toast('互相点亮！你们已经彼此照亮 ✨');
      else toast('已点亮该光点');
    } catch (e) { console.warn(e); toast((e && e.message) || '点亮失败'); }
  }
  async function nsDoReport() {
    const s = nsRt.currentSoul;
    if (!s) return;
    if (!confirm('确认举报该光点？我们会记录并复核。')) return;
    try {
      await invoke('netsocial_report', { id: s.anon_id, reason: '用户主动举报' });
      toast('已收到举报，感谢守护社区');
      $('nsSoulModal').hidden = true;
    } catch (e) { console.warn(e); toast((e && e.message) || '举报失败'); }
  }
  async function nsDoBlock() {
    const s = nsRt.currentSoul;
    if (!s) return;
    if (!confirm('拉黑后双方将互相不可见且无法收发消息，确认？')) return;
    try {
      await invoke('netsocial_block', { id: s.anon_id });
      toast('已拉黑该光点');
      $('nsSoulModal').hidden = true;
      if (nsRt.view === 'city') loadCity(); else loadRoam();
    } catch (e) { console.warn(e); toast((e && e.message) || '操作失败'); }
  }

  // ---- 匿名聊天窗口 ----
  function openNsChat(soul) {
    nsRt.chatPeer = { id: soul.anon_id, nick: soul.nickname || '星客', color_idx: soul.color_idx };
    const c = nsColor(soul.color_idx);
    $('nsChatDot').style.background = c;
    $('nsChatDot').style.color = c;
    $('nsChatName').textContent = nsRt.chatPeer.nick;
    $('nsChatScroll').innerHTML = '';
    $('nsSoulModal').hidden = true;
    $('nsChatModal').hidden = false;
    nsStartPoll();
    nsLoadHistory();
  }
  async function nsLoadHistory() {
    if (!nsRt.chatPeer) return;
    try {
      const r = await invoke('netsocial_history', { id: nsRt.chatPeer.id });
      $('nsChatScroll').innerHTML = '';
      (r.messages || []).forEach((m) => nsAppendBubble(m));
    } catch (e) { console.warn(e); }
  }
  function nsAppendBubble(m) {
    const scroll = $('nsChatScroll');
    const row = document.createElement('div');
    row.className = 'ns-msg-row ' + (m.dir === 'out' ? 'me' : 'peer');
    if (m.dir !== 'out') {
      const from = document.createElement('div');
      from.className = 'ns-bubble-from';
      const dot = document.createElement('span');
      dot.className = 'ns-dot';
      dot.style.background = nsColor(m.peer_color != null ? m.peer_color : nsRt.chatPeer.color_idx);
      dot.style.color = dot.style.background;
      const nm = document.createElement('span');
      nm.textContent = m.peer_nick || nsRt.chatPeer.nick;
      from.appendChild(dot); from.appendChild(nm);
      row.appendChild(from);
    }
    const b = document.createElement('div');
    b.className = 'ns-bubble';
    b.textContent = String(m.text == null ? '' : m.text); // 一律 textContent，不渲染 HTML
    row.appendChild(b);
    scroll.appendChild(row);
    scroll.scrollTop = scroll.scrollHeight;
  }
  function nsStartPoll() {
    if (nsRt.pollTimer) clearInterval(nsRt.pollTimer);
    nsRt.pollTimer = setInterval(nsPollOnce, 2000); // §8.1：2s 轮询
  }
  async function nsPollOnce() {
    if (document.hidden) return;
    try {
      const r = await invoke('netsocial_poll', {});
      (r.events || []).forEach((ev) => {
        if (ev.type === 'msg') {
          // 聊天窗打开即渲染新消息；in 方向按 昵称+光点色 对端匹配，防串窗
          if (nsRt.chatPeer && !$('nsChatModal').hidden) {
            const p = ev.payload || {};
            const isMine = p.dir === 'out';
            const isPeer = p.dir !== 'out'
              && p.peer_nick === nsRt.chatPeer.nick
              && nsColor(p.peer_color) === nsColor(nsRt.chatPeer.color_idx);
            if (isMine || isPeer) nsAppendBubble(p);
          }
        } else if (ev.type === 'error') {
          toast(ev.payload.message || '消息未能送达');
        } else if (ev.type === 'disconnected') {
          refreshNsStatus();
        } else if (ev.type === 'connected') {
          refreshNsStatus();
        }
      });
    } catch (e) { console.warn(e); }
  }
  async function nsSendChat() {
    if (!nsRt.chatPeer) return;
    const input = $('nsChatInput');
    const text = (input.value || '').trim();
    if (!text) return;
    try {
      await invoke('netsocial_send', { id: nsRt.chatPeer.id, text });
      nsAppendBubble({ dir: 'out', text });
      input.value = '';
    } catch (e) {
      console.warn(e);
      toast((e && e.message) || '发送失败');
    }
  }

  // ---- 同城灯塔 ----
  async function loadCity() {
    const box = $('nsCityList');
    box.innerHTML = '';
    const city = ($('nsCityInput').value || '').trim();
    if (city) {
      box.innerHTML = '<div class="ns-empty">正在照亮同城灵魂…</div>';
      try {
        const res = await invoke('netsocial_city_souls', { city });
        renderSoulCards(box, res.souls || []);
        // 保存城市到 profile
        await invoke('netsocial_set_profile', { city }).catch(() => {});
      } catch (e) {
        console.warn(e);
        box.innerHTML = '<div class="ns-empty">' + escapeHtml((e && e.message) || '暂不可用') + '</div>';
      }
    } else {
      box.innerHTML = '<div class="ns-empty">填写城市或点选快捷城市后，看看同城灵魂</div>';
    }
    // 话题墙 / 活动预告占位卡片（§8.1）
    nsLoadTopics();
    nsLoadEvents();
  }
  async function nsLoadTopics() {
    const box = $('nsTopicsList');
    box.innerHTML = '<div class="ns-empty">加载中…</div>';
    try {
      const res = await invoke('netsocial_topics', {});
      box.innerHTML = '';
      (res.topics || []).forEach((t) => {
        const card = document.createElement('div');
        card.className = 'ns-ph-card';
        const ti = document.createElement('div');
        ti.className = 'ns-ph-title'; ti.textContent = t.title || '话题';
        const badge = document.createElement('span');
        badge.className = 'ns-ph-badge'; badge.textContent = '话题墙 · 即将上线';
        card.appendChild(ti); card.appendChild(badge);
        box.appendChild(card);
      });
      if (!(res.topics || []).length) box.innerHTML = '<div class="ns-empty">暂无话题</div>';
    } catch (e) {
      box.innerHTML = '<div class="ns-empty">话题墙暂不可用</div>';
    }
  }
  async function nsLoadEvents() {
    const box = $('nsEventsList');
    box.innerHTML = '<div class="ns-empty">加载中…</div>';
    try {
      const res = await invoke('netsocial_events', {});
      box.innerHTML = '';
      (res.events || []).forEach((ev) => {
        const card = document.createElement('div');
        card.className = 'ns-ph-card';
        const ti = document.createElement('div');
        ti.className = 'ns-ph-title'; ti.textContent = ev.title || '活动';
        const meta = document.createElement('div');
        meta.className = 'ns-ph-meta';
        meta.textContent = [ev.when, ev.where].filter(Boolean).join(' · ');
        const badge = document.createElement('span');
        badge.className = 'ns-ph-badge'; badge.textContent = '活动预告 · 即将上线';
        card.appendChild(ti); card.appendChild(meta); card.appendChild(badge);
        box.appendChild(card);
      });
      if (!(res.events || []).length) box.innerHTML = '<div class="ns-empty">暂无活动</div>';
    } catch (e) {
      box.innerHTML = '<div class="ns-empty">活动预告暂不可用</div>';
    }
  }

  // ---- 静态事件绑定（仅绑一次） ----
  function bindNsStatic() {
    // 连接 / 断开
    $('btnNsConnect').addEventListener('click', async () => {
      try {
        if (nsRt.connected) {
          await invoke('netsocial_disconnect', {});
          toast('已断开连接');
        } else {
          await invoke('netsocial_connect', { server_url: $('nsServerUrl').value.trim() });
          toast('已连接到星河服务端');
        }
      } catch (e) {
        console.warn(e);
        toast((e && e.message) || '连接失败');
      }
      refreshNsStatus();
    });
    // 子视图
    document.querySelectorAll('#pane-social .ns-subtab').forEach((b) => {
      b.addEventListener('click', () => setNsView(b.dataset.view));
    });
    // 自定义标签：Enter 追加为已选 chip
    NS_TAG_GROUPS.forEach(([g, , inputId]) => {
      $(inputId).addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          const v = ($(inputId).value || '').trim().slice(0, 8);
          if (!v) return;
          const box = $('nsTags' + g[0].toUpperCase() + g.slice(1));
          if (!Array.from(box.querySelectorAll('.ns-chip')).some((c) => c.dataset.tag === v)) {
            box.appendChild(nsChip(v, true));
          }
          $(inputId).value = '';
        }
      });
    });
    // 保存我的光点（灵魂标签）
    $('btnNsSaveProfile').addEventListener('click', async () => {
      const tags = nsCollectTags();
      nsSaveLocalTags(tags);
      try {
        await invoke('netsocial_set_profile', { soul_tags: tags });
        toast('我的光点已保存');
      } catch (e) { console.warn(e); toast((e && e.message) || '保存失败'); }
    });
    // 漫游 / 匹配
    $('btnNsRoam').addEventListener('click', loadRoam);
    $('btnNsMatch').addEventListener('click', loadMatch);
    // 详情浮层
    $('nsDetailClose').addEventListener('click', () => { $('nsSoulModal').hidden = true; });
    $('btnNsLike').addEventListener('click', nsDoLike);
    $('btnNsReport').addEventListener('click', nsDoReport);
    $('btnNsBlock').addEventListener('click', nsDoBlock);
    $('btnNsMessage').addEventListener('click', () => {
      if (nsRt.currentSoul) openNsChat(nsRt.currentSoul);
    });
    // 聊天窗
    $('nsChatClose').addEventListener('click', () => { $('nsChatModal').hidden = true; nsRt.chatPeer = null; });
    $('btnNsSend').addEventListener('click', nsSendChat);
    $('nsChatInput').addEventListener('keydown', (e) => { if (e.key === 'Enter') nsSendChat(); });
    // 同城快捷城市
    const cityBox = $('nsCityChips');
    NS_CITIES.forEach((c) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'ns-chip';
      chip.textContent = c;
      chip.onclick = () => { $('nsCityInput').value = c; };
      cityBox.appendChild(chip);
    });
    $('btnNsCityGo').addEventListener('click', loadCity);
    // 纯净模式开关
    bindSwitch($('swNsPure'), false, (on) => { nsRt.pureMode = on; });
    // 隐私弹窗：年龄声明确认
    $('btnNsPrivacyOk').addEventListener('click', async () => {
      nsRt.pureMode = $('swNsPure').dataset.on === 'true';
      try {
        await invoke('netsocial_register', { age_declared: '18plus', pure_mode: nsRt.pureMode });
      } catch (e) { console.warn(e); }
      localStorage.setItem(NS_PRIVACY_LS, '1');
      $('nsPrivacyModal').hidden = true;
      toast('欢迎进入星球广场，匿名默认已开启');
    });
    $('btnNsPrivacyMinor').addEventListener('click', async () => {
      nsRt.pureMode = true;
      $('swNsPure').classList.add('on');
      $('swNsPure').dataset.on = 'true';
      try {
        await invoke('netsocial_register', { age_declared: 'minor', pure_mode: true });
      } catch (e) { console.warn(e); }
      localStorage.setItem(NS_PRIVACY_LS, '1');
      $('nsPrivacyModal').hidden = true;
      toast('已开启未成年人纯净模式：列表仅显示昵称与光点色');
    });
  }

  /* ============================================================
   * 事件订阅（qibai://*）
   * ============================================================ */
  function evtPayload(event) { return (event && event.payload) || {}; }

  // qibai://desk-changed → 刷新角色 tab 在场状态与客人选择器
  listen('qibai://desk-changed', () => {
    refreshPersonaSel();
    if (currentTab === 'roles') loadRoles();
    if (currentTab === 'pk') loadPkTab();
  });

  // qibai://skill-approval → 全页审批弹窗
  let currentRequestId = null;
  listen('qibai://skill-approval', (event) => {
    const p = evtPayload(event);
    currentRequestId = p.request_id;
    $('approvalSkill').textContent = p.skill || 'unknown';
    $('approvalDesc').textContent = p.description || '元气小小想执行一个需要确认的操作。';
    $('approvalModal').classList.add('show');
  });

  async function resolveApproval(approve) {
    // v45.7：先 await 结果，成功才关弹窗；失败保留弹窗并把 err 显示在弹窗内。
    let errEl = document.getElementById('approvalErr');
    if (!errEl) {
      errEl = document.createElement('div');
      errEl.id = 'approvalErr';
      errEl.style.cssText = 'color:#b71c1c;font-size:12px;margin:8px 0;padding:6px 8px;background:#fdecea;border-radius:4px;white-space:pre-wrap;';
      document.getElementById('approvalModal').appendChild(errEl);
    }
    errEl.hidden = true;
    try {
      await invoke('approve_skill', { request_id: currentRequestId, approve });
      $('approvalModal').classList.remove('show');
      currentRequestId = null;
    } catch (e) {
      const msg = String((e && e.message) || e || '');
      toast('审批失败：' + msg.slice(0, 120));
      errEl.textContent = msg || String(e);
      errEl.hidden = false;
      // 不 currentRequestId = null，保留便于重试
    }
  }
  $('approvalAllow').addEventListener('click', () => resolveApproval(true));
  $('approvalDeny').addEventListener('click', () => resolveApproval(false));

  // qibai://reminder → toast
  listen('qibai://reminder', (event) => {
    const p = evtPayload(event);
    const tag = REMINDER_TAGS[p.type] || '小提醒';
    toast(`${tag}：${p.message || ''}`);
  });

  // qibai://toast → toast
  listen('qibai://toast', (event) => {
    const p = evtPayload(event);
    if (p.text) toast(p.text);
  });

  // qibai://llm-status → 聊天 thinking 指示
  listen('qibai://llm-status', (event) => {
    const p = evtPayload(event);
    if (p.state === 'thinking') {
      $('thinkingTip').classList.add('on');
      $('llmStatusLine').textContent = 'LLM 状态：思考中…';
    } else if (p.state === 'error') {
      $('thinkingTip').classList.remove('on');
      $('llmStatusLine').textContent = 'LLM 状态：出错了';
    } else {
      $('thinkingTip').classList.remove('on');
      $('llmStatusLine').textContent = 'LLM 状态：空闲';
      // v20：done 不再单独刷新聊天——conversation-updated 是唯一刷新来源，
      // 避免与它并发产生两个在途 loadConversation（旧快照覆盖新渲染的根因）。
    }
  });

  // qibai://pk-round → 同步结果区（对局可能由桌宠窗/其他侧发起）
  listen('qibai://pk-round', (event) => {
    const p = evtPayload(event);
    if (!p || !p.a) return;
    $('kMainName').textContent = (PERSONA_NAMES[p.a] || p.a) + '（你）';
    $('kGuestName').textContent = PERSONA_NAMES[p.b] || p.b || '客人';
    $('kMainHand').innerHTML = MOVE_SVG[p.move_a] || MOVE_SVG.rock;
    $('kGuestHand').innerHTML = MOVE_SVG[p.move_b] || MOVE_SVG.rock;
    if (p.result === 'draw') {
      $('kResult').textContent = '平局！心有灵犀～';
    } else if (p.winner) {
      $('kResult').textContent = `${PERSONA_NAMES[p.winner] || p.winner} 赢啦！`;
    }
    if (Array.isArray(p.lines)) renderLines(p.lines);
  });

  // qibai://intimacy-updated → 升级动画 + 解锁 banner + 亲密度页刷新
  listen('qibai://intimacy-updated', (event) => {
    const p = evtPayload(event);
    if (!p) return;
    const unlocked = Array.isArray(p.unlocked) ? p.unlocked : [];
    if (p.leveled_up) triggerUpgradeAnim();
    if (p.leveled_up || unlocked.length) showIntiBanner(p);
    if (p.persona_id === intiPersona && currentTab === 'intimacy') loadIntiState();
    // 跨页联动：升级可能刚解锁 assistant_skills(L4)/anniversary(L7)，重新校验
    if (p.leveled_up) { refreshSkillsGate(); refreshAnnivGate(); }
    // 迭代③：亲密度升级到 L4 后 vault gated 横幅自动解除；正在保险箱页则立即刷新状态
    if (currentTab === 'vault') refreshVaultStatus();
  });

  /* ============================================================
   * 技能与专家 tab（PLUGIN_IMPL_CONTRACT §5）
   *   段①内置技能：get_skills_status（总开关 + 4 个旧开关 live 绑定；
   *     open_social / execute_command 为只读说明行）
   *   段②插件技能：plugin_list(type=skill, builtin=false)
   *   段③专家：expert_list + expert_set_active
   * ============================================================ */
  const BUILTIN_ROWS = [
    { fn: 'launch_app',     title: '打开应用（launch_app）',      desc: '元气小小可帮你启动常用程序',             live: true,  key: 'launch_app' },
    { fn: 'open_folder',    title: '打开文件夹（open_folder）',   desc: '元气小小可帮你打开本地目录',             live: true,  key: 'open_folder' },
    { fn: 'set_reminder',   title: '设置提醒（set_reminder）',   desc: '元气小小可创建新的定时提醒',             live: true,  key: 'set_reminder' },
    { fn: 'check_code',     title: '代码检查（check_code）',     desc: '只读扫描本地代码片段',               live: true,  key: 'check_code' },
    { fn: 'open_social',    title: '星球广场（open_social）',    desc: '只读网络技能，由聊天指令触发跳转',   live: false, note: '只读 · 无需授权' },
    { fn: 'execute_command', title: '执行命令（execute_command）', desc: '危险命令，每次执行前逐次确认',       live: false, note: '逐次批准' },
  ];

  function loadPluginsTab() {
    refreshBuiltinSkills();
    refreshPluginList();
    refreshExpertList();
    refreshConnectorList();
    refreshSkillPackList();
    bindConnectorForm();
    bindPackInstall();
    bindSkillInstall();
  }

  // 段①：内置技能（数据/开关语义与设置页 skillsCard 一致）
  async function refreshBuiltinSkills() {
    let sk = { total_enabled: true, skills: {} };
    try { sk = await invoke('get_skills_status'); } catch (e) { console.warn(e); }
    bindSwitch($('pbSwTotal'), !!sk.total_enabled,
      (v) => invoke('set_skill_enabled', { skill: 'total', enabled: v }).catch(console.warn));
    const box = $('pbBuiltinRows');
    box.innerHTML = '';
    BUILTIN_ROWS.forEach((r) => {
      const row = document.createElement('div');
      row.className = 'toggle-row';
      const lw = document.createElement('div');
      lw.className = 'label-wrap';
      const t = document.createElement('div');
      t.className = 'title'; t.textContent = r.title;
      const d = document.createElement('div');
      d.className = 'desc'; d.textContent = r.desc;
      lw.appendChild(t); lw.appendChild(d);
      row.appendChild(lw);
      if (r.live) {
        const sw = document.createElement('div');
        sw.className = 'switch';
        bindSwitch(sw, !!(sk.skills && sk.skills[r.key]),
          (v) => invoke('set_skill_enabled', { skill: r.key, enabled: v }).catch(console.warn));
        row.appendChild(sw);
      } else {
        const tag = document.createElement('span');
        tag.className = 'plg-chip granted';
        tag.textContent = r.note;
        row.appendChild(tag);
      }
      box.appendChild(row);
    });

    // v45.8：动态渲染文档型技能（kind=doc，core 线返回 kind/source/doc_path/load_error）
    if (sk.skills && typeof sk.skills === 'object') {
      Object.entries(sk.skills).forEach(([name, info]) => {
        // 跳过已在 BUILTIN_ROWS 硬编码的 tool 技能（只渲染 doc 或新条目）
        if (BUILTIN_ROWS.some((r) => r.key === name)) return;
        if (info && info.kind !== 'doc' && !info.source && !info.doc_path && !info.load_error) return;
        const row = document.createElement('div');
        row.className = 'toggle-row';
        const lw = document.createElement('div');
        lw.className = 'label-wrap';
        const t = document.createElement('div');
        t.className = 'title';
        // 名称 + 文档技能徽标
        t.textContent = name;
        if (info && info.kind === 'doc') {
          const badge = document.createElement('span');
          badge.className = 'plg-chip granted';
          badge.style.cssText = 'margin-left:6px;font-size:11px;';
          badge.textContent = '文档技能';
          t.appendChild(badge);
        }
        const d = document.createElement('div');
        d.className = 'desc';
        d.textContent = (info && info.desc) || (info && info.description) || '文档型技能（SKILL.md）';
        if (info && info.source === 'user') {
          const srcPath = info.doc_path || ('~/.qibai_skills/' + name + '/SKILL.md');
          d.textContent += '  · 来源：用户 (' + srcPath + ')';
        } else if (info && info.source === 'builtin') {
          d.textContent += '  · 来源：内置';
        }
        lw.appendChild(t); lw.appendChild(d);
        // v45.8：加载失败红字
        if (info && info.load_error) {
          const err = document.createElement('div');
          err.style.cssText = 'color:#b71c1c;font-size:12px;margin-top:2px;';
          err.textContent = '加载失败：' + (info.load_error || name);
          lw.appendChild(err);
        }
        row.appendChild(lw);
        const sw = document.createElement('div');
        sw.className = 'switch';
        bindSwitch(sw, !!info && info.enabled !== false,
          (v) => invoke('set_skill_enabled', { skill: name, enabled: v }).catch(console.warn));
        row.appendChild(sw);
        box.appendChild(row);
      });
    }
  }

  // 段②：插件技能卡片
  async function refreshPluginList() {
    const box = $('pbPluginList');
    box.innerHTML = '<div class="plg-empty">加载中…</div>';
    let list = [];
    try { list = await invoke('plugin_list'); } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="plg-empty">插件列表加载失败</div>';
      return;
    }
    const exts = (list || []).filter((p) => p.type === 'skill' && !p.builtin);
    box.innerHTML = '';
    if (!exts.length) {
      box.innerHTML = '<div class="plg-empty">还没有安装外部插件。点右上角「安装插件」添加示例插件。</div>';
      return;
    }
    exts.forEach((p) => box.appendChild(renderPluginCard(p)));
  }

  function renderPluginCard(p) {
    const card = document.createElement('div');
    card.className = 'plg-card' + (p.error ? ' plg-error' : '');

    const head = document.createElement('div');
    head.className = 'plg-head';
    const nm = document.createElement('span');
    nm.className = 'plg-name'; nm.textContent = p.name || p.id;
    head.appendChild(nm);
    if (p.version && p.version !== '—') {
      const v = document.createElement('span');
      v.className = 'plg-ver'; v.textContent = 'v' + p.version;
      head.appendChild(v);
    }
    const st = document.createElement('span');
    st.className = 'plg-state' + (p.installed && p.enabled ? ' on' : '');
    st.textContent = p.error ? '损坏' : p.installed ? (p.enabled ? '已启用' : '已安装 · 停用') : '未安装';
    head.appendChild(st);
    card.appendChild(head);

    if (p.description) {
      const d = document.createElement('div');
      d.className = 'plg-desc'; d.textContent = p.description;
      card.appendChild(d);
    }

    const meta = [];
    if (p.author) meta.push(p.author);
    if (p.functions && p.functions.length) {
      meta.push('函数：' + p.functions.map((f) => f.name).join('、'));
    }
    if (meta.length) {
      const m = document.createElement('div');
      m.className = 'plg-meta-line'; m.textContent = meta.join(' · ');
      card.appendChild(m);
    }

    if (p.error) {
      const e = document.createElement('div');
      e.className = 'plg-error-text'; e.textContent = p.error;
      card.appendChild(e);
      return card; // 坏 manifest：无授权/启停/卸载操作区
    }

    // 权限：已授权徽章 + 未授权「授权/拒绝」操作
    const perms = p.permissions || [];
    if (perms.length) {
      const row = document.createElement('div');
      row.className = 'plg-perms';
      perms.forEach((perm) => {
        const granted = (p.granted || []).includes(perm);
        const chip = document.createElement('span');
        chip.className = 'plg-chip' + (granted ? ' granted' : '');
        chip.textContent = perm + (granted ? ' · 已授权' : ' · 待授权');
        row.appendChild(chip);
        if (!granted) {
          const acts = document.createElement('div');
          acts.className = 'plg-perm-actions';
          const allow = document.createElement('button');
          allow.type = 'button'; allow.className = 'allow'; allow.textContent = '授权';
          allow.addEventListener('click', async () => {
            allow.disabled = true;
            try {
              await invoke('plugin_grant_permission', { id: p.id, permission: perm, approve: true });
              toast(`已授权 ${p.name}：${perm}`);
            } catch (e) {
              console.warn(e);
              toast('授权被拒绝：' + ((e && e.message) || ''));
            }
            refreshPluginList();
          });
          const deny = document.createElement('button');
          deny.type = 'button'; deny.className = 'deny'; deny.textContent = '拒绝';
          deny.addEventListener('click', async () => {
            try {
              await invoke('plugin_grant_permission', { id: p.id, permission: perm, approve: false });
              toast(`已拒绝 ${p.name} 的 ${perm} 权限`);
            } catch (e) { console.warn(e); }
            refreshPluginList();
          });
          acts.appendChild(allow); acts.appendChild(deny);
          row.appendChild(acts);
        }
      });
      card.appendChild(row);
    }

    // 底部：启停开关 + 卸载
    const foot = document.createElement('div');
    foot.className = 'plg-foot';
    const lw = document.createElement('div');
    lw.className = 'label-wrap';
    const t = document.createElement('div'); t.className = 'title'; t.textContent = '启用插件';
    const d = document.createElement('div'); d.className = 'desc'; d.textContent = p.id;
    lw.appendChild(t); lw.appendChild(d);
    foot.appendChild(lw);
    const sw = document.createElement('div');
    sw.className = 'switch';
    bindSwitch(sw, !!p.enabled, (v) => {
      invoke('plugin_set_enabled', { id: p.id, enabled: v })
        .then(() => toast(v ? `已启用 ${p.name}` : `已停用 ${p.name}`))
        .catch((e) => { console.warn(e); refreshPluginList(); });
    });
    foot.appendChild(sw);
    const un = document.createElement('button');
    un.type = 'button'; un.className = 'ghost danger-ghost'; un.textContent = '卸载';
    un.addEventListener('click', async () => {
      try {
        await invoke('plugin_uninstall', { id: p.id });
        toast(`已卸载 ${p.name}`);
      } catch (e) { console.warn(e); }
      refreshPluginList(); refreshExpertList();
    });
    foot.appendChild(un);
    card.appendChild(foot);
    return card;
  }

  // 段③：专家卡片（system_prompt 摘要 / 白名单 chips / persona_binding / 启用·停用）
  async function refreshExpertList() {
    const box = $('pbExpertList');
    box.innerHTML = '<div class="plg-empty">加载中…</div>';
    let list = [];
    try { list = await invoke('expert_list'); } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="plg-empty">专家列表加载失败</div>';
      return;
    }
    box.innerHTML = '';
    if (!list.length) {
      box.innerHTML = '<div class="plg-empty">尚未安装专家。安装插件后可在此启用。</div>';
      return;
    }
    list.forEach((ex) => {
      const card = document.createElement('div');
      card.className = 'plg-card' + (ex.active ? ' plg-active' : '');

      const head = document.createElement('div');
      head.className = 'plg-head';
      const nm = document.createElement('span'); nm.className = 'plg-name';
      nm.textContent = ex.name || ex.id;
      head.appendChild(nm);
      if (ex.version) {
        const v = document.createElement('span'); v.className = 'plg-ver';
        v.textContent = 'v' + ex.version;
        head.appendChild(v);
      }
      const st = document.createElement('span');
      st.className = 'plg-state' + (ex.active ? ' on' : '');
      st.textContent = ex.active ? '使用中' : '未启用';
      head.appendChild(st);
      card.appendChild(head);

      if (ex.description) {
        const d = document.createElement('div'); d.className = 'plg-desc';
        d.textContent = ex.description;
        card.appendChild(d);
      }
      if (ex.system_prompt) {
        const sp = document.createElement('div'); sp.className = 'plg-sys';
        const summary = ex.system_prompt.length > 90
          ? ex.system_prompt.slice(0, 90) + '…' : ex.system_prompt;
        sp.textContent = '人设：' + summary;
        card.appendChild(sp);
      }
      if (ex.skills && ex.skills.length) {
        const chips = document.createElement('div'); chips.className = 'plg-chips';
        ex.skills.forEach((s) => {
          const c = document.createElement('span'); c.className = 'plg-chip';
          c.textContent = s;
          chips.appendChild(c);
        });
        card.appendChild(chips);
      }
      if (ex.persona_binding) {
        const m = document.createElement('div'); m.className = 'plg-meta-line';
        const parts = [];
        if (ex.persona_binding.voice) parts.push('音色：' + ex.persona_binding.voice);
        if (ex.persona_binding.mood) parts.push('语气：' + ex.persona_binding.mood);
        if (parts.length) m.textContent = parts.join(' · ');
        card.appendChild(m);
      }

      const foot = document.createElement('div');
      foot.className = 'plg-foot';
      const lw = document.createElement('div'); lw.className = 'label-wrap';
      const t = document.createElement('div'); t.className = 'title';
      t.textContent = ex.active ? '当前启用的专家' : '未启用';
      const d = document.createElement('div'); d.className = 'desc'; d.textContent = ex.id;
      lw.appendChild(t); lw.appendChild(d);
      foot.appendChild(lw);
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = ex.active ? 'ghost' : 'primary';
      btn.textContent = ex.active ? '停用' : '启用';
      btn.addEventListener('click', async () => {
        try {
          await invoke('expert_set_active', { id: ex.active ? '' : ex.id });
          toast(ex.active ? '已停用专家，恢复默认人设' : `已启用专家 ${ex.name}`);
        } catch (e) { console.warn(e); }
        refreshExpertList();
      });
      foot.appendChild(btn);
      card.appendChild(foot);
      box.appendChild(card);
    });
  }

  // 安装插件入口：打开文件选择（mock 中模拟安装示例插件包）
  $('btnPluginInstall').addEventListener('click', () => $('pluginFileInput').click());
  $('pluginFileInput').addEventListener('change', async () => {
    $('pluginFileInput').value = ''; // 允许重复选择同名文件
    try {
      await invoke('plugin_install', { dir: 'demo://examples' });
      toast('示例插件已安装：日期时间（技能）、科学家助手（专家）');
    } catch (e) {
      console.warn(e);
      toast('安装失败：' + ((e && e.message) || ''));
    }
    refreshPluginList();
    refreshExpertList();
  });

  // ===== 段②b：连接器（MCP/HTTP）面板 =====
  function bindConnectorForm() {
    const kindSel = $('connKind');
    const toggle = () => {
      const isHttp = kindSel.value === 'http';
      $('connHttpFields').style.display = isHttp ? '' : 'none';
      $('connMcpFields').style.display = isHttp ? 'none' : '';
    };
    kindSel.addEventListener('change', toggle);
    $('btnConnAdd').addEventListener('click', async () => {
      const kind = kindSel.value;
      const id = $('connId').value.trim();
      const name = $('connName').value.trim() || id;
      const desc = $('connDesc').value.trim();
      const enabled = true;
      let args = {};
      if (kind === 'mcp') {
        const command = $('connCommand').value.trim();
        const argList = $('connArgs').value.trim().split(/\s+/).filter(Boolean);
        if (!command) { toast('请填写 MCP 启动命令'); return; }
        args = { id, name, kind, command, args: argList, description: desc, enabled };
      } else {
        const url = $('connUrl').value.trim();
        if (!url) { toast('请填写 HTTP 端点 URL'); return; }
        args = { id, name, kind, command: '', args: [], url, description: desc, enabled };
      }
      if (!/^[a-zA-Z0-9_]+$/.test(id)) { toast('ID 仅支持字母、数字、下划线'); return; }
      try {
        await invoke('connector_add', args);
        toast('连接器已添加并自动授权');
      } catch (e) {
        console.warn(e);
        toast('添加失败：' + ((e && e.message) || ''));
      }
      ['connId', 'connName', 'connDesc', 'connCommand', 'connArgs', 'connUrl'].forEach((k) => {
        const el = $(k); if (el) el.value = '';
      });
      refreshConnectorList();
    });
  }

  async function refreshConnectorList() {
    const box = $('pbConnectorList');
    box.innerHTML = '<div class="plg-empty">加载中…</div>';
    let list = [];
    try { list = await invoke('connector_list'); } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="plg-empty">连接器列表加载失败</div>';
      return;
    }
    box.innerHTML = '';
    if (!list.length) {
      box.innerHTML = '<div class="plg-empty">还没有连接器。上方填写 MCP 或 HTTP 配置即可添加。</div>';
      return;
    }
    list.forEach((c) => {
      const card = document.createElement('div');
      card.className = 'plg-card' + (c.enabled ? ' plg-active' : '');
      const head = document.createElement('div');
      head.className = 'plg-head';
      const nm = document.createElement('span'); nm.className = 'plg-name';
      nm.textContent = c.name || c.id;
      const ver = document.createElement('span'); ver.className = 'plg-ver';
      ver.textContent = (c.entry_kind === 'mcp' ? 'MCP' : 'HTTP') + ' · v' + (c.version || '1.0.0');
      const st = document.createElement('span'); st.className = 'plg-state ' + (c.enabled ? 'on' : '');
      st.textContent = c.enabled ? '已启用' : '已停用';
      head.appendChild(nm); head.appendChild(ver); head.appendChild(st);
      card.appendChild(head);
      const d = document.createElement('div'); d.className = 'plg-desc';
      d.textContent = c.description || (c.entry_kind === 'mcp' ? 'MCP stdio 连接器' : 'HTTP 端点连接器');
      card.appendChild(d);
      const foot = document.createElement('div'); foot.className = 'plg-foot';
      const chip = document.createElement('span'); chip.className = 'plg-chip granted';
      chip.textContent = (c.granted || []).join('、') || '无授权';
      const rm = document.createElement('button'); rm.className = 'danger-ghost'; rm.type = 'button';
      rm.textContent = '移除';
      rm.addEventListener('click', async () => {
        try {
          await invoke('connector_remove', { id: c.id });
          toast('连接器已移除');
        } catch (e) {
          console.warn(e);
          toast('移除失败：' + ((e && e.message) || ''));
        }
        refreshConnectorList();
      });
      foot.appendChild(chip); foot.appendChild(rm);
      card.appendChild(foot);
      box.appendChild(card);
    });
  }

  // ===== 段②c：技能包（.qbpk）面板 =====
  function bindPackInstall() {
    $('btnPackInstall').addEventListener('click', () => $('packFileInput').click());
    $('packFileInput').addEventListener('change', async () => {
      const file = $('packFileInput').files && $('packFileInput').files[0];
      $('packFileInput').value = '';
      if (!file) return;
      if (!/\.qbpk$/i.test(file.name)) { toast('请选择 .qbpk 技能包文件'); return; }
      try {
        await invoke('skill_pack_install', { zip_path: 'demo://' + file.name });
        toast('技能包已安装：' + file.name);
      } catch (e) {
        console.warn(e);
        toast('安装失败：' + ((e && e.message) || ''));
      }
      refreshPluginList();
      refreshSkillPackList();
    });
  }

  // v45.8：三段式安装（stage → 确认摘要 → commit / 取消）
  function bindSkillInstall() {
    const urlBtn = $('btnSkillInstallUrl');
    const fileBtn = $('btnSkillInstallFile');
    const dirBtn = $('btnSkillOpenDir');
    const urlInput = $('skillUrlInput');
    const fileInput = $('skillFileInput');
    if (!urlBtn) return;

    urlBtn.addEventListener('click', () => {
      urlInput.style.display = urlInput.style.display === 'none' ? 'block' : 'none';
      if (urlInput.style.display === 'block') urlInput.focus();
    });
    urlInput.addEventListener('keydown', async (e) => {
      if (e.key !== 'Enter') return;
      e.preventDefault(); e.stopPropagation();
      const url = urlInput.value.trim();
      if (!url) return;
      urlInput.style.display = 'none';
      await stageAndConfirm({ source_url: url });
    });
    fileBtn.addEventListener('click', () => fileInput.click());
    fileInput.addEventListener('change', async () => {
      const f = fileInput.files && fileInput.files[0];
      fileInput.value = '';
      if (!f) return;
      await stageAndConfirm({ file_name: f.name, file_path: f.path || f.name });
    });
    dirBtn.addEventListener('click', async () => {
      try {
        const r = await invoke('open_workspace_dir');
        toast(r || '技能目录：~/.qibai_skills');
      } catch (e) {
        toast('技能目录：~/.qibai_skills');
      }
    });
  }

  // stage → 弹窗确认 → commit（取消不发 commit）
  async function stageAndConfirm(opts) {
    // stage 期间 disable 按钮，防重复发起
    const urlBtn = $('btnSkillInstallUrl');
    const fileBtn = $('btnSkillInstallFile');
    if (urlBtn) urlBtn.disabled = true;
    if (fileBtn) fileBtn.disabled = true;
    let summary;
    try {
      summary = await invoke('skill_install_stage', opts);
    } catch (e) {
      toast('安装失败：' + ((e && e.message) || e));
      if (urlBtn) urlBtn.disabled = false;
      if (fileBtn) fileBtn.disabled = false;
      return;
    }
    // 渲染结构化模态
    const body = $('skillInstallBody');
    const perms = summary.permissions || [];
    const permHtml = ['network', 'execute', 'files'].map((p) => {
      const has = perms.includes(p);
      return `<span class="perm-item ${has ? 'perm-yes' : 'perm-no'}">${has ? '✓' : '✗'} ${p}</span>`;
    }).join('');
    const sourceDomain = summary.source_url
      ? summary.source_url.replace(/^https?:\/\//, '').replace(/\/.*$/, '')
      : (summary.file_name || '?');
    body.innerHTML = `
      <div class="row"><span class="k">名称</span><span>${summary.name || '?'}</span></div>
      <div class="row"><span class="k">版本</span><span>${summary.version || '未知'}</span></div>
      <div class="row"><span class="k">作者</span><span>${summary.author || '未知'}</span></div>
      <div class="row"><span class="k">来源</span><span>${sourceDomain}</span></div>
      <div class="row"><span class="k">说明</span><span>${summary.description || '无'}</span></div>
      <div class="perms">${permHtml}</div>
      <div class="row" style="margin-top:6px;"><span class="k">文件数</span><span>${summary.file_count || 0}</span></div>
      <div class="row"><span class="k">总大小</span><span>${summary.total_size || 0} 字节</span></div>
    `;
    const modal = $('skillInstallModal');
    const btnConfirm = $('btnSkillInstallConfirm');
    const btnCancel = $('btnSkillInstallCancel');
    modal.removeAttribute("hidden");
    btnCancel.focus(); // 默认焦点在取消

    let resolved = false;
    const cleanup = () => {
      modal.setAttribute("hidden", "");
      if (urlBtn) urlBtn.disabled = false;
      if (fileBtn) fileBtn.disabled = false;
    };
    const doCancel = async () => {
      if (resolved) return;
      resolved = true;
      cleanup();
      try { await invoke('skill_install_cancel', { stage_id: summary.stage_id }); } catch (e) {}
      toast('已取消安装');
    };
    const doConfirm = async () => {
      if (resolved) return;
      resolved = true;
      cleanup();
      try {
        await invoke('skill_install_commit', { stage_id: summary.stage_id });
        toast('已安装：' + (summary.name || ''));
        refreshBuiltinSkills();
        refreshPluginList();
      } catch (e) {
        toast('安装失败：' + ((e && e.message) || e));
      }
    };
    // 一次性绑定（避免重复绑）
    btnConfirm.onclick = (e) => { e.stopPropagation(); doConfirm(); };
    btnCancel.onclick = (e) => { e.stopPropagation(); doCancel(); };
    modal.querySelector('.clear-modal-mask').onclick = (e) => { e.stopPropagation(); doCancel(); };
    modal.onkeydown = (e) => {
      if (e.key === 'Escape') { e.stopPropagation(); doCancel(); }
    };
    modal.tabIndex = -1;
    modal.focus();
  }

  async function refreshSkillPackList() {
    const box = $('pbSkillPackList');
    box.innerHTML = '<div class="plg-empty">加载中…</div>';
    let list = [];
    try { list = await invoke('skill_pack_list'); } catch (e) {
      console.warn(e);
      box.innerHTML = '<div class="plg-empty">技能包列表加载失败</div>';
      return;
    }
    box.innerHTML = '';
    if (!list.length) {
      box.innerHTML = '<div class="plg-empty">还没有可导出的技能包。安装外部插件后即可在此打包导出。</div>';
      return;
    }
    list.forEach((p) => {
      const card = document.createElement('div');
      card.className = 'plg-card';
      const head = document.createElement('div'); head.className = 'plg-head';
      const nm = document.createElement('span'); nm.className = 'plg-name';
      nm.textContent = p.name || p.id;
      const ver = document.createElement('span'); ver.className = 'plg-ver';
      ver.textContent = 'v' + (p.version || '1.0.0');
      head.appendChild(nm); head.appendChild(ver);
      card.appendChild(head);
      const d = document.createElement('div'); d.className = 'plg-desc';
      d.textContent = p.description || '';
      card.appendChild(d);
      const foot = document.createElement('div'); foot.className = 'plg-foot';
      const chip = document.createElement('span'); chip.className = 'plg-chip granted';
      chip.textContent = (p.entry_kind || '') + ' · 已安装';
      const ex = document.createElement('button'); ex.className = 'ghost'; ex.type = 'button';
      ex.textContent = '导出 .qbpk';
      ex.addEventListener('click', async () => {
        try {
          await invoke('skill_pack_export', { plugin_id: p.id, zip_path: 'demo://' + p.id + '.qbpk' });
          toast('技能包已导出：' + p.id + '.qbpk');
        } catch (e) {
          console.warn(e);
          toast('导出失败：' + ((e && e.message) || ''));
        }
      });
      foot.appendChild(chip); foot.appendChild(ex);
      card.appendChild(foot);
      box.appendChild(card);
    });
  }

  /* ============================================================
   * 冒烟 hash 钩子（浏览器 mock 专用，Tauri 无副作用）
   *   #tab=chat|roles|intimacy|events|settings|pk
   *   #fire=summon&b=huopo   （角色 tab 自动召唤）
   *   #fire=greet            （串门寒暄）
   *   #fire=pk&ma=scissors&mb=rock（猜拳 tab 自动开一局）
   *   #fire=approval        （弹审批窗）
   *   #fire=plugins-install  （技能与专家 tab：自动安装示例插件，进入权限交互态）
   *   #fire=conn-add        （技能与专家 tab：自动添加一个 mock MCP 连接器→列表→移除）
   *   #fire=inti-upgrade[&b=huopo] （亲密度 tab：seed 当前等级+1，验证徽章动画+banner；b 可指定角色）
   * ============================================================ */
  async function handleHashFire(kv) {
    if (kv.fire === 'conn-add') {
      switchTab('plugins');
      setTimeout(async () => {
        try {
          await invoke('connector_add', {
            id: 'smoke_conn', name: '冒烟连接器', kind: 'mcp',
            command: 'python3', args: ['mock_server.py'], description: '冒烟测试',
            enabled: true,
          });
          const list = await invoke('connector_list');
          if (!list.some((c) => c.id === 'smoke_conn')) throw new Error('连接器未出现在列表');
          await invoke('connector_remove', { id: 'smoke_conn' });
          toast('连接器闭环冒烟通过：添加→列表→移除');
        } catch (e) {
          console.warn(e);
          toast('连接器冒烟失败：' + ((e && e.message) || ''));
        }
        refreshConnectorList();
      }, 200);
      return;
    }
    if (kv.fire === 'inti-upgrade') {
      switchTab('intimacy');
      setTimeout(async () => {
        try {
          if (!intiPersona) {
            try {
              const d = await invoke('get_personas');
              intiPersona = (d && d.active_persona) || 'gudian';
            } catch (e) { intiPersona = 'gudian'; }
          }
          if (kv.b) intiPersona = kv.b; // 可选：指定演示角色
          await loadIntimacyTab();
          const nextLevel = ((intiState && intiState.level) || 1) + 1;
          const mock = window.__qibaiMock;
          if (mock && mock.intimacy && typeof mock.intimacy.seed === 'function') {
            // seed(level, personaId?)：直接置到下一等级阈值起点，
            // 内部跨档时广播 qibai://intimacy-updated(leveled_up=true)
            mock.intimacy.seed(nextLevel, intiPersona);
          } else {
            // 降级：循环 chat 事件直到升级（上限 100 次）
            for (let i = 0; i < 100; i++) {
              const res = await invoke('intimacy_event', { persona_id: intiPersona, kind: 'chat' });
              if (res && res.leveled_up) break;
            }
          }
        } catch (e) { console.warn(e); }
      }, 400);
      return;
    }
    if (kv.fire === 'approval') {
      setTimeout(() => {
        currentRequestId = 'mock-req-' + Date.now();
        $('approvalSkill').textContent = 'launch_app';
        $('approvalDesc').textContent = '元气小小想为你启动「浏览器」，需要你的确认。';
        $('approvalModal').classList.add('show');
      }, 400);
      return;
    }
    if (kv.fire === 'plugins-install') {
      // 技能与专家 tab：自动走一遍「安装示例插件」流程（权限交互态冒烟）
      switchTab('plugins');
      setTimeout(async () => {
        try {
          await invoke('plugin_install', { dir: 'demo://examples' });
        } catch (e) { console.warn(e); }
        loadPluginsTab();
      }, 400);
      return;
    }
    if (kv.fire === 'summon') {
      switchTab('roles');
      const id = kv.b || 'huopo';
      setTimeout(async () => {
        try {
          await invoke('pet_summon', { persona_id: id });
          toast(`${PERSONA_NAMES[id] || id} 已召唤上桌`);
          loadRoles();
        } catch (e) { console.warn(e); }
      }, 400);
      return;
    }
    if (kv.fire === 'greet') {
      switchTab('pk');
      setTimeout(async () => {
        try {
          const [data, desk] = await Promise.all([
            invoke('get_personas'),
            invoke('get_desk_state'),
          ]);
          const active = data.active_persona;
          let guests = ((desk && desk.present) || []).filter((x) => x !== active);
          if (!guests.length) {
            await invoke('pet_summon', { persona_id: 'huopo' });
            guests = ['huopo'];
          }
          pkState.activeId = active;
          pkState.guestId = guests[0];
          await loadPkTab();
          await doGreet();
        } catch (e) { console.warn(e); }
      }, 500);
      return;
    }
    if (kv.fire === 'pk') {
      switchTab('pk');
      setTimeout(async () => {
        try {
          const [data, desk] = await Promise.all([
            invoke('get_personas'),
            invoke('get_desk_state'),
          ]);
          const active = data.active_persona;
          let guests = ((desk && desk.present) || []).filter((x) => x !== active);
          if (!guests.length) {
            await invoke('pet_summon', { persona_id: 'huopo' });
            guests = ['huopo'];
          }
          pkState.activeId = active;
          pkState.guestId = guests[0];
          await loadPkTab();
          await doPkRound(kv.ma || 'scissors', kv.mb || 'rock');
        } catch (e) { console.warn(e); }
      }, 500);
    }
  }

  /* ============================================================
   * 启动
   * ============================================================ */
  async function init() {
    // 设置表单
    try {
      const s = await invoke('get_settings');
      apply(s || {});
    } catch (e) {
      apply({});
      console.warn('get_settings failed, using defaults', e);
    }
    refreshPersonaSel(); // 缓存主角色 id（聊天/技能门控上报用）
    refreshSkillsGate(); // 助理技能门控校验（assistant_skills，L4）
    bindPersonaGrid();

    // 默认立绘
    $('kMainHand').innerHTML = MOVE_SVG.rock;
    $('kGuestHand').innerHTML = MOVE_SVG.rock;

    // 事件订阅（全部经 QibaiBridge.listen）
    // （注册见上方 listen 调用，此处无额外动作）

    // hash 钩子先于默认 tab 执行（内部会 switchTab）
    const kv = parseHashKv();
    if (kv.fire) {
      handleHashFire(kv);
    } else if (kv.tab) {
      switchTab(kv.tab);
    } else {
      switchTab('chat');
    }
    // 当前 tab 数据兜底预载（handleHashFire 异步流程内也会再载）
    onTabShown(currentTab);
    // v20.1/v20.2：控制台打开完成后再清一次初始渲染残影——无合成器 X11 下 HTML
    // 首帧像素可能残留，覆盖在异步加载出的内容上（=「右键后内容覆盖、卡片不
    // 消失」）。先走 rAF reflow，再调 Rust console_repaint（窗口级 hide/show
    // 强制重绘，reflow 清不掉窗口表面旧像素时的真正兜底）。
    requestAnimationFrame(() => {
      const b = document.body;
      const d = b.style.display;
      b.style.display = 'none';
      void b.offsetHeight;
      b.style.display = d;
    });
    try {
      invoke('console_repaint').catch(() => {});
    } catch (e) { /* bridge 缺失时静默 */ }
  }

  document.addEventListener('DOMContentLoaded', init);

  // v45.14：以下四个尾部模块（任务面板 / 计划确认+高危模态 / 知识库设置 / 音频诊断）
  // 原先是写在主 IIFE 之外的顶层代码，却直接引用 IIFE 内的 $/invoke/listen/toast —
  // 浏览器里一进就是 ReferenceError（$ is not defined），四块功能等于从来没跑起来。
  // 这里不再提前闭合 IIFE，让它们留在同一作用域内。

// ===== P0 任务面板骨架（IPC 名以 core 最终为准，先按 task_list/task_get/task_*_control 约定） =====
(async function initTasksPanel() {
  const listEl = $('tasksList');
  const emptyEl = $('tasksEmpty');
  const detailEl = $('taskDetail');
  let currentTaskId = null;

  async function refreshList() {
    if (!listEl) return;
    let tasks = [];
    try { tasks = await invoke('task_list') || []; } catch (e) { /* core 未接时静默 */ }
    listEl.innerHTML = '';
    emptyEl.style.display = tasks.length ? 'none' : 'block';
    tasks.forEach((t) => {
      const row = document.createElement('div');
      row.className = 'task-row';
      const badge = t.state || t.status || 'draft';
      const done = t.done_steps || 0;
      const total = t.total_steps || 0;
      const cur = t.current_step || (badge === 'running' ? done + 1 : done);
      const tokens = t.tokens_total || 0;
      const est = t.tokens_estimated ? ' (估算)' : '';
      row.innerHTML = `
        <span class="task-badge ${badge}">${badge}</span>
        <span class="task-title"></span>
        <span class="task-progress">${cur}/${total}${tokens ? ' · ' + tokens + ' tokens' + est : ''}</span>
      `;
      row.querySelector('.task-title').textContent = t.title || t.id || '(未命名)';
      row.onclick = () => openDetail(t.id || t.task_id);
      listEl.appendChild(row);
    });
  }

  // v45.13：步骤 id → step 索引（并行 chip 渲染用）
  function stepIndexMap(t) {
    const m = {};
    (t.steps || []).forEach((s) => { if (s.id) m[s.id] = s; });
    return m;
  }

  // v45.13：并行组 chip 状态色：等待灰 / 运行蓝 / 成功绿 / 失败红(悬停 error) / 已跳过浅灰
  function chipColor(state) {
    switch (state) {
      case 'running': case 'waiting_confirm': return { bg: '#dbeafe', fg: '#1d4ed8', border: '#93c5fd' };
      case 'done': return { bg: '#dcfce7', fg: '#15803d', border: '#86efac' };
      case 'failed': return { bg: '#fee2e2', fg: '#b91c1c', border: '#fca5a5' };
      case 'skipped': return { bg: '#f3f4f6', fg: '#9ca3af', border: '#e5e7eb' };
      default: return { bg: '#f3f4f6', fg: '#6b7280', border: '#e5e7eb' };
    }
  }

  // v45.13：渲染并行总览条 + 并行组 + partial 徽标 + 失败原因。
  // 数据全部来自后端 task_get 透出的 t.parallel（真实取自 task 内部状态），前端不猜。
  function renderParallel(t) {
    const p = t.parallel || {};
    const prog = p.progress || { done: 0, total: (t.steps || []).length };
    const limit = p.max || 3;
    const running = p.running || [];
    const total = prog.total || (t.steps || []).length;
    const done = prog.done || 0;

    // partial 橙色徽标
    const badge = $('taskPartialBadge');
    if (badge) {
      const isPartial = (t.state === 'partial') || (t.status === 'partial') || ((t.state === 'failed' || t.status === 'failed') && (p.failed || []).length > 0 && done > 0);
      badge.hidden = !isPartial;
    }

    // 总览条
    const bar = $('taskParallelBar');
    if (bar) {
      if (!total) { bar.style.display = 'none'; } else {
        bar.style.display = 'block';
        $('taskParallelProgressText').textContent = `进度 ${done}/${total}`;
        $('taskParallelRunningText').textContent = `并发 运行 ${running.length}/${limit}`;
        const pct = total ? Math.round((done / total) * 100) : 0;
        $('taskParallelProgressFill').style.width = pct + '%';
      }
    }

    // 并行组
    const groupsEl = $('taskParallelGroups');
    if (groupsEl) {
      groupsEl.innerHTML = '';
      const groups = p.groups || [];
      if (!groups.length || !total) { groupsEl.style.display = 'none'; } else {
        groupsEl.style.display = 'block';
        const smap = stepIndexMap(t);
        groups.forEach((grp, gi) => {
          const grpArr = Array.isArray(grp) ? grp : [];
          const grpRunning = grpArr.filter((id) => (p.running || []).includes(id)).length;
          const row = document.createElement('div');
          row.style.cssText = 'display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding:4px 0;border-bottom:1px dashed #f0e6d8;';
          const title = document.createElement('span');
          title.style.cssText = 'font-size:12px;color:#6b7280;min-width:96px;';
          title.textContent = `组 ${gi + 1}（并行 ${grpRunning}/${grpArr.length}）`;
          row.appendChild(title);
          grpArr.forEach((sid) => {
            const s = smap[sid] || {};
            const st = s.state || s.status || 'pending';
            const c = chipColor(st);
            const chip = document.createElement('span');
            chip.textContent = s.title || s.intent || sid;
            chip.style.cssText = `font-size:12px;padding:2px 8px;border-radius:10px;background:${c.bg};color:${c.fg};border:1px solid ${c.border};`;
            if (s.error) chip.title = s.error; // 失败红悬停显示 error 原文
            row.appendChild(chip);
          });
          groupsEl.appendChild(row);
        });
      }
    }

    // 失败原因列表（partial / failed 时）
    const failEl = $('taskFailedReasons');
    if (failEl) {
      failEl.innerHTML = '';
      const failedIds = p.failed || [];
      if (!failedIds.length) { failEl.style.display = 'none'; } else {
        failEl.style.display = 'block';
        const smap = stepIndexMap(t);
        failedIds.forEach((sid) => {
          const s = smap[sid] || {};
          const line = document.createElement('div');
          line.style.cssText = 'color:#b91c1c;font-size:12px;margin-top:2px;';
          line.textContent = `✕ ${s.title || sid}：${s.error || '未知错误'}`;
          failEl.appendChild(line);
        });
      }
    }
  }

  // v45.14：终态集合 —— 只有终态任务容得下「重跑」（运行中要先暂停/取消）
  const TERMINAL_STATES = ['failed', 'partial', 'cancelled', 'done'];
  function st_now(t) { return (t && (t.state || t.status)) || 'draft'; }

  async function openDetail(id) {
    currentTaskId = id;
    let t = null;
    // 参数名必须是 taskId：Tauri v2 把命令的 snake_case 参数按 camelCase 匹配，
    // 早先这里写 { id } 落不到 task_id 上，调用直接缺参，又被 try/catch 吞掉——
    // 表现为「点任务行、详情面板空白」。
    try { t = await invoke('task_get', { taskId: id }) || {}; } catch (e) { t = {}; }
    detailEl.hidden = false;
    $('taskDetailTitle').textContent = t.title || id;
    const tokens = t.tokens_total || 0;
    const est = t.tokens_estimated ? ' (估算)' : '';
    const pausedReason = t.paused_reason ? ` · 暂停原因：${t.paused_reason}` : '';
    const total = (t.parallel && t.parallel.progress) ? t.parallel.progress.total : (t.steps || []).length;
    const doneN = (t.parallel && t.parallel.progress) ? t.parallel.progress.done : (t.steps || []).filter(s => (s.state||s.status)==='done').length;
    $('taskDetailMeta').textContent =
      `状态：${t.state || t.status || '?'} · 进度 ${doneN}/${total} 步` +
      (tokens ? ` · 累计 ${tokens} tokens${est}` : '') + pausedReason;
    const stepsEl = $('taskDetailSteps');
    stepsEl.innerHTML = '';
    (t.steps || []).forEach((s, i) => {
      const div = document.createElement('div');
      div.className = 'task-step ' + (s.state || s.status || 'pending');
      const attempts = s.attempts ? ` · 重试 ${s.attempts}` : '';
      div.innerHTML = `<span>${i + 1}. ${s.title || s.intent || '—'}${attempts}</span>`;
      if (s.error) {
        const err = document.createElement('div');
        err.style.cssText = 'color:#b91c1c;font-size:12px;margin-top:2px;';
        err.textContent = `失败原因：${s.error}`;
        div.appendChild(err);
      }
      // v45.14：Workflow 重跑——终态任务允许从任意一步起重跑（该步及之后一起重来）
      if (currentTaskId && TERMINAL_STATES.includes(st_now(t)) && (s.id || s.step_id)) {
        const rerunBtn = document.createElement('button');
        rerunBtn.type = 'button';
        rerunBtn.className = 'mini-btn';
        rerunBtn.style.cssText = 'display:block;margin-top:6px;font-size:12px;';
        rerunBtn.textContent = '从此步重跑';
        rerunBtn.onclick = async (ev) => {
          ev.stopPropagation();
          try {
            await invoke('task_rerun', { taskId: currentTaskId, fromStep: s.id || s.step_id });
            await refreshList();
            await openDetail(currentTaskId);
            toast('已从该步重跑');
          } catch (e) { toast((e && e.message) || e); }
        };
        div.appendChild(rerunBtn);
      }

      // runs 折叠
      if (s.runs && s.runs.length) {
        const runsWrap = document.createElement('details');
        runsWrap.style.cssText = 'margin-top:4px;font-size:12px;color:#6b7280;';
        runsWrap.innerHTML = `<summary>尝试记录（${s.runs.length} 次）</summary>`;
        s.runs.forEach((r) => {
          const line = document.createElement('div');
          const rTokens = r.tokens ? ` · ${r.tokens} tokens${r.tokens_estimated ? ' (估算)' : ''}` : '';
          line.textContent = `#${r.attempt || '?'} ${r.model || '?'}${rTokens}${r.error ? ' — ' + r.error : ''}`;
          runsWrap.appendChild(line);
        });
        div.appendChild(runsWrap);
      }
      stepsEl.appendChild(div);
    });
    // v45.13：渲染并行总览
    renderParallel(t);
    // 未确认计划时禁用开始按钮
    const st = t.state || t.status || 'draft';
    const needConfirm = (st === 'planned' || st === 'draft') && !t.confirmed;
    $('btnTaskStart').disabled = needConfirm;
    $('taskPlanWarning').textContent = needConfirm ? '计划未确认，先点计划里的"确认执行"才能开始。' : '';
    // 暂停/继续按钮按状态切换
    $('btnTaskPause').disabled = st !== 'running';
    $('btnTaskResume').disabled = st !== 'paused' && st !== 'interrupted';
    $('btnTaskCancel').disabled = !['running','planned','paused','waiting_confirm'].includes(st);
    // v45.14：终态都能重跑（done 也能整体重来；failed/partial/cancelled 重跑没成功的步）
    $('btnTaskRerunFailed').disabled = !TERMINAL_STATES.includes(st);
    $('btnTaskRerunFailed').textContent = st === 'done' ? '整体重跑' : '重跑未完成步';
  }

  function bindBtn(id, cmd) {
    const el = $(id);
    if (!el) return;
    el.onclick = async () => {
      if (!currentTaskId) return;
      try { await invoke(cmd, { taskId: currentTaskId }); refreshList(); openDetail(currentTaskId); } catch (e) { toast((e && e.message) || e); }
    };
  }
  bindBtn('btnTaskStart', 'task_start');
  bindBtn('btnTaskPause', 'task_pause');
  bindBtn('btnTaskResume', 'task_resume');
  bindBtn('btnTaskCancel', 'task_cancel');
  bindBtn('btnTaskRerunFailed', 'task_rerun');  // v45.14：fromStep 省略=重跑所有没成功的步
  $('btnTaskDetailClose').onclick = () => { detailEl.hidden = true; currentTaskId = null; };
  $('btnTasksRefresh').onclick = refreshList;

  // v45.13：并行上限输入（1-8，默认 3），调 task_set_parallel_max，落盘+立即生效。
  const parallelInput = $('taskParallelMax');
  async function loadParallelMax() {
    try {
      const s = await invoke('get_settings') || {};
      if (parallelInput) parallelInput.value = (s.task_parallel_max != null) ? s.task_parallel_max : 3;
    } catch (e) {}
  }
  if (parallelInput) {
    parallelInput.onchange = async () => {
      let n = parseInt(parallelInput.value, 10);
      if (isNaN(n)) n = 3;
      n = Math.min(8, Math.max(1, n));
      try {
        const r = await invoke('task_set_parallel_max', { n }) || {};
        parallelInput.value = r.task_parallel_max || n;
        toast(`并行上限已设为 ${r.task_parallel_max || n}`);
        if (currentTaskId) openDetail(currentTaskId);
      } catch (e) { toast('设置并行上限失败：' + ((e && e.message) || e)); }
    };
  }
  loadParallelMax();

  // 事件监听（名字以 core emit 为准，先兼容）
  try {
    await listen('task://updated', () => { refreshList(); if (currentTaskId) openDetail(currentTaskId); });
    await listen('task://finished', () => { toast('任务完成'); refreshList(); });
    // v45.13：并行分支变化事件（调度线 emit；带 parallel 字段）。
    await listen('qibai://task-branches', (e) => {
      const pl = (e && e.payload) || e || {};
      if (currentTaskId && pl.task_id === currentTaskId) openDetail(currentTaskId);
      else refreshList();
    });
  } catch (e) {}

  refreshList();
})();

// ===== P0 计划确认 + 高危动作模态 =====
(async function initTaskModals() {
  // 计划确认模态：展示步骤列表，支持改标题/删/上下移
  const planModal = $('planConfirmModal');
  const planListEl = $('planStepsList');
  let planSteps = [];
  let planTaskId = null;

  function renderPlanSteps() {
    planListEl.innerHTML = '';
    planSteps.forEach((s, i) => {
      const row = document.createElement('div');
      row.style.cssText = 'display:flex;gap:6px;align-items:center;padding:4px 0;border-bottom:1px solid #f0e6d8;';
      row.innerHTML = `
        <span style="flex:1;"><input value="${(s.title || s.intent || '').replace(/"/g,'&quot;')}" style="width:100%;border:1px solid #e5d9c8;border-radius:4px;padding:2px 6px;font-size:13px;"></span>
        <button type="button" class="mini-btn up">↑</button>
        <button type="button" class="mini-btn down">↓</button>
        <button type="button" class="mini-btn del">删</button>
      `;
      const input = row.querySelector('input');
      input.onchange = () => { s.title = input.value; };
      row.querySelector('.up').onclick = () => { if (i > 0) { [planSteps[i-1], planSteps[i]] = [planSteps[i], planSteps[i-1]]; renderPlanSteps(); } };
      row.querySelector('.down').onclick = () => { if (i < planSteps.length-1) { [planSteps[i+1], planSteps[i]] = [planSteps[i], planSteps[i+1]]; renderPlanSteps(); } };
      row.querySelector('.del').onclick = () => { planSteps.splice(i, 1); renderPlanSteps(); };
      planListEl.appendChild(row);
    });
  }

  window.__openPlanConfirm = (taskId, steps) => {
    planTaskId = taskId;
    planSteps = (steps || []).map((s) => ({ ...s }));
    renderPlanSteps();
    planModal.removeAttribute('hidden');
    $('btnPlanCancel').focus();
  };
  $('btnPlanCancel').onclick = () => planModal.setAttribute('hidden', '');
  $('btnPlanConfirm').onclick = async () => {
    if (!planTaskId) return;
    try {
      await invoke('task_plan', { taskId: planTaskId, steps: planSteps });
      await invoke('task_confirm_plan', { taskId: planTaskId });
      planModal.setAttribute('hidden', '');
      toast('计划已确认');
    } catch (e) { toast('确认失败：' + ((e && e.message) || e)); }
  };

  // 高危动作模态
  const actionModal = $('actionConfirmModal');
  let pendingAction = null;
  window.__openActionConfirm = (payload) => {
    pendingAction = payload;
    $('actionConfirmKind').textContent = payload.action_kind || '—';
    $('actionConfirmTarget').textContent = payload.target || payload.intent || '—';
    $('actionConfirmRisk').textContent = payload.risk || '—';
    $('actionConfirmTask').textContent = payload.task_id || '—';
    const rememberWrap = $('actionRememberWrap');
    // Dangerous 不显示"本任务内不再问"
    if (payload.risk === 'Dangerous') {
      rememberWrap.style.display = 'none';
      $('btnActionAllow').classList.remove('primary');
    } else {
      rememberWrap.style.display = 'block';
      $('btnActionAllow').classList.add('primary');
    }
    actionModal.removeAttribute('hidden');
    $('btnActionDeny').focus();
  };
  $('btnActionDeny').onclick = async () => {
    if (!pendingAction) return;
    try {
      await invoke('task_confirm_action', {
        taskId: pendingAction.task_id,
        actionId: pendingAction.action_id,
        allow: false,
        remember: false,
      });
      toast('已拒绝该操作');
    } catch (e) { toast('拒绝失败：' + ((e && e.message) || e)); }
    actionModal.setAttribute('hidden', '');
    pendingAction = null;
  };
  $('btnActionAllow').onclick = async () => {
    if (!pendingAction) return;
    const remember = $('actionRemember').checked;
    try {
      await invoke('task_confirm_action', {
        taskId: pendingAction.task_id,
        actionId: pendingAction.action_id,
        allow: true,
        remember,
      });
    } catch (e) { toast('确认失败：' + ((e && e.message) || e)); }
    actionModal.setAttribute('hidden', '');
    pendingAction = null;
  };

  // 监听 waiting_confirm 事件（core emit）
  try {
    await listen('task://waiting_confirm', (e) => window.__openActionConfirm(e.payload || e));
  } catch (e) {}
})();

// ===== v45.11：知识库设置 =====
(async function initKbSettings() {
  const foldersEl = $('kbFoldersList');
  const statusEl = $('kbStatus');
  if (!foldersEl) return;

  const embedEl = $('kbEmbedStatus');
  const fetchBtn = $('btnKbEmbedFetch');
  let fetching = false;
  try {
    await listen('kb:embed_progress', (ev) => {
      const p = (ev && ev.payload) || {};
      if (!fetching || !embedEl) return;
      const mb = (p.done / 1048576).toFixed(1);
      const totalMb = p.total ? (p.total / 1048576).toFixed(1) : '?';
      embedEl.textContent = `下载向量模型中… ${p.pct || 0}%（${mb} / ${totalMb} MB）`;
    });
  } catch (e) {}
  if (fetchBtn) {
    fetchBtn.onclick = async () => {
      if (fetching) return;
      fetching = true;
      fetchBtn.disabled = true;
      fetchBtn.textContent = '下载中…';
      try {
        const r = await invoke('kb_embed_fetch');
        toast(`向量模型已安装（${r.model}）`);
        refreshEmbed();
      } catch (e) {
        toast('下载失败：' + ((e && e.message) || e));
        refreshEmbed();
      } finally {
        fetching = false;
        fetchBtn.disabled = false;
        fetchBtn.textContent = '下载向量模型';
      }
    };
  }
  async function refreshEmbed() {
    if (!embedEl) return;
    try {
      const e = await invoke('kb_embed_status');
      let text;
      if (!e.enabled) text = '语义检索：已关闭（纯 BM25 关键词）';
      else if (e.loaded) text = `语义检索：已开启（${e.model} · ${e.dim} 维 · 已向量化 ${e.vector_chunks || 0} 块 · 权重 ${(+e.hybrid_weight).toFixed(2)}）`;
      else if (e.installed) text = `语义检索：模型已下载但加载失败，当前走纯 BM25（${e.hint || ''}）`;
      else text = `语义检索：未安装模型，当前走纯 BM25。${e.hint || ''}`;
      embedEl.textContent = text;
    } catch (err) {
      embedEl.textContent = '语义检索状态读取失败：' + ((err && err.message) || err);
    }
  }

  async function refreshStatus() {
    try {
      const s = await invoke('kb_status');
      foldersEl.innerHTML = '';
      (s.folders || []).forEach((f) => {
        const row = document.createElement('div');
        row.style.cssText = 'display:flex;justify-content:space-between;align-items:center;padding:2px 0;font-size:13px;';
        row.innerHTML = `<span style="overflow:hidden;text-overflow:ellipsis;">${f}</span>`;
        const rm = document.createElement('button');
        rm.className = 'mini-btn'; rm.textContent = '移除';
        rm.onclick = async () => {
          try { await invoke('kb_remove_folder', { path: f }); refreshStatus(); } catch (e) { toast('移除失败：' + ((e && e.message) || e)); }
        };
        row.appendChild(rm);
        foldersEl.appendChild(row);
      });
      if (!s.folders || !s.folders.length) {
        foldersEl.innerHTML = '<div class="hint" style="font-size:12px;">还没有知识库目录。加一个文件夹，小小就能在里面找资料。</div>';
      }
      const err = s.last_error ? `<span style="color:#b91c1c;">${s.last_error}</span>` : '';
      // 后端 indexed_at 是 Unix 秒；万一有的是毫秒也兼容
      const ts = s.indexed_at > 1e12 ? s.indexed_at : s.indexed_at * 1000;
      const when = s.indexed_at ? ' · 上次索引 ' + new Date(ts).toLocaleString('zh-CN') : '';
      statusEl.innerHTML = `文件 ${s.files || 0} · 块 ${s.chunks || 0}${when} ${err}`;
      refreshEmbed();
    } catch (e) {
      statusEl.innerHTML = '<span style="color:#b91c1c;">状态读取失败：' + ((e && e.message) || e) + '</span>';
    }
  }

  $('btnKbAdd').onclick = async () => {
    const p = $('kbFolderInput').value.trim();
    if (!p) return;
    try {
      await invoke('kb_add_folder', { path: p });
      $('kbFolderInput').value = '';
      refreshStatus();
    } catch (e) { toast('添加失败：' + ((e && e.message) || e)); }
  };
  $('btnKbReindex').onclick = async () => {
    statusEl.textContent = '索引中…';
    try {
      const r = await invoke('kb_reindex');
      statusEl.textContent = `完成：文件 ${r.file_count || 0} · 块 ${r.chunk_count || 0}`;
      refreshStatus();
    } catch (e) { toast('索引失败：' + ((e && e.message) || e)); refreshStatus(); }
  };
  refreshStatus();
})();

// ===== v45.11：输出设备测试 + 音频诊断 =====
(async function initAudioDiag() {
  const btn = $('btnAudioTest');
  const result = $('audioTestResult');
  const diag = $('audioDiagLog');
  if (!btn) return;
  let logs = [];
  function pushLog(line) {
    logs.push(line);
    if (logs.length > 20) logs.shift();
    if (diag) diag.textContent = logs.join('\n');
  }
  try {
    await listen('log:audio', (e) => pushLog(String(e.payload || e)));
  } catch (e) {}
  btn.onclick = async () => {
    btn.disabled = true;
    const old = btn.textContent;
    btn.textContent = '测试中…';
    result.textContent = '';
    result.style.color = '';
    try {
      const r = await invoke('audio_test_output');
      pushLog('[ui] audio_test_output ok: ' + JSON.stringify(r));
      const tools = r.tools || {};
      const toolLine = ['pacat', 'pw-play', 'paplay', 'rodio']
        .map((k) => `${k} ${tools[k] ? '✓' : '✗'}`).join('  /  ');
      // 兼容新旧字段：新字段 selected_name/selected_index/sink_landing_name/sink_landing_index
      // 旧字段 sink_landing.{name,idx} / selected(字符串)
      const landing = r.sink_landing || {};
      const selName = r.selected_name || r.selected || '?';
      const selIdx = r.selected_index != null ? r.selected_index : (landing.selected_idx != null ? landing.selected_idx : null);
      const landName = r.sink_landing_name || landing.name || 'unknown';
      const landIdx = r.sink_landing_index != null ? r.sink_landing_index : (landing.idx != null ? landing.idx : null);
      const idxEq = selIdx !== null && landIdx !== null && selIdx === landIdx;
      const idxKnown = selIdx !== null && landIdx !== null;
      const status = idxEq
        ? `<div style="margin-top:4px;padding:4px 8px;background:#052e16;color:#86efac;border-radius:4px;font-size:12px;">✓ 已生效：落点与选择一致</div>`
        : `<div style="margin-top:4px;padding:4px 8px;background:#7f1d1d;color:#fecaca;border-radius:4px;font-size:12px;">✗ 选择未生效：实际落在 #${landIdx ?? '?'}，不是 #${selIdx ?? '?'}<br><span style="font-size:11px;">可能原因：pacat/pw-play/paplay 未安装，已回落到 ALSA 默认</span></div>`;
      result.innerHTML =
        `<div style="margin-top:4px;line-height:1.7;font-size:12px;">` +
        `<div>你选的：<b>${selName}</b>${selIdx !== null ? ` <span style="color:#9ca3af;">(#${selIdx})</span>` : ''}</div>` +
        `<div>实际落在：<b style="color:${idxEq ? '#86efac' : '#fca5a5'};">${landName}</b>${landIdx !== null ? ` <span style="color:#9ca3af;">(#${landIdx})</span>` : ''}</div>` +
        `<div style="color:#6b7280;margin-top:2px;">后端：${r.backend || '?'} &nbsp;|&nbsp; 工具：${toolLine}</div>` +
        (r.note ? `<div style="font-size:11px;color:#9ca3af;margin-top:2px;">${r.note}</div>` : '') +
        status +
        `</div>`;
    } catch (e) {
      const msg = (e && e.message) || String(e);
      result.style.color = '#b91c1c';
      result.textContent = msg;
      pushLog('[ui] audio_test_output FAIL: ' + msg);
    } finally {
      btn.disabled = false;
      btn.textContent = old;
    }
  };
})();

// 主 IIFE 收口（见上：尾部四个模块已并入同一作用域）
})();
