#!/usr/bin/env node
/* render_frames.mjs — deterministic frame capture via headless Chromium + CDP.
 * No npm dependencies. Requires: chromium/chrome on PATH (or CHROME env), Node >= 18.
 *
 *   node render_frames.mjs --in scene.html --out frames [--fps 30] [--duration 8]
 *          [--width 1920] [--height 1080] [--alpha] [--workers 2 --worker 0]
 *          [--frames 0,45,120]  [--audit audit.json]
 *
 * Output: <out>/00000.png ... one file per frame, plus optional audit JSON.
 */
import { spawn, execSync } from 'node:child_process';
import { writeFileSync, mkdirSync, existsSync, copyFileSync, readFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const argv = process.argv.slice(2);
const arg = (k, d = null) => {
  const i = argv.indexOf('--' + k);
  return i >= 0 ? (argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : true) : d;
};
const IN = arg('in');
if (!IN) { console.error('missing --in <scene.html>'); process.exit(2); }
const OUT = arg('out', 'frames');
const W = arg('width') ? +arg('width') : null, H = arg('height') ? +arg('height') : null;
const ALPHA = !!arg('alpha', false);
const FORMAT = ALPHA ? 'png' : String(arg('format', 'jpeg'));
const QUALITY = +(arg('quality', 96));
const EXT = FORMAT === 'png' ? 'png' : 'jpg';
const WORKERS = +(arg('workers', 1)), WORKER = +(arg('worker', 0));
function parseFrameList(v) {
  const out = [];
  for (const part of String(v).split(',')) {
    const t = part.trim();
    if (!t) continue;
    const m = t.match(/^(\d+)\s*[-:]\s*(\d+)$/);
    if (m) { const a = +m[1], b = +m[2]; for (let i = Math.min(a, b); i <= Math.max(a, b); i++) out.push(i); }
    else if (/^\d+$/.test(t)) out.push(+t);
    else throw new Error('bad --frames item: ' + t);
  }
  return [...new Set(out)].sort((x, y) => x - y);
}
const ONLY = arg('frames') ? parseFrameList(arg('frames')) : null;
const CLEAN = !!arg('clean', false);
const AUDIT = arg('audit');
const SHOTS = arg('shots') ? String(arg('shots')).split(',').map(x => x.trim()).filter(Boolean) : null;
const REUSE = arg('reuse');
const TIMEOUT = +(arg('timeout', 45)) * 1000;

function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  for (const c of ['chromium', 'chromium-browser', 'google-chrome', 'google-chrome-stable', '/usr/local/bin/chromium']) {
    try { return execSync(`command -v ${c}`, { encoding: 'utf8' }).trim(); } catch {}
  }
  throw new Error('no chromium found; set CHROME=/path/to/chrome');
}

const fileUrl = 'file://' + path.resolve(IN) + '?render=1';
mkdirSync(OUT, { recursive: true });

const chrome = spawn(findChrome(), [
  '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
  '--hide-scrollbars', '--force-device-scale-factor=1', '--disable-lcd-text',
  `--window-size=${W || 1920},${H || 1080}`, '--remote-debugging-port=0', fileUrl,
], { stdio: ['ignore', 'pipe', 'pipe'] });

const wsUrl = await new Promise((res, rej) => {
  let buf = '';
  const timer = setTimeout(() => rej(new Error('chromium did not start in 20s\n' + buf)), 20000);
  chrome.stderr.on('data', d => {
    buf += d;
    const m = buf.match(/DevTools listening on (ws:\/\/[^\s]+)/);
    if (m) { clearTimeout(timer); res(m[1]); }
  });
  chrome.on('exit', c => { clearTimeout(timer); rej(new Error('chromium exited early: ' + c)); });
});

const ws = new WebSocket(wsUrl);
await new Promise((r, j) => { ws.addEventListener('open', r); ws.addEventListener('error', j); });
let seq = 0;
const pending = new Map();
ws.addEventListener('message', ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
});
const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = ++seq;
  pending.set(id, m => m.error ? rej(new Error(`${method}: ${JSON.stringify(m.error)}`)) : res(m.result || {}));
  ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
});

const { targetInfos } = await send('Target.getTargets');
const page = targetInfos.find(t => t.type === 'page');
const { sessionId } = await send('Target.attachToTarget', { targetId: page.targetId, flatten: true });
await send('Page.enable', {}, sessionId);
if (ALPHA) await send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } }, sessionId);

const evalJs = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true }, sessionId);
  if (r.exceptionDetails) throw new Error('scene error: ' + (r.exceptionDetails.exception?.description || r.exceptionDetails.text));
  return r.result?.value;
};

