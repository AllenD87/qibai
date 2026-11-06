//! 联网社交客户端核心（迭代④，冻结契约 NETSOCIAL_SPEC §7）。
//!
//! - §7.1 状态机：`Unregistered → Registered ⇄ Online / Offline`，非法迁移拒绝。
//! - §7.2 脱敏：`peer_key = sha256(anon_id)` 前 12 hex；联系人 / 消息缓存永不落原始
//!   `anon_id` 明文；聊天事件只带 `peer_key/nick/color/text/ts/dir`。
//! - §7.3 加密缓存：`<data_dir>/netsocial.json`（AES-256-GCM），密钥
//!   `<data_dir>/netsocial.key`（随机 32B，权限 600）；篡改密钥 / 密文报错。
//! - §7.4 网络层：HTTP 用 reqwest；WS 用 tokio-tungstenite；收消息 → 脱敏写缓存 →
//!   环形事件队列（上限 200，`netsocial_poll` 消费清空）；断线指数退避
//!   1s/2s/4s/8s 重连；连接任务 `tokio::spawn`，core 对外保持同步/薄异步 facade。
//!
//! 匿名优先：本模块不采集、不展示、不缓存任何真实身份字段；`anon_id` 为 UUIDv4
//! 假名凭证（离线模式本地生成；联网注册后以服务端下发的 UUIDv4 为准）。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

const CACHE_FILE: &str = "netsocial.json";
const KEY_FILE: &str = "netsocial.key";
const CACHE_AAD: &[u8] = b"qibai-netsocial-v1";
const EVENT_CAP: usize = 200;
const BACKOFF_SECS: [u64; 4] = [1, 2, 4, 8];
const HEARTBEAT_SECS: u64 = 30;
/// 联系人缓存上限（防爆）。
const CONTACT_CAP: usize = 500;
/// 消息缓存上限（环形裁剪）。
const MESSAGE_CAP: usize = 2000;

// ---- 错误（EngineError::Net 包装；message 中文） ----

#[derive(Debug, Error)]
pub enum NetError {
    #[error("未注册：请先调用 netsocial_register 创建匿名身份")]
    NotRegistered,
    #[error("未连接：请先调用 netsocial_connect 连接社交服务器")]
    NotOnline,
    #[error("非法的状态迁移：{0}")]
    InvalidTransition(String),
    #[error("缺少服务器地址：请传入 server_url 或先配置已存地址")]
    MissingServerUrl,
    #[error("网络错误：{0}")]
    Network(String),
    #[error("加密错误：{0}")]
    Crypto(String),
    #[error("文件错误：{0}")]
    Io(String),
    #[error("协议错误：{0}")]
    Protocol(String),
}

// ---- §7.1 状态机 ----

/// 联网社交客户端状态机（§7.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetState {
    /// 未注册（无匿名身份）。
    Unregistered,
    /// 已注册（本地匿名身份就绪，未配置 / 未连接服务器）。
    Registered,
    /// 已连接（WS 握手上行）。
    Online,
    /// 曾经连接过，当前断开（保留配置，可重连）。
    Offline,
}

impl NetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            NetState::Unregistered => "unregistered",
            NetState::Registered => "registered",
            NetState::Online => "online",
            NetState::Offline => "offline",
        }
    }
}

// ---- 数据结构 ----

/// 灵魂标签（三组各最多 5 个标签，单标签 ≤ 8 字符；§3 注册规则）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoulTags {
    #[serde(default)]
    pub personality: Vec<String>,
    #[serde(default)]
    pub interests: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
}

/// 脱敏联系人记录（§7.2：只存 peer_key，不存原始 anon_id）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactRecord {
    pub peer_key: String,
    pub peer_nick: String,
    pub peer_color: u8,
    pub peer_online: bool,
    pub last_seen: u64,
}

/// 脱敏消息记录（§7.2：只存 peer_key + text + ts + dir）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRecord {
    pub peer_key: String,
    pub text: String,
    pub ts: u64,
    /// `in` | `out`
    pub dir: String,
}

/// 加密缓存内容（§7.3）。`anon_id`/`token` 为匿名假名凭证，AES 加密防止随手查看。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct PersistedCache {
    #[serde(default)]
    anon_id: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    color_idx: u8,
    #[serde(default)]
    soul_tags: SoulTags,
    #[serde(default)]
    city: String,
    #[serde(default)]
    avatar: i32,
    #[serde(default)]
    age_declared: String,
    #[serde(default)]
    pure_mode: bool,
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    server_registered: bool,
    #[serde(default)]
    contacts: Vec<ContactRecord>,
    #[serde(default)]
    messages: Vec<MessageRecord>,
}

/// 运行时内层状态（Arc<Mutex> 与后台 WS 任务共享）。
struct Inner {
    state: NetState,
    last_error: Option<String>,
    cache: PersistedCache,
    /// 待消费事件环形队列（上限 200，§7.4）。
    events: VecDeque<serde_json::Value>,
    /// 后台帧处理写脏了缓存（如 hello 下发服务端 anon_id 回写），待 flush 落盘。
    dirty: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            state: NetState::Unregistered,
            last_error: None,
            cache: PersistedCache::default(),
            events: VecDeque::with_capacity(EVENT_CAP),
            dirty: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    nonce_b64: String,
    ciphertext_b64: String,
}

/// 后台 WS 任务指令。
enum WsCmd {
    /// 发送一条聊天消息（to = 对方服务端 anon_id，仅在 WS 帧中使用，不落盘）。
    SendMsg { to: String, body: String },
    /// 停止连接任务（netsocial_disconnect）。
    Shutdown,
}

// ---- §7.2 脱敏 ----

/// `peer_key = sha256(anon_id)` 前 12 位 hex（§7.2）。
pub fn peer_key(anon_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(anon_id.as_bytes());
    let digest = h.finalize();
    digest
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---- WebSocket 传输抽象（生产 = tokio-tungstenite；测试可注入 mock） ----

#[async_trait]
pub trait WsStream: Send + Sync {
    /// 读取下一帧文本；连接关闭 / 出错返回 Err。
    async fn recv(&mut self) -> Result<String, NetError>;
    /// 写入一帧文本。
    async fn send(&mut self, text: String) -> Result<(), NetError>;
    /// 关闭连接。
    async fn close(&mut self) -> Result<(), NetError>;
}

#[async_trait]
pub trait WsConnector: Send + Sync {
    /// 完成 WebSocket 握手；成功后调用方接管读写循环。
    async fn connect(&self, ws_url: &str, token: &str) -> Result<Box<dyn WsStream>, NetError>;
}

/// HTTP 传输抽象：生产 = reqwest；测试可注入 mock（单测不触网）。
#[async_trait]
pub trait HttpApi: Send + Sync {
    async fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, NetError>;
}

/// 生产 HTTP 实现：reqwest。
#[derive(Default)]
pub struct RealHttp {
    client: reqwest::Client,
}

#[async_trait]
impl HttpApi for RealHttp {
    async fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, NetError> {
        let resp = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            _ => return Err(NetError::Protocol(format!("未知 HTTP 方法：{method}"))),
        };
        let resp = match body {
            Some(b) => resp.json(b),
            None => resp,
        };
        let resp = resp
            .send()
            .await
            .map_err(|e| NetError::Network(e.to_string()))?;
        let status = resp.status();
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| NetError::Protocol(format!("响应不是合法 JSON：{e}")))?;
        if !status.is_success() {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("服务器返回错误");
            let code = v.get("error").and_then(|c| c.as_str()).unwrap_or("http_error");
            return Err(NetError::Network(format!("{code}: {msg}")));
        }
        Ok(v)
    }
}

