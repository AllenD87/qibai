# 《元气小小》技能/专家体系 · 架构调整梳理与任务清单（实施后版本）

> 版本：V2.0（2026-10-23）｜状态：**T1-T10 已实施完成**｜目标：去硬编码 → 可插拔能力体系，对齐 WorkBuddy 开放平台能力模型。
> 关联：`docs/DOUBAO_WORK_INTEGRATION.md`（豆包工作集成）、`docs/IPC_CONTRACT.md`（IPC 契约）。
> 本文档为插件化体系的唯一权威说明（前身：V1.0 架构梳理 + 实施契约，内容已折叠至此）。

---

## 〇、实施总览

| 项 | 结果 |
|---|---|
| 测试基线 | **206 passed / 0 failed**（原 179 + 新增 27，只增不减） |
| 新增 Rust 模块 | `plugin_manifest.rs` / `plugin_registry.rs` / `skill_adapter.rs` / `expert.rs` |
| 改造模块 | `skills.rs`（去 match）、`settings.rs`（PluginSettings 兼容）、`engine.rs`（动态注入 + 7 新 IPC）、`llm_client.rs`（兼容保持） |
| 前端 | console 新增「技能与专家」tab（`src/console.html` / `console.js` / `tauri-bridge.js`），冒烟截图 4 张见 `artifacts/smoke-plugins-*.jpg` |
| 样例插件 | `plugins/examples/com.qibai.datetime`（技能）、`plugins/examples/com.qibai.scientist`（专家），可安装可调用（单测锁定） |
| 受限项 | `src-tauri` 编译需真机，沙箱内未做；WorkBuddy 官方清单格式未到手，经适配层映射表对齐 |

---

## 一、目标架构：可插拔能力体系

### 1.1 能力模型（对齐 WorkBuddy 开放平台）

WorkBuddy 开放平台定义四类能力：**技能（skill）/ 专家（expert）/ 连接器（connector）/ Buddy 应用（buddy）**。
《元气小小》第一阶实现前两类，连接器作为技能的一种执行后端预留，Buddy 应用后续评估。

- **技能（Skill）**：可被 LLM function-calling 调用的**动作能力**。名称、描述、参数 JSON Schema、执行体。
- **专家（Expert）**：**人设 + 能力装配**。系统提示词（性格/知识/口吻）+ 技能白名单 + 可选音色/形象映射 + 记忆风格。可挂载到任意 persona 或独立存在。
- **连接器（Connector，预留）**：技能的执行后端类型之一（外部数据源/系统接入）。本期解析合法、执行返回「连接器为预留类型，本期未接入」。

### 1.2 插件包格式（qibai-plugin.json，适配层可映射官方字段）

```
plugins/
  <plugin-id>/
    qibai-plugin.json      # 清单（字段见下）
    src/…                  # 可执行体（内置类型无需）
    assets/…               # 图标等
    README.md
```

`qibai-plugin.json` 核心字段（**WorkBuddy 风格命名，官方文档细节到手后逐字段对齐**）：

```jsonc
{
  "id": "com.qibai.weather",        // 唯一 id（反向域名）
  "type": "skill",                  // skill | expert | connector
  "name": "天气查询",               // 展示名
  "description": "查询实时天气与预报", // 给 LLM 的工具描述
  "icon": "assets/icon.png",
  "version": "1.0.0",
  "author": "…",
  "permissions": ["network:read"],   // 权限声明（见 §2.4）
  "entry": {
    "kind": "command" | "http" | "builtin",   // 执行后端
    "command": "scripts/weather.sh",
    "http": "POST https://…/invoke"
  },
  "functions": [                     // type=skill：LLM 工具 schema
    { "name": "get_weather", "description": "…",
      "parameters": { "type": "object", "properties": { "city": {"type":"string"} } } }
  ],
  // type=expert 追加：
  "system_prompt": "你是一位资深气象专家，说话简洁专业…",
  "skills": ["com.qibai.weather", "com.qibai.alarm"],   // 专家技能白名单
  "persona_binding": { "voice": "wenrou", "mood": "professional" }  // 可选：音色/形象绑定
}
```

**格式适配层**：`ManifestLoader` 做字段归一化（同义词映射表见 §2.1）+ 校验（id/type/kind/version/函数唯一性/权限合法集）。官方 WorkBuddy 规范（含必填字段/代码包结构）可获取后，只调整 `SYNONYM_MAP` 与校验规则，**不改业务代码**。

### 1.3 生命周期

```
安装（复制目录 → plugins/<id>/） → 扫描发现（启动/手动刷新） → 校验（schema）
→ 注册（注册表登记 + 权限确认） → 启停（enable/disable） → 调用（LLM tool → executor）
→ 卸载（删除目录 + 注册表摘除，幂等）
```

