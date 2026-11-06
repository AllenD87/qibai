#!/usr/bin/env python3
# scripts/_detect_blobs.py —— 连通域计数（flood-fill），供 verify_persona_switch.sh 调用。
# 用法: python3 _detect_blobs.py <image> [min_area_frac]
# 输出: blob 个数（>=1 的有效团块；0=空白）。
import sys
from PIL import Image
import numpy as np

def main():
    path = sys.argv[1]
    min_frac = float(sys.argv[2]) if len(sys.argv) > 2 else 0.005
    im = Image.open(path).convert('L')
    a = np.asarray(im)
    # 暗色=前景（角色/文字）。threshold: 暗于 128 算块。
    mask = a < 128
    h, w = mask.shape
    total = h * w
    seen = np.zeros_like(mask, dtype=bool)
    blobs = 0
    # 简单 BFS（用 numpy 膨胀迭代，避免纯 Python 慢）
    from collections import deque
    for y in range(h):
        xs = np.where(mask[y] & ~seen[y])[0]
        for x in xs:
            if seen[y, x] or not mask[y, x]:
                continue
            # flood
            q = deque([(y, x)])
            seen[y, x] = True
            size = 0
            minx = maxx = x
            miny = maxy = y
            while q:
                cy, cx = q.popleft()
                size += 1
                if cx < minx: minx = cx
                if cx > maxx: maxx = cx
                if cy < miny: miny = cy
                if cy > maxy: maxy = cy
                for dy, dx in ((1,0),(-1,0),(0,1),(0,-1)):
                    ny, nx = cy+dy, cx+dx
                    if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                        seen[ny, nx] = True
                        q.append((ny, nx))
            if size >= total * min_frac:
                blobs += 1
    print(blobs)

if __name__ == '__main__':
    main()
