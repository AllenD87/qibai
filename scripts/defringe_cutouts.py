#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
defringe_cutouts.py — 立绘抠图白边清理（前端 UI 修复·需求一）

背景：AI 抠图产出的 RGBA 图 alpha 通道是劣质「白键」结果——
  背景雾、轮廓脏边、以及白袍/白袜等亮部布料全是半透明近白像素，
  颜色无法区分「背景雾」与「亮部布料」，必须用空间连通性区分：

处理两类资产：
  A. RGBA 半透明图（src/assets/ai_char_3d/*.png、src/assets/personas/*/character.png，共 10 张）
     算法（泛洪 + 提升）：
       1) 从已有 alpha=0 区域出发，四邻域泛洪「半透明(0<a<255) 且 脏边色
          （近白 min(r,g,b)>=232 或 低饱和中性灰 max-min<=30）」的像素 → 这些是
          与背景连通的脏边/背景雾 → alpha 置 0。
       2) 其余未被泛洪到达的半透明像素（被不透明身体包围的亮部布料/亮部高光，
          是劣质白键误伤的衣身）→ alpha 提升到 255（补全衣身，不透明）。
       3) alpha 通道做 3x3 均值模糊（BoxBlur r=1）平滑边缘。
     覆盖保存原文件名/原尺寸。

  B. 白底 RGB 原图（src/assets/ai_char/<key>_raw.png，共 5 张）
     - 从图像四条边界泛洪「近白」区域 → alpha=0（白→透明，不损伤衣内白色）
     - 边缘残留半透明脏边（近白/中性灰）再清一次
     - alpha 通道 1px 均值模糊平滑
     - 输出为 src/assets/ai_char/<key>.png（_raw 原图保留不动）