### 1.4 调用链（LLM 侧）

```
用户输入 → Engine.agent_loop
  → tools = 动态技能注册表 schema（内置 ∪ 已启用插件）∪ 当前专家白名单过滤
  → LLM function calling → SkillsRegistry.dispatch(function_name)
      → 内置：BuiltinKind 枚举分发到 SkillExecutor
      → 外部：SkillAdapter（command/http 执行体，10s 超时 + 失败降级）
  → 结果回填 → 最终回复
```

---

## 二、实现细节（契约折叠）

### 2.1 模块清单（qibai-core/src/）

| 模块 | 职责 | 关键点 |
|---|---|---|
| `plugin_manifest.rs` | Manifest 结构 + ManifestLoader | `SYNONYM_MAP`：type/capability/plugin_type/category、kind/backend/executor/entry_kind、entry/exec/invoke、command/script/cmd、http/url/endpoint、permissions/perms/grants、functions/tools/actions、system_prompt/prompt/systemPrompt、skills/skill_ids/whitelist、persona_binding/persona/binding；类型值同义词 skill\|capability、expert\|assistant、connector\|integration；kind 值 command\|cli\|shell、http\|api\|webhook、builtin\|native |
| `plugin_registry.rs` | 扫描/注册表/生命周期 | 扫描根 env `QIBAI_PLUGINS_DIR` 优先，否则 `data_dir()/plugins`；坏 manifest 记录 error 不中止扫描；id 冲突拒后到者；install/uninstall/set_enabled/grant 全部幂等；同版本重复装 = no-op、跨版本就地覆盖升级（保留启停与授权）；重扫走 `rescan()`，不会把已装条目误判成「id 冲突」；外部插件声明 kind=builtin 拒绝；`files:write` 对外部插件硬拒 |
| `skill_adapter.rs` | 外部执行体 | command：argv 为 `entry.command`，args 序列化为**单行 JSON 经 stdin** 传入；http：POST `entry.http`，body=`{"args": args}`；默认 10s 超时（可注入）；超时/失败 → `SkillOutcome::Denied("技能执行失败（已降级）：{原因}")`；connector → 预留拒绝 |
| `expert.rs` | 专家体系 | `ExpertRegistry`：activate/deactivate、`expert_prompt(base)` 追加专家段、`whitelist()`（空=全开放）、`persona_binding()`；voice 联动：已知 persona id（gudian/dushi/wenrou/huopo/reqing）经 `voice_profile::default_profile_for` 解析标签，未知字符串原样透传 |

### 2.2 内置技能迁移（T3）

6 个内置技能成为**内置插件包**（编译期注册，信任包 grants 默认全授，无需安装确认）：

| 插件 id | function name | 执行后端 | 权限声明 | 危险标记 |
|---|---|---|---|---|
| `com.qibai.launch_app` | `launch_app` | builtin | `system:launch` | 否 |
| `com.qibai.open_folder` | `open_folder` | builtin | `system:launch` | 否 |
| `com.qibai.set_reminder` | `set_reminder` | builtin | `reminder:set` | 否 |
| `com.qibai.check_code` | `check_code` | builtin | `files:read` | 否 |
| `com.qibai.open_social` | `open_social` | builtin | `network:read` | 否 |
| `com.qibai.execute_command` | `execute_command` | builtin | `command:run` | **是（逐次批准）** |

- `SkillsRegistry` 公开 API / `SkillOutcome` / `SkillError` / `SKILL_*` 常量 / `crate::skills::tool_schemas()` 自由函数 / `run_agent_loop` 签名 / engine 既有 IPC 全部保持。
- 新增构造器 `with_executor_and_plugins(settings, executor, plugin_settings)`。
- **去 match**：注册表按 function name 解析条目；内置条目经 `BuiltinKind` 枚举分发（枚举仅出现于内置包定义处），分发层无技能名字符串 match。
- `SKILL_EXECUTE_COMMAND` 特判（不进 last_skill）保留。

### 2.3 设置兼容与迁移（T3/T8）

- `SkillsSettings` 逐技能 bool 字段**保留原样**（旧 settings.json 兼容解析）。
- `Settings` 新增 `#[serde(default)] pub plugins: PluginSettings`：
  ```rust
  PluginSettings {
      enabled: HashMap<String, bool>,      // 插件 id → 启停（含内置 com.qibai.*）
      grants:  HashMap<String, Vec<String>>, // 插件 id → 已批准权限
      experts: HashMap<String, bool>,      // 专家 id → 启用（active 特判见 §2.5）
  }
  ```
- **迁移幂等**：注册表初始化时 `plugins.enabled` 为空 → 从旧 bool 播种（`com.qibai.launch_app` ← `skills.launch_app`，……；execute_command 无旧开关 → true）；非空 → 以之为准。不写盘，首次 save 自然落盘；重复加载结果一致。

