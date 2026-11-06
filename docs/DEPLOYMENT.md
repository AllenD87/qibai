# 《元气小小》部署方案 V1.0

> 适用版本：v11（自动更新 + CI/CD）。工程根：`qibai/`，Tauri 2 桌面壳 + qibai-core（Rust）+ 静态前端（src/）。
> 三个部署对象：①客户端（三平台桌面程序）②可选云服务端（联网社交）③大模型 API 接入（豆包/火山方舟，用户自配 key）。
> 本文档随交付包发布；真机验证清单见 §8。

---

## 1. 部署总览

```
┌─────────────────────── 用户桌面（三平台其一）───────────────────────┐
│  《元气小小》桌面程序（Tauri 2 打包产物）                              │
│   ├─ 桌宠窗（每角色独立：透明/置顶/无边框/不占任务栏）            │
│   ├─ 控制台窗（聊天/角色/亲密度/设置/保险箱/社交/代码检查）        │
│   ├─ 守护演出窗（全屏透明置顶，久坐劝休时出现）                   │
│   ├─ 托盘图标 + 开机自启 + 系统通知                              │
│   ├─ 本地数据（SQLite 角色/亲密度/事件/保险箱密文，本地优先）     │
│   └─ 语音：本地 TTS（三平台命令）｜云端 TTS（可选 key）           │
└──────────────┬──────────────────────────────────┬────────────────┘
               │ 直连（用户自配 key）               │ 可选（配置服务地址后启用）
               ▼                                   ▼
  豆包/火山方舟 API（大模型 + 云端 TTS）     元气小小联网服务端（Node 22 + SQLite）
  对话/技能/情感标签                         星球广场/同城灯塔（WebSocket :8787）
```

原则：**本地优先**——所有隐私数据（角色、对话、保险箱密文）只在用户机器；云端仅发送用户明确配置 key 后的必要文本片段；联网社交默认离线，配置服务器地址才启用。

---

## 2. 客户端部署（三平台）

### 2.1 产物形态（Tauri bundle，`bundle.targets = "all"`）

| 平台 | 产物 | 说明 |
|---|---|---|
| Windows 10/11 x64 | `Qibai_0.1.0_x64-setup.exe`（NSIS）+ `.msi` | 需 WebView2 Runtime（Win10/11 一般已内置，缺失时安装包可引导） |
| macOS 11+（Intel/Apple Silicon） | `.dmg` + `.app` | 建议双架构 Universal 或按架构分产物 |
| Linux（主流发行版） | `.AppImage` + `.deb` + `.rpm` | AppImage 免安装直跑；deb/rpm 走包管理器 |

### 2.2 构建环境准备（构建机）

- Rust toolchain（stable，2024+）+ `tauri-cli`（`cargo install tauri-cli --version ^2`）
- Node.js ≥ 22（仅服务端与开发工具链需要；前端为静态文件，无打包构建步骤）
- 平台系统依赖：
  - **Windows**：MSVC 构建工具链；无需额外 GUI 库（WebView2 随系统）
  - **macOS**：Xcode Command Line Tools（签名/公证需 Apple Developer 账号）
  - **Linux**：`webkit2gtk-4.1`、`gtk3`、`libsoup3`、`javascriptcoregtk-4.1`、`libayatana-appindicator3`（托盘）等（Debian/Ubuntu 示例：`sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`）

### 2.3 构建命令

```bash
# 开发（真机跑独立窗口，验证双窗口/托盘/通知/发声）
cd src-tauri && cargo run

# 打包（产物输出 src-tauri/target/release/bundle/）
cd src-tauri
cargo tauri build          # 当前平台全部 targets
# 或指定平台/格式：cargo tauri build --bundles nsis,dmg,appimage
```

> 注意：`qibai-core` 为独立 workspace，`cd qibai-core && cargo test` 不依赖 GUI 库；`src-tauri` 构建需上述系统依赖。

### 2.4 签名与公证（正式对外分发必做）

| 平台 | 要求 | 做法 |
|---|---|---|
| Windows | Authenticode 代码签名（EV 或 OV 证书） | `signtool sign /fd SHA256 /tr <TSA> /td SHA256 app.exe`；NSIS 安装包同样签名；绕过 SmartScreen 拦截 |
| macOS | Developer ID Application 证书 + **公证（notarization）** | `tauri build` 后：`xcrun notarytool submit` + `stapler`；未公证会被 Gatekeeper 拦截 |
| Linux | 非强制 | AppImage 可选 GPG 签名（`gpg --detach-sign`）；分发需信任来源 |

