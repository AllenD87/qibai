# 元气小小 Qibai — 迭代④ 联网社交规格（星球 × 同城）

> 本文件是迭代④的**冻结契约**：服务端（`server/`）、客户端核心（`qibai-core/src/netsocial.rs`）、
> 前端（`src/console.html|css|js`、`src/tauri-bridge.js`）三路并行实现均以本文为准。
> 任何字段名、协议消息、命令名、状态机迁移、评分规则如有歧义，以本文为最终裁决。
> 关联文档：`docs/IPC_CONTRACT.md`（IPC 契约总表，本迭代在其追加新命令节）。

---

## 0. 设计原则（不可违背）

- **本地优先**：客户端社交模块默认离线。不配置服务端地址时，本地社交（v8 多角色）完整可用；配置 `server_url` 后才启用联网。
- **匿名优先**：用户以服务端生成的匿名 ID（`anon_id`，UUIDv4）进入，配自定义昵称；不采集、不展示、不存储任何真实身份信息。头像沿用现有 5 形象之一（`avatar` 0-4）或占位。
- **位置隐私**：只使用用户自填城市字符串（城市级），**绝不**采集/上传 GPS 坐标；同城 = 同城市分组。
- **合规底线**：匿名默认；举报/拉黑/屏蔽；内容审核接口占位；年龄声明 + 未成年人纯净模式；隐私政策文案（前端展示 + 服务端 README 含示例）。
- **脱敏显示**：聊天窗口只显示「昵称 + 光点色」，不显示真实 ID 原文；本地缓存只存匿名 ID 的哈希（`peer_key`），不存原始 `anon_id` 明文。

---

## 1. 总体架构

```
server/                  Node 22+ 服务端（独立目录，不进 Cargo workspace）
  ├─ package.json        (deps: ws；node:sqlite 为 Node 22 内置实验模块，零原生依赖)
  ├─ src/                server.js(入口+HTTP路由) / ws.js(WebSocket) / store.js(SQLite) /
  │                      match.js(匹配+同城) / moderation.js(审核占位) / test/ 单测
  └─ README.md           本地部署说明（可配置端口/数据目录、环境变量、公网部署注意事项）

qibai-core/src/netsocial.rs   客户端核心：状态机 / 脱敏 / 加密缓存 / 匿名ID / 网络客户端
qibai-core/src/engine.rs      IPC 门面新增 netsocial_* 命令（薄封装）
qibai-core/src/skills.rs      新增 open_social 技能（只读安全、默认启用）
src/console.html|css|js       控制台新增「社交」tab（星球广场 / 同城灯塔）
src/tauri-bridge.js           Tauri 环境走 invoke；浏览器 mock 模式下 netsocial 命令 JS 直连 localhost 服务端（真实协议）
```

- 服务端绑定：`0.0.0.0`（默认）或 `127.0.0.1`（环境变量 `QIBAI_SERVER_HOST`）；端口默认 `8787`（`QIBAI_SERVER_PORT`）；数据目录默认 `<server>/data/`（`QIBAI_DATA_DIR`）。
- 客户端浏览器 mock 直连地址：`http://127.0.0.1:8787`（可配置，见 §8）。

---

## 2. SQLite 数据模型（`store.js` 建表）

