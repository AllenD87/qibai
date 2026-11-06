//! 本地密码保险箱（迭代③，冻结规格 VAULT_SPEC v1）。
//!
//! - 纯 Rust，与 Tauri/GUI 解耦：AES-256-GCM 整库加密；PBKDF2-SHA256 派生主密钥。
//! - 数据文件 `<data_dir>/vault.qbkv`，导出备份为同格式信封字符串（.qbkv）。
//! - 状态机 `unset → locked ⇄ unlocked`；`mode = keyring`（Tauri 层 32B 随机 key
//!   经钥匙串读写，core 只收发 key 字节）或 `master_password`（主密码派生）。
//! - 设计红线：密码明文只存在于 core 内存（zeroize）与当次命令瞬时返回；
//!   永不落盘、永不进 LLM 上下文、永不进朗读文本。

use crate::reminder::{Clock, SystemClock};
use crate::settings::VaultSettings;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

/// PBKDF2 迭代次数（问题7隐私路由：210k → 600k，对齐 OWASP 2023 推荐下限
/// https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html ）。
/// 落盘存文件头：旧库仍按文件头记录的旧迭代数解开（向后兼容），仅新建库用 600k。
pub const KDF_ITERATIONS: u32 = 600_000;

const VAULT_AAD: &[u8] = b"qibai-vault-v1";
const VERIFIER_PLAINTEXT: &[u8] = b"qibai-vault-verifier-v1";
const VAULT_FILE: &str = "vault.qbkv";
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;

// ---- 枚举 ----

/// 保险箱状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultState {
    Unset,
    Locked,
    Unlocked,
}

/// 解锁模式。unset 时为 None。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultMode {
    None,
    Keyring,
    MasterPassword,
}

impl VaultMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            VaultMode::None => "none",
            VaultMode::Keyring => "keyring",
            VaultMode::MasterPassword => "master_password",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keyring" => Some(VaultMode::Keyring),
            "master_password" => Some(VaultMode::MasterPassword),
            _ => None,
        }
    }
}

// ---- 结构化错误（契约 §4；不 panic） ----

#[derive(Debug, Error, Serialize)]
pub enum VaultError {
    #[error("vault locked")]
    VaultLocked,
    #[error("vault cooldown: remaining {remaining_secs}s")]
    VaultCooldown { remaining_secs: u64 },
    #[error("wrong master password")]
    VaultWrongPassword,
    #[error("vault not initialized")]
    VaultNotInitialized,
    #[error("vault already unlocked")]
    VaultAlreadyUnlocked,
    #[error("intimacy gate locked: required L{required}")]
    VaultGateLocked { required: u8 },
    #[error("entry not found")]
    VaultNotFound,
    #[error("vault import failed: {0}")]
    VaultImportFailed(String),
    #[error("keyring unavailable")]
    VaultKeyringUnavailable,
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("io error: {0}")]
    Io(String),
}

// ---- 数据结构 ----

/// 一条密码记录（仅以密文形式落盘；内存中为明文，lock 时整体丢弃）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub site: String,
    pub account: String,
    pub password: String,
    pub note: String,
    pub icon: String,
    pub updated_at: u64,
}

/// add/update 的入参。update 时 password 传空串表示不改。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VaultEntryInput {
    pub site: String,
    pub account: String,
    pub password: String,
    pub note: String,
    pub icon: String,
}

/// vault_copy 的 core 内部返回：明文只传给 Tauri/mock 层瞬时写剪贴板，
/// 前端载荷由 Tauri 层映射为 `{ copied: true, clear_after_secs }`（不含明文）。
#[derive(Debug, Clone, Serialize)]
pub struct VaultCopyHold {
    pub plaintext: String,
    pub clear_after_secs: u64,
    pub clear_at: u64,
}

/// vault_status 返回（契约 §4）。
#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    pub state: VaultState,
    pub mode: VaultMode,
    /// Tauri 层探测钥匙串可用性；core 不感知，由调用方填入。
    pub keyring_available: bool,
    /// active persona 亲密度 < L4 时 true。
    pub gated: bool,
    pub required_level: u8,
    pub idle_lock_min: u32,
    pub max_wrong_attempts: u32,
    pub cooldown_secs: u64,
    pub clipboard_clear_secs: u64,
    pub failed_attempts: u32,
    pub cooldown_remaining_secs: u64,
    /// "manual" | "idle" | "cooldown" | null。
    pub locked_reason: Option<String>,
    /// 仅 unlocked 时给出真实数量，否则 0。
    pub entries_count: u32,
}

// ---- 落盘信封（契约 §1） ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KdfBlock {
    algo: String,
    iterations: u32,
    salt_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    mode: String,
    kdf: KdfBlock,
    nonce_b64: String,
    aad: String,
    verifier_b64: String,
    ciphertext_b64: String,
}

// ---- 加密原语 ----

fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

fn random_hex_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// PBKDF2-HMAC-SHA256 派生 32B 主密钥。
/// 问题7隐私路由：派生用的密码副本拷进可变 Vec，pbkdf2 完成后立即 zeroize，
/// 避免密码字节在 core 内存逗留（主密钥 key 仍由调用方包进 Zeroizing）。
fn derive_key(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    let mut pw = password.to_vec();
    let mut key = vec![0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(&pw, salt, iterations, &mut key);
    pw.zeroize();
    key
}

fn aes_encrypt(key: &[u8], nonce: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Crypto(e.to_string()))?;
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload { msg: plaintext, aad: VAULT_AAD },
        )
        .map_err(|e| VaultError::Crypto(e.to_string()))
}

fn aes_decrypt(key: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Crypto(e.to_string()))?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload { msg: ciphertext, aad: VAULT_AAD },
        )
        .map_err(|e| VaultError::Crypto(e.to_string()))
}

fn b64_decode(s: &str) -> Result<Vec<u8>, VaultError> {
    STANDARD.decode(s).map_err(|e| VaultError::Crypto(e.to_string()))
}

/// 用同一把 key、同一个 nonce 整体重加密（verifier + entries）。
/// 每次落盘整体重加密；盐固定（同一保险箱沿用同一盐），nonce 每次重新随机。
fn build_envelope(
    mode: VaultMode,
    salt: &[u8],
    iterations: u32,
    key: &[u8],
    entries: &[VaultEntry],
) -> Result<Envelope, VaultError> {
    let nonce = random_bytes(NONCE_LEN);
    let verifier = aes_encrypt(key, &nonce, VERIFIER_PLAINTEXT)?;
    let entries_json =
        serde_json::to_vec(entries).map_err(|e| VaultError::Crypto(e.to_string()))?;
    let ciphertext = aes_encrypt(key, &nonce, &entries_json)?;
    Ok(Envelope {
        format: "qibai-vault".to_string(),
        version: 1,
        mode: mode.as_str().to_string(),
        kdf: KdfBlock {
            algo: "PBKDF2-SHA256".to_string(),
            iterations,
            salt_b64: STANDARD.encode(salt),
        },
        nonce_b64: STANDARD.encode(&nonce),
        aad: "qibai-vault-v1".to_string(),
        verifier_b64: STANDARD.encode(verifier),
        ciphertext_b64: STANDARD.encode(ciphertext),
    })
}

/// 校验 verifier 并解出 entries；verifier 不符（含 GCM 标签校验失败）视为密码错误。
fn open_envelope(env: &Envelope, key: &[u8]) -> Result<Vec<VaultEntry>, VaultError> {
    let nonce = b64_decode(&env.nonce_b64)?;
    let verifier_ct = b64_decode(&env.verifier_b64)?;
    // 错误 key → GCM AEAD 校验失败 → 一律归约为 VaultWrongPassword（防暴力计数据此）。
    let verifier_pt = aes_decrypt(key, &nonce, &verifier_ct)
        .map_err(|_| VaultError::VaultWrongPassword)?;
    if verifier_pt.as_slice() != VERIFIER_PLAINTEXT {
        return Err(VaultError::VaultWrongPassword);
    }
    let ct = b64_decode(&env.ciphertext_b64)?;
    let pt = aes_decrypt(key, &nonce, &ct)?;
    serde_json::from_slice(&pt).map_err(|e| VaultError::Crypto(e.to_string()))
}

// ---- Manager ----

/// 保险箱状态机。时钟可注入（参考 reminder.rs 的 SystemClock 模式）。
pub struct VaultManager<C: Clock = SystemClock> {
    path: std::path::PathBuf,
    clock: C,
    settings: VaultSettings,
    // 持久事实（落盘信封镜像）：
    mode: VaultMode,
    kdf_iterations: u32,
    salt: Vec<u8>,
    // 运行时（仅内存）：
    state: VaultState,
    /// unlocked 时驻留主密钥；Zeroizing drop 时自动清零。
    key: Option<Zeroizing<Vec<u8>>>,
    entries: Vec<VaultEntry>,
    failed_attempts: u32,
    cooldown_until: Option<u64>,
    last_activity: u64,
    locked_reason: Option<String>,
}

impl Default for VaultManager<SystemClock> {
    fn default() -> Self {
        Self::new(SystemClock, VaultSettings::default())
    }
}

impl<C: Clock> VaultManager<C> {
    /// 构造一个未初始化（unset）的保险箱；不落盘。
    pub fn new(clock: C, settings: VaultSettings) -> Self {
        let path = crate::settings::data_dir()
            .map(|d| d.join(VAULT_FILE))
            .unwrap_or_else(|_| std::env::temp_dir().join("qibai-vault.qbkv"));
        let now = clock.now_secs();
        Self {
            path,
            clock,
            settings,
            mode: VaultMode::None,
            kdf_iterations: KDF_ITERATIONS,
            salt: Vec::new(),
            state: VaultState::Unset,
            key: None,
            entries: Vec::new(),
            failed_attempts: 0,
            cooldown_until: None,
            last_activity: now,
            locked_reason: None,
        }
    }