### 2.5 自动更新（**已接入（v11）**）

`tauri-plugin-updater` v2 已在客户端接入：`src-tauri/tauri.conf.json` 的 `updater` 段（`active: true`）+ `src-tauri/src/main.rs` 注册插件并暴露 `check_update` 命令。

**① signer 密钥生成（一次性，仅在发布机本机执行）：**

```bash
tauri signer generate -w ~/.qibai/update.key
```

- 生成输出的**公钥** → 填入 `src-tauri/tauri.conf.json` 的 `updater.pubkey`（当前为 `PASTE_PUBLIC_KEY` 占位字符串，上线前替换）；
- 生成输出的**私钥内容** → 存入 CI Secrets 的 `UPDATE_PRIVATE_KEY`，由 `release.yml` 在打包签名时注入；**绝不入库**（本地用法见根目录 `.env.example`）。

**② endpoints 配置：**

- `updater.endpoints` 当前为 GitHub Releases 占位 URL：`https://github.com/<owner>/<repo>/releases/latest/download/{{target}}/{{arch}}/{{current_version}}`；
- 把其中的 `<owner>/<repo>` 替换为真实 GitHub 仓库即可启用；
- endpoints 为数组、支持配置多个 URL，可追加镜像静态站地址做容灾。

**③ 更新检查行为：**

- 启动时后台静默检查：`main.rs` 的 `spawn_updater_check()` 在 setup 后 spawn 后台任务，`updater.check()` → 有可用更新则 `download_and_install()`，安装完成后 `app.restart()` 接管；任何错误一律静默（不 panic、不弹窗、不影响启动）；
- 控制台设置页「关于与更新」卡片提供手动「检查更新」按钮 → Tauri 命令 `check_update`，返回结构化状态 `{status: available|latest|error, current_version, latest_version?, error?}`；手动检查只回报状态、不自动安装；
- 隐私：更新流程只向 endpoint 上报当前版本号并校验包签名，不含任何用户数据。

**④ 发布流程（与 §5 release.yml 联动）：**

打 `v*` tag → release.yml 三平台构建 → 产物安装包 + `latest.json`（version / signature / url）上传 GitHub Releases → 客户端检查更新命中 `latest.json` → 下载并校验签名 → 触发安装器（Windows NSIS / macOS dmg；Linux 替换 AppImage，deb 仅作附件不参与更新）。

- 版本策略：应用版本仍 `0.1.0`，自 `0.1.0` 起遵循语义化版本，破坏性变更升 minor；「v11」是本迭代标签，不写入应用版本号。

### 2.6 首次启动与本地数据

