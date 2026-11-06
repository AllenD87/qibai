# com.qibai.websearch

示例联网搜索技能插件（格式演示包）。

## 用法

对元气小小说：

- 「搜索一下 量子计算最新进展」
- 「帮我查查 石家庄今天天气」
- 「网上说的 XX 是真的吗，帮我搜搜」

LLM 会调用 `web_search` 工具（参数 `{"query": "..."}`），内核按
**搜狗 → 360 → 移动百度** 三级降级链抓取网页，解析前 5 条结果
（标题 + 摘要 + 链接）回填对话。

## 实现说明

本包是**占位清单**：`web_search` 实际是 qibai-core 内置技能
（`BuiltinKind::WebSearch`），运行时被 skills.rs 直接 dispatch 拦截，
`scripts/search.sh` 不会被执行。清单仅用于演示插件格式与专家白名单挂载。

## 权限声明

`network:http`——只读 GET，8s 超时，浏览器 UA，不写本地文件。

## 安装

```bash
cp -r com.qibai.websearch <plugins_dir>/
```