    /// 从 `<data_dir>/vault.qbkv` 恢复为 locked 状态（不解密 entries）。
    /// 文件缺失 → 全新 unset；文件损坏 → 宽容回退 unset（启动不崩溃）。
    pub fn load(clock: C, settings: VaultSettings) -> Result<Self, VaultError> {
        let mut s = Self::new(clock, settings);
        if !s.path.exists() {
            return Ok(s);
        }
        let raw = std::fs::read_to_string(&s.path).map_err(|e| VaultError::Io(e.to_string()))?;
        let env: Envelope = match serde_json::from_str(&raw) {
            Ok(e) => e,
            Err(_) => return Ok(s),
        };
        if env.format != "qibai-vault" || env.version != 1 {
            return Ok(s);
        }
        s.mode = VaultMode::parse(&env.mode).unwrap_or(VaultMode::None);
        s.kdf_iterations = env.kdf.iterations;
        s.salt = b64_decode(&env.kdf.salt_b64).unwrap_or_default();
        s.state = VaultState::Locked;
        Ok(s)
    }

    pub fn update_settings(&mut self, s: VaultSettings) {
        self.settings = s;
    }

    pub fn state(&self) -> VaultState {
        self.state
    }

    pub fn mode(&self) -> VaultMode {
        self.mode
    }

    /// unlocked 时给对话拦截做 site/account 子串判定；其余状态 None。
    pub fn unlocked_entries(&self) -> Option<&[VaultEntry]> {
        if self.state == VaultState::Unlocked {
            Some(&self.entries)
        } else {
            None
        }
    }

    pub fn status(&self, keyring_available: bool, gate_open: bool) -> VaultStatus {
        let now = self.clock.now_secs();
        let cooldown_remaining_secs = self
            .cooldown_until
            .map(|u| u.saturating_sub(now))
            .unwrap_or(0);
        VaultStatus {
            state: self.state,
            mode: self.mode,
            keyring_available,
            gated: !gate_open,
            required_level: 4,
            idle_lock_min: self.settings.idle_lock_min,
            max_wrong_attempts: self.settings.max_wrong_attempts,
            cooldown_secs: self.settings.cooldown_secs,
            clipboard_clear_secs: self.settings.clipboard_clear_secs,
            failed_attempts: self.failed_attempts,
            cooldown_remaining_secs,
            locked_reason: self.locked_reason.clone(),
            entries_count: if self.state == VaultState::Unlocked {
                self.entries.len() as u32
            } else {
                0
            },
        }
    }

    // ---- 初始化（仅 unset） ----

    /// 主密码模式初始化：生成随机 salt，空库整体加密落盘，随后立即锁定。
    pub fn setup_master_password(&mut self, master_password: &str) -> Result<(), VaultError> {
        self.ensure_unset()?;
        let salt = random_bytes(SALT_LEN);
        let key = derive_key(master_password.as_bytes(), &salt, self.kdf_iterations);
        self.mode = VaultMode::MasterPassword;
        self.salt = salt;
        self.key = Some(Zeroizing::new(key));
        self.entries = Vec::new();
        self.persist()?;
        self.lock_internal("manual");
        Ok(())
    }

    /// keyring 模式初始化：Tauri 层已把随机 32B key 存入钥匙串并把字节传给 core 落盘。
    pub fn setup_keyring(&mut self, key_bytes: &[u8]) -> Result<(), VaultError> {
        self.ensure_unset()?;
        if key_bytes.len() != KEY_LEN {
            return Err(VaultError::VaultKeyringUnavailable);
        }
        self.mode = VaultMode::Keyring;
        self.salt = Vec::new(); // kdf 字段占位
        self.key = Some(Zeroizing::new(key_bytes.to_vec()));
        self.entries = Vec::new();
        self.persist()?;
        self.lock_internal("manual");
        Ok(())
    }

    // ---- 解锁 / 锁定 ----

