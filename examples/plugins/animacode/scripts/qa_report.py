#!/usr/bin/env python3
"""qa_report.py — measure the rendered sequence and produce review artifacts.

  python3 qa_report.py --frames frames --fps 30 --audit audit.json \
          --sheet contact_sheet.png --plot motion_plot.png --out qa_report.json

Checks (see references/quality-gates.md for thresholds):
  frame count / duration      FAIL on mismatch
  dead motion (>0.35s static) WARN  (intentional holds are fine — judge them)
  blank frames (<1% alpha)    FAIL
  text safe-area / overlap    FAIL on overlap, WARN on safe-area / tiny type
  luminance + colour spread   INFO (flags "dark & dull" renders)
Exit 0 = pass (warnings allowed), 1 = fail.
"""
import argparse, json, os, sys, glob, re, hashlib
import numpy as np
from PIL import Image, ImageDraw, ImageFont

def font(size):
    for p in ['/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc',
              '/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc',
              '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf']:
        if os.path.exists(p):
            try: return ImageFont.truetype(p, size)
            except Exception: pass
    return ImageFont.load_default()

def parse_color(c):
    c = (c or '').strip()
    m = re.match(r'^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$', c)
    if m:
        h = m.group(1)
        if len(h) == 3: h = ''.join(ch * 2 for ch in h)
        return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))
    m = re.match(r'^rgba?\(([^)]+)\)$', c)
    if m:
        parts = [x.strip() for x in m.group(1).split(',')]
        if len(parts) >= 3:
            try:
                return tuple(int(float(x)) for x in parts[:3])
            except ValueError:
                return None
    return None

