# Animacode MCP 接入（插件包）

把 animacode 的动画能力作为**本地 MCP 连接器**接进核心：模型写一份 `scene.json`，就能构建多比例场景、逐帧渲染、跑量化质检、合成带音轨 MP4。全程本地，不联网，零 npm 依赖（只复用系统 chromium + ffmpeg）。

## 包内容

`examples/plugins/animacode/`

```
qibai-plugin.json   连接器清单：entry.kind=mcp，command=node，args={QIBAI_HOME}/plugins/animacode/mcp/server.mjs
SKILL.md            模型操作说明（先 schema → 写 spec → build → 抽帧 → 全量 → qa → mux）
mcp/server.mjs      stdio MCP（JSON-RPC 2.0，一行一个 JSON），协议 2024-11-05
scripts/            build_scene / render_frames / qa_report / make_video / preview
assets/             animacore.js、blocks.js、spec_scene.html、studio.html、示例 spec
references/         spec 字段、块、API、质量门禁、动效、与 Remotion 的差异
```

## 五个工具

| 工具 | 作用 | 失败语义 |
| --- | --- | --- |
| `anima_schema` | 返回 spec 与六个镜头块字段 | — |
| `anima_build` | 校验 + 构建每比例 | spec 非法返回逐条错误 |
| `anima_render` | 渲帧；`shots`+`reuse` 只渲改动镜头 | 缺帧时报告 missing |
| `anima_qa` | 安全区/重叠/小字号/对比度/空帧/长静止/突变/视觉回归 | verdict=FAIL 时 `isError=true`，禁止交付 |
| `anima_mux` | 帧 + 音轨合成 MP4 | 无帧时报错 |

## 安装

**A. 技能包 zip**：设置页「技能/插件 → 导入技能包」，选 `animacode-plugin.zip`（压缩包根目录就是清单，不要套一层文件夹）。

**B. 手动放置**：

```bash
mkdir -p ~/.qibai/plugins
cp -r examples/plugins/animacode ~/.qibai/plugins/
```

放置后重启应用：`plugins_dir()` 扫描 `~/.qibai/plugins`，`entry.kind=mcp` 的 connector 会被放进 LLM 工具表。

**C. 只登记 MCP（不装插件包）**：设置 `mcp_servers` 加一项

```json
{"name":"animacode","transport":"stdio","command":"node","args":["/绝对路径/mcp/server.mjs"],"enabled":true}
```

## 前置依赖

- `node`（≥18）在 PATH
- `chromium` 或 `google-chrome` 在 PATH（渲染用；`CHROME` 环境变量可指定）
- `ffmpeg`（合成用；有 ffmpeg 才做 mux）

## 注意

- `{QIBAI_HOME}` 只替换 `entry.mcp.args`，默认解析到 `~/.qibai`。若把 `QIBAI_PLUGINS_DIR` 指到别处，插件目录与 args 里的路径会失配——此时改用方案 C 的绝对路径。
- `trust` 默认 `confirm`：每次调用会走确认；要免确认需把该连接器设为 `readonly`。
- 渲染会启子进程（chromium）并写帧到项目目录，属重操作；建议由用户在「做一条动画」时显式触发。