```sql
CREATE TABLE IF NOT EXISTS users (
  anon_id    TEXT PRIMARY KEY,        -- 服务端生成 UUIDv4
  token_hash TEXT NOT NULL,           -- sha256(token)，不存明文 token
  nickname   TEXT NOT NULL,
  color_idx  INTEGER NOT NULL DEFAULT 0,   -- 光点色 0-11
  soul_tags  TEXT NOT NULL DEFAULT '{}',   -- JSON: {"personality":[],"interests":[],"status":[]}
  city       TEXT NOT NULL DEFAULT '',
  age_declared TEXT NOT NULL DEFAULT '',   -- '18plus' | 'minor'（未成年人纯净模式）
  pure_mode  INTEGER NOT NULL DEFAULT 0,   -- 1=未成年人纯净模式
  avatar     INTEGER NOT NULL DEFAULT 0,   -- 0-4 现有 5 形象；占位可 -1
  created_at INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  status     TEXT NOT NULL DEFAULT 'offline'  -- 'offline' | 'online'
);
CREATE TABLE IF NOT EXISTS messages (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  sender    TEXT NOT NULL,             -- anon_id
  receiver  TEXT NOT NULL,
  body      TEXT NOT NULL,
  ts        INTEGER NOT NULL,
  delivered INTEGER NOT NULL DEFAULT 1 -- 0=离线投递(收件箱未读)
);
CREATE TABLE IF NOT EXISTS likes (
  liker  TEXT NOT NULL,
  liked  TEXT NOT NULL,
  ts     INTEGER NOT NULL,
  PRIMARY KEY (liker, liked)
);
CREATE TABLE IF NOT EXISTS reports (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  reporter TEXT NOT NULL,
  target   TEXT NOT NULL,
  reason   TEXT NOT NULL,
  ts       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS blocks (
  blocker TEXT NOT NULL,
  blocked TEXT NOT NULL,
  ts      INTEGER NOT NULL,
  PRIMARY KEY (blocker, blocked)
);
```

- 规范化城市：`city = String(city).trim()`，同城匹配用**规范化后完全相等**（不做地理编码，不做省后缀剥离）。
- 拉黑为**双向屏蔽**：`blocks` 表任一方向存在即双方互相不可见、消息拒绝。
- 举报不阻断通信，仅记录；`moderation` 占位见 §6。

---

## 3. HTTP API（`server.js`，JSON，统一前缀 `/api`）

所有请求体/响应均为 UTF-8 JSON。错误统一 `{ "error": "<code>", "message": "<中文说明>" }`，HTTP 状态 4xx/5xx。

| 方法 | 路径 | 请求 | 成功响应 |
|---|---|---|---|
| GET | `/api/health` | — | `{ "ok": true, "name": "qibai-net", "version": "0.1.0", "ts": <unix> }` |
| POST | `/api/register` | `{ nickname, color_idx, soul_tags, city, age_declared, pure_mode, avatar }` | `{ "anon_id", "token", "profile" }`（profile 见下） |
| GET | `/api/me?token=` | — | `{ "profile" }`（token 无效 → 401 `invalid_token`） |
| PUT | `/api/me` | `{ token, nickname?, color_idx?, soul_tags?, city?, avatar? }` | `{ "profile" }` |
| GET | `/api/souls` | `?token=&tag=&city=&online=&q=` | `{ "souls": [profile...] }`（漫游列表，见 §4.1） |
| GET | `/api/souls/:id` | `?token=` | `{ "soul": profile }`（不存在→404；已拉黑→403 `blocked`） |
| POST | `/api/souls/:id/like` | `{ token }` | `{ "liked": true, "matched": bool, "mutual": bool }`（matched=对方已点亮过你） |
| POST | `/api/souls/:id/report` | `{ token, reason }` | `{ "ok": true }` |
| POST | `/api/souls/:id/block` | `{ token }` | `{ "ok": true, "blocked": bool }` |
| GET | `/api/match` | `?token=&limit=` | `{ "souls": [ { "profile", "score" } ...] }`（§5 评分排序） |
| GET | `/api/city/:city/souls` | `?token=` | `{ "souls": [profile...], "city": "<规范化>" }`（同城，按 last_seen 降序） |
| GET | `/api/topics` | — | `{ "topics": [ { "id", "title", "sticky", "ts" } ] }`（话题墙占位，至少 3 条示例） |
| GET | `/api/events` | — | `{ "events": [ { "id", "title", "when", "where", "ts" } ] }`（活动预告占位，至少 2 条示例） |
| POST | `/api/moderation/check` | `{ token, text }` | `{ "ok": true, "flagged": bool, "reason": "" }`（占位实现，见 §6） |