def rel_lum(rgb):
    def ch(v):
        v = v / 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = (ch(x) for x in rgb[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b

def contrast_ratio(a, b):
    la, lb = rel_lum(a), rel_lum(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--frames', default='frames')
    ap.add_argument('--fps', type=float, default=30)
    ap.add_argument('--duration', type=float, default=None)
    ap.add_argument('--audit', default=None)
    ap.add_argument('--sheet', default=None)
    ap.add_argument('--title', default='')
    ap.add_argument('--plot', default=None)
    ap.add_argument('--out', default='qa_report.json')
    ap.add_argument('--dead-ratio', type=float, default=0.0025, help='fraction of pixels that must change per frame')
    ap.add_argument('--dead-min', type=float, default=0.45, help='seconds of static before warning')
    ap.add_argument('--dead-peak', type=float, default=2.5, help='local change amplitude; small subjects still count as motion')
    ap.add_argument('--work-w', type=int, default=720, help='downsample width for motion analysis')
    ap.add_argument('--fingerprint', default=None, help='write per-frame perceptual hashes here')
    ap.add_argument('--baseline', default=None, help='compare against a previous fingerprint file')
    ap.add_argument('--fail-on-change', action='store_true', help='treat visual diff as a failure')
    ap.add_argument('--contrast', action='store_true', default=True, help='audit text contrast (default on)')
    a = ap.parse_args()

    files = sorted(glob.glob(os.path.join(a.frames, '*.png')) + glob.glob(os.path.join(a.frames, '*.jpg')))
    if not files:
        print('FAIL: no frames in', a.frames); sys.exit(1)

    meta = None
    try:
        meta = Image.open(files[0]).size
    except Exception:
        pass

    # ---- per-frame measures on a downsampled grayscale copy
    imgs, alphas, lumas = [], [], []
    bboxes = []
    for f in files:
        im = Image.open(f).convert('RGBA')
        w, h = im.size
        g = im.convert('L').resize((a.work_w, max(1, round(a.work_w * h / w))), Image.BILINEAR)
        imgs.append(np.asarray(g, dtype=np.float32))
        arr = np.asarray(im)
        alpha = arr[:, :, 3]
        alphas.append(float((alpha > 8).mean()) if alpha.min() < 250 else 1.0)
        lumas.append(float(np.asarray(im.convert('L').resize((64, 36)), dtype=np.float32).mean()))
    stack = np.stack(imgs)
    if len(imgs) > 1:
        d = np.abs(np.diff(stack, axis=0))
        diffs = d.mean(axis=(1, 2))
        changed = (d > 8).mean(axis=(1, 2))   # share of pixels that actually moved
        k = max(1, int(d[0].size * 0.01))
        peaks = np.array([np.sort(fr.ravel())[-k:].mean() for fr in d])
    else:
        diffs = np.array([0.0]); changed = np.array([0.0]); peaks = np.array([0.0])

    fps = a.fps
    total = len(files)
    issues, warnings = [], []

    # ---- duration / count
    expected = round(fps * a.duration) if a.duration else None
    if expected and total != expected:
        issues.append(f'frame count {total} != expected {expected} ({a.duration}s @ {fps}fps)')

    # ---- blank frames
    blank = [i for i, v in enumerate(alphas) if v < 0.01]
    if blank:
        issues.append(f'{len(blank)} blank/empty frames (first: {blank[0]})')

    # ---- dead motion runs
    dead_mask = (changed < a.dead_ratio) & (peaks < a.dead_peak)
    runs, start = [], None
    for i, d in enumerate(dead_mask):
        if d and start is None: start = i
        if not d and start is not None: runs.append((start, i)); start = None
    if start is not None: runs.append((start, len(dead_mask)))
    dead_runs = [r for r in runs if (r[1] - r[0]) / fps >= a.dead_min]
    for s, e in dead_runs:
        warnings.append(f'static {s/fps:.2f}-{e/fps:.2f}s ({(e-s)/fps:.2f}s, peak {peaks[s:e].max():.1f}) — confirm it is an intentional hold')

    # ---- motion spikes (pops / single-frame flashes)
    if len(diffs) > 3:
        med = float(np.median(diffs)) or 1e-6
        spikes = [i for i, d in enumerate(diffs) if d > max(med * 12, 12)]
        if spikes:
            warnings.append(f'{len(spikes)} abrupt motion spike(s) at t≈' + ', '.join(f'{i/fps:.2f}s' for i in spikes[:6]))

    # ---- alpha bbox edge contact (only meaningful for non-full-bleed comps)
    for i, f in enumerate(files):
        if alphas[i] > 0.985: continue
        im = Image.open(f).convert('RGBA'); arr = np.asarray(im)[:, :, 3]
        ys, xs = np.where(arr > 8)
        if len(xs) == 0: continue
        bboxes.append((i, int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())))
    edge = [b[0] for b in bboxes if b[1] <= 2 or b[2] <= 2 or b[3] >= meta[0] - 3 or b[4] >= meta[1] - 3]
    if edge and len(edge) > len(bboxes) * 0.15:
        warnings.append(f'{len(edge)} frames have content touching the frame edge — check it is not accidental cropping')

    # ---- text audit
    overlap = safe = tiny = 0
    if a.audit and os.path.exists(a.audit):
        aud = json.load(open(a.audit, encoding='utf-8'))
        for fr in aud.get('audits', {}).values():
            for it in (fr or {}).get('issues', []):
                if it['type'] == 'text-overlap': overlap += 1
                elif it['type'] == 'safe-area': safe += 1
                elif it['type'] == 'tiny-type': tiny += 1
        if overlap: issues.append(f'{overlap} text overlap(s) detected by scene audit')
        if safe: warnings.append(f'{safe} text box(es) outside the 6% safe area')
        if tiny: warnings.append(f'{tiny} text element(s) under 28px')

    # ---- luminance / palette
    luma_mean = float(np.mean(lumas))
    sample = Image.open(files[len(files) // 2]).convert('RGB').resize((64, 36))
    px = np.asarray(sample, dtype=np.float32)
    lum = px.mean(axis=2)
    dark_share = float((lum < 40).mean())
    sat = float(np.mean(px.max(axis=2) - px.min(axis=2)))

    # ---- text contrast (WCAG): sample the real background behind each text box
    contrast_rows = []
    if a.contrast and a.audit and os.path.exists(a.audit):
        aud = json.load(open(a.audit, encoding='utf-8'))
        by_index = {int(os.path.basename(f).split('.')[0]): f for f in files}
        for idx, fr in aud.get('audits', {}).items():
            fpath = by_index.get(int(idx))
            if not fpath: continue
            frame = np.asarray(Image.open(fpath).convert('RGB'))
            fh, fw = frame.shape[:2]
            for t in (fr or {}).get('texts', []):
                # only judge settled text: mid-fade pixels are legitimately blended with the bg
                if t.get('alpha', 1) < 0.9: continue
                rgb = parse_color(t.get('color'))
                if not rgb: continue
                x, y, w, h = t.get('box', [0, 0, 0, 0])
                sx, sy = fw / meta[0], fh / meta[1]
                x0, y0 = int(max(0, x * sx)), int(max(0, y * sy))
                x1, y1 = int(min(fw, (x + w) * sx)), int(min(fh, (y + h) * sy))
                if x1 - x0 < 3 or y1 - y0 < 2: continue
                # sample a ring around the glyph box: the box itself is mostly text pixels,
                # so an inner crop would report the text colour as its own background
                pad = max(4, int(0.10 * max(w * sx, h * sy)))
                ex0, ey0 = max(0, x0 - pad), max(0, y0 - pad)
                ex1, ey1 = min(fw, x1 + pad), min(fh, y1 + pad)
                patch = frame[ey0:ey1, ex0:ex1]
                mask = np.ones(patch.shape[:2], dtype=bool)
                mask[y0 - ey0:y1 - ey0, x0 - ex0:x1 - ex0] = False
                ring = patch[mask]
                if len(ring) < 12: continue
                bg = tuple(int(v) for v in np.median(ring, axis=0))
                eff = tuple(int(round(c * t.get('alpha', 1) + b * (1 - t.get('alpha', 1)))) for c, b in zip(rgb, bg))
                ratio = contrast_ratio(eff, bg)
                sz = t.get('size', 0)
                need = 3.0 if sz >= 48 else (3.8 if sz >= 28 else 4.5)
                if ratio < need:
                    contrast_rows.append((float(idx) / fps, t.get('text', '')[:12], round(ratio, 2), need))

    seen_c = {}
    for tm, txt, ratio, need in contrast_rows:
        key = txt
        if key not in seen_c or ratio < seen_c[key][0]:
            seen_c[key] = (ratio, need, tm)
    for txt, (ratio, need, tm) in seen_c.items():
        msg = f'low contrast "{txt}" {ratio}:1 at {tm:.2f}s (need {need}:1)'
        if ratio < 1.5: issues.append(msg + ' — effectively invisible')
        else: warnings.append(msg)

    # ---- visual regression fingerprint
    fp = None
    if a.fingerprint or a.baseline:
        def phash(p):
            im = Image.open(p).convert('L').resize((16, 16), Image.BILINEAR)
            return hashlib.sha1(np.asarray(im, dtype=np.uint8).tobytes()).hexdigest()[:16]
        fp = dict(fps=fps, frames=[phash(f) for f in files])
        if a.fingerprint:
            json.dump(fp, open(a.fingerprint, 'w', encoding='utf-8'), indent=1)
        if a.baseline and os.path.exists(a.baseline):
            base = json.load(open(a.baseline, encoding='utf-8'))
            old = base.get('frames', [])
            diff = [i for i in range(min(len(old), len(fp['frames']))) if old[i] != fp['frames'][i]]
            if diff:
                msg = f'{len(diff)} frame(s) differ from baseline (first at {diff[0]})'
                (issues if a.fail_on_change else warnings).append(msg)
            elif len(old) != len(fp['frames']):
                warnings.append(f'baseline has {len(old)} frames, now {len(fp["frames"])}')

    # ---- artifacts
    if a.sheet:
        make_sheet(files, a.sheet, fps, title=a.title or f'{total} frames · {total/fps:.1f}s')
    if a.plot:
        make_plot(diffs, fps, a.plot, dead_runs)

    report = dict(
        frames=total, fps=fps, duration=round(total / fps, 3), size=meta,
        motion=dict(mean=round(float(diffs.mean()), 3), max=round(float(diffs.max()), 3),
                    changed_mean=round(float(changed.mean()), 4),
                    dead_seconds=round(sum((e - s) / fps for s, e in dead_runs), 2)),
        luminance=dict(mean=round(luma_mean, 1), dark_share=round(dark_share, 3), saturation=round(sat, 1)),
        text=dict(overlap=overlap, outside_safe=safe, tiny=tiny,
                  contrast_flags=len(seen_c), worst_contrast=min([r for r, _, _ in seen_c.values()], default=None)),
        issues=issues, warnings=warnings,
    )
    report['verdict'] = 'FAIL' if issues else ('PASS_WITH_WARNINGS' if warnings else 'PASS')
    json.dump(report, open(a.out, 'w', encoding='utf-8'), ensure_ascii=False, indent=1)

    print(f'frames {total} @ {fps}fps = {total/fps:.2f}s   {meta}')
    print(f'motion: mean {diffs.mean():.2f}  peak {diffs.max():.2f}  dead {report["motion"]["dead_seconds"]}s')
    print(f'luma: mean {luma_mean:.0f}  dark<40 {dark_share*100:.0f}%  sat {sat:.0f}')
    print(f'text: overlap {overlap}  outside-safe {safe}  tiny {tiny}  contrast-flags {len(seen_c)}')
    for m in issues: print('  FAIL ·', m)
    for m in warnings: print('  WARN ·', m)
    print('VERDICT:', report['verdict'])
    sys.exit(1 if issues else 0)

def make_sheet(files, out, fps, title='', cols=4, cell_w=480, rows=3, gap=10):
    n = min(len(files), cols * rows)
    idxs = [round(i * (len(files) - 1) / (n - 1)) for i in range(n)] if n > 1 else [0]
    cw, ch = cell_w, int(cell_w * 9 / 16)
    label_h = 26
    W = cols * cw + (cols + 1) * gap
    H = rows * (ch + label_h) + (rows + 1) * gap + 46
    sheet = Image.new('RGB', (W, H), (16, 17, 22))
    d = ImageDraw.Draw(sheet)
    d.text((gap, 14), title, fill=(230, 232, 240), font=font(20))
    for k, i in enumerate(idxs):
        r, c = divmod(k, cols)
        x = gap + c * (cw + gap); y = 46 + gap + r * (ch + label_h + gap)
        im = Image.open(files[i]).convert('RGB').resize((cw, ch), Image.LANCZOS)
        sheet.paste(im, (x, y))
        d.text((x, y + ch + 4), f'{i/fps:6.2f}s  #{i:05d}', fill=(150, 158, 172), font=font(15))
    sheet.save(out)

def make_plot(diffs, fps, out, dead_runs, W=1000, H=240):
    img = Image.new('RGB', (W, H), (16, 17, 22))
    d = ImageDraw.Draw(img)
    d.text((12, 8), 'motion energy per frame (mean abs diff)', fill=(200, 205, 215), font=font(14))
    top = 34; bot = H - 22
    mx = max(float(diffs.max()), 1e-6)
    for s, e in dead_runs:
        d.rectangle([int(s / len(diffs) * W), top, int(e / len(diffs) * W), bot], fill=(60, 26, 30))
    pts = [(int(i / max(1, len(diffs) - 1) * (W - 1)), int(bot - (v / mx) * (bot - top))) for i, v in enumerate(diffs)]
    d.line(pts, fill=(120, 200, 255), width=2)
    d.text((12, H - 18), 'red bands = static runs   max %.1f' % mx, fill=(140, 146, 158), font=font(13))
    img.save(out)

if __name__ == '__main__':
    main()
