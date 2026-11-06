# 元气小小 Qibai — 迭代③：本地密码保险箱（冻结规格 v1）

> 本文档为迭代③的**冻结契约**：Rust core、Tauri 命令层、前端 mock 三方必须同形实现。
> 加密/状态机/门控逻辑一律在 `qibai-core`（纯 Rust，沙箱可编译可测）；Tauri 层只做
> 钥匙串适配、剪贴板 OS 写操作、文件写盘；前端（含浏览器 mock）只做交互与展示。
> 设计红线：**密码明文永不落盘、永不进 LLM 上下文、永不进 TTS 朗读文本**。

---

## 1. 数据格式（`<data_dir>/vault.qbkv`，导出备份同格式）

单个 JSON 信封（UTF-8）：

```jsonc
{
  "format": "qibai-vault",
  "version": 1,
  "mode": "keyring" | "master_password",
  "kdf": {
    "algo": "PBKDF2-SHA256",
    "iterations": 210000,          // 硬性 ≥ 210000（KDF_ITERATIONS 常量）
    "salt_b64": "<16B 随机盐>"
  },
  "nonce_b64": "<12B 随机 nonce>",
  "aad": "qibai-vault-v1",
  "verifier_b64": "<AES-256-GCM(常量 \"qibai-vault-verifier-v1\")，同一把 key>",
  "ciphertext_b64": "<AES-256-GCM(entries JSON)>"
}
```

- 每次落盘整体重加密；**盐固定**（同一保险箱自 setup 起沿用同一随机盐，解锁/解密始终从文件头读取盐），**nonce 每次落盘重新随机生成**。
- `mode=keyring`：key = 系统钥匙串中的 32B 随机主密钥（由 Tauri 层读写），kdf 字段仅占位（verifier 用同一 key 加密，用于校验钥匙串返回的 key 是否为本保险箱的 key）。
- `mode=master_password`：key = PBKDF2-SHA256(master_password, salt, iterations) 派生。
- entries JSON（仅存在于密文中）：

```jsonc
[
  { "id": "<uuid>", "site": "微博", "account": "user@x.com",
    "password": "...", "note": "...", "icon": "weibo", "updated_at": 1700000000 }
]
```

## 2. 设置项（settings.json 新增 `vault` 段）

```jsonc
"vault": {
  "idle_lock_min": 5,        // 闲置自动锁定（分钟，默认 5，可配 1..=120）
  "max_wrong_attempts": 5,   // 连续输错主密码次数（默认 5）
  "cooldown_secs": 30,       // 冷却秒数（默认 30）
  "clipboard_clear_secs": 20 // 剪贴板自动清除秒数（默认 20）
}
```

## 3. 状态机

- `state`: `unset`（未初始化）→ `locked` → `unlocked` →（lock/闲置超时/冷却）→ `locked`；`lock()` 可随时手动。
- `mode`: `keyring` / `master_password`（unset 时前端向导选择；Tauri 层探测钥匙串可用性后建议默认）。
- 防暴力（仅 master_password 模式）：连续 `max_wrong_attempts` 次解锁失败 → 进入 `cooldown_until = now + cooldown_secs`；冷却期内任何解锁尝试（含正确密码）一律拒绝并返回 `VaultCooldown{remaining}`。
- 闲置锁定：`tick(now)` 时若 `unlocked` 且 `now - last_activity > idle_lock_min*60` → 自动锁定。`touch()` 刷新 last_activity（list/add/update/delete/search/copy 均 touch）。
- 内存 key 用 `zeroize` 清零；`lock()` 立即置零。

## 4. IPC 命令（core Engine 方法 = Tauri 命令 = 浏览器 mock，同名同形）

