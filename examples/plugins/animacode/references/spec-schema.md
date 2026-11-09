# scene.json

一份 spec 出多版视频。所有字段如下，构建时校验，不合法直接退出并列出原因。

```json
{
  "title": "内部标识，仅用于 manifest",
  "duration": 15,
  "fps": 30,
  "theme": { "bg": "#0B0D12", "ink": "#F2F0EA", "accent": "#FF5A3C", "accent2": "#2FA38F", "muted": "#8C93A0", "vignette": 0.35, "grain": 0.02 },
  "formats": [{ "id": "916", "width": 1080, "height": 1920 }, { "id": "169", "width": 1920, "height": 1080 }],
  "audio": { "src": "audio/bed.wav", "gain": 1 },
  "shots": [{ "id": "s1", "t0": 0, "t1": 3, "block": "title", "in": {...}, "out": {...}, "data": {...} }],
  "captions": [{ "t0": 0.2, "t1": 2.3, "text": "一句话" }]
}
```

## 顶层

| 字段 | 必填 | 说明 |
|---|---|---|
| `duration` | 是 | 秒，>0；帧数 = `round(fps*duration)` |
| `fps` | 否 | 默认 30 |
| `theme` | 否 | 颜色令牌；块库全部从 `theme` 取色，缺省有内置值 |
| `formats` | 否 | 缺省为 `169`；每项必须 `id/width/height` |
| `audio` | 否 | `src` 相对 spec 文件；缺失只告警，仍会构建 |
| `shots` | 是 | 至少一条，见下 |
| `captions` | 否 | 全局字幕层，见下 |

`theme.vignette` / `theme.grain` 是整片纹理，最上层绘制；`grain` 按帧号生成，确定性。

## shots

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 是 | 唯一；同时作为文字审计 group 的前缀 |
| `t0` / `t1` | 是 | 秒，必须 `t1>t0` 且落在 `0..duration`；相邻镜头可交叠做转场 |
| `block` | 是 | `title` / `stat` / `bars` / `process` / `timeline` / `compare` / `quote` / `endcard` |
| `in` | 否 | `{type:'fade'|'rise'|'scale'|'maskUp', dur:0.5, ease:'outExpo'}`，`dur` 必须小于镜头长度 |
| `out` | 否 | `{dur:0.4}`，淡出固定 `inQuad` |
| `data` | 是 | 块的具体内容，见 `blocks.md` |

**校验规则**：`id` 缺失或重复、`block` 未知、`t0/t1` 非数字/越界/倒置、`in.dur >= 镜头长度` → 错误；镜头短于 0.4s → 告警（几乎必然看不清）。

## captions

| 字段 | 说明 |
|---|---|
| `t0` / `t1` | 出现区间，越界即错误 |
| `text` | 单条 ≤26 字，超出告警（胶囊会换行破相） |

字幕绘制在固定底部基线（竖版 `0.13H`、横版 `0.105H`），半透明胶囊 + 逐字 karaoke：已读部分 `accent`，未读 `ink`。**字幕时间点不会自动跟随音轨**，需按配音切点手填。

## 多格式约定

块库用 `AC.makeLayout(spec, W, H)` 决定版式：

- `portrait = H > W`，`u = min(W,H)/1080` 是统一缩放单位，所有尺寸都乘 `u`
- 内容盒：竖版 `y=0.15H, h=0.54H`；横版 `y=0.13H, h=0.64H`，左右留 7–8%
- 竖版自动换排法（流程块竖排、标签右置；条形图重算宽度与留白）

所以同一份 `data` 不需要为比例写两套；若某块在某个比例明显失衡，改 `blocks.js` 的版式常量，不要手改产物 HTML。
