#!/usr/bin/env node
/* animacode MCP server — stdio, JSON-RPC 2.0, zero dependencies.
 * Wire it into a local agent (Tauri/Rust, Ollama, any MCP client):
 *   { "command": "node", "args": ["<skill>/mcp/server.mjs"], "env": { "CHROME": "/usr/bin/chromium" } }
 */
import { createInterface } from 'node:readline';
import { execFile } from 'node:child_process';
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SKILL = path.resolve(HERE, '..');
const S = p => path.join(SKILL, 'scripts', p);

const run = (cmd, args, { timeout = 600000, cwd } = {}) => new Promise(res => {
  execFile(cmd, args, { timeout, cwd, maxBuffer: 4 * 1024 * 1024, env: process.env }, (err, stdout, stderr) => {
    res({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout || '').trim(), stderr: String(stderr || '').trim() });
  });
});

const must = (v, name) => { if (v == null || v === '') throw new Error(`${name} is required`); return v; };
const tailJson = txt => { const lines = txt.split('\n').filter(Boolean); for (let i = lines.length - 1; i >= 0; i--) { try { return JSON.parse(lines[i]); } catch {} } return null; };

// ------------------------------------------------------------------ tools
const TOOLS = {
  anima_schema: {
    description: 'Return the scene.json schema and the six block data fields. Call this before writing a spec — it is cheaper than guessing.',
    inputSchema: { type: 'object', properties: {} },
    async run() {
      const a = readFileSync(path.join(SKILL, 'references/spec-schema.md'), 'utf8');
      const b = readFileSync(path.join(SKILL, 'references/blocks.md'), 'utf8');
      return a + '\n\n---\n\n' + b;
    },
  },
  anima_build: {
    description: 'Validate a scene.json and build one scene per format. Returns built formats or the exact list of spec errors. Fix and call again.',
    inputSchema: {
      type: 'object',
      properties: {
        spec: { type: 'string', description: 'absolute path to scene.json' },
        out: { type: 'string', description: 'output dir, default build next to spec' },
        format: { type: 'string', description: 'only build this format id, e.g. 916' },
      },
      required: ['spec'],
    },
    async run({ spec, out, format }) {
      must(spec, 'spec');
      if (!existsSync(spec)) throw new Error('spec not found: ' + spec);
      const r = await run(process.execPath, [S('build_scene.mjs'), '--spec', spec, '--out', out || path.join(path.dirname(spec), 'build'), ...(format ? ['--format', format] : [])], { timeout: 30000 });
      if (r.code !== 0) return { isError: true, text: r.stderr || r.stdout };
      const manifest = JSON.parse(readFileSync(path.join(out || path.join(path.dirname(spec), 'build'), 'manifest.json'), 'utf8'));
      return { text: r.stdout, data: { built: manifest.built.map(b => ({ format: b.format, size: b.size, scene: b.scene })) } };
    },
  },
  anima_render: {
    description: 'Render frames with headless Chromium. For a single changed shot use shots+reuse so untouched frames are copied instead of re-rendered.',
    inputSchema: {
      type: 'object',
      properties: {
        scene: { type: 'string', description: 'absolute path to build/<id>/scene.html' },
        out: { type: 'string', description: 'frames dir' },
        shots: { type: 'array', items: { type: 'string' }, description: 'only these shot ids, e.g. ["s4"]' },
        reuse: { type: 'string', description: 'previous frames dir to copy untouched frames from' },
        frames: { type: 'array', items: { type: 'number' }, description: 'only these frame indices (sample review)' },
        audit: { type: 'string', description: 'audit json output path' },
      },
      required: ['scene', 'out'],
    },
    async run({ scene, out, shots, reuse, frames, audit }) {
      must(scene, 'scene'); must(out, 'out');
      const args = [S('render_frames.mjs'), '--in', scene, '--out', out];
      if (shots && shots.length) args.push('--shots', shots.join(','));
      if (reuse) args.push('--reuse', reuse);
      if (frames && frames.length) args.push('--frames', frames.join(','));
      if (audit) args.push('--audit', audit);
      const r = await run(process.execPath, args, { timeout: 900000, cwd: path.dirname(scene) });
      const json = tailJson(r.stdout);
      if (r.code !== 0) return { isError: true, text: (r.stderr || r.stdout).slice(0, 4000) };
      return { text: r.stdout.slice(-1200), data: json };
    },
  },
  anima_qa: {
    description: 'Run the quality gate on rendered frames. Returns verdict, issues, warnings and metrics. FAIL must be fixed; warnings must be judged.',
    inputSchema: {
      type: 'object',
      properties: {
        frames: { type: 'string', description: 'frames dir' },
        fps: { type: 'number' },
        duration: { type: 'number' },
        audit: { type: 'string' },
        out: { type: 'string', description: 'qa_report.json path; put it inside build/<id>/ so Studio can read it' },
        sheet: { type: 'string' }, plot: { type: 'string' },
        baseline: { type: 'string' }, failOnChange: { type: 'boolean' },
      },
      required: ['frames', 'fps', 'duration', 'out'],
    },
    async run({ frames, fps, duration, audit, out, sheet, plot, baseline, failOnChange }) {
      const args = [S('qa_report.py'), '--frames', frames, '--fps', String(fps), '--duration', String(duration), '--out', out];
      if (audit) args.push('--audit', audit);
      if (sheet) args.push('--sheet', sheet);
      if (plot) args.push('--plot', plot);
      if (baseline) args.push('--baseline', baseline);
      if (failOnChange) args.push('--fail-on-change');
      const r = await run('python3', args, { timeout: 180000 });
      let report = null;
      try { report = JSON.parse(readFileSync(out, 'utf8')); } catch {}
      if (!report) return { isError: true, text: (r.stderr || r.stdout).slice(0, 4000) };
      return {
        isError: report.verdict === 'FAIL',
        text: `VERDICT ${report.verdict}\n` + [...report.issues.map(i => 'FAIL · ' + i), ...report.warnings.map(w => 'WARN · ' + w)].join('\n'),
        data: { verdict: report.verdict, issues: report.issues, warnings: report.warnings, motion: report.motion, text: report.text, luminance: report.luminance },
      };
    },
  },
  anima_mux: {
    description: 'Mux frames into a delivery MP4; pass audio to get an AAC track.',
    inputSchema: {
      type: 'object',
      properties: {
        frames: { type: 'string' }, out: { type: 'string' },
        fps: { type: 'number' }, crf: { type: 'number' }, audio: { type: 'string' },
      },
      required: ['frames', 'out'],
    },
    async run({ frames, out, fps = 30, crf = 16, audio }) {
      const args = [S('make_video.sh'), frames, out, String(fps), String(crf)];
      if (audio) args.push(audio);
      const r = await run('bash', args, { timeout: 180000 });
      if (r.code !== 0) return { isError: true, text: (r.stderr || r.stdout).slice(0, 3000) };
      return { text: r.stdout, data: { out } };
    },
  },
};