/// 生产连接器：tokio-tungstenite。
#[derive(Default)]
pub struct TungsteniteConnector;

#[async_trait]
impl WsConnector for TungsteniteConnector {
    async fn connect(&self, ws_url: &str, token: &str) -> Result<Box<dyn WsStream>, NetError> {
        let url = format!("{ws_url}?token={token}");
        let (stream, _resp) =
            tokio_tungstenite::connect_async(&url)
                .await
                .map_err(|e| NetError::Network(format!("WebSocket 握手失败：{e}")))?;
        Ok(Box::new(TungsteniteStream { inner: stream }))
    }
}

struct TungsteniteStream {
    inner: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

#[async_trait]
impl WsStream for TungsteniteStream {
    async fn recv(&mut self) -> Result<String, NetError> {
        use futures_util::StreamExt;
        loop {
            match self.inner.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => {
                    return Ok(t.to_string())
                }
                // Binary / Ping / Pong / Frame 内部帧：忽略，继续读。
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(NetError::Network(e.to_string())),
                None => return Err(NetError::Network("连接已断开".to_string())),
            }
        }
    }

    async fn send(&mut self, text: String) -> Result<(), NetError> {
        use futures_util::SinkExt;
        self.inner
            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
            .await
            .map_err(|e| NetError::Network(e.to_string()))
    }

    async fn close(&mut self) -> Result<(), NetError> {
        self.inner
            .close(None)
            .await
            .map_err(|e| NetError::Network(e.to_string()))
    }
}

fn ws_url_of(http_url: &str) -> String {
    if let Some(rest) = http_url.strip_prefix("https://") {
        format!("wss://{rest}/ws")
    } else if let Some(rest) = http_url.strip_prefix("http://") {
        format!("ws://{rest}/ws")
    } else {
        format!("ws://{http_url}/ws")
    }
}

// ---- 管理器 ----

/// 联网社交客户端核心管理器（§7）。
pub struct NetsocialManager {
    /// 后台会话循环回写自身用的自引用（OnceLock 仅在构造时写入一次）。
    self_ref: std::sync::OnceLock<Arc<NetsocialManager>>,
    inner: Arc<Mutex<Inner>>,
    data_dir: PathBuf,
    http: Arc<dyn HttpApi>,
    connector: Arc<dyn WsConnector>,
    cmd_tx: Mutex<Option<UnboundedSender<WsCmd>>>,
    /// 启动时恢复加密缓存失败（篡改）的记录，供 status 暴露。
    load_error: Mutex<Option<String>>,
}

impl NetsocialManager {
    /// 生产构造：数据目录 = 平台应用数据目录（测试可用 QIBAI_DATA_DIR 覆盖）。
    pub fn new() -> Arc<Self> {
        let dir = crate::settings::data_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::with_transports(&dir, Arc::new(TungsteniteConnector), Arc::new(RealHttp::default()))
    }

    /// 测试 / 联调用：指定数据目录 + 自定义双传输（WS + HTTP）。
    pub fn with_transports(
        dir: &Path,
        connector: Arc<dyn WsConnector>,
        http: Arc<dyn HttpApi>,
    ) -> Arc<Self> {
        let mgr = Arc::new(Self {
            self_ref: std::sync::OnceLock::new(),
            inner: Arc::new(Mutex::new(Inner::default())),
            data_dir: dir.to_path_buf(),
            http,
            connector,
            cmd_tx: Mutex::new(None),
            load_error: Mutex::new(None),
        });
        // 恢复加密缓存；文件不存在 = 全新；存在但解密失败 = 篡改（记录，不 panic）。
        match mgr.restore() {
            Ok(()) => {}
            Err(e) => {
                *mgr.load_error.lock().unwrap() = Some(e.to_string());
                mgr.inner.lock().unwrap().last_error = Some(e.to_string());
            }
        }
        // 自引用：后台会话循环据此回写缓存（hello 服务端 anon_id 等）。
        let _ = mgr.self_ref.set(mgr.clone());
        mgr
    }

    /// 测试便捷构造：内存 mock 双传输（WS 握手立即成功；HTTP register 返回假 token）。
    #[cfg(test)]
    fn for_test(dir: &Path) -> Arc<Self> {
        Self::with_transports(dir, Arc::new(MockConnector), Arc::new(MockHttp))
    }

    /// 后台帧处理写脏后落盘（hello 回写服务端 anon_id 等）。
    pub fn flush_if_dirty(&self) {
        let dirty = self.inner.lock().unwrap().dirty;
        if dirty && self.persist().is_ok() {
            self.inner.lock().unwrap().dirty = false;
        }
    }

    pub fn state(&self) -> NetState {
        self.inner.lock().unwrap().state
    }

    pub fn anon_id(&self) -> Option<String> {
        self.inner.lock().unwrap().cache.anon_id.clone()
    }

    // ---- §7.3 加密缓存原语 ----

    fn cache_path(&self) -> PathBuf {
        self.data_dir.join(CACHE_FILE)
    }

    fn key_path(&self) -> PathBuf {
        self.data_dir.join(KEY_FILE)
    }

