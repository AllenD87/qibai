---
name: animacode
description: 用声明式 scene.json 逐帧生成确定性动画，自带 headless 渲染、量化质检、多比例输出与本地 Studio。适用于品牌片头片尾与 MG 动效、数据与科普动画、流程演示、带字幕与配乐的短视频、批量模板化视频、内网离线私有化渲染。当用户要求用代码/程序/脚本做动画或动效视频，或需要一套可复现、可质检、可批量出片的生产流程时使用；也用于把已有动画工程改成 spec 驱动、排查帧级缺陷（文字重叠、越界、糊在一起、空帧、长静止、低对比）、一条内容同时输出 16:9 / 9:16 / 1:1。
---

# Animacode

用代码做动画，并且**能自证质量**：同一时间轴重渲像素一致，渲染完立刻拿到可量化的缺陷清单。

## 两条路径

| 场景 | 用哪条 | 入口 |
|---|---|---|
| 常规出片、多比例、要批量 | **Spec 路径（默认）** | 写 `scene.json` → `build_scene.mjs` |
| 自定义运动、非常规构图 | 手写自由路径 | `assets/template/scene.html` + `animacore.js` |

先判断：镜头是不是能落到 `title / stat / bars / process / timeline / compare / quote / endcard` 八种块上。能，就走 spec；不能，才手写，并在交付说明里写明"自由路径，无 spec 校验"。

## 五步工作流（Spec 路径）

1. **写 spec**：在工程目录写 `scene.json`（字段见 `references/spec-schema.md`）。先定 `duration`、`fps`、`theme`、`formats`，再排 `shots`（`t0/t1` 可交叠做转场），最后写 `captions`。
2. **构建**：`node <skill>/scripts/build_scene.mjs --spec scene.json --out build`
   每个比例输出 `build/<id>/{scene.html,studio.html,animacore.js,blocks.js}`。校验不过会直接报错并列出问题，先修 spec 再渲染。
3. **抽样渲**：`node <skill>/scripts/render_frames.mjs --in build/916/scene.html --out chk --frames 30,120,260`
   每个镜头至少看首、中、尾三帧。用 Read 打开 JPEG 目检构图、层级、断行。
4. **全量渲 + 质检**：
   ```bash
   node <skill>/scripts/render_frames.mjs --in build/916/scene.html --out frames --audit audit.json
   python3 <skill>/scripts/qa_report.py --frames frames --fps 30 --duration 15 \
     --audit audit.json --sheet sheet.png --plot plot.png --out build/916/qa_report.json --fingerprint fp.json
   # 降采样宽度默认按帧数自动算（内存安全）；要复现某次结果再显式 --work-w
   ```
   `qa_report.json` 放在 `build/<id>/` 下，Studio 才能读到。
5. **合成 + 复盘**：
   ```bash
   bash <skill>/scripts/make_video.sh frames out.mp4 30 16 audio/bed.wav
   node <skill>/scripts/preview.mjs --dir . --port 8899   # 打开打印出的 studio 链接
   ```

**改了单个镜头**：只重渲它，其余帧复用，不要整片重来。
`render_frames.mjs --in build/916/scene.html --out frames --shots s4 --reuse frames`（其余帧自动拷过来，QA 仍看全片）。

**门禁**：`VERDICT: FAIL` 必须修到 `PASS` 或 `PASS_WITH_WARNINGS`。告警要逐条判定，不能静默放过；确认是有意设计时，在交付里写明（例如尾卡静止 0.6s）。

## 硬规则（v1–v2 实测教训，不要重犯）

- **渲染态保持原生像素**：画布不得被 CSS 拉伸；`Scene` 传 `mount:false`，只在预览里缩放。渲染器会自动按 `__ANIMA.width/height` 设视口，多比例不会被裁。
- **一个文字组一个 group**：逐字标题/同段字幕必须传同一个 `group`，否则被误判重叠。
- **不可见不画**：进度 ≤0.001 的元素直接 `return`，否则留下 4px 细条。
- **文字继承颜色**：颜色/透明度必须显式传，不要依赖画布残留状态；图层内透明度用 `A0 * x` 赋值，不要连续 `*=`。
- **格式选择**：默认 JPEG q96（约 100ms/帧）；只有需要透明通道才 PNG（约 700ms/帧）。
- **接触表会骗人**：缩略图上看不见的小元素，必须回 100% 帧确认，不能据此判缺陷。
- **`file://` 不可用**：Studio 读 `qa_report.json`、scene 加载音频都被 CORS 拦，必须走 `preview.mjs`。
- **改短时长必须清旧帧**：新片比旧片短时，旧帧会留在目录里，质检按旧帧数报 FAIL、合成按旧帧数出更长的视频。`render_frames.mjs` 收尾会报 `stale` 数，加 `--clean` 删掉；`make_video.sh` 第 6 参数给 `duration` 会直接拦住不一致。
- **字幕胶囊上的文字传 `audit:false`**：胶囊是填充层，审计环取样会拿胶囊色去比纸面，必然误报低对比。文字照常画，只是不进审计。
- **先跑 `bash scripts/doctor.sh`**：缺 Chromium / ffmpeg / numpy / Pillow 时，渲染或质检会以奇怪方式失败（例如质检退出码 0 但无输出）。

