//! 设置持久化。
//!
//! JSON 文件存放于平台应用数据目录：
//! - Linux:   `~/.local/share/qibai/settings.json`
//! - Windows: `%APPDATA%\qibai\settings.json`
//! - macOS:   `~/Library/Application Support/qibai/settings.json`
//!
//! 测试时可用环境变量 `QIBAI_DATA_DIR` 覆盖数据目录（指向临时目录）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot resolve home/data directory")]
    NoHome,
}

fn default_true() -> bool {
    true
}

fn default_intimacy() -> u8 {
    1
}

fn default_audio_output() -> String {
    "default".to_string()
}

fn default_provider() -> String {
    "qwen_local".to_string()
}

fn default_model() -> String {
    "deepseek-chat".to_string()
}

/// LLM 提供商配置。api_key 为空字符串时，运行时优先读环境变量 `DOUBAO_API_KEY`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// v45.9：旧 settings.json 可能没有 provider/base_url/model 三者，缺字段按默认值加载，
    /// 避免升级后整份配置读失败（曾经缺 provider 会直接 `missing field` 报错）。
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// 连续对话记忆开关（v30：默认开启——总是携带最近 50 条对话历史 + 角色长期记忆摘要）。
    /// false 仅用于测试显式构造（llm_client 单测验证"不带历史"路径）。
    #[serde(default)]
    pub remember_chat: bool,
    /// v18.8-fix24：备用大模型服务商（欠费/失败自动切换；空=无备用）。
    #[serde(default)]
    pub backup_providers: Vec<BackupLlmConfig>,
}

/// 备用大模型服务商配置（OpenAI 兼容端点）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupLlmConfig {
    pub name: String,
    #[serde(rename = "base_url")]
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider: "qwen_local".to_string(),
            // 隐私路由：不再硬编码内网明文 HTTP IP 端点（100.92.x.x 为开发机临时地址）。
            // 默认空——全新用户/损坏回退时必须显式在设置页填 base_url，否则 LLM 调用报错提示配置。
            // 已有 settings.json 的用户不受影响（文件里的值原样加载）。
            base_url: String::new(),
            model: "Qwen/Qwen3.6-35B-A3B".to_string(),
            api_key: String::new(),
            // v30：连续对话记忆默认开启（用户明确诉求）；老配置文件已序列化的 false 值
            // 由 Engine 侧强制记忆逻辑兜底（见 engine.rs send_message 注释）。
            remember_chat: true,
            backup_providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderEnabled {
    pub sit: bool,
    pub water: bool,
    pub eye: bool,
}

impl Default for ReminderEnabled {
    fn default() -> Self {
        Self {
            sit: true,
            water: true,
            eye: true,
        }
    }
}

/// 久坐 / 喝水 / 护眼提醒间隔（分钟）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderSettings {
    #[serde(rename = "sit_min")]
    pub sit_min: u32,
    #[serde(rename = "water_min")]
    pub water_min: u32,
    #[serde(rename = "eye_min")]
    pub eye_min: u32,
    pub enabled: ReminderEnabled,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            sit_min: 45,
            water_min: 30,
            eye_min: 20,
            enabled: ReminderEnabled::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSettings {
    /// 用户是否已确认成年年龄。未确认时人设会输出年龄声明提示。
    #[serde(default)]
    #[serde(rename = "age_confirmed")]
    pub age_confirmed: bool,
    /// 未成年人纯净模式：全龄友好、不做情感越界、弱化恋爱表述。
    #[serde(default)]
    #[serde(rename = "pure_mode")]
    pub pure_mode: bool,
    /// 问题6：桌宠窗口靠近屏幕边缘时自动吸附（默认 true；旧 settings 缺字段 serde default 补 true）。
    #[serde(default = "default_true")]
    #[serde(rename = "snap_to_edge")]
    pub snap_to_edge: bool,
    /// v31.2：立绘背景透明模式（默认 true——用户真机已确认带合成器）。
    /// 合成器环境下 canvas 透明区由合成器正确合成（露桌面），配合 28px 圆角
    /// 即纯立绘圆角浮窗；无合成器 X11 下透明会黑底/重影（重影根治依赖不透明
    /// 覆盖），故保留开关，右键菜单可随时关闭退回圆角浅色卡片。
    #[serde(default = "default_transparent_bg")]
    #[serde(rename = "transparent_bg")]
    pub transparent_bg: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            age_confirmed: false,
            pure_mode: false,
            snap_to_edge: true,
            transparent_bg: true,
        }
    }
}

/// 旧版 settings.json 缺少 transparent_bg 字段时的反序列化兜底：
/// 用户系统带合成器，透明背景默认开启。
fn default_transparent_bg() -> bool { true }