错误一律结构化：`VaultLocked` / `VaultCooldown{remaining_secs}` / `VaultWrongPassword` /
`VaultNotInitialized` / `VaultAlreadyUnlocked` / `VaultGateLocked{required}` / `VaultNotFound` /
`VaultImportFailed` / `VaultKeyringUnavailable`（结构化错误，不 panic）。

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `vault_status` | — | `VaultStatus` | 见下 |
| `vault_setup` | `{ master_password }` | `"ok"` | 仅 unset 且走主密码模式；生成盐/verifier/空库落盘 |
| `vault_setup_keyring` | — | `"ok"` | 仅 unset；Tauri 层生成随机 32B key 存钥匙串后调 core 落盘（keyring 模式） |
| `vault_unlock` | `{ master_password }` | `"ok"` | 主密码解锁（防暴力计数/冷却） |
| `vault_unlock_system` | — | `"ok"` | keyring 模式：Tauri 层取 key → core 校验解锁（不计数失败） |
| `vault_lock` | — | `"ok"` | 立即锁定 |
| `vault_list` | — | `[Entry]` | 仅 unlocked；自动 touch |
| `vault_search` | `{ q }` | `[Entry]` | 本地过滤 site/account/note（大小写不敏感、中文子串）；仅 unlocked |
| `vault_add` | `{ site, account, password, note, icon }` | `{ id }` | 仅 unlocked |
| `vault_update` | `{ id, site, account, password, note, icon }` | `"ok"` | 仅 unlocked；password 传空串表示不改 |
| `vault_delete` | `{ id }` | `"ok"` | 仅 unlocked |
| `vault_copy` | `{ id }` | `{ copied: true, clear_after_secs }` | core 返回明文+清除时长给 Tauri/mock 层写剪贴板并调度清除；**返回给前端的载荷不含明文** |
| `vault_export` | — | `{ path }` | core 生成 .qbkv payload，Tauri 层写 `<data_dir>/vault-backup-<ts>.qbkv`；mock 层生成 Blob 下载并返回占位 path |
| `vault_import_payload` | `{ payload, master_password }` | `"ok"` | 用备份自己的 kdf+密码解密校验；成功后：若本地 unset 则以该备份重建保险箱；若已初始化且 unlocked 则以**本地** key 重加密合并（冲突按 id 覆盖）；若本地 locked 则结构化拒绝 `VaultLocked`。**导入成功后统一回到 `locked` 状态**（重建与合并均如此），随后用户按本地模式重新解锁。Tauri/mock 前端统一用 file input 读 .qbkv 文本传 payload |

`VaultStatus`：

```jsonc
{
  "state": "unset|locked|unlocked",
  "mode": "none|keyring|master_password",
  "keyring_available": true,        // Tauri 层探测（mock 恒 true 表示"可用"路径演示）
  "gated": true,                    // active persona 亲密度 < 4 → 保险箱不可用
  "required_level": 4,
  "idle_lock_min": 5,
  "max_wrong_attempts": 5,
  "cooldown_secs": 30,
  "clipboard_clear_secs": 20,
  "failed_attempts": 0,
  "cooldown_remaining_secs": 0,
  "locked_reason": null,            // "manual"|"idle"|"cooldown"|null
  "entries_count": 0                // 仅 unlocked 时给出，否则 0
}
```

## 5. 对话触发（engine.send_message 内、LLM 调用之前拦截）

- 触发判定（本地、无 LLM）：文本含「密码」，且满足任一：
  - 含「保险箱」；
  - 匹配查询意图 `/密码.{0,6}(是什么|是多少|忘了|多少|帮我查|给我)/`；
  - 命中已解锁条目中任一 site/account 的子串（如「微博」）。
- 分支（按序）：
  1. active persona 未达 L4（`is_gate_open(level,"vault")==false`）→ 回复「亲密度达到 L4（贴心）后会解锁本地密码保险箱，继续和我聊天吧～」；`skill:"vault"`，不进 LLM。
  2. 命中且 `unset` → 回复引导「先在控制台·保险箱页初始化（设置主密码）」。
  3. 命中且 `locked` → 回复「保险箱已锁定，先在控制台解锁（主密码或系统解锁）后再问我吧」。
  4. 命中且 `unlocked` → 本地检索；找到 → `reply` 含明文（site/account/password），`secret: true` + `secret_entry_id`，前端据此自动调用 `vault_copy({id})` 完成复制；未找到 → 「没找到和「XX」相关的密码记录哦」。
- **明文处理**：`reply` 明文仅在当次返回给已解锁会话 UI；**写入 conversation 的是脱敏文本**（「已为你复制「微博」的密码，20 秒后自动清除」）；喇叭重读读 conversation，故明文永不发声。
- **剪贴板接缝（冻结）**：core 拦截不做剪贴板（无 OS 访问），只返回 `secret_entry_id`；**前端**在收到 `secret:true` 回复后调用 `invoke('vault_copy', { id: secret_entry_id })`（Tauri 层写系统剪贴板 + spawn 清除任务；mock 层 navigator.clipboard，失败则降级 toast + mock 剪贴板变量并记录）。语音朗读一律用 `speak_text`。
- `SendMessageOutput` 新增字段（均 `#[serde(skip_serializing_if = "Option::is_none")]`）：
  - `pub secret: Option<bool>`（拦截命中时为 true，前端据此触发 vault_copy）
  - `pub secret_entry_id: Option<String>`（供前端调用 vault_copy）
  - `pub speak_text: Option<String>`（拦截回复 = 脱敏文本；非拦截 None，前端沿用 reply）