- 数据目录（随平台惯例）：
  - Windows：`%APPDATA%\com.qibai.app\`
  - macOS：`~/Library/Application Support/com.qibai.app/`
  - Linux：`~/.local/share/com.qibai.app/`
- 首次启动向导：选择主角色（默认古典佳人）→ 可选配置豆包 API key（不配则走本地 mock/模板降级）→ 可选配置联网社交服务器地址（默认离线）
- 数据文件：`data.sqlite`（角色/亲密度/事件/社交本地缓存）+ `vault`（AES-256-GCM 密文，主密钥存系统钥匙串）——**均为本地，不随云同步上传**

---

## 3. 联网社交服务端（可选，默认离线）

### 3.1 单机部署（最小可行）

- 要求：Node.js ≥ 22.5（`server/package.json` engines 锁定）
- 安装与启动：

```bash
cd qibai/server
npm install                      # 仅 ws 一个运行时依赖
QIBAI_SERVER_PORT=8787 npm start
```

- 进程守护（systemd 单元示例）：

```ini
[Unit]
Description=Qibai Net Server
After=network.target
[Service]
WorkingDirectory=/opt/qibai/server
ExecStart=/usr/bin/node src/server.js
Environment=QIBAI_SERVER_PORT=8787
Restart=always
User=qibai
[Install]
WantedBy=multi-user.target
```

- 防火墙：仅对外开放 `8787/tcp`（HTTP 健康检查 + WebSocket）
- 数据：SQLite（`server/data/`），默认 WAL 模式；**定期备份单文件 + `-wal`**（`sqlite3 data/qibai.db ".backup bk.db"` 或定时复制）
- 反滥用：服务端已内置匿名优先/举报/拉黑/纯净模式；上线运营建议再加：连接频率限流（同 IP 每秒 ≤ N 次）、内容长度上限、请求体超时

### 3.2 容器化（可选）

```dockerfile
FROM node:22-slim
WORKDIR /app
COPY server/package*.json ./
RUN npm ci --omit=dev
COPY server/src ./src
COPY server/data ./data
ENV QIBAI_SERVER_PORT=8787
EXPOSE 8787
CMD ["node", "src/server.js"]
```

```bash
docker build -t qibai-net:0.1 .
docker run -d --name qibai-net -p 8787:8787 -v qibai-net-data:/app/data qibai-net:0.1
```

### 3.3 规模预期与演进

- 当前设计：单机 WebSocket + SQLite，适合内测/小规模（百级并发以内）
- 用户规模上来后的演进：①SQLite → PostgreSQL（会话/匹配查询）②横向扩容：WS 网关无状态化 + Redis 房间/匹配状态 ③同城匹配的 IP 归属地解析与隐私脱敏
- 合规：匿名优先、可举报、可拉黑、纯净模式（详见 `docs/NETSOCIAL_SPEC.md`）

---

## 4. 大模型接入（豆包/火山方舟，用户自配）

| 项 | 配置 |
|---|---|
| 入口 | 设置页「模型」：API Key + 模型名（默认豆包），可切换其他兼容 OpenAI 协议的端点 |
| 触发 | 聊天/技能/情绪标签/亲密度事件等场景调用；key 只存本机配置，**不落盘日志、不入库** |
| 降级 | 无 key → 本地 mock/模板回复（隐私默认态）；有 key → 云端质量 + 技能调用 |
| 云端 TTS | 每角色映射不同火山 speaker（温柔小雅/爽快思思/软萌团子/元气甜妹/热情艾娜）；无 key → 本地 TTS 音色 profile（见 VOICE.md） |
| 计费 | 按用户自身 key 的火山方舟额度计费，应用侧不做代收费 |
| 隐私边界 | 仅发送当前消息上下文（窗口内最近 N 条 + 当前角色档案摘要）；保险箱明文、密码、系统文件内容**永不进 LLM**（已硬编码过滤） |

---

## 5. CI/CD（**已配置（v11）**）

GitHub Actions 两条流水线均已落地（`.github/workflows/`）：

### 5.1 `ci.yml` — 持续集成（push / PR 触发）

- 触发：push 到任意分支 / 任意 `pull_request`；三平台矩阵 `ubuntu-latest / windows-latest / macos-latest`；
- 职责：跑 `qibai-core` `cargo test --locked`（保持 179/179 基线只增不减）+ `server` `npm ci && npm test`（Node ≥ 22.5，流水线装 Node 22）；不产物、不签名、不发布、不依赖任何 secret；
- Linux 系统依赖（仅 ubuntu，Ubuntu 24.04 包名）：`libwebkit2gtk-4.1-dev`（webkit2gtk-4.1）/ `libgtk-3-dev`（gtk3）/ `libsoup-3.0-dev`（libsoup3）/ `libayatana-appindicator3-dev` + `librsvg2-dev` / `patchelf` / `libfuse2`。

### 5.2 `release.yml` — 发版（仅 tag `v*` 触发）

- 触发：push tag `v*`（例：`git tag v0.1.0 && git push origin v0.1.0`）；三平台并行 `cargo tauri build`，分平台 bundles：Windows `nsis` / macOS `dmg` / Linux `appimage,deb`；
- updater 签名：`UPDATE_PRIVATE_KEY` **必配**——对每个 bundle 跑 `tauri signer sign` 生成 `.sig`，写入 `latest.json` 的 `signature`（未配置时流水线仍跑通，但 `signature` 为 null，客户端 updater 校验会失败）；
- 平台签名 / 公证全部条件化：macOS `APPLE_CERT` / `APPLE_NOTARY` 与 Windows `WIN_SIGN` 为**可选**——有对应 secret 才执行 codesign / notarytool / signtool，无证书则产出未签名包（发布时需随说明标注「未签名」）；
- 产物：三平台安装包上传对应 tag 的 GitHub Release，并生成 `latest.json`（`version` 取自 `tauri.conf.json`，含各平台 `signature` / `url`）供客户端 updater 使用；deb 仅作附件、不参与 updater（Linux 更新走 AppImage）。

### 5.3 Secrets 清单

| Secret | 必要性 | 用途 |
|---|---|---|
| `UPDATE_PRIVATE_KEY` | **必配** | tauri signer 私钥（`tauri signer generate` 输出），对 bundle 签名生成 `.sig` / latest.json `signature` |
| `APPLE_CERT` / `APPLE_NOTARY` | 可选 | macOS Developer ID 证书（base64 p12）+ 公证凭据；未配置则产出未签名 dmg |
| `WIN_SIGN` | 可选 | Windows Authenticode（pfx + 密码）；未配置则产出未签名 exe |

> 以上 secret 仅以 `${{ secrets.XXX }}` 占位出现于流水线，真实值只存 GitHub → Settings → Secrets and variables → Actions，**绝不入库**；本地占位清单见根目录 `.env.example`。

触发模型：push / PR 仅触发测试流水线（ci.yml，跑三平台单测）；**构建三平台包与发布仅打 tag `v*` 时触发**（release.yml）。

---

## 6. 发布渠道与迭代节奏

| 阶段 | 渠道 | 内容 |
|---|---|---|
| 内测（当前） | 源码包 + 构建指引分发 | 真机验证：双窗口独立运行/托盘/通知/发声/息屏/钥匙串 |
| 公测 | 官网下载 + GitHub Releases | 签名 + 公证 + 自动更新 |
| 正式 | 官网 + 应用商店（Microsoft Store / Mac App Store / Flathub 评估） | 商店审核需隐私政策、截图、权限说明 |

迭代节奏建议：单测基线（179）只增不减 → 每版本冒烟清单全过 → 用户验收门 → 发布。

---

## 7. 配置项清单（客户端）

| 配置 | 默认 | 位置 |
|---|---|---|
| 豆包/模型 API Key + 模型名 | 空（降级模式） | 设置页，仅本机 |
| 联网服务地址 | 空（离线） | 设置页 |
| 语音引擎（自动/本地/云端） | 自动 | 设置页 |
| 每角色音色（按性格自动/覆盖） | 按性格 | 设置页/角色卡 |
| 守护阈值（久坐分钟/三档强制/冷却） | 120min/30min | 设置页 |
| 保险箱主密钥 | 系统钥匙串 | 首次设置，本地 |
| 数据目录 | 平台惯例 | 见 §2.6 |

---

## 8. 真机验证清单（沙箱无法覆盖，发布前必须逐项）

- [ ] `cd src-tauri && cargo run`：桌宠窗独立置顶透明、控制台独立呼出、双角色两窗口并存
- [ ] 托盘图标常驻/菜单/退出；开机自启生效
- [ ] 系统通知弹出（久坐/纪念日）
- [ ] 本地 TTS 真实发声 + 五角色音色差异可听；麦克风/STT（如启用）
- [ ] 守护演出：截图→图标碎裂→全屏劝休→息屏/锁屏/倒计时（三平台命令）
- [ ] 系统钥匙串存取（Win Credential Manager / macOS Keychain / Linux gnome-keyring）
- [ ] OS 剪贴板写入与 20s 自清
- [ ] 联网社交真实链路（服务端 + 多客户端 星球/同城/举报/拉黑）
- [ ] 安装包：签名验证通过、无 SmartScreen/Gatekeeper 拦截、卸载干净
- [ ] 自动更新（v11 已接入，待真机端到端）：旧版 → 新版静默升级成功（启动后台 check → download_and_install → 重启）；signer 密钥端到端（公钥编译进包 / `UPDATE_PRIVATE_KEY` 签名 / `latest.json` signature 校验通过）；无证书流水线产物为未签名安装包，发布时需显式标注「未签名」

---

## 9. 安全与合规要点

- 本地优先：对话/角色/保险箱均不上传；云端仅必要片段且需用户配 key
- 保险箱：AES-256-GCM + PBKDF2(210,000)；主密钥钥匙串；闲置锁定；明文永不进 LLM/日志/朗读
- 合规全龄：无擦边内容；社交匿名优先 + 举报拉黑 + 纯净模式
- 代码检查：只读分析，不修改文件；mock 层不读写用户文件
- 密钥管理：发布签名私钥/更新私钥只存 CI Secrets，永不进仓库

---

*版本历史：V1.0 2026-10-22 首发（随 v10 交付）；V1.1 2026-10-23 同步 v11（自动更新已接入 §2.5、CI/CD 已配置 §5）。*
