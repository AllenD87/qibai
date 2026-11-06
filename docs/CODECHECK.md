# 代码检查（codecheck）模块说明

迭代①新增的「代码检查」核心模块，位于 `qibai-core/src/codecheck.rs`。
本模块是**纯 Rust 库逻辑**，不依赖 `crate::` 内任何其他模块，只依赖
`serde / serde_json / thiserror / async-trait / tokio(process)`。

---

## 1. 只读保证（安全红线）

模块对用户文件系统**只读**，绝不写、改、删任何文件。原因与做法：

| 语言 | 做法 | 为什么不这样做 |
|---|---|---|
| JavaScript (.js/.mjs/.cjs) | `node --check <file>` | Node 自带语法检查，纯解析不落盘 |
| Python (.py) | `python3 -c "import ast,sys; ast.parse(open(sys.argv[1],encoding='utf-8').read())" <file>` | **不用** `py_compile`：它会写 `__pycache__/`，违反只读 |
| JSON (.json) | `serde_json` 直接 parse | 无外部命令 |
| Rust (.rs) | 启发式 + 人工提示 | **绝不运行 `cargo check`**：它会写 `target/`，违反只读 |
| TypeScript (.ts/.tsx) | 启发式 + 人工提示 | `tsc` 需项目环境，且可能写 `.tsbuildinfo` |
| 其他 (html/css/md/txt/unknown) | 启发式 + 人工提示 | 无可靠轻量检查器 |

路径扫描硬限制：单文件 ≤5MB、总计 ≤50MB、目录遍历 ≤500 个文件；超限截断并在报告中加 hint。
跳过目录：`target/ node_modules/ .git/ _shots/ _backup/ .venv/ __pycache__/` 及一切隐藏目录/文件。

---

## 2. 扩展名分发

| 扩展名 | 语言名 | 检查方式 |
|---|---|---|
| .js / .mjs / .cjs | javascript | `node --check` + 启发式 |
| .py | python | `ast.parse` + 启发式 |
| .json | json | serde_json parse + 启发式 |
| .rs | rust | 启发式 + "建议本地 cargo check" 提示 |
| .ts / .tsx | typescript | 启发式 + "建议本地 tsc --noEmit" 提示 |
| .html / .htm | html | 启发式 |
| .css | css | 启发式 |
| .md / .markdown | markdown | 启发式 |
| .txt | text | 启发式 |
| 其他 | unknown | 启发式 |

目录模式：报告 `language = "mixed"`；单文件按上表。

---

## 3. LLM 降级（无 key 是默认路径，不是错误）

本地 trait：

```rust
#[async_trait]
pub trait SuggestLlm: Send + Sync {
    async fn suggest(&self, report: &CodeReport, language: &str) -> Option<String>;
}
```

- `run_check(req, runner, None)` → `CodeReport.llm_note = None`，输出纯静态报告；
- 传了 `suggester` 但返回 `None`（如网络失败、无 key）→ 同样 `llm_note = None`；
- 真实 `LlmProvider` 由集成代理在 engine 侧适配后传入，本模块不关心具体实现。

---

## 4. 报告 JSON 形状（冻结契约）

### CodeIssue

```json
{
  "severity": "error",
  "category": "syntax",
  "message": "SyntaxError: Missing semicolon. (2:10)",
  "location": "/abs/path/to/file.js:2",
  "suggestion": "根据报错行修正 JS 语法"
}
```

- `severity`: `"error" | "warning" | "hint"`
- `category`: `"syntax" | "logic" | "security" | "style" | "unknown"`
- `location`: `"file:line"` 字符串，或 `null`
- `suggestion`: 字符串，或 `null`

### CodeReport

```json
{
  "source": "path",
  "path": "/abs/path/to/dir-or-file",
  "files_checked": 3,
  "language": "javascript",
  "summary": "发现 2 个问题：error 1 / warning 1 / hint 0",
  "issues": [ /* CodeIssue[] */ ],
  "llm_note": null
}
```