/// 技能权限：总开关 + 每技能开关。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillsSettings {
    #[serde(rename = "total_enabled", default = "default_true")]
    pub total_enabled: bool,
    #[serde(default)]
    #[serde(rename = "launch_app")]
    pub launch_app: bool,
    #[serde(default)]
    #[serde(rename = "open_folder")]
    pub open_folder: bool,
    #[serde(default)]
    #[serde(rename = "set_reminder")]
    pub set_reminder: bool,
    /// 代码检查（只读安全技能，无需审批）；旧 settings.json 无此字段时默认为 true。
    #[serde(default = "default_true")]
    #[serde(rename = "check_code")]
    pub check_code: bool,
    /// 打开联网社交（只读安全技能，无需审批）；旧 settings.json 无此字段时默认为 true。
    #[serde(default = "default_true")]
    #[serde(rename = "open_social")]
    pub open_social: bool,
    /// 联网搜索（只读，三引擎降级链）；旧 settings.json 无此字段时默认为 true。
    #[serde(default = "default_true")]
    #[serde(rename = "web_search")]
    pub web_search: bool,
    /// 网页阅读器（fetch_page：抓取网页正文供总结）；默认开启。
    #[serde(default = "default_true")]
    #[serde(rename = "fetch_page")]
    pub fetch_page: bool,
    /// 文件/文件夹操作（限 $HOME 沙盒，删除走审批）；旧 settings.json 无此字段时默认为 true。
    #[serde(default = "default_true")]
    #[serde(rename = "file_ops")]
    pub file_ops: bool,
    /// v24 编程技能：git 工作区状态（只读）；默认开启。
    #[serde(default = "default_true")]
    #[serde(rename = "git_status")]
    pub git_status: bool,
    /// v24 编程技能：运行测试/检查命令（危险，执行需审批）；默认开启（总开关+审批双保险）。
    #[serde(default = "default_true")]
    #[serde(rename = "run_tests")]
    pub run_tests: bool,
    /// v24 编程技能：代码扫描搜索（只读）；默认开启。
    #[serde(default = "default_true")]
    #[serde(rename = "code_scan")]
    pub code_scan: bool,
}

// ---- 迭代③：本地密码保险箱设置（VAULT_SPEC §2） ----

fn default_idle_lock_min() -> u32 {
    5
}

fn default_max_wrong_attempts() -> u32 {
    5
}

fn default_cooldown_secs() -> u64 {
    30
}

fn default_clipboard_clear_secs() -> u64 {
    20
}

/// 保险箱行为参数。旧 settings.json 无 `vault` 段时按默认值加载（serde default）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultSettings {
    /// 闲置自动锁定（分钟，可配 1..=120）。
    #[serde(default = "default_idle_lock_min")]
    #[serde(rename = "idle_lock_min")]
    pub idle_lock_min: u32,
    /// 连续输错主密码次数，达到后进入冷却。
    #[serde(default = "default_max_wrong_attempts")]
    #[serde(rename = "max_wrong_attempts")]
    pub max_wrong_attempts: u32,
    /// 冷却秒数。
    #[serde(default = "default_cooldown_secs")]
    #[serde(rename = "cooldown_secs")]
    pub cooldown_secs: u64,
    /// 剪贴板自动清除秒数（core 只计时，OS 写/清由 Tauri/mock 层执行）。
    #[serde(default = "default_clipboard_clear_secs")]
    #[serde(rename = "clipboard_clear_secs")]
    pub clipboard_clear_secs: u64,
}

impl Default for VaultSettings {
    fn default() -> Self {
        Self {
            idle_lock_min: 5,
            max_wrong_attempts: 5,
            cooldown_secs: 30,
            clipboard_clear_secs: 20,
        }
    }
}

impl Default for SkillsSettings {
    fn default() -> Self {
        // 默认开放安全技能
        Self {
            total_enabled: true,
            launch_app: true,
            open_folder: false,  // v43：默认关闭，避免闲聊被误判为“打开文件夹”而弹出资源管理器
            set_reminder: true,
            check_code: true,
            open_social: true,
            web_search: true,
            fetch_page: true,
            file_ops: true,
            git_status: true,
            run_tests: true,
            code_scan: true,
        }
    }
}

/// 插件体系设置（契约 §2）。旧 settings.json 无该块时按默认空表加载（serde default）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSettings {
    /// 插件 id → 启停（含内置 `com.qibai.*`）。
    #[serde(default)]
    pub enabled: HashMap<String, bool>,
    /// 插件 id → 已批准权限列表（安装确认结果；内置插件 grants 由注册表默认注入，不落盘）。
    #[serde(default)]
    pub grants: HashMap<String, Vec<String>>,
    /// 专家 id → 启用状态；value=true 表示该专家为当前激活（active 特判）。
    #[serde(default)]
    pub experts: HashMap<String, bool>,
}

