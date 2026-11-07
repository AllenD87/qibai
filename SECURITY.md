# 安全策略 / Security Policy

[中文](#中文) ｜ [English](#english)

---

<a id="中文"></a>
## 中文

### 报告漏洞

**请不要开公开 issue。** 用下面任一方式私下联系：

- GitHub 私密漏洞报告：仓库 → **Security** → **Report a vulnerability**（推荐，能直接开私密 advisory）
- 邮件：<alunupc@126.com>，标题以 `[SECURITY]` 开头

请尽量提供：复现步骤、受影响版本 / 提交号、平台（Windows / macOS / Linux）、是否可稳定复现、你判断的影响面。
**不要**在报告里附带真实密钥、他人隐私数据或可执行的攻击载荷。

### 响应时间

| 阶段 | 目标时间 |
|---|---|
| 确认收到 | 3 个工作日内 |
| 初步定级与影响说明 | 7 个工作日内 |
| 修复或缓解方案 | 视严重程度，通常 30 天内 |

修复会在 `main` 上提交并在 Release notes 里致谢（你可以要求匿名）。

### 本项目特别在意的面

元气小小是**本地优先**的桌面应用，以下问题优先级最高：

1. **本地数据外泄**：对话、知识库文件、截图在未获用户明确触发的情况下出网；
2. **密钥与凭据**：`secrets` 保险箱（AES-GCM + PBKDF2）被绕过、Key 被写入日志或崩溃转储；
3. **执行边界**：`execute_command` 等执行类技能突破白名单目录、跳过逐次确认；
4. **供应链**：`fetch_resources.sh` / `fetch_personas.sh` 下载链被替换、校验被绕过；
5. **提示注入**：外部内容（网页、文件、MCP 返回）诱导模型调用越权技能。

### 不在范围内

- 需要用户本机已被攻破才能触发的问题；
- 第三方组件自身的漏洞（请上报给上游，本项目会在 `THIRD-PARTY-NOTICES.md` 跟进）；
- 你自己填入的第三方 API Key 被服务商侧封禁 / 计费问题；
- 纯 UI 观感或功能建议（请走普通 issue）。

### 版本与分支

只维护 `main` 与最新 Release 对应的版本；更早的 Release 不单独出补丁。

---

<a id="english"></a>
## English

### Reporting a vulnerability

**Please do not open a public issue.** Contact us privately instead:

- GitHub private vulnerability report: repo → **Security** → **Report a vulnerability** (preferred — it opens a private advisory)
- Email: <alunupc@126.com> with a subject starting `[SECURITY]`

Please include: reproduction steps, affected version / commit, platform (Windows / macOS / Linux), whether it reproduces reliably, and the impact you believe it has.
**Do not** include real keys, other people's private data, or weaponised payloads in the report.

### Response times

| Stage | Target |
|---|---|
| Acknowledgement | within 3 business days |
| Initial severity and impact statement | within 7 business days |
| Fix or mitigation | usually within 30 days, depending on severity |

Fixes land on `main` and are credited in the release notes (anonymously if you prefer).

### What matters most in this project

Qibai is a **local-first** desktop app, so these classes of issue take top priority:

1. **Local data leaving the machine**: conversations, knowledge-base files or screenshots going out without an explicit user action;
2. **Keys and credentials**: the `secrets` vault (AES-GCM + PBKDF2) being bypassed, or API keys ending up in logs or crash dumps;
3. **Execution boundary**: execution-class skills such as `execute_command` escaping the whitelisted directory or skipping per-action confirmation;
4. **Supply chain**: `fetch_resources.sh` / `fetch_personas.sh` download chains being swapped or their verification bypassed;
5. **Prompt injection**: external content (web pages, files, MCP responses) steering the model into privileged skills.

### Out of scope

- Issues that require the user's machine to already be compromised;
- Vulnerabilities inside third-party components (report them upstream; this project tracks them in `THIRD-PARTY-NOTICES.md`);
- Bans or billing problems caused by third-party API keys you supplied yourself;
- Pure UI polish or feature requests (please use a normal issue).

### Versions and branches

Only `main` and the version behind the latest Release are maintained; older Releases do not receive separate patches.