    /// 读取 / 创建密钥文件；首次生成随机 32B 并以权限 600 落盘。
    fn load_or_create_key(&self) -> Result<[u8; 32], NetError> {
        let path = self.key_path();
        if path.exists() {
            let raw = std::fs::read(&path).map_err(|e| NetError::Io(e.to_string()))?;
            if raw.len() != 32 {
                return Err(NetError::Crypto(format!(
                    "密钥文件长度异常（{} 字节），疑似被篡改",
                    raw.len()
                )));
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&raw);
            Ok(k)
        } else {
            std::fs::create_dir_all(&self.data_dir)
                .map_err(|e| NetError::Io(e.to_string()))?;
            let mut k = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut k);
            self.write_key_file(&path, &k)?;
            Ok(k)
        }
    }

    #[cfg(unix)]
    fn write_key_file(&self, path: &Path, key: &[u8; 32]) -> Result<(), NetError> {
        use std::os::unix::fs::OpenOptionsExt;
        let f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| NetError::Io(e.to_string()))?;
        std::io::Write::write_all(&mut { f }, key).map_err(|e| NetError::Io(e.to_string()))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn write_key_file(&self, path: &Path, key: &[u8; 32]) -> Result<(), NetError> {
        std::fs::write(path, key).map_err(|e| NetError::Io(e.to_string()))
    }

    /// 加密落盘（AES-256-GCM，§7.3）。
    fn persist(&self) -> Result<(), NetError> {
        let key = self.load_or_create_key()?;
        let cache = self.inner.lock().unwrap().cache.clone();
        let plain = serde_json::to_vec(&cache).map_err(|e| NetError::Protocol(e.to_string()))?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| NetError::Crypto(e.to_string()))?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload { msg: &plain, aad: CACHE_AAD },
            )
            .map_err(|_| NetError::Crypto("AES-GCM 加密失败".to_string()))?;
        let envelope = Envelope {
            format: "qibai-netsocial".to_string(),
            version: 1,
            nonce_b64: STANDARD.encode(nonce_bytes),
            ciphertext_b64: STANDARD.encode(ciphertext),
        };
        let raw = serde_json::to_string_pretty(&envelope)
            .map_err(|e| NetError::Protocol(e.to_string()))?;
        // 问题7-2：原子写（tmp+rename）。
        crate::fsutil::atomic_write_json(&self.cache_path(), &raw).map_err(|e| NetError::Io(e.to_string()))?;
        Ok(())
    }

    /// 启动恢复；文件不存在 = 全新空缓存。密钥 / 密文被篡改时报错。
    fn restore(&self) -> Result<(), NetError> {
        let path = self.cache_path();
        if !path.exists() {
            return Ok(());
        }
        let key = self.load_or_create_key()?;
        let raw = std::fs::read_to_string(&path).map_err(|e| NetError::Io(e.to_string()))?;
        let envelope: Envelope =
            serde_json::from_str(&raw).map_err(|e| NetError::Protocol(e.to_string()))?;
        let nonce = STANDARD
            .decode(envelope.nonce_b64)
            .map_err(|e| NetError::Crypto(e.to_string()))?;
        let ciphertext = STANDARD
            .decode(envelope.ciphertext_b64)
            .map_err(|e| NetError::Crypto(e.to_string()))?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| NetError::Crypto(e.to_string()))?;
        let plain = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload { msg: &ciphertext, aad: CACHE_AAD },
            )
            .map_err(|_| NetError::Crypto("缓存密文或密钥被篡改，无法解密".to_string()))?;
        let cache: PersistedCache =
            serde_json::from_slice(&plain).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.inner.lock().unwrap().cache = cache;
        Ok(())
    }

    // ---- §7.5 命令实现 ----

    /// `netsocial_status`。
    pub fn status(&self) -> serde_json::Value {
        let g = self.inner.lock().unwrap();
        serde_json::json!({
            "state": g.state.as_str(),
            "anon_id": g.cache.anon_id.clone(),
            "nickname": g.cache.nickname.clone(),
            "color_idx": g.cache.color_idx,
            "city": g.cache.city.clone(),
            "server_url": g.cache.server_url.clone(),
            "online": g.state == NetState::Online,
            "registered": g.cache.server_registered,
            "last_error": g.last_error.clone().or_else(|| self.load_error.lock().unwrap().clone()),
        })
    }

    /// `netsocial_register`（§7.1：本地生成 UUIDv4；若已配置 server_url 则补注册换 token）。
    pub async fn register(&self, input: &serde_json::Value) -> Result<serde_json::Value, NetError> {
        {
            let mut g = self.inner.lock().unwrap();
            if g.state == NetState::Online {
                return Err(NetError::InvalidTransition("在线中请先 disconnect 再重设身份".into()));
            }
            if g.cache.anon_id.is_none() {
                g.cache.anon_id = Some(gen_uuid_v4());
            }
            apply_profile_input(&mut g.cache, input);
        }
        // 已配置服务器地址且未服务端注册 → 在线注册换 token
        let (url, need_reg) = {
            let g = self.inner.lock().unwrap();
            (g.cache.server_url.clone(), !g.cache.server_registered)
        };
        if !url.is_empty() && need_reg {
            let body = self.register_body();
            let resp = self.http_post(&url, "/register", &body).await?;
            let mut g = self.inner.lock().unwrap();
            if let Some(token) = resp.get("token").and_then(|t| t.as_str()) {
                g.cache.token = Some(token.to_string());
            }
            // 服务端下发的 UUIDv4 为联网身份正身（离线占位的本地生成 id 仅兜底）。
            if let Some(id) = resp.get("anon_id").and_then(|t| t.as_str()) {
                g.cache.anon_id = Some(id.to_string());
            }
            g.cache.server_registered = true;
        }
        {
            let mut g = self.inner.lock().unwrap();
            if g.state == NetState::Unregistered {
                g.state = NetState::Registered;
            } else if g.state == NetState::Offline {
                g.state = NetState::Registered;
            }
        }
        self.persist()?;
        let g = self.inner.lock().unwrap();
        Ok(serde_json::json!({
            "ok": true,
            "anon_id": g.cache.anon_id.clone(),
            "state": g.state.as_str(),
            "registered": g.cache.server_registered,
        }))
    }

    fn register_body(&self) -> serde_json::Value {
        let g = self.inner.lock().unwrap();
        serde_json::json!({
            "nickname": g.cache.nickname,
            "color_idx": g.cache.color_idx,
            "soul_tags": g.cache.soul_tags,
            "city": g.cache.city,
            "age_declared": if g.cache.age_declared.is_empty() { "18plus" } else { &g.cache.age_declared },
            "pure_mode": g.cache.pure_mode,
            "avatar": g.cache.avatar,
        })
    }

    /// `netsocial_connect`（§7.1：Registered/Offline → Online；握手成功才算 Online）。
    pub async fn connect(&self, server_url: Option<String>) -> Result<serde_json::Value, NetError> {
        {
            let g = self.inner.lock().unwrap();
            if g.cache.anon_id.is_none() {
                return Err(NetError::NotRegistered);
            }
        }
        // 解析 server_url：入参 > 已存
        let url = {
            let g = self.inner.lock().unwrap();
            match server_url {
                Some(u) if !u.trim().is_empty() => u.trim().to_string(),
                _ => {
                    if g.cache.server_url.is_empty() {
                        return Err(NetError::MissingServerUrl);
                    }
                    g.cache.server_url.clone()
                }
            }
        };
        // 先停掉旧连接任务（幂等）。
        if let Some(tx) = self.cmd_tx.lock().unwrap().take() {
            let _ = tx.send(WsCmd::Shutdown);
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.cache.server_url = url.clone();
        }
        self.persist()?;
        // 未服务端注册 → 先 HTTP 注册换 token
        let need_reg = self.inner.lock().unwrap().cache.server_registered;
        if !need_reg {
            let body = self.register_body();
            let resp = self.http_post(&url, "/register", &body).await?;
            let mut g = self.inner.lock().unwrap();
            if let Some(token) = resp.get("token").and_then(|t| t.as_str()) {
                g.cache.token = Some(token.to_string());
            }
            if let Some(id) = resp.get("anon_id").and_then(|t| t.as_str()) {
                g.cache.anon_id = Some(id.to_string());
            }
            g.cache.server_registered = true;
            drop(g);
            self.persist()?;
        }
        let token = self
            .inner
            .lock()
            .unwrap()
            .cache
            .token
            .clone()
            .ok_or_else(|| NetError::Protocol("缺少认证 token".to_string()))?;
        // WS 握手（await 成功才置 Online）。
        let ws_url = ws_url_of(&url);
        // 问题7隐私路由：明文 ws:// 升级提示——公网部署应配 https:// 端点走 wss:// TLS。
        if ws_url.starts_with("ws://") {
            eprintln!(
                "[qibai] netsocial 警告: 使用明文 ws://（非 wss:// TLS），通信可被局域网嗅探；公网部署请改用 https:// 端点"
            );
        }
        let stream = self
            .connector
            .connect(&ws_url, &token)
            .await
            .map_err(|e| {
                let mut g = self.inner.lock().unwrap();
                g.last_error = Some(e.to_string());
                e
            })?;
        {
            let mut g = self.inner.lock().unwrap();
            g.state = NetState::Online;
            g.last_error = None;
            push_event(&mut g, "connected", serde_json::json!({"server_url": url}));
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.cmd_tx.lock().unwrap() = Some(tx);
        let inner = self.inner.clone();
        let connector = self.connector.clone();
        let mgr = self.self_ref.get().unwrap().clone();
        tokio::spawn(async move {
            ws_session_loop(url, token, inner, connector, rx, Some(stream), mgr).await;
        });
        Ok(serde_json::json!({"ok": true, "state": "online"}))
    }

    /// `netsocial_disconnect`（§7.1：Online → Offline 同步迁移；停止后台重连任务）。
    pub fn disconnect(&self) -> Result<serde_json::Value, NetError> {
        {
            let mut g = self.inner.lock().unwrap();
            if g.state != NetState::Online {
                return Err(NetError::NotOnline);
            }
            // 同步置 Offline（不等后台任务收尾），保证调用后立即 status 一致。
            g.state = NetState::Offline;
            push_event(&mut g, "disconnected", serde_json::json!({"reason": "user_requested"}));
        }
        if let Some(tx) = self.cmd_tx.lock().unwrap().take() {
            let _ = tx.send(WsCmd::Shutdown);
        }
        Ok(serde_json::json!({"ok": true, "state": "offline"}))
    }

    /// `netsocial_set_profile`（本地持久化；在线时 PUT /api/me）。
    pub async fn set_profile(&self, input: &serde_json::Value) -> Result<serde_json::Value, NetError> {
        {
            let mut g = self.inner.lock().unwrap();
            if g.cache.anon_id.is_none() {
                return Err(NetError::NotRegistered);
            }
            apply_profile_input(&mut g.cache, input);
        }
        let online = self.inner.lock().unwrap().state == NetState::Online;
        if online {
            let body = {
                let g = self.inner.lock().unwrap();
                serde_json::json!({
                    "token": g.cache.token,
                    "nickname": g.cache.nickname,
                    "color_idx": g.cache.color_idx,
                    "soul_tags": g.cache.soul_tags,
                    "city": g.cache.city,
                    "avatar": g.cache.avatar,
                })
            };
            let url = self.inner.lock().unwrap().cache.server_url.clone();
            self.http_put(&url, "/me", &body).await?;
        }
        self.persist()?;
        Ok(serde_json::json!({"ok": true}))
    }

    /// `netsocial_roam`（GET /api/souls，原样透传）。
    pub async fn roam(&self, params: &serde_json::Value) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        let mut qs: Vec<(String, String)> = Vec::new();
        for k in ["tag", "city", "online", "q"] {
            if let Some(v) = params.get(k).and_then(|v| v.as_str()) {
                qs.push((k.to_string(), v.to_string()));
            }
        }
        let resp = self.http_get("/souls", &qs).await?;
        Ok(serde_json::json!({"souls": resp.get("souls").cloned().unwrap_or_default()}))
    }

    /// `netsocial_match`（GET /api/match）。
    pub async fn r#match(&self, limit: Option<u32>) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        let qs: Vec<(String, String)> = match limit {
            Some(l) => vec![("limit".to_string(), l.to_string())],
            None => vec![],
        };
        let resp = self.http_get("/match", &qs).await?;
        Ok(serde_json::json!({"souls": resp.get("souls").cloned().unwrap_or_default()}))
    }

    /// `netsocial_city_souls`（GET /api/city/:city/souls）。
    pub async fn city_souls(&self, city: &str) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        let resp = self
            .http_get(&format!("/city/{}/souls", urlencode(city)), &[])
            .await?;
        Ok(serde_json::json!({
            "souls": resp.get("souls").cloned().unwrap_or_default(),
            "city": resp.get("city").cloned().unwrap_or(serde_json::json!(city)),
        }))
    }

    /// `netsocial_like`。
    pub async fn like(&self, id: &str) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        self.require_id(id)?;
        let body = self.auth_body();
        let resp = self
            .http_post(&self.base_url()?, &format!("/souls/{id}/like"), &body)
            .await?;
        Ok(serde_json::json!({
            "liked": resp.get("liked").cloned().unwrap_or(serde_json::json!(true)),
            "matched": resp.get("matched").cloned().unwrap_or(serde_json::json!(false)),
            "mutual": resp.get("mutual").cloned().unwrap_or(serde_json::json!(false)),
        }))
    }

    /// `netsocial_report`。
    pub async fn report(&self, id: &str, reason: &str) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        self.require_id(id)?;
        let mut body = self.auth_body();
        body["reason"] = serde_json::json!(reason);
        self.http_post(&self.base_url()?, &format!("/souls/{id}/report"), &body)
            .await?;
        Ok(serde_json::json!({"ok": true}))
    }

    /// `netsocial_block`。
    pub async fn block(&self, id: &str) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        self.require_id(id)?;
        let body = self.auth_body();
        let resp = self
            .http_post(&self.base_url()?, &format!("/souls/{id}/block"), &body)
            .await?;
        Ok(serde_json::json!({
            "ok": true,
            "blocked": resp.get("blocked").cloned().unwrap_or(serde_json::json!(true)),
        }))
    }

    /// `netsocial_send`（离线 / 未连接 → not_online 结构化错误）。
    pub async fn send(&self, id: &str, text: &str) -> Result<serde_json::Value, NetError> {
        self.require_online()?;
        self.require_id(id)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(NetError::InvalidTransition("消息内容不能为空".to_string()));
        }
        if text.chars().count() > 500 {
            return Err(NetError::InvalidTransition("消息内容超长（上限 500 字符）".to_string()));
        }
        let peer = peer_key(id);
        let ts = now_secs();
        {
            let mut g = self.inner.lock().unwrap();
            g.cache.messages.push(MessageRecord {
                peer_key: peer.clone(),
                text: text.to_string(),
                ts,
                dir: "out".to_string(),
            });
            trim_messages(&mut g);
        }
        if let Some(tx) = self.cmd_tx.lock().unwrap().as_ref() {
            let _ = tx.send(WsCmd::SendMsg {
                to: id.to_string(),
                body: text.to_string(),
            });
        }
        self.persist()?;
        Ok(serde_json::json!({"ok": true}))
    }

    /// `netsocial_history`（按 peer_key 过滤或全部；不含原始 anon_id）。
    pub fn history(&self, id: Option<&str>) -> Result<serde_json::Value, NetError> {
        let g = self.inner.lock().unwrap();
        let key = id.map(peer_key);
        let contacts: Vec<&ContactRecord> = match &key {
            Some(k) => g.cache.contacts.iter().filter(|c| &c.peer_key == k).collect(),
            None => g.cache.contacts.iter().collect(),
        };
        let messages: Vec<&MessageRecord> = match &key {
            Some(k) => g.cache.messages.iter().filter(|m| &m.peer_key == k).collect(),
            None => g.cache.messages.iter().collect(),
        };
        Ok(serde_json::json!({
            "contacts": contacts.iter().map(|c| serde_json::json!({
                "peer_key": c.peer_key, "peer_nick": c.peer_nick,
                "peer_color": c.peer_color, "peer_online": c.peer_online,
                "last_seen": c.last_seen,
            })).collect::<Vec<_>>(),
            "messages": messages.iter().map(|m| serde_json::json!({
                "peer_key": m.peer_key, "text": m.text, "ts": m.ts, "dir": m.dir,
            })).collect::<Vec<_>>(),
        }))
    }

    /// `netsocial_poll`（消费事件队列并清空）。
    pub fn poll(&self) -> serde_json::Value {
        let mut g = self.inner.lock().unwrap();
        let events: Vec<serde_json::Value> = g.events.drain(..).collect();
        serde_json::json!({"events": events})
    }

    /// `netsocial_topics`（占位接口，无需鉴权）。
    pub async fn topics(&self) -> Result<serde_json::Value, NetError> {
        let resp = self.http_get_no_auth("/topics").await?;
        Ok(serde_json::json!({"topics": resp.get("topics").cloned().unwrap_or_default()}))
    }

    /// `netsocial_events`（占位接口，无需鉴权）。
    pub async fn events_wall(&self) -> Result<serde_json::Value, NetError> {
        let resp = self.http_get_no_auth("/events").await?;
        Ok(serde_json::json!({"events": resp.get("events").cloned().unwrap_or_default()}))
    }

    /// `netsocial_cache_clear`（删除 key + 密文两文件，重置身份）。
    pub fn cache_clear(&self) -> Result<serde_json::Value, NetError> {
        for p in [self.cache_path(), self.key_path()] {
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| NetError::Io(e.to_string()))?;
            }
        }
        let mut g = self.inner.lock().unwrap();
        *g = Inner::default();
        if let Some(tx) = self.cmd_tx.lock().unwrap().take() {
            let _ = tx.send(WsCmd::Shutdown);
        }
        Ok(serde_json::json!({"ok": true}))
    }

    // ---- HTTP 小助手 ----

    fn base_url(&self) -> Result<String, NetError> {
        let g = self.inner.lock().unwrap();
        if g.cache.server_url.is_empty() {
            Err(NetError::MissingServerUrl)
        } else {
            Ok(g.cache.server_url.clone())
        }
    }

    fn auth_body(&self) -> serde_json::Value {
        let g = self.inner.lock().unwrap();
        serde_json::json!({"token": g.cache.token.clone().unwrap_or_default()})
    }

    fn require_online(&self) -> Result<(), NetError> {
        let g = self.inner.lock().unwrap();
        if g.state != NetState::Online {
            return Err(NetError::NotOnline);
        }
        Ok(())
    }

    fn require_id(&self, id: &str) -> Result<(), NetError> {
        if id.trim().is_empty() {
            return Err(NetError::InvalidTransition("缺少目标 id".to_string()));
        }
        Ok(())
    }

    async fn http_post(&self, base: &str, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, NetError> {
        self.http
            .request("POST", &format!("{base}/api{path}"), Some(body))
            .await
    }

    async fn http_put(&self, base: &str, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, NetError> {
        self.http
            .request("PUT", &format!("{base}/api{path}"), Some(body))
            .await
    }

    async fn http_get(&self, path: &str, qs: &[(String, String)]) -> Result<serde_json::Value, NetError> {
        let base = self.base_url()?;
        let token = self.inner.lock().unwrap().cache.token.clone().unwrap_or_default();
        let mut all: Vec<(String, String)> = vec![("token".to_string(), token)];
        all.extend_from_slice(qs);
        let q: Vec<String> = all.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let url = format!("{base}/api{path}?{}", q.join("&"));
        self.http.request("GET", &url, None).await
    }

    async fn http_get_no_auth(&self, path: &str) -> Result<serde_json::Value, NetError> {
        let base = self.base_url()?;
        self.http
            .request("GET", &format!("{base}/api{path}"), None)
            .await
    }

    /// 测试钩子：直接投递一帧 WS 文本（模拟服务端推送，不经过网络）。
    #[cfg(test)]
    pub async fn feed_frame(&self, text: &str) {
        handle_frame(&self.inner, text);
        // 帧处理可能写入消息缓存 → 落盘保持一致（测试目录内）。
        let _ = self.persist();
    }
}

// ---- 自由函数 ----

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// 生成本地 UUIDv4（离线身份兜底；联网注册后以服务端下发为准）。
fn gen_uuid_v4() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}{}{}{}-{}{}-{}{}-{}{}-{}{}{}{}{}{}",
        h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]
    )
}

