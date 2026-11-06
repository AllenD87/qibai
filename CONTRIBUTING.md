# 参与贡献 / Contributing

[中文](#中文) ｜ [English](#english)

---

<a id="中文"></a>
## 中文

感谢你愿意改进元气小小。这是个人向的桌面 AI 项目，维护精力有限，所以流程尽量短，但下面三条是硬线。

### 三条硬线

#### 1. 必须能过测试

```bash
cargo test -p qibai-core
cargo test -p qibai-core --test mcp_transport
```

核心库的测试覆盖任务状态机、并行调度、上下文压缩、知识库、MCP 传输——这些是这个项目的价值所在，**测试是它们的护栏**。改动核心逻辑的 PR 必须同时补测试。

前端改动至少跑一次语法检查：

```bash
node --check src/console.js src/pet.js src/tauri-bridge.js
```

#### 2. 执行类能力一律默认关闭

这个项目真的会动用户的文件和电脑。任何新增的执行类能力（写文件、执行命令、调用外部工具、截图、模拟输入）必须满足：

- 默认**关闭**，由用户显式开启；
- 走**白名单**（路径/命令/工具名），越界直接拒绝；
- 走**确认闸门**（危险步必须经用户确认才执行，未确认则留痕为「未执行」）；
- 拒绝时给出可读原因，不要静默失败。

`qibai-core/src/guard.rs` 与任务状态机里的 `waiting_confirm` 是参考实现，请沿用同一套机制。

#### 3. 大文件不进 git

| 类型 | 上限 | 正确做法 |
|---|---|---|
| 源码文件 | — | 直接提交 |
| 图片（界面用） | 单文件 ≤ 5 MB | 超过则压缩为 WebP 或放 Release |
| 模型权重、音色、二进制 | 一律不进 | `fetch_resources.sh` / Release 附件 |
| 美术帧 | 一律不进 | Release 附件 + `fetch_personas.sh` |
| 演示视频 | 一律不进 | Release / 外链 |

提交前自检：

```bash
git ls-files -z | xargs -0 du -m 2>/dev/null | awk '$1>5 {print}'
```

有输出就说明带了不该带的文件。

### 开发流程

1. Fork → 从 `main` 拉分支：`feat/xxx`、`fix/xxx`；
2. 本地跑通测试（见上）；
3. 提交信息用 Conventional Commits：`feat: xxx` / `fix: xxx` / `docs: xxx` / `chore: xxx`；
4. 开 PR，说明**改了什么 / 为什么 / 怎么验证的**，附测试输出；
5. CI 三平台（Ubuntu / Windows / macOS）必须全绿。

#### 首次上手建议

```bash
# 只跑核心库，不需要图形环境（CI 就是这么跑的）
cargo test -p qibai-core

# 跑壳层才需要 webkit2gtk 等系统依赖
cd src-tauri && cargo build
```

改动 `qibai-core` 时**不需要**装图形依赖，这一点是刻意设计的——请保持它（不要在 core 里引入任何 GUI/sys 相关依赖）。

### 报告问题

- **功能/构建问题** → 开 Issue，附：操作系统、`rustc -V`、复现步骤、错误日志；
- **安全问题**（密钥泄漏、越界执行、数据外传） → **不要开公开 Issue**，走 Security → Private vulnerability reporting；
- **使用疑问** → Discussions。

### 不接受的内容

- 绕过确认闸门或白名单的「便捷」改动；
- 引入需要联网上传用户数据的默认行为；
- 未声明的第三方素材（图片/字体/音频）——许可纠纷会连累整个项目；
- 大文件直接提交。

---

<a id="english"></a>
## English

Thanks for wanting to improve Qibai. This is a personal desktop-AI project with limited maintenance capacity, so the process is deliberately short — but these three rules are hard lines.

### Three hard rules

#### 1. Tests must pass

```bash
cargo test -p qibai-core
cargo test -p qibai-core --test mcp_transport
```

The core library's tests cover the task state machine, parallel scheduling, context compaction, the knowledge base and MCP transport — that coverage is where this project's value lives, and **the tests are its guardrails**. A PR that changes core logic must add tests alongside it.

For front-end changes, at least run the syntax check:

```bash
node --check src/console.js src/pet.js src/tauri-bridge.js
```

#### 2. Execution-type capabilities are off by default, always

This project really does touch the user's files and machine. Any new execution-type capability (writing files, running commands, calling external tools, screenshots, simulated input) must:

- be **disabled by default** and enabled explicitly by the user;
- go through a **whitelist** (paths / commands / tool names), refusing anything out of bounds;
- go through a **confirmation gate** (risky steps only run after the user confirms; otherwise the attempt is recorded as "not executed");
- give a readable reason when refusing — never fail silently.

`qibai-core/src/guard.rs` and `waiting_confirm` in the task state machine are the reference implementation; reuse the same mechanism.

#### 3. No large files in git

| Kind | Limit | Correct approach |
|---|---|---|
| Source files | — | Commit directly |
| Images (UI use) | ≤ 5 MB per file | Compress to WebP, or put it in a Release |
| Model weights, voices, binaries | Never | `fetch_resources.sh` / Release attachments |
| Artwork frames | Never | Release attachments + `fetch_personas.sh` |
| Demo videos | Never | Release / external link |

Check before committing:

```bash
git ls-files -z | xargs -0 du -m 2>/dev/null | awk '$1>5 {print}'
```

Any output means you are carrying files you should not be.

### Workflow

1. Fork, then branch from `main`: `feat/xxx`, `fix/xxx`;
2. Get the tests green locally (see above);
3. Use Conventional Commits: `feat: xxx` / `fix: xxx` / `docs: xxx` / `chore: xxx`;
4. Open a PR explaining **what changed / why / how you verified it**, with the test output attached;
5. CI must be green on all three platforms (Ubuntu / Windows / macOS).

#### First-time tips

```bash
# Core library only — no graphics environment needed (this is how CI runs it)
cargo test -p qibai-core

# Building the shell is what needs webkit2gtk and friends
cd src-tauri && cargo build
```

Working on `qibai-core` **does not** require graphics dependencies. That is a deliberate design choice — please keep it (do not introduce any GUI- or sys-related dependency into core).

### Reporting problems

- **Feature / build problems** → open an Issue with: OS, `rustc -V`, reproduction steps, error log;
- **Security problems** (key leakage, out-of-bounds execution, data exfiltration) → **do not open a public issue**; use Security → Private vulnerability reporting;
- **Usage questions** → Discussions.

### What we do not accept

- "Convenience" changes that bypass the confirmation gate or the whitelist;
- Default behaviours that upload user data over the network;
- Undeclared third-party assets (images / fonts / audio) — a licensing dispute drags down the whole project;
- Large files committed directly.
