# 六个镜头块

每个块是 `(ctx, t, shot, L, S, scene)` 纯函数，只依赖 `t`。颜色一律从 `L.theme` 取，`L.u` 是缩放单位。

## title

```json
{ "kicker": "ANIMACODE · 02", "headline": "一帧一百毫秒", "sub": "程序化动画的成本账", "meta": "15 SEC · 30 FPS · ZERO DEPENDENCY" }
```

- `headline` 逐字 spring 起跳（stagger 从中心向两侧），同组 `group` 避免被判重叠；字号按容器自动收敛（竖版 118u / 横版 132u 起）
- `kicker` 30u 起，必须 ≥28px，否则触发 tiny-type
- `sub` 在标题下方 `+160u`
- `meta` 可选，左下角编目行（含 1px 短线），用于压住大面积留白；不需要就别填

## stat

```json
{ "value": 108, "suffix": "ms", "label": "单帧渲染耗时", "note": "1080×1920 · 450 帧 · headless" }
```

`value` 从 0 滚到目标（`outExpo`），千分位自动加；数字 190–220u 大字号并 scale 弹入；`label` 距数字 `size*0.56`，`note` 再下 66u；上方 7 根刻度线是"测量感"装饰，透明跟随入场。

## bars

```json
{ "title": "每帧耗时对比", "unit": "ms", "items": [{ "label": "动画渲染", "value": 7 }, ...] }
```

- 最多 5 条；条高 `rowH = min(box.h/(n+1), 158u)`，整组垂直居中
- 标签列宽 `min(0.30 box.w, 260u)`，条形从标签右侧 16u 起
- **数值右对齐成列**贴 `box.x+box.w`，并按最大数值预留 `reserve`，保证最宽一条也不出安全区
- 只有最后一条用 `accent` 高亮，其余 `muted`（不要首条高亮，会被读成"最高值"）

## process

```json
{ "title": "从 spec 到成片", "steps": ["写 scene.json", "构建多格式", "逐帧渲染", "质检门禁", "合成交付"] }
```

- 最多 5 步；横版横向排，竖版纵向排、标签放节点右侧 58u（避免压在连线上）
- 连线先画、节点后落；第一个节点 `accent`，序号反白
- 竖版整簇按可用高度收缩（`span = min(0.58 box.h, 860u)`），不要靠调字号硬塞

## quote

```json
{ "text": "渲染不是瓶颈，返工才是。", "by": "animacode v2" }
```

单段文字，66u 起自动收敛；`by` 前自动加破折号。

## endcard

```json
{ "brand": "ANIMACODE", "tagline": "代码即时间轴 · 质量可验证" }
```

三根柱子（accent / ink / accent2）依次 spring 起立，落地后以 ±4% 幅度慢呼吸；品牌字 96u、字距 6；`tagline` 在品牌下方 46u。

**不要加扫光**：线性渐变扫光无论怎样裁切/降透明都会读成硬边矩形（v2 已两次实测后移除）。尾卡 0.6s 静止是有意停留，质检会告警，交付时说明。

## 共同约定

- 块内透明度用 `ctx.globalAlpha = A0 * x`（`A0` 是进入块时的值），**不要** `*=`，否则同镜头后续元素越来越暗
- 不可见元素（进度 ≤0.001 / 尺寸 <8px）直接 `return`，避免残留细条
- 任何文字都走 `scene.text(...)`，否则不进审计、不被对比度门禁覆盖
