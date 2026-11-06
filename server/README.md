# 栖伴 · 联网社交服务端（星球 × 同城）

迭代④独立 Node 服务端：HTTP REST + WebSocket + SQLite，单机可跑、零编译型原生依赖。
协议以 `docs/NETSOCIAL_SPEC.md` 为冻结契约，本目录实现其 §3（HTTP API）/ §4（WebSocket）/
§5（匹配算法）/ §6（审核占位）/ §9.1（服务端单测）。

## 运行环境

- Node.js ≥ 22.5（使用内置实验模块 `node:sqlite`，v22 下会打印 `ExperimentalWarning`，不影响功能）。
- 唯一 npm 依赖：`ws`（WebSocket）。

## 快速开始

```bash
cd server
npm install        # 仅安装 ws
npm start          # 默认监听 0.0.0.0:8787，数据落 ./data/
```

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `QIBAI_SERVER_HOST` | `0.0.0.0` | 监听地址；本机调试可设 `127.0.0.1` |
| `QIBAI_SERVER_PORT` | `8787` | 监听端口 |
| `QIBAI_DATA_DIR` | `<server>/data` | SQLite 数据目录（自动创建） |

## 数据文件

- `<data_dir>/qibai.db`：SQLite 主库（WAL 模式，旁有 `-wal`/`-shm` 临时文件，运行中存在属正常）。
- 五张表：`users` / `messages` / `likes` / `reports` / `blocks`（结构见规格 §2）。
- **不存明文 token**：`users.token_hash = sha256(token)`；token 仅在注册响应中出现一次，请客户端保存。
- 备份：停服后直接复制 `qibai.db`（连同 `-wal` 最好一并复制）即可。

## curl 冒烟示例

```bash
# 健康检查
curl -s http://127.0.0.1:8787/api/health
# => {"ok":true,"name":"qibai-net","version":"0.1.0","ts":...}

# 注册（返回 anon_id + token）
curl -s -X POST http://127.0.0.1:8787/api/register \
  -H 'Content-Type: application/json' \
  -d '{"nickname":"星尘","color_idx":3,"soul_tags":{"personality":["温柔"],"interests":["音乐"],"status":["想聊天"]},"city":"石家庄","age_declared":"18plus","pure_mode":false,"avatar":0}'
# => {"anon_id":"...","token":"<32hex>","profile":{...}}

# 登录态读取
curl -s "http://127.0.0.1:8787/api/me?token=<token>"

# 漫游 / 匹配 / 同城
curl -s "http://127.0.0.1:8787/api/souls?token=<token>"
curl -s "http://127.0.0.1:8787/api/match?token=<token>&limit=20"
curl -s "http://127.0.0.1:8787/api/city/%E7%9F%B3%E5%AE%B6%E5%BA%84/souls?token=<token>"

# 点亮 / 举报 / 拉黑
curl -s -X POST http://127.0.0.1:8787/api/souls/<other_anon_id>/like -H 'Content-Type: application/json' -d '{"token":"<token>"}'
curl -s -X POST http://127.0.0.1:8787/api/souls/<other_anon_id>/report -H 'Content-Type: application/json' -d '{"token":"<token>","reason":"垃圾信息"}'
curl -s -X POST http://127.0.0.1:8787/api/souls/<other_anon_id>/block  -H 'Content-Type: application/json' -d '{"token":"<token>"}'
```

## 双客户端联调示例（真实协议）

两个 Node 进程各持一个 token，WS 地址为 `ws://127.0.0.1:8787/ws?token=<token>`：

1. A、B 分别 `POST /api/register` 换 token；
2. 双方各自 `new WebSocket('ws://127.0.0.1:8787/ws?token=...')`，连上即收到
   `{"type":"hello","anon_id":...,"profile":...}`，随后可能收到 `{"type":"inbox","messages":[...]}`；
3. A 发 `{"type":"msg.send","to":"<B的anon_id>","body":"你好"}`：
   - A 收到回执 `{"type":"msg.deliver","mid":...,"from":"<A自己>","body":"你好"}`；
   - B 在线时实时收到 `{"type":"msg.deliver","mid":...,"from":"<A>","body":"你好"}`；
   - B 离线时消息入库（delivered=0），B 下次上线在 `inbox` 里补收；
