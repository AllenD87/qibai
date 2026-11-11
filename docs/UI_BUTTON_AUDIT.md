# 按钮与命令链路审计（v45.16）

对本轮「所有按钮 / 功能链路」体检的结论与修复记录。审计方式：解析 `src-tauri/src/main.rs` 里每个
`#[tauri::command]` 的参数名（含 `rename_all` 判定 camelCase / snake_case），与 `src/*.js` 中
`invoke('cmd', {...})` 的顶层参数键逐一比对；再核对按钮 id 与 JS 绑定。

## 判定依据（容易踩的机制）

1. **Tauri 宏默认把参数名转成 camelCase 再查 key**（`tauri-macros` 的
   `wrapper_attributes.argument_case = ArgumentCase::Camel` 是默认值）。只有显式写
   `#[tauri::command(rename_all = "snake_case")]` 才按 snake 匹配。
2. **`src/tauri-bridge.js` 的 `invoke(cmd, args)` 真机分支原样透传**，没有任何 key 转换层。
   因此「前端传什么 key」必须和后端「宏转换后的 key」完全一致，多一个下划线就报缺参数。
3. 缺必填 key 时报的是「参数不合法」，异常被 `try/catch` 吞掉后往往只表现为「点了没反应」。

## 体检结果

- `src/console.html` 共 **57 个按钮**，全部在 `console.js` 中找到绑定，无孤儿按钮。
- 前端 `invoke` 命令 **111 个**（另含 `plugin:event|listen/unlisten` 两条 Tauri 内置事件通道）。
- 本轮修复后：**无「前端调用但后端不存在」的自定义命令**；参数键全部对齐。

## 已修缺陷

| # | 位置 | 缺陷 | 修复 |
|---|---|---|---|
| 1 | `btnAudioTest`（测试输出设备） | 调 `audio_test_output`，后端**从未注册该命令** | 注册命令，转 `Engine::audio_test_output`；返回补 `selected_index` / `sink_landing_index` / `sink_landing_name` |
| 2 | 同上（前端渲染） | 只认 `landing.idx`，后端返回的是 `landing.index` → 永远显示「选择未生效」 | 兼容 `index`/`idx`/新字段；无播放流时改为中性提示，不再误判失败 |
| 3 | `btnSkillOpenDir`（打开技能目录） | 调 `open_workspace_dir`，**命令不存在**，catch 里只能吐写死的 `~/.qibai_skills`（路径也是错的） | 新增 `skill_open_dir`，打开真实的 `<数据目录>/plugins/skills`，跨平台唤起文件管理器 |
| 4 | 聊天来源链接（`pet.js`） | 调 `open_external`，**命令不存在**，落回 `window.open` 在 Tauri 里开空白 WebView | 新增 `open_external`，只放行 http/https/mailto，交系统浏览器 |
| 5 | `btnPluginInstall` + `pluginFileInput` | 写死 `invoke('plugin_install', { dir: 'demo://examples' })`，点一次必失败 | 改为选 `.qbpk`/`.zip` → 读内容 → `skill_pack_install_bytes`；演示脚本里的同款桩一并去掉 |
| 6 | 从文件安装技能（v45.8 两段式） | 前端传 `{file_name, file_path}`，后端签名是 `source: Value` —— **缺 `source` 外层，等于收不到参数**；且 WebView 的 `f.path` 不可靠 | 后端改显式 `kind`/`value` 入参；文件走新增字节通道 `skill_install_stage_bytes`（落临时文件后复用 `.md`/`.zip` 分支） |
| 7 | 安装确认 / 取消按钮 | `skill_install_commit` / `cancel` 参数是 `stage_id`，宏按 `stageId` 查 → 必失败 | 两个命令补 `rename_all = "snake_case"` |
| 8 | 技能包导入 / 导出 | `skill_pack_install(zip_path)`、`skill_pack_export(plugin_id, zip_path)` 同款不匹配 | 补 `rename_all = "snake_case"`（导入已改走字节通道） |
| 9 | 社交「保存我的光点」/ 城市回写 | 传 `{soul_tags}` / `{city}`，后端要 `input: Value` → 缺 key 必失败 | 前端包一层 `{ input: {...} }` |
| 10 | 社交隐私确认（成人 / 未成年） | 传 `{age_declared, pure_mode}`，后端要 `input` → 必失败，等于用户点了「同意」却没注册 | 同上包 `{ input: {...} }` |
| 11 | 社交漫游 | 调 `netsocial_roam` 传 `{}`，后端要 `params` | 改传 `{ params: {} }` |
| 12 | 添加连接器（MCP / HTTP） | `connector_add` 的 `url`、`headers` 是必填，前端按类型只填一侧 → 缺 key 必失败 | 后端把两者改 `Option`，缺省按空值处理 |

## 判定为「不必改」的项

- `intimacy_event` 多传 `task_id`、`plugin_install` 多传 `demo`、`debug_log` 多传
  `drag`/`failed`/`message`、`ui_panel_show` 多传 `payload`（实际是嵌套对象）、
  `skill_install_commit` 可选 `overwrite`：serde 忽略未用键，不影响。
- `task_*` 一族（`task_get` / `task_plan` / `task_confirm_plan` / `task_confirm_action` /
  `task_start` / `task_cancel` / `task_pause` / `task_resume` / `task_rerun` /
  `task_rerun_failed`）：前端按 **camelCase**（`taskId` / `actionId` / `fromStep`）调用，
  `console.js` 里有明确注释；这些命令**保持默认 camelCase**，不动。
  （本轮一度给它们加了 `rename_all = "snake_case"`，复审发现会把可用的链路改坏，已撤回。）
- `get_social_events(_limit)`：参数带下划线前缀且为 `Option`，缺省即 `None`，函数体也忽略。
- `plugin:event|listen` / `plugin:event|unlisten`：Tauri 内置事件通道，不是自定义命令。

## 验证

- `node --check src/console.js`、`node --check src/pet.js`：通过。
- `cargo test --locked`（`qibai-core`）：**667 passed / 0 failed / 3 ignored**（lib），
  集成测试 1 + 8 + 1 全通过。
- 脚本对账：57 按钮全绑定；无缺失命令；参数键无遗留不匹配（仅剩嵌套对象形态的误报）。
- **边界**：沙箱缺 `webkit2gtk`，`src-tauri` 无法真实编译。新增/改动的 Tauri 命令
  （base64 解码、`open_path` 的三平台 cfg 分支、`Option` 入参）与 `rename_all` 改动
  **未过真实编译**，需在你的机器上 `cargo build` 确认；`rustfmt --check` 已确认
  `main.rs` 语法可解析。
