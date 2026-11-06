# 网页摘要（web-summary）· com.qibai.skill.websummary

把指定网页抓下来提炼成中文摘要。只读、防 SSRF、仅白名单站点。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.websummary` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | `network:read`（最小集） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「使用网页摘要，总结 https://…」「帮我看看这篇文章讲了什么」「总结一下这个链接」 |

## 文件清单

```
web-summary/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 可执行脚本：白名单抓取 + 标题/正文提取
├── SKILL.md            # WorkBuddy 技能规范：Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy 技能规范：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明（上架时可放 icon.png）
└── README.md           # 本文档
```

## 来源参考

- 素材源：VoltAgent/awesome-agent-skills（`SKILLS.md` 全量技能库，约 26k stars）。
- 原素材特点：社区 SKILL.md 多为「主模型直接总结网页」或「curl 任意 URL」，普通缺 URL 校验与防 SSRF。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/skills-research.md` 的「web-summary」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md 含 Trigger/Instructions/Example + changelog | ✅ `SKILL.md` 四个分区齐全 |
| config.yaml 含 name/version/description/triggers/category | ✅ `config.yaml` 五字段齐全 |
| assets/ 资源目录 | ✅ `assets/`（含说明 README） |
| 上架前安全扫描硬标准 | ✅ 见下方安全自查表 |

## 权限与安全设计

- 防 SSRF：仅接受 http/https；域名需命中默认白名单（资讯/文档/百科类），或用户在
  `QIBAI_WEBSUMMARY_ALLOWED_DOMAINS` 中显式追加；拒绝 `file://`、内网地址等。
- 防命令注入：URL 以参数形式（双引号包裹）传给 curl，不经 shell 拼接；参数先经协议/域名校验。
- 防数据外泄：只请求用户指定的网页，不读取、不上传任何本地文件。
- 防路径越界：不涉及本地路径读写。
- 限时 15s、限制响应 2MB，避免占用过大。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 仅抓取用户指定 URL，不传本地数据 |
| 3 | 无越权系统命令 | ✅ 仅 curl 只读请求，无写操作 |
| 4 | 无不安全 API 调用 | ✅ URL 经协议+域名白名单校验后参数化调用 |
| 5 | 命令注入防护 | ✅ 参数不进 shell 字符串拼接 |
| 6 | 路径遍历防护 | ✅ 无本地路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参数/非法协议/非白名单/抓取失败均有中文提示 |

## 完善点（相对原素材）

- 原素材普遍缺：URL 协议校验、SSRF 防护、超时/大小限制 → 本包全部补齐。
- 原素材多为「直接给 LLM 原始 HTML」→ 本包先机械提取标题与正文，控制输入长度，降低 token 消耗。
- 新增：白名单可经环境变量扩展，兼顾安全与可用性。

## 语音调用示例

- 「元气小小，使用网页摘要，总结 https://zh.wikipedia.org/wiki/人工智能」
- 「元气小小，帮我看看这篇文章讲了什么」→ 命中 web-summary

## 自测结果

- `bash -n run.sh` ✅ 通过；脚本实跑：白名单内 URL 正常返回标题+正文，`file://`、非白名单域名、空参数均优雅拒绝。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
