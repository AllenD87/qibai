#!/usr/bin/env python3
"""栖伴专家头像生成器：512x512 PNG 渐变底 + 圆角白卡 + 主题汉字（≤500KB）。

用法: python3 avatar_gen.py <输出png路径> <汉字(1-2字)> <顶部色> <底部色> [副标题]
示例: python3 avatar_gen.py planner.png 日程 0x0E7B8C 0x8FE3D9
"""
import sys
from PIL import Image, ImageDraw, ImageFont

OUT = sys.argv[1]
TEXT = sys.argv[2]
C1 = tuple(int(sys.argv[3][i:i+2], 16) for i in (2, 4, 6))
C2 = tuple(int(sys.argv[4][i:i+2], 16) for i in (2, 4, 6))
SUB = sys.argv[5] if len(sys.argv) > 5 else ""

SIZE = 512
img = Image.new("RGB", (SIZE, SIZE))
px = img.load()
for y in range(SIZE):
    t = y / SIZE
    r = int(C1[0] + (C2[0] - C1[0]) * t)
    g = int(C1[1] + (C2[1] - C1[1]) * t)
    b = int(C1[2] + (C2[2] - C1[2]) * t)
    for x in range(0, SIZE, 2):
        px[x, y] = (r, g, b)
        if x + 1 < SIZE:
            px[x + 1, y] = (r, g, b)

d = ImageDraw.Draw(img)
# 圆角白卡
d.rounded_rectangle([64, 64, 448, 448], radius=48, fill=(255, 255, 255, 255))

# 加载中文字体（按优先级探测）
font_path = None
for p in [
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
]:
    try:
        ImageFont.truetype(p, 180)
        font_path = p
        break
    except Exception:
        continue
if font_path:
    font = ImageFont.truetype(font_path, 180)
    sub_font = ImageFont.truetype(font_path, 40)
else:
    font = ImageFont.load_default()
    sub_font = font

# 主汉字居中
bbox = d.textbbox((0, 0), TEXT, font=font)
tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
x = (SIZE - tw) / 2 - bbox[0]
y = (SIZE - th) / 2 - bbox[1] - (28 if SUB else 0)
d.text((x, y), TEXT, font=font, fill=(40, 60, 90))

if SUB:
    bbox2 = d.textbbox((0, 0), SUB, font=sub_font)
    sw = bbox2[2] - bbox2[0]
    d.text(((SIZE - sw) / 2, 420), SUB, font=sub_font, fill=(120, 140, 160))

img.save(OUT, "PNG", optimize=True)
import os
print("saved", OUT, os.path.getsize(OUT), "bytes")
