/* ============================================================
 * 元气小小 Qibai — 独立面板窗逻辑（panel.js，v18）
 * 只渲染后端白名单消毒过的 HTML（qibai-core::panel），
 * 前端再执行一遍 DOM 级消毒（双保险：剥 script/iframe/on 事件/非法协议）。
 * 不加载任何外部资源（CSP: default-src 'none'）。
 * ============================================================ */
(function () {
  'use strict';

  const Q = window.QibaiBridge;
  const { invoke, listen, isTauri } = Q;

  const titleEl = document.getElementById('panelTitle');
  const contentEl = document.getElementById('panelContent');
  const emptyEl = document.getElementById('panelEmpty');
  const closeBtn = document.getElementById('panelClose');

  /* ---------- 前端消毒（与 Rust 白名单一致，DOM 级兜底） ---------- */
  const ALLOWED_TAGS = new Set([
    'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption',
    'ul', 'ol', 'li', 'div', 'span', 'p', 'br', 'b', 'strong', 'i', 'em',
    'u', 'h1', 'h2', 'h3', 'h4', 'h5', 'small', 'code', 'pre', 'details',
    'summary', 'a',
  ]);
  const ALLOWED_ATTRS = new Set(['class', 'id', 'colspan', 'rowspan', 'href', 'title', 'align']);

  function safeHref(v) {
    v = (v || '').trim().toLowerCase();
    return v.indexOf('http://') === 0 || v.indexOf('https://') === 0 ||
      v.indexOf('#') === 0 || v.indexOf('/') === 0;
  }

  function sanitizeNode(node) {
    if (!node || node.nodeType !== 1) return null; // 只处理元素节点
    const tag = node.tagName.toLowerCase();
    if (!ALLOWED_TAGS.has(tag)) return null;
    // 剥属性：只保留白名单且值安全
    const attrs = Array.prototype.slice.call(node.attributes);
    attrs.forEach(function (attr) {
      const name = attr.name.toLowerCase();
      if (!ALLOWED_ATTRS.has(name)) {
        node.removeAttribute(attr.name);
        return;
      }
      if (name === 'href' && !safeHref(attr.value)) {
        node.removeAttribute(attr.name);
        return;
      }
      if (attr.value.indexOf('<') >= 0 || attr.value.indexOf('>') >= 0 || attr.value.indexOf('"') >= 0) {
        node.removeAttribute(attr.name);
      }
    });
    // 递归清理子节点
    const kids = Array.prototype.slice.call(node.childNodes);
    kids.forEach(function (child) {
      if (child.nodeType === 1) {
        if (!sanitizeNode(child)) node.removeChild(child);
      } else if (child.nodeType === 3) {
        // 文本节点保留（生成端已转义，这里仅做最终保险）
        child.nodeValue = child.nodeValue.replace(/<\//g, '&lt;/');
      } else {
        node.removeChild(child); // 注释等一律移除
      }
    });
    return node;
  }

  function sanitizeHtml(html) {
    const doc = new DOMParser().parseFromString(html || '', 'text/html');
    const body = doc.body;
    const kids = Array.prototype.slice.call(body.childNodes);
    const keep = [];
    kids.forEach(function (child) {
      if (child.nodeType === 1 && sanitizeNode(child)) keep.push(child);
    });
    const out = document.createElement('div');
    keep.forEach(function (n) { out.appendChild(n); });
    return out.innerHTML;
  }

  /* ---------- 渲染 ---------- */
  function render(payload) {
    const title = (payload && payload.title) || '元气小小 · 面板';
    const html = (payload && payload.html) || '';
    if (titleEl) titleEl.textContent = title;
    const clean = sanitizeHtml(html);
    if (contentEl) {
      contentEl.innerHTML = clean || '<p class="qibai-empty">暂无内容～</p>';
    }
    if (emptyEl) emptyEl.classList.add('hidden');
  }

  /* ---------- 事件订阅 ---------- */
  if (listen && isTauri) {
    listen('qibai://panel-content', function (event) {
      render(event.payload || {});
    });
  } else {
    // 浏览器独立模式冒烟：URL ?html=<encoded> 或 #html=<encoded> 可预览
    var m = /html=([^&]+)/.exec(location.search) || /html=([^&]+)/.exec(location.hash);
    if (m) {
      try { render({ title: '冒烟预览', html: decodeURIComponent(m[1]) }); } catch (e) { /* 忽略 */ }
    }
  }

  // 暴露渲染与消毒接口（供冒烟页/控制台/后续插件复用）
  (globalThis || window).QibaiPanel = {
    render: render,
    sanitize: sanitizeHtml,
    showScreen: showScreen,
    refreshPerm: refreshPerm,
  };

  /* ---------- v18.3 屏幕「眼睛 + 手」 ---------- */
  const screenWrap = document.getElementById('screenWrap');
  const screenStatus = document.getElementById('screenStatus');
  const screenImg = document.getElementById('screenImg');
  const screenMarker = document.getElementById('screenMarker');
  const screenInput = document.getElementById('screenInput');
  const permStateEl = document.getElementById('permState');
  const btnType = document.getElementById('btnType');
  const btnEnter = document.getElementById('btnEnter');
  const btnEsc = document.getElementById('btnEsc');
  const btnDown = document.getElementById('btnDown');
  const btnUp = document.getElementById('btnUp');
  const btnClosePerm = document.getElementById('btnClosePerm');

  let lastNative = { w: 0, h: 0 }; // 截图原图像素尺寸（坐标换算基准）

  // 面板点击 = 用户显式授权一次（先 ack 再 click，用后即焚）
  function ackThen(fn) {
    if (isTauri && invoke) {
      invoke('screen_gui_ack', {})
        .then(function () { return fn(); })
        .catch(function (e) {
          console.warn('[screen] ack 失败', e);
          return fn();
        });
    } else if (typeof fn === 'function') {
      fn(); // 浏览器冒烟：直接执行
    }
  }

  function showScreen(payload) {
    if (!screenWrap) return;
    screenWrap.classList.remove('hidden');
    if (panelEmpty) panelEmpty.classList.add('hidden');
    if (contentEl) contentEl.innerHTML = '';
    const b64 = payload && payload.image;
    if (!b64) {
      if (screenStatus) screenStatus.innerHTML = '屏幕模式 · <span class="perm-off">截图数据缺失</span>';
      return;
    }
    lastNative.w = (payload.w || 0);
    lastNative.h = (payload.h || 0);
    if (screenImg) screenImg.src = 'data:image/png;base64,' + b64;
    if (screenStatus) screenStatus.innerHTML = '屏幕模式 · 已截图 ' + lastNative.w + '×' + lastNative.h +
      ' · 点击画面=在该处点击 · <span class="perm-off">语音「打开操作权限」可让我语音操作</span>';
    refreshPerm();
  }

  function refreshPerm() {
    if (!(isTauri && invoke)) return;
    invoke('screen_gui_state', {})
      .then(function (st) {
        if (!permStateEl) return;
        const on = st && st.enabled;
        permStateEl.innerHTML = on
          ? '<span class="perm-on">● 操作权限已开</span>'
          : '<span class="perm-off">○ 操作权限已关（面板点击可临时授权）</span>';
      })
      .catch(function () {});
  }

  if (screenImg && screenWrap) {
    screenImg.addEventListener('click', function (ev) {
      if (!lastNative.w || !lastNative.h) return;
      const rect = screenImg.getBoundingClientRect();
      if (!rect.width || !rect.height) return;
      // 显示坐标 → 物理坐标（原图比例）
      const sx = Math.round(((ev.clientX - rect.left) / rect.width) * lastNative.w);
      const sy = Math.round(((ev.clientY - rect.top) / rect.height) * lastNative.h);
      if (screenMarker) {
        screenMarker.style.left = (ev.clientX - rect.left) + 'px';
        screenMarker.style.top = (ev.clientY - rect.top) + 'px';
        screenMarker.classList.remove('hidden');
      }
      ackThen(function () {
        if (isTauri && invoke) {
          invoke('screen_click', { x: sx, y: sy }).catch(function (e) {
            if (screenStatus) screenStatus.innerHTML = '屏幕模式 · <span class="perm-off">点击失败：' + (e || '') + '</span>';
          });
        }
      });
    });
  }

  function doType() {
    const v = (screenInput && screenInput.value || '').trim();
    if (!v) return;
    ackThen(function () {
      if (isTauri && invoke) invoke('screen_type', { text: v }).catch(function () {});
      if (screenInput) screenInput.value = '';
    });
  }
  function doKey(keys) {
    ackThen(function () {
      if (isTauri && invoke) invoke('screen_key', { keys: keys }).catch(function () {});
    });
  }
  function doScroll(dy) {
    ackThen(function () {
      if (isTauri && invoke) invoke('screen_scroll', { dy: dy }).catch(function () {});
    });
  }

  if (btnType) btnType.addEventListener('click', doType);
  if (btnEnter) btnEnter.addEventListener('click', function () { doKey('Return'); });
  if (btnEsc) btnEsc.addEventListener('click', function () { doKey('Escape'); });
  if (btnDown) btnDown.addEventListener('click', function () { doScroll(3); });
  if (btnUp) btnUp.addEventListener('click', function () { doScroll(-3); });
  if (btnClosePerm) {
    btnClosePerm.addEventListener('click', function () {
      if (isTauri && invoke) {
        invoke('screen_gui_set', { enabled: false }).then(refreshPerm).catch(function () {});
      }
    });
  }
  if (screenInput) {
    screenInput.addEventListener('keydown', function (ev) {
      if (ev.key === 'Enter') { ev.preventDefault(); doType(); }
    });
  }

  if (listen) {
    // 浏览器 mock 模式下同样注册（mockEmit 走同一条总线），保证冒烟可验证
    listen('qibai://screen-shot', function (event) {
      showScreen(event.payload || {});
    });
  }
  if (!isTauri && /smoke=screen/.test(location.search) && invoke) {
    // 浏览器独立冒烟：自动触发截图 mock，验证截图展示 + 坐标点击换算
    invoke('screen_capture', {}).then(function () {
      if (screenStatus) screenStatus.innerHTML = '屏幕模式 · 冒烟：画面已展示，点击"点我"按钮测试坐标换算';
    }).catch(function () {});
  }

  /* ---------- 关闭 ---------- */
  if (closeBtn) {
    closeBtn.addEventListener('click', function () {
      if (isTauri && invoke) {
        invoke('ui_panel_close', {}).catch(function () {});
      } else {
        window.close();
      }
    });
  }
})();