/// 顶层设置对象，与前端 SettingsJson 契约一一对应。
/// 注：含 voice.rate(f32)，无法实现 Eq（NaN），故仅 PartialEq。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub llm: LlmSettings,
    pub reminders: ReminderSettings,
    pub ui: UiSettings,
    pub skills: SkillsSettings,
    /// 全屏守护演出设置。旧 settings.json 无该字段时按默认值加载。
    #[serde(default)]
    pub guard: crate::guard::GuardSettings,
    /// 亲密度 L1-L10。本期仅预留字段，不实现成长逻辑。
    #[serde(default = "default_intimacy")]
    #[serde(rename = "intimacy_level")]
    pub intimacy_level: u8,
    /// 语音朗读 / STT 设置（迭代②）。旧 settings.json 无该字段时按默认值加载。
    #[serde(default)]
    pub voice: crate::tts::VoiceSettings,
    /// 本地密码保险箱设置（迭代③）。旧 settings.json 无 `vault` 段时按默认值加载。
    #[serde(default)]
    pub vault: VaultSettings,
    /// 插件 / 专家体系设置（契约 §2）。旧 settings.json 无 `plugins` 段时按默认空表加载。
    #[serde(default)]
    pub plugins: PluginSettings,
    /// 用户自定义连接器配置（MCP / HTTP 连接器）。旧 settings.json 无 `connectors` 段时为空。
    #[serde(default)]
    pub connectors: Vec<crate::connectors::ConnectorConfig>,
    /// v45.12：MCP（外部工具）服务器列表。默认空数组——**未配置时绝不 spawn 任何子进程**。
    /// 与 `connectors`（本地插件包体系）完全并行：本字段是第二条独立接入通道。
    /// 旧 settings.json 无该字段时按空数组加载（serde default），升级零影响。
    #[serde(default)]
    pub mcp_servers: Vec<crate::mcp::McpServerConfig>,
    /// 输出设备名（sink name 或 `hw:CARD=...`；空 / "default" = 系统默认）。
    /// v45.9：旧 settings.json 无该字段时按 "default" 加载，不读崩。
    #[serde(default = "default_audio_output")]
    pub audio_output: String,

    // ===== v45.9 统一 B 类设置（单份落盘，全部 serde default 向后兼容）=====
    /// 搜索引擎偏好：auto|baidu|360|sogou。
    #[serde(default = "default_search_engine")]
    pub search_engine: String,
    /// HTTP 超时（搜索 + 抓页共用），秒。
    #[serde(default = "default_http_timeout_secs")]
    pub http_timeout_secs: u64,
    /// 抓页最大字节数。
    #[serde(default = "default_page_max_bytes")]
    pub page_max_bytes: u64,
    /// 搜索结果返回条数。
    #[serde(default = "default_search_result_n")]
    pub search_result_n: usize,
    /// 抓页并发数。
    #[serde(default = "default_fetch_concurrency")]
    pub fetch_concurrency: usize,
    /// 抓页重试次数。
    #[serde(default = "default_fetch_retries")]
    pub fetch_retries: usize,
    /// LLM API 地址。
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    /// HuggingFace 镜像前缀。
    #[serde(default = "default_hf_mirror_url")]
    pub hf_mirror_url: String,
    /// TTS 分段区间（字）。
    #[serde(default = "default_tts_seg_min")]
    pub tts_seg_min: usize,
    #[serde(default = "default_tts_seg_max")]
    pub tts_seg_max: usize,
    /// TTS 语速 / 音调。
    #[serde(default = "default_tts_speed")]
    pub tts_speed: f32,
    #[serde(default = "default_tts_pitch")]
    pub tts_pitch: f32,
    /// TTS 请求超时，秒。
    #[serde(default = "default_tts_timeout_secs")]
    pub tts_timeout_secs: u64,
    /// Whisper 模型下载 URL。
    #[serde(default = "default_whisper_model_url")]
    pub whisper_model_url: String,
    /// 对话保留最近 N 条。
    #[serde(default = "default_history_recent_n")]
    pub history_recent_n: usize,
    /// LLM temperature。
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// LLM max_tokens。
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// 长文注入 prompt 的最大段数。
    #[serde(default = "default_longtext_inject_max")]
    pub longtext_inject_max: usize,
    /// 执行权限白名单（路径前缀）。
    #[serde(default = "default_exec_whitelist")]
    pub exec_whitelist: Vec<String>,
    /// 单次操作文件数上限。
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    /// 单文件字节上限。
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    /// 总字节上限。
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: u64,
    /// 命令冷却，秒。
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    /// 点击每日上限。
    #[serde(default = "default_click_daily_limit")]
    pub click_daily_limit: u32,
    /// 陪伴每日上限。
    #[serde(default = "default_companion_daily_cap")]
    pub companion_daily_cap: u32,
    /// 倒计时范围。
    #[serde(default = "default_countdown_min")]
    pub countdown_min: u32,
    #[serde(default = "default_countdown_max")]
    pub countdown_max: u32,

    // ===== v45.9 P0：长任务 / 上下文组装 =====
    #[serde(default = "default_context_budget_tokens")]
    pub context_budget_tokens: u64,
    #[serde(default = "default_context_recent_turns")]
    pub context_recent_turns: usize,
    #[serde(default = "default_task_max_steps")]
    pub task_max_steps: u32,
    #[serde(default = "default_task_max_minutes")]
    pub task_max_minutes: u32,
    /// v45.9：任务 token 预算（默认 200k）。
    #[serde(default = "default_task_max_tokens")]
    pub task_max_tokens: u32,
    /// v45.9：单步失败自动重试次数（默认 2 = 最多 3 次尝试）。
    #[serde(default = "default_task_auto_retry")]
    pub task_auto_retry: u32,
    /// v45.13：任务内并行分支上限（1..=8，默认 3）。
    /// 并行分支共享父任务的 token / 步数 / 时间预算池；0 或越界在 clamp_in_place 里回 3 并留日志。
    #[serde(default = "default_task_parallel_max")]
    pub task_parallel_max: u32,
    /// v45.9 P0：是否启用 context::assemble 四层组装（false = 走旧 prompt 拼接）。
    #[serde(default = "default_context_layers_enabled")]
    pub context_layers_enabled: bool,
    /// v45.10 P1：知识库根目录列表。
    #[serde(default)]
    pub kb_folders: Vec<String>,
    /// v45.10 P1：检索 top_k。
    #[serde(default = "default_kb_top_k")]
    pub kb_top_k: usize,
    /// v45.10 P1：单块目标字数。
    #[serde(default = "default_kb_chunk_chars")]
    pub kb_chunk_chars: usize,
    /// v45.10 P1：块数硬上限（超过 reindex 报错，不静默截断）。
    #[serde(default = "default_kb_max_chunks")]
    pub kb_max_chunks: usize,
    /// P2：语义检索开关。true 且本机已装向量模型时才走「向量 + BM25」混合检索，
    /// 否则一律纯 BM25（模型缺席不影响检索可用性）。
    #[serde(default = "default_kb_embed_enabled")]
    pub kb_embed_enabled: bool,
    /// P2：混合检索里向量权重（0..1）。默认 0.45：语义与字面各占一半，字面略重。
    #[serde(default = "default_kb_hybrid_weight")]
    pub kb_hybrid_weight: f32,
}

