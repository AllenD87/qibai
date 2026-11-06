# 豆包工作 / 办公能力接入架构

本文件说明后续如何把「飞书文档 / 日历 / 邮件」等办公能力接入元气小小，
包括凭据接入路径、SkillProvider 抽象、未配置时的降级策略。

---

## 1. 目标

在不污染现有 MVP 技能体系（`launch_app` / `open_folder` / `set_reminder`）的前提下，
把办公类能力作为一组独立的 `SkillProvider` 注入到 agent loop。
要求：

- 凭据**绝不硬编码**；运行时从 env / 配置文件读取；
- 未配置凭据时**优雅降级**，给用户友好提示而不是崩溃；
- 所有技能统一走白名单 + 总开关 + 每技能开关的权限模型。

---

## 2. 抽象（已在 `qibai-core/src/doubao_work.rs` 实现骨架）

```rust
/// 凭据运行时读取
pub trait WorkCredentialStore: Send + Sync {
    fn get(&self, service: &str) -> Option<String>;
}

/// 办公技能扩展 trait
#[async_trait]
pub trait SkillProvider: Send + Sync {
    fn id(&self) -> &str;            // 如 "feishu_doc"
    fn name(&self) -> &str;           // 如 "飞书文档"
    fn requires_credentials(&self) -> bool;
    async fn invoke(&self, args: &Value) -> Result<Value, WorkError>;
}
```

---

## 3. 凭据接入路径（按优先级）

| 顺序 | 来源 | 示例 |
|---|---|---|
| 1 | 环境变量 | `FEISHU_DOC_TOKEN`、`FEISHU_CALENDAR_TOKEN`、`FEISHU_MAIL_TOKEN` |
| 2 | 配置文件 `<data_dir>/settings.json` 扩展字段 | `work.feishu_doc.token` 等 |

规则：

- 环境变量优先；为空再读配置；都读不到 → `WorkError::NotConfigured`。
- 凭据只在 `invoke` 时按需读取，不长期缓存到除进程外的地方。
- 前端「设置」页提供录入入口，写入 `settings.json`；敏感字段以最小权限存储。

---

## 4. 执行路径

```
用户消息
  → LLM 返回 tool_call(name="feishu_doc.search", args={query})
  → Engine 通过 SkillProviderRegistry 路由到对应 SkillProvider
  → WorkCredentialStore::get("feishu_doc")
      ├─ Some(token) → provider.invoke(args) → 返回结构化结果
      └─ None        → WorkError::NotConfigured → 降级文本回填
  → 模型拿到结果后给用户最终 JSON 回复
```

`invoke_or_degrade` 是统一入口：

```rust
pub async fn invoke_or_degrade<P: SkillProvider>(
    provider: &P,
    creds: &dyn WorkCredentialStore,
    args: &Value,
) -> Result<Value, WorkError>
```

未配置时返回：`办公技能「飞书文档」尚未配置凭据。请到设置页绑定对应服务。`

---

## 5. 后续接入清单

| 服务 | 凭据 env | 备注 |
|---|---|---|
| 飞书文档 | `FEISHU_DOC_TOKEN` | 搜索 / 创建 / 追加内容 |
| 飞书日历 | `FEISHU_CALENDAR_TOKEN` | 查日程 / 建日程 |
| 飞书邮件 | `FEISHU_MAIL_TOKEN` | 查收件箱 / 起草 |
| 飞书 IM | `FEISHU_IM_TOKEN` | 发消息（需二次审批） |

每新增一个技能：

1. 实现 `SkillProvider`；
2. 在 `WorkCredentialStore` 注册对应 service 名；
3. 在 Engine 工具 schema 里加 OpenAI function 定义；
4. 在前端设置页加凭据录入 UI。

---

## 6. 降级策略（总览）

- **未配置凭据**：返回友好提示，不重试、不阻塞对话；
- **凭据过期 / 401**：提示用户重新授权；
- **网络错误**：告知「办公服务暂时连不上」，并建议稍后再试；
- **未授权操作**（如往别人日历写日程）：走与 `execute_command` 一样的
  `qibai://skill-approval` 审批流，前端确认后执行。
