---
name: animacode
description: 用代码做确定性动画成片。写一份 scene.json，构建 16:9 / 9:16 多比例、逐帧渲染、自动质检、合成带音轨 MP4，并在本地 Studio 里复盘。全本地、零 npm 依赖、不联网。
---

# Animacode（元气小小内）

用户要"做一条动画/动效片/数据短片/片头片尾/竖版视频"时用本技能。**不要直接写 React 或 canvas 代码**，先写 `scene.json`。

## 工具

- `anima_schema` — 取 spec 与六个镜头块字段。**第一步必须调用**，不要凭记忆编字段。
- `anima_build` — 校验 + 构建每个比例。返回错误时按错误逐条改 spec，再调一次。
- `anima_render` — 渲帧。先抽样看片（`frames:[镜头首中尾帧号]`），再全量。
- `anima_qa` — 质检门禁。`isError=true` 表示 verdict=FAIL，**禁止交付**。
- `anima_mux` — 帧 + 音轨合成 MP4。

## 流程

1. `anima_schema`
2. 写 `scene.json`：先 `duration`/`fps`/`theme`/`formats`，再排 `shots`，最后 `captions`
3. `anima_build` → 修到无错误
4. `anima_render` 抽样 → 用图片查看能力看每镜头首中尾帧
5. `anima_render` 全量（带 `audit`）
6. `anima_qa` → FAIL 按 issues 改；WARN 逐条判断，确认是有意设计要在回复里写明
7. `anima_mux`（有配乐传 `audio`）
8. 交付：成片路径 + verdict + 告警说明；需要复盘时提示用户开本地 Studio

## 规矩

- 文字一律走块的 `data`，不要自己画；块外的运动才考虑手写路径
- 竖版与横版共用一份 spec，改 `blocks.js` 或 spec 后重 build，不要手改产物 HTML
- 改了单个镜头：`anima_render(shots:["s4"], reuse:"<旧帧目录>")`，不要整片重渲
- 尾卡静止 0.5–0.7s 是有意停留，QA 会告警，交付时说明，不要为消警告加扫光