fn default_context_layers_enabled() -> bool { true }
fn default_task_max_tokens() -> u32 { 200_000 }
fn default_task_auto_retry() -> u32 { 2 }
fn default_task_parallel_max() -> u32 { 3 }
fn default_kb_top_k() -> usize { 6 }
fn default_kb_chunk_chars() -> usize { 700 }
fn default_kb_max_chunks() -> usize { 20_000 }
fn default_kb_embed_enabled() -> bool { true }
fn default_kb_hybrid_weight() -> f32 { 0.45 }

fn default_search_engine() -> String { "auto".into() }
fn default_http_timeout_secs() -> u64 { 8 }
fn default_page_max_bytes() -> u64 { 8 * 1024 * 1024 }
fn default_search_result_n() -> usize { 8 }
fn default_fetch_concurrency() -> usize { 4 }
fn default_fetch_retries() -> usize { 1 }
fn default_api_base_url() -> String { "https://ark.cn-beijing.volces.com/api/v3/chat/completions".into() }
fn default_hf_mirror_url() -> String { "https://hf-mirror.com".into() }
fn default_tts_seg_min() -> usize { 40 }
fn default_tts_seg_max() -> usize { 80 }
fn default_tts_speed() -> f32 { 1.0 }
fn default_tts_pitch() -> f32 { 1.0 }
fn default_tts_timeout_secs() -> u64 { 30 }
fn default_whisper_model_url() -> String { "https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin".into() }
fn default_history_recent_n() -> usize { 50 }
fn default_temperature() -> f32 { 0.7 }
fn default_max_tokens() -> u32 { 4096 }
fn default_longtext_inject_max() -> usize { 6 }
fn default_exec_whitelist() -> Vec<String> { vec!["~/Documents/qibai".into()] }
fn default_max_files() -> usize { 32 }
fn default_max_file_bytes() -> u64 { 8 * 1024 * 1024 }
fn default_max_total_bytes() -> u64 { 32 * 1024 * 1024 }
fn default_click_daily_limit() -> u32 { 100 }
fn default_companion_daily_cap() -> u32 { 20 }
fn default_countdown_min() -> u32 { 3 }
fn default_countdown_max() -> u32 { 60 }
fn default_context_budget_tokens() -> u64 { 8000 }
fn default_context_recent_turns() -> usize { 10 }
fn default_task_max_steps() -> u32 { 20 }
fn default_task_max_minutes() -> u32 { 60 }

