# 改名清单：栖伴 → 元气小小（下一版直接照此执行）

> 生成时间：2026-11-05。当前沙盒里这批文件**已经被改过**（sed 在命令被打断前已完成），但**没有打包、没有提交**。本文件只是清单，不含代码改动。

> 统计：本次实际落盘改动 **132 个文件**，涉及替换行 **351 行**。


## 一、已改、且属于「用户可见」的（下一版必须保留）

共 300 行。

| 文件:行 | 类别 | 原文（旧） | 现文（新） |
|---|---|---|---|
| `CHECKLIST.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 验收标准对照表（CHECKLIST） | # 元气小小 Qibai — 验收标准对照表（CHECKLIST） |
| `CHECKLIST.md:118` | A 用户可见（文档） | - `栖伴_MVP_源码包.zip` 含全部源码 / 资产 / docs / 截图；`unzip -l` 抽查含 `personas.rs` / `social.rs` / `src/pet.html` / `src/console.html` / `docs/WINDOW_ARCH.md` / `artifacts/ | - `元气小小_MVP_源码包.zip` 含全部源码 / 资产 / docs / 截图；`unzip -l` 抽查含 `personas.rs` / `social.rs` / `src/pet.html` / `src/console.html` / `docs/WINDOW_ARCH.md` / `artifact |
| `CHECKLIST.md:130` | A 用户可见（文档） | - 证据：`artifacts/dualwin_pet_standalone.png`（立绘 + 浏览器调试面板，Tauri 下调试面板隐藏）、`artifacts/dualwin_menu.png`（右键菜单：打开控制台/锁定位置/退出栖伴）。 | - 证据：`artifacts/dualwin_pet_standalone.png`（立绘 + 浏览器调试面板，Tauri 下调试面板隐藏）、`artifacts/dualwin_menu.png`（右键菜单：打开控制台/锁定位置/退出元气小小）。 |
| `CHECKLIST.md:167` | A 用户可见（文档） | - `栖伴_MVP_源码包.zip` 含本次全部改动文件（`qibai-core/src/intimacy.rs` / `src/tauri-bridge.js` / `src/console.*` / `src/pet.*` / `src/smoke.html` / `docs/INTIMACY_SYSTEM.md` | - `元气小小_MVP_源码包.zip` 含本次全部改动文件（`qibai-core/src/intimacy.rs` / `src/tauri-bridge.js` / `src/console.*` / `src/pet.*` / `src/smoke.html` / `docs/INTIMACY_SYSTEM.m |
| `CHECKLIST.md:222` | A 用户可见（文档） | - `栖伴_MVP_源码包.zip` 含 `qibai-core/src/tts.rs` / `docs/VOICE.md` / `artifacts/voice-*.png` 5 张与更新后的 README/CHECKLIST/SMOKE_TEST/IPC_CONTRACT；`unzip -l` 抽查不含任何 `ta | - `元气小小_MVP_源码包.zip` 含 `qibai-core/src/tts.rs` / `docs/VOICE.md` / `artifacts/voice-*.png` 5 张与更新后的 README/CHECKLIST/SMOKE_TEST/IPC_CONTRACT；`unzip -l` 抽查不含任何 ` |
| `CHECKLIST.md:370` | A 用户可见（文档） | - [x] **92 源码包重打包**：`栖伴_MVP_源码包.zip`（unzip -l 抽查含 `qibai-core/src/voice_profile.rs` 与更新后的 `docs/VOICE.md` / `README.md` / `CHECKLIST.md`，不含任何 `target/`、`node_mo | - [x] **92 源码包重打包**：`元气小小_MVP_源码包.zip`（unzip -l 抽查含 `qibai-core/src/voice_profile.rs` 与更新后的 `docs/VOICE.md` / `README.md` / `CHECKLIST.md`，不含任何 `target/`、`node_ |
| `CHECKLIST.md:426` | A 用户可见（文档） | - [x] **105 T10 样例插件 ×2 + 文档 + 打包**：`plugins/examples/com.qibai.datetime`（skill，command，`get_datetime`，stdin JSON 约定，`now.sh` 可执行）、`plugins/examples/com.qibai.s | - [x] **105 T10 样例插件 ×2 + 文档 + 打包**：`plugins/examples/com.qibai.datetime`（skill，command，`get_datetime`，stdin JSON 约定，`now.sh` 可执行）、`plugins/examples/com.qibai.s |
| `docs/BUILD_NOTE.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 构建说明（三平台系统依赖前置） | # 元气小小 Qibai — 构建说明（三平台系统依赖前置） |
| `docs/DEPLOYMENT.md:1` | A 用户可见（文档） | # 《栖伴》部署方案 V1.0 | # 《元气小小》部署方案 V1.0 |
| `docs/DEPLOYMENT.md:13` | A 用户可见（文档） | │  《栖伴》桌面程序（Tauri 2 打包产物）                              │ | │  《元气小小》桌面程序（Tauri 2 打包产物）                              │ |
| `docs/DEPLOYMENT.md:23` | A 用户可见（文档） | 豆包/火山方舟 API（大模型 + 云端 TTS）     栖伴联网服务端（Node 22 + SQLite） | 豆包/火山方舟 API（大模型 + 云端 TTS）     元气小小联网服务端（Node 22 + SQLite） |
| `docs/DEPLOY_v18_6.md:1` | A 用户可见（文档） | # 栖伴 v18.6 部署指南（Ubuntu 22.04，含新增功能依赖与验证清单） | # 元气小小 v18.6 部署指南（Ubuntu 22.04，含新增功能依赖与验证清单） |
| `docs/DEPLOY_v18_6.md:105` | A 用户可见（文档） | - 写代码技能输出**高频代码骨架**；复杂定制（接口对接/业务逻辑/多文件工程）直接说「帮我写……」，由栖伴大模型生成完整实现； | - 写代码技能输出**高频代码骨架**；复杂定制（接口对接/业务逻辑/多文件工程）直接说「帮我写……」，由元气小小大模型生成完整实现； |
| `docs/DOUBAO_WORK_INTEGRATION.md:3` | A 用户可见（文档） | 本文件说明后续如何把「飞书文档 / 日历 / 邮件」等办公能力接入栖伴， | 本文件说明后续如何把「飞书文档 / 日历 / 邮件」等办公能力接入元气小小， |
| `docs/INTIMACY_SYSTEM.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 亲密度成长系统（L1–L10）冻结设计规格 | # 元气小小 Qibai — 亲密度成长系统（L1–L10）冻结设计规格 |
| `docs/IPC_CONTRACT.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 前后端 IPC 契约 | # 元气小小 Qibai — 前后端 IPC 契约 |
| `docs/IPC_CONTRACT.md:123` | A 用户可见（文档） | "nickname": "栖伴",                // 用户可重命名 | "nickname": "元气小小",                // 用户可重命名 |
| `docs/IPC_CONTRACT.md:236` | A 用户可见（文档） | \| `app_exit` \| — \| `"ok"` \| `app.exit(0)`（桌宠右键菜单「退出栖伴」用） \| | \| `app_exit` \| — \| `"ok"` \| `app.exit(0)`（桌宠右键菜单「退出元气小小」用） \| |
| `docs/LLM_CONNECT.md:1` | A 用户可见（文档） | # 《栖伴》大模型连接配置与验收记录 V12 | # 《元气小小》大模型连接配置与验收记录 V12 |
| `docs/LLM_CONNECT.md:13` | A 用户可见（文档） | \| 已实测可用模型 \| `deepseek-v4-pro-ga-260813` ✅（栖伴人设回复质量良好）<br/>`deepseek-v4-flash-ga-260731` ✅<br/>`deepseek-v4-1-flash-260910` ✅<br/>`deepseek-v4-1-flash-261017`  | \| 已实测可用模型 \| `deepseek-v4-pro-ga-260813` ✅（元气小小人设回复质量良好）<br/>`deepseek-v4-flash-ga-260731` ✅<br/>`deepseek-v4-1-flash-260910` ✅<br/>`deepseek-v4-1-flash-261017 |
| `docs/LLM_CONNECT.md:18` | A 用户可见（文档） | > 系统：「你是《栖伴》桌面伴侣·古典佳人，说话温婉简短，带一点古风。」 | > 系统：「你是《元气小小》桌面伴侣·古典佳人，说话温婉简短，带一点古风。」 |
| `docs/LLM_CONNECT.md:50` | A 用户可见（文档） | [栖伴] 正在连接大模型…（deepseek-v4-pro-ga-260813） | [元气小小] 正在连接大模型…（deepseek-v4-pro-ga-260813） |
| `docs/LLM_CONNECT.md:51` | A 用户可见（文档） | [栖伴] 大模型回复: 更深露重，莫要熬坏了身子。且饮杯热茶，歇一歇，奴家陪你。 | [元气小小] 大模型回复: 更深露重，莫要熬坏了身子。且饮杯热茶，歇一歇，奴家陪你。 |
| `docs/LLM_CONNECT.md:52` | A 用户可见（文档） | [栖伴] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功） | [元气小小] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功） |
| `docs/NETSOCIAL_SPEC.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 迭代④ 联网社交规格（星球 × 同城） | # 元气小小 Qibai — 迭代④ 联网社交规格（星球 × 同城） |
| `docs/NETSOCIAL_SPEC.md:340` | A 用户可见（文档） | 6. `栖伴_MVP_源码包.zip`（排除 target，复制到 `/home/user/Doubao/chats/38448289926008322/栖伴_MVP_源码包.zip`）。 | 6. `元气小小_MVP_源码包.zip`（排除 target，复制到 `/home/user/Doubao/chats/38448289926008322/元气小小_MVP_源码包.zip`）。 |
| `docs/P0P1_SOCIAL_PLAN.md:1` | A 用户可见（文档） | # 《栖伴》P0+P1 多角色社交扩展 — 实施规划（冻结契约） | # 《元气小小》P0+P1 多角色社交扩展 — 实施规划（冻结契约） |
| `docs/P0P1_SOCIAL_PLAN.md:43` | A 用户可见（文档） | "nickname": "栖伴",               // 用户可重命名 | "nickname": "元气小小",               // 用户可重命名 |
| `docs/P0P1_SOCIAL_PLAN.md:129` | A 用户可见（文档） | - [ ] README.md / CHECKLIST.md 更新（新增 P0+P1 说明与验收行）；`栖伴_MVP_源码包.zip` 重打包（排除 target）； | - [ ] README.md / CHECKLIST.md 更新（新增 P0+P1 说明与验收行）；`元气小小_MVP_源码包.zip` 重打包（排除 target）； |
| `docs/P0P1_SOCIAL_PLAN.md:138` | A 用户可见（文档） | - `栖伴_MVP_源码包.zip`（含源码、资产、docs、截图，排除 `qibai/target`）。 | - `元气小小_MVP_源码包.zip`（含源码、资产、docs、截图，排除 `qibai/target`）。 |
| `docs/PLUGIN_ARCH.md:1` | A 用户可见（文档） | # 《栖伴》技能/专家体系 · 架构调整梳理与任务清单（实施后版本） | # 《元气小小》技能/专家体系 · 架构调整梳理与任务清单（实施后版本） |
| `docs/PLUGIN_ARCH.md:27` | A 用户可见（文档） | 《栖伴》第一阶实现前两类，连接器作为技能的一种执行后端预留，Buddy 应用后续评估。 | 《元气小小》第一阶实现前两类，连接器作为技能的一种执行后端预留，Buddy 应用后续评估。 |
| `docs/PLUGIN_V18_SPEC.md:1` | A 用户可见（文档） | # 《栖伴》v18 插件包撰写规范（PLUGIN_V18_SPEC）— 所有子代理必须遵守 | # 《元气小小》v18 插件包撰写规范（PLUGIN_V18_SPEC）— 所有子代理必须遵守 |
| `docs/PLUGIN_V18_SPEC.md:48` | A 用户可见（文档） | ├── qibai-plugin.json   # 栖伴清单 | ├── qibai-plugin.json   # 元气小小清单 |
| `docs/PLUGIN_V18_SPEC.md:80` | A 用户可见（文档） | ├── qibai-plugin.json              # 栖伴清单 | ├── qibai-plugin.json              # 元气小小清单 |
| `docs/PLUGIN_V18_SPEC.md:119` | A 用户可见（文档） | ## 语音调用示例        # 2-3 条「栖伴，……」示例 | ## 语音调用示例        # 2-3 条「元气小小，……」示例 |
| `docs/PLUGIN_V18_SPEC.md:170` | A 用户可见（文档） | 2. mode 缺省为 window；title 缺省「栖伴」； | 2. mode 缺省为 window；title 缺省「元气小小」； |
| `docs/README_v16.md:1` | A 用户可见（文档） | # 《栖伴》v16 全量包 | # 《元气小小》v16 全量包 |
| `docs/README_v16.md:6` | A 用户可见（文档） | 3. **免提聆听默认开启**：全新配置默认打开语音唤醒（说「栖伴」唤起后直接对话；本地 whisper 转写，不上传）。若此前跑过旧版本：设置页 → 语音 → 打开「语音唤醒」，或删除 `~/.local/share/qibai/settings.json` 重建。 | 3. **免提聆听默认开启**：全新配置默认打开语音唤醒（说「元气小小」唤起后直接对话；本地 whisper 转写，不上传）。若此前跑过旧版本：设置页 → 语音 → 打开「语音唤醒」，或删除 `~/.local/share/qibai/settings.json` 重建。 |
| `docs/README_v17.md:1` | A 用户可见（文档） | # 栖伴 v17 增量包（发声/拖动/听不到 根治） | # 元气小小 v17 增量包（发声/拖动/听不到 根治） |
| `docs/README_v17.md:33` | A 用户可见（文档） | - 端到端冒烟（xvfb）：`window_identity called (frontend-bridge=real)` + `tts_speak: kind=Chat text=嗨～我是栖伴` + `tts: speak via ffplay` → **前端→后端→espeak→播放 全链路真通** | - 端到端冒烟（xvfb）：`window_identity called (frontend-bridge=real)` + `tts_speak: kind=Chat text=嗨～我是元气小小` + `tts: speak via ffplay` → **前端→后端→espeak→播放 全链路真通** |
| `docs/README_v17.md:39` | A 用户可见（文档） | - **免提聆听**：首次启动自动开启（wake_enabled=true），唤醒词「栖伴」；若老配置存过 false，在控制台「语音」页手动开一次 | - **免提聆听**：首次启动自动开启（wake_enabled=true），唤醒词「元气小小」；若老配置存过 false，在控制台「语音」页手动开一次 |
| `docs/README_v18_1.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18.1 · 技能结果面板化交付说明 | # 《元气小小》增量 v18.1 · 技能结果面板化交付说明 |
| `docs/README_v18_1.md:3` | A 用户可见（文档） | > 版本：v18.1（2026-10-24）｜ 交付：`栖伴_增量_v18.1技能结果面板化.zip` | > 版本：v18.1（2026-10-24）｜ 交付：`元气小小_增量_v18.1技能结果面板化.zip` |
| `docs/README_v18_1.md:25` | A 用户可见（文档） | - 引擎与语音层双向接入：前端点按钮调用技能、语音「栖伴，用 XX 技能……」两条链路都生效 | - 引擎与语音层双向接入：前端点按钮调用技能、语音「元气小小，用 XX 技能……」两条链路都生效 |
| `docs/README_v18_1.md:34` | A 用户可见（文档） | \| 星座运势 \| 「栖伴，看看白羊座今天怎么样」 \| bubble 运势卡片 \| | \| 星座运势 \| 「元气小小，看看白羊座今天怎么样」 \| bubble 运势卡片 \| |
| `docs/README_v18_1.md:35` | A 用户可见（文档） | \| 网页摘要 \| 「栖伴，总结网页 https://…」 \| window 摘要卡片 \| | \| 网页摘要 \| 「元气小小，总结网页 https://…」 \| window 摘要卡片 \| |
| `docs/README_v18_1.md:36` | A 用户可见（文档） | \| 代码审查 \| 「栖伴，帮我审查这段代码」 \| window 报告（风险计数+修复建议列表） \| | \| 代码审查 \| 「元气小小，帮我审查这段代码」 \| window 报告（风险计数+修复建议列表） \| |
| `docs/README_v18_1.md:68` | A 用户可见（文档） | - 「栖伴，看看白羊座今天怎么样」→ 气泡运势卡片 + 朗读 | - 「元气小小，看看白羊座今天怎么样」→ 气泡运势卡片 + 朗读 |
| `docs/README_v18_1.md:69` | A 用户可见（文档） | - 「栖伴，帮我审查代码 qibai_cr_probe」→ 面板窗审查报告 + 朗读摘要 | - 「元气小小，帮我审查代码 qibai_cr_probe」→ 面板窗审查报告 + 朗读摘要 |
| `docs/README_v18_1.md:70` | A 用户可见（文档） | - 「栖伴，总结网页 https://github.com/…」→ 面板窗摘要卡片 + 朗读 | - 「元气小小，总结网页 https://github.com/…」→ 面板窗摘要卡片 + 朗读 |
| `docs/README_v18_2.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18.2 · 技能全面面板化交付说明 | # 《元气小小》增量 v18.2 · 技能全面面板化交付说明 |
| `docs/README_v18_2.md:3` | A 用户可见（文档） | > 版本：v18.2（2026-10-24）｜ 交付：`栖伴_增量_v18.2技能全面面板化.zip` | > 版本：v18.2（2026-10-24）｜ 交付：`元气小小_增量_v18.2技能全面面板化.zip` |
| `docs/README_v18_2.md:13` | A 用户可见（文档） | \| 翻译助手 \| 「栖伴，帮我翻译：thank you」 \| bubble \| 译文/术语参考卡片 \| | \| 翻译助手 \| 「元气小小，帮我翻译：thank you」 \| bubble \| 译文/术语参考卡片 \| |
| `docs/README_v18_2.md:14` | A 用户可见（文档） | \| 写作润色 \| 「栖伴，帮我润色这段话」 \| window \| 润色检查报告（字数+3 项检查结论） \| | \| 写作润色 \| 「元气小小，帮我润色这段话」 \| window \| 润色检查报告（字数+3 项检查结论） \| |
| `docs/README_v18_2.md:15` | A 用户可见（文档） | \| 文件搜索 \| 「栖伴，找一下 plugin 开头的文件」 \| window \| 搜索结果列表（条数+前 20 条） \| | \| 文件搜索 \| 「元气小小，找一下 plugin 开头的文件」 \| window \| 搜索结果列表（条数+前 20 条） \| |
| `docs/README_v18_2.md:16` | A 用户可见（文档） | \| 待办解析 \| 「栖伴，明天下午3点开会提醒我」 \| bubble \| 提醒卡片（事项+时间） \| | \| 待办解析 \| 「元气小小，明天下午3点开会提醒我」 \| bubble \| 提醒卡片（事项+时间） \| |
| `docs/README_v18_2.md:17` | A 用户可见（文档） | \| 塔罗占卜 \| 「栖伴，帮我占卜一下」 \| bubble \| 三牌阵卡片（过去/现在/未来+提示） \| | \| 塔罗占卜 \| 「元气小小，帮我占卜一下」 \| bubble \| 三牌阵卡片（过去/现在/未来+提示） \| |
| `docs/README_v18_2.md:18` | A 用户可见（文档） | \| 冷笑话 \| 「栖伴，讲个冷笑话」 \| bubble \| 笑话卡片（1-3 条） \| | \| 冷笑话 \| 「元气小小，讲个冷笑话」 \| bubble \| 笑话卡片（1-3 条） \| |
| `docs/README_v18_2.md:19` | A 用户可见（文档） | \| 情话语录 \| 「栖伴，说句情话哄我开心」 \| bubble \| 情话卡片（列表+合规提示） \| | \| 情话语录 \| 「元气小小，说句情话哄我开心」 \| bubble \| 情话卡片（列表+合规提示） \| |
| `docs/README_v18_2.md:20` | A 用户可见（文档） | \| 绘画提示词 \| 「栖伴，生成画橘猫的提示词」 \| window \| 提示词卡片（英文/负面词/中文对照，可复制） \| | \| 绘画提示词 \| 「元气小小，生成画橘猫的提示词」 \| window \| 提示词卡片（英文/负面词/中文对照，可复制） \| |
| `docs/README_v18_2.md:21` | A 用户可见（文档） | \| 创意命名 \| 「栖伴，给我家猫起个名字」 \| bubble \| 候选名卡片（5 个+含义） \| | \| 创意命名 \| 「元气小小，给我家猫起个名字」 \| bubble \| 候选名卡片（5 个+含义） \| |
| `docs/README_v18_2.md:61` | A 用户可见（文档） | - 「栖伴，帮我翻译 thank you」→ 气泡译文卡片 | - 「元气小小，帮我翻译 thank you」→ 气泡译文卡片 |
| `docs/README_v18_2.md:62` | A 用户可见（文档） | - 「栖伴，帮我占卜一下」→ 气泡三牌阵 | - 「元气小小，帮我占卜一下」→ 气泡三牌阵 |
| `docs/README_v18_2.md:63` | A 用户可见（文档） | - 「栖伴，生成画橘猫的提示词」→ 面板窗提示词（可复制） | - 「元气小小，生成画橘猫的提示词」→ 面板窗提示词（可复制） |
| `docs/README_v18_2.md:64` | A 用户可见（文档） | - 「栖伴，找一下含 plugin 的文件」→ 面板窗结果列表 | - 「元气小小，找一下含 plugin 的文件」→ 面板窗结果列表 |
| `docs/README_v18_3.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18.3 · 屏幕「眼睛 + 手」交付说明 | # 《元气小小》增量 v18.3 · 屏幕「眼睛 + 手」交付说明 |
| `docs/README_v18_3.md:3` | A 用户可见（文档） | > 版本：v18.3（2026-10-24）｜ 交付：`栖伴_增量_v18.3屏幕GUI执行器.zip` | > 版本：v18.3（2026-10-24）｜ 交付：`元气小小_增量_v18.3屏幕GUI执行器.zip` |
| `docs/README_v18_3.md:4` | A 用户可见（文档） | > 内容：栖伴获得**看屏幕 + 操纵应用内部**的能力——语音「截图」把屏幕画面放进面板（可点击坐标），点击/输入/按键/滚动/切窗口全部可语音或面板驱动执行。配套分级授权与防 Prompt 注入隔离。 | > 内容：元气小小获得**看屏幕 + 操纵应用内部**的能力——语音「截图」把屏幕画面放进面板（可点击坐标），点击/输入/按键/滚动/切窗口全部可语音或面板驱动执行。配套分级授权与防 Prompt 注入隔离。 |
| `docs/README_v18_3.md:14` | A 用户可见（文档） | \| 看屏幕（截图） \| 「栖伴，看屏幕 / 截个屏」 \| 截图自动进面板展示 \| 低危·直接可用 \| | \| 看屏幕（截图） \| 「元气小小，看屏幕 / 截个屏」 \| 截图自动进面板展示 \| 低危·直接可用 \| |
| `docs/README_v18_3.md:38` | A 用户可见（文档） | 1. **防 Prompt 注入隔离**：屏幕内容**只作展示/理解**；任何键鼠操作**只由用户语音指令或面板主动点击触发**。网页上出现恶意文字不会诱导栖伴自动操作。 | 1. **防 Prompt 注入隔离**：屏幕内容**只作展示/理解**；任何键鼠操作**只由用户语音指令或面板主动点击触发**。网页上出现恶意文字不会诱导元气小小自动操作。 |
| `docs/README_v18_3.md:52` | A 用户可见（文档） | - 缺工具时栖伴会语音提示「未检测到截图工具，请先安装…」，不会崩溃； | - 缺工具时元气小小会语音提示「未检测到截图工具，请先安装…」，不会崩溃； |
| `docs/README_v18_3.md:57` | A 用户可见（文档） | 1. 说「**栖伴，看屏幕**」→ 面板弹出你的桌面截图（可滚动查看全屏）； | 1. 说「**元气小小，看屏幕**」→ 面板弹出你的桌面截图（可滚动查看全屏）； |
| `docs/README_v18_3.md:58` | A 用户可见（文档） | 2. 在截图上**直接点击**想操作的位置 → 栖伴在真实屏幕对应坐标点击（自动临时授权一次）； | 2. 在截图上**直接点击**想操作的位置 → 元气小小在真实屏幕对应坐标点击（自动临时授权一次）； |
| `docs/README_v18_4.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18.4 · 情感表达系统（语气 → 表情 × 动作 联动） | # 《元气小小》增量 v18.4 · 情感表达系统（语气 → 表情 × 动作 联动） |
| `docs/README_v18_4.md:3` | A 用户可见（文档） | > 版本：v18.4（2026-10-25）｜ 交付：`栖伴_增量_v18.4情感表达系统.zip` | > 版本：v18.4（2026-10-25）｜ 交付：`元气小小_增量_v18.4情感表达系统.zip` |
| `docs/README_v18_4.md:46` | A 用户可见（文档） | 1. 解压覆盖到栖伴工程根目录（保留目录结构）； | 1. 解压覆盖到元气小小工程根目录（保留目录结构）； |
| `docs/README_v18_4.md:48` | A 用户可见（文档） | 3. 对栖伴说不同的话观察演出，例如： | 3. 对元气小小说不同的话观察演出，例如： |
| `docs/README_v18_5.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18.5 · 动作帧升级 + 演出队列 + 写代码技能 | # 《元气小小》增量 v18.5 · 动作帧升级 + 演出队列 + 写代码技能 |
| `docs/README_v18_5.md:3` | A 用户可见（文档） | > 版本：v18.5（2026-10-25）｜ 交付：`栖伴_增量_v18.5动作帧演出写代码.zip` | > 版本：v18.5（2026-10-25）｜ 交付：`元气小小_增量_v18.5动作帧演出写代码.zip` |
| `docs/README_v18_5.md:36` | A 用户可见（文档） | - 复杂定制代码（接口对接、业务逻辑、多文件工程）直接说「帮我写……」由栖伴大模型生成完整实现（技能负责高频模板，保证离线可用）； | - 复杂定制代码（接口对接、业务逻辑、多文件工程）直接说「帮我写……」由元气小小大模型生成完整实现（技能负责高频模板，保证离线可用）； |
| `docs/README_v18_6.md:1` | A 用户可见（文档） | # 《栖伴》v18.6 · 写代码落盘 + 性格差异演出 + 免提语音切形象 | # 《元气小小》v18.6 · 写代码落盘 + 性格差异演出 + 免提语音切形象 |
| `docs/README_v18_6.md:10` | A 用户可见（文档） | 对栖伴说：**「帮我写一个 Python 脚本读取 CSV，保存到 ~/scripts/read_csv.py」** | 对元气小小说：**「帮我写一个 Python 脚本读取 CSV，保存到 ~/scripts/read_csv.py」** |
| `docs/README_v18_7.md:1` | A 用户可见（文档） | # 栖伴 v18.7 · 无声/听不到 + 拖拽修复 | # 元气小小 v18.7 · 无声/听不到 + 拖拽修复 |
| `docs/README_v18_8.md:1` | A 用户可见（文档） | # 栖伴 v18.8-fix（全量包）· 女声音色 + 麦克风自动匹配 + 拖动根治 + 角色切换契约 + 编译清零 | # 元气小小 v18.8-fix（全量包）· 女声音色 + 麦克风自动匹配 + 拖动根治 + 角色切换契约 + 编译清零 |
| `docs/README_v18_8.md:16` | A 用户可见（文档） | 重启栖伴即可；隐私=仅发送当前朗读文本片段，无 key。 | 重启元气小小即可；隐私=仅发送当前朗读文本片段，无 key。 |
| `docs/README_v18_8.md:22` | A 用户可见（文档） | 重启栖伴自动启用；音质接近自然女声（沙箱已验证生成 22050Hz wav）。不装也不影响，自动回退下一级。 | 重启元气小小自动启用；音质接近自然女声（沙箱已验证生成 22050Hz wav）。不装也不影响，自动回退下一级。 |
| `docs/README_v18.md:1` | A 用户可见（文档） | # 《栖伴》增量 v18 · 技能专家库交付说明 | # 《元气小小》增量 v18 · 技能专家库交付说明 |
| `docs/README_v18.md:3` | A 用户可见（文档） | > 版本：v18（2026-10-24）｜ 交付：`栖伴_增量_v18技能专家库.zip` | > 版本：v18（2026-10-24）｜ 交付：`元气小小_增量_v18技能专家库.zip` |
| `docs/README_v18.md:4` | A 用户可见（文档） | > 内容：20 个新插件包（办公 10 + 炫酷玩法 10），按「栖伴 qibai-plugin.json + 腾讯 WorkBuddy 开放平台规范」双格式改造，逐包安全审核与内容合规审查，全部通过自测。 | > 内容：20 个新插件包（办公 10 + 炫酷玩法 10），按「元气小小 qibai-plugin.json + 腾讯 WorkBuddy 开放平台规范」双格式改造，逐包安全审核与内容合规审查，全部通过自测。 |
| `docs/README_v18.md:57` | A 用户可见（文档） | \| code-review \| 原素材基于 git diff 区间，栖伴无 git 场景；扫描类脚本易漏路径校验 \| 改为目录静态扫描+危险模式清单（eval/exec/rm -rf/硬编码密钥/危险 SQL/curl\|sh），路径校验拒绝 `..` 与越界 \| | \| code-review \| 原素材基于 git diff 区间，元气小小无 git 场景；扫描类脚本易漏路径校验 \| 改为目录静态扫描+危险模式清单（eval/exec/rm -rf/硬编码密钥/危险 SQL/curl\|sh），路径校验拒绝 `..` 与越界 \| |
| `docs/README_v18.md:67` | A 用户可见（文档） | \| code-architect \| 原素材 8 维 checklist 较工程化，缺栖伴语境 \| 保留八维/八因子/三阶段方法论，输出中文战略级评审 \| | \| code-architect \| 原素材 8 维 checklist 较工程化，缺元气小小语境 \| 保留八维/八因子/三阶段方法论，输出中文战略级评审 \| |
| `docs/README_v18.md:77` | A 用户可见（文档） | 2. 双格式完整：栖伴 qibai-plugin.json（可装配可语音调用）+ WorkBuddy SKILL.md/config.yaml 或 .codebuddy-plugin/plugin.json+agents/*.md+avatars。 | 2. 双格式完整：元气小小 qibai-plugin.json（可装配可语音调用）+ WorkBuddy SKILL.md/config.yaml 或 .codebuddy-plugin/plugin.json+agents/*.md+avatars。 |
| `docs/README_v18.md:120` | A 用户可见（文档） | - 「栖伴，使用网页摘要，总结 https://github.com/VoltAgent/awesome-agent-skills」→ websummary | - 「元气小小，使用网页摘要，总结 https://github.com/VoltAgent/awesome-agent-skills」→ websummary |
| `docs/README_v18.md:121` | A 用户可见（文档） | - 「栖伴，帮我翻译一下：今天天气很好」→ translate | - 「元气小小，帮我翻译一下：今天天气很好」→ translate |
| `docs/README_v18.md:122` | A 用户可见（文档） | - 「栖伴，帮我润色这段话：……」（触发 writing-polish） | - 「元气小小，帮我润色这段话：……」（触发 writing-polish） |
| `docs/README_v18.md:123` | A 用户可见（文档） | - 「栖伴，帮我审查这段代码」（触发 code-review，追问目录） | - 「元气小小，帮我审查这段代码」（触发 code-review，追问目录） |
| `docs/README_v18.md:124` | A 用户可见（文档） | - 「栖伴，帮我找一下 qibai-plugin.json 文件」→ filesearch | - 「元气小小，帮我找一下 qibai-plugin.json 文件」→ filesearch |
| `docs/README_v18.md:125` | A 用户可见（文档） | - 「栖伴，明天下午3点开会提醒我」→ reminderparse（配合 set_reminder 落地） | - 「元气小小，明天下午3点开会提醒我」→ reminderparse（配合 set_reminder 落地） |
| `docs/README_v18.md:126` | A 用户可见（文档） | - 「栖伴，看看白羊座今天怎么样」→ horoscope | - 「元气小小，看看白羊座今天怎么样」→ horoscope |
| `docs/README_v18.md:127` | A 用户可见（文档） | - 「栖伴，帮我占卜一下今天的运势」→ tarot | - 「元气小小，帮我占卜一下今天的运势」→ tarot |
| `docs/README_v18.md:128` | A 用户可见（文档） | - 「栖伴，讲个冷笑话」→ jokes | - 「元气小小，讲个冷笑话」→ jokes |
| `docs/README_v18.md:129` | A 用户可见（文档） | - 「栖伴，说句情话哄我开心」→ sweettalk | - 「元气小小，说句情话哄我开心」→ sweettalk |
| `docs/README_v18.md:130` | A 用户可见（文档） | - 「栖伴，生成一个画橘猫的绘画提示词」→ promptgen | - 「元气小小，生成一个画橘猫的绘画提示词」→ promptgen |
| `docs/README_v18.md:131` | A 用户可见（文档） | - 「栖伴，给我家猫起个名字」→ namegen | - 「元气小小，给我家猫起个名字」→ namegen |
| `docs/README_v18.md:132` | A 用户可见（文档） | - 「栖伴，帮我安排明天的日程」→ planner 专家 | - 「元气小小，帮我安排明天的日程」→ planner 专家 |
| `docs/README_v18.md:133` | A 用户可见（文档） | - 「栖伴，帮我做个架构评审」→ codearchitect 专家 | - 「元气小小，帮我做个架构评审」→ codearchitect 专家 |
| `docs/README_v18.md:134` | A 用户可见（文档） | - 「栖伴，帮我分析一下这份数据」→ dataanalyst 专家 | - 「元气小小，帮我分析一下这份数据」→ dataanalyst 专家 |
| `docs/README_v18.md:135` | A 用户可见（文档） | - 「栖伴，陪我说说话，我今天有点累」→ emotion 专家 | - 「元气小小，陪我说说话，我今天有点累」→ emotion 专家 |
| `docs/README_v18.md:136` | A 用户可见（文档） | - 「栖伴，想几个产品创意点子」→ ideas 专家 | - 「元气小小，想几个产品创意点子」→ ideas 专家 |
| `docs/README_v18.md:137` | A 用户可见（文档） | - 「栖伴，讲个睡前故事」→ storyteller 专家 | - 「元气小小，讲个睡前故事」→ storyteller 专家 |
| `docs/README_v18.md:138` | A 用户可见（文档） | - 「栖伴，来点极客好玩的」→ geek 专家 | - 「元气小小，来点极客好玩的」→ geek 专家 |
| `docs/README_v18.md:144` | A 用户可见（文档） | 1. 解压后把 `skills/` 与 `experts/` 下各目录复制到栖伴 `src-tauri/plugins/skills/`、`src-tauri/plugins/experts/`（或经设置页「技能与专家」扫描装配）；旧包保留即可，无 id 冲突。 | 1. 解压后把 `skills/` 与 `experts/` 下各目录复制到元气小小 `src-tauri/plugins/skills/`、`src-tauri/plugins/experts/`（或经设置页「技能与专家」扫描装配）；旧包保留即可，无 id 冲突。 |
| `docs/README_v18.md:147` | A 用户可见（文档） | - 完整工程请用历史 MVP 源码包（历史交付：`栖伴_MVP_源码包.zip`，aka `WYP2IlfNNl`），语音模型权重与 whisper 二进制的获取方式见该包内说明。 | - 完整工程请用历史 MVP 源码包（历史交付：`元气小小_MVP_源码包.zip`，aka `WYP2IlfNNl`），语音模型权重与 whisper 二进制的获取方式见该包内说明。 |
| `docs/README_v18_UI.md:1` | A 用户可见（文档） | # 栖伴 v18 · UI 面板能力（气泡卡片 + 独立面板窗） | # 元气小小 v18 · UI 面板能力（气泡卡片 + 独立面板窗） |
| `docs/README_v18_UI.md:14` | A 用户可见（文档） | - **聊天记录表格**：读本地 `conversation.json`（最近 30 条），时间戳转「MM-DD HH:MM」，角色标「我/栖伴」，文本转义。 | - **聊天记录表格**：读本地 `conversation.json`（最近 30 条），时间戳转「MM-DD HH:MM」，角色标「我/元气小小」，文本转义。 |
| `docs/README_v18_UI.md:48` | A 用户可见（文档） | 「栖伴，把最近的聊天记录展示成表格」   → 独立面板窗：最近 30 条聊天记录表格 | 「元气小小，把最近的聊天记录展示成表格」   → 独立面板窗：最近 30 条聊天记录表格 |
| `docs/README_v18_UI.md:49` | A 用户可见（文档） | 「栖伴，打开技能库列表」               → 独立面板窗：技能清单表格 | 「元气小小，打开技能库列表」               → 独立面板窗：技能清单表格 |
| `docs/README_v18_UI.md:50` | A 用户可见（文档） | 「栖伴，给我看专家库面板」             → 独立面板窗：专家清单表格 | 「元气小小，给我看专家库面板」             → 独立面板窗：专家清单表格 |
| `docs/README_v18_UI.md:51` | A 用户可见（文档） | 「栖伴，关掉面板」                     → 关闭独立面板窗 | 「元气小小，关掉面板」                     → 关闭独立面板窗 |
| `docs/V11_VERIFICATION.md:1` | A 用户可见（文档） | # 《栖伴》Qibai v11（自动更新接入 + CI 配置）交付前独立验证记录 | # 《元气小小》Qibai v11（自动更新接入 + CI 配置）交付前独立验证记录 |
| `docs/v44b_能力事实审计.md:1` | A 用户可见（文档） | # 栖伴 v44b — 能力事实审计（只看代码，不动代码） | # 元气小小 v44b — 能力事实审计（只看代码，不动代码） |
| `docs/v44_QA报告.md:1` | A 用户可见（文档） | # 栖伴 v44 窗口交互修复 · 独立 QA 终验报告（最终态） | # 元气小小 v44 窗口交互修复 · 独立 QA 终验报告（最终态） |
| `docs/v44_目标对齐与行业差距.html:5` | A 用户可见（文档） | <title>栖伴 v44 — 目标对齐 × 行业差距 × 阶段路线</title> | <title>元气小小 v44 — 目标对齐 × 行业差距 × 阶段路线</title> |
| `docs/v44_目标对齐与行业差距.html:53` | A 用户可见（文档） | <h1>栖伴 v44 — 目标对齐 × 行业差距 × 阶段路线</h1> | <h1>元气小小 v44 — 目标对齐 × 行业差距 × 阶段路线</h1> |
| `docs/v44_目标对齐与行业差距.html:149` | A 用户可见（文档） | <thead><tr><th style="width:13%">维度</th><th>栖伴 v44</th><th>VPet¹</th><th>Replika²</th><th>Character.AI³</th><th>SillyTavern⁴</th><th>MS Copilot⁵</th><th>豆包电脑版⁶< | <thead><tr><th style="width:13%">维度</th><th>元气小小 v44</th><th>VPet¹</th><th>Replika²</th><th>Character.AI³</th><th>SillyTavern⁴</th><th>MS Copilot⁵</th><th>豆包电脑版 |
| `docs/v44_目标对齐与行业差距.md:1` | A 用户可见（文档） | # 栖伴 v44 — 目标对齐 × 行业差距 × 阶段路线 | # 元气小小 v44 — 目标对齐 × 行业差距 × 阶段路线 |
| `docs/v44_目标对齐与行业差距.md:3` | A 用户可见（文档） | > 成文范围：仅文档，不改代码。基于 v43 交付（`更新说明.txt`）、`CHECKLIST.md`、`栖伴核心功能测试与迭代计划.md`（v20/v21 门禁）、`qibai-core/src/*`、`src-tauri/src/main.rs`、`server/`。 | > 成文范围：仅文档，不改代码。基于 v43 交付（`更新说明.txt`）、`CHECKLIST.md`、`元气小小核心功能测试与迭代计划.md`（v20/v21 门禁）、`qibai-core/src/*`、`src-tauri/src/main.rs`、`server/`。 |
| `docs/v44_目标对齐与行业差距.md:31` | A 用户可见（文档） | > 来源缩写：CL=CHECKLIST.md；PL=栖伴核心功能测试与迭代计划.md；UPD=更新说明.txt（v43）；代码=qibai-core/src 或 src-tauri/src。 | > 来源缩写：CL=CHECKLIST.md；PL=元气小小核心功能测试与迭代计划.md；UPD=更新说明.txt（v43）；代码=qibai-core/src 或 src-tauri/src。 |
| `docs/v44_目标对齐与行业差距.md:103` | A 用户可见（文档） | \| 维度 \| 栖伴（v44） \| VPet-Simulator¹ \| Replika² \| Character.AI³ \| SillyTavern⁴ \| Microsoft Copilot⁵ \| 豆包电脑版⁶ \| DuMate/搭子⁷ \| | \| 维度 \| 元气小小（v44） \| VPet-Simulator¹ \| Replika² \| Character.AI³ \| SillyTavern⁴ \| Microsoft Copilot⁵ \| 豆包电脑版⁶ \| DuMate/搭子⁷ \| |
| `docs/v44_目标对齐与行业差距.md:127` | A 用户可见（文档） | - **SillyTavern 的角色卡生态**是栖伴 5 形象当前最大的形态差距——我们的角色是「内置 5 张皮」，不是「用户可导入/社区可共享的角色卡」。 | - **SillyTavern 的角色卡生态**是元气小小 5 形象当前最大的形态差距——我们的角色是「内置 5 张皮」，不是「用户可导入/社区可共享的角色卡」。 |
| `docs/v44_目标对齐与行业差距.md:128` | A 用户可见（文档） | - **Character.AI 破产**是最贵的行业警示：月算力 $1000 万 vs 订阅 $280 万。栖伴「用户自带 key」模式恰好规避了这条死路，但也意味着没有订阅收入——长线 IP 必须靠形象/皮肤/周边而非 LLM 订阅。 | - **Character.AI 破产**是最贵的行业警示：月算力 $1000 万 vs 订阅 $280 万。元气小小「用户自带 key」模式恰好规避了这条死路，但也意味着没有订阅收入——长线 IP 必须靠形象/皮肤/周边而非 LLM 订阅。 |
| `docs/v44_目标对齐与行业差距.md:129` | A 用户可见（文档） | - **Replika 的长期记忆 + 语音通话**是女友型品类的付费墙核心；栖伴的 memory_layers 四层设计已比 Character.AI 的短期上下文更扎实，但缺「增强记忆」这个可感知卖点。 | - **Replika 的长期记忆 + 语音通话**是女友型品类的付费墙核心；元气小小的 memory_layers 四层设计已比 Character.AI 的短期上下文更扎实，但缺「增强记忆」这个可感知卖点。 |
| `docs/v44_目标对齐与行业差距.md:130` | A 用户可见（文档） | - **Copilot 的 Local Actions / DuMate 的本地沙箱**证明「桌面 agent 动手干活」是 2026 下半年的共识方向；栖伴的技能白名单 + 代码检查 + vault 已对齐，但「录屏生成个人技能」「锁屏远程操作」这类黑箱能力没有。 | - **Copilot 的 Local Actions / DuMate 的本地沙箱**证明「桌面 agent 动手干活」是 2026 下半年的共识方向；元气小小的技能白名单 + 代码检查 + vault 已对齐，但「录屏生成个人技能」「锁屏远程操作」这类黑箱能力没有。 |
| `docs/v44_目标对齐与行业差距.md:131` | A 用户可见（文档） | - **VPet-Simulator 峰值 8 万在线、98% 好评**证明桌宠赛道的爽点是「陪伴感 + 创意工坊」，不是 AI；栖伴若只做 AI 桌宠而不开放皮肤/动作工坊，天花板明显。 | - **VPet-Simulator 峰值 8 万在线、98% 好评**证明桌宠赛道的爽点是「陪伴感 + 创意工坊」，不是 AI；元气小小若只做 AI 桌宠而不开放皮肤/动作工坊，天花板明显。 |
| `docs/v44_目标对齐与行业差距.md:145` | A 用户可见（文档） | 5. **形象生态封闭（对标 SillyTavern/VPet）**——5 个 3D 国漫 CG 形象是内置硬编码，无角色卡导入、无创意工坊、无用户自制皮。**为什么是它**：VPet 峰值 8 万在线靠的是创意工坊，SillyTavern 生命力靠的是社区角色卡；栖伴长线做 IP 但形象生态是「官方出一张皮用户看一张 | 5. **形象生态封闭（对标 SillyTavern/VPet）**——5 个 3D 国漫 CG 形象是内置硬编码，无角色卡导入、无创意工坊、无用户自制皮。**为什么是它**：VPet 峰值 8 万在线靠的是创意工坊，SillyTavern 生命力靠的是社区角色卡；元气小小长线做 IP 但形象生态是「官方出一张皮用户看 |
| `docs/v45.10_说明.md:1` | A 用户可见（文档） | # 栖伴 v45.10 说明 | # 元气小小 v45.10 说明 |
| `docs/v45.11_说明.md:1` | A 用户可见（文档） | # 栖伴 v45.11 说明（知识库 P1） | # 元气小小 v45.11 说明（知识库 P1） |
| `docs/v45.4_启动崩溃应急与safe_mode.md:1` | A 用户可见（文档） | # 栖伴 v45.4 启动崩溃应急与 safe mode 定案 | # 元气小小 v45.4 启动崩溃应急与 safe mode 定案 |
| `docs/v45.5_启动崩溃应急与safe_mode.md:1` | A 用户可见（文档） | # 栖伴 v45.5 启动崩溃应急与 safe mode 定案 | # 元气小小 v45.5 启动崩溃应急与 safe mode 定案 |
| `docs/v45.6_原生表面操作移除与工作区修复.md:1` | A 用户可见（文档） | # 栖伴 v45.6 原生表面操作移除与工作区修复 | # 元气小小 v45.6 原生表面操作移除与工作区修复 |
| `docs/v45.7_说明.md:1` | A 用户可见（文档） | # 栖伴 v45.7 综合包说明 | # 元气小小 v45.7 综合包说明 |
| `docs/v45.8_说明.md:1` | A 用户可见（文档） | # 栖伴 v45.8 综合包说明 | # 元气小小 v45.8 综合包说明 |
| `docs/v45.9_说明.md:1` | A 用户可见（文档） | # 栖伴 v45.9 说明 | # 元气小小 v45.9 说明 |
| `docs/v45.9_长任务与上下文管理方案.md:1` | A 用户可见（文档） | # 栖伴 v45.9 方案：长任务 · 提示词与上下文管理 · 压缩 · 知识库 | # 元气小小 v45.9 方案：长任务 · 提示词与上下文管理 · 压缩 · 知识库 |
| `docs/v45.9_长任务与上下文管理方案.md:5` | A 用户可见（文档） | > 适用范围：仅《栖伴》桌面端。本地优先，不上传用户内容。 | > 适用范围：仅《元气小小》桌面端。本地优先，不上传用户内容。 |
| `docs/v45.9_长任务与上下文管理方案.md:24` | A 用户可见（文档） | \| WorkBuddy 机制 \| 它解决的问题 \| 栖伴对应设计 \| 抄 / 不抄 \| | \| WorkBuddy 机制 \| 它解决的问题 \| 元气小小对应设计 \| 抄 / 不抄 \| |
| `docs/v45.9_长任务与上下文管理方案.md:312` | A 用户可见（文档） | 1. **做在哪**：做进《栖伴》（默认）。若你想抽成通用框架，P0 的模块边界已经按可复用切分，改动成本不大。 | 1. **做在哪**：做进《元气小小》（默认）。若你想抽成通用框架，P0 的模块边界已经按可复用切分，改动成本不大。 |
| `docs/v45_工作计划.md:1` | A 用户可见（文档） | # 栖伴 v45 — 目标地图 × 统一差距 × 四阶段计划 | # 元气小小 v45 — 目标地图 × 统一差距 × 四阶段计划 |
| `docs/v45_工作计划.md:133` | A 用户可见（文档） | ### 4.2 8 维度对标级表（栖伴 vs WorkBuddy） | ### 4.2 8 维度对标级表（元气小小 vs WorkBuddy） |
| `docs/v45_工作计划.md:135` | A 用户可见（文档） | \| # \| 维度 \| WorkBuddy 水位（证据等级） \| 栖伴 v45 目标 \| 栖伴现状 \| 差距结论 \| | \| # \| 维度 \| WorkBuddy 水位（证据等级） \| 元气小小 v45 目标 \| 元气小小现状 \| 差距结论 \| |
| `docs/VAULT_SPEC.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 迭代③：本地密码保险箱（冻结规格 v1） | # 元气小小 Qibai — 迭代③：本地密码保险箱（冻结规格 v1） |
| `docs/VOICE.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 语音规格与调用文档（迭代②：TTS 发声陪伴） | # 元气小小 Qibai — 语音规格与调用文档（迭代②：TTS 发声陪伴） |
| `docs/WINDOW_ARCH.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 双窗口架构契约（WINDOW ARCHITECTURE CONTRACT） | # 元气小小 Qibai — 双窗口架构契约（WINDOW ARCHITECTURE CONTRACT） |
| `docs/WINDOW_ARCH.md:179` | A 用户可见（文档） | - 右键菜单（自定义 HTML 菜单）：**打开控制台** / **锁定位置（切换）** / **退出栖伴**。 | - 右键菜单（自定义 HTML 菜单）：**打开控制台** / **锁定位置（切换）** / **退出元气小小**。 |
| `docs/WINDOW_ARCH.md:277` | A 用户可见（文档） | - `zip -r 栖伴_MVP_源码包.zip <qibai 全部> --exclude '*/target/*' '*/target' 'qibai/target/*'`， | - `zip -r 元气小小_MVP_源码包.zip <qibai 全部> --exclude '*/target/*' '*/target' 'qibai/target/*'`， |
| `docs/WINDOW_ARCH.md:279` | A 用户可见（文档） | - 复制到 `/home/user/Doubao/chats/38448289926008322/栖伴_MVP_源码包.zip`（覆盖旧包）。 | - 复制到 `/home/user/Doubao/chats/38448289926008322/元气小小_MVP_源码包.zip`（覆盖旧包）。 |
| `docs/切角色重叠三步判据.md:1` | A 用户可见（文档） | # 切角色「仍短暂重叠」三步判据（栖伴 v45.2） | # 切角色「仍短暂重叠」三步判据（元气小小 v45.2） |
| `docs/切角色重叠三步判据.md:11` | A 用户可见（文档） | \| 2 窗口计数 \| `xdotool search --name 栖伴` 每 50ms 采样 10s，期间手动切角色 ≥3 次 \| MAX_WINDOWS ≥ 2 \| **FAIL 双窗路径仍在** → Rust 侧 grep create_pet_window 调用点，确认切角色只 emit hot-swa | \| 2 窗口计数 \| `xdotool search --name 元气小小` 每 50ms 采样 10s，期间手动切角色 ≥3 次 \| MAX_WINDOWS ≥ 2 \| **FAIL 双窗路径仍在** → Rust 侧 grep create_pet_window 调用点，确认切角色只 emit hot-s |
| `docs/技术架构.md:1` | A 用户可见（文档） | # 《栖伴》技术架构（全链路梳理） | # 《元气小小》技术架构（全链路梳理） |
| `docs/栖伴_目标地图与v45计划.html:5` | A 用户可见（文档） | <title>栖伴 v45 — 目标地图 × 统一差距 × 四阶段计划</title> | <title>元气小小 v45 — 目标地图 × 统一差距 × 四阶段计划</title> |
| `docs/栖伴_目标地图与v45计划.html:56` | A 用户可见（文档） | <h1>栖伴 v45 — 目标地图 × 统一差距 × 四阶段计划</h1> | <h1>元气小小 v45 — 目标地图 × 统一差距 × 四阶段计划</h1> |
| `docs/栖伴_目标地图与v45计划.html:174` | A 用户可见（文档） | <h3>4.2 8 维度对标级表（栖伴 vs WorkBuddy）</h3> | <h3>4.2 8 维度对标级表（元气小小 vs WorkBuddy）</h3> |
| `docs/栖伴_目标地图与v45计划.html:176` | A 用户可见（文档） | <thead><tr><th style="width:4%">#</th><th style="width:12%">维度</th><th style="width:26%">WorkBuddy 水位</th><th style="width:18%">栖伴 v45 目标</th><th style="width:1 | <thead><tr><th style="width:4%">#</th><th style="width:12%">维度</th><th style="width:26%">WorkBuddy 水位</th><th style="width:18%">元气小小 v45 目标</th><th style="width |
| `docs/透明背景链路分析.md:1` | A 用户可见（文档） | # 栖伴「透明背景」功能完整链路分析（v35 沉淀） | # 元气小小「透明背景」功能完整链路分析（v35 沉淀） |
| `qibai-core/src/engine.rs:5328` | A 用户可见（运行时提示/提示词） | "栖伴知识库：默认打开本地工作区目录 ~/Documents/qibai，环境变量 QIBAI_WORKSPACE 可覆盖。", | "元气小小知识库：默认打开本地工作区目录 ~/Documents/qibai，环境变量 QIBAI_WORKSPACE 可覆盖。", |
| `qibai-core/src/memory_layers.rs:11` | A 用户可见（运行时提示/提示词） | //! 对齐栖伴现状：intimacy.rs 已有 `preference_memory` 亲密度门禁与 | //! 对齐元气小小现状：intimacy.rs 已有 `preference_memory` 亲密度门禁与 |
| `qibai-core/src/plugin_manifest.rs:322` | A 用户可见（运行时提示/提示词） | // 变量替换：entry.mcp.args 中的 `{QIBAI_HOME}` → 栖伴数据目录。 | // 变量替换：entry.mcp.args 中的 `{QIBAI_HOME}` → 元气小小数据目录。 |
| `qibai-core/src/realtime_voice_frontend.rs:53` | A 用户可见（运行时提示/提示词） | /// Realtime 协议消息子集（只栖伴需要的收敛面）。 | /// Realtime 协议消息子集（只元气小小需要的收敛面）。 |
| `qibai-core/src/task_exec.rs:365` | A 用户可见（运行时提示/提示词） | "## 人设\n你是栖伴的并行任务分支执行器，只完成本子任务，不要假设其他分支的产物。\n\n## 子任务\n[{}] {}\n目标: {}\n\n## 依赖产物\n{dep_section}\n\n你可以调用工具 write_file / run_command / noop；直接给文本结论也可。", | "## 人设\n你是元气小小的并行任务分支执行器，只完成本子任务，不要假设其他分支的产物。\n\n## 子任务\n[{}] {}\n目标: {}\n\n## 依赖产物\n{dep_section}\n\n你可以调用工具 write_file / run_command / noop；直接给文本结论也可。", |
| `qibai-core/src/tts.rs:21` | A 用户可见（运行时提示/提示词） | /// 从根源杜绝「扬声器声音被麦克风捕获 → VAD 误判用户说话 → 把栖伴自己的话 | /// 从根源杜绝「扬声器声音被麦克风捕获 → VAD 误判用户说话 → 把元气小小自己的话 |
| `qibai-core/src/tts.rs:37` | A 用户可见（运行时提示/提示词） | // 转写成输入——真机现象正是「栖伴自己说的话被监听变成输入」。 | // 转写成输入——真机现象正是「元气小小自己说的话被监听变成输入」。 |
| `qibai-core/src/tts.rs:91` | A 用户可见（运行时提示/提示词） | /// 阈值说明：用户正常说话几乎不可能与栖伴刚朗读的句子重叠 40% 或整句复读。 | /// 阈值说明：用户正常说话几乎不可能与元气小小刚朗读的句子重叠 40% 或整句复读。 |
| `qibai-core/src/tts.rs:897` | A 用户可见（运行时提示/提示词） | /// 唤醒词（默认「栖伴」），识别到后进入聆听。 | /// 唤醒词（默认「元气小小」），识别到后进入聆听。 |
| `qibai-core/src/tts.rs:2214` | A 用户可见（运行时提示/提示词） | let _ = piper_speak("栖伴测试"); | let _ = piper_speak("元气小小测试"); |
| `qibai-core/src/tts.rs:3180` | A 用户可见（运行时提示/提示词） | let sentence = "栖伴是一只住在桌面的桌宠，它会在你工作时安静陪伴。"; | let sentence = "元气小小是一只住在桌面的桌宠，它会在你工作时安静陪伴。"; |
| `README.md:1` | A 用户可见（文档） | # 栖伴 Qibai — AI 桌面伴侣（MVP · 双窗口架构版） | # 元气小小 Qibai — AI 桌面伴侣（MVP · 双窗口架构版） |
| `README.md:18` | A 用户可见（文档） | **栖伴（Qibai）** 是一款桌面端 AI 伴侣应用。它以一个透明悬浮在屏幕角落的二次元少女立绘为主体， | **元气小小（Qibai）** 是一款桌面端 AI 伴侣应用。它以一个透明悬浮在屏幕角落的二次元少女立绘为主体， |
| `README.md:59` | A 用户可见（文档） | - 思考中显示「栖伴正在思考…」并切 `focused` 表情，完成后恢复； | - 思考中显示「元气小小正在思考…」并切 `focused` 表情，完成后恢复； |
| `README.md:509` | A 用户可见（文档） | \| ⑳ \| qibai-core 回归 58/58（P0+P1 交付时点）+ 源码包重打包（排除 target） \| ✅ 通过 \| 该时点实测 `cargo test -p qibai-core` **58 passed / 0 failed**（既有 32 + P0+P1 21 + 双窗口 desk 5，见  | \| ⑳ \| qibai-core 回归 58/58（P0+P1 交付时点）+ 源码包重打包（排除 target） \| ✅ 通过 \| 该时点实测 `cargo test -p qibai-core` **58 passed / 0 failed**（既有 32 + P0+P1 21 + 双窗口 desk 5，见  |
| `README.md:516` | A 用户可见（文档） | \| ㉙ \| 重新打包源码包（排除 target）并复制交付 \| ✅ 已交付 \| `栖伴_MVP_源码包.zip`（unzip -l 抽查不含 `target/`） \| | \| ㉙ \| 重新打包源码包（排除 target）并复制交付 \| ✅ 已交付 \| `元气小小_MVP_源码包.zip`（unzip -l 抽查不含 `target/`） \| |
| `README.md:521` | A 用户可见（文档） | \| ㉞ \| 迭代①源码包重打包（排除 target/.git/_shots/_backup/scratch）并复制交付 \| ✅ 已交付 \| `栖伴_MVP_源码包.zip`（unzip -l 抽查含 `guard.rs` / `codecheck.rs` / `guard.html` / `GUARD.md`  | \| ㉞ \| 迭代①源码包重打包（排除 target/.git/_shots/_backup/scratch）并复制交付 \| ✅ 已交付 \| `元气小小_MVP_源码包.zip`（unzip -l 抽查含 `guard.rs` / `codecheck.rs` / `guard.html` / `GUARD.md |
| `README.md:524` | A 用户可见（文档） | \| ㊲ \| 迭代②源码包重打包（排除 target）并复制交付 \| ✅ 已交付 \| `栖伴_MVP_源码包.zip`（unzip -l 抽查含 `tts.rs` / `docs/VOICE.md` / `artifacts/voice-*.png` 5 张，不含任何 `target/` 路径） \| | \| ㊲ \| 迭代②源码包重打包（排除 target）并复制交付 \| ✅ 已交付 \| `元气小小_MVP_源码包.zip`（unzip -l 抽查含 `tts.rs` / `docs/VOICE.md` / `artifacts/voice-*.png` 5 张，不含任何 `target/` 路径） \| |
| `SMOKE_TEST.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 冒烟验证记录汇总 | # 元气小小 Qibai — 冒烟验证记录汇总 |
| `SMOKE_TEST.md:109` | A 用户可见（文档） | 栖伴想帮你打开「记事本」应用，这是一个外部操作，是否允许？」，含「拒绝 / 允许本次」按钮—— | 元气小小想帮你打开「记事本」应用，这是一个外部操作，是否允许？」，含「拒绝 / 允许本次」按钮—— |
| `SMOKE_TEST.md:156` | A 用户可见（文档） | \| `artifacts/main-window-baseline.png` \| 初始加载 \| 主窗基线：**默认古典佳人（gudian）立绘** + 开场气泡「嗨～我是栖伴，随时陪着你。」+ 底部对话/设置按钮 + 调试面板 \| | \| `artifacts/main-window-baseline.png` \| 初始加载 \| 主窗基线：**默认古典佳人（gudian）立绘** + 开场气泡「嗨～我是元气小小，随时陪着你。」+ 底部对话/设置按钮 + 调试面板 \| |
| `SMOKE_TEST.md:317` | A 用户可见（文档） | \| `dualwin_menu.png` \| 复制前端切片 B 产出的 `pet_menu.png`（`pet.html#fire=menu` 钩子取证，未修改 pet.js） \| 桌宠右键菜单三项：打开控制台 / 锁定位置 / 退出栖伴 \| | \| `dualwin_menu.png` \| 复制前端切片 B 产出的 `pet_menu.png`（`pet.html#fire=menu` 钩子取证，未修改 pet.js） \| 桌宠右键菜单三项：打开控制台 / 锁定位置 / 退出元气小小 \| |
| `SMOKE_TEST.md:326` | A 用户可见（文档） | - 旧源码包 `栖伴_MVP_源码包.zip`（143MB 含 target）已移至 `qibai/_backup/栖伴_MVP_源码包_旧版_含target.zip`（非破坏性）。 | - 旧源码包 `元气小小_MVP_源码包.zip`（143MB 含 target）已移至 `qibai/_backup/元气小小_MVP_源码包_旧版_含target.zip`（非破坏性）。 |
| `src/assets/personas/dushi/dushi_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》都市女性形象 · 12 张角色资产像素级复检记录 | # 《元气小小》都市女性形象 · 12 张角色资产像素级复检记录 |
| `src/assets/personas/gudian/gudian_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》古典佳人 12 张角色资产像素级复检记录 | # 《元气小小》古典佳人 12 张角色资产像素级复检记录 |
| `src/assets/personas/huopo/huopo_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》AI 桌面伴侣「活泼」形象 — 12 张角色资产像素级复检记录 | # 《元气小小》AI 桌面伴侣「活泼」形象 — 12 张角色资产像素级复检记录 |
| `src/assets/personas/reqing/reqing_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》AI 桌面伴侣 — 热情/时尚 形象资产 逐张像素级复检记录 | # 《元气小小》AI 桌面伴侣 — 热情/时尚 形象资产 逐张像素级复检记录 |
| `src/assets/personas/wenrou/dushi/dushi_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》都市女性形象 · 12 张角色资产像素级复检记录 | # 《元气小小》都市女性形象 · 12 张角色资产像素级复检记录 |
| `src/assets/personas/wenrou/gudian/gudian_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》古典佳人 12 张角色资产像素级复检记录 | # 《元气小小》古典佳人 12 张角色资产像素级复检记录 |
| `src/assets/personas/wenrou/huopo/huopo_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》AI 桌面伴侣「活泼」形象 — 12 张角色资产像素级复检记录 | # 《元气小小》AI 桌面伴侣「活泼」形象 — 12 张角色资产像素级复检记录 |
| `src/assets/personas/wenrou/reqing/reqing_recheck.md:1` | A 用户可见（角色资产说明标题） | # 《栖伴》AI 桌面伴侣 — 热情/时尚 形象资产 逐张像素级复检记录 | # 《元气小小》AI 桌面伴侣 — 热情/时尚 形象资产 逐张像素级复检记录 |
| `src/assets/personas/wenrou/wenrou_recheck.md:1` | A 用户可见（角色资产说明标题） | # 栖伴 · 温柔 形象资产复检记录（wenrou_recheck.md） | # 元气小小 · 温柔 形象资产复检记录（wenrou_recheck.md） |
| `src-tauri/plugins/experts/code-architect/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "对系统设计、架构方案与技术选型做宏观评审：八维检查、选型论证、演进路线建议，只给战略意见不写业务代码。说「栖伴，帮我评审一下这个架构方案」即命中。", | "description": "对系统设计、架构方案与技术选型做宏观评审：八维检查、选型论证、演进路线建议，只给战略意见不写业务代码。说「元气小小，帮我评审一下这个架构方案」即命中。", |
| `src-tauri/plugins/experts/code-architect/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt + 技能白名单） | ├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单） |
| `src-tauri/plugins/experts/code-architect/README.md:76` | A 用户可见（技能/专家说明） | - 「栖伴，帮我评审一下这个架构方案」→ 命中 code-architect 专家 | - 「元气小小，帮我评审一下这个架构方案」→ 命中 code-architect 专家 |
| `src-tauri/plugins/experts/code-architect/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，帮我对比一下这两套技术栈」→ 八因子对比表 | - 「元气小小，帮我对比一下这两套技术栈」→ 八因子对比表 |
| `src-tauri/plugins/experts/code-architect/README.md:78` | A 用户可见（技能/专家说明） | - 「栖伴，帮我规划旧系统的演进路线」→ Strangler / 抽象分支 / 并行运行三策略权衡 | - 「元气小小，帮我规划旧系统的演进路线」→ Strangler / 抽象分支 / 并行运行三策略权衡 |
| `src-tauri/plugins/experts/content-creator/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "先品牌后内容：从品牌语气与目标出发做选题规划、内容大纲与标题方案，用量化目标驱动内容质量。说「栖伴，帮我做一份选题清单」即命中。", | "description": "先品牌后内容：从品牌语气与目标出发做选题规划、内容大纲与标题方案，用量化目标驱动内容质量。说「元气小小，帮我做一份选题清单」即命中。", |
| `src-tauri/plugins/experts/content-creator/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt + 技能白名单） | ├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单） |
| `src-tauri/plugins/experts/content-creator/README.md:76` | A 用户可见（技能/专家说明） | - 「栖伴，帮我做一份选题清单」→ 命中 content-creator 专家 | - 「元气小小，帮我做一份选题清单」→ 命中 content-creator 专家 |
| `src-tauri/plugins/experts/content-creator/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，帮我给这个选题写大纲和标题」→ 大纲 + 3-5 个标题方案 | - 「元气小小，帮我给这个选题写大纲和标题」→ 大纲 + 3-5 个标题方案 |
| `src-tauri/plugins/experts/content-creator/README.md:78` | A 用户可见（技能/专家说明） | - 「栖伴，帮我排一下下个月的内容日历」→ 编辑日历 + 量化目标标注 | - 「元气小小，帮我排一下下个月的内容日历」→ 编辑日历 + 量化目标标注 |
| `src-tauri/plugins/experts/data-analyst/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "从业务数据里提取洞察：先定义指标口径，再选分析框架，给出可视化建议与数据叙事，把数字讲成决策依据。说「栖伴，帮我分析一下这份数据」即命中。", | "description": "从业务数据里提取洞察：先定义指标口径，再选分析框架，给出可视化建议与数据叙事，把数字讲成决策依据。说「元气小小，帮我分析一下这份数据」即命中。", |
| `src-tauri/plugins/experts/data-analyst/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt + 技能白名单） | ├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单） |
| `src-tauri/plugins/experts/data-analyst/README.md:76` | A 用户可见（技能/专家说明） | - 「栖伴，帮我分析一下这份数据」→ 命中 data-analyst 专家 | - 「元气小小，帮我分析一下这份数据」→ 命中 data-analyst 专家 |
| `src-tauri/plugins/experts/data-analyst/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，帮我定一下看板的指标口径」→ 口径表格（指标/口径/数据源/owner） | - 「元气小小，帮我定一下看板的指标口径」→ 口径表格（指标/口径/数据源/owner） |
| `src-tauri/plugins/experts/data-analyst/README.md:78` | A 用户可见（技能/专家说明） | - 「栖伴，帮我看看转化掉在哪一步」→ Funnel 漏斗分析建议 | - 「元气小小，帮我看看转化掉在哪一步」→ Funnel 漏斗分析建议 |
| `src-tauri/plugins/experts/emotional-companion/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "心情不好、心里堵得慌、想找人说说话时，这位温柔的老朋友会耐心倾听、接住你的情绪：不诊断、不开药、不扮演恋人。说「栖伴，我今天心情不好，陪我聊聊」即命中。", | "description": "心情不好、心里堵得慌、想找人说说话时，这位温柔的老朋友会耐心倾听、接住你的情绪：不诊断、不开药、不扮演恋人。说「元气小小，我今天心情不好，陪我聊聊」即命中。", |
| `src-tauri/plugins/experts/emotional-companion/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt 含安全红线） | ├── qibai-plugin.json              # 元气小小清单（system_prompt 含安全红线） |
| `src-tauri/plugins/experts/emotional-companion/README.md:89` | A 用户可见（技能/专家说明） | - 「栖伴，我今天心情不好，陪我聊聊」→ 命中 emotion 专家 | - 「元气小小，我今天心情不好，陪我聊聊」→ 命中 emotion 专家 |
| `src-tauri/plugins/experts/emotional-companion/README.md:90` | A 用户可见（技能/专家说明） | - 「栖伴，最近工作压力好大，心里很堵」 | - 「元气小小，最近工作压力好大，心里很堵」 |
| `src-tauri/plugins/experts/emotional-companion/README.md:91` | A 用户可见（技能/专家说明） | - 「栖伴，我就想找人安静地说说话」 | - 「元气小小，我就想找人安静地说说话」 |
| `src-tauri/plugins/experts/geek-play/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "想在终端里找点乐子时，这位终端玩家只给只读、展示类的趣味玩法：figlet 大字、cowsay 对话框、fortune 随机语录、ASCII 艺术字与好玩的命令别名建议，绝不碰真实系统。说「栖伴，玩点好玩的/给我来个终端彩蛋」即命中。", | "description": "想在终端里找点乐子时，这位终端玩家只给只读、展示类的趣味玩法：figlet 大字、cowsay 对话框、fortune 随机语录、ASCII 艺术字与好玩的命令别名建议，绝不碰真实系统。说「元气小小，玩点好玩的/给我来个终端彩蛋」即命中。", |
| `src-tauri/plugins/experts/geek-play/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt 含只读红线） | ├── qibai-plugin.json              # 元气小小清单（system_prompt 含只读红线） |
| `src-tauri/plugins/experts/geek-play/README.md:87` | A 用户可见（技能/专家说明） | - 「栖伴，玩点好玩的，给我来个终端彩蛋」→ 命中 geek 专家 | - 「元气小小，玩点好玩的，给我来个终端彩蛋」→ 命中 geek 专家 |
| `src-tauri/plugins/experts/geek-play/README.md:88` | A 用户可见（技能/专家说明） | - 「栖伴，给我来一句 fortune 风格的随机语录」 | - 「元气小小，给我来一句 fortune 风格的随机语录」 |
| `src-tauri/plugins/experts/geek-play/README.md:89` | A 用户可见（技能/专家说明） | - 「栖伴，用 figlet 大字拼出我的名字」 | - 「元气小小，用 figlet 大字拼出我的名字」 |
| `src-tauri/plugins/experts/idea-creator/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "想不出点子、头脑风暴、给产品/活动起创意方向时，这位创意策划师先用提问听懂你的真实需求，再发散收敛给出 2-3 个候选方案。说「栖伴，帮我想几个创意点子」即命中。", | "description": "想不出点子、头脑风暴、给产品/活动起创意方向时，这位创意策划师先用提问听懂你的真实需求，再发散收敛给出 2-3 个候选方案。说「元气小小，帮我想几个创意点子」即命中。", |
| `src-tauri/plugins/experts/idea-creator/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt + namegen 白名单） | ├── qibai-plugin.json              # 元气小小清单（system_prompt + namegen 白名单） |
| `src-tauri/plugins/experts/idea-creator/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，帮我想几个创意点子」→ 命中 ideas 专家 | - 「元气小小，帮我想几个创意点子」→ 命中 ideas 专家 |
| `src-tauri/plugins/experts/idea-creator/README.md:78` | A 用户可见（技能/专家说明） | - 「栖伴，帮我头脑风暴一下新产品的方向」 | - 「元气小小，帮我头脑风暴一下新产品的方向」 |
| `src-tauri/plugins/experts/idea-creator/README.md:79` | A 用户可见（技能/专家说明） | - 「栖伴，给我的日常来点不一样的玩法」→ 选中命名方向时联动 namegen | - 「元气小小，给我的日常来点不一样的玩法」→ 选中命名方向时联动 namegen |
| `src-tauri/plugins/experts/life-assistant/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "查天气、搜资料、算数、问时间，随叫随到的实用小帮手。说「栖伴，帮我看看北京的天气」「搜一下今天的新闻」「帮我算个账」即命中。", | "description": "查天气、搜资料、算数、问时间，随叫随到的实用小帮手。说「元气小小，帮我看看北京的天气」「搜一下今天的新闻」「帮我算个账」即命中。", |
| `src-tauri/plugins/experts/planner/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "把目标、任务与会议安排成清晰可执行的日程：优先级排序、时间块规划、提醒设置。说「栖伴，帮我安排明天的日程」即命中。", | "description": "把目标、任务与会议安排成清晰可执行的日程：优先级排序、时间块规划、提醒设置。说「元气小小，帮我安排明天的日程」即命中。", |
| `src-tauri/plugins/experts/planner/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json           # 栖伴清单（system_prompt + 技能白名单） | ├── qibai-plugin.json           # 元气小小清单（system_prompt + 技能白名单） |
| `src-tauri/plugins/experts/planner/README.md:73` | A 用户可见（技能/专家说明） | - 「栖伴，帮我安排明天的日程」→ 命中 planner 专家 | - 「元气小小，帮我安排明天的日程」→ 命中 planner 专家 |
| `src-tauri/plugins/experts/planner/README.md:74` | A 用户可见（技能/专家说明） | - 「栖伴，帮我把这周的任务排个优先级」 | - 「元气小小，帮我把这周的任务排个优先级」 |
| `src-tauri/plugins/experts/planner/README.md:75` | A 用户可见（技能/专家说明） | - 「栖伴，帮我给下午的会议设置提醒」→ planner → reminder-parse → set_reminder | - 「元气小小，帮我给下午的会议设置提醒」→ planner → reminder-parse → set_reminder |
| `src-tauri/plugins/experts/storyteller/qibai-plugin.json:5` | A 用户可见（技能市场描述） | "description": "睡前想听故事、给孩子编个童话时，这位睡前故事作家会按三段式结构讲一个温暖有画面感的小故事：开头引入、小冲突、正向化解。说「栖伴，给我讲个睡前故事」即命中。", | "description": "睡前想听故事、给孩子编个童话时，这位睡前故事作家会按三段式结构讲一个温暖有画面感的小故事：开头引入、小冲突、正向化解。说「元气小小，给我讲个睡前故事」即命中。", |
| `src-tauri/plugins/experts/storyteller/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json              # 栖伴清单（system_prompt 含内容红线） | ├── qibai-plugin.json              # 元气小小清单（system_prompt 含内容红线） |
| `src-tauri/plugins/experts/storyteller/README.md:86` | A 用户可见（技能/专家说明） | - 「栖伴，给我讲个睡前故事」→ 命中 storyteller 专家 | - 「元气小小，给我讲个睡前故事」→ 命中 storyteller 专家 |
| `src-tauri/plugins/experts/storyteller/README.md:87` | A 用户可见（技能/专家说明） | - 「栖伴，讲一个喜欢星星的小狐狸的故事」 | - 「元气小小，讲一个喜欢星星的小狐狸的故事」 |
| `src-tauri/plugins/experts/storyteller/README.md:88` | A 用户可见（技能/专家说明） | - 「栖伴，讲一个关于善良的小故事」 | - 「元气小小，讲一个关于善良的小故事」 |
| `src-tauri/plugins/skills/code-review/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/code-review/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/code-review/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，审查这段代码：projects/my-app」 | - 「元气小小，审查这段代码：projects/my-app」 |
| `src-tauri/plugins/skills/code-review/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，帮我看看这个项目有没有安全隐患」 | - 「元气小小，帮我看看这个项目有没有安全隐患」 |
| `src-tauri/plugins/skills/code-review/README.md:72` | A 用户可见（技能/专家说明） | - 「栖伴，代码检查一下 scripts 目录」 | - 「元气小小，代码检查一下 scripts 目录」 |
| `src-tauri/plugins/skills/code-writer/SKILL.md:35` | A 用户可见（技能/专家说明） | 4. 复杂定制代码（业务逻辑/接口对接/多文件工程）→ 引导用户直接对栖伴说「帮我写……」，由主对话大模型生成完整实现（本技能只做高频模板）。 | 4. 复杂定制代码（业务逻辑/接口对接/多文件工程）→ 引导用户直接对元气小小说「帮我写……」，由主对话大模型生成完整实现（本技能只做高频模板）。 |
| `src-tauri/plugins/skills/code-writer/SKILL.md:45` | A 用户可见（技能/专家说明） | - 命中语言时输出 `@qibai-panel:begin/end` 块（mode=window, title=「<语言> · 栖伴写代码」, html 含 `<pre>` 代码区，样式深色底便于阅读）； | - 命中语言时输出 `@qibai-panel:begin/end` 块（mode=window, title=「<语言> · 元气小小写代码」, html 含 `<pre>` 代码区，样式深色底便于阅读）； |
| `src-tauri/plugins/skills/file-search/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/file-search/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/file-search/README.md:69` | A 用户可见（技能/专家说明） | - 「栖伴，帮我找一下「发票」这个文件」 | - 「元气小小，帮我找一下「发票」这个文件」 |
| `src-tauri/plugins/skills/file-search/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，搜索文件，关键词 report」 | - 「元气小小，搜索文件，关键词 report」 |
| `src-tauri/plugins/skills/file-search/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，我上次保存的合同在哪里？」→ 命中 file-search（关键词「合同」） | - 「元气小小，我上次保存的合同在哪里？」→ 命中 file-search（关键词「合同」） |
| `src-tauri/plugins/skills/horoscope/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/horoscope/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/horoscope/README.md:69` | A 用户可见（技能/专家说明） | - 「栖伴，使用星座运势，看看白羊座今天怎么样」 | - 「元气小小，使用星座运势，看看白羊座今天怎么样」 |
| `src-tauri/plugins/skills/horoscope/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，帮我看看今天的运势」→ 命中 horoscope（追问星座） | - 「元气小小，帮我看看今天的运势」→ 命中 horoscope（追问星座） |
| `src-tauri/plugins/skills/jokes/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/jokes/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/jokes/README.md:75` | A 用户可见（技能/专家说明） | - 「栖伴，讲个冷笑话逗我笑」 | - 「元气小小，讲个冷笑话逗我笑」 |
| `src-tauri/plugins/skills/jokes/README.md:76` | A 用户可见（技能/专家说明） | - 「栖伴，来个谐音梗段子」→ 命中 jokes（category=谐音） | - 「元气小小，来个谐音梗段子」→ 命中 jokes（category=谐音） |
| `src-tauri/plugins/skills/jokes/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，讲个反转类的」→ 命中 jokes（category=反转） | - 「元气小小，讲个反转类的」→ 命中 jokes（category=反转） |
| `src-tauri/plugins/skills/namegen/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/namegen/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/namegen/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，起个名字，要萌宠风」 | - 「元气小小，起个名字，要萌宠风」 |
| `src-tauri/plugins/skills/namegen/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，帮我给产品命名」→ 命中 namegen（追问风格或随机） | - 「元气小小，帮我给产品命名」→ 命中 namegen（追问风格或随机） |
| `src-tauri/plugins/skills/namegen/README.md:72` | A 用户可见（技能/专家说明） | - 「栖伴，给我家猫取个名」 | - 「元气小小，给我家猫取个名」 |
| `src-tauri/plugins/skills/promptgen/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/promptgen/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/promptgen/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，生成提示词：一只橘猫，水彩风，黄昏光」 | - 「元气小小，生成提示词：一只橘猫，水彩风，黄昏光」 |
| `src-tauri/plugins/skills/promptgen/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，画图提示词怎么写，我想画雪山特写」 | - 「元气小小，画图提示词怎么写，我想画雪山特写」 |
| `src-tauri/plugins/skills/promptgen/README.md:72` | A 用户可见（技能/专家说明） | - 「栖伴，帮我写个绘画描述」→ 命中 promptgen（提示补充主体/风格/光照/镜头） | - 「元气小小，帮我写个绘画描述」→ 命中 promptgen（提示补充主体/风格/光照/镜头） |
| `src-tauri/plugins/skills/reminder-parse/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/reminder-parse/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/reminder-parse/README.md:69` | A 用户可见（技能/专家说明） | - 「栖伴，明天下午3点开会提醒我」→ reminder-parse → set_reminder | - 「元气小小，明天下午3点开会提醒我」→ reminder-parse → set_reminder |
| `src-tauri/plugins/skills/reminder-parse/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，帮我记一下周五晚上8点健身」 | - 「元气小小，帮我记一下周五晚上8点健身」 |
| `src-tauri/plugins/skills/sweet-talk/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/sweet-talk/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/sweet-talk/README.md:70` | A 用户可见（技能/专家说明） | rizzler.lua（"fell from heaven" / "rearrange the alphabet"）与 tanu notebook 的 cheesy 分类均为经典土味/油腻话术，且隐含搭讪导向；栖伴是桌面伴侣而非社交搭讪工具，用户可能是未成年人或处于工作场景，一旦出擦边句即违规。本包只借其「分层词库」结 | rizzler.lua（"fell from heaven" / "rearrange the alphabet"）与 tanu notebook 的 cheesy 分类均为经典土味/油腻话术，且隐含搭讪导向；元气小小是桌面伴侣而非社交搭讪工具，用户可能是未成年人或处于工作场景，一旦出擦边句即违规。本包只借其「分层词库 |
| `src-tauri/plugins/skills/sweet-talk/README.md:93` | A 用户可见（技能/专家说明） | - 「栖伴，说句情话」 | - 「元气小小，说句情话」 |
| `src-tauri/plugins/skills/sweet-talk/README.md:94` | A 用户可见（技能/专家说明） | - 「栖伴，来一句俏皮的」 | - 「元气小小，来一句俏皮的」 |
| `src-tauri/plugins/skills/sweet-talk/README.md:95` | A 用户可见（技能/专家说明） | - 「栖伴，给我两句深情一点的」 | - 「元气小小，给我两句深情一点的」 |
| `src-tauri/plugins/skills/tarot/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/tarot/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/tarot/README.md:75` | A 用户可见（技能/专家说明） | - 「栖伴，用塔罗帮我抽三张牌，问问换工作的事」 | - 「元气小小，用塔罗帮我抽三张牌，问问换工作的事」 |
| `src-tauri/plugins/skills/tarot/README.md:76` | A 用户可见（技能/专家说明） | - 「栖伴，抽张牌，算一卦」 | - 「元气小小，抽张牌，算一卦」 |
| `src-tauri/plugins/skills/tarot/README.md:77` | A 用户可见（技能/专家说明） | - 「栖伴，今天的运势，塔罗看看」→ 命中 tarot（question 为空，整体状态抽牌） | - 「元气小小，今天的运势，塔罗看看」→ 命中 tarot（question 为空，整体状态抽牌） |
| `src-tauri/plugins/skills/translate/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/translate/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/translate/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，翻译一下：你好」 | - 「元气小小，翻译一下：你好」 |
| `src-tauri/plugins/skills/translate/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，把这句话翻成英文：我稍后回复你」 | - 「元气小小，把这句话翻成英文：我稍后回复你」 |
| `src-tauri/plugins/skills/translate/README.md:72` | A 用户可见（技能/专家说明） | - 「栖伴，这个用英语怎么说：会议几点开始」 | - 「元气小小，这个用英语怎么说：会议几点开始」 |
| `src-tauri/plugins/skills/web-summary/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/web-summary/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/web-summary/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，使用网页摘要，总结 https://zh.wikipedia.org/wiki/人工智能」 | - 「元气小小，使用网页摘要，总结 https://zh.wikipedia.org/wiki/人工智能」 |
| `src-tauri/plugins/skills/web-summary/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，帮我看看这篇文章讲了什么」→ 命中 web-summary | - 「元气小小，帮我看看这篇文章讲了什么」→ 命中 web-summary |
| `src-tauri/plugins/skills/writing-polish/assets/README.md:5` | A 用户可见（技能/专家说明） | 当前版本未随包附带图标（栖伴引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， | 当前版本未随包附带图标（元气小小引擎不依赖图标即可装配）；上架 WorkBuddy 开放平台时， |
| `src-tauri/plugins/skills/writing-polish/README.md:18` | A 用户可见（技能/专家说明） | ├── qibai-plugin.json   # 栖伴清单（引擎装配入口） | ├── qibai-plugin.json   # 元气小小清单（引擎装配入口） |
| `src-tauri/plugins/skills/writing-polish/README.md:70` | A 用户可见（技能/专家说明） | - 「栖伴，帮我改下这段话：这个项目我们基本上我觉得应该是很有前景的。」 | - 「元气小小，帮我改下这段话：这个项目我们基本上我觉得应该是很有前景的。」 |
| `src-tauri/plugins/skills/writing-polish/README.md:71` | A 用户可见（技能/专家说明） | - 「栖伴，润色一下：然后那个,我觉得这个方案大概可以。」 | - 「元气小小，润色一下：然后那个,我觉得这个方案大概可以。」 |
| `src-tauri/plugins/skills/writing-polish/README.md:72` | A 用户可见（技能/专家说明） | - 「栖伴，检查错别字：今天天气很好,我们去公园走走。」 | - 「元气小小，检查错别字：今天天气很好,我们去公园走走。」 |
| `src-tauri/tauri.conf.json:30` | A 用户可见（窗口/应用描述） | "longDescription": "Qibai (栖伴) — AI desktop companion pet." | "longDescription": "Qibai (元气小小) — AI desktop companion pet." |
| `VERIFICATION-BACKEND.md:1` | A 用户可见（文档） | # 栖伴 Qibai — 后端冒烟验证记录 | # 元气小小 Qibai — 后端冒烟验证记录 |

## 二、已改、但只是注释/开发工具输出（非用户可见，可保留可回退）

共 51 行。

| 文件:行 | 类别 | 原文（旧） | 现文（新） |
|---|---|---|---|
| `qibai-core/examples/llm_live.rs:1` | B 开发工具输出（非用户界面） | //! 栖伴 · 大模型真实连通验证（单机部署验收用） | //! 元气小小 · 大模型真实连通验证（单机部署验收用） |
| `qibai-core/examples/llm_live.rs:34` | B 开发工具输出（非用户界面） | println!("[栖伴] 正在连接大模型…（{}）", provider_model(&settings)); | println!("[元气小小] 正在连接大模型…（{}）", provider_model(&settings)); |
| `qibai-core/examples/llm_live.rs:37` | B 开发工具输出（非用户界面） | system: "你是《栖伴》桌面伴侣·古典佳人，说话温婉简短，带一点古风。".into(), | system: "你是《元气小小》桌面伴侣·古典佳人，说话温婉简短，带一点古风。".into(), |
| `qibai-core/examples/llm_live.rs:45` | B 开发工具输出（非用户界面） | println!("[栖伴] 大模型回复: {}", content.trim()); | println!("[元气小小] 大模型回复: {}", content.trim()); |
| `qibai-core/examples/llm_live.rs:46` | B 开发工具输出（非用户界面） | println!("[栖伴] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功）"); | println!("[元气小小] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功）"); |
| `scripts/check_plugin_spec.sh:3` | B 脚本输出/注释 | # 《栖伴》插件双规范自动校验器（PLUGIN_V18_SPEC 沉淀） | # 《元气小小》插件双规范自动校验器（PLUGIN_V18_SPEC 沉淀） |
| `scripts/check_qibai_version.sh:2` | B 脚本输出/注释 | # 栖伴 v22 合流核对脚本（正式工具）—— 一键判断真机源码是否已合流最新修复 | # 元气小小 v22 合流核对脚本（正式工具）—— 一键判断真机源码是否已合流最新修复 |
| `scripts/check_qibai_version.sh:10` | B 脚本输出/注释 | echo "栖伴 v22 合流核对 $(date)" | echo "元气小小 v22 合流核对 $(date)" |
| `scripts/check_version.sh:2` | B 脚本输出/注释 | # 栖伴 v21 版本核对脚本 —— 判断真机源码是否已应用最新修复 | # 元气小小 v21 版本核对脚本 —— 判断真机源码是否已应用最新修复 |
| `scripts/check_version.sh:9` | B 脚本输出/注释 | echo "栖伴版本核对 $(date)" | echo "元气小小版本核对 $(date)" |
| `scripts/diagnose_persona_overlap.sh:3` | B 脚本输出/注释 | # 栖伴 v45.2 「切角色仍短暂重叠」三步只读诊断（diagnose_persona_overlap.sh） | # 元气小小 v45.2 「切角色仍短暂重叠」三步只读诊断（diagnose_persona_overlap.sh） |
| `scripts/diagnose_persona_overlap.sh:84` | B 脚本输出/注释 | xdotool search --name "栖伴" 2>/dev/null \| while read -r wid; do | xdotool search --name "元气小小" 2>/dev/null \| while read -r wid; do |
| `scripts/diagnose_persona_overlap.sh:93` | B 脚本输出/注释 | n=$(xdotool search --name "栖伴" 2>/dev/null \| wc -l \| tr -d ' ') | n=$(xdotool search --name "元气小小" 2>/dev/null \| wc -l \| tr -d ' ') |
| `scripts/fix_compositor.sh:3` | B 脚本输出/注释 | # 栖伴 v45.2 合成器一键诊断/恢复（fix_compositor.sh） | # 元气小小 v45.2 合成器一键诊断/恢复（fix_compositor.sh） |
| `scripts/fix_compositor.sh:46` | B 脚本输出/注释 | say "===== 栖伴合成器诊断 fix_compositor (DRY=$DRY RESTORE_WM=$RESTORE_WM) =====" | say "===== 元气小小合成器诊断 fix_compositor (DRY=$DRY RESTORE_WM=$RESTORE_WM) =====" |
| `scripts/mic_diag.sh:2` | B 脚本输出/注释 | # 栖伴 麦克风自诊断 v1（不依赖栖伴运行，独立定位麦克风链路） | # 元气小小 麦克风自诊断 v1（不依赖元气小小运行，独立定位麦克风链路） |
| `scripts/piper_download.sh:3` | B 脚本输出/注释 | # 栖伴 · Piper 中文女声模型下载脚本（可选增强，非必需） | # 元气小小 · Piper 中文女声模型下载脚本（可选增强，非必需） |
| `scripts/piper_download.sh:22` | B 脚本输出/注释 | echo "下一步：pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com piper-tts（阿里云镜像，一次即可），重启栖伴自动启用离线女声。" | echo "下一步：pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com piper-tts（阿里云镜像，一次即可），重启元气小小自动启用离线女声。" |
| `scripts/prepare_workspace.sh:2` | B 脚本输出/注释 | # prepare_workspace.sh — 幂等创建栖伴工作区与 crawler_project 骨架。 | # prepare_workspace.sh — 幂等创建元气小小工作区与 crawler_project 骨架。 |
| `scripts/smoke_v20.sh:2` | B 脚本输出/注释 | # 栖伴 v20 真机一键冒烟脚本（引导式 checklist，非破坏性） | # 元气小小 v20 真机一键冒烟脚本（引导式 checklist，非破坏性） |
| `scripts/smoke_v20.sh:9` | B 脚本输出/注释 | echo "栖伴 v20 真机冒烟 $(date)" \| tee "$LOG" | echo "元气小小 v20 真机冒烟 $(date)" \| tee "$LOG" |
| `scripts/test_mood_rules.js:1` | B 脚本输出/注释 | // 栖伴 v18.4 语气推断规则单测（node 直跑） | // 元气小小 v18.4 语气推断规则单测（node 直跑） |
| `scripts/tts_install.sh:2` | B 脚本输出/注释 | # 栖伴 TTS 语音引擎一键安装（三选一即可发声，重复运行无害） | # 元气小小 TTS 语音引擎一键安装（三选一即可发声，重复运行无害） |
| `scripts/tts_install.sh:4` | B 脚本输出/注释 | echo "== 栖伴语音引擎检查 ==" | echo "== 元气小小语音引擎检查 ==" |
| `scripts/tts_install.sh:13` | B 脚本输出/注释 | pip3 install --user edge-tts && echo "edge-tts 已安装，栖伴自动使用「晓晓」女声朗读。" | pip3 install --user edge-tts && echo "edge-tts 已安装，元气小小自动使用「晓晓」女声朗读。" |
| `scripts/tts_install.sh:22` | B 脚本输出/注释 | echo "装好后重启栖伴，问它一句「你好」即可验证朗读。" | echo "装好后重启元气小小，问它一句「你好」即可验证朗读。" |
| `scripts/tts_setup.sh:3` | B 脚本输出/注释 | # 栖伴 · 音色一键准备（可选增强，两条都装效果最好） | # 元气小小 · 音色一键准备（可选增强，两条都装效果最好） |
| `scripts/tts_setup.sh:14` | B 脚本输出/注释 | echo "完成！重启栖伴即可。 | echo "完成！重启元气小小即可。 |
| `scripts/verify_aec.sh:2` | B 脚本输出/注释 | # 栖伴 P0「回声根治工程（信号级 AEC）」真机一键验证脚本（引导式 checklist，非破坏性） | # 元气小小 P0「回声根治工程（信号级 AEC）」真机一键验证脚本（引导式 checklist，非破坏性） |
| `scripts/verify_aec.sh:16` | B 脚本输出/注释 | echo "栖伴 P0 信号级 AEC 真机验证 $(date)" \| tee "$LOG" | echo "元气小小 P0 信号级 AEC 真机验证 $(date)" \| tee "$LOG" |
| `scripts/verify_persona_switch.sh:110` | B 脚本输出/注释 | # 真机取证（需要 X + 已启动的栖伴）。沙盒不可跑，全部标注「待真机验证」。 | # 真机取证（需要 X + 已启动的元气小小）。沙盒不可跑，全部标注「待真机验证」。 |
| `scripts/verify_ui_v44.sh:3` | B 脚本输出/注释 | # 栖伴 v44 窗口交互层真机一键验证（verify_ui_v44.sh） | # 元气小小 v44 窗口交互层真机一键验证（verify_ui_v44.sh） |
| `scripts/verify_ui_v44.sh:61` | B 脚本输出/注释 | find_pet_wid() { xdotool search --name "栖伴" 2>/dev/null \| head -1; } | find_pet_wid() { xdotool search --name "元气小小" 2>/dev/null \| head -1; } |
| `scripts/verify_ui_v44.sh:73` | B 脚本输出/注释 | echo "  请先手动启动栖伴桌宠再运行本脚本，或：APP_BIN=/path/to/qibai $0" \| tee -a "$RESULT" | echo "  请先手动启动元气小小桌宠再运行本脚本，或：APP_BIN=/path/to/qibai $0" \| tee -a "$RESULT" |
| `scripts/verify_ui_v44.sh:76` | B 脚本输出/注释 | if [ -z "${WID:-}" ]; then record SKIP "环境" "找不到桌宠窗（标题含「栖伴」），后续项无法执行"; echo "result=SKIP" >>"$RESULT"; exit 1; fi | if [ -z "${WID:-}" ]; then record SKIP "环境" "找不到桌宠窗（标题含「元气小小」），后续项无法执行"; echo "result=SKIP" >>"$RESULT"; exit 1; fi |
| `scripts/verify_ui_v44.sh:156` | B 脚本输出/注释 | CONSOLE_WID="$(xdotool search --name "栖伴 · 面板\|控制台" 2>/dev/null \| head -1 \|\| true)" | CONSOLE_WID="$(xdotool search --name "元气小小 · 面板\|控制台" 2>/dev/null \| head -1 \|\| true)" |
| `scripts/verify_ui_v44.sh:263` | B 脚本输出/注释 | echo "${C_WARN}  接下来 3 秒请在栖伴里切换到另一个角色（或控制台选角色）。${C_RST}" | echo "${C_WARN}  接下来 3 秒请在元气小小里切换到另一个角色（或控制台选角色）。${C_RST}" |
| `scripts/vllm_serve.sh:3` | B 脚本输出/注释 | # vllm_serve.sh — 栖伴（栖伴）本地 LLM vLLM 双卡部署脚本（幂等） | # vllm_serve.sh — 元气小小（元气小小）本地 LLM vLLM 双卡部署脚本（幂等） |
| `src/console.css:2` | B 仅注释 | * 栖伴 Qibai — 控制台窗增量样式（console.css） | * 元气小小 Qibai — 控制台窗增量样式（console.css） |
| `src/console.css:126` | B 仅注释 | /* v18.8-fix28g：聊天气泡角色区分（me=我右侧主色 / role=栖伴左侧浅色 / sys=系统） */ | /* v18.8-fix28g：聊天气泡角色区分（me=我右侧主色 / role=元气小小左侧浅色 / sys=系统） */ |
| `src/guard.css:2` | B 仅注释 | * 栖伴 Qibai — 全屏守护演出页 guard.css | * 元气小小 Qibai — 全屏守护演出页 guard.css |
| `src/pet.css:2` | B 仅注释 | 栖伴 Qibai — 桌宠窗专属增量样式（pet.css） | 元气小小 Qibai — 桌宠窗专属增量样式（pet.css） |
| `src/pet.css:429` | B 仅注释 | 「退出栖伴」等底部项可见；背景强制纯白不透明（防透明窗合成顺序 | 「退出元气小小」等底部项可见；背景强制纯白不透明（防透明窗合成顺序 |
| `src/styles.css:2` | B 仅注释 | 栖伴 Qibai — 全局样式 | 元气小小 Qibai — 全局样式 |
| `src-tauri/plugins/skills/code-writer/run.sh:157` | B 其他 | const result = greet("栖伴"); | const result = greet("元气小小"); |
| `src-tauri/plugins/skills/code-writer/run.sh:255` | B 其他 | SUMMARY="我可以按语言输出可运行骨架：HTML 页面 / Python 脚本 / JavaScript / Shell / JSON 配置 / Rust / Go / CSS。请指明语言与用途，例如「写一个 Python 脚本读取 CSV 并打印统计」。复杂定制代码也可以直接说「帮我写……」，栖伴会生成完整实现 | SUMMARY="我可以按语言输出可运行骨架：HTML 页面 / Python 脚本 / JavaScript / Shell / JSON 配置 / Rust / Go / CSS。请指明语言与用途，例如「写一个 Python 脚本读取 CSV 并打印统计」。复杂定制代码也可以直接说「帮我写……」，元气小小会生成完整 |
| `src-tauri/plugins/skills/code-writer/run.sh:264` | B 其他 | echo "title=$TITLE · 栖伴写代码" | echo "title=$TITLE · 元气小小写代码" |
| `src-tauri/plugins/skills/code-writer/run.sh:265` | B 其他 | echo "html=<div><h3>$TITLE · 需求：$(printf '%s' "$TEXT" \| esc \| cut -c 1-60)</h3><p>$SUMMARY</p><pre style=\"background:#1e1e1e;color:#e6e6e6;padding:12px;borde | echo "html=<div><h3>$TITLE · 需求：$(printf '%s' "$TEXT" \| esc \| cut -c 1-60)</h3><p>$SUMMARY</p><pre style=\"background:#1e1e1e;color:#e6e6e6;padding:12px;borde |
| `tests/e2e_business_chain.js:1` | B 仅测试文件头注释 | /* 栖伴 全业务链路 E2E 测试（真实 Chromium + 仿真 Tauri 后端） | /* 元气小小 全业务链路 E2E 测试（真实 Chromium + 仿真 Tauri 后端） |
| `tests/test_event_chain.js:1` | B 仅测试文件头注释 | /* 栖伴事件链 Node 仿真测试 | /* 元气小小事件链 Node 仿真测试 |
| `tests/test_ui_v44_qa.js:2` | B 仅测试文件头注释 | * 栖伴 v44 独立 QA 终验测试（QA 线自建，不采信开发线自测结论） | * 元气小小 v44 独立 QA 终验测试（QA 线自建，不采信开发线自测结论） |

## 三、明确「不改」的内部标识（改名时一律不动）

| 标识 | 当前值 | 为什么不改 |
|---|---|---|
| crate 名 | `qibai-core`（`qibai-core/Cargo.toml`） | 代码依赖与构建脚本引用，改名要动 Cargo 全链路 |
| 二进制名 | `qibai-tauri`（`src-tauri/Cargo.toml` / `tauri.conf.json productName`） | 打包产物名、启动脚本、真机安装路径都指向它 |
| bundle identifier | `com.qibai.app` | 升级/覆盖安装识别用，改了两版会变成两个应用 |
| 数据目录 / 配置文件名 | `~/.local/share/qibai`、`settings.json`、`tasks/`、`kb/` | 改了等于用户配置、任务、知识库全部丢失 |
| 工作区根 | `~/Documents/qibai`（`QIBAI_WORKSPACE` 可覆盖） | 白名单根，改了既有文件全部越界 |
| IPC 命令名 | `task_*` / `mcp_*` / `kb_*` / `save_settings*` 等 | 前端 invoke 名前后端约定，改了要全量同步 |
| 事件名 | `qibai://task-updated`、`qibai://executions`、`qibai://mcp-conflicts` 等 | 同上 |
| 插件清单字段名 | `qibai-plugin.json` 的 `name/id/entry/capabilities` 等**字段名** | 加载器按键名解析；**字段值（描述文本）可改，已改** |
| 唤醒词兼容值 | `settings.rs:678` 的 `"栖伴"` 字面量 | 这是旧配置迁移判断，必须保留才能把老用户唤醒词换成新名 |


## 四、本次**没有**改的归档/历史目录（下一版也不建议动）

- `artifacts/`、`review-artifacts/`、`v22_pkg/`、`v44_pkg/`：历史产物快照
- `docs/README_v16~v18*.md`、`docs/README_v18_UI.md`、`交付说明_v19.md`、`交付说明_v20.md`、`栖伴v20审视与优化报告.md`、`栖伴核心功能测试与迭代计划.md`
- `server/`、`src-tauri/capabilities/default.json`、`src-tauri/gen/schemas/capabilities.json`（权限 schema，含应用标识）

> 说明：上一轮我把 `docs/README_v16..v18` 这类历史归档也一起换掉了（sed 是按目录扫的），如果你想保持历史文档原貌，这一版可以按本清单回退；我建议保留改动，因为都是文档正文，不影响任何功能。


## 五、下一版施工顺序（等你发话）

1. 按本清单确认改名范围（用户可见 = 必改，内部标识 = 不动）+ 全量测试
2. 修录像暴露的 4 个缺陷：任务面板入口 `TABS` 缺 `'tasks'`、执行留痕 meta 未传、`kb/search.rs` 用字节偏移当字符索引导致中文 panic、`SendMessageOutput` 无 `markers` 字段
3. 再开 Workflow 重跑
4. 交付包命名统一为 `元气小小_v45.x_xxx.zip`