### 2.4 权限模型三层门控（T8）

| 权限 | 含义 | 默认 |
|---|---|---|
| `files:read` / `files:write` | 文件系统访问 | 默认拒；**`files:write` 对外部插件永不可授（硬拒）**；只读技能（check_code 等）声明 `files:read` |
| `network:read` | 出网请求（HTTP 执行体/社交） | 安装时确认，仅配置后启用 |
| `command:run` | 执行外部命令 | 安装时确认；**危险命令仍需用户逐次批准** |
| `system:launch` / `reminder:set` | 系统启动类/提醒 | 仅内置守护技能持有 |
| `system:*` | 通配声明 | 仅内置预留 |

1. **manifest 声明**：permissions 字段校验（未知权限拒绝）。
2. **安装确认**：外部插件安装后默认 `enabled=false`、`grants=[]`；前端展示声明的权限，用户逐项确认 → `plugin_grant_permission(id, perm, approve)` 写入 grants。
3. **运行时强制**：`invoke` 统一判定（全局总开关 → 条目 enabled → granted 覆盖 manifest.permissions），不足 → `Denied("技能需要权限 {perm}，请在设置页授权")`。

**红线不因插件放宽**：
- 保险箱明文不进 LLM：`try_vault_intercept` 在 provider 之前拦截，插件无任何权限可关闭；
- `files:write` 对外部插件永不可授；
- 危险命令审批：内置 `execute_command` / `run_tests` 恒走 pending 审批流且**逐次**批准；外部插件（command / MCP，basename ∈ {rm, dd, shutdown, poweroff, reboot, halt, mv, chmod, chown, kill, pkill, systemctl, docker, curl, bash, sh, zsh, python, python3, node} 或 `mkfs.*` 前缀或 args 含 `-rf`）走 pending 审批流，`qibai://skill-approval` 事件 → 前端确认 → approve 后执行。
  外部插件批准一次即记入**任务域放行名单**（本次任务内同插件不再重复询问，批准后会连续调用同一 MCP 服务器的多个工具，逐步弹窗会被误认为卡死）；用户发起新一轮请求时清空名单。
- 审批幂等：同一 `request_id` 重复提交回放上次结果（「未重复执行」），不会报 `approval request not found`；pending 表是纯内存，重启后旧 id 一律按「已失效」话术回应。
- 缺件预检：启动命令在补全 PATH 后仍解析不到 → 直接 `Denied` + 补救建议，**不生成** pending。

### 2.5 专家体系装配（T5/T6）

- 专家 manifest（type=expert）经注册表扫描装配进 `ExpertRegistry`；`settings.plugins.experts` 中 value=true 的专家为激活态（首个 true 为 active，同时仅一位）。
- `engine.rs` 装配：system prompt = `build_system_prompt_for_persona` 后追加 active expert 的 `expert_prompt` 段；工具表 = 动态注册表 schema + `start_guard`，若 active expert 白名单非空则仅保留白名单内工具（按插件 id 或 function name 匹配；`start_guard` 恒保留）。

### 2.6 新 IPC（engine → 前端，字段已与前端 mock 逐字段对齐）

| IPC | 返回/入参 |
|---|---|
| `plugin_list()` | `[{id,type,name,description,icon,version,author,permissions,granted,enabled,installed,entry_kind,functions:[{name,description}],error,builtin}]` |
| `plugin_install(dir)` | ok（幂等） |
| `plugin_uninstall(id)` | ok（幂等；内置拒绝） |
| `plugin_set_enabled(id, enabled)` | ok（同步落盘） |
| `plugin_grant_permission(id, permission, approve)` | ok（files:write 外部硬拒；同步落盘） |
| `expert_list()` | `[{id,name,description,version,author,system_prompt,skills,persona_binding,active}]` |
| `expert_set_active(id)` | ok（空串 = 停用；同步落盘） |

### 2.7 样例插件包（T10）

- `plugins/examples/com.qibai.datetime`：**技能**样例。type=skill、kind=command、command=`scripts/now.sh`、permissions=`["command:run"]`、functions=`[{name:"get_datetime"}]`。`scripts/now.sh` 读取 stdin 单行 JSON（本示例不读取），stdout 输出当前日期时间回填 LLM。
- `plugins/examples/com.qibai.scientist`：**专家**样例。type=expert、system_prompt=科学家助手人设（严谨简洁、全龄友好）、skills 白名单=`["com.qibai.launch_app","com.qibai.open_folder"]`、persona_binding=`{"voice":"gudian","mood":"professional"}`。
- 可安装可调用验证（单测锁定）：`datetime_plugin_full_lifecycle`（install→list→未授权 Denied→grant→enable→tool_schemas 含 get_datetime→invoke 返回日期→落盘→卸载幂等）；`scientist_expert_activation_and_whitelist`（install→activate→prompt 含人设→白名单过滤生效）。