- 拦截路径绝不调用 LLM（单测用 mock provider 断言未被调用）。

## 6. L4 门控接通点（3 处 kind: placeholder → active）

1. `qibai-core/src/intimacy.rs` `GATES` 中 `{ gate: "vault", ..., kind: "placeholder" }` → `"active"`。
2. `src/console.js` `INTIMACY_GATE_META` 中 vault 项 `kind: 'placeholder'` → `'active'`。
3. `src/tauri-bridge.js` `INTIMACY_GATES` 中 vault 项 `kind: 'placeholder'` → `'active'`。
判定仍以 `check_gate` / `vault_status.gated` 后端为准，前端只做展示。

## 7. 安全边界

- 代码内无硬编码密钥/默认密码（KDF 迭代常量、aad 常量除外）。
- 明文只在：core 内存（zeroize）、`vault_copy` 命令的 Tauri/mock 层瞬时、已解锁会话的当次 `reply`。
- 剪贴板清除计时：core 计算 `clear_after_secs`/过期判定（可单测），Tauri 层 spawn 清除任务、mock 层 `setTimeout`。
- LLM 无明文访问路径：拦截在 LLM 之前；工具表不含 vault 工具；conversation 不落明文。

## 8. 受限项（沙箱如实标注 + 替代验证）

| 项 | 沙箱状态 | 替代验证路径 |
|---|---|---|
| 系统钥匙串（Windows Credential Manager / macOS Keychain / Linux Secret Service） | 沙箱无对应系统服务 | core 的 key 字节接口单测 + Tauri 层代码走查 + 真机验证清单 |
| 真实剪贴板写/清（OS 级） | 沙箱无桌面会话 | core 计时逻辑单测 + mock 层 navigator.clipboard 演示 + 真机验证 |
| src-tauri 编译（缺 webkit2gtk 等系统库） | 不可编译 | 与冻结契约逐项对齐走查 + qibai-core 全量单测覆盖逻辑 |
| 浏览器 mock 冒烟 | 可做（chrome + 截图） | 冒烟页 + 截图证据 |

## 9. 新增单测清单（qibai-core，交付时逐项验证记录）

1. `crypto_roundtrip`：AES-256-GCM 加解密往返还原。
2. `wrong_password_rejected`：错误主密码不解锁。
3. `brute_force_cooldown`：连错 5 次 → 冷却 30s（冷却期内正确密码也拒）→ 冷却结束恢复。
4. `idle_lock_timeout`：注入时钟，闲置超时自动锁定；`touch` 刷新。
5. `clipboard_timer`：copy_hold 返回 clear_after_secs；过期判定正确。
6. `export_import_roundtrip`：导出 payload → 重导入 → 数据还原；错误备份密码拒绝。
7. `gate_block_before_l4`：对话拦截在 L4 前拒绝；L4 后放行。
8. `chat_intercept_no_llm`：mock LLM 断言未被调用；secret 回复的明文不入 conversation。
9. `keyring_key_path`：随机 key 解锁往返（模拟钥匙串字节）。
10. `no_plaintext_on_disk`：落盘文件字节不含明文子串。
11. `kdf_iterations_const`：常量 ≥ 210000；`kdf_210k_runtime_bench`：真实 210000 迭代耗时记录（宽松上限，防 CI 抖动）。
12. `entry_crud_search`：增删改查 + 中文/大小写检索。
13. `settings_vault_defaults`：vault 段默认值与持久化。
14. 既有 138 用例保持全绿。

---

## 10. 测试与验证结果（2026-10-22 交付记录）

> 本节为独立全链路验证 agent 的实测记录（未修改任何实现文件）；数字一律以实测为准。
> 完整证据链：`artifacts/VAULT_CORE_TEST_REPORT.md`（core 自测）、`artifacts/CODE_CHANGES_CORE.md` / `VAULT_FRONTEND_CHANGES.md`（变更说明）、`artifacts/VAULT_FRONTEND_CHECK.md`（前端走查）、`artifacts/VAULT_TEST_REPORT.md`（独立复核）。

### 10.1 core 全量测试

```text
cargo test -p qibai-core  →  test result: ok. 153 passed; 0 failed; 0 ignored; finished in 4.50s
```

