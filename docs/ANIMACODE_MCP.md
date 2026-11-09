# Animacode MCP 接入（技能包 / 连接器）

把 animacode 的动画能力作为**本地 MCP 连接器**接进核心：模型写一份 `scene.json`，就能构建多比例场景、逐帧渲染、跑量化质检、合成带音轨 MP4。全程本地、不联网，零 npm 依赖（复用系统 chromium + ffmpeg）。

装好之后它同时占两个位置：

- **技能库**：`qibai-plugin.json` + `SKILL.md` 进能力包目录，LLM 的系统提示里会出现「[技能] Animacode 动画成片：…」，可用 `skill_read` 取正文步骤；
- **工具表**：`entry.kind=mcp` 的连接器启用后，五个 `anima_*` 函数直接出现在 LLM 工具表里，模型可以真调。

## 包内容

`examples/plugins/animacode/`

```
qibai-plugin.json   连接器清单：entry.kind=mcp，command=node，args={PLUGIN_DIR}/mcp/server.mjs
SKILL.md            模型操作说明（先 schema → 写 spec → build → 抽帧 → 全量 → qa → mux）
mcp/server.mjs      stdio MCP（JSON-RPC 2.0，一行一个 JSON），协议 2024-11-05
scripts/            build_scene / render_frames / qa_report / make_video / preview
assets/             animacore.js、blocks.js、spec_scene.html、studio.html、示例 spec
references/         spec 字段、块、API、质量门禁、动效、与 Remotion 的差异
```

`{PLUGIN_DIR}` 由 `ManifestLoader::load` 按**包实际所在目录**替换成绝对路径，所以包被装到用户根、内置根或被 `QIBAI_PLUGINS_DIR` 指到别处都不会失配——不用手改清单。

## 五个工具

| 工具 | 作用 | 失败语义 |
| --- | --- | --- |
| `anima_schema` | 返回 spec 与六个镜头块字段 | — |
| `anima_build` | 校验 + 构建每比例 | spec 非法返回逐条错误 |
| `anima_render` | 渲帧；`shots`+`reuse` 只渲改动镜头 | 缺帧时报告 missing |
| `anima_qa` | 安全区/重叠/小字号/对比度/空帧/长静止/突变/视觉回归 | verdict=FAIL 时 `isError=true`，禁止交付 |
| `anima_mux` | 帧 + 音轨合成 MP4 | 无帧时报错 |

## 安装

**A. 技能包（推荐）**：设置页「技能 / 插件 → 导入技能包」，选 `animacode-v2.1.0.qbpk`（zip，根目录就是 `qibai-plugin.json`，不要再套文件夹）。解压落盘到 `<数据目录>/plugins/animacode/`。

**B. 手动放置**：

```bash
# 数据目录：macOS ~/Library/Application Support/com.Qibai.Qibai
#           Linux ~/.local/share/qibai
#           Windows %APPDATA%/com.Qibai.Qibai
mkdir -p "<数据目录>/plugins"
cp -r examples/plugins/animacode "<数据目录>/plugins/"
```

**C. 只登记 MCP（不装插件包）**：设置里 `mcp_servers` 加一项

```json
{"name":"animacode","transport":"stdio","command":"node","args":["/绝对路径/animacode/mcp/server.mjs"],"enabled":true}
```

## 启用（外部包默认是关的）

A/B 两条路径装完，包会出现在技能 / 插件页，但**默认不启用**——外部包一律先关着，避免“拷进来就自动拿到命令行”。在列表里打开开关即可，之后 `anima_*` 进工具表。

## 前置依赖

- `node`（≥18）在 PATH
- `chromium` 或 `google-chrome` 在 PATH（渲染用；`CHROME` 环境变量可指定）
- `ffmpeg`（合成用；没有就只能出帧，不能出 MP4）
- `python3` + numpy + Pillow（`qa_report.py` 质检用）

缺依赖时先跑 `bash scripts/doctor.sh` 自检——它会直接说缺哪个。

## 验收口径（仓库内已固化为测试）

`qibai-core/tests/animacode_mcp.rs` 走的就是产品链路：

1. 复制包到临时根 → `ManifestLoader::load` 解析清单，断言 `{PLUGIN_DIR}` 已替换成包内 `mcp/server.mjs`；
2. `PluginRegistry::new` 扫到 `animacode`、默认不进工具表 → `set_enabled` 后五个 `anima_*` 全进；
3. `SkillAdapter` 真拉 stdio server：`anima_schema` 返回字段说明，`anima_build` 用示例 spec 产出 `169` 比例。

没装 node 的机器上该用例打印 SKIP 跳过，不算失败。

平台边界：**macOS / Linux 验收完整出片链路**；Windows 目前只验收 MCP 握手与 `anima_schema`——构建脚本按 POSIX 规则拼 spec 目录，在 Windows 上会拼出 `D:\C:\...` 这类路径，CI 里因此只跑到 schema 为止（三平台测试均为绿）。

## 注意

- `trust` 默认 `confirm`：每次调用走确认；要免确认需把连接器设为 `readonly`。
- 渲染会起子进程（chromium）并写帧到项目目录，属重操作；建议由用户在「做一条动画」时显式触发。
- 技能包落在 `<数据目录>/plugins/<id>/`（平铺），能力包扫描器同时覆盖 `skills/ experts/` 与平铺两层，所以装好的包在技能库里也能看到正文。