/// 应用 profile 输入（昵称默认「星客」、color_idx clamp 0-11、标签裁剪）。
fn apply_profile_input(cache: &mut PersistedCache, input: &serde_json::Value) {
    if let Some(nick) = input.get("nickname").and_then(|v| v.as_str()) {
        let nick = nick.trim();
        cache.nickname = if nick.is_empty() {
            "星客".to_string()
        } else {
            nick.chars().take(12).collect()
        };
    }
    if cache.nickname.is_empty() {
        cache.nickname = "星客".to_string();
    }
    if let Some(c) = input.get("color_idx").and_then(|v| v.as_u64()) {
        cache.color_idx = (c.clamp(0, 11)) as u8;
    }
    if let Some(tags) = input.get("soul_tags") {
        cache.soul_tags = parse_soul_tags(tags);
    }
    if let Some(city) = input.get("city").and_then(|v| v.as_str()) {
        cache.city = city.trim().to_string();
    }
    if let Some(a) = input.get("avatar").and_then(|v| v.as_i64()) {
        cache.avatar = a.clamp(-1, 4) as i32;
    }
    if let Some(a) = input.get("age_declared").and_then(|v| v.as_str()) {
        cache.age_declared = a.to_string();
    }
    if let Some(p) = input.get("pure_mode").and_then(|v| v.as_bool()) {
        cache.pure_mode = p;
    }
}