**profile 形状**（`soul` 中每个元素同形）：

```jsonc
{
  "anon_id": "uuidv4",        // 漫游/匹配/同城列表中展示用（前端本地会哈希化，见 §7 脱敏）
  "nickname": "星尘",          // 用户自定义，1-12 字符，非法字符剔除
  "color_idx": 3,             // 光点色 0-11
  "soul_tags": { "personality": ["温柔"], "interests": ["音乐"], "status": ["想聊天"] },
  "city": "石家庄",
  "avatar": 0,                // 0-4
  "pure_mode": false,
  "online": true,
  "last_seen": 1730000000,
  "mutual_liked": false       // 仅在 souls/city/match 列表中：对方是否已点亮我
}
```

**注册规则**：`nickname` 去首尾空白，空则默认「星客」；`color_idx` clamp 0-11；`soul_tags` 三组各最多 5 个标签、单标签 ≤ 8 字符；`city` 空串允许；`age_declared` 缺省 `'18plus'`；`pure_mode` 布尔。`token` 为服务端生成的随机 32 hex，客户端保存。

---

## 4. WebSocket（`ws.js`，路径 `/ws?token=`）

### 4.1 连接与鉴权
- 连接即鉴权：token 无效 → 发 `{ "type": "error", "code": "invalid_token" }` 并关闭（code 4001）。
- 成功：`status=online`、更新 `last_seen`；向该连接发送 `{ "type": "hello", "anon_id", "profile" }`；随后发送离线收件箱 `{ "type": "inbox", "messages": [msg...] }`（delivered=0 的消息，逐条置 delivered=1）；向其他在线用户广播 presence（见下）。

### 4.2 消息类型（服务端→客户端）
```jsonc
{ "type": "msg.deliver", "mid": 12, "from": "<anon_id>", "body": "<text>", "ts": 1730000000 }
{ "type": "presence", "who": "<anon_id>", "online": true, "ts": 1730000000 }
{ "type": "error", "code": "blocked", "message": "对方已将你拉黑，消息未送达" }
{ "type": "pong", "ts": 1730000000 }
```

### 4.3 消息类型（客户端→服务端）
```jsonc
{ "type": "msg.send", "to": "<anon_id>", "body": "<text>" }   // 服务端存库后投递
{ "type": "ping", "ts": 1730000000 }                           // 心跳
```

### 4.4 路由与投递规则
- `msg.send` 校验：`to` 存在；发送方与接收方未互相拉黑（任一方向存在 block → 拒收，`{type:"error",code:"blocked"}`）；`body` 去首尾空白、≤ 500 字符、经 `moderation` 占位检查（flagged 且拒绝投递 → `{type:"error",code:"censored"}`）。
- 投递：接收方在线 → 实时推送 `msg.deliver` 且 `delivered=1`；离线 → 入库 `delivered=0`，上线时经 `inbox` 补投。
- 发送方无论对方是否在线都收到回执 `{ "type": "msg.deliver", "mid", "from": "<自己的 anon_id>", "body", "ts" }`（前端用 `from==自己` 判定方向为「我发出」）。
- 心跳：服务端每 60s 未收到客户端任何帧则判定离线（更新 status/last_seen，广播 presence）；客户端每 30s 发 `ping`，服务端回 `pong`。
- 消息不持久化到磁盘之外的任何位置；SQLite 仅存 `messages` 表（联调本地可用；生产部署建议加密，见 server/README.md）。

---

## 5. 匹配算法（`match.js`，服务端实现 + 单测）

输入：当前用户 + 候选池（排除自己、被拉黑、被举报过、举报过的人）。评分：

```
score = 同组标签交集加权和：
  personality 交集每个 × 3
  interests   交集每个 × 2
  status      交集每个 × 1
        + 同城(规范化相等) +5
        + 对方在线 +2
```