---

## 三、WorkBuddy 开放平台规范对齐（现状与缺口）

**已确认**（公开文档 + 页面数据）：
- 平台能力四类：**技能 / 专家 / 连接器 / Buddy 应用**；"专家=以专家人设提供专用问答与服务的智能体；技能=将资产封装为可复用的技能"
- 开放流程：注册入驻 → 资质认证 → 创建业务类型 → 完善基础信息与能力配置 → 测试调试 → 提交审核 → 发布 → 运行数据/版本维护

**缺口（如实标注）**：
- 技能/专家的**具体清单格式、代码包结构、必填字段、上传/审核要求**：公开文档未开放明细（页面为 JS 渲染，登录后开发者文档才有），需平台入驻或文档补充后逐字段对齐
- 因此本设计先行：**能力模型对齐 + 适配层（ManifestLoader 字段映射）已落地**，官方规范到位后只改 `SYNONYM_MAP` / 校验规则，无需重构业务

---

## 四、任务清单（实施结果）

| # | 任务 | 验收要点 | 状态与结果 |
|---|---|---|---|
| T1 | 插件包格式与 Manifest Schema | Schema 校验单测；WorkBuddy 字段同义词映射表 | 【已实施】`plugin_manifest.rs`；8 项单测（含 synonym_fields_are_normalized / rejects_bad_version / rejects_unknown_permission / expert_requires_system_prompt / skill_requires_entry_or_functions / duplicate_function_names_rejected） |
| T2 | 插件加载器 PluginLoader | 加载/卸载幂等单测；id 冲突检测；坏 manifest 拒绝 | 【已实施】`plugin_registry.rs`；7 项单测（bad_manifest_is_recorded_not_fatal / install_uninstall_idempotent_and_version_locked / external_builtin_kind_is_rejected / files_write_hard_reject_for_external_plugins 等） |
| T3 | 内置 6 技能迁移到统一注册通道 | 迁移后 179 基线全过；旧 settings.json 兼容 | 【已实施】迁移后 **206 全过**；SkillsSettings 保留、PluginSettings 兼容解析、迁移幂等 |
| T4 | 技能执行适配层 SkillAdapter | 三类执行体单测；超时/失败降级路径 | 【已实施】`skill_adapter.rs`；6 项单测（command stdin JSON / 超时降级 / 失败降级 / connector 预留拒绝 / 危险命令判定 / http POST） |
| T5 | 专家体系 Expert | 专家装配单测；与 persona/voice_profile 联动 | 【已实施】`expert.rs`；3 项单测（activate/prompt 装配 / settings 恢复 / voice 联动） |
| T6 | LLM agent loop 动态工具注入 | 工具注入单测；权限拒绝路径 | 【已实施】engine 动态 tool_schemas + 专家白名单过滤 + 专家提示词追加；3 项 engine 验收单测 |
| T7 | 前端「技能与专家」管理 UI | mock 冒烟截图；权限确认交互 | 【已实施】console 新 tab；冒烟截图 4 张（桌面全貌/权限交互态/专家卡/移动视口）见 `artifacts/smoke-plugins-*.jpg` |
| T8 | 安全与权限模型 | 权限门控单测；危险命令逐次批准 | 【已实施】三层门控 + files:write 硬拒 + 危险命令判定逐次批准；保险箱红线未旁路 |
| T9 | 测试与回归 | 基线只增不减；冒烟截图 | 【已实施】新增 27 项，**206 passed / 0 failed**（179 + 27） |
| T10 | 文档与打包 | 样例可安装可调用；zip 干净 | 【已实施】本文档定稿；样例插件×2 可安装可调用（单测锁定）；完整包 + 纯源码包重新打包 |

---

## 五、风险与后续决策点

1. **WorkBuddy 官方格式未到手**（最大不确定性）：适配层已落地，官方文档到位后只动 `SYNONYM_MAP` / 校验规则，不动业务。
2. **内置技能迁移回归**：206 基线（原 179 + 新增 27）全过，前端 mock 冒烟截图确认，作为迁移放行门已通过。
3. **权限安全**：插件可执行外部命令/HTTP，三层门控（声明→安装确认→运行时强制）+ 危险命令逐次批准 + files:write 硬拒；恶意插件防护（manifest 校验、id 唯一、坏包拒绝）。
4. **范围控制**：连接器（connector）与 Buddy 应用本期只预留类型与执行后端（connector 执行返回预留拒绝文案），完整平台接入需平台审核发布链路，属独立里程碑。
5. **打包受限项**：`src-tauri` 编译需真机，沙箱内未执行；`cargo build --workspace` 零警告，core 与前端可测部分全部完成。
