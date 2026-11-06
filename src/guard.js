/* ============================================================
 * 元气小小 Qibai — 全屏守护演出 guard.js
 * 职责：
 *   1. 监听 qibai://guard-start，收到后演出：截图底图(或 CSS 模拟桌面)
 *      → Canvas 图标碎裂特效 → 伴侣现身 → 气泡文案 → 收尾动作。
 *   2. 全程「跳过」按钮 / ESC 可调；结束（自然或跳过）调 guard_skip 收尾。
 *   3. 浏览器独立模式：待机态提供「演示」按钮，本地直接播完整演出。
 *
 * payload 契约（后端 emit qibai://guard-start）：
 *   { mode, hours, message, end_action, countdown_secs, persona_id, screenshot? }
 * ============================================================ */
(function () {
  'use strict';

  var bridge = window.QibaiBridge || null;

  /* ---------- DOM ---------- */
  var standby = document.getElementById('standby');
  var standbyStatusText = document.getElementById('standby-status-text');
  var demoBtn = document.getElementById('demo-btn');
  var stage = document.getElementById('stage');
  var desktop = document.getElementById('desktop');
  var fx = document.getElementById('fx');
  var personaWrap = document.getElementById('persona-wrap');
  var persona = document.getElementById('persona');
  var personaFallback = document.getElementById('persona-fallback');
  var bubble = document.getElementById('bubble');
  var bubbleText = document.getElementById('bubble-text');
  var toast = document.getElementById('toast');
  var countdown = document.getElementById('countdown');
  var countdownNum = document.getElementById('countdown-num');
  var skipBtn = document.getElementById('skip-btn');
  var exitBtn = document.getElementById('exit-btn');

  /* ---------- 状态 ---------- */
  var state = {
    active: false,
    timers: [],
    countdownTimer: null,
    raf: 0
  };

  /* 演示用 payload（浏览器冒烟入口；不依赖后端、不读真实屏幕） */
  var DEMO_PAYLOAD = {
    mode: 'strong',
    hours: 2,
    message: '你连续工作 2 小时啦，休息五分钟吧',
    end_action: 'countdown',
    countdown_secs: 120,
    persona_id: 'wenrou',
    screenshot: null
  };

  function later(fn, ms) {
    var t = setTimeout(fn, ms);
    state.timers.push(t);
    return t;
  }
  function clearPendingTimers() {
    state.timers.forEach(function (t) { clearTimeout(t); });
    state.timers = [];
  }

  /* ============================================================
   * 演出主流程
   * ============================================================ */
  function startShow(p) {
    if (!p || state.active) return;
    state.active = true;

    standby.hidden = true;
    stage.hidden = false;
    bubble.hidden = true;
    toast.hidden = true;
    countdown.hidden = true;
    personaWrap.hidden = true;
    fx.classList.remove('done');

    // ① 底图：screenshot 为 data URL 时整屏铺底；否则 CSS 模拟桌面（纯视觉占位）
    if (p.screenshot && typeof p.screenshot === 'string' && p.screenshot.indexOf('data:') === 0) {
      desktop.style.backgroundImage = 'url("' + p.screenshot + '")';
      desktop.classList.add('has-shot');
    } else {
      desktop.style.backgroundImage = '';
      desktop.classList.remove('has-shot');
    }

    // ② Canvas 碎裂特效（约 1.4s）
    runCrackFx();

    // ③+④ 特效结束后：伴侣现身 + 气泡
    later(function () { revealPersona(p); }, 1400);
  }

  function revealPersona(p) {
    fx.classList.add('done');
    personaFallback.hidden = true;
    persona.hidden = false;
    persona.onerror = function () {
      persona.hidden = true;
      personaFallback.hidden = false;
    };
    persona.src = 'assets/personas/' + (p.persona_id || 'wenrou') + '/character.png';
    bubbleText.textContent = p.message || '该休息一下啦';
    personaWrap.hidden = false;
    bubble.hidden = false;

    // ⑤ 收尾动作：气泡展示后进入
    later(function () { runEndAction(p); }, 2200);
  }

  function runEndAction(p) {
    var action = p.end_action || 'none';
    if (action === 'countdown') {
      startCountdown(p.countdown_secs || 120);
    } else if (action === 'sleep' || action === 'lock') {
      invokeSystemAction(action);
    } else {
      // none：直接结束
      later(function () { endShow('auto'); }, 1200);
    }
  }

  /* 倒计时遮罩：60–300 秒休息倒计时 */
  function startCountdown(secs) {
    secs = Math.max(60, Math.min(300, secs || 120));
    countdown.hidden = false;
    var left = secs;
    renderCountdown(left);
    if (state.countdownTimer) clearInterval(state.countdownTimer);
    state.countdownTimer = setInterval(function () {
      left -= 1;
      if (left <= 0) {
        clearInterval(state.countdownTimer);
        state.countdownTimer = null;
        endShow('auto');
        return;
      }
      renderCountdown(left);
    }, 1000);
  }

  function renderCountdown(secs) {
    var m = Math.floor(secs / 60);
    var s = secs % 60;
    countdownNum.textContent =
      (m < 10 ? '0' + m : '' + m) + ':' + (s < 10 ? '0' + s : '' + s);
  }

  /* sleep / lock：调后端执行系统命令，按返回 {simulated, detail} 展示 */
  function invokeSystemAction(action) {
    showToast('正在请求执行：' + (action === 'sleep' ? '息屏' : '锁屏') + '…');
    Promise.resolve(bridge ? bridge.invoke('guard_system_action', { action: action }) : Promise.reject(new Error('no bridge')))
      .then(function (r) {
        var detail = (r && r.detail) ? r.detail : '已执行：' + action;
        showToast(detail);
      })
      .catch(function () {
        showToast('演示环境：未真实执行「' + (action === 'sleep' ? '息屏' : '锁屏') + '」，真机由后端执行');
      })
      .then(function () {
        later(function () { endShow('auto'); }, 2000);
      });
  }

  function showToast(msg) {
    toast.textContent = msg;
    toast.hidden = false;
  }

  /* ⑥ 收尾：自然结束 reason=auto；用户跳过 reason=user */
  function endShow(reason) {
    if (!state.active) return;
    state.active = false;
    if (state.countdownTimer) {
      clearInterval(state.countdownTimer);
      state.countdownTimer = null;
    }
    cancelAnimationFrame(state.raf);
    clearPendingTimers();

    // 通知后端收尾（浏览器 mock 下会 reject，静默忽略）
    if (bridge) {
      Promise.resolve(bridge.invoke('guard_skip', { reason: reason })).catch(function () {});
    }

    // 复位舞台 → 回到待机
    stage.hidden = true;
    standby.hidden = false;
    bubble.hidden = true;
    toast.hidden = true;
    countdown.hidden = true;
    personaWrap.hidden = true;
    var ctx = fx.getContext('2d');
    ctx.clearRect(0, 0, fx.width, fx.height);
  }

  /* ============================================================
   * Canvas 碎裂特效：裂纹 + 粒子飞散（纯视觉动画）
   * ============================================================ */
  function runCrackFx() {
    var dpr = window.devicePixelRatio || 1;
    var w = window.innerWidth;
    var h = window.innerHeight;
    fx.width = w * dpr;
    fx.height = h * dpr;
    var ctx = fx.getContext('2d');
    ctx.scale(dpr, dpr);

    var cx = w / 2;
    var cy = h * 0.42;
    var DUR = 1400;
    var start = performance.now();

    // 预生成裂纹折点
    var CRACKS = 14;
    var cracks = [];
    for (var i = 0; i < CRACKS; i++) {
      var angle = (i / CRACKS) * Math.PI * 2 + (Math.random() - 0.5) * 0.5;
      var segs = [];
      var px = cx, py = cy;
      var segLen = Math.max(w, h) * 0.12;
      for (var j = 0; j < 5; j++) {
        var a = angle + (Math.random() - 0.5) * 0.6;
        px += Math.cos(a) * segLen * (0.6 + Math.random() * 0.7);
        py += Math.sin(a) * segLen * (0.6 + Math.random() * 0.7);
        segs.push({ x: px, y: py });
      }
      cracks.push(segs);
    }
    // 粒子碎片
    var parts = [];
    for (var k = 0; k < 36; k++) {
      var ang = Math.random() * Math.PI * 2;
      parts.push({
        x: cx + (Math.random() - 0.5) * 40,
        y: cy + (Math.random() - 0.5) * 40,
        vx: Math.cos(ang) * (2 + Math.random() * 5),
        vy: Math.sin(ang) * (2 + Math.random() * 5),
        size: 3 + Math.random() * 6,
        life: 0
      });
    }

    function frame(now) {
      var t = (now - start) / DUR; // 0..1
      if (t >= 1) { ctx.clearRect(0, 0, w, h); return; }
      ctx.clearRect(0, 0, w, h);

      // 裂纹：前 40% 时间画出
      ctx.strokeStyle = 'rgba(251, 211, 228, ' + (0.9 * (1 - t)) + ')';
      ctx.lineWidth = 1.4;
      var drawUpto = Math.min(1, t / 0.4);
      cracks.forEach(function (segs) {
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        var upto = Math.floor(segs.length * drawUpto);
        for (var i = 0; i < upto; i++) ctx.lineTo(segs[i].x, segs[i].y);
        ctx.stroke();
      });

      // 碎片：后 60% 飞散淡出
      if (t > 0.4) {
        var fade = 1 - (t - 0.4) / 0.6;
        ctx.fillStyle = 'rgba(209, 115, 151, ' + (0.8 * fade) + ')';
        parts.forEach(function (p) {
          p.x += p.vx;
          p.y += p.vy;
          p.vy += 0.08; // 轻微下坠
          ctx.fillRect(p.x, p.y, p.size, p.size);
        });
      }
      state.raf = requestAnimationFrame(frame);
    }
    state.raf = requestAnimationFrame(frame);
  }

  /* ============================================================
   * 接线：事件 / 按钮 / 快捷键
   * ============================================================ */
  if (bridge) {
    bridge.listen('qibai://guard-start', function (e) {
      startShow((e && e.payload) ? e.payload : e);
    });
    standbyStatusText.textContent = bridge.isTauri
      ? '监听中 · qibai://guard-start'
      : '浏览器待机 · 可直接演示';
    if (bridge.isTauri) demoBtn.style.display = 'none';
  }

  demoBtn.addEventListener('click', function () { startShow(DEMO_PAYLOAD); });
  skipBtn.addEventListener('click', function () { endShow('user'); });
  exitBtn.addEventListener('click', function () { exitGuard('user'); });
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    // 演出中 ESC = 跳过演出回待机；待机态 ESC = 退出守护窗
    if (state.active) endShow('user');
    else exitGuard('user');
  });
  /* 退出守护窗：Tauri 下走 guard_skip（后端关窗+广播 guard-end），浏览器下直接关页 */
  function exitGuard(reason) {
    if (bridge && bridge.isTauri) {
      Promise.resolve(bridge.invoke('guard_skip', { reason: reason || 'user-exit' }))
        .catch(function () {});
      return;
    }
    try { window.close(); } catch (e) { /* mock 环境忽略 */ }
  }
})();