impl Default for Settings {
    fn default() -> Self {
        Self {
            llm: LlmSettings::default(),
            reminders: ReminderSettings::default(),
            ui: UiSettings::default(),
            skills: SkillsSettings::default(),
            guard: crate::guard::GuardSettings::default(),
            intimacy_level: 1,
            voice: crate::tts::VoiceSettings::default(),
            vault: VaultSettings::default(),
            plugins: PluginSettings::default(),
            connectors: vec![],
            mcp_servers: Vec::new(), // v45.12：默认空——零子进程
            audio_output: "default".to_string(),
            search_engine: default_search_engine(),
            http_timeout_secs: default_http_timeout_secs(),
            page_max_bytes: default_page_max_bytes(),
            search_result_n: default_search_result_n(),
            fetch_concurrency: default_fetch_concurrency(),
            fetch_retries: default_fetch_retries(),
            api_base_url: default_api_base_url(),
            hf_mirror_url: default_hf_mirror_url(),
            tts_seg_min: default_tts_seg_min(),
            tts_seg_max: default_tts_seg_max(),
            tts_speed: default_tts_speed(),
            tts_pitch: default_tts_pitch(),
            tts_timeout_secs: default_tts_timeout_secs(),
            whisper_model_url: default_whisper_model_url(),
            history_recent_n: default_history_recent_n(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            longtext_inject_max: default_longtext_inject_max(),
            exec_whitelist: default_exec_whitelist(),
            max_files: default_max_files(),
            max_file_bytes: default_max_file_bytes(),
            max_total_bytes: default_max_total_bytes(),
            cooldown_secs: default_cooldown_secs(),
            click_daily_limit: default_click_daily_limit(),
            companion_daily_cap: default_companion_daily_cap(),
            countdown_min: default_countdown_min(),
            countdown_max: default_countdown_max(),
            context_budget_tokens: default_context_budget_tokens(),
            context_recent_turns: default_context_recent_turns(),
            task_max_steps: default_task_max_steps(),
            task_max_minutes: default_task_max_minutes(),
            task_max_tokens: default_task_max_tokens(),
            task_auto_retry: default_task_auto_retry(),
            task_parallel_max: default_task_parallel_max(),
            context_layers_enabled: default_context_layers_enabled(),
            kb_folders: Vec::new(),
            kb_top_k: default_kb_top_k(),
            kb_chunk_chars: default_kb_chunk_chars(),
            kb_max_chunks: default_kb_max_chunks(),
            kb_embed_enabled: default_kb_embed_enabled(),
            kb_hybrid_weight: default_kb_hybrid_weight(),
        }
    }
}

impl Settings {
    /// 边界夹取：加载后 / 保存前调一次，把用户填的越界值拉回合法区间。
    pub fn clamp_in_place(&mut self) {
        self.http_timeout_secs = self.http_timeout_secs.clamp(1, 60);
        self.page_max_bytes = self.page_max_bytes.clamp(64 * 1024, 64 * 1024 * 1024);
        self.search_result_n = self.search_result_n.clamp(1, 20);
        self.fetch_concurrency = self.fetch_concurrency.clamp(1, 16);
        self.fetch_retries = self.fetch_retries.clamp(0, 5);
        self.tts_seg_min = self.tts_seg_min.clamp(20, 120);
        self.tts_seg_max = self.tts_seg_max.max(self.tts_seg_min + 10).clamp(30, 200);
        self.tts_speed = self.tts_speed.clamp(0.5, 2.0);
        self.tts_pitch = self.tts_pitch.clamp(0.5, 2.0);
        self.tts_timeout_secs = self.tts_timeout_secs.clamp(5, 120);
        self.history_recent_n = self.history_recent_n.clamp(5, 200);
        self.temperature = self.temperature.clamp(0.0, 2.0);
        self.max_tokens = self.max_tokens.clamp(256, 32768);
        self.longtext_inject_max = self.longtext_inject_max.clamp(1, 32);
        if self.exec_whitelist.is_empty() {
            self.exec_whitelist = default_exec_whitelist();
        }
        self.max_files = self.max_files.clamp(1, 256);
        self.max_file_bytes = self.max_file_bytes.clamp(64 * 1024, 256 * 1024 * 1024);
        self.max_total_bytes = self.max_total_bytes.max(self.max_file_bytes).clamp(1 * 1024 * 1024, 1024 * 1024 * 1024);
        self.cooldown_secs = self.cooldown_secs.clamp(0, 300);
        self.click_daily_limit = self.click_daily_limit.clamp(0, 10000);
        self.companion_daily_cap = self.companion_daily_cap.clamp(0, 1000);
        self.countdown_min = self.countdown_min.clamp(1, 60);
        self.countdown_max = self.countdown_max.max(self.countdown_min + 1).clamp(2, 600);
        // search_engine 白名单
        if !matches!(self.search_engine.as_str(), "auto" | "baidu" | "360" | "so360" | "sogou") {
            self.search_engine = "auto".into();
        }
        // v45.10 P1 kb 范围
        self.kb_top_k = self.kb_top_k.clamp(1, 20);
        self.kb_chunk_chars = self.kb_chunk_chars.clamp(500, 800);
        self.kb_max_chunks = self.kb_max_chunks.clamp(100, 200_000);
        self.kb_hybrid_weight = self.kb_hybrid_weight.clamp(0.0, 1.0);
        // v45.13：并行上限——合法区间 1..=8 内不动；0 或越界（>8）一律回默认 3 并留日志。
        if self.task_parallel_max == 0 || self.task_parallel_max > 8 {
            eprintln!(
                "[qibai] settings clamp: task_parallel_max={} 越界，夹回默认 3",
                self.task_parallel_max
            );
            self.task_parallel_max = 3;
        }
        // audio_output 空串归一
        if self.audio_output.is_empty() {
            self.audio_output = "default".into();
        }
        // ===== v45.12：MCP 配置清洗（只剔除/降级，不做危险改写）=====
        // 原则：name 为空 / command 为空 → 整条剔除并留日志；transport 非 "stdio"
        // （含占位的 "http"）→ 降级为 disabled（绝不 spawn）；trust 仅两档由枚举类型保证。
        let mut mcp_kept = Vec::with_capacity(self.mcp_servers.len());
        for mut cfg in self.mcp_servers.drain(..) {
            if cfg.name.trim().is_empty() {
                eprintln!("[qibai] mcp clamp: 丢弃 name 为空的 server（command={}）", cfg.command);
                continue;
            }
            if cfg.command.trim().is_empty() {
                eprintln!("[qibai] mcp clamp: 丢弃 server「{}」：command 为空", cfg.name);
                continue;
            }
            if cfg.transport != "stdio" {
                // http 等远程传输暂未实现：置 disabled，不让 registry 误 spawn。
                eprintln!(
                    "[qibai] mcp clamp: server「{}」transport={} 暂不支持，降级为 disabled（仅支持 stdio）",
                    cfg.name, cfg.transport
                );
                cfg.enabled = false;
            }
            // trust 只允许 readonly / confirm 两档（serde 枚举已保证第三值无法反序列化）。
            let _ = cfg.trust;
            mcp_kept.push(cfg);
        }
        self.mcp_servers = mcp_kept;
    }
}

/// 解析应用数据目录。测试可用 QIBAI_DATA_DIR 覆盖。
pub fn data_dir() -> Result<PathBuf, SettingsError> {
    if let Ok(override_dir) = std::env::var("QIBAI_DATA_DIR") {
        let p = PathBuf::from(override_dir);
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }
    let dir = directories::ProjectDirs::from("com", "Qibai", "Qibai")
        .ok_or(SettingsError::NoHome)?
        .data_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn settings_path() -> Result<PathBuf, SettingsError> {
    Ok(data_dir()?.join("settings.json"))
}

impl Settings {
    /// 读取设置；文件不存在或损坏时回退到默认设置（不崩溃）。
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(s) => s,
            Err(_) => Self::default(),
        }
    }