4. 心跳：客户端每 30s 发 `{"type":"ping"}`，服务端回 `{"type":"pong"}`；**60s 无任何帧 → 服务端判离线并广播 presence**；
5. 拒收帧：对方拉黑你 → `{"type":"error","code":"blocked"}`；消息超长(>500字)或审核命中 → `{"type":"error","code":"censored"}`；token 无效 → error 帧后以 close code **4001** 关闭。

服务端日志区分每次 HTTP 请求（`[HTTP ...]`）与每条 WS 连接（`[WS ...] open/close/msg.send/...`），可直接复用为联调协议日志。

## CORS（file:// 页面直连）

所有 `/api/*` 响应附带 `Access-Control-Allow-Origin: *`；`OPTIONS` 预检请求直接回 `204`，
并带 `Access-Control-Allow-Methods: GET,POST,PUT,DELETE,OPTIONS` 与
`Access-Control-Allow-Headers: Content-Type`。WebSocket 握手不受同源策略限制，无需 CORS 配置。


## 单测

```bash
npm test    # node --test "src/test/*.test.js"，8 组 30 例
```

覆盖：注册字段校验/token 哈希存储、HTTP+WS 鉴权（含 4001）、匹配权重(3/2/1)+同城+5+在线+2+排除项+同分排序、
同城规范化与空城市、在线投递/离线补投/拉黑拒收/超长与审核拒收、举报落库与拉黑幂等及双向不可见、互亮 matched、health 与占位接口形状。

## 公网部署注意事项（本次不部署，留作后续）

- **反向代理 + WSS**：本服务只跑明文 HTTP/WS。生产用 Nginx 反代，并把 `ws://` 升级为 `wss://`：
  ```nginx
  server {
    listen 443 ssl;
    ssl_certificate     /path/fullchain.pem;
    ssl_certificate_key /path/privkey.pem;
    location / { proxy_pass http://127.0.0.1:8787; }
    location /ws {
      proxy_pass http://127.0.0.1:8787;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "Upgrade";
      proxy_read_timeout 300s;   # 长连接，避免被中间件提前掐断
    }
  }
  ```
- **HTTPS 证书**：由反向代理终结 TLS；客户端必须连 `wss://`。
- **SQLite 备份**：定时（如每日）停写副本或 `sqlite3 qibai.db ".backup ..."`；WAL 模式下直接复制主库 + wal 亦可。
- **审核接入点**：`src/moderation.js` 是唯一审核入口（词表种子 + 判定函数 `moderate()`）。
  生产应在此处接入第三方内容安全（如阿里云/网易易盾文本审核），HTTP `/api/moderation/check` 与
  WS `msg.send` 投递前共用同一函数，无需改动调用点。
- **未成年人保护**：注册时要求年龄声明（`age_declared`: `18plus`/`minor`）；未成年人走纯净模式
  （`pure_mode=1`，列表只展示昵称+光点色）。前端需弹隐私提示并二次确认；服务端已落库该标志。
- **进程守护**：建议 pm2 / systemd 托管；多实例注意 SQLite 单写者特性——本服务定位单机小规模，
  水平扩展需换 PostgreSQL 或前置共享缓存。

## 合规说明

- **匿名默认**：不采集真实身份；用户以服务端生成 UUIDv4（`anon_id`）+ 自定义昵称进入；不采集 GPS，
  同城仅基于用户自填城市字符串（城市级）。
- **举报/拉黑**：`/api/souls/:id/report` 记录举报原因；`/api/souls/:id/block` 双向屏蔽
  （任一方向存在即双方互不可见、消息拒收）。
- **隐私政策示例文案**（前端弹窗可直接采用）：

  > 我们默认您以匿名身份使用社交功能。系统不会收集您的真实姓名、手机号或精确地理位置；
  > "同城"仅使用您主动填写的城市名。聊天记录仅保存在您本机（加密缓存）与本机服务器数据库中，
  > 用于消息送达。如遇到骚扰，请使用"举报"或"拉黑"，我们会尽快处理。
  > 若您是未成年人，请开启"未成年人纯净模式"，该模式下陌生人的标签与城市将被隐藏。

- 部署公网前请替换为正式备案的隐私政策与用户协议，并按所在地法规完成必要的备案/审核流程。