// ------------------------------------------------------------------ protocol
const send = msg => process.stdout.write(JSON.stringify(msg) + '\n');
const ok = (id, result) => send({ jsonrpc: '2.0', id, result });
const err = (id, code, message) => send({ jsonrpc: '2.0', id, error: { code, message } });

const rl = createInterface({ input: process.stdin });
rl.on('line', async line => {
  if (!line.trim()) return;
  let msg;
  try { msg = JSON.parse(line); } catch { return err(null, -32700, 'parse error'); }
  if (msg.method === 'notifications/initialized' || msg.method === 'initialized') return;
  if (msg.id == null) return;

  if (msg.method === 'initialize') {
    return ok(msg.id, {
      protocolVersion: '2024-11-05',
      capabilities: { tools: { listChanged: false } },
      serverInfo: { name: 'animacode', version: '2.1.0' },
    });
  }
  if (msg.method === 'tools/list') {
    return ok(msg.id, {
      tools: Object.entries(TOOLS).map(([name, t]) => ({ name, description: t.description, inputSchema: t.inputSchema })),
    });
  }
  if (msg.method === 'tools/call') {
    const { name, arguments: a = {} } = msg.params || {};
    const tool = TOOLS[name];
    if (!tool) return err(msg.id, -32602, 'unknown tool: ' + name);
    try {
      const out = await tool.run(a);
      return ok(msg.id, {
        content: [{ type: 'text', text: typeof out === 'string' ? out : out.text || '' }],
        ...(out && typeof out === 'object' && out.data ? { structuredContent: out.data } : {}),
        isError: !!(out && out.isError),
      });
    } catch (e) {
      return ok(msg.id, { content: [{ type: 'text', text: 'error: ' + e.message }], isError: true });
    }
  }
  if (msg.method === 'ping') return ok(msg.id, {});
  return err(msg.id, -32601, 'method not found: ' + msg.method);
});
process.stderr.write('animacode mcp ready (stdio)\n');