排序：score 降序；同分按 `last_seen` 降序。`limit` 默认 20、上限 50。
标签比较：忽略大小写、trim 后按字符串相等（去重后集合求交）。

---

## 6. 内容审核占位（`moderation.js`）

`POST /api/moderation/check`：内置极小敏感词表（如若干涉黄涉政暴力词），命中 → `{ flagged: true, reason: "<类别>" }`；未命中 → `{ flagged: false }`。词表与判定逻辑集中一处，单测覆盖「命中/未命中/空文本/超长」。消息发送路径在投递前调用同一函数。**占位实现**：接口与调用点就绪，词表为种子示例，生产可扩展第三方审核（README 说明）。

---

## 7. 客户端核心（`qibai-core/src/netsocial.rs`）

### 7.1 状态机

```
Unregistered ──netsocial_register──▶ Registered(已生成 anon_id+昵称，未配置/未连接)
Registered ──netsocial_connect(server_url)──▶ Online
Online ──netsocial_disconnect / 网络断──▶ Offline（保留配置，可重连）
Registered/Offline ──netsocial_register(重设 profile)──▶ Registered
```

- `anon_id`：客户端本地生成 UUIDv4（复用 `rand` crate）。**首次注册**：若配置了 `server_url`，调 `POST /api/register` 换取 token 并持久化；未配置时仅本地保存（离线占位，配置后补注册）。
- 状态字段：`state`（unregistered/registered/online/offline）、`anon_id`、`nickname`、`color_idx`、`soul_tags`、`city`、`avatar`、`server_url`、`registered`（服务端是否已注册）、`last_error`。

### 7.2 脱敏规则（单测覆盖）

- 缓存联系人：只存 `peer_key = sha256(anon_id)` 前 12 位 hex + `peer_nick` + `peer_color` + `peer_online` + `last_seen`。**不存** 原始 `anon_id` 明文。
- 缓存消息：只存 `{ peer_key, text, ts, dir }`（dir=`in`|`out`）。不存对方原始 ID。
- 聊天窗口数据：前端只拿 `{ peer_key, peer_nick, peer_color, text, ts, dir }`。
- 自己的 `anon_id` 仅存于加密缓存内（见 7.3），前端展示用昵称+光点色。

### 7.3 加密缓存（单测覆盖）

- 文件：`<data_dir>/netsocial.json`（AES-256-GCM 加密，复用迭代③ `vault.rs` 的 aes-gcm/pbkdf2/rand 依赖）。
- 密钥：`<data_dir>/netsocial.key`（首次生成随机 32 字节，权限 600）。缓存内容：`{ anon_id, token, nickname, color_idx, soul_tags, city, avatar, server_url, contacts:[peer_key, peer_nick, peer_color, peer_online, last_seen], messages:[peer_key,text,ts,dir] }`。
- 说明：`anon_id`/`token` 是匿名假名凭证，AES 加密防止随手查看；不依赖用户口令（避免打扰）。文档标注生产建议可升级为 vault 主口令加密。
- `netsocial_cache_clear()` 删除两文件。

### 7.4 网络层

- HTTP：`reqwest`（已有依赖）。WS：新增 `tokio-tungstenite` + `tokio` features 补 `net`、`io-util`、`rt-multi-thread`。
- 收消息 → 写缓存（脱敏）+ 入**待消费事件队列**（环形，上限 200，防爆）。
- 事件类型（供 `netsocial_poll` 消费）：`connected` / `disconnected{reason}` / `hello{profile}` / `inbox{count}` / `msg{peer_key,peer_nick,peer_color,text,ts,dir}` / `presence{peer_key,online}` / `error{code,message}`。
- 断线自动重连：指数退避 1s/2s/4s/8s，上限 8s；`netsocial_disconnect` 可停止。
- 线程模型：连接任务 `tokio::spawn`；core 为同步接口（现有 engine 模式），网络任务与状态通过内部锁/通道桥接（实现者可选用 `tokio::sync::mpsc` + 轮询 drain，保持同步 facade）。