    /// 主密码解锁（带防暴力计数与冷却）。冷却期内一切解锁尝试（含正确密码）拒绝。
    pub fn unlock(&mut self, master_password: &str) -> Result<(), VaultError> {
        let now = self.clock.now_secs();
        // 冷却到期自动解除
        if let Some(until) = self.cooldown_until {
            if now >= until {
                self.cooldown_until = None;
                self.failed_attempts = 0;
            }
        }
        if let Some(until) = self.cooldown_until {
            return Err(VaultError::VaultCooldown {
                remaining_secs: until.saturating_sub(now),
            });
        }
        self.ensure_locked()?;
        if self.mode != VaultMode::MasterPassword {
            return Err(VaultError::VaultWrongPassword);
        }
        let key = derive_key(master_password.as_bytes(), &self.salt, self.kdf_iterations);
        match self.open_with_key(&key) {
            Ok(()) => {
                self.failed_attempts = 0;
                self.cooldown_until = None;
                self.touch();
                Ok(())
            }
            Err(VaultError::VaultWrongPassword) => {
                self.failed_attempts += 1;
                if self.failed_attempts >= self.settings.max_wrong_attempts {
                    self.cooldown_until = Some(now + self.settings.cooldown_secs);
                    self.failed_attempts = 0;
                    self.locked_reason = Some("cooldown".to_string());
                }
                Err(VaultError::VaultWrongPassword)
            }
            Err(e) => Err(e),
        }
    }

    /// keyring 模式解锁：Tauri 层从钥匙串取回 key 字节后调用。
    /// 不计数失败（防暴力由系统钥匙串承担），不查冷却。
    pub fn unlock_with_key(&mut self, key_bytes: &[u8]) -> Result<(), VaultError> {
        self.ensure_locked()?;
        if self.mode != VaultMode::Keyring {
            return Err(VaultError::VaultWrongPassword);
        }
        self.open_with_key(key_bytes)?;
        self.touch();
        Ok(())
    }

    /// 手动立即锁定（key zeroize、entries 清空）。
    pub fn lock(&mut self) {
        self.lock_internal("manual");
    }

    /// 闲置超时自动锁（由 tick 驱动；契约：now - last_activity > idle_lock_min*60）。
    /// 返回本次是否发生了自动锁定。
    pub fn tick(&mut self) -> bool {
        let now = self.clock.now_secs();
        if self.state != VaultState::Unlocked {
            return false;
        }
        if now.saturating_sub(self.last_activity) > self.settings.idle_lock_min as u64 * 60 {
            self.lock_internal("idle");
            return true;
        }
        false
    }

    /// 刷新最后活跃时刻（list/add/update/delete/search/copy 均自动调用）。
    pub fn touch(&mut self) {
        self.last_activity = self.clock.now_secs();
    }

    // ---- CRUD（仅 unlocked；均 touch） ----

    pub fn list(&mut self) -> Result<Vec<VaultEntry>, VaultError> {
        self.require_unlocked()?;
        self.touch();
        Ok(self.entries.clone())
    }

