/* blocks.js — declarative shot library for animacode v2.
 * A shot is data, not code: { id, t0, t1, block, in, out, data }.
 * Every block is a pure function of (t, shot, layout) -> canvas draw.
 * (c) 2026 animacode
 */
(function (global) {
  'use strict';
  const AC = global.AC;
  const BLOCKS = {};

  // ------------------------------------------------------------------ helpers
  const FONT = () => AC.FONT_STACK;
  const hexToRgb = h => {
    const s = (h || '#000').replace('#', '');
    const n = s.length === 3 ? s.split('').map(c => c + c).join('') : s;
    return [parseInt(n.slice(0, 2), 16), parseInt(n.slice(2, 4), 16), parseInt(n.slice(4, 6), 16)];
  };
  function mix(a, b, p) {
    const A = hexToRgb(a), B = hexToRgb(b);
    return `rgb(${Math.round(A[0] + (B[0] - A[0]) * p)},${Math.round(A[1] + (B[1] - A[1]) * p)},${Math.round(A[2] + (B[2] - A[2]) * p)})`;
  }
  function fitSize(ctx, str, base, maxW, weight = 700, ls = 0) {
    let s = base;
    ctx.font = `${weight} ${s}px ${FONT()}`;
    const w = ctx.measureText(str).width + ls * [...str].length;
    if (w > maxW) s = Math.max(24, Math.floor(s * maxW / w));
    return s;
  }
  function ease(t, a, b, e) { return AC.tween(t, a, b, e); }

  // ------------------------------------------------------------------- title
  BLOCKS.title = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const cx = L.W / 2, cy = L.box.y + L.box.h * 0.46;

    // kicker
    const kp = ease(lt, 0.05, 0.45, 'outQuad');
    const kicker = (d.kicker || '').toUpperCase();
    if (kicker) {
      const ks = Math.round(30 * L.u);
      ctx.globalAlpha = A0 * kp;
      scene.text(kicker, cx, cy - 190 * L.u, {
        size: ks, weight: 600, align: 'center', color: accent, letterSpacing: 4, group: shot.id + ':k',
      });
      ctx.globalAlpha = A0;
    }

    // headline: per-char spring rise, one group so audit does not flag typeset neighbours
    const size = fitSize(ctx, d.headline || '', Math.round((L.portrait ? 118 : 132) * L.u), L.box.w * 0.96, 800);
    const chars = [...(d.headline || '')];
    let widths = chars.map(ch => scene.measure(ch, { size, weight: 800 }));
    const total = widths.reduce((a, b) => a + b, 0);
    let x = cx - total / 2;
    chars.forEach((ch, i) => {
      const p = AC.spring(lt, 0.18 + AC.stagger(i, chars.length, 0.055, 'center'), { stiffness: 150, damping: 14 });
      if (p <= 0.001) { x += widths[i]; return; }
      const sq = 1 + 0.10 * Math.max(0, 1 - Math.abs(p - 1) * 6) * Math.sin((lt - 0.18) * 14);
      ctx.save();
      ctx.translate(x + widths[i] / 2, cy - 40 * L.u - 60 * L.u * (1 - p));
      ctx.scale(1 / sq, sq);
      ctx.globalAlpha = A0 * Math.min(1, p * 2.2);
      scene.text(ch, 0, 0, { size, weight: 800, align: 'center', baseline: 'middle', color: ink, group: shot.id + ':h' });
      ctx.restore();
      x += widths[i];
    });

    // rule
    const rw = AC.tween(lt, 0.62, 0.5, 'outQuint') * L.box.w * 0.34;
    if (rw > 8) {
      const rs = AC.spring(lt, 0.95, { stiffness: 130, damping: 12 });
      ctx.globalAlpha = A0 * Math.min(1, rw / 60);
      ctx.fillStyle = accent;
      AC.roundRect(ctx, cx - rw / 2, cy + 70 * L.u + (rs - 1) * 12, rw, 6, 3);
      ctx.fill();
    }

    // meta: bottom-left catalogue line (optional)
    if (d.meta) {
      const mp = AC.tween(lt, 1.15, 0.6, 'outQuad');
      const my = L.box.y + L.box.h - 6 * L.u;
      ctx.globalAlpha = A0 * mp;
      ctx.fillStyle = muted;
      ctx.fillRect(L.box.x, my - 46 * L.u, 44 * L.u * mp, 1.5 * L.u);
      scene.text(d.meta, L.box.x, my, {
        size: Math.round(28 * L.u), weight: 500, color: muted, letterSpacing: 2, group: shot.id + ':m',
      });
      ctx.globalAlpha = A0;
    }

    // sub
    if (d.sub) {
      const sp = AC.spring(lt, 1.0, { stiffness: 120, damping: 14 });
      const ss = Math.round((L.portrait ? 38 : 36) * L.u);
      ctx.globalAlpha = A0 * Math.min(1, sp * 1.6);
      scene.text(d.sub, cx, cy + 160 * L.u + 16 * (1 - sp), {
        size: ss, weight: 400, align: 'center', color: muted, group: shot.id + ':s',
      });
    }
  };

  // -------------------------------------------------------------------- stat
  BLOCKS.stat = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const cx = L.W / 2, cy = L.box.y + L.box.h * 0.48;
    const p = AC.tween(lt, 0.1, 0.85, 'outExpo');
    const v = Math.round(AC.lerp(0, d.value || 0, p));
    const big = Math.round((L.portrait ? 190 : 220) * L.u);
    const suffix = d.suffix || '';
    const label = d.label || '', note = d.note || '';

    const numStr = v.toLocaleString('en-US') + (suffix ? ' ' + suffix : '');
    const size = fitSize(ctx, numStr, big, L.box.w * 0.9, 800);
    const sp = AC.spring(lt, 0.1, { stiffness: 150, damping: 15 });
    ctx.save();
    ctx.translate(cx, cy - 40 * L.u);
    ctx.scale(0.96 + 0.04 * sp, 0.96 + 0.04 * sp);
    scene.text(numStr, 0, 0, { size, weight: 800, align: 'center', baseline: 'middle', color: accent, group: shot.id + ':n' });
    ctx.restore();

    if (label) {
      const lp = AC.tween(lt, 0.55, 0.5, 'outQuad');
      ctx.globalAlpha = A0 * lp;
      scene.text(label, cx, cy + size * 0.56, {
        size: Math.round((L.portrait ? 46 : 42) * L.u), weight: 500, align: 'center', color: ink, group: shot.id + ':l',
      });
    }
    if (note) {
      const np = AC.tween(lt, 0.8, 0.5, 'outQuad');
      ctx.globalAlpha = A0 * np;
      scene.text(note, cx, cy + size * 0.56 + 66 * L.u, {
        size: Math.round(30 * L.u), weight: 400, align: 'center', color: muted, group: shot.id + ':t',
      });
    }
    // baseline tick marks: a quiet sign of measurement, drawn only when visible
    const tp = AC.tween(lt, 0.2, 0.6, 'outQuint');
    if (tp > 0.02) {
      ctx.globalAlpha = A0 * 0.5 * tp;
      ctx.fillStyle = muted;
      for (let i = 0; i < 7; i++) {
        const bx = cx - L.box.w * 0.42 + i * (L.box.w * 0.84 / 6);
        ctx.fillRect(bx, cy - size * 0.92, 1.5 * L.u, (i % 2 ? 10 : 20) * L.u * tp);
      }
    }
  };

  // -------------------------------------------------------------------- bars
  BLOCKS.bars = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const items = (d.items || []).slice(0, 5);
    const max = Math.max.apply(null, items.map(i => i.value)) || 1;
    const n = items.length;
    const rowH = Math.min(L.box.h / (n + 1), 158 * L.u);
    const top = L.box.y + L.box.h * 0.55 - (n - 1) * rowH / 2;
    const labelW = Math.min(L.box.w * 0.30, 260 * L.u);
    const barX = L.box.x + labelW + 16 * L.u;
    const vsz = Math.round(30 * L.u);
    const sample = String(max) + (d.unit ? ' ' + d.unit : '');
    const reserve = 16 * L.u + scene.measure(sample, { size: vsz, weight: 600 }) + 8 * L.u;
    const barMax = L.box.w - labelW - reserve;   // the widest value must stay inside the safe area

    if (d.title) {
      ctx.globalAlpha = A0 * AC.tween(lt, 0.05, 0.4, 'outQuad');
      scene.text(d.title, L.box.x, L.box.y + 20 * L.u, {
        size: Math.round(34 * L.u), weight: 600, color: ink, group: shot.id + ':bt',
      });
    }

    items.forEach((it, i) => {
      const y = top + i * rowH - rowH * 0.5;
      const p = AC.spring(lt, 0.28 + i * 0.12, { stiffness: 145, damping: 15 });
      const bw = Math.max(0, barMax * (it.value / max) * Math.max(0, p));
      ctx.globalAlpha = A0 * Math.min(1, p * 2);
      scene.text(it.label, L.box.x, y + rowH * 0.55, {
        size: Math.round(32 * L.u), weight: 500, color: muted, group: shot.id + ':bl' + i,
      });
      if (bw > 6) {
        ctx.fillStyle = i === n - 1 ? accent : muted;
        AC.roundRect(ctx, barX, y + rowH * 0.30, bw, rowH * 0.34, rowH * 0.17);
        ctx.fill();
      }
      const vp = AC.tween(lt, 0.5 + i * 0.12, 0.6, 'outExpo');
      if (vp > 0.02) {
        ctx.globalAlpha = A0 * vp;
        // values are a right-aligned column: the column edge, not the bar end, decides the safe area
        scene.text(String(it.value) + (d.unit ? ' ' + d.unit : ''), L.box.x + L.box.w, y + rowH * 0.55, {
          size: vsz, weight: 600, align: 'right', color: ink, group: shot.id + ':bv' + i,
        });
      }
    });
  };

  // ------------------------------------------------------------------ process
  BLOCKS.process = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const steps = (d.steps || []).slice(0, 5);
    const n = steps.length;
    const portrait = L.portrait;
    const cx = L.W / 2;
    const spanP = portrait ? Math.min(L.box.h * 0.58, 860 * L.u) : 0;
    const clusterH = portrait ? spanP + 160 * L.u : 230 * L.u;
    const topY = L.box.y + (L.box.h - clusterH) / 2;
    const cy = portrait ? topY + 90 * L.u + spanP / 2 : L.box.y + L.box.h * 0.5 + 8 * L.u;
    const titleY = portrait ? topY + 26 * L.u : cy - 190 * L.u;

    if (d.title) {
      ctx.globalAlpha = A0 * AC.tween(lt, 0.05, 0.4, 'outQuad');
      scene.text(d.title, cx, titleY, {
        size: Math.round(34 * L.u), weight: 600, align: 'center', color: ink, group: shot.id + ':pt',
      });
    }

    const span = portrait ? Math.min(L.box.h * 0.58, 860 * L.u) : Math.min(L.box.w * 0.80, 1140 * L.u);
    const gap = span / Math.max(1, n - 1);
    const r = 32 * L.u;

    // connector draws first, then nodes land on top
    for (let i = 1; i < n; i++) {
      const p = AC.tween(lt, 0.35 + i * 0.16, 0.45, 'outQuint');
      if (p <= 0.01) continue;
      ctx.globalAlpha = A0 * 0.55 * Math.min(1, p * 2);
      ctx.fillStyle = muted;
      const a = portrait ? { x: cx - 96 * L.u, y: cy - span / 2 + (i - 1) * gap } : { x: cx - span / 2 + (i - 1) * gap, y: cy };
      const b = portrait ? { x: cx - 96 * L.u, y: cy - span / 2 + i * gap } : { x: cx - span / 2 + i * gap, y: cy };
      ctx.fillRect(Math.min(a.x, b.x) - 1, Math.min(a.y, b.y) - 1,
        portrait ? 2 : (b.x - a.x) * p, portrait ? (b.y - a.y) * p : 2);
    }

    steps.forEach((s, i) => {
      const p = AC.spring(lt, 0.2 + i * 0.16, { stiffness: 150, damping: 13 });
      if (p <= 0.001) return;
      const px = portrait ? cx - 96 * L.u : cx - span / 2 + i * gap;
      const py = portrait ? cy - span / 2 + i * gap : cy;
      ctx.globalAlpha = A0 * Math.min(1, p * 2);
      ctx.fillStyle = i === 0 ? accent : ink;
      ctx.beginPath(); ctx.arc(px, py, r * Math.min(1, p), 0, Math.PI * 2); ctx.fill();
      scene.text(String(i + 1), px, py + 1, {
        size: Math.round(30 * L.u), weight: 700, align: 'center', baseline: 'middle',
        color: i === 0 ? '#14110F' : (L.theme.bg || '#0E1116'), group: shot.id + ':pn' + i,
      });
      const label = s;
      const ls = Math.round((portrait ? 38 : 30) * L.u);
      // portrait: label sits to the right of the node so the connector stays clean
      const lx = portrait ? px + 58 * L.u : px;
      const ly = portrait ? py + 14 * L.u : py + 72 * L.u;
      scene.text(label, lx, ly, {
        size: ls, weight: 500, align: portrait ? 'left' : 'center', color: ink, group: shot.id + ':pl' + i,
      });
    });
  };

  // -------------------------------------------------------------------- quote
  BLOCKS.quote = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const cx = L.W / 2, cy = L.box.y + L.box.h * 0.46;
    const size = fitSize(ctx, d.text || '', Math.round((L.portrait ? 62 : 66) * L.u), L.box.w * 0.92, 600);
    const p = AC.spring(lt, 0.1, { stiffness: 140, damping: 15 });
    ctx.globalAlpha = A0 * Math.min(1, p * 2);
    scene.text(d.text || '', cx, cy + 30 * (1 - p), {
      size, weight: 600, align: 'center', color: ink, group: shot.id + ':q',
    });
    if (d.by) {
      ctx.globalAlpha = A0 * AC.tween(lt, 0.5, 0.5, 'outQuad');
      scene.text('— ' + d.by, cx, cy + size * 1.25, {
        size: Math.round(30 * L.u), weight: 400, align: 'center', color: muted, group: shot.id + ':qb',
      });
    }
  };

  // ----------------------------------------------------------------- endcard
  BLOCKS.endcard = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const accent2 = th.accent2 || '#2FA38F';
    const cx = L.W / 2, cy = L.box.y + L.box.h * 0.46;

    // mark: three bars grow in sequence, breathing after they land
    const bars = [
      { h: 150, c: accent, dx: -76 },
      { h: 230, c: ink, dx: 0 },
      { h: 110, c: accent2, dx: 76 },
    ];
    bars.forEach((b, i) => {
      const p = AC.spring(lt, 0.15 + i * 0.09, { stiffness: 150, damping: 13 });
      if (p <= 0.001) return;
      const h = b.h * L.u * p * (1 + Math.sin(t * 2.2 + i) * 0.04);
      if (h < 8) return;
      ctx.globalAlpha = A0 * Math.min(1, p * 2);
      ctx.fillStyle = b.c;
      AC.roundRect(ctx, cx + b.dx * L.u - 13 * L.u, cy - h, 26 * L.u, h, 13 * L.u);
      ctx.fill();
    });

    const brand = d.brand || 'ANIMACODE';
    const q = AC.spring(lt, 0.55, { stiffness: 120, damping: 15 });
    const bs = fitSize(ctx, brand, Math.round((L.portrait ? 92 : 96) * L.u), L.box.w * 0.88, 800, 6);
    ctx.globalAlpha = A0 * Math.min(1, q * 2);
    scene.text(brand, cx, cy + 120 * L.u + 20 * (1 - q), {
      size: bs, weight: 800, align: 'center', color: ink, letterSpacing: 6, group: shot.id + ':b',
    });

    if (d.tagline) {
      const tp = AC.tween(lt, 0.85, 0.5, 'outQuad');
      ctx.globalAlpha = A0 * tp;
      scene.text(d.tagline, cx, cy + 120 * L.u + bs + 46 * L.u, {
        size: Math.round(32 * L.u), weight: 400, align: 'center', color: muted, group: shot.id + ':tg',
      });
    }

    // no decorative sweep: the mark's slow breathing is the only motion here
  };

  // ---------------------------------------------------------------- timeline
  // 时间轴/里程碑：一条轴 + 若干节点，节点随进度依次点亮。
  // data: { title?, mid?, items:[{ label, note?, at? }] }   at = 0..1 轴上的位置
  BLOCKS.timeline = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha, u = L.u;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const items = (d.items || []).slice(0, 6);
    const n = items.length;
    if (!n) return;
    const portrait = L.portrait;

    if (d.title) {
      ctx.globalAlpha = A0 * AC.tween(lt, 0.05, 0.4, 'outQuad');
      scene.text(d.title, portrait ? L.box.x : L.W / 2, L.box.y + 8 * u, {
        size: Math.round((portrait ? 42 : 34) * u), weight: 600, align: portrait ? 'left' : 'center', color: ink, group: shot.id + ':tt',
      });
      ctx.globalAlpha = A0;
    }

    const pos = items.map((it, i) => AC.clamp(it.at == null ? (n > 1 ? i / (n - 1) : 0.5) : +it.at, 0, 1));
    const top = L.box.y + (d.title ? 86 * u : 34 * u);
    const bot = L.box.y + L.box.h - 22 * u;
    const prog = AC.tween(lt, 0.15, Math.max(0.5, d.grow || 1.2), 'outQuint');
    const portAx = L.box.x + L.box.w * 0.16;   // 竖版：轴放在左六分之一处，右侧留给标签

    if (portrait) {
      const ax = portAx;
      ctx.globalAlpha = A0 * 0.35;
      ctx.fillStyle = muted;
      ctx.fillRect(ax - 1, top, 2, bot - top);
      ctx.globalAlpha = A0 * 0.95;
      ctx.fillStyle = accent;
      ctx.fillRect(ax - 1, top, 2, (bot - top) * prog);
      // 竖版：轴在左三分之一，标签栏留足宽度的同时不压到轴
    } else {
      const ay = top + (bot - top) * 0.52;
      ctx.globalAlpha = A0 * 0.35;
      ctx.fillStyle = muted;
      ctx.fillRect(L.box.x + 16 * u, ay - 1, L.box.w - 32 * u, 2);
      ctx.globalAlpha = A0 * 0.95;
      ctx.fillStyle = accent;
      ctx.fillRect(L.box.x + 16 * u, ay - 1, (L.box.w - 32 * u) * prog, 2);
    }

    items.forEach((it, i) => {
      const p = AC.spring(lt, 0.3 + i * 0.11, { stiffness: 150, damping: 14 });
      if (p <= 0.001) return;
      const lit = AC.clamp((prog - pos[i] + 0.04) / 0.04, 0, 1);   // 轴走到节点才点亮
      const r = 13 * u * Math.min(1, p);
      ctx.globalAlpha = A0 * Math.min(1, p * 2);

      if (portrait) {
        const ny = top + (bot - top) * pos[i];
        ctx.fillStyle = lit > 0.9 ? accent : muted;
        ctx.beginPath(); ctx.arc(portAx, ny, r, 0, Math.PI * 2); ctx.fill();
        const lx = portAx + 48 * u;
        scene.text(it.label || '', lx, ny + 12 * u, {
          size: Math.round(42 * u), weight: 600, color: lit > 0.9 ? ink : muted, group: shot.id + ':i' + i,
        });
        if (it.note) scene.text(it.note, lx, ny + 58 * u, {
          size: Math.round(34 * u), weight: 400, color: muted, group: shot.id + ':n' + i,
        });
      } else {
        const nx = L.box.x + 16 * u + (L.box.w - 32 * u) * pos[i];
        const ay = top + (bot - top) * 0.52;
        ctx.fillStyle = lit > 0.9 ? accent : muted;
        ctx.beginPath(); ctx.arc(nx, ay, r, 0, Math.PI * 2); ctx.fill();
        const above = i % 2 === 0;
        const ly = ay + (above ? -52 * u : 62 * u);
        const ls = Math.round(30 * u), ns = Math.round(28 * u);
        // 首尾节点居中的标签会顶出 6% 安全区：按实际文字宽度把中心夹回来
        const mw = Math.max(scene.measure(it.label || '', { size: ls, weight: 600 }),
                            it.note ? scene.measure(it.note, { size: ns, weight: 400 }) : 0) / 2 + 4 * u;
        const tx = AC.clamp(nx, L.W * 0.06 + mw, L.W * 0.94 - mw);
        scene.text(it.label || '', tx, ly, {
          size: ls, weight: 600, align: 'center', color: lit > 0.9 ? ink : muted, group: shot.id + ':i' + i,
        });
        if (it.note) scene.text(it.note, tx, ly + (above ? -38 * u : 38 * u), {
          size: ns, weight: 400, align: 'center', color: muted, group: shot.id + ':n' + i,
        });
      }
    });
  };

  // ----------------------------------------------------------------- compare
  // 左右对比：两栏卡片，中间 VS/箭头。竖版上下堆叠。
  // data: { title?, mid?, left:{title, items[], }, right:{title, items[]} }
  BLOCKS.compare = function (ctx, t, shot, L, S, scene) {
    const d = shot.data || {}, th = L.theme;
    const lt = t - shot.t0, A0 = ctx.globalAlpha, u = L.u;
    const ink = th.ink || '#F2F0EA', accent = th.accent || '#FF5A3C', muted = th.muted || '#8C93A0';
    const portrait = L.portrait;

    if (d.title) {
      ctx.globalAlpha = A0 * AC.tween(lt, 0.05, 0.4, 'outQuad');
      scene.text(d.title, portrait ? L.box.x : L.W / 2, L.box.y + 8 * u, {
        size: Math.round((portrait ? 42 : 34) * u), weight: 600, align: portrait ? 'left' : 'center', color: ink, group: shot.id + ':ct',
      });
      ctx.globalAlpha = A0;
    }

    const top = L.box.y + (d.title ? 84 * u : 12 * u);
    const height = L.box.y + L.box.h - top - 10 * u;
    const gap = portrait ? 76 * u : Math.min(150 * u, L.box.w * 0.13);
    const rows = Math.min(4, Math.max((d.left && d.left.items || []).length, (d.right && d.right.items || []).length, 1));
    const rowH = Math.min(78 * u, 86 * u);
    // 卡片高度贴合内容，再在可用区里垂直居中：短清单不留半屏空白
    const cardH = Math.min(portrait ? (height - gap) / 2 : height, 166 * u + (rows - 1) * rowH);
    const groupH = portrait ? cardH * 2 + gap : cardH;
    const yOff = top + (height - groupH) / 2;
    const panels = portrait
      ? [{ key: 'left', x: L.box.x, y: yOff, w: L.box.w, h: cardH },
         { key: 'right', x: L.box.x, y: yOff + cardH + gap, w: L.box.w, h: cardH }]
      : [{ key: 'left', x: L.box.x, y: yOff, w: (L.box.w - gap) / 2, h: cardH },
         { key: 'right', x: L.box.x + (L.box.w - gap) / 2 + gap, y: yOff, w: (L.box.w - gap) / 2, h: cardH }];

    panels.forEach((pn, k) => {
      const side = d[pn.key] || {};
      const items = (side.items || []).slice(0, 4);
      const p = AC.spring(lt, 0.14 + k * 0.12, { stiffness: 140, damping: 15 });
      if (p <= 0.001) return;
      ctx.save();
      ctx.globalAlpha = A0 * Math.min(1, p * 1.8);
      ctx.translate(0, 26 * u * (1 - p));
      // 面板底色 = 前景色 6% 提亮，保证正文对比度不被拉低
      ctx.fillStyle = AC.hexA(k ? accent : muted, 0.07);
      AC.roundRect(ctx, pn.x, pn.y, pn.w, pn.h, 18 * u);
      ctx.fill();
      const padX = 34 * u, innerW = pn.w - padX * 2;
      const hs = fitSize(ctx, side.title || '', Math.round(38 * u), innerW, 700);
      scene.text(side.title || '', pn.x + padX, pn.y + 62 * u, {
        size: hs, weight: 700, color: k ? accent : ink, group: shot.id + ':c' + k + 'h',
      });
      const baseY = pn.y + 62 * u + 58 * u;
      items.forEach((it, i) => {
        const ip = AC.tween(lt, 0.32 + k * 0.1 + i * 0.1, 0.4, 'outQuad');
        if (ip <= 0.01) return;
        const y = baseY + i * rowH;
        ctx.globalAlpha = A0 * ip;
        ctx.fillStyle = k ? accent : muted;
        ctx.fillRect(pn.x + padX, y - 20 * u, 18 * u * ip, 3 * u);
        const is = fitSize(ctx, String(it), Math.round(31 * u), innerW - 34 * u, 500);
        scene.text(String(it), pn.x + padX + 32 * u, y, {
          size: is, weight: 500, color: ink, group: shot.id + ':c' + k + 'i' + i,
        });
      });
      ctx.restore();
    });

    // middle marker
    const mp = AC.tween(lt, 0.6, 0.4, 'outQuad');
    if (mp > 0.02) {
      ctx.globalAlpha = A0 * mp;
      const mid = d.mid || 'VS';
      if (portrait) {
        const my = yOff + cardH + gap / 2 + 14 * u;
        scene.text(mid, L.box.x + L.box.w / 2, my, {
          size: Math.round(34 * u), weight: 700, align: 'center', color: accent, group: shot.id + ':cm',
        });
      } else {
        scene.text(mid, L.box.x + L.box.w / 2, yOff + cardH / 2, {
          size: Math.round(30 * u), weight: 700, align: 'center', baseline: 'middle', color: accent, group: shot.id + ':cm',
        });
      }
    }
  };

  // ---------------------------------------------------------------- captions
  // Global layer, drawn after all shots. Karaoke highlight follows cue progress.
  function renderCaptions(ctx, t, spec, L, scene) {
    const cues = spec.captions || [];
    const th = L.theme, A0 = ctx.globalAlpha;
    const size = Math.round((L.portrait ? 46 : 40) * L.u);
    const ink = th.ink || '#FFFFFF', accent = th.accent || '#FF5A3C';
    // 胶囊底色跟主题明度走：亮底主题用浅色胶囊，深底主题用深色胶囊。
    // 否则亮底主题里"深墨字压在深胶囊上"，实测对比度只有 1.0–2.9:1。
    const bgc = hexToRgb(th.bg || '#0E1116');
    const lin = v => { v /= 255; return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
    const lightTheme = 0.2126 * lin(bgc[0]) + 0.7152 * lin(bgc[1]) + 0.0722 * lin(bgc[2]) > 0.45;
    const capFill = lightTheme ? AC.hexA(th.ink || '#17181B', 0.10) : 'rgba(10,10,12,0.62)';
    const bottom = L.H - (L.portrait ? L.H * 0.13 : L.H * 0.105);
    cues.forEach((c, ci) => {
      if (t < c.t0 || t > c.t1) return;
      const pin = AC.tween(t, c.t0, 0.2, 'outQuad');
      const pout = 1 - AC.tween(t, c.t1 - 0.2, 0.2, 'inQuad');
      const a = Math.min(pin, pout);
      if (a < 0.03) return;
      const text = c.text || '';
      const fullW = scene.measure(text, { size, weight: 600 });
      const padX = 26 * L.u, padY = 18 * L.u;
      const bw = Math.min(fullW + padX * 2, L.W * 0.86);
      const bh = size * 1.5 + padY;
      ctx.save();
      ctx.globalAlpha = A0 * a;
      ctx.fillStyle = capFill;
      AC.roundRect(ctx, L.W / 2 - bw / 2, bottom - bh / 2, bw, bh, bh / 2);
      ctx.fill();
      const prog = (t - c.t0) / Math.max(0.001, (c.t1 - c.t0));
      const n = [...text].length;
      const k = Math.floor(Math.min(1, prog * 1.15) * n);
      const x0 = L.W / 2 - fullW / 2;
      const head = [...text].slice(0, k).join('');
      const tail = [...text].slice(k).join('');
      if (head) scene.text(head, x0, bottom + size * 0.36, {
        size, weight: 600, color: accent, group: 'cap' + ci,
      });
      if (tail) {
        const hw = scene.measure(head, { size, weight: 600 });
        scene.text(tail, x0 + hw, bottom + size * 0.36, {
          size, weight: 600, color: ink, group: 'cap' + ci,
        });
      }
      ctx.restore();
    });
  }

  global.AC.BLOCKS = BLOCKS;
  global.AC.renderCaptions = renderCaptions;

  global.AC.makeLayout = function (spec, W, H) {
    const portrait = H > W;
    const u = Math.min(W, H) / 1080;
    const mx = Math.round(W * (portrait ? 0.08 : 0.07));
    const box = portrait
      ? { x: mx, y: Math.round(H * 0.15), w: W - 2 * mx, h: Math.round(H * 0.54) }
      : { x: mx, y: Math.round(H * 0.13), w: W - 2 * mx, h: Math.round(H * 0.64) };
    return { W, H, portrait, u, box, theme: spec.theme || {} };
  };
})(typeof window !== 'undefined' ? window : globalThis);