### 7.5 engine.rs 新增命令（全部返回 `serde_json::Value`，错误 `EngineError`）

| 命令 | 参数 | 返回 |
|---|---|---|
| `netsocial_status` | — | `{ state, anon_id, nickname, color_idx, city, server_url, online, registered, last_error }` |
| `netsocial_register` | `{ nickname?, color_idx?, soul_tags?, city?, avatar?, age_declared?, pure_mode? }` | `{ ok, anon_id, state, registered }`（已有身份时仅更新 profile） |
| `netsocial_connect` | `{ server_url? }` | `{ ok, state }`（无参时用已存 server_url；无 server_url 且无参 → 错误 `missing_server_url`） |
| `netsocial_disconnect` | — | `{ ok, state }` |
| `netsocial_set_profile` | `{ nickname?, color_idx?, soul_tags?, city?, avatar? }` | `{ ok }`（本地持久化 + 若在线 PUT /api/me） |
| `netsocial_roam` | `{ tag?, city?, online?, q? }` | `{ souls: [...] }`（原样透传服务端字段，供前端渲染） |
| `netsocial_match` | `{ limit? }` | `{ souls: [ { profile, score } ] }` |
| `netsocial_city_souls` | `{ city }` | `{ souls, city }` |
| `netsocial_like` | `{ id }` | `{ liked, matched, mutual }` |
| `netsocial_report` | `{ id, reason }` | `{ ok }` |
| `netsocial_block` | `{ id }` | `{ ok, blocked }` |
| `netsocial_send` | `{ id, text }` | `{ ok, mid? }`（离线/未连接 → 错误 `not_online`） |
| `netsocial_history` | `{ id? }` | `{ contacts: [...], messages: [...] }`（按 peer_key 或全部） |
| `netsocial_poll` | — | `{ events: [ { type, payload } ... ] }`（消费后清空；无事件返回空数组） |
| `netsocial_topics` | — | `{ topics: [...] }` |
| `netsocial_events` | — | `{ events: [...] }` |
| `netsocial_cache_clear` | — | `{ ok }` |

所有命令在未注册/未连接时按 §7.1 状态机给出结构化错误（`EngineError::Net`，message 中文）。

### 7.6 skills.rs 新增技能

- 常量 `SKILL_OPEN_SOCIAL: &str = "open_social"`，加入 `ALL_SKILLS`；`status()/set_enabled()/check()` 各加一支（只读安全技能，默认 `true`，无需审批，参照 `check_code`）。
- `invoke` 分支：参数 `{ view?: "planet"|"city" }` → 返回 `SkillOutcome::Ok("已为你打开星球广场…")`；engine 的 `send_message` 技能回填时在回复 `skill` 字段带上 `open_social`，前端收到后打开「社交」tab 并切换到对应视图（参照现有 guard/codecheck 的前端跳转链路）。
- 技能描述列表（供 LLM tool schema）追加 `open_social`，描述含触发意图：带我去星球逛逛 / 星球广场 / 去星球看看 / 同城看看 / 打开社交。

---

## 8. 前端（`src/console.*` + `src/tauri-bridge.js`）