    /// 本地过滤 site/account/note：大小写不敏感 ASCII + 中文子串。
    pub fn search(&mut self, q: &str) -> Result<Vec<VaultEntry>, VaultError> {
        self.require_unlocked()?;
        self.touch();
        let needle = q.to_lowercase();
        Ok(self
            .entries
            .iter()
            .filter(|e| {
                e.site.to_lowercase().contains(&needle)
                    || e.account.to_lowercase().contains(&needle)
                    || e.note.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect())
    }

    pub fn add(&mut self, input: VaultEntryInput) -> Result<String, VaultError> {
        self.require_unlocked()?;
        let id = random_hex_id();
        let now = self.clock.now_secs();
        self.entries.push(VaultEntry {
            id: id.clone(),
            site: input.site,
            account: input.account,
            password: input.password,
            note: input.note,
            icon: input.icon,
            updated_at: now,
        });
        self.touch();
        self.persist()?;
        Ok(id)
    }

    /// password 传空串表示不改；其余字段整体替换。
    pub fn update(&mut self, id: &str, input: VaultEntryInput) -> Result<(), VaultError> {
        self.require_unlocked()?;
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(VaultError::VaultNotFound)?;
        e.site = input.site;
        e.account = input.account;
        if !input.password.is_empty() {
            e.password = input.password;
        }
        e.note = input.note;
        e.icon = input.icon;
        e.updated_at = self.clock.now_secs();
        self.touch();
        self.persist()
    }

    pub fn delete(&mut self, id: &str) -> Result<(), VaultError> {
        self.require_unlocked()?;
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            return Err(VaultError::VaultNotFound);
        }
        self.touch();
        self.persist()
    }

    // ---- 剪贴板计时（core 只算时长；OS 写/清由 Tauri/mock 层做） ----

    pub fn copy_hold(&mut self, id: &str) -> Result<VaultCopyHold, VaultError> {
        self.require_unlocked()?;
        let plaintext = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or(VaultError::VaultNotFound)?
            .password
            .clone();
        self.touch();
        let now = self.clock.now_secs();
        let clear_after_secs = self.settings.clipboard_clear_secs;
        Ok(VaultCopyHold {
            plaintext,
            clear_after_secs,
            clear_at: now + clear_after_secs,
        })
    }

    /// 过期判定（供单测 / Tauri 层清除任务轮询）。
    pub fn clipboard_expired(&self, clear_at: u64) -> bool {
        self.clock.now_secs() >= clear_at
    }

    // ---- 导出 / 导入（.qbkv 信封） ----

    /// 生成备份信封字符串（要求 unlocked；Tauri 层写盘为 vault-backup-<ts>.qbkv）。
    pub fn export_payload(&mut self) -> Result<String, VaultError> {
        self.require_unlocked()?;
        self.touch();
        let key = self.key.as_ref().ok_or(VaultError::VaultLocked)?;
        let env = build_envelope(self.mode, &self.salt, self.kdf_iterations, key, &self.entries)?;
        serde_json::to_string(&env).map_err(|e| VaultError::Crypto(e.to_string()))
    }

    /// 用备份自身 kdf+密码解密校验 verifier：
    /// - 本地 unset → 以该备份重建保险箱；
    /// - 本地已初始化（unlocked）→ 以本地 key 重加密落盘，备份内容并入（id 冲突覆盖）。
    pub fn import_payload(
        &mut self,
        payload: &str,
        master_password: &str,
    ) -> Result<(), VaultError> {
        let env: Envelope = serde_json::from_str(payload)
            .map_err(|e| VaultError::VaultImportFailed(format!("信封解析失败: {e}")))?;
        if env.format != "qibai-vault" || env.version != 1 {
            return Err(VaultError::VaultImportFailed("非 qibai-vault 信封".to_string()));
        }
        if env.mode != "master_password" {
            return Err(VaultError::VaultImportFailed(
                "keyring 备份需经钥匙串路径导入".to_string(),
            ));
        }
        let salt = b64_decode(&env.kdf.salt_b64)?;
        let key = derive_key(master_password.as_bytes(), &salt, env.kdf.iterations);
        let entries = match open_envelope(&env, &key) {
            Ok(entries) => entries,
            Err(VaultError::VaultWrongPassword) => {
                return Err(VaultError::VaultImportFailed("备份密码错误".to_string()))
            }
            Err(e) => return Err(e),
        };
        match self.state {
            VaultState::Unset => {
                // 以备份重建：沿用备份 kdf 落盘；derived key 用完即零化，
                // 成功后统一回到 locked，随后用户按本地模式重新解锁。
                self.mode = VaultMode::MasterPassword;
                self.salt = salt;
                self.kdf_iterations = env.kdf.iterations;
                self.key = Some(Zeroizing::new(key));
                self.entries = entries;
                self.persist()?;
                self.discard_unlocked_key();
            }
            VaultState::Locked => {
                // 本地已初始化但未解锁：无本地 key 可重加密 → 结构化拒绝。
                return Err(VaultError::VaultLocked);
            }
            VaultState::Unlocked => {
                // 以本地 key 重加密合并：id 冲突覆盖；成功后同样统一回到 locked。
                for e in entries {
                    match self.entries.iter().position(|x| x.id == e.id) {
                        Some(i) => self.entries[i] = e,
                        None => self.entries.push(e),
                    }
                }
                self.persist()?;
                self.discard_unlocked_key();
            }
        }
        Ok(())
    }

    // ---- 内部 ----

    fn ensure_unset(&self) -> Result<(), VaultError> {
        match self.state {
            VaultState::Unset => Ok(()),
            _ => Err(VaultError::VaultAlreadyUnlocked),
        }
    }

    fn ensure_locked(&self) -> Result<(), VaultError> {
        match self.state {
            VaultState::Unset => Err(VaultError::VaultNotInitialized),
            VaultState::Unlocked => Err(VaultError::VaultAlreadyUnlocked),
            VaultState::Locked => Ok(()),
        }
    }

    fn require_unlocked(&self) -> Result<(), VaultError> {
        match self.state {
            VaultState::Unset => Err(VaultError::VaultNotInitialized),
            VaultState::Locked => Err(VaultError::VaultLocked),
            VaultState::Unlocked => Ok(()),
        }
    }

    /// 用给定 key 校验 verifier 并解出 entries，转入 unlocked。
    fn open_with_key(&mut self, key: &[u8]) -> Result<(), VaultError> {
        let raw =
            std::fs::read_to_string(&self.path).map_err(|e| VaultError::Io(e.to_string()))?;
        let env: Envelope =
            serde_json::from_str(&raw).map_err(|e| VaultError::Crypto(e.to_string()))?;
        let entries = open_envelope(&env, key)?;
        self.key = Some(Zeroizing::new(key.to_vec()));
        self.entries = entries;
        self.state = VaultState::Unlocked;
        self.locked_reason = None;
        Ok(())
    }

    fn lock_internal(&mut self, reason: &str) {
        if self.state == VaultState::Unlocked {
            self.locked_reason = Some(reason.to_string());
        }
        self.state = VaultState::Locked;
        self.key = None; // Zeroizing drop → 主密钥内存清零
        self.entries.clear();
    }

    /// 导入成功后的统一收尾：回到 locked 并零化内存 key/entries；
    /// 不改 locked_reason（契约枚举仅 manual/idle/cooldown/null）。
    fn discard_unlocked_key(&mut self) {
        self.state = VaultState::Locked;
        self.key = None;
        self.entries.clear();
    }

    /// 每次落盘整体重加密；盐固定（同一保险箱沿用同一盐），nonce 每次重新随机。
    fn persist(&self) -> Result<(), VaultError> {
        let key = self.key.as_ref().ok_or(VaultError::VaultLocked)?;
        let env = build_envelope(self.mode, &self.salt, self.kdf_iterations, key, &self.entries)?;
        let raw =
            serde_json::to_string_pretty(&env).map_err(|e| VaultError::Crypto(e.to_string()))?;
        // 问题7-2：原子写（tmp+rename）——vault.qbkv 是加密保险箱文件，写半截即坏库，更需原子。
        crate::fsutil::atomic_write_json(&self.path, &raw).map_err(|e| VaultError::Io(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClock(u64);
    impl Clock for FakeClock {
        fn now_secs(&self) -> u64 {
            self.0
        }
    }

    /// 调用方需在整个测试体持有 `env_lock`。设置 QIBAI_DATA_DIR 到独立临时目录，
    /// 并把 KDF 迭代降到 1000 保证速度（真实 210k 见 kdf_210k_runtime_bench）。
    fn fresh_vault(label: &str) -> VaultManager<FakeClock> {
        let tmp = crate::test_support::unique_temp_dir(label);
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mut v = VaultManager::new(FakeClock(1000), VaultSettings::default());
        v.kdf_iterations = 1_000;
        v
    }

    fn done() {
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    fn input(site: &str, account: &str, password: &str, note: &str, icon: &str) -> VaultEntryInput {
        VaultEntryInput {
            site: site.to_string(),
            account: account.to_string(),
            password: password.to_string(),
            note: note.to_string(),
            icon: icon.to_string(),
        }
    }

    // §9-1 AES-256-GCM 加解密往返还原（含落盘后重建）。
    #[test]
    fn crypto_roundtrip() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-roundtrip");
        v.setup_master_password("s3cret-pass").unwrap();
        assert_eq!(v.state(), VaultState::Locked);
        v.unlock("s3cret-pass").unwrap();
        let id = v
            .add(input("微博", "user@x.com", "pwd123", "", "weibo"))
            .unwrap();
        let list = v.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].password, "pwd123");
        drop(list);

        // 同一数据目录重新打开 → 磁盘 envelope → 解锁还原。
        let mut v2 = VaultManager::load(FakeClock(1000), VaultSettings::default()).unwrap();
        v2.kdf_iterations = 1_000;
        assert_eq!(v2.state(), VaultState::Locked);
        v2.unlock("s3cret-pass").unwrap();
        let restored = v2.list().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, id);
        assert_eq!(restored[0].site, "微博");
        done();
    }

    // §9-2 错误主密码不解锁。
    #[test]
    fn wrong_password_rejected() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-wrongpw");
        v.setup_master_password("right-pw").unwrap();
        match v.unlock("wrong-pw").unwrap_err() {
            VaultError::VaultWrongPassword => {}
            other => panic!("expected VaultWrongPassword, got {other:?}"),
        }
        assert_eq!(v.state(), VaultState::Locked);
        assert!(v.list().is_err());
        v.unlock("right-pw").unwrap();
        done();
    }

