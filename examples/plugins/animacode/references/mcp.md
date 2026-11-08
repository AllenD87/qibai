# MCP 接入（本地模型 / 元气小小）

`mcp/server.mjs` 是 stdio MCP 服务，零依赖，JSON-RPC 2.0。**技能本身不含模型**——它只提供确定性工具，模型负责写 spec 与读质检结论。

```json
{ "command": "node", "args": ["<skill>/mcp/server.mjs"], "env": { "CHROME": "/usr/local/bin/chromium" } }
```

## 工具

| 工具 | 入参 | 返回 |
|---|---|---|
| `anima_schema` | 无 | spec 全字段 + 六块 data 字段（约 3.7k 字符），写 spec 前先取 |
| `anima_build` | `spec`（绝对路径）, `out?`, `format?` | 各格式 scene 路径；spec 非法时 `isError:true` + 逐条错误 |
| `anima_render` | `scene`, `out`, `shots?`, `reuse?`, `frames?`, `audit?` | `{rendered,reused,missing,ms,perFrameMs}` |
| `anima_qa` | `frames`, `fps`, `duration`, `out`, `audit?`, `sheet?`, `baseline?` | `{verdict,issues,warnings,motion,text,luminance}` |
| `anima_mux` | `frames`, `out`, `fps?`, `crf?`, `audio?` | 成片路径 |

`anima_qa` 在 `verdict=FAIL` 时返回 `isError:true`，模型能直接把这个信号当"必须改"，不需要自己判断画面好坏。

## 给本地模型的最小循环

```
anima_schema → 写 scene.json → anima_build
  ↳ isError: 按错误列表改 spec（id 重复/越界/block 未知/in.dur 过长）
anima_render(frames:[每个镜头首中尾]) → 看帧
anima_render(out=frames, audit=audit.json)
anima_qa → verdict
  ↳ FAIL: 按 issues 改；WARN: 判断是否"有意停留"
anima_mux → 交付
改了单个镜头：anima_render(shots:["s4"], reuse:"frames")
```

## 为什么弱模型也能用

1. **校验器是确定性的**：spec 错误由 `build_scene` 逐条返回，模型只需按字符串修，不用理解版式。
2. **门禁是量化的**：`overlap / outside-safe / tiny / contrast / dead / spike` 都是数字与时间戳，模型照单修。
3. **上下文很小**：schema 3.7k 字符、qa 返回几百字节；不需要把参考文档、源码塞进上下文。
4. **画面质量不看模型审美**：缓动、留白、安全区由 `blocks.js` 的常量保证；模型只负责选块与填内容。

代价是：模型写不出六块之外的运动。要非常规运动时走手写路径，或先扩块。

## 接入元气小小（Tauri + Rust core）

1. 把整个 `animacode/` 拷进其技能目录（`SKILL.md` + scripts + references 都是普通文件）。
2. Rust 侧在启动时按上面的 JSON 片段 spawn `node <skill>/mcp/server.mjs`，走现有 stdio MCP 通道，无需新协议。
3. 系统提示里加两条硬约束：
   - 写 spec 前必须先 `anima_schema`，不要凭记忆编字段；
   - `anima_qa` 返回 `isError` 时禁止交付，必须修到 `PASS`/`PASS_WITH_WARNINGS`。
4. 权限白名单只放这五个工具；`anima_preview` 未注册为 MCP 工具（起服务由人手动触发，避免模型乱开端口）。
5. 设备侧依赖：Chromium（`CHROME` 环境变量可指定）、ffmpeg、Node ≥18、Python3 + numpy/Pillow。渲染是 CPU 单机任务，450 帧约 49s。

## 已验证

`initialize / tools/list / tools/call` 全通：`anima_schema` 3.7k 字符，`anima_build` 出两格式，`anima_render` 3 帧 108.7ms/帧，`anima_qa` 返回 `PASS_WITH_WARNINGS` 与告警原文。