/// 标签三组各最多 5 个、单标签 ≤ 8 字符、trim 去重。
fn parse_soul_tags(v: &serde_json::Value) -> SoulTags {
    fn clean(arr: &[serde_json::Value]) -> Vec<String> {
        let mut out: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(8).collect::<String>())
            .collect();
        out.sort();
        out.dedup();
        out.truncate(5);
        out
    }
    SoulTags {
        personality: v.get("personality").and_then(|x| x.as_array()).map(|a| clean(a)).unwrap_or_default(),
        interests: v.get("interests").and_then(|x| x.as_array()).map(|a| clean(a)).unwrap_or_default(),
        status: v.get("status").and_then(|x| x.as_array()).map(|a| clean(a)).unwrap_or_default(),
    }
}

fn upsert_contact(g: &mut Inner, key: &str, nick: Option<&str>, color: Option<u8>, online: Option<bool>, last_seen: u64) {
    if let Some(c) = g.cache.contacts.iter_mut().find(|c| c.peer_key == key) {
        if let Some(n) = nick {
            c.peer_nick = n.to_string();
        }
        if let Some(cl) = color {
            c.peer_color = cl;
        }
        if let Some(o) = online {
            c.peer_online = o;
        }
        if last_seen > 0 {
            c.last_seen = last_seen;
        }
    } else {
        g.cache.contacts.push(ContactRecord {
            peer_key: key.to_string(),
            peer_nick: nick.unwrap_or("星尘光点").to_string(),
            peer_color: color.unwrap_or(0),
            peer_online: online.unwrap_or(false),
            last_seen,
        });
        g.cache.contacts.truncate(CONTACT_CAP);
    }
}

fn trim_messages(g: &mut Inner) {
    if g.cache.messages.len() > MESSAGE_CAP {
        let drain = g.cache.messages.len() - MESSAGE_CAP;
        g.cache.messages.drain(0..drain);
    }
}

