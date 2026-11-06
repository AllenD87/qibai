# 文件搜索（file-search）· com.qibai.skill.filesearch

在用户主目录内按文件名关键词只读查找文件：自动排除依赖目录与敏感目录，搜不到明说、绝不编造路径。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.filesearch` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | `files:read`（只读，最小集） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「找文件」「搜索文件」「文件在哪里」「帮我找一下简历」 |

## 文件清单

```
file-search/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 只读 find 检索脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源 1：alirezarezvani/claude-skills `productivity/capture/skills/capture/SKILL.md`（重点 Section 3「Connections」）+ `scripts/workspace_inventory.py` —— Glob 文件名 + Grep 内容双通道、「NEVER fabricate connections，搜不到就明说」的诚信红线。
- 素材源 2：alirezarezvani/claude-skills `engineering/skills/focused-fix/SKILL.md`（Phase 1 SCOPE 先画边界、Phase 2 TRACE）—— 搜索前先限定根范围。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/skills-research.md` 的 §5（file-search）。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（name/display_name/display_name_en/description/description_zh/description_en/version/author） | ✅ 9 字段齐全（含 examples_zh） |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，与 SKILL.md / functions 三处一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- **最小权限**：仅 `files:read`；脚本只跑只读 `find`，不写、不删、不改任何文件。
- **范围锁定**：find 根固定为 `$HOME`，maxdepth 6；自动排除 node_modules/.git/target/.cache、.ssh/.gnupg 隐私目录，以及名称含 secret/password/credential/token 的目录——避免把依赖里的密钥文件搜出来误报。
- **诚信红线**：0 命中时输出「未找到 + 换关键词建议」，绝不编造路径；keyword 全程引号传参，拒绝 `..` 与超长输入。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无 curl/wget、无网络调用 |
| 3 | 无越权系统命令 | ✅ 仅只读 find/grep/sed |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ keyword 引号包裹传参，不拼 shell 命令串 |
| 6 | 路径遍历防护 | ✅ 拒绝 `..`、根固定 $HOME、maxdepth 6、敏感目录 prune |
| 7 | 异常输入优雅失败 | ✅ 空参/超长/非法写法均中文提示后 exit 0 |

## 完善点（相对原素材）

- 原素材脚本常遍历全目录、无排除规则 → 本包**内置默认排除清单**（依赖目录 + 隐私目录 + 敏感命名目录）。
- 原素材不限制根边界（`..`/symlink 可越界）→ 本包固定 `$HOME` 根 + maxdepth 6 + 拒绝 `..`。
- 原素材 Grep 内容通道可能把 `.env`、密钥文件带进上下文 → 本包**只保留 Glob 文件名通道**，不读取文件内容，从根上规避敏感内容泄露。
- 原素材未强调「搜不到明说」→ 本包把该红线写进输出文案与 SKILL.md Instructions。

## 语音调用示例

- 「元气小小，帮我找一下「发票」这个文件」
- 「元气小小，搜索文件，关键词 report」
- 「元气小小，我上次保存的合同在哪里？」→ 命中 file-search（关键词「合同」）

## 自测结果

- `bash -n run.sh` ✅；实跑：命中路径真实打印、空参/`..`/超长/未命中路径均友好中文输出、无网络调用。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