- **153 passed / 0 failed** = 既有 138 全保持 + 迭代③新增 15；
- 新增用例即 §9 清单 1–13（11 个 vault + 2 个 engine）外补 1 个 `relock_after_save_keeps_data`（保存后重锁再解锁数据不丢）与 1 个 settings 兼容用例；
- **PBKDF2-210k 实测**：`cargo test kdf_210k_runtime_bench -- --nocapture` → `elapsed=1377 ms`（debug 未优化构建，宽松断言上限 8000ms；core 自测报告声称 1587ms，为运行间抖动，均在限内）；
- 独立最小程序直连复核：落盘 `vault.qbkv` 628B 不含密码 / site / account / 备注 / 主密码任一明文子串；错密码保持 locked；连错 5 次第 5 次起冷却 remaining 30s 且冷却期内正确密码也拒；导出信封 574B 含 `qibai-vault`；错备份密码导入结构化拒绝；正确备份导入重建条目完整还原。

### 10.2 浏览器 mock 走查

- **39/39 断言全部通过**（headless Chromium 146，CDP 驱动）；
- 本环境 WebCrypto `deriveKey(PBKDF2-SHA256, 210000)` 单次实测 ~25–27 ms，故全程直接对真实交付文件（迭代常量保持 210000）走完整状态机，未用降迭代副本；
- 覆盖：五态分发 / CRUD + 搜索 / 复制 20s 自动清（复制后立即含明文 → 21s 后清空实测通过）/ 导出导入三条路径（unset 重建 / locked 拒绝 / unlocked 合并后统一回 locked）/ 冷却倒计时 / L1→L6 门控翻转 / conversation 脱敏不落明文。

### 10.3 冒烟截图清单（`artifacts/vault-*.png`，9 张）

| 截图 | 断言 |
|---|---|
| `vault-gated-lock.png` | L1 `gated=true`：锁定横幅常显、操作区禁用 |
| `vault-unset-wizard.png` | L6 `gated=false`：初始化向导（主密码两次输入 + 钥匙串按钮） |
| `vault-locked-unlock.png` | `state=locked`：主密码解锁表单 + 系统解锁按钮 |
| `vault-unlocked-list.png` | 解锁后条目列表：密码一律 `••••••••` 掩码 |
| `vault-entry-crud.png` | 新增 2 条 → 编辑账号 → 两段式删除确认（toast「已更新」/「已删除」） |
| `vault-copy-toast.png` | 复制 toast「已复制，20 秒后自动清除」；21 秒后剪贴板复核为空串 |
| `vault-export-import.png` | 导出备份 toast + reset→unset→导入条目完整还原 |
| `vault-chat-secret.png` | 对话「我的微博密码是什么」→ 当次 UI 明文回复 + 自动复制 toast |
| `vault-cooldown.png` | 连错 5 次：冷却横幅剩余 29 秒 + 按钮置灰 |

### 10.4 受限项（沙箱如实标注 + 替代验证）

| 项 | 沙箱状态 | 替代验证 |
|---|---|---|
| 系统钥匙串（Win Credential Manager / macOS Keychain / Linux Secret Service） | 无系统服务 | core 32B key 字节接口 + `keyring_key_path` 单测；mock sessionStorage 暂存演示系统解锁路径；Tauri 层已接线（2026-10-22 确认） |
| OS 级剪贴板写 / 清 | 无桌面会话 | core `clipboard_timer` 单测；mock navigator.clipboard 降级 + setTimeout(20s) 实测通过；Tauri 层写/清已接线 |
| `src-tauri` 编译（缺 webkit2gtk 等） | 不可编译 | core 全量 153 单测覆盖逻辑；14 个 vault 命令注册 + `vault_tick` 闲置 tick + `qibai://vault-locked` 广播已按契约逐一对齐接线 |
| 浏览器 mock 冒烟 | 已完成 | CDP + 9 张截图证据（§10.3） |

### 10.5 真机验证清单（待装齐系统依赖后执行）

1. **钥匙串三平台**：Windows Credential Manager / macOS Keychain / Linux Secret Service 分别走 `vault_setup_keyring` → `vault_unlock_system`；杀应用重开后 key 仍在、免密解锁可用；错误 key 结构化拒绝；卸载后 key 清除行为确认；
2. **OS 剪贴板**：复制后肉眼核验系统剪贴板 = 明文密码、20s 后自动清空；切后台 / 锁屏期间计时仍生效；跨应用粘贴验证；
3. **`src-tauri` 真机**：装齐 webkit2gtk 依赖后 `cargo build`（src-tauri）；14 个 vault 命令与 core 同形联调；真实 IPC 端到端；导出备份落盘 `<data_dir>/vault-backup-<ts>.qbkv` 与 file input 导入读盘；`qibai://vault-locked` 广播触发前端锁定横幅刷新；
4. **对话链路**：配真实 `DOUBAO_API_KEY` 后，非拦截消息正常走 LLM、拦截消息绝不调 LLM 且明文不落盘（抓包 / conversation 文件核验）。