fn push_event(g: &mut Inner, typ: &str, payload: serde_json::Value) {
    g.events.push_back(serde_json::json!({"type": typ, "payload": payload}));
    if g.events.len() > EVENT_CAP {
        g.events.pop_front();
    }
}

/// 处理一帧服务端推送（§7.4：收消息 → 脱敏写缓存 → 事件队列）。
fn handle_frame(inner: &Arc<Mutex<Inner>>, text: &str) {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("hello") => {
            let profile = v.get("profile").cloned().unwrap_or(serde_json::json!({}));
            // 联调口径：服务端 hello 下发的 anon_id 为联网正身，写回状态与加密缓存；
            // 本地生成的仅作注册前占位（§7.1 × §3）。
            let server_id = v.get("anon_id").and_then(|x| x.as_str()).map(str::to_string);
            let mut g = inner.lock().unwrap();
            if let Some(sid) = &server_id {
                if g.cache.anon_id.as_deref() != Some(sid.as_str()) {
                    g.cache.anon_id = Some(sid.clone());
                    g.dirty = true;
                }
            }
            push_event(&mut g, "hello", profile);
        }
        Some("inbox") => {
            let msgs = v.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default();
            let mut count = 0usize;
            for m in &msgs {
                let from = m.get("from").and_then(|x| x.as_str()).unwrap_or("");
                let body = m.get("body").and_then(|x| x.as_str()).unwrap_or("");
                let ts = m.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
                if from.is_empty() || body.is_empty() {
                    continue;
                }
                let key = peer_key(from);
                let mut g = inner.lock().unwrap();
                upsert_contact(&mut g, &key, None, None, Some(true), ts);
                g.cache.messages.push(MessageRecord {
                    peer_key: key,
                    text: body.to_string(),
                    ts,
                    dir: "in".to_string(),
                });
                trim_messages(&mut g);
                count += 1;
            }
            let mut g = inner.lock().unwrap();
            push_event(&mut g, "inbox", serde_json::json!({"count": count}));
        }
        Some("msg.deliver") => {
            let from = v.get("from").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let ts = v.get("ts").and_then(|x| x.as_u64()).unwrap_or(now_secs());
            if from.is_empty() || body.is_empty() {
                return;
            }
            // dir 判定：from == 当前缓存的 anon_id（可能已被 hello 回写为服务端正身）→ out
            let self_id = inner.lock().unwrap().cache.anon_id.clone().unwrap_or_default();
            let dir = if from == self_id { "out" } else { "in" };
            let key = peer_key(&from);
            if dir == "out" {
                // 自己消息的送达回执：send() 已按正确 peer 落盘，这里不重复写缓存、
                // 不建联系人，仅透传事件（前端可据此更新发送状态）。
                let mut g = inner.lock().unwrap();
                push_event(
                    &mut g,
                    "msg",
                    serde_json::json!({
                        "peer_key": key, "peer_nick": "我", "peer_color": 0,
                        "text": body, "ts": ts, "dir": "out",
                    }),
                );
                return;
            }
            let (nick, color) = {
                let mut g = inner.lock().unwrap();
                upsert_contact(&mut g, &key, None, None, Some(true), ts);
                let c = g.cache.contacts.iter().find(|c| c.peer_key == key);
                let nick = c.map(|c| c.peer_nick.clone()).unwrap_or_else(|| "星尘光点".to_string());
                let color = c.map(|c| c.peer_color).unwrap_or(0);
                g.cache.messages.push(MessageRecord {
                    peer_key: key.clone(),
                    text: body.clone(),
                    ts,
                    dir: dir.to_string(),
                });
                trim_messages(&mut g);
                (nick, color)
            };
            let mut g = inner.lock().unwrap();
            push_event(
                &mut g,
                "msg",
                serde_json::json!({
                    "peer_key": key, "peer_nick": nick, "peer_color": color,
                    "text": body, "ts": ts, "dir": dir,
                }),
            );
        }
        Some("presence") => {
            let who = v.get("who").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let online = v.get("online").and_then(|x| x.as_bool()).unwrap_or(false);
            if who.is_empty() {
                return;
            }
            let key = peer_key(&who);
            {
                let mut g = inner.lock().unwrap();
                upsert_contact(&mut g, &key, None, None, Some(online), 0);
            }
            let mut g = inner.lock().unwrap();
            push_event(&mut g, "presence", serde_json::json!({"peer_key": key, "online": online}));
        }
        Some("error") => {
            let mut g = inner.lock().unwrap();
            push_event(
                &mut g,
                "error",
                serde_json::json!({
                    "code": v.get("code").cloned().unwrap_or(serde_json::json!("unknown")),
                    "message": v.get("message").cloned().unwrap_or(serde_json::json!("")),
                }),
            );
        }
        _ => {}
    }
}

/// 后台 WS 会话：握手 → 读写循环 → 断线指数退避重连（1/2/4/8s）→ Shutdown 退出。
/// `first_stream` 为 `connect` 已握手上行的流（复用，避免首连握手被丢弃）。
async fn ws_session_loop(
    url: String,
    token: String,
    inner: Arc<Mutex<Inner>>,
    connector: Arc<dyn WsConnector>,
    mut cmd_rx: UnboundedReceiver<WsCmd>,
    mut first_stream: Option<Box<dyn WsStream>>,
    mgr: Arc<NetsocialManager>,
) {
    let ws_url = ws_url_of(&url);
    let mut attempt = 0usize;
    loop {
        let mut stream = match first_stream.take() {
            Some(s) => s,
            None => match connector.connect(&ws_url, &token).await {
                Ok(s) => s,
                Err(e) => {
                    {
                        let mut g = inner.lock().unwrap();
                        g.state = NetState::Offline;
                        g.last_error = Some(e.to_string());
                        push_event(&mut g, "error", serde_json::json!({"code":"connect_failed","message":e.to_string()}));
                    }
                    let backoff = BACKOFF_SECS[attempt.min(3)];
                    attempt += 1;
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(backoff)) => {}
                        _ = async {
                            match cmd_rx.recv().await {
                                Some(WsCmd::Shutdown) | None => true,
                                Some(_) => false,
                            }
                        } => break,
                    }
                    continue;
                }
            },
        };
        attempt = 0;
        {
            let mut g = inner.lock().unwrap();
            g.state = NetState::Online;
            g.last_error = None;
        }
        let mut shutdown = false;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_SECS));
        loop {
            tokio::select! {
                frame = stream.recv() => {
                    match frame {
                        Ok(text) => {
                            handle_frame(&inner, &text);
                            // 帧处理可能写脏缓存（hello 回写服务端 anon_id）→ 即时落盘。
                            mgr.flush_if_dirty();
                        }
                        Err(e) => {
                            {
                                let mut g = inner.lock().unwrap();
                                g.state = NetState::Offline;
                                push_event(&mut g, "disconnected", serde_json::json!({"reason": e.to_string()}));
                            }
                            break;
                        }
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(WsCmd::SendMsg { to, body }) => {
                            let _ = stream.send(serde_json::json!({"type":"msg.send","to":to,"body":body}).to_string()).await;
                        }
                        Some(WsCmd::Shutdown) | None => {
                            shutdown = true;
                            let _ = stream.close().await;
                            break;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    let _ = stream.send(serde_json::json!({"type":"ping","ts": now_secs()}).to_string()).await;
                }
            }
        }
        {
            let mut g = inner.lock().unwrap();
            g.state = NetState::Offline;
        }
        if shutdown {
            let mut g = inner.lock().unwrap();
            push_event(&mut g, "disconnected", serde_json::json!({"reason":"shutdown"}));
            break;
        }
        let backoff = BACKOFF_SECS[attempt.min(3)];
        attempt += 1;
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
    }
}

