#!/usr/bin/env node
/* build_scene.mjs — turn a declarative scene.json into one scene per format.
 *
 *   node build_scene.mjs --spec scene.json --out build [--format 916]
 *
 * Output: build/<id>/{scene.html,studio.html,animacore.js,blocks.js}
 * The spec is inlined into the HTML, so scenes run from file:// with no server.
 */
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync, statSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const argv = process.argv.slice(2);
const arg = (k, d = null) => {
  const i = argv.indexOf('--' + k);
  return i >= 0 ? (argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : true) : d;
};
const SPEC = arg('spec', 'scene.json');
const OUT = arg('out', 'build');
const ONLY = arg('format');
const SKILL = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const RT = path.join(SKILL, 'assets/runtime');
const TP = path.join(SKILL, 'assets/template');
const BLOCK_NAMES = ['title', 'stat', 'bars', 'process', 'quote', 'endcard'];

if (!existsSync(SPEC)) { console.error('spec not found:', SPEC); process.exit(2); }
const raw = JSON.parse(readFileSync(SPEC, 'utf8'));
const specDir = path.dirname(path.resolve(SPEC));

// ---------------------------------------------------------------- validation
const errs = [], warns = [];
const fps = +raw.fps || 30, duration = +raw.duration;
if (!(duration > 0)) errs.push('duration must be > 0');
if (!(fps > 0)) errs.push('fps must be > 0');
if (!Array.isArray(raw.shots) || !raw.shots.length) errs.push('shots[] is empty');

const ids = new Set();
(raw.shots || []).forEach((s, i) => {
  const at = `shots[${i}]`;
  if (!s.id) errs.push(`${at}: missing id`);
  if (ids.has(s.id)) errs.push(`${at}: duplicate id "${s.id}"`);
  ids.add(s.id);
  if (!BLOCK_NAMES.includes(s.block)) errs.push(`${at}: unknown block "${s.block}" (use ${BLOCK_NAMES.join('/')})`);
  if (!(typeof s.t0 === 'number' && typeof s.t1 === 'number')) errs.push(`${at}: t0/t1 must be numbers`);
  else {
    if (s.t1 <= s.t0) errs.push(`${at}: t1 must be > t0`);
    if (s.t0 < 0 || s.t1 > duration + 1e-6) errs.push(`${at}: outside 0..${duration}`);
    if (s.t1 - s.t0 < 0.4) warns.push(`${at}: only ${(s.t1 - s.t0).toFixed(2)}s on screen`);
  }
  if ((s.in && s.in.dur || 0) >= (s.t1 - s.t0)) errs.push(`${at}: in.dur >= shot length`);
});
(raw.captions || []).forEach((c, i) => {
  const at = `captions[${i}]`;
  if (!(typeof c.t0 === 'number' && typeof c.t1 === 'number')) errs.push(`${at}: t0/t1 must be numbers`);
  else {
    if (c.t1 <= c.t0) errs.push(`${at}: t1 must be > t0`);
    if (c.t1 > duration + 1e-6) errs.push(`${at}: outside 0..${duration}`);
    if ((c.text || '').length > 26) warns.push(`${at}: ${c.text.length} chars — captions over 26 chars will wrap ugly`);
  }
});
const formats = Array.isArray(raw.formats) && raw.formats.length ? raw.formats : [{ id: '169', width: 1920, height: 1080 }];
formats.forEach((f, i) => {
  if (!f.id || !f.width || !f.height) errs.push(`formats[${i}]: id/width/height required`);
});
if (ONLY && !formats.find(f => f.id === ONLY)) errs.push(`--format "${ONLY}" not in spec.formats`);

let audioAbs = null;
if (raw.audio && raw.audio.src) {
  audioAbs = path.resolve(specDir, raw.audio.src);
  if (!existsSync(audioAbs)) { audioAbs = null; warns.push(`audio missing: ${raw.audio.src} (scenes still build, mux will be skipped)`); }
}

if (errs.length) {
  console.error('SPEC INVALID\n - ' + errs.join('\n - '));
  process.exit(1);
}

// ---------------------------------------------------------------- build
const built = [];
formats.filter(f => !ONLY || f.id === ONLY).forEach(f => {
  const dir = path.join(OUT, f.id);
  mkdirSync(dir, { recursive: true });
  copyFileSync(path.join(RT, 'animacore.js'), path.join(dir, 'animacore.js'));
  copyFileSync(path.join(RT, 'blocks.js'), path.join(dir, 'blocks.js'));

  const out = {
    ...raw,
    __width: f.width,
    __height: f.height,
    fps,
    duration,
    shots: [...raw.shots].sort((a, b) => a.t0 - b.t0),
    captions: (raw.captions || []).slice().sort((a, b) => a.t0 - b.t0),
  };
  if (audioAbs) {
    const rel = path.relative(dir, audioAbs).split(path.sep).join('/');
    out.audio = { ...raw.audio, src: rel };
  }
  const json = JSON.stringify(out, null, 1).replace(/</g, '\\u003c');
  const sceneTpl = readFileSync(path.join(TP, 'spec_scene.html'), 'utf8');
  const studioTpl = readFileSync(path.join(TP, 'studio.html'), 'utf8');
  writeFileSync(path.join(dir, 'scene.html'), sceneTpl.replace('__SPEC_JSON__', json));
  writeFileSync(path.join(dir, 'studio.html'), studioTpl.replace('__SPEC_JSON__', json));
  writeFileSync(path.join(dir, 'spec.json'), JSON.stringify(out, null, 1));
  built.push({ format: f.id, size: `${f.width}x${f.height}`, dir, scene: path.join(dir, 'scene.html'), studio: path.join(dir, 'studio.html') });
});

writeFileSync(path.join(OUT, 'manifest.json'), JSON.stringify({
  spec: path.resolve(SPEC), fps, duration, shots: raw.shots.length,
  captions: (raw.captions || []).length, audio: audioAbs, built,
}, null, 1));

console.log(`built ${built.length} format(s) from ${path.basename(SPEC)} · ${fps}fps · ${duration}s · ${raw.shots.length} shots · ${(raw.captions || []).length} cues`);
built.forEach(b => console.log(`  ${b.format.padEnd(4)} ${b.size.padEnd(11)} ${path.relative(process.cwd(), b.scene)}`));
if (warns.length) console.log('warnings:\n - ' + warns.join('\n - '));
