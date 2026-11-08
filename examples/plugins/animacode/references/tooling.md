# 脚本参数与实测基线

全部零 npm 依赖。运行需要：Node ≥18、Python 3 + numpy + Pillow、ffmpeg、Chromium/Chrome。

```bash
export CHROME=/usr/local/bin/chromium     # 找不到时显式给路径
```

## build_scene.mjs

```
node build_scene.mjs --spec scene.json --out build [--format 916]
```

- 校验 + 每格式输出 `build/<id>/{scene.html,studio.html,animacore.js,blocks.js,spec.json}`
- 生成 `build/manifest.json`（含各格式 scene/studio 绝对路径、音频绝对路径）
- spec 内联进 HTML，`<` 转义为 `\u003c`，场景可直接 `file://` 打开（Studio 除外）
- 错误即退出 1；告警只打印不阻断（镜头过短、字幕过长、音频缺失）

## render_frames.mjs

```
node render_frames.mjs --in build/916/scene.html --out frames \
  [--fps 30] [--duration 15] [--width W] [--height H] \
  [--format jpeg|png] [--quality 96] [--alpha] \
  [--frames 30,120,260] [--shots s3,s4] [--reuse frames-old] [--audit audit.json] [--timeout 45]
```

- 启动 headless Chromium，CDP 直连，等 `window.__ready`
- **优先读 `__ANIMA.width/height` 设 `Emulation.setDeviceMetricsOverride`**；未自报才用 `--width/--height`，避免 1080×1920 被裁成横版
- `--frames` 只渲指定帧（抽样审片用）；`--alpha` 强制 PNG + 透明背景
- **增量渲**：`--shots s3,s4` 只渲这些镜头覆盖的帧，`--reuse <旧帧目录>` 把其余帧直接拷过来，QA 仍看完整时间轴。实测改一个 3s 镜头：450 帧里只渲 91 帧、复用 359 帧，16s 完成（全量 49s）
- `--audit` 在增量模式下会与已有 audit 合并，不会丢掉其他镜头的审计
- 输出 `<out>/00000.jpg|png`，末行打印 `perFrameMs`

## qa_report.py

```
python3 qa_report.py --frames frames --fps 30 --duration 15 \
  [--audit audit.json] [--sheet sheet.png] [--plot plot.png] [--title "..."] \
  [--out build/916/qa_report.json] [--fingerprint fp.json] [--baseline fp.json] [--fail-on-change] \
  [--dead-ratio 0.0025] [--dead-min 0.45] [--dead-peak 2.5] [--work-w 720] [--contrast]
```

`--out` 指向 `build/<id>/qa_report.json`，Studio 才会读到；`--sheet`/`--plot` 供人眼复盘。

## make_video.sh

```
bash make_video.sh <frames-dir> <out.mp4> [fps=30] [crf=16] [audio.wav]
```

无音频走纯视频；有音频 `-map 0:v -map 1:a -c:a aac -b:a 192k -shortest`。固定 bt709 标记 + `yuv420p` + faststart，避免播放器偏色/黑屏。

## preview.mjs

```
node preview.mjs --dir . --port 8899
```

- 默认起在 `127.0.0.1`，只读本地文件，路径做了越界防护
- 目录下有 `build/manifest.json` 时，自动打印每个格式的 Studio 地址
- **必须用它而不是 `file://`**：Studio 读 `qa_report.json`、scene 加载音频都会被 CORS 拦

## 性能基线（实测，2524 字体、Noto Sans CJK）

| 场景 | 分辨率 | 格式 | 每帧 | 450 帧总耗时 |
|---|---|---|---|---|
| v2 竖版 | 1080×1920 | JPEG q96 | 108ms | 48.7s |
| v2 横版 | 1920×1080 | JPEG q96 | 109ms | 49.2s |
| v1 高噪点 | 1920×1080 | PNG | ~700ms | — |

结论：默认 JPEG；只有透明需求才 PNG。加 `--workers 2` 可并行两个 Chromium（脚本已支持按帧取模分片，需要自己起两个进程）。

## 离线与私有化

- 无 npm 依赖、不下载任何资源、不联网、无遥测、无 license key
- 字体用系统已装 CJK 字体（`AC.FONT_STACK` 首选 Noto Sans CJK SC）
- 只要有 Chromium 与 ffmpeg 即可在断网内网跑完整条链路
- 产物是普通文件夹，不打包、不生成 `.skill`
