/* animacore.js — deterministic 2D animation runtime (no dependencies)
 * Contract: Scene.render(t) must be a pure function of t. Never read Date.now / Math.random.
 * Usage: <script src="animacore.js"></script>, then const scene = new AC.Scene({...})
 */
(function (global) {
  'use strict';

  // ---------------------------------------------------------------- easing
  // All take p in [0,1] and return eased value. Overshoot allowed (back/elastic/spring).
  const E = {
    linear: p => p,
    inQuad: p => p * p,
    outQuad: p => 1 - (1 - p) * (1 - p),
    inOutQuad: p => (p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2),
    inCubic: p => p * p * p,
    outCubic: p => 1 - Math.pow(1 - p, 3),
    inOutCubic: p => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2),
    outQuart: p => 1 - Math.pow(1 - p, 4),
    outQuint: p => 1 - Math.pow(1 - p, 5),
    inOutQuart: p => (p < 0.5 ? 8 * p * p * p * p : 1 - Math.pow(-2 * p + 2, 4) / 2),
    outExpo: p => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p)),
    inOutExpo: p => p === 0 ? 0 : p === 1 ? 1 : p < 0.5
      ? Math.pow(2, 20 * p - 10) / 2 : (2 - Math.pow(2, -20 * p + 10)) / 2,
    outSine: p => Math.sin((p * Math.PI) / 2),
    inOutSine: p => -(Math.cos(Math.PI * p) - 1) / 2,
    outBack: p => { const s = 1.70158; return 1 + (s + 1) * Math.pow(p - 1, 3) + s * Math.pow(p - 1, 2); },
    inBack: p => { const s = 1.70158; return (s + 1) * p * p * p - s * p * p; },
    inOutBack: p => { const s = 1.70158 * 1.525; return p < 0.5
      ? (Math.pow(2 * p, 2) * ((s + 1) * 2 * p - s)) / 2
      : (Math.pow(2 * p - 2, 2) * ((s + 1) * (p * 2 - 2) + s) + 2) / 2; },
    outElastic: p => p === 0 ? 0 : p === 1 ? 1
      : Math.pow(2, -10 * p) * Math.sin((p * 10 - 0.75) * ((2 * Math.PI) / 3)) + 1,
    outBounce: p => {
      const n = 7.5625, d = 2.75;
      if (p < 1 / d) return n * p * p;
      if (p < 2 / d) return n * (p -= 1.5 / d) * p + 0.75;
      if (p < 2.5 / d) return n * (p -= 2.25 / d) * p + 0.9375;
      return n * (p -= 2.625 / d) * p + 0.984375;
    },
    hold: p => (p >= 1 ? 1 : 0),
  };

  // ------------------------------------------------------------- time math
  const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
  const lerp = (a, b, p) => a + (b - a) * p;

  /** Progress of a tween starting at `start` lasting `dur` seconds, in [0,1]. */
  function tween(t, start, dur, easing) {
    if (dur <= 0) return t >= start ? 1 : 0;
    const p = clamp((t - start) / dur);
    const fn = typeof easing === 'function' ? easing : (E[easing] || E.outCubic);
    return fn(p);
  }

  /** Value moved from `from` to `to` following a tween. */
  function move(t, start, dur, from, to, easing) {
    return lerp(from, to, tween(t, start, dur, easing));
  }

  /** Stagger delay for index i: each step 0.08s, centered origin spreads both ways. */
  function stagger(i, count, each = 0.08, origin = 'start') {
    const from = origin === 'center' ? Math.abs(i - (count - 1) / 2)
      : origin === 'end' ? count - 1 - i : i;
    return from * each;
  }

  /** Analytic damped-spring step response. Deterministic, no integration. */
  function spring(t, start, opts = {}) {
    const { from = 0, to = 1, stiffness = 170, damping = 18, mass = 1, velocity = 0 } = opts;
    const dt = t - start;
    if (dt <= 0) return from;
    const w0 = Math.sqrt(stiffness / mass);
    const zeta = damping / (2 * Math.sqrt(stiffness * mass));
    const wd = w0 * Math.sqrt(Math.max(1e-6, 1 - zeta * zeta));
    const a = (velocity + zeta * w0 * (to - from)) / wd;
    const v = to - Math.exp(-zeta * w0 * dt) * ((to - from) * Math.cos(wd * dt) + a * Math.sin(wd * dt));
    return v;
  }

  /** Keyframe track: frames = [[time, value], ...]; eased per segment. */
  function keyframes(t, frames, easing = 'inOutCubic') {
    if (t <= frames[0][0]) return frames[0][1];
    for (let i = 1; i < frames.length; i++) {
      if (t <= frames[i][0]) {
        const [t0, v0] = frames[i - 1], [t1, v1] = frames[i];
        return lerp(v0, v1, tween(t, t0, t1 - t0, easing));
      }
    }
    return frames[frames.length - 1][1];
  }

  /** Deterministic PRNG (mulberry32) — always seed your randomness. */
  function rng(seed = 1) {
    let a = seed >>> 0;
    return function () {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  // --------------------------------------------------------------- drawing
  const FONT_STACK = '"Noto Sans CJK SC","Noto Sans",system-ui,-apple-system,"Segoe UI",Roboto,sans-serif';
  const SERIF_STACK = '"Noto Serif CJK SC","Noto Serif",Georgia,serif';

  function roundRect(ctx, x, y, w, h, r) {
    const rr = Math.min(r, Math.abs(w) / 2, Math.abs(h) / 2);
    ctx.beginPath();
    ctx.moveTo(x + rr, y);
    ctx.arcTo(x + w, y, x + w, y + h, rr);
    ctx.arcTo(x + w, y + h, x, y + h, rr);
    ctx.arcTo(x, y + h, x, y, rr);
    ctx.arcTo(x, y, x + w, y, rr);
    ctx.closePath();
  }

  function hexA(hex, alpha) {
    const h = hex.replace('#', '');
    const n = h.length === 3 ? h.split('').map(c => c + c).join('') : h;
    const r = parseInt(n.slice(0, 2), 16), g = parseInt(n.slice(2, 4), 16), b = parseInt(n.slice(4, 6), 16);
    return `rgba(${r},${g},${b},${alpha})`;
  }

  // ------------------------------------------------------------------ Scene
  class Scene {
    constructor(opts = {}) {
      this.width = opts.width || 1920;
      this.height = opts.height || 1080;
      this.fps = opts.fps || 30;
      this.duration = opts.duration || 10;
      this.background = opts.background || '#0b0d12';
      this.transparent = !!opts.transparent;
      this.layers = [];
      this._texts = [];
      this.canvas = opts.canvas || (typeof document !== 'undefined' ? document.createElement('canvas') : null);
      if (this.canvas) {
        this.canvas.width = this.width;
        this.canvas.height = this.height;
      }
      this.ctx = this.canvas.getContext('2d');
      this.ctx.textBaseline = 'alphabetic';
      if (opts.canvas && opts.mount !== false) opts.canvas.style.width = '100%';
    }

    add(layer, name) {
      this.layers.push(typeof layer === 'function' ? { name, draw: layer } : layer);
      return this;
    }

    /** Draw one frame at time t (seconds). Pure: same t -> same pixels. */
    render(t) {
      const ctx = this.ctx;
      this._texts = [];
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(0, 0, this.width, this.height);
      if (!this.transparent) {
        ctx.fillStyle = this.background;
        ctx.fillRect(0, 0, this.width, this.height);
      }
      for (const layer of this.layers) {
        ctx.save();
        layer.draw(ctx, t, this);
        ctx.restore();
      }
      return this;
    }

    /** Draw text and register its box for the legibility audit. */
    text(str, x, y, o = {}) {
      const ctx = this.ctx;
      const size = o.size || 48;
      const weight = o.weight || 500;
      const family = o.serif ? SERIF_STACK : FONT_STACK;
      ctx.font = `${weight} ${size}px ${family}`;
      ctx.fillStyle = o.color || ctx.fillStyle;
      ctx.textAlign = o.align || 'left';
      ctx.textBaseline = o.baseline || 'alphabetic';
      if ('letterSpacing' in ctx) ctx.letterSpacing = (o.letterSpacing || 0) + 'px';
      const a0 = ctx.globalAlpha;               // compose with the layer's fade
      const eff = a0 * (o.alpha == null ? 1 : o.alpha);
      if (eff < 0.04) { ctx.globalAlpha = a0; return { x: 0, y: 0, w: 0, h: 0 }; }  // not visible -> not audited
      ctx.globalAlpha = eff;
      const col = String(ctx.fillStyle);     // sampled by the contrast gate
      const m = ctx.measureText(str);
      const asc = m.actualBoundingBoxAscent || size * 0.8;
      const desc = m.actualBoundingBoxDescent || size * 0.25;
      const w = m.width;
      const left = o.align === 'center' ? x - w / 2 : o.align === 'right' ? x - w : x;
      // map the local box through the current transform so the audit sees device pixels
      const M = ctx.getTransform();
      const pt = (px, py) => ({ x: M.a * px + M.c * py + M.e, y: M.b * px + M.d * py + M.f });
      const c1 = pt(left, y - asc), c2 = pt(left + w, y + desc);
      this._texts.push({
        text: str, size: Math.abs(size * Math.sqrt(Math.abs(M.a * M.d - M.b * M.c))),
        x: Math.min(c1.x, c2.x), y: Math.min(c1.y, c2.y),
        w: Math.abs(c2.x - c1.x), h: Math.abs(c2.y - c1.y), weight,
        color: col, alpha: eff, group: o.group || null,
      });
      ctx.fillText(str, x, y);
      ctx.globalAlpha = a0;
      if ('letterSpacing' in ctx) ctx.letterSpacing = '0px';
      return { x: left, y: y - asc, w, h: asc + desc };
    }

    measure(str, o = {}) {
      const ctx = this.ctx;
      ctx.font = `${o.weight || 500} ${o.size || 48}px ${o.serif ? SERIF_STACK : FONT_STACK}`;
      if ('letterSpacing' in ctx) ctx.letterSpacing = (o.letterSpacing || 0) + 'px';
      const w = ctx.measureText(str).width;
      if ('letterSpacing' in ctx) ctx.letterSpacing = '0px';
      return w;
    }

    /**
     * Legibility audit of the frame just rendered.
     * Returns safe-area violations, text overlaps, and undersized type.
     */
    audit(margin = 0.06, minSize = 28) {
      const mx = this.width * margin, my = this.height * margin;
      const issues = [];
      const t = this._texts;
      for (const b of t) {
        if (b.x < mx || b.y < my || b.x + b.w > this.width - mx || b.y + b.h > this.height - my) {
          issues.push({ type: 'safe-area', text: b.text, box: [b.x, b.y, b.w, b.h].map(Math.round) });
        }
        if (b.size < minSize) issues.push({ type: 'tiny-type', text: b.text, size: b.size });
      }
      for (let i = 0; i < t.length; i++) {
        for (let j = i + 1; j < t.length; j++) {
          const a = t[i], b = t[j];
          if (a.group && a.group === b.group) continue; // same line/block: typeset overlap is intentional
          const ox = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
          const oy = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
          if (ox > 4 && oy > 4) issues.push({ type: 'text-overlap', texts: [a.text, b.text], overlap: [Math.round(ox), Math.round(oy)] });
        }
      }
      return { issues, texts: t.map(b => ({ text: b.text, box: [b.x, b.y, b.w, b.h].map(Math.round), size: b.size, color: b.color, alpha: b.alpha })) };
    }
  }

  // --------------------------------------------------------- motion effects
  /** Fake motion blur: redraw recent frames of a layer at low alpha. */
  function trail(ctx, t, drawAt, opts = {}) {
    const { samples = 5, span = 0.05, alpha = 0.16 } = opts;
    for (let i = samples; i >= 1; i--) {
      const tt = Math.max(0, t - (span * i) / samples);
      ctx.globalAlpha = alpha * (1 - i / (samples + 1));
      drawAt(tt);
    }
    ctx.globalAlpha = 1;
    drawAt(t);
  }

  /** Deterministic particle burst; purely analytic in t. */
  function burst(ctx, t, o = {}) {
    const {
      x = 0, y = 0, count = 40, t0 = 0, life = 0.9, seed = 7,
      speed = 420, spread = Math.PI * 2, angle = -Math.PI / 2,
      gravity = 160, size = 6, colors = ['#ffffff'], drag = 0.9, shrink = true,
    } = o;
    const dt = t - t0;
    if (dt < 0 || dt > life) return;
    const r = rng(seed);
    for (let i = 0; i < count; i++) {
      const a = angle + (r() - 0.5) * spread;
      const v = speed * (0.35 + r() * 0.65);
      const k = Math.pow(drag, dt * 60);
      const px = x + Math.cos(a) * v * ((1 - k) / (1 - drag)) * 0.016;
      const py = y + Math.sin(a) * v * ((1 - k) / (1 - drag)) * 0.016 + 0.5 * gravity * dt * dt;
      const p = clamp(dt / life);
      ctx.globalAlpha = (1 - p) * (0.4 + r() * 0.6);
      ctx.fillStyle = colors[i % colors.length];
      const s = size * (0.5 + r()) * (shrink ? 1 - p * 0.7 : 1);
      ctx.beginPath();
      ctx.arc(px, py, s, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  /** Cheap cached film grain; call after all layers. */
  let _grainTile = null;
  function grain(ctx, w, h, frame, o = {}) {
    const { alpha = 0.06, seed = 3, scale = 1 } = o;
    if (!_grainTile) {
      const c = document.createElement('canvas');
      c.width = c.height = 128;
      const g = c.getContext('2d');
      const img = g.createImageData(128, 128);
      const r = rng(seed);
      for (let i = 0; i < img.data.length; i += 4) {
        const v = 128 + (r() - 0.5) * 255;
        img.data[i] = img.data[i + 1] = img.data[i + 2] = v;
        img.data[i + 3] = 255;
      }
      g.putImageData(img, 0, 0);
      _grainTile = c;
    }
    const pattern = ctx.createPattern(_grainTile, 'repeat');
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.globalCompositeOperation = 'overlay';
    const ox = (frame * 47) % 128, oy = (frame * 71) % 128;
    ctx.translate(-ox * scale, -oy * scale);
    ctx.fillStyle = pattern;
    ctx.fillRect(0, 0, w + 128, h + 128);
    ctx.restore();
  }

  /** Radial vignette; strength 0 disables. */
  function vignette(ctx, w, h, strength = 0.35, color = '0,0,0') {
    if (!strength) return;
    const g = ctx.createRadialGradient(w / 2, h / 2, Math.min(w, h) * 0.35, w / 2, h / 2, Math.max(w, h) * 0.75);
    g.addColorStop(0, `rgba(${color},0)`);
    g.addColorStop(1, `rgba(${color},${strength})`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
  }

  /** Soft glow behind a shape: set shadow then draw. */
  function glow(ctx, color, blur = 40, alpha = 1) {
    ctx.shadowColor = color;
    ctx.shadowBlur = blur;
    ctx.globalAlpha *= alpha;
  }
  function noGlow(ctx) { ctx.shadowBlur = 0; ctx.shadowColor = 'transparent'; }

  const AC = {
    ease: E, tween, move, stagger, spring, keyframes, rng,
    clamp, lerp, roundRect, hexA, FONT_STACK, SERIF_STACK,
    Scene, trail, burst, grain, vignette, glow, noGlow,
  };

  global.AC = AC;
})(typeof window !== 'undefined' ? window : globalThis);