### 8.1 社交 tab
- tab 按钮：`<button class="ctab" data-tab="social">社交</button>`（置于保险箱之后、猜拳之前）。
- 视图容器：`#social`（含子视图切换 `planet` / `city`，默认 `planet`）。
- **星球广场视图**：
  - 星空背景：纯 CSS（`radial-gradient` 多重背景 + 动画星点），光点用 SVG/div 定位，不新增美术素材。
  - 我的光点：昵称 + 光点色 + 在线状态；「灵魂标签」面板（性格/兴趣/状态三组多选标签，预置 3 组常用标签 + 自定义输入，保存调 `netsocial_set_profile`）。
  - 漫游浏览：`netsocial_roam` 渲染光点列表（卡片式，昵称+光点色+标签+城市+在线）；点击光点 → 详情浮层（「点亮」「举报」「拉黑」「发消息」）。
  - 点亮：`netsocial_like`，若 `matched=true` 显示「互相点亮」提示。
  - 匿名聊天窗口：弹层，标题=对方昵称+光点色圆点；消息气泡只显示昵称+色（脱敏）；输入框发送调 `netsocial_send`；轮询 `netsocial_poll`（2s）渲染新消息。
- **同城灯塔视图**：
  - 城市：文本输入 + 快捷城市（北京/上海/广州/深圳/成都/杭州/武汉/西安/石家庄）+ 保存；`netsocial_city_souls` 渲染同城灵魂列表。
  - 同城话题墙占位：从 `netsocial_topics` 渲染卡片（标注「话题墙 · 即将上线」）。
  - 活动预告占位：从 `netsocial_events` 渲染卡片（标注「活动预告 · 即将上线」）。
- **连接状态条**：顶部显示 离线（默认）/ 已连接 `server_url`；「连接/断开」按钮 + 服务器地址输入（默认 `http://127.0.0.1:8787`）；断开状态下社交页各视图显示「离线模式 · 配置服务器后启用联网」提示。
- **合规元素**：首次进入社交 tab 弹隐私提示（匿名默认、不采集位置、举报/拉黑机制、未成年人纯净模式开关 + 年龄声明确认）；设置区含「未成年人纯净模式」开关（开则隐藏漫游列表的陌生人标签展示？——简化为：纯净模式开启时漫游/同城列表只显示昵称+色，隐藏标签与城市；聊天仍可用）。

### 8.2 tauri-bridge.js（浏览器 mock 模式）
- 浏览器环境（无 `__TAURI__`）下，`netsocial_*` 命令在 mockInvoke 内**直连真实本地服务端**（fetch + WebSocket，协议字段与 §3/§4 完全一致），实现与 Rust core 相同语义（本地状态 + 缓存 localStorage 简化版）。
- mock 下 `netsocial_poll` 由 JS 侧维护事件队列（WebSocket 收消息入队）。
- 关键约束：mock 直连只作为浏览器冒烟/演示路径；Tauri 真机一律走 Rust core（invoke）。两条路径字段语义一致，冒烟记录注明 mock 直连 = 真实协议验证。

---

## 9. 测试矩阵（交付前逐项验证并记录）

### 9.1 服务端单测（Node，`server/src/test/*.test.js`，`node --test` 或自写断言 runner）
1. 注册：字段校验（昵称空/超长、color_idx clamp、soul_tags 上限）、token 生成与哈希存储。
2. 鉴权：`/api/me` 有效/无效 token；WS 无效 token 拒绝并关闭。
3. 匹配算法：标签交集权重分（personality×3/interests×2/status×1）、同城+5、在线+2、排除自己/拉黑/举报、同分按 last_seen。
4. 同城分组：规范化相等匹配；空城市不误匹配。
5. 消息路由：在线投递 / 离线收件箱补投 / 拉黑拒收（双向）/ 超长拒收 / moderation 命中拒收。
6. 举报/拉黑：记录落库、重复幂等、拉黑后双方不可见（souls 列表与详情 403）。
7. 点亮：互亮判定 matched。
8. health/占位接口（topics/events/moderation）形状正确。

### 9.2 客户端 core 单测（Rust，`qibai-core/src/netsocial.rs` 内嵌 `#[cfg(test)]`）
1. 状态机迁移：Unregistered→Registered→Online→Offline 全迁移 + 非法迁移拒绝。
2. 脱敏：`peer_key` 为 sha256 截断 12 hex、缓存与历史不含原始 anon_id 明文、联系人去重。
3. 加密缓存：首次生成 key、加解密往返、篡改密钥/密文报错、cache_clear 删除。
4. 事件队列：环形上限 200、poll 消费清空。
5. 技能：`open_social` 白名单/默认启用/invoke 返回（补进 skills.rs 既有测试）。
6. **回归**：既有 153 项全过（`cargo test -p qibai-core`）。