    pub fn try_load() -> Result<Self, SettingsError> {
        let path = settings_path()?;
        let raw = std::fs::read_to_string(&path)?;
        // 问题7-2：JSON 损坏 → fsutil 备份坏件为 .corrupt-<ts> 后返回 Err（load() 兜成默认）；
        // 不再静默覆盖丢失用户配置。IO 错误仍向上抛。
        let mut s: Self = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => {
                crate::fsutil::load_json_or_backup::<Self>(&path);
                return Err(e.into());
            }
        };
        // v18.8-fix4：唤醒词迁移——旧版默认「栖伴」已被新版「小小」取代；
        // 若设置文件仍存旧值，改成新默认（否则用户喊"小小"无效）。
        if s.voice.wake_word == "栖伴" || s.voice.wake_word.is_empty() {
            s.voice.wake_word = "小小".to_string();
        }
        // v43：旧安装的 settings.json 里 open_folder 可能仍是 true，会在闲聊被
        // LLM 误判时弹出文件夹。首次升级强制关闭一次并写标记；用户之后可在
        // 控制台显式重新开启，不会被反复覆盖。
        let marker = path.with_file_name(".open_folder_disabled_v43");
        if !marker.exists() {
            if s.skills.open_folder {
                s.skills.open_folder = false;
                let _ = s.save();
                eprintln!("[qibai] v43 migration: open_folder disabled");
            }
            let _ = std::fs::write(&marker, b"1");
        }
        Ok(s)
    }

    pub fn save(&self) -> Result<PathBuf, SettingsError> {
        let path = settings_path()?;
        let raw = serde_json::to_string_pretty(self)?;
        // 问题7-2：原子写（tmp+rename），防写半截损坏 settings.json。
        crate::fsutil::atomic_write_json(&path, &raw)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        // 隐私路由：默认 base_url/api_key 为空（不再硬编码内网 IP/测试 key），需用户显式配置。
        assert_eq!(s.llm.provider, "qwen_local");
        assert_eq!(s.llm.base_url, "");
        assert_eq!(s.llm.model, "Qwen/Qwen3.6-35B-A3B");
        assert_eq!(s.llm.api_key, "");
        assert!(s.skills.total_enabled);
        assert_eq!(s.intimacy_level, 1);
    }

    #[test]
    fn round_trip_via_temp_dir() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        let mut s = Settings::default();
        s.reminders.sit_min = 60;
        s.ui.pure_mode = true;
        s.intimacy_level = 7;
        let path = s.save().unwrap();
        assert!(path.exists());
        let loaded = Settings::load();
        assert_eq!(loaded.reminders.sit_min, 60);
        assert!(loaded.ui.pure_mode);
        assert_eq!(loaded.intimacy_level, 7);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn corrupted_file_falls_back_to_defaults() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-bad");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        std::fs::write(tmp.join("settings.json"), "{{{not json").unwrap();
        let s = Settings::load();
        assert_eq!(s, Settings::default());
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// guard 字段 round-trip：自定义值落盘后原样读回；非法 countdown_secs
    /// 在落盘时保留原始值，clamp 语义由 `GuardSettings::sanitize()` / 调度器入口完成。
    #[test]
    fn guard_field_round_trip_and_countdown_clamp_semantics() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-guard");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };

        let mut s = Settings::default();
        s.guard.sit_min = 90;
        s.guard.mode = crate::guard::GuardMode::Light;
        s.guard.end_action = crate::guard::GuardEndAction::Lock;
        s.guard.countdown_secs = 9999; // 超上限，先原样落盘

        s.save().unwrap();
        let loaded = Settings::load();
        assert_eq!(loaded.guard.sit_min, 90);
        assert_eq!(loaded.guard.mode, crate::guard::GuardMode::Light);
        assert_eq!(loaded.guard.end_action, crate::guard::GuardEndAction::Lock);
        // 反序列化保留原始非法值……
        assert_eq!(loaded.guard.countdown_secs, 9999);
        // ……但经 sanitize 后 clamp 到上限
        assert_eq!(
            loaded.guard.clone().sanitize().countdown_secs,
            crate::guard::COUNTDOWN_MAX
        );
        // 下限同理
        let mut too_small = loaded.guard.clone();
        too_small.countdown_secs = 10;
        assert_eq!(too_small.sanitize().countdown_secs, crate::guard::COUNTDOWN_MIN);

        // check_code 默认 true，且能显式写 false 并 round-trip
        assert!(loaded.skills.check_code);
        let mut s2 = Settings::load();
        s2.skills.check_code = false;
        s2.save().unwrap();
        assert!(!Settings::load().skills.check_code);

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 旧版 settings.json（无 guard 字段、skills 无 check_code 字段）必须平滑加载，
    /// 不得因缺字段而回退默认或崩溃。
    #[test]
    fn legacy_settings_without_guard_or_check_code_loads() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-legacy");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };

        // 手工拼一份“迭代①之前”的 settings：skills 只有三个技能、无 guard。
        let legacy = serde_json::json!({
            "llm": {
                "provider": "doubao",
                "base_url": "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
                "model": "doubao-pro-32k"
            },
            "reminders": {
                "sit_min": 45, "water_min": 30, "eye_min": 20,
                "enabled": {"sit": true, "water": true, "eye": true}
            },
            "ui": {"age_confirmed": true, "pure_mode": false},
            "skills": {
                "total_enabled": true,
                "launch_app": true,
                "open_folder": false,
                "set_reminder": true
            },
            "intimacy_level": 3
        });
        std::fs::write(
            tmp.join("settings.json"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();

        let s = Settings::load();
        // 缺 guard → 默认值
        assert_eq!(s.guard, crate::guard::GuardSettings::default());
        assert!(s.guard.enabled);
        assert_eq!(s.guard.sit_min, 120);
        // 缺 skills.check_code → 默认 true（安全技能默认开放）
        assert!(s.skills.check_code);
        // 缺 ui.transparent_bg → 默认 true（v31.2：合成器系统默认透明背景）
        assert!(s.ui.transparent_bg);
        // 旧字段不受影响
        assert!(!s.skills.open_folder);
        assert_eq!(s.intimacy_level, 3);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 迭代②：voice 块 ----

    /// voice 自定义值落盘后原样读回；key 为空时 skip_serializing_if 不进 JSON。
    #[test]
    fn voice_round_trip_and_blank_key_omitted() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-voice");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };

        let mut s = Settings::default();
        s.voice.enabled = true;
        s.voice.engine = crate::tts::TtsEngineKind::Local;
        s.voice.volume = 90;
        s.voice.rate = 1.5;
        s.voice.scope = crate::tts::TtsScope::Reminders;
        s.voice.cloud_provider = "volcano".to_string();
        s.voice.cloud_voice = "zh_female_emo".to_string();
        s.voice.cloud_api_key = "sk-test-1".to_string();
        s.voice.stt_enabled = true;
        s.voice.stt_provider = "volcano".to_string();
        s.voice.stt_api_key = "stt-test-1".to_string();
        s.save().unwrap();

        let loaded = Settings::load();
        assert_eq!(loaded.voice, s.voice);
        // 落盘 JSON 含全部自定义字段（pretty 打印带空格，按解析后的值断言）
        let raw = std::fs::read_to_string(tmp.join("settings.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["voice"]["engine"], "local");
        assert_eq!(v["voice"]["scope"], "reminders");
        assert_eq!(v["voice"]["cloud_voice"], "zh_female_emo");
        assert_eq!(v["voice"]["cloud_api_key"], "sk-test-1");
        assert_eq!(v["voice"]["stt_api_key"], "stt-test-1");

        // 清空 key 再保存 → JSON 中不再出现两个 key 字段（skip_serializing_if）
        let mut s2 = loaded;
        s2.voice.cloud_api_key.clear();
        s2.voice.stt_api_key.clear();
        s2.save().unwrap();
        let raw2 = std::fs::read_to_string(tmp.join("settings.json")).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&raw2).unwrap();
        assert!(v2["voice"].get("cloud_api_key").is_none());
        assert!(v2["voice"].get("stt_api_key").is_none());

        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 旧版 settings.json（迭代①，无 voice 块）必须平滑加载默认 voice，不崩溃不回退。
    #[test]
    fn legacy_settings_without_voice_loads_default_voice() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-legacy-voice");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };

        // 复用 legacy 样例（无 voice、无 check_code、无 guard）。
        let legacy = serde_json::json!({
            "llm": {
                "provider": "doubao",
                "base_url": "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
                "model": "doubao-pro-32k"
            },
            "reminders": {
                "sit_min": 45, "water_min": 30, "eye_min": 20,
                "enabled": {"sit": true, "water": true, "eye": true}
            },
            "ui": {"age_confirmed": true, "pure_mode": false},
            "skills": {
                "total_enabled": true,
                "launch_app": true,
                "open_folder": false,
                "set_reminder": true
            },
            "intimacy_level": 3
        });
        std::fs::write(
            tmp.join("settings.json"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();

        let s = Settings::load();
        assert_eq!(s.voice, crate::tts::VoiceSettings::default());
        assert!(s.voice.enabled);
        assert_eq!(s.voice.engine, crate::tts::TtsEngineKind::Auto);
        assert_eq!(s.voice.volume, 80);
        assert_eq!(s.voice.scope, crate::tts::TtsScope::All);
        assert!(!s.voice.stt_enabled);
        // 旧字段不受影响
        assert_eq!(s.intimacy_level, 3);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    // ---- 迭代③：vault 块 ----

    /// vault 段默认值、自定义值落盘回读、旧 settings.json 无 vault 段平滑加载。
    #[test]
    fn vault_settings_defaults_and_legacy_compat() {
        let _g = crate::test_support::env_lock();
        let tmp = crate::test_support::unique_temp_dir("settings-vault");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };

        // 默认值（契约 §2）
        let s = Settings::default();
        assert_eq!(s.vault.idle_lock_min, 5);
        assert_eq!(s.vault.max_wrong_attempts, 5);
        assert_eq!(s.vault.cooldown_secs, 30);
        assert_eq!(s.vault.clipboard_clear_secs, 20);

        // 自定义值落盘后原样读回
        let mut s2 = s;
        s2.vault.idle_lock_min = 15;
        s2.vault.max_wrong_attempts = 8;
        s2.vault.cooldown_secs = 60;
        s2.vault.clipboard_clear_secs = 30;
        s2.save().unwrap();
        let loaded = Settings::load();
        assert_eq!(loaded.vault.idle_lock_min, 15);
        assert_eq!(loaded.vault.max_wrong_attempts, 8);
        assert_eq!(loaded.vault.cooldown_secs, 60);
        assert_eq!(loaded.vault.clipboard_clear_secs, 30);

        // 旧版 settings.json（无 vault 段）→ 默认 vault，不崩溃不回退整体默认
        let legacy = serde_json::json!({
            "llm": {
                "provider": "doubao",
                "base_url": "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
                "model": "doubao-pro-32k"
            },
            "reminders": {
                "sit_min": 45, "water_min": 30, "eye_min": 20,
                "enabled": {"sit": true, "water": true, "eye": true}
            },
            "ui": {"age_confirmed": true, "pure_mode": false},
            "skills": {
                "total_enabled": true,
                "launch_app": true,
                "open_folder": false,
                "set_reminder": true
            },
            "intimacy_level": 3
        });
        std::fs::write(
            tmp.join("settings.json"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();
        let s3 = Settings::load();
        assert_eq!(s3.vault, crate::settings::VaultSettings::default());
        assert_eq!(s3.intimacy_level, 3);
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }
}
