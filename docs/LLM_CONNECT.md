# 《元气小小》大模型连接配置与验收记录 V12

> 2026-10-23 单机部署·真机运行·连接大模型验收。**真实 Key 不入本仓库/交付包**，按本指引配置。

---

## 1. 已验证的大模型端点（实测 2026-10-23）

| 项 | 值 |
|---|---|
| 端点（OpenAI 兼容） | `https://ark.cn-beijing.volces.com/api/v3/chat/completions` |
| 认证 | Bearer API Key（火山方舟「API Key 管理」创建，免费） |
| 已实测可用模型 | `deepseek-v4-pro-ga-260813` ✅（元气小小人设回复质量良好）<br/>`deepseek-v4-flash-ga-260731` ✅<br/>`deepseek-v4-1-flash-260910` ✅<br/>`deepseek-v4-1-flash-261017` ✅ |
| 未开通（404） | 豆包系全部（doubao-seed-2-1-*/doubao-pro-*/doubao-lite-*）→ 需在方舟控制台「开通管理」逐一开通 |
| 免费额度 | 方舟每个模型有免费调用额度（如 Doubao-Seed 系列 50 万 tokens）；实测 DeepSeek 系列已可直接调用 |

> **注意（v45.26 起）**：应用默认模型为 `doubao-pro-32k`，默认端点即上表 Ark 地址。豆包系需先在方舟控制台「开通管理」开通，**未开通时返回 404**，报错信息不会明说原因。不想开通就把模型名换成上表实测可用的 `deepseek-v4-pro-ga-260813`——端点和 Key 都不用动，只改模型名一项。

实测示例（HTTP 200，5.2s）：
> 系统：「你是《元气小小》桌面伴侣·古典佳人，说话温婉简短，带一点古风。」
> 用户：「深夜还在加班，安慰我一句」
> 回复：「更深露重，莫要熬坏了身子。且饮杯热茶，歇一歇，奴家陪你。」

## 2. 配置方式（三选一，优先级从上到下）

### 2.1 环境变量（推荐，真机/CI 通用）
```bash
export DOUBAO_API_KEY="<你的方舟API Key>"     # 代码解析优先级最高
export DOUBAO_MODEL="deepseek-v4-pro-ga-260813"
# 可选：默认即火山方舟端点，自定义端点时
export DOUBAO_BASE_URL="https://ark.cn-beijing.volces.com/api/v3/chat/completions"
```

### 2.2 应用设置页
控制台 → 设置 → 模型：填 API Key、模型名（如上），端点默认火山方舟；支持任意 OpenAI 兼容端点（如局域网 `http://host:8080/v1`）。

### 2.3 qibai-core 配置（settings.json，数据目录）
`llm.provider / llm.protocol / llm.base_url / llm.model / llm.api_key` 字段。
`llm.protocol` 取 `auto`（默认，按 provider 推断）/ `openai` / `anthropic`；缺字段的老配置按 `auto` 加载，行为与升级前一致。

### 2.4 Anthropic Claude（原生协议，v45.31-e）

Claude 官方不提供 OpenAI 兼容端点，所以应用内置了 Anthropic 原生 `/v1/messages` 适配：

| 项 | 值 |
|---|---|
| 服务商下拉 | `Anthropic` |
| 接口协议 | `自动`（按服务商推断，选 Anthropic 即走原生） |
| Base URL | `https://api.anthropic.com`（留空同样生效，后端自动补 `/v1/messages`） |
| 模型名 | 如 `claude-sonnet-4-5`、`claude-opus-4-1`（填控制台里看到的模型 id） |
| 鉴权 | Key 填 `sk-ant-...`；后端发 `x-api-key` + `anthropic-version: 2023-06-01`，**不是 Bearer** |

适配层覆盖：`system` 提升到顶层字段、`max_tokens` 必填、OpenAI 工具 schema → `input_schema`、工具结果回传 `tool_result`、流式 `content_block_delta` / `input_json_delta` 重组、usage 合并。

**边界**：
- 备用服务商 `backup_providers` 只在 OpenAI 兼容协议下生效，Anthropic 分支不做多端点 failover。
- `json_object` 用系统提示约束（Anthropic 无 `response_format` 字段）。
- 走 one-api / new-api 这类中转网关时，把「接口协议」改成 `强制 OpenAI 兼容`，就能复用原来的 `/chat/completions` 路径。

## 3. 应用代码路径验收（本机已跑通）

交付包内置验收工具：

```bash
cd qibai/qibai-core
DOUBAO_API_KEY="<key>" DOUBAO_MODEL="deepseek-v4-pro-ga-260813" \
  ~/.cargo/bin/cargo run --example llm_live
```

输出（实测记录）：
```
[元气小小] 正在连接大模型…（deepseek-v4-pro-ga-260813）
[元气小小] 大模型回复: 更深露重，莫要熬坏了身子。且饮杯热茶，歇一歇，奴家陪你。
[元气小小] 连通验证 ✅（应用代码路径 DoubaoProvider 真实调用成功）
```

## 4. 单机部署运行（真机）

- 完整步骤见 `docs/DEPLOYMENT.md`（§2 客户端部署 / §8 真机验证清单）
- 本机快速预览：浏览器打开 `src/console.html`（mock 全功能）
- 真机独立窗口：`cd src-tauri && cargo run`（需先装 GUI 系统库，见 DEPLOYMENT §2.2）
- 云主机验证受限项（如实标注）：src-tauri 编译需 webkit2gtk 等系统库，云主机 apt 源不可达无法安装 → 真机构建请在用户本机执行；**大模型连通性已在云主机用应用真实代码路径验证通过**

## 5. 安全

- 本仓库/交付包**不含任何真实 Key**（`.env.example` 为占位模板；llm_live.rs 只读环境变量）
- Key 仅存本机配置；保险箱明文/密码/系统文件永不进 LLM（已有硬编码过滤）