- `source`: `"path" | "snippet"`
- `path`: path 模式为绝对路径字符串；snippet 模式为 `null`
- `files_checked`: u32
- `language`: `javascript / python / rust / typescript / json / html / css / markdown / text / unknown / mixed`
- `summary`: 有 issues → `"发现 N 个问题：error X / warning Y / hint Z"`；无 → `"未发现问题"`
- `llm_note`: 字符串或 `null`

### CheckRequest JSON（前端 / LLM 调用时传入）

```json
{ "source": "path", "path": "/abs/path" }
{ "source": "snippet", "snippet": "const x = 1;", "language": "javascript" }
```

`from_json` 校验：`path` 与 `snippet` 恰好提供一个；`source` 必须是 `path|snippet`；
非法输入返回结构化错误字符串（非 panic）。

---

## 5. 启发式规则（全部可单测）

| 类别 | 规则 | severity |
|---|---|---|
| security | `eval(` | warning |
| security | `child_process` 引用 | hint |
| security | `execSync(` / `spawn(` | warning |
| security | `rm -rf` 与 `$` / 反引号插值组合 | error |
| security | 硬编码密钥（api_key/apikey/api-key/password/passwd/token/secret/doubao_api_key 赋值给字面量；`sk-` 前缀 ≥8 位） | error |
| security | `innerHTML` 与 `+` 拼接 | warning |
| logic | JS 宽松相等 `==` / `!=`（非 `===` / `!==`） | warning |
| logic | `while(true)` | hint |
| logic | 空 `catch` / 空 `except: pass` | warning |
| logic | 除以字面量 0（`/ 0`、`/0`） | warning |
| style | 行 >120 字符 | hint |
| style | 行首 Tab 与空格混用 | warning |
| style | 行尾空白 | warning |
| style | TODO / FIXME / XXX | hint |

snippet 模式自动嗅探：含 `function` / `=>` / `var|let|const` → javascript；
含 `def ` / `import ` / `print(` → python；trim 后以 `{` 或 `[` 开头 → json；否则 unknown。

---

## 6. 沙箱验证记录（2026-10-22）

- `cargo 1.99.1` (`~/.cargo/bin/cargo`) 编译通过；
- `node v22.23.2` (`/opt/nodejs/22/bin/node`) 对 `tests/fixtures/bad.js` 真实跑出 `SyntaxError`，对 `healthy.js` 通过；
- `python3.12` (`/opt/python3.12/bin/python3`) 对 `tests/fixtures/bad.py` 真实跑出 `SyntaxError: '(' was never closed`，对 `healthy.py` 通过；
- `bad.json`（尾随逗号）由 serde_json 检出；
- scratch crate 单测 **19 passed / 0 failed**（详见交付回交记录）。

---

## 7. 真机验证清单（集成后跑）

- [ ] 在 `qibai-core` 下 `cargo test` 全绿（集成代理在 `lib.rs` 加 `pub mod codecheck;` 后）；
- [ ] tauri-bridge 把 `check_code` 注册为白名单技能（只读、无需审批、默认开）；
- [ ] 真实工程目录（如 qibai 自身）跑一次：确认 `target/`、`_backup/` 被跳过，且 `target/` 下无新增写；
- [ ] 无 `DOUBAO_API_KEY` 环境变量时，`llm_note=null` 仍能返回完整静态报告；
- [ ] 前端 mock 按 §4 形状渲染 `CodeReport`，字段对齐无 undefined。

---

## 8. fixtures（供测试 + 控制台演示引用）

绝对路径：`qibai-core/tests/fixtures/`

- `bad.js`：含 `eval`、`sk-` 硬编码 key、`innerHTML+变量`、`==`、未闭合 `function(` → 真实语法错误；
- `bad.py`：`print(message` 未闭合括号；
- `bad.json`：尾随逗号；
- `healthy.js` / `healthy.py`：干净文件。
