#!/usr/bin/env node
/* preview.mjs — local Studio server (offline, no dependencies).
 *   node preview.mjs --dir build/916 [--port 8899]
 * Serves the folder so studio.html can read qa_report.json and audio.
 */
import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync, readdirSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const argv = process.argv.slice(2);
const arg = (k, d = null) => {
  const i = argv.indexOf('--' + k);
  return i >= 0 ? (argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : true) : d;
};
const DIR = path.resolve(arg('dir', '.'));
const PORT = +(arg('port', 8899));
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.json': 'application/json; charset=utf-8', '.wav': 'audio/wav', '.mp3': 'audio/mpeg', '.m4a': 'audio/mp4', '.png': 'image/png', '.jpg': 'image/jpeg', '.mp4': 'video/mp4', '.webm': 'video/webm', '.css': 'text/css' };

if (!existsSync(DIR)) { console.error('dir not found:', DIR); process.exit(2); }

const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://local');
  let rel = decodeURIComponent(url.pathname);
  if (rel === '/' || rel === '/studio') rel = '/studio.html';
  const file = path.join(DIR, rel);
  if (!file.startsWith(DIR) || !existsSync(file) || statSync(file).isDirectory()) {
    res.writeHead(404, { 'content-type': 'text/plain' }); res.end('404 ' + rel); return;
  }
  res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' });
  res.end(readFileSync(file));
});

server.listen(PORT, '127.0.0.1', () => {
  const base = `http://127.0.0.1:${PORT}`;
  const manifest = path.join(DIR, 'build/manifest.json');
  if (existsSync(manifest)) {
    // project root: every built format gets its own studio, audio stays reachable
    const m = JSON.parse(readFileSync(manifest, 'utf8'));
    console.log(`animacode — serving ${DIR}`);
    m.built.forEach(b => {
      const rel = path.relative(DIR, b.studio).split(path.sep).join('/');
      console.log(`  ${b.format.padEnd(4)} ${b.size.padEnd(11)} ${base}/${rel}`);
    });
  } else if (existsSync(path.join(DIR, 'studio.html'))) {
    console.log(`animacode studio → ${base}/studio.html`);
  } else {
    console.log(`serving ${DIR} → ${base}/`);
  }
});
