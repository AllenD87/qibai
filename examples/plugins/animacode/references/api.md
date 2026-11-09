# animacore.js API

零依赖 canvas 运行时。核心契约：**`Scene.render(t)` 是 `t` 的纯函数**，不读 `Date.now()`、不用 `Math.random()`，同一 `t` 重渲像素一致。

## Scene

```js
const scene = new AC.Scene({ canvas, width, height, fps, duration, background, transparent, mount });
scene.add((ctx, t, scene) => {...}, 'name');   // 层按加入顺序绘制
scene.addWarped(TABLE, (ctx, scriptT, scene) => {...}, 'name');  // 收到脚本时间，按 TABLE 映射后绘制
scene.render(t);                                // 画一帧
```

- 传了 `canvas` 且 `mount !== false` 会把画布 `style.width='100%'` —— **渲染与 Studio 必须 `mount:false`**，否则 CSS 拉伸让截图变成 918ms/帧
- 不传 `canvas` 时自己建一个离屏画布（Studio 曾因此画到不可见画布上，务必显式传）
- `transparent:true` 不清底色，用于出带 alpha 的 PNG/WebM

## 文字：唯一正确入口

```js
scene.text(str, x, y, { size, weight, color, align, baseline, letterSpacing, serif, group, alpha });
scene.measure(str, { size, weight, letterSpacing });
```

- 绘制同时把**变换后的设备像素盒**登记进审计表（`x/y/w/h/size/color/alpha`）
- `alpha` 由图层 `globalAlpha` 与本参数相乘，`<0.04` 视为不可见：不画、不审计（避免淡入中间态误报叠加/对比度）
- `color` 未传时继承当前 `fillStyle`——旧版会吞掉调用方颜色，已修
- `group` 相同不判重叠；逐字标题、karaoke 前后半句必须同组
- `audit:false`：只画、不登记审计盒。用于**压在同色胶囊/标牌上的文字**（字幕胶囊），否则环取样会拿胶囊色比纸面，必然误报低对比

## 审计

```js
const { issues, texts } = scene.audit(0.06, 28);
```

三类问题：`safe-area`（出 6% 边距）、`text-overlap`（不同组、交叠 >4px）、`tiny-type`（`size<28px`）。`texts` 带颜色与透明度，供 `qa_report.py` 做对比度取样。

## 动画工具

| API | 说明 |
|---|---|
| `AC.tween(t, start, dur, ease)` | 0→1 进度，`ease` 见下表；`dur` 秒 |
| `AC.move(t, start, dur, from, to, ease)` | 直接插值 |
| `AC.spring(t, start, {stiffness, damping, mass})` | 弹簧；落地可过冲，适合元素入场 |
| `AC.stagger(i, count, each, origin)` | 逐项延迟，`origin: start/center/end` |
| `AC.keyframes(t, [[time, value], ...], ease)` | 关键帧 |
| `AC.lerp`, `AC.clamp` | 数值 |
| `AC.rng(seed)` | 确定性随机，替代 `Math.random()` |
| `AC.remap(t, TABLE)` | 脚本时间 → 成片时间，分段线性；改节奏时只改表 |

缓动名：`linear / inQuad / outQuad / inOutQuad / inCubic / outCubic / inOutCubic / inQuart / outQuart / inOutQuart / inQuint / outQuint / inOutQuint / inExpo / outExpo / inOutExpo / outBack / outElastic / inOutSine`。

## 绘制与特效

| API | 说明 |
|---|---|
| `AC.roundRect(ctx,x,y,w,h,r)` | 只建路径，需自行 `fill()` |
| `AC.hexA(hex, a)` | 十六进制转 rgba |
| `AC.vignette(ctx,w,h,strength)` | 暗角 |
| `AC.grain(ctx,w,h,frame,{alpha})` | 胶片颗粒，按帧号确定性 |
| `AC.glow(ctx,color,blur,alpha)` / `AC.noGlow(ctx)` | 阴影发光开关 |
| `AC.trail(ctx,t,drawAt,{samples,span,alpha})` | 伪运动模糊：重绘近几帧 |
| `AC.burst(ctx,t,{...})` | 粒子爆散 |
| `AC.FONT_STACK` / `AC.SERIF_STACK` | 字体栈（含 Noto Sans CJK SC） |

## v2 扩展

| API | 说明 |
|---|---|
| `AC.BLOCKS` | 八个镜头块，见 `blocks.md` |
| `AC.makeLayout(spec, W, H)` | 返回 `{W,H,portrait,u,box,theme}` |
| `AC.renderCaptions(ctx,t,spec,L,scene)` | karaoke 字幕层 |

## 场景桥（模板提供）

```js
window.__ANIMA = { width, height, fps, duration, shots, seek(t), audit(), scene, spec };
window.__ready = true;   // 字体与资源就绪后置位，渲染器轮询它
```

渲染器先等 `__ready`，再按 `__ANIMA.width/height` 设视口，因此不同比例不会被裁成横版。
