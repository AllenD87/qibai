# 参与贡献 / Contributing

感谢你愿意改进元气小小。这是个人向的桌面 AI 项目，维护精力有限，所以流程尽量短，但下面三条是硬线。

---

## 三条硬线

### 1. 必须能过测试

```bash
cargo test -p qibai-core
cargo test -p qibai-core --test mcp_transport
```

核心库的测试覆盖任务状态机、并行调度、上下文压缩、知识库、MCP 传输——这些是这个项目的价值所在，**测试是它们的护栏**。改动核心逻辑的 PR 必须同时补测试。

前端改动至少跑一次语法检查：

```bash
node --check src/console.js src/pet.js src/tauri-bridge.js
```

### 2. 执行类能力一律默认关闭

这个项目真的会动用户的文件和电脑。任何新增的执行类能力（写文件、执行命令、调用外部工具、截图、模拟输入）必须满足：

- 默认**关闭**，由用户显式开启；
- 走**白名单**（路径/命令/工具名），越界直接拒绝；
- 走**确认闸门**（危险步必须经用户确认才执行，未确认则留痕为"未执行"）；
- 拒绝时给出可读原因，不要静默失败。

`qibai-core/src/guard.rs` 与任务状态机里的 `waiting_confirm` 是参考实现，请沿用同一套机制。

### 3. 大文件不进 git

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

---

## 开发流程

1. Fork → 从 `main` 拉分支：`feat/xxx`、`fix/xxx`；
2. 本地跑通测试（见上）；
3. 提交信息用 Conventional Commits：`feat: xxx` / `fix: xxx` / `docs: xxx` / `chore: xxx`；
4. 开 PR，说明**改了什么 / 为什么 / 怎么验证的**，附测试输出；
5. CI 三平台（Ubuntu / Windows / macOS）必须全绿。

### 首次上手建议

```bash
# 只跑核心库，不需要图形环境（CI 就是这么跑的）
cargo test -p qibai-core

# 跑壳层才需要 webkit2gtk 等系统依赖
cd src-tauri && cargo build
```

改动 `qibai-core` 时**不需要**装图形依赖，这一点是刻意设计的——请保持它（不要在 core 里引入任何 GUI/sys 相关依赖）。

---

## 报告问题

- **功能/构建问题** → 开 Issue，附：操作系统、`rustc -V`、复现步骤、错误日志；
- **安全问题**（密钥泄漏、越界执行、数据外传） → **不要开公开 Issue**，走 Security → Private vulnerability reporting；
- **使用疑问** → Discussions。

## 不接受的内容

- 绕过确认闸门或白名单的"便捷"改动；
- 引入需要联网上传用户数据的默认行为；
- 未声明的第三方素材（图片/字体/音频）——许可纠纷会连累整个项目；
- 大文件直接提交。