原图先备份到 /tmp/defringe_backup/。
运行：python3 scripts/defringe_cutouts.py
"""
import os
import shutil
from collections import deque

import numpy as np
from PIL import Image, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, 'src', 'assets')
BACKUP = '/tmp/defringe_backup'
WHITE_MIN = 232      # min(r,g,b) >= 232 视为近白
GRAY_SAT_MAX = 30    # max-min <= 30 视为低饱和中性灰


def fringe_mask(r, g, b, a):
    """半透明(0<a<255) 且 脏边色（近白 或 中性灰）的布尔数组。"""
    semi = (a > 0) & (a < 255)
    near_white = (r >= WHITE_MIN) & (g >= WHITE_MIN) & (b >= WHITE_MIN)
    neutral = (np.abs(r.astype(int) - g.astype(int)) <= GRAY_SAT_MAX) & \
              (np.abs(g.astype(int) - b.astype(int)) <= GRAY_SAT_MAX) & \
              (np.abs(r.astype(int) - b.astype(int)) <= GRAY_SAT_MAX)
    return semi & (near_white | neutral)


def count_fringe(im):
    """统计可见脏边：0<a<255 且 脏边色 的像素数。返回 (count, total)。"""
    im = im.convert('RGBA')
    arr = np.array(im)
    m = fringe_mask(arr[..., 0], arr[..., 1], arr[..., 2], arr[..., 3])
    return int(m.sum()), arr.shape[0] * arr.shape[1]


def defringe_rgba(im):
    """A 类（任务书口径）：0<a<255 且近白(min(r,g,b)>=232) 的像素 → alpha 0；
    其余半透明像素原样保留（白袍/亮部衣身为低亮白，min<232，不误伤）；
    最后对 alpha 通道做 3x3 均值模糊平滑。返回 (新图, 清除数, 0)。"""
    im = im.convert('RGBA')
    arr = np.array(im)
    r, g, b, a = arr[..., 0], arr[..., 1], arr[..., 2], arr[..., 3]
    near_white = (r >= WHITE_MIN) & (g >= WHITE_MIN) & (b >= WHITE_MIN)
    kill = (a > 0) & (a < 255) & near_white
    cleared = int(kill.sum())
    arr[kill, 3] = 0
    out = Image.fromarray(arr, 'RGBA')
    alpha = out.getchannel('A').filter(ImageFilter.BoxBlur(1))
    out.putalpha(alpha)
    return out, cleared, 0


def cutout_white_bg(im):
    """B 类：白底 RGB → RGBA。边界泛洪抠白 + 边缘脏边清理 + alpha 平滑。返回 (新图, 清除数)。"""
    im = im.convert('RGBA')
    arr = np.array(im)
    h, w = arr.shape[:2]
    r, g, b, _ = arr[..., 0], arr[..., 1], arr[..., 2], arr[..., 3]
    near_white = (r >= WHITE_MIN) & (g >= WHITE_MIN) & (b >= WHITE_MIN)

    # 1) 从四边泛洪近白 → 背景
    bg = np.zeros((h, w), dtype=bool)
    dq = deque()

    def seed(x, y):
        if not bg[y, x] and near_white[y, x]:
            bg[y, x] = True
            dq.append((y, x))

    for x in range(0, w, 2):
        seed(x, 0)
        seed(x, h - 1)
    for y in range(0, h, 2):
        seed(0, y)
        seed(w - 1, y)
    while dq:
        y, x = dq.popleft()
        for ny, nx in ((y+1, x), (y-1, x), (y, x+1), (y, x-1)):
            if 0 <= ny < h and 0 <= nx < w and not bg[ny, nx] and near_white[ny, nx]:
                bg[ny, nx] = True
                dq.append((ny, nx))

    arr[bg, 3] = 0
    removed = int(bg.sum())

    # 2) 边缘残留半透明脏边（近白/中性灰）→ alpha 0
    fringe = fringe_mask(arr[..., 0], arr[..., 1], arr[..., 2], arr[..., 3])
    arr[fringe, 3] = 0
    removed += int(fringe.sum())

    out = Image.fromarray(arr, 'RGBA')
    alpha = out.getchannel('A').filter(ImageFilter.BoxBlur(1))
    out.putalpha(alpha)
    return out, removed


def backup(path):
    rel = os.path.relpath(path, ASSETS)
    dst = os.path.join(BACKUP, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    if os.path.exists(dst):
        return dst  # 已存在则保留首次备份（原图），不被处理后覆盖
    shutil.copy2(path, dst)
    return dst


def process_rgba_path(path):
    before, total = count_fringe(Image.open(path))
    backup(path)
    out, cleared, boosted = defringe_rgba(Image.open(path))
    out.save(path)
    after, _ = count_fringe(Image.open(path))
    print(f'[RGBA] {os.path.relpath(path, ROOT)}  脏边 {before} → {after}（清 {cleared} / 补全衣身 {boosted} / 共 {total}px）')


def process_raw_to_transparent(raw_path, out_path):
    im = Image.open(raw_path)
    out, removed = cutout_white_bg(im)
    out.save(out_path)
    after, total = count_fringe(Image.open(out_path))
    print(f'[RAW→透明] {os.path.relpath(out_path, ROOT)}  抠掉 {removed}px（共 {total}px），残留可见脏边 {after}')


def main():
    print('== A 类：RGBA 立绘 半透明近白边清理（覆盖保存） ==')
    for f in sorted(os.listdir(os.path.join(ASSETS, 'ai_char_3d'))):
        if f.endswith('.png'):
            process_rgba_path(os.path.join(ASSETS, 'ai_char_3d', f))
    personas_dir = os.path.join(ASSETS, 'personas')
    for pid in sorted(os.listdir(personas_dir)):
        p = os.path.join(personas_dir, pid, 'character.png')
        if os.path.isfile(p):
            process_rgba_path(p)

    print('== B 类：白底 raw → 透明版 ai_char/<key>.png ==')
    for k in ['huopo', 'gudian', 'dushi', 'wenrou', 'reqing']:
        raw = os.path.join(ASSETS, 'ai_char', k + '_raw.png')
        out = os.path.join(ASSETS, 'ai_char', k + '.png')
        if os.path.isfile(raw):
            process_raw_to_transparent(raw, out)
        else:
            print(f'  缺失 {raw}，跳过')
    print('== 完成。原图备份在', BACKUP, '==')


if __name__ == '__main__':
    main()