### 9.3 联调冒烟（真实协议）
- localhost 起服务端（`node server/src/server.js`）+ **两个客户端进程**（Node 脚本，各自独立 token）连入：
  a. 双客户端注册 → `GET /api/me` 登录态；
  b. 双方标签相近 → `/api/match` 出现对方且排序合理；
  c. A→B 聊天（B 在线实时收到；B 下线 → A 再发 → B 上线收 inbox 补投）；
  d. 同城可见：设置相同城市 → `/api/city/:city/souls` 可见；
  e. B 拉黑 A → A 再发消息被拒（error blocked）、A 的 souls 列表不再含 B；
  f. A 举报 B → 记录落库。
- 记录：完整协议日志（`artifacts/netsocial-sim-log.txt`）+ 冒烟结论写入 `SMOKE_TEST.md`。
- 若 qibai-core 网络层（§7.4）已就绪，追加一次 Rust 客户端真实连接冒烟（示例 bin 或集成测试），结果同样记录。

### 9.4 浏览器 mock 冒烟（截图证据，`artifacts/`）
1. `console.html` 社交 tab 星球广场（星空视图 + 我的光点 + 标签面板）。
2. 漫游列表 + 光点详情浮层 + 点亮（互相点亮提示）。
3. 匿名聊天窗口（双客户端各开一个浏览器，A/B 互聊，气泡只显示昵称+色）。
4. 同城灯塔（城市选择 + 同城列表 + 话题墙 + 活动预告）。
5. 隐私提示 + 纯净模式开关。
每张截图标注 `artifacts/net-*.png`。

---

## 10. 沙箱受限项与替代验证路径（如实记录，不隐瞒）

| 受限项 | 影响 | 替代验证 |
|---|---|---|
| src-tauri 无法编译（缺 webkit2gtk/gtk3 系统库，禁 sudo/apt） | Tauri 真机命令接线无法真机验证 | 命令在 qibai-core/engine 层单测覆盖；main.rs 接线代码层面完成 + 文档标注「待真机验证」 |
| 沙箱无 GUI 桌面环境跑 Tauri 双窗口 | 真实双窗口联网社交无法演示 | 浏览器 mock 直连真实服务端（真实协议 + 真实 UI 渲染）替代 |
| node:sqlite 为 Node 22 实验模块 | 有 ExperimentalWarning（不影响功能） | 单测与冒烟实际跑通即证明可用；README 注明需 Node ≥ 22.5 |
| 公网部署 | 本次不部署公网 | 部署说明写入 server/README.md，托管方案由用户后续决策 |

---

## 11. 交付物清单

1. `server/`（可运行服务端 + 单测 + `server/README.md` 本地部署与公网注意事项）。
2. `qibai-core/src/netsocial.rs` + `engine.rs` 新命令 + `skills.rs` 新技能 + `lib.rs`/`Cargo.toml` 接线（153 基线全过 + 新增单测）。
3. `src/console.html|css|js` 社交 tab + `src/tauri-bridge.js` mock 直连。
4. `docs/NETSOCIAL_SPEC.md`（本文件）、`docs/IPC_CONTRACT.md` 追加新命令节、`README.md`/`CHECKLIST.md`/`SMOKE_TEST.md` 更新。
5. `artifacts/netsocial-sim-log.txt`（协议联调日志）+ `artifacts/net-*.png`（UI 冒烟截图）。
6. `元气小小_MVP_源码包.zip`（排除 target，复制到 `/home/user/Doubao/chats/38448289926008322/元气小小_MVP_源码包.zip`）。