    // §9-3 连错 5 次 → 冷却 30s；冷却期内正确密码也拒；冷却结束恢复。
    #[test]
    fn brute_force_cooldown() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-cooldown");
        v.setup_master_password("pw").unwrap();
        for i in 0..5 {
            match v.unlock("bad").unwrap_err() {
                VaultError::VaultWrongPassword => {}
                other => panic!("attempt {i}: expected VaultWrongPassword, got {other:?}"),
            }
        }
        // 第 6 次：冷却期内，正确密码也被拒，并给出剩余秒数。
        match v.unlock("pw").unwrap_err() {
            VaultError::VaultCooldown { remaining_secs } => {
                assert!(remaining_secs > 0 && remaining_secs <= 30, "remaining={remaining_secs}");
            }
            other => panic!("expected VaultCooldown, got {other:?}"),
        }
        // 冷却期内即使多次尝试也仍拒绝。
        assert!(matches!(
            v.unlock("pw").unwrap_err(),
            VaultError::VaultCooldown { .. }
        ));
        // 快进冷却结束 → 恢复。
        v.clock = FakeClock(1000 + 31);
        v.unlock("pw").unwrap();
        assert_eq!(v.state(), VaultState::Unlocked);
        done();
    }

    // §9-4 注入时钟：闲置超时自动锁定；touch 刷新后不再锁。
    #[test]
    fn idle_lock_timeout() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-idle");
        v.setup_master_password("pw").unwrap();
        v.unlock("pw").unwrap();
        v.add(input("微博", "a@x.com", "p", "", "")).unwrap();

        // last_activity=1000；恰好 5 分钟（300s）不算闲置（> 才算）。
        v.clock = FakeClock(1000 + 300);
        assert!(!v.tick());
        assert_eq!(v.state(), VaultState::Unlocked);
        // 再快进 1s → 自动锁定。
        v.clock = FakeClock(1000 + 301);
        assert!(v.tick());
        assert_eq!(v.state(), VaultState::Locked);

        // touch 刷新基线后，从新时刻起算。
        v.unlock("pw").unwrap(); // touch → last_activity = 1301
        v.clock = FakeClock(1301 + 300);
        assert!(!v.tick());
        v.clock = FakeClock(1301 + 301);
        assert!(v.tick());
        assert_eq!(v.state(), VaultState::Locked);
        done();
    }

    // §9-5 copy_hold 返回 clear_after_secs；过期判定正确。
    #[test]
    fn clipboard_timer() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-clip");
        v.setup_master_password("pw").unwrap();
        v.unlock("pw").unwrap();
        let id = v.add(input("微博", "a@x.com", "PLAIN-9f3a", "", "")).unwrap();
        let hold = v.copy_hold(&id).unwrap();
        assert_eq!(hold.clear_after_secs, 20);
        assert_eq!(hold.plaintext, "PLAIN-9f3a");
        assert!(!v.clipboard_expired(hold.clear_at));
        v.clock = FakeClock(hold.clear_at); // 恰好到期
        assert!(v.clipboard_expired(hold.clear_at));
        // 未命中 id → 结构化错误
        assert!(matches!(
            v.copy_hold("no-such-id").unwrap_err(),
            VaultError::VaultNotFound
        ));
        done();
    }

    // §9-6 导出 payload → 重导入数据还原；错误备份密码拒绝。
    #[test]
    fn export_import_roundtrip() {
        let _g = crate::test_support::env_lock();
        let tmp_a = crate::test_support::unique_temp_dir("vault-exp-a");
        let tmp_b = crate::test_support::unique_temp_dir("vault-exp-b");

        // A：初始化 + 加条目 → 导出信封。
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp_a) };
        let mut va = VaultManager::new(FakeClock(1000), VaultSettings::default());
        va.kdf_iterations = 1_000;
        va.setup_master_password("pwA").unwrap();
        va.unlock("pwA").unwrap();
        let id = va
            .add(input("微博", "user@x.com", "EXPORT-PLAIN-123", "备份", "weibo"))
            .unwrap();
        let payload = va.export_payload().unwrap();
        assert!(payload.contains("qibai-vault"));

        // B：全新 unset → 导入备份重建。导入成功后统一回到 locked，需再解锁校验。
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp_b) };
        let mut vb = VaultManager::new(FakeClock(1000), VaultSettings::default());
        vb.kdf_iterations = 1_000;
        assert_eq!(vb.state(), VaultState::Unset);
        vb.import_payload(&payload, "pwA").unwrap();
        assert_eq!(vb.state(), VaultState::Locked); // 统一回到 locked，derived key 已零化
        assert!(vb.list().is_err()); // locked 时不可读
        vb.unlock("pwA").unwrap(); // 按本地模式重新解锁 → 数据还原
        let entries = vb.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].password, "EXPORT-PLAIN-123");

        // 错误备份密码 → 结构化错误，且 vault 仍 unset 未被污染。
        let mut vc = VaultManager::new(FakeClock(1000), VaultSettings::default());
        vc.kdf_iterations = 1_000;
        assert!(matches!(
            vc.import_payload(&payload, "wrong-backup-pw").unwrap_err(),
            VaultError::VaultImportFailed(_)
        ));

        // C：A 库自身仍 unlocked → 再导入同一份备份：以本地 key 重加密合并，
        // 备份条目与本地同 id → 覆盖；导入成功后同样统一回到 locked。
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp_a) };
        va.import_payload(&payload, "pwA").unwrap();
        assert_eq!(va.state(), VaultState::Locked);
        va.unlock("pwA").unwrap();
        assert_eq!(va.list().unwrap().len(), 1);
        assert_eq!(va.list().unwrap()[0].password, "EXPORT-PLAIN-123");
        done();
    }

    // §9-9 keyring key 字节解锁往返（模拟钥匙串）。
    #[test]
    fn keyring_key_path() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-keyring");
        let mut key = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut key);
        v.setup_keyring(&key).unwrap();
        assert_eq!(v.mode(), VaultMode::Keyring);
        assert_eq!(v.state(), VaultState::Locked);
        // 主密码路径不适用
        assert!(v.unlock("whatever").is_err());
        // 正确 key 字节解锁
        v.unlock_with_key(&key).unwrap();
        let id = v.add(input("GitHub", "git@x.com", "gh-pw", "", "")).unwrap();

        // 落盘重建：错误 key 拒绝，正确 key 还原。
        let mut v2 = VaultManager::load(FakeClock(1000), VaultSettings::default()).unwrap();
        let bad = [7u8; KEY_LEN];
        assert!(v2.unlock_with_key(&bad).is_err());
        v2.unlock_with_key(&key).unwrap();
        let entries = v2.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        done();
    }

    // §9-10 落盘文件字节不含明文子串。
    #[test]
    fn no_plaintext_on_disk() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-noplain");
        v.setup_master_password("pw").unwrap();
        v.unlock("pw").unwrap();
        v.add(input("唯一站点Zxq", "acct-唯一", "MARKER-PLAIN-9f3a2c1b", "备注唯一N", "")).unwrap();
        drop(v);

        let raw = std::fs::read(crate::settings::data_dir().unwrap().join(VAULT_FILE)).unwrap();
        let raw_s = String::from_utf8_lossy(&raw);
        assert!(!raw_s.contains("MARKER-PLAIN-9f3a2c1b"));
        assert!(!raw_s.contains("唯一站点Zxq"));
        assert!(!raw_s.contains("acct-唯一"));
        // verifier 常量与 AAD 允许出现（非秘密）。
        assert!(raw_s.contains("qibai-vault-v1"));
        done();
    }

    // §9-11a KDF 迭代常量 ≥ 210000。
    #[test]
    fn kdf_iterations_const() {
        assert!(KDF_ITERATIONS >= 210_000, "KDF_ITERATIONS={KDF_ITERATIONS}");
    }

    // §9-11b 真实 210000 迭代耗时记录（宽松上限，防 CI 抖动）。
    #[test]
    fn kdf_210k_runtime_bench() {
        let t0 = std::time::Instant::now();
        let _ = derive_key(b"bench-password-example", &[7u8; SALT_LEN], KDF_ITERATIONS);
        let ms = t0.elapsed().as_millis();
        println!("PBKDF2-SHA256 iterations={KDF_ITERATIONS} elapsed={ms} ms");
        assert!(ms < 8_000, "KDF 210k 过慢: {ms}ms");
    }

    // §9-12 增删改查 + 中文/大小写检索 + 空密码不改。
    #[test]
    fn entry_crud_search() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-crud");
        v.setup_master_password("pw").unwrap();
        v.unlock("pw").unwrap();
        let id1 = v.add(input("微博", "a@x.com", "p1", "追剧", "weibo")).unwrap();
        let id2 = v.add(input("GitHub", "git@x.com", "p2", "代码托管", "")).unwrap();
        let _id3 = v.add(input("Google", "g@x.com", "p3", "", "")).unwrap();

        // 中文子串
        assert_eq!(v.search("微博").unwrap().len(), 1);
        // ASCII 大小写不敏感
        assert_eq!(v.search("github").unwrap().len(), 1);
        assert_eq!(v.search("GITHUB").unwrap().len(), 1);
        // note 命中
        assert_eq!(v.search("代码").unwrap().len(), 1);
        // 无命中
        assert!(v.search("不存在的站点").unwrap().is_empty());

        // update：password 传空串 → 不改；其余字段替换。
        v.update(&id1, input("微博-小号", "a2@x.com", "", "追剧", "")).unwrap();
        let after = v.search("微博-小号").unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].account, "a2@x.com");
        assert_eq!(after[0].password, "p1"); // 未改

        // delete + 未命中结构化错误
        v.delete(&id2).unwrap();
        assert!(v.search("github").unwrap().is_empty());
        assert!(matches!(v.delete("nope").unwrap_err(), VaultError::VaultNotFound));
        assert_eq!(v.list().unwrap().len(), 2);
        done();
    }

    // §9-补充 回归：保存条目后 lock() → 再用主密码 unlock() → 条目仍在且可解密。
    // 验证每次落盘整体重加密（盐固定、nonce 重随机）后，解锁派生仍正确。
    #[test]
    fn relock_after_save_keeps_data() {
        let _g = crate::test_support::env_lock();
        let mut v = fresh_vault("vault-relock");
        v.setup_master_password("pw").unwrap();
        v.unlock("pw").unwrap();
        let id = v
            .add(input("微博", "a@x.com", "RELOCK-PLAIN-7b2e", "", "weibo"))
            .unwrap();

        // 手动锁定 → 内存 key/entries 清零
        v.lock();
        assert_eq!(v.state(), VaultState::Locked);
        assert!(v.list().is_err());

        // 再用主密码解锁（读文件头盐重新派生 key）→ 条目仍在且密文可解密
        v.unlock("pw").unwrap();
        assert_eq!(v.state(), VaultState::Unlocked);
        let entries = v.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].password, "RELOCK-PLAIN-7b2e");

        // 错误密码在重锁后依然拒绝
        assert!(v.unlock("pw").is_err()); // 已 unlocked
        done();
    }
}