// ---- 测试用 mock 连接器 ----

#[cfg(test)]
struct MockConnector;

#[cfg(test)]
struct MockStream;

#[cfg(test)]
#[async_trait]
impl WsConnector for MockConnector {
    async fn connect(&self, _ws_url: &str, _token: &str) -> Result<Box<dyn WsStream>, NetError> {
        Ok(Box::new(MockStream))
    }
}

#[cfg(test)]
#[async_trait]
impl WsStream for MockStream {
    async fn recv(&mut self) -> Result<String, NetError> {
        // 挂起直至任务被取消（Shutdown 时 tokio::spawn 任务结束）。
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        Err(NetError::Network("mock stream closed".to_string()))
    }
    async fn send(&mut self, _text: String) -> Result<(), NetError> {
        Ok(())
    }
    async fn close(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

/// 测试用 HTTP：register 返回假匿名身份 + token；其余调用报错（单测不触网）。
#[cfg(test)]
struct MockHttp;

#[cfg(test)]
#[async_trait]
impl HttpApi for MockHttp {
    async fn request(
        &self,
        method: &str,
        url: &str,
        _body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, NetError> {
        if method == "POST" && url.ends_with("/api/register") {
            return Ok(serde_json::json!({
                "anon_id": "mock-server-anon-uuid",
                "token": "mock-token-123",
                "profile": {"nickname": "星客"},
            }));
        }
        Err(NetError::Network(format!("测试环境无真实服务端：{method} {url}")))
    }
}

// ---- 单测（§9.2 1-4 组） ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;
    use serde_json::json;

    // §9.2-1 状态机迁移：全迁移 + 非法迁移拒绝
    #[tokio::test]
    async fn state_machine_full_transitions_and_rejects_illegal() {
        let dir = unique_temp_dir("netsocial-sm");
        let mgr = NetsocialManager::for_test(&dir);
        // 初始 Unregistered
        assert_eq!(mgr.state(), NetState::Unregistered);
        // 非法：未注册直接 connect → NotRegistered
        match mgr.connect(Some("http://127.0.0.1:9".into())).await {
            Err(NetError::NotRegistered) => {}
            other => panic!("expected NotRegistered, got {other:?}"),
        }
        // 非法：未注册直接 disconnect → NotOnline
        assert!(matches!(mgr.disconnect(), Err(NetError::NotOnline)));
        // register → Registered
        let out = mgr.register(&json!({"nickname":"测试星","color_idx":5})).await.unwrap();
        assert_eq!(mgr.state(), NetState::Registered);
        assert!(out["anon_id"].is_string());
        assert_eq!(out["registered"], json!(false)); // 无 server_url → 离线占位
        // connect(mock) → Online（MockHttp 换假 token + MockConnector 握手）
        let out = mgr.connect(Some("http://127.0.0.1:9".into())).await.unwrap();
        assert_eq!(out["state"], json!("online"));
        assert_eq!(mgr.state(), NetState::Online);
        // disconnect → Offline（状态由后台任务异步翻转，轮询等待）
        mgr.disconnect().unwrap();
        wait_state(&mgr, NetState::Offline).await;
        // 离线后再 register（重设 profile）→ Registered
        mgr.register(&json!({"nickname":"改名星"})).await.unwrap();
        assert_eq!(mgr.state(), NetState::Registered);
        // 未连接时 send → NotOnline
        match mgr.send("peer-id-1", "你好").await {
            Err(NetError::NotOnline) => {}
            other => panic!("expected NotOnline, got {other:?}"),
        }
        // history 本地可用（不依赖网络）
        let h = mgr.history(None).unwrap();
        assert!(h["contacts"].is_array());
    }

    /// 轮询等待后台任务把状态翻转为目标态（带超时）。
    async fn wait_state(mgr: &NetsocialManager, want: NetState) {
        for _ in 0..200 {
            if mgr.state() == want {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("等待状态 {:?} 超时（当前 {:?}）", want, mgr.state());
    }

    // 联调修复①：disconnect 同步把状态置 Offline（不等后台任务），再 connect 可回 Online
    #[tokio::test]
    async fn disconnect_synchronously_offline_and_reconnect_ok() {
        let dir = unique_temp_dir("netsocial-disc-sync");
        let mgr = NetsocialManager::for_test(&dir);
        mgr.register(&json!({"nickname":"同步星"})).await.unwrap();
        mgr.connect(Some("http://127.0.0.1:9".into())).await.unwrap();
        assert_eq!(mgr.state(), NetState::Online);
        // disconnect 后立即 status → offline（不等待异步收尾）
        mgr.disconnect().unwrap();
        assert_eq!(mgr.state(), NetState::Offline, "disconnect 必须同步迁移到 Offline");
        assert_eq!(mgr.status()["state"], json!("offline"));
        // 再 connect 可回 online
        mgr.connect(Some("http://127.0.0.1:9".into())).await.unwrap();
        assert_eq!(mgr.state(), NetState::Online);
        assert_eq!(mgr.status()["state"], json!("online"));
        // 收尾
        mgr.disconnect().unwrap();
        wait_state(&mgr, NetState::Offline).await;
    }

    // 联调修复②：hello 帧下发的服务端 anon_id 为正身，写回状态与加密缓存
    #[tokio::test]
    async fn hello_frame_server_anon_id_takes_precedence() {
        let dir = unique_temp_dir("netsocial-hello-id");
        let mgr = NetsocialManager::for_test(&dir);
        mgr.register(&json!({"nickname":"口径星"})).await.unwrap();
        let local_id = mgr.anon_id().unwrap();
        assert_ne!(local_id, "server-issued-uuid-b03a", "前置：本地占位 id");
        // 服务端 hello 下发不同的 anon_id → 以服务端为准
        mgr.feed_frame("{\"type\":\"hello\",\"anon_id\":\"server-issued-uuid-b03a\",\"profile\":{\"nickname\":\"服务端星\"}}").await;
        assert_eq!(mgr.anon_id().unwrap(), "server-issued-uuid-b03a", "状态应以服务端 anon_id 为准");
        // 加密缓存落盘后，新管理器恢复出来也是服务端正身
        let mgr2 = NetsocialManager::for_test(&dir);
        assert_eq!(
            mgr2.anon_id().unwrap(),
            "server-issued-uuid-b03a",
            "加密缓存应以服务端 anon_id 为准"
        );
        // 后续自己消息回执的 dir 判定也用服务端正身
        mgr.feed_frame("{\"type\":\"msg.deliver\",\"mid\":9,\"from\":\"server-issued-uuid-b03a\",\"body\":\"自己的回执\",\"ts\":42}").await;
        let events = mgr.poll();
        let ev_raw = serde_json::to_string(&events).unwrap();
        assert!(ev_raw.contains("\"dir\":\"out\""), "hello 回写后自己回执应判 out: {ev_raw}");
    }

    // §9.2-2 脱敏：peer_key = sha256 截断 12 hex；缓存/历史不含原始 anon_id；联系人去重
    #[test]
    fn peer_key_is_sha256_truncated_12hex() {
        use sha2::Digest;
        let id = "123e4567-e89b-42d3-a456-426614174000";
        let mut h = Sha256::new();
        h.update(id.as_bytes());
        let d = h.finalize();
        let expect: String = d.iter().take(6).map(|b| format!("{b:02x}")).collect();
        assert_eq!(peer_key(id), expect);
        assert_eq!(peer_key(id).len(), 12);
    }

    #[tokio::test]
    async fn cache_and_history_never_store_raw_anon_id() {
        let dir = unique_temp_dir("netsocial-desens");
        let mgr = NetsocialManager::for_test(&dir);
        mgr.register(&json!({"nickname":"脱敏星"})).await.unwrap();
        let anon = mgr.anon_id().unwrap();
        // 模拟服务端推送：一条来自我自己（dir=out）、一条来自陌生 peer（dir=in）
        mgr.feed_frame(&format!("{{\"type\":\"msg.deliver\",\"mid\":1,\"from\":\"{anon}\",\"body\":\"我发的\",\"ts\":100}}")).await;
        mgr.feed_frame("{\"type\":\"msg.deliver\",\"mid\":2,\"from\":\"peer-anon-xyz\",\"body\":\"你好呀\",\"ts\":200}").await;
        // inbox 补投也走脱敏
        mgr.feed_frame("{\"type\":\"inbox\",\"messages\":[{\"from\":\"peer-anon-xyz\",\"body\":\"离线消息\",\"ts\":300}]}").await;
        // 历史输出不含任何原始 anon_id 明文
        let hist = mgr.history(None).unwrap();
        let raw = serde_json::to_string(&hist).unwrap();
        assert!(!raw.contains(&anon), "历史泄漏自身 anon_id: {raw}");
        assert!(!raw.contains("peer-anon-xyz"), "历史泄漏 peer anon_id: {raw}");
        // dir 判定正确：入站消息落盘（in）；自己回执不落盘（只进事件队列）
        let msgs = hist["messages"].as_array().unwrap();
        assert!(msgs.iter().any(|m| m["dir"] == json!("in") && m["text"] == json!("你好呀")));
        assert!(msgs.iter().any(|m| m["dir"] == json!("in") && m["text"] == json!("离线消息")));
        assert!(msgs.iter().all(|m| m["text"] != "我发的"), "自己回执不应写缓存");
        // 联系人按 peer_key 去重（peer-anon-xyz 出现两次只一条；自己回执不建联系人）
        let contacts = hist["contacts"].as_array().unwrap();
        assert_eq!(contacts.len(), 1, "contacts={contacts:?}");
        assert_eq!(contacts[0]["peer_key"], json!(peer_key("peer-anon-xyz")));
        // 事件队列：自己回执带 dir=out；消息事件只带 peer_key
        let events = mgr.poll();
        let ev_raw = serde_json::to_string(&events).unwrap();
        assert!(!ev_raw.contains("peer-anon-xyz"));
        assert!(ev_raw.contains(peer_key("peer-anon-xyz").as_str()));
        assert!(ev_raw.contains("\"dir\":\"out\""), "事件应含自己回执: {ev_raw}");
    }

    // §9.2-3 加密缓存：首次生成 key、加解密往返、篡改密钥/密文报错、cache_clear 删除
    #[tokio::test]
    async fn encrypted_cache_roundtrip_tamper_and_clear() {
        use std::fs;
        let dir = unique_temp_dir("netsocial-enc");
        let mgr = NetsocialManager::for_test(&dir);
        mgr.register(&json!({"nickname":"缓存星","city":"石家庄","color_idx":7})).await.unwrap();
        // 文件已生成：密文信封 + 32B 密钥
        assert!(dir.join(CACHE_FILE).exists());
        assert!(dir.join(KEY_FILE).exists());
        let key = fs::read(dir.join(KEY_FILE)).unwrap();
        assert_eq!(key.len(), 32);
        // 新管理器同目录 → 恢复 → 昵称/城市保留
        let mgr2 = NetsocialManager::for_test(&dir);
        let st = mgr2.status();
        assert_eq!(st["nickname"], json!("缓存星"));
        assert_eq!(st["city"], json!("石家庄"));
        // 缓存文件原始内容不含明文昵称
        let envelope_raw = fs::read_to_string(dir.join(CACHE_FILE)).unwrap();
        assert!(!envelope_raw.contains("缓存星"), "密文信封泄漏明文昵称");
        // 篡改密文（翻转 base64 主体中间字节）→ 恢复报错
        let mut bad_bytes = envelope_raw.clone().into_bytes();
        let mid = bad_bytes.len() / 2;
        bad_bytes[mid] ^= 0x55;
        fs::write(dir.join(CACHE_FILE), &bad_bytes).unwrap();
        let mgr3 = NetsocialManager::for_test(&dir);
        assert!(mgr3.status()["last_error"].is_string(), "篡改密文应报错");
        // 还原密文后篡改密钥 → 恢复报错
        fs::write(dir.join(CACHE_FILE), &envelope_raw).unwrap();
        let mut bad_key = key.clone();
        bad_key[0] ^= 0xff;
        fs::write(dir.join(KEY_FILE), &bad_key).unwrap();
        let mgr4 = NetsocialManager::for_test(&dir);
        assert!(mgr4.status()["last_error"].is_string(), "篡改密钥应报错");
        // 还原 key
        fs::write(dir.join(KEY_FILE), &key).unwrap();
        // cache_clear 删除两文件并重置身份
        let mgr5 = NetsocialManager::for_test(&dir);
        mgr5.cache_clear().unwrap();
        assert!(!dir.join(CACHE_FILE).exists());
        assert!(!dir.join(KEY_FILE).exists());
        assert_eq!(mgr5.state(), NetState::Unregistered);
    }

    // §9.2-4 事件队列：环形上限 200、poll 消费清空
    #[tokio::test]
    async fn event_ring_buffer_caps_at_200_and_poll_drains() {
        let dir = unique_temp_dir("netsocial-events");
        let mgr = NetsocialManager::for_test(&dir);
        mgr.register(&json!({"nickname":"事件星"})).await.unwrap();
        // 连续投递 250 条入站消息 → 每条产生 1 个事件，环形裁剪到 200
        for i in 0..250u64 {
            mgr.feed_frame(&format!("{{\"type\":\"msg.deliver\",\"mid\":{i},\"from\":\"peer-{i}\",\"body\":\"m{i}\",\"ts\":{i}}}")).await;
        }
        let v = mgr.poll();
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), EVENT_CAP, "环形上限应为 {EVENT_CAP}");
        // 最先的 50 条被挤出队列（队列里最早一条应是 peer-50 的事件）
        assert_eq!(events[0]["payload"]["peer_key"], json!(peer_key("peer-50")));
        // 第二次 poll 为空
        let v2 = mgr.poll();
        assert!(v2["events"].as_array().unwrap().is_empty());
        // 再推一条 → 单条可消费
        mgr.feed_frame("{\"type\":\"presence\",\"who\":\"peer-new\",\"online\":true}").await;
        let v3 = mgr.poll();
        assert_eq!(v3["events"].as_array().unwrap().len(), 1);
        assert_eq!(v3["events"][0]["type"], json!("presence"));
    }
}