## 质检怎么读

| 指标 | 含义 | 处理 |
|---|---|---|
| `text.overlap > 0` | 不同组文字相交 | FAIL，改布局或合并 group |
| `text.outside_safe > 0` | 出 6% 安全区 | FAIL，收字号或让位 |
| `text.contrast_flags > 0` | 稳定态文字对比度不足 | 换色/加深底色；<1.5:1 直接 FAIL |
| `motion.dead_seconds > 0` | 变化像素占比与局部峰值都低 | 确认是否为有意的停留 |
| `motion spike` | 单帧突变 | 通常是硬切或参数跳变，加过渡 |
| `blank` | 空帧 | 起始帧别留纯黑，除非有意淡入 |
| `fingerprint diff` | 与基线帧不一致 | 确认改动符合预期，再更新基线 |

阈值与判定细节见 `references/quality-gates.md`。

## 改节奏 / 重录配音（不要手改上百处秒数）

写场景时保留一条**脚本时间轴**（每段在脚本里的绝对区间），另建一张**映射表**把它压/伸到成片时间，所有图层用 `scene.addWarped(TABLE, (ctx, scriptT) => {...}, 'name')` 注册：

```js
const WARP = [[0,5.6, 0,7], [5.6,6, 7,7.4], [6,12.5, 7.4,18.9], [12.5,12.9, 18.9,19.3]];
scene.addWarped(WARP, (ctx, t) => { /* 这里全用脚本时间 */ }, 'main');
```

配音重录后只改 `WARP` 的右侧数字，画面编排与缓动时间点一行不用动；字幕按每段新时长按字数权重重新分。`AC.remap(t, TABLE)` 也可单独用。

## 多比例

一份 spec 出多版：`formats` 里列 `{id,width,height}`，块库按 `H>W` 自动切竖版（流程块竖排+右置标签、字号按 `u` 缩放、条形图重新留白）。**不要**手改生成后的 HTML，改 `blocks.js` 或 `scene.json` 后重 build。

## 音频与字幕

- `audio.src` 指向音轨（相对 spec），`make_video.sh` 第 5 参数混流为 AAC；音轨缺失时构建只告警。
- `captions` 是全局层，逐字 karaoke 由 `AC.renderCaptions` 画；单条 ≤26 字，超出构建会警告。
- 目前**没有**字幕与音轨的自动对齐检测，时间点由作者按音轨切点填。

## 接入本地模型 / MCP

`mcp/server.mjs` 暴露五个工具（schema / build / render / qa / mux）。技能不含模型：校验器与门禁是确定性的，弱模型只按错误列表改 spec 即可。接法与 Rust 侧配置见 `references/mcp.md`。

## 本地 Studio

`preview.mjs` 起服务后打开 `build/<id>/studio.html`：画布缩放播放、滑动/逐帧、`A` 切换审计框（安全区边线 + 每个文字盒）、右侧列出 `qa_report.json` 的 verdict 与告警，点带时间戳的告警直接跳帧。这是离线版的 Remotion Studio，多了"审计可视化"。

## 与 Remotion 的分工

不是复刻：Remotion 强在 React 生态和云端分发，本技能强在**质量闭环、零依赖离线、多比例一次成型**。差异与证据见 `references/vs-remotion.md`；选型与性能数据见 `references/tooling.md`。

## 文件地图

```
assets/runtime/animacore.js     缓动/spring/文字审计/纹理，零依赖
assets/runtime/blocks.js        八个声明式镜头块 + 字幕层 + 版式
assets/template/spec_scene.html 数据驱动场景（__SPEC_JSON__ 由构建内联）
assets/template/studio.html     本地 Studio
assets/template/scene.html      v1 手写模板（自由路径）
assets/example-spec/scene.json  可直接跑的起步 spec
scripts/build_scene.mjs         spec 校验 + 多格式构建
scripts/render_frames.mjs       headless Chromium + CDP 逐帧截图
scripts/qa_report.py            帧级质检 + 接触表 + 运动曲线 + 指纹
scripts/make_video.sh           帧→MP4，可选混音
scripts/preview.mjs             本地静态服务（Studio 用）
scripts/new_scene.sh            v1 手写工程脚手架
scripts/doctor.sh               环境自检（node/chromium/ffmpeg/python 依赖/中文字体）
references/spec-schema.md       scene.json 全字段
references/blocks.md            八个块的 data 字段与版式行为
references/api.md               animacore 运行时 API
references/quality-gates.md     门禁阈值与缺陷对照
references/motion-design.md     动效设计规则（缓动/节奏/层级）
references/tooling.md           脚本参数、性能基线、离线部署
references/vs-remotion.md       对标结论与超越点证据
references/mcp.md               MCP/本地模型接入（stdio 五工具）
mcp/server.mjs                  stdio MCP 服务，零依赖
CHANGELOG.md                   版本改动与实测依据
```