const t0 = Date.now();
let ready = false;
while (Date.now() - t0 < TIMEOUT) {
  if (await evalJs('typeof window.__ready !== "undefined" && window.__ready === true')) { ready = true; break; }
  await new Promise(r => setTimeout(r, 150));
}
if (!ready) throw new Error('scene never set window.__ready (check fonts/assets or a JS error)');

// the scene owns its pixel size; multi-format renders must not be cropped to a CLI default
const box = JSON.parse(await evalJs('JSON.stringify({w:window.__ANIMA.width,h:window.__ANIMA.height})'));
const VW = W || box.w, VH = H || box.h;
await send('Emulation.setDeviceMetricsOverride', { width: VW, height: VH, deviceScaleFactor: 1, mobile: false }, sessionId);

const fps = +(await evalJs('window.__ANIMA?.fps')) || +(arg('fps', 30));
const duration = +(await evalJs('window.__ANIMA?.duration')) || +(arg('duration', 8));
const total = Math.round(fps * duration);
const all = Array.from({ length: total }, (_, i) => i).filter(i => i % WORKERS === WORKER);
let list = ONLY || all;

// incremental: render only the frames covered by the named shots, reuse the rest
if (SHOTS) {
  const shots = JSON.parse(await evalJs('JSON.stringify(window.__ANIMA.shots || [])'));
  const picked = shots.filter(s => SHOTS.includes(s.id));
  if (!picked.length) throw new Error('no shots matched --shots ' + SHOTS.join(','));
  list = list.filter(i => { const t = i / fps; return picked.some(s => t >= s.t0 && t <= s.t1); });
  process.stderr.write(`incremental: ${picked.map(s => s.id).join(',')} → ${list.length}/${total} frames\n`);
}

const reused = [], missing = [];

const audits = {};
const t1 = Date.now();
let done = 0;
for (const i of list) {
  const t = (i / fps).toFixed(6);
  await evalJs(`window.__ANIMA.seek(${t})`);
  const params = FORMAT === 'png'
    ? { format: 'png', fromSurface: true, captureBeyondViewport: false }
    : { format: 'jpeg', quality: QUALITY, fromSurface: true, captureBeyondViewport: false };
  const shot = await send('Page.captureScreenshot', params, sessionId);
  writeFileSync(path.join(OUT, String(i).padStart(5, '0') + '.' + EXT), Buffer.from(shot.data, 'base64'));
  if (AUDIT && (i % Math.max(1, Math.round(fps / 6)) === 0 || i === list[list.length - 1])) {
    audits[i] = await evalJs('window.__ANIMA.audit()');
  }
  done++;
  if (done % 30 === 0) process.stdout.write(`. ${done}/${list.length}\n`);
}
// copy untouched frames from a previous render so QA still sees the whole timeline
if (REUSE) {
  for (const i of all) {
    if (list.includes(i)) continue;
    const src = path.join(REUSE, String(i).padStart(5, '0') + '.' + EXT);
    const dst = path.join(OUT, String(i).padStart(5, '0') + '.' + EXT);
    if (src !== dst && existsSync(src)) { copyFileSync(src, dst); reused.push(i); } else if (!existsSync(dst)) missing.push(i);
  }
}

if (AUDIT) {
  const prev = (() => { try { return JSON.parse(readFileSync(AUDIT, 'utf8')).audits || {}; } catch { return {}; } })();
  writeFileSync(AUDIT, JSON.stringify({ fps, duration, frames: total, audits: { ...prev, ...audits } }, null, 1));
}

// a shorter cut leaves the tail of the previous cut on disk: QA counts them,
// ffmpeg happily encodes them, and the video ends up longer than the audio.
let stale = [];
try {
  const dir = await (await import('node:fs/promises')).readdir(OUT);
  stale = dir.map(n => n.match(/^(\d{5})\.(jpg|png)$/)).filter(Boolean).map(m => +m[1]).filter(i => i >= total);
} catch {}
if (stale.length) {
  if (CLEAN) { for (const i of stale) { try { (await import('node:fs')).unlinkSync(path.join(OUT, String(i).padStart(5, '0') + '.' + EXT)); } catch {} } console.log('removed ' + stale.length + ' stale frame(s) beyond #' + (total - 1)); }
  else console.log('WARNING: ' + stale.length + ' stale frame(s) on disk beyond #' + (total - 1) + ' (older longer cut?) — rerun with --clean');
}

const ms = Date.now() - t1;
console.log(JSON.stringify({ rendered: list.length, reused: reused.length, missing: missing.length, stale, fps, duration, total, format: EXT, ms, perFrameMs: +(ms / Math.max(1, list.length)).toFixed(1) }));
if (missing.length) console.log('missing frames (no source in --reuse): ' + (missing.length > 8 ? missing.slice(0, 8).join(',') + '…' : missing.join(',')));
ws.close();
chrome.kill('SIGKILL');
process.exit(0);
