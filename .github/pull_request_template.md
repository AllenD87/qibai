## 这个 PR 做什么 / What this PR does

<!-- 一段话说明动机：解决什么问题、对应用户的哪种场景 -->

## 改动范围 / Scope

- [ ] 仅文档 / docs only
- [ ] 前端（`src/`）/ front end
- [ ] 核心（`qibai-core/`）/ core
- [ ] 壳层（`src-tauri/`）/ shell
- [ ] 服务（`server/`）/ server
- [ ] 脚本 / scripts
- [ ] CI 或仓库配置 / CI or repo config

## 自测 / Verification

<!-- 贴真实命令与结果，不要只写"测过了" -->

```
cargo check -p qibai-core      # 结果：
cargo test  -p qibai-core      # 结果：
node --check src/*.js          # 结果：
```

如有界面改动，请附改动前后的截图（已脱敏）。

## 三条硬线 / Three hard rules

- [ ] 测试通过，没有新增失败或跳过未说明的用例 / Tests pass, no unexplained skips
- [ ] 执行类能力默认关闭，新增能力有明确授权路径 / Execution skills default off with explicit approval path
- [ ] 没有把大文件（> 5 MB）、模型权重、全量美术帧提交进 git / No large files, weights or full frame sets in git

## 关联 / Related

<!-- Fixes #123 / 关联 issue 或讨论 -->
