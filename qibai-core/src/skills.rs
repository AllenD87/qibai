//! 技能注册表（白名单）。
//!
//! T3 插件化改造：6 个内置技能成为**内置插件包**，统一走 `PluginRegistry` 注册通道。
//! 注册表按 function name 解析条目（不再按技能名字符串 match）；内置执行通过
//! [`BuiltinKind`](crate::plugin_registry::BuiltinKind) 枚举分发（枚举仅出现于内置包定义处）。
//!
//! 危险操作（`execute_command` 类能力）不立即执行：进入 pending 状态，
//! 向前端发起 `qibai://skill-approval` 事件，前端用户确认后才执行。
//!
//! 权限模型：总开关 + 每技能开关 + 权限强制（manifest 声明 → 安装确认 → 运行时判定）。
//! 「一键停用全部授权」= total_enabled=false（set_skill_enabled 或 save_settings 均可触发）。

use crate::plugin_registry::{BuiltinKind, PluginRegistry, PluginType};
use crate::settings::{PluginSettings, SkillsSettings};
use async_trait::async_trait;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

/// 白名单技能名（= 内置 function name；常量保留供测试与旧代码引用）。
pub const SKILL_LAUNCH_APP: &str = "launch_app";
pub const SKILL_OPEN_FOLDER: &str = "open_folder";
pub const SKILL_SET_REMINDER: &str = "set_reminder";
/// 代码检查（只读安全技能，无需审批）。
pub const SKILL_CHECK_CODE: &str = "check_code";
/// 打开联网社交（只读安全技能，无需审批；迭代④）。
pub const SKILL_OPEN_SOCIAL: &str = "open_social";
/// 危险技能：必须经前端确认。
pub const SKILL_EXECUTE_COMMAND: &str = "execute_command";
/// 联网搜索（只读安全技能，三引擎降级链）。
pub const SKILL_WEB_SEARCH: &str = "web_search";
/// v32 网页阅读器（抓取网页正文供总结）。
pub const SKILL_FETCH_PAGE: &str = "fetch_page";
/// 文件/文件夹操作（限 $HOME 沙盒；delete 走审批）。
pub const SKILL_FILE_OPS: &str = "file_ops";
/// 本地建工程（create_project 技能：骨架+项目上下文，创建前审批）。
pub const SKILL_CREATE_PROJECT: &str = "create_project";
/// 读取项目上下文（延续项目任务）。
pub const SKILL_PROJECT_CONTEXT: &str = "project_context";
/// v24 编程技能：git 工作区状态（只读）。
pub const SKILL_GIT_STATUS: &str = "git_status";
/// v24 编程技能：运行测试/检查命令（危险，执行需审批）。
pub const SKILL_RUN_TESTS: &str = "run_tests";
/// v24 编程技能：代码扫描搜索（只读）。
pub const SKILL_CODE_SCAN: &str = "code_scan";

pub const ALL_SKILLS: [&str; 14] = [
    SKILL_LAUNCH_APP,
    SKILL_OPEN_FOLDER,
    SKILL_SET_REMINDER,
    SKILL_CHECK_CODE,
    SKILL_OPEN_SOCIAL,
    SKILL_EXECUTE_COMMAND,
    SKILL_WEB_SEARCH,
    SKILL_FETCH_PAGE,
    SKILL_FILE_OPS,
    SKILL_CREATE_PROJECT,
    SKILL_PROJECT_CONTEXT,
    SKILL_GIT_STATUS,
    SKILL_RUN_TESTS,
    SKILL_CODE_SCAN,
];

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("skill not in whitelist: {0}")]
    NotWhitelisted(String),
    #[error("skills are globally disabled")]
    GloballyDisabled,
    #[error("skill disabled by user: {0}")]
    Disabled(String),
    #[error("execution failed: {0}")]
    Exec(String),
    #[error("这条确认已失效：找不到请求 {0}（可能已经执行过、或应用重启过；需要就重新发起同一操作）")]
    NoSuchRequest(String),
}

/// 执行结果（回填给 LLM / 前端的文本）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", content = "detail")]
pub enum SkillOutcome {
    /// 成功，附带给用户看的反馈文本。
    Ok(String),
    /// 需要前端确认：request_id + 描述。
    PendingApproval { request_id: String, description: String },
    /// 优雅拒绝（未授权 / 白名单 / 危险未确认）。
    Denied(String),
}

/// 平台执行器——抽象出来便于测试时 mock。
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    async fn launch_app(&self, target: &str) -> Result<String, String>;
    async fn open_folder(&self, path: &str) -> Result<String, String>;
    async fn set_reminder(&self, minutes: u64, message: &str) -> Result<String, String>;
    /// 危险命令执行（仅在用户批准后由 registry 调用）。
    async fn execute_command(&self, argv: &[String]) -> Result<String, String>;
    /// 代码检查（只读静态分析）；返回报告摘要文本。
    async fn check_code(&self, req: crate::codecheck::CheckRequest) -> Result<String, String>;
}

/// 系统执行器：用 std::process::Command 做跨平台封装。
pub struct OsExecutor;

#[async_trait]
impl SkillExecutor for OsExecutor {
    async fn launch_app(&self, target: &str) -> Result<String, String> {
        if target.trim().is_empty() {
            return Err("empty app target".into());
        }
        #[cfg(target_os = "linux")]
        let res = tokio::process::Command::new("xdg-open").arg(target).spawn();
        #[cfg(target_os = "windows")]
        let res = tokio::process::Command::new("cmd")
            .args(["/C", "start", "", target])
            .spawn();
        #[cfg(target_os = "macos")]
        let res = tokio::process::Command::new("open").arg(target).spawn();
        match res {
            Ok(_) => Ok(format!("已尝试打开：{target}")),
            Err(e) => Err(format!("launch_app 失败: {e}")),
        }
    }

    async fn open_folder(&self, path: &str) -> Result<String, String> {
        // v45 修：此前这里把原始字符串直接丢给 xdg-open——不展开 ~、不认「下载/桌面」，
        // 也不检查是否存在，于是用户说"打开下载文件夹"必然报错。现统一走 fileops 的
        // 解析 + 安全校验 + 真实失败检测，与 file_ops 的 open 语义一致。
        let home = crate::fileops::default_home()
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| "定位不到用户主目录".to_string())?;
        let arg = if path.trim().is_empty() { "~" } else { path };
        crate::fileops::run_file_op("open", arg, None, None, &home).await
    }

    async fn set_reminder(&self, minutes: u64, message: &str) -> Result<String, String> {
        // 真正的定时由 reminder 调度器承担；这里只做登记确认。
        if minutes == 0 {
            return Err("minutes must be > 0".into());
        }
        Ok(format!("已设置 {minutes} 分钟后的提醒：{message}"))
    }

    async fn execute_command(&self, argv: &[String]) -> Result<String, String> {
        if argv.is_empty() {
            return Err("empty command".into());
        }
        match tokio::process::Command::new(&argv[0]).args(&argv[1..]).output().await {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                Ok(format!("命令已执行，输出：{}", stdout.trim()))
            }
            Err(e) => Err(format!("execute_command 失败: {e}")),
        }
    }

    async fn check_code(&self, req: crate::codecheck::CheckRequest) -> Result<String, String> {
        // 纯静态检查：无 LLM 建议（None），失败也被 codecheck 包成结构化报告，
        // 摘要统一回填给对话（"未发现问题" 或 "发现 N 个问题…"）。
        let runner = crate::codecheck::OsCheckRunner;
        let report = crate::codecheck::run_check(req, &runner, None).await;
        Ok(report.summary)
    }
}

/// 待批准请求（危险命令 / 外部 command-kind 危险脚本）。
struct Pending {
    plugin_id: String,
    /// 内置 execute_command 的 argv；外部 command-kind 的原始 args。
    builtin_argv: Vec<String>,
    /// 待执行的函数名（外部插件/MCP 工具名；内置包为空）。
    fn_name: String,
    args: serde_json::Value,
    /// 批准后写入本任务放行名单的键（外部插件 = `plugin:<id>`；文件写改删 = `file:<动作>:<路径>`）。
    /// 内置 `execute_command` / `run_tests` 恒为 None——高危命令坚持逐次批准。
    grant_key: Option<String>,
}

/// 已处理过的确认（幂等回放用）。
///
/// 「允许本次」和「拒绝」都是一次性动作：前端连点两次、IPC 重试、
/// 或多窗口各点一次时，同一条 request_id 会被提交多次。
/// 这里记住每条确认的最终结果，重复提交一律回放，绝不二次执行，
/// 也不再报「找不到请求」这种让人误以为没跑起来的错。
#[derive(Debug, Clone)]
enum Resolved {
    /// 已执行成功（附结果文本）。
    RanOk(String),
    /// 已执行但失败（附错误文本）。
    RanErr(String),
    /// 用户拒绝。
    Cancelled,
}

pub struct SkillsRegistry {
    /// 旧字段镜像（status() 契约与旧 settings.json 兼容）。
    pub settings: SkillsSettings,
    /// 插件注册表（真源）。
    plugins: PluginRegistry,
    /// 外部插件执行体（command/http，10s 超时 + 降级）。
    adapter: crate::skill_adapter::SkillAdapter,
    executor: Box<dyn SkillExecutor>,
    pending: HashMap<String, Pending>,
    /// 已处理的确认（幂等回放；FIFO 上限 64 条，防无界增长）。
    resolved: HashMap<String, Resolved>,
    resolved_order: VecDeque<String>,
    /// 「允许本次」放行的外部插件（本次任务内后续调用不再重复询问）。
    granted: HashSet<String>,
    next_seq: u64,
}

impl SkillsRegistry {
    pub fn new(settings: SkillsSettings) -> Self {
        Self::with_executor(settings, Box::new(OsExecutor))
    }

    pub fn with_executor(settings: SkillsSettings, executor: Box<dyn SkillExecutor>) -> Self {
        Self::with_executor_and_plugins(settings, executor, PluginSettings::default())
    }

    /// T3 新增：带插件设置的构造器（Engine::new 使用）。
    pub fn with_executor_and_plugins(
        settings: SkillsSettings,
        executor: Box<dyn SkillExecutor>,
        plugin_settings: PluginSettings,
    ) -> Self {
        let plugins = PluginRegistry::new(&plugin_settings, &settings);
        Self {
            settings,
            plugins,
            adapter: crate::skill_adapter::SkillAdapter::new(),
            executor,
            pending: HashMap::new(),
            resolved: HashMap::new(),
            resolved_order: VecDeque::new(),
            granted: HashSet::new(),
            next_seq: 1,
        }
    }

    /// 新一轮用户请求开始：清掉「允许本次」的任务域放行名单。
    pub fn clear_approval_grants(&mut self) {
        self.granted.clear();
    }

    /// 当前放行名单大小（测试/诊断用）。
    pub fn granted_len(&self) -> usize {
        self.granted.len()
    }

    pub fn plugins(&self) -> &PluginRegistry {
        &self.plugins
    }

    pub fn plugins_mut(&mut self) -> &mut PluginRegistry {
        &mut self.plugins
    }

    /// save_settings 后把新的 PluginSettings 应用到注册表（enabled/grants 同步）。
    pub fn apply_plugin_settings(&mut self, ps: &PluginSettings) {
        self.plugins.apply_settings(ps, &self.settings);
    }

    /// 当前全部技能的开关快照（供 get_skills_status 契约）。形状不变。
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "total_enabled": self.settings.total_enabled,
            "skills": {
                SKILL_LAUNCH_APP: self.settings.launch_app,
                SKILL_OPEN_FOLDER: self.settings.open_folder,
                SKILL_SET_REMINDER: self.settings.set_reminder,
                SKILL_CHECK_CODE: self.settings.check_code,
                SKILL_OPEN_SOCIAL: self.settings.open_social,
                SKILL_WEB_SEARCH: self.settings.web_search,
                SKILL_FETCH_PAGE: self.settings.fetch_page,
                SKILL_FILE_OPS: self.settings.file_ops,
                SKILL_GIT_STATUS: self.settings.git_status,
                SKILL_RUN_TESTS: self.settings.run_tests,
                SKILL_CODE_SCAN: self.settings.code_scan,
            },
        })
    }

    /// 启停：name 是内置 function name 或插件 id 均接受；未知 → NotWhitelisted。
    pub fn set_enabled(&mut self, skill: &str, enabled: bool) -> Result<(), SkillError> {
        let pid = self
            .plugins
            .resolve_toggle_id(skill)
            .ok_or_else(|| SkillError::NotWhitelisted(skill.to_string()))?;
        self.plugins
            .set_enabled(&pid, enabled)
            .map_err(|e| SkillError::NotWhitelisted(e))?;
        // 镜像同步（枚举分发，非技能字符串 match）。
        if let Some(e) = self.plugins.entry(&pid) {
            if let Some(kind) = e.builtin {
                match kind {
                    BuiltinKind::LaunchApp => self.settings.launch_app = enabled,
                    BuiltinKind::OpenFolder => self.settings.open_folder = enabled,
                    BuiltinKind::SetReminder => self.settings.set_reminder = enabled,
                    BuiltinKind::CheckCode => self.settings.check_code = enabled,
                    BuiltinKind::OpenSocial => self.settings.open_social = enabled,
                    BuiltinKind::ExecuteCommand => {} // 无旧开关字段
                    BuiltinKind::QueryWeather => {}   // 无旧开关字段
                    BuiltinKind::ListSkills => {}     // 无旧开关字段
                    BuiltinKind::WebSearch => self.settings.web_search = enabled,
                    BuiltinKind::FetchPage => self.settings.fetch_page = enabled,
                    BuiltinKind::FileOps => self.settings.file_ops = enabled,
                    BuiltinKind::CreateProject => {}   // 无旧开关字段
                    BuiltinKind::ResumeProject => {}   // 无旧开关字段
                    BuiltinKind::GitStatus => self.settings.git_status = enabled,
                    BuiltinKind::RunTests => self.settings.run_tests = enabled,
                    BuiltinKind::CodeScan => self.settings.code_scan = enabled,
                }
            }
        }
        Ok(())
    }

    /// 一键停用 / 启用全部授权。
    pub fn set_total_enabled(&mut self, on: bool) {
        self.settings.total_enabled = on;
    }

    /// 动态工具表（§3.5）：enabled 内置 + 已启用外部插件。
    pub fn tool_schemas(&self) -> Vec<serde_json::Value> {
        self.plugins.tool_schemas()
    }

    /// 执行技能（由 agent loop 在收到 tool_calls 时调用）。
    pub async fn invoke(&mut self, name: &str, args: &serde_json::Value) -> SkillOutcome {
        // 1. 按 function name 解析条目（无 → 白名单拒绝）。
        let pid = match self.plugins.resolve_function(name) {
            Some(e) => e.manifest.id.clone(),
            None => {
                return SkillOutcome::Denied(
                    SkillError::NotWhitelisted(name.to_string()).to_string(),
                )
            }
        };
        // 2. 总开关。
        if !self.settings.total_enabled {
            return SkillOutcome::Denied(SkillError::GloballyDisabled.to_string());
        }
        // 3. 条目 enabled。
        let entry = self.plugins.entry(&pid).cloned().unwrap();
        if !entry.enabled {
            return SkillOutcome::Denied(SkillError::Disabled(name.to_string()).to_string());
        }
        // 4. 权限强制（grants 覆盖 manifest.permissions；内置默认全授）。
        if let Some(perm) = self.plugins.missing_permission(&pid) {
            return SkillOutcome::Denied(format!("技能需要权限 {perm}，请在设置页授权"));
        }
        // 5. 分发执行。
        match entry.builtin {
            Some(BuiltinKind::ExecuteCommand) => {
                // 危险技能：不执行，挂起等待前端确认（语义与现状一致）。
                let argv: Vec<String> = args
                    .get("argv")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                // 测试模式（OPEN_ALL）：危险命令直接执行，不再挂起审批。
                if crate::engine::open_all() {
                    let argv2 = argv;
                    if argv2.is_empty() {
                        return SkillOutcome::Denied("命令为空".to_string());
                    }
                    let out = tokio::task::spawn_blocking(move || {
                        std::process::Command::new(&argv2[0])
                            .args(&argv2[1..])
                            .output()
                    })
                    .await
                    .unwrap_or(Err(std::io::Error::new(std::io::ErrorKind::Other, "spawn失败")));
                    match out {
                        Ok(o) => {
                            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
                            let err = String::from_utf8_lossy(&o.stderr);
                            if !err.is_empty() {
                                text.push_str(&err);
                            }
                            SkillOutcome::Ok(if text.trim().is_empty() {
                                format!("命令执行完成（退出码 {}）", o.status.code().unwrap_or(-1))
                            } else {
                                text
                            })
                        }
                        Err(e) => SkillOutcome::Denied(format!("命令执行失败: {e}")),
                    }
                } else {
                    let request_id = format!("approval-{}", self.next_seq);
                    self.next_seq += 1;
                    let description = format!(
                        "即将执行命令：{}（来自对话技能请求）",
                        argv.join(" ")
                    );
                    self.pending.insert(
                        request_id.clone(),
                        Pending {
                            plugin_id: pid,
                            builtin_argv: argv, grant_key: None,
                            fn_name: String::new(),
                            args: args.clone(),
                        },
                    );
                    SkillOutcome::PendingApproval {
                        request_id,
                        description,
                    }
                }
            }
            Some(kind) => self.exec_builtin(kind, &pid, name, args).await,
            None => {
                // 外部插件危险判定（T8）：command/http 启动命令与 MCP 启动命令
                // 统一用 basename 危险表判定，命中 → pending 逐次批准。
                if let Some(es) = &entry.manifest.entry {
                    let launch_cmd: Option<String> = match es.kind {
                        crate::plugin_manifest::EntryKind::Command => es.command.clone(),
                        crate::plugin_manifest::EntryKind::Mcp => es
                            .mcp
                            .as_ref()
                            .map(|m| m.command.clone()),
                        _ => None,
                    };
                    if let Some(cmd) = launch_cmd {
                        // 预检：可执行根本不在（补全后的）PATH 里时，弹审批只会让用户
                        // 点完「允许」再收到失败。这里直接如实说缺什么、怎么补。
                        if crate::proc_env::resolve_launcher(&cmd, entry.dir.as_deref()).is_none() {
                            return SkillOutcome::Denied(crate::proc_env::missing_binary_hint(&cmd));
                        }
                        if !crate::engine::open_all()
                            && crate::skill_adapter::is_dangerous_command(&cmd, args)
                            && !self.granted.contains(&format!("plugin:{pid}"))
                        {
                            let request_id = format!("approval-{}", self.next_seq);
                            self.next_seq += 1;
                            let description = format!(
                                "即将执行外部技能 {name}：{cmd}（来自对话技能请求；允许后本任务内该技能不再重复询问）"
                            );
                            self.pending.insert(
                                request_id.clone(),
                                Pending {
                                    plugin_id: pid.clone(),
                                    builtin_argv: vec![],
                                    fn_name: name.to_string(),
                                    args: args.clone(),
                                    grant_key: Some(format!("plugin:{pid}")),
                                },
                            );
                            return SkillOutcome::PendingApproval {
                                request_id,
                                description,
                            };
                        }
                    }
                }
                self.exec_external(&entry, name, args).await
            }
        }
    }

    /// 内置包执行分发（BuiltinKind 枚举——内置包自己的定义处）。
    async fn exec_builtin(
        &mut self,
        kind: BuiltinKind,
        pid: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> SkillOutcome {
        match kind {
            BuiltinKind::LaunchApp => {
                let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("");
                match self.executor.launch_app(target).await {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::OpenFolder => {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                match self.executor.open_folder(path).await {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::SetReminder => {
                let minutes = args.get("minutes").and_then(|v| v.as_u64()).unwrap_or(0);
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("提醒");
                match self.executor.set_reminder(minutes, message).await {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::CheckCode => {
                // path 与 snippet 恰好提供一个；语言可选。
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let snippet = args
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let language = args
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let req = match (path, snippet) {
                    (Some(p), None) => crate::codecheck::CheckRequest::from_path(p),
                    (None, Some(s)) => {
                        crate::codecheck::CheckRequest::from_snippet(s, language)
                    }
                    (None, None) => {
                        return SkillOutcome::Denied(
                            "check_code 需要提供 path（文件/文件夹）或 snippet（代码片段）之一".to_string(),
                        )
                    }
                    (Some(_), Some(_)) => {
                        return SkillOutcome::Denied(
                            "check_code 的 path 与 snippet 只能提供一个，不能同时提供".to_string(),
                        )
                    }
                };
                match self.executor.check_code(req).await {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::QueryWeather => {
                let city = args
                    .get("city")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("石家庄");
                match crate::weather::fetch_weather(city).await {
                    Ok(text) => SkillOutcome::Ok(text),
                    Err(e) => SkillOutcome::Denied(format!("查天气失败：{e}")),
                }
            }
            BuiltinKind::ListSkills => {
                let mut lines = Vec::new();
                for e in self.plugins.entries.values() {
                    if !e.enabled || e.error.is_some() {
                        continue;
                    }
                    if e.manifest.plugin_type != PluginType::Skill || !e.is_exposed() {
                        continue;
                    }
                    for f in &e.manifest.functions {
                        lines.push(format!("· {}：{}", f.name, f.description));
                    }
                }
                SkillOutcome::Ok(if lines.is_empty() {
                    "当前没有可用技能".to_string()
                } else {
                    lines.join("\n")
                })
            }
            BuiltinKind::OpenSocial => {
                // 只读安全技能（迭代④）：参数 { view?: "planet"|"city" }；
                // 前端收到回复的 skill=open_social 后打开「社交」tab 并切换视图。
                let view = args.get("view").and_then(|v| v.as_str()).unwrap_or("planet");
                let view_text = match view {
                    "city" => "同城灯塔",
                    _ => "星球广场",
                };
                SkillOutcome::Ok(format!("已为你打开{view_text}…"))
            }
            BuiltinKind::WebSearch => {
                // 只读联网搜索：缺 query → 优雅拒绝；结果/错误均回填中文文本。
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                if query.trim().is_empty() {
                    return SkillOutcome::Denied(
                        "web_search 需要提供 query（搜索关键词）。例如：{\"query\":\"今天天气\"}"
                            .to_string(),
                    );
                }
                match crate::web_search::search_web(query).await {
                    Ok(text) => SkillOutcome::Ok(text),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::FetchPage => {
                // 只读网页阅读器：缺 url → 优雅拒绝；抓正文回填 LLM 总结。
                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if url.trim().is_empty() {
                    return SkillOutcome::Denied(
                        "fetch_page 需要提供 url（网页地址）。例如：{\"url\":\"https://example.com\"}".to_string(),
                    );
                }
                match crate::web_search::fetch_page(url).await {
                    Ok(text) => SkillOutcome::Ok(text),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::FileOps => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action.is_empty() {
                    return SkillOutcome::Denied(
                        "file_ops 需要提供 action（list/open/create/rename/move/copy/delete）".to_string(),
                    );
                }
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let home = match crate::fileops::default_home() {
                    Some(h) => h,
                    None => return SkillOutcome::Denied("无法定位用户主目录".to_string()),
                };
                let home_s = home.to_string_lossy().to_string();
                // 高危/写操作：delete/write/edit 一律先消毒再挂起审批（前置检查不打扰用户）。
                if action == "delete" || action == "write" || action == "edit" {
                    let sanitized = match crate::fileops::sanitize_path(path, &home_s) {
                        Ok(p) => p,
                        Err(e) => return SkillOutcome::Denied(e),
                    };
                    // 前置存在性检查：write 要求新文件；edit 要求文件已存在
                    if action == "write" && sanitized.exists() {
                        return SkillOutcome::Denied(format!(
                            "{} 已存在，未覆盖；要修改内容请用 edit",
                            sanitized.display()
                        ));
                    }
                    if action == "edit" && !sanitized.exists() {
                        return SkillOutcome::Denied(format!(
                            "{} 不存在，无法修改；要新建文件请用 write",
                            sanitized.display()
                        ));
                    }
                    // 同一任务内「同一路径 + 同一动作」批准过一次就不再问：
                    // 出片流水线每轮都要重写同一份 scene.json，反复弹窗等于卡死。
                    let gk = format!("file:{action}:{}", sanitized.display());
                    if !self.granted.contains(&gk) {
                        let n_bytes = args
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|c| c.len())
                            .unwrap_or(0);
                        let description = match action {
                            "delete" => format!("即将删除：{}（来自对话技能请求）", path),
                            "write" => format!("即将写入：{}（{} 字节）", path, n_bytes),
                            _ => format!("即将修改：{}（{} 字节）", path, n_bytes),
                        };
                        let request_id = format!("approval-{}", self.next_seq);
                        self.next_seq += 1;
                        self.pending.insert(
                            request_id.clone(),
                            Pending {
                                plugin_id: pid.to_string(),
                                builtin_argv: vec![],
                                grant_key: Some(gk),
                                fn_name: name.to_string(),
                                args: args.clone(),
                            },
                        );
                        return SkillOutcome::PendingApproval {
                            request_id,
                            description,
                        };
                    }
                }
                let dest = args.get("dest").and_then(|v| v.as_str());
                let content = args.get("content").and_then(|v| v.as_str());
                match crate::fileops::run_file_op(action, path, dest, content, &home_s).await {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::CreateProject => {
                let home = match crate::fileops::default_home() {
                    Some(h) => h,
                    None => return SkillOutcome::Denied("无法定位用户主目录".to_string()),
                };
                let home_s = home.to_string_lossy().to_string();
                match name {
                    "create_project" => {
                        // 方案确认：先校验 + 生成方案描述，挂起等用户确认后才真正建项目。
                        let pname = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let parent = args.get("parent").and_then(|v| v.as_str()).unwrap_or("");
                        let stack = args.get("stack").and_then(|v| v.as_str()).unwrap_or("python");
                        let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("");
                        if let Err(e) = crate::project_scaffold::validate_name(pname) {
                            return SkillOutcome::Denied(e);
                        }
                        if let Err(e) = crate::fileops::sanitize_path(parent, &home_s) {
                            return SkillOutcome::Denied(e);
                        }
                        let lang = normalize_lang(stack);
                        let git_init = args
                            .get("git_init")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let request_id = format!("approval-{}", self.next_seq);
                        self.next_seq += 1;
                        let description =
                            crate::project_scaffold::build_plan(pname, &lang, goal, git_init);
                        self.pending.insert(
                            request_id.clone(),
                            Pending {
                                plugin_id: pid.to_string(),
                                builtin_argv: vec![], grant_key: None,
                                fn_name: name.to_string(),
                                args: args.clone(),
                            },
                        );
                        return SkillOutcome::PendingApproval {
                            request_id,
                            description,
                        };
                    }
                    "project_context" => {
                        let pname = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        match find_project_by_name(pname).and_then(|dir| {
                            crate::project_context::load_context(&dir).map(|ctx| (dir, ctx))
                        }) {
                            Some((dir, ctx)) => {
                                let name = dir
                                    .file_name()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let files = if ctx.files.is_empty() {
                                    "（暂无）".to_string()
                                } else {
                                    ctx.files.join("、")
                                };
                                SkillOutcome::Ok(format!(
                                    "工程「{name}」目标：{}；进度：{}；文件：{}",
                                    ctx.goal, ctx.progress, files
                                ))
                            }
                            None => SkillOutcome::Denied(
                                "没有找到工程记录或 PROJECT.md，说「创建工程」开始第一个吧".to_string(),
                            ),
                        }
                    }
                    "project_plan" => {
                        let pname = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("");
                        let plan = args.get("plan").and_then(|v| v.as_str()).unwrap_or("");
                        let delta = crate::project_context::ContextDelta {
                            progress: Some(if plan.is_empty() {
                                goal.to_string()
                            } else {
                                format!("方案已制定：{plan}")
                            }),
                            files: vec![],
                            decisions: Some(goal.to_string()),
                        };
                        match find_project_by_name(pname)
                            .and_then(|dir| crate::project_context::update_context(&dir, &delta).ok())
                        {
                            Some(_) => SkillOutcome::Ok("已把方案写入工程上下文，等待你确认后开工".to_string()),
                            None => SkillOutcome::Denied("未找到工程记录，无法写入方案".to_string()),
                        }
                    }
                    "project_confirm" => {
                        let pname = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let delta = crate::project_context::ContextDelta {
                            progress: Some("用户已确认方案，开工".to_string()),
                            files: vec![],
                            decisions: Some("已确认执行".to_string()),
                        };
                        match find_project_by_name(pname)
                            .and_then(|dir| crate::project_context::update_context(&dir, &delta).ok())
                        {
                            Some(_) => SkillOutcome::Ok("已确认，开始干活".to_string()),
                            None => SkillOutcome::Denied("未找到工程记录，无法确认".to_string()),
                        }
                    }
                    other => SkillOutcome::Denied(format!(
                        "未知项目操作 {other}，支持 create_project/project_context/project_plan/project_confirm"
                    )),
                }
            }
            BuiltinKind::ResumeProject => {
                // 续作（只读无审批）：name 留空 → 最近一次任务；否则定位 parent/name。
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let parent_dir = args.get("parent_dir").and_then(|v| v.as_str()).unwrap_or("");
                let home = match crate::fileops::default_home() {
                    Some(h) => h,
                    None => return SkillOutcome::Denied("无法定位用户主目录".to_string()),
                };
                let home_s = home.to_string_lossy().to_string();
                match crate::project_scaffold::resume_summary(name, parent_dir, &home_s) {
                    Ok(msg) => SkillOutcome::Ok(msg),
                    Err(e) => SkillOutcome::Denied(e),
                }
            }
            BuiltinKind::GitStatus => {
                // v24 编程技能：git 工作区状态（只读，不修改）。
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let dir = if path.trim().is_empty() {
                    std::env::current_dir()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    path.to_string()
                };
                if dir.is_empty() {
                    return SkillOutcome::Denied("无法确定项目目录".to_string());
                }
                match tokio::task::spawn_blocking(move || git_status_text(&dir)).await {
                    Ok(Ok(text)) => SkillOutcome::Ok(text),
                    Ok(Err(e)) => SkillOutcome::Denied(e),
                    Err(e) => SkillOutcome::Denied(format!("git 状态查询失败: {e}")),
                }
            }
            BuiltinKind::RunTests => {
                // v24 编程技能：运行测试/检查命令（危险：执行本地命令，非 OPEN_ALL 挂起审批）。
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                const ALLOWED: [(&str, &[&str]); 4] = [
                    ("cargo test", &["cargo", "test"]),
                    ("cargo check", &["cargo", "check"]),
                    ("node --check", &["node", "--check"]),
                    ("npm test", &["npm", "test"]),
                ];
                let argv: Vec<String> = match ALLOWED.iter().find(|(k, _)| *k == cmd) {
                    Some((_, argv)) => argv.iter().map(|s| s.to_string()).collect(),
                    None => {
                        return SkillOutcome::Denied(
                            "不支持的测试命令，仅支持 cargo test / cargo check / node --check / npm test"
                                .to_string(),
                        )
                    }
                };
                if !crate::engine::open_all() {
                    let request_id = format!("approval-{}", self.next_seq);
                    self.next_seq += 1;
                    let description =
                        format!("运行测试命令：{cmd}（会执行本地命令，需要你确认）");
                    self.pending.insert(
                        request_id.clone(),
                        Pending {
                            plugin_id: pid.to_string(),
                            builtin_argv: argv, grant_key: None,
                            fn_name: name.to_string(),
                            args: args.clone(),
                        },
                    );
                    return SkillOutcome::PendingApproval {
                        request_id,
                        description,
                    };
                }
                match tokio::task::spawn_blocking(move || {
                    std::process::Command::new(&argv[0])
                        .args(&argv[1..])
                        .output()
                })
                .await
                {
                    Ok(Ok(o)) => {
                        let mut text = String::from_utf8_lossy(&o.stdout).to_string();
                        let err = String::from_utf8_lossy(&o.stderr).to_string();
                        if !err.trim().is_empty() {
                            text.push('\n');
                            text.push_str(&err);
                        }
                        let status = if o.status.success() { "通过" } else { "失败" };
                        let code = o.status.code().map(|c| c.to_string()).unwrap_or_default();
                        let body = text.chars().take(3000).collect::<String>();
                        SkillOutcome::Ok(format!("{cmd} 执行{status}（退出码 {code}）\n{body}"))
                    }
                    Ok(Err(e)) => SkillOutcome::Denied(format!("命令启动失败: {e}")),
                    Err(e) => SkillOutcome::Denied(format!("任务执行失败: {e}")),
                }
            }
            BuiltinKind::CodeScan => {
                // v24 编程技能：代码扫描（只读，纯 Rust 遍历，无外部依赖）。
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if query.is_empty() {
                    return SkillOutcome::Denied(
                        "code_scan 需要提供 query（搜索关键字）。例如：{\"query\":\"func_name\"}"
                            .to_string(),
                    );
                }
                if query.chars().count() > 60 {
                    return SkillOutcome::Denied("搜索关键字太长了，精简到 60 字以内".to_string());
                }
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let dir = if path.trim().is_empty() {
                    std::env::current_dir()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    path.to_string()
                };
                if dir.is_empty() {
                    return SkillOutcome::Denied("无法确定项目目录".to_string());
                }
                match tokio::task::spawn_blocking(move || scan_code_text(&dir, &query)).await {
                    Ok(text) => SkillOutcome::Ok(text),
                    Err(e) => SkillOutcome::Denied(format!("代码扫描失败: {e}")),
                }
            }
            BuiltinKind::ExecuteCommand => unreachable!("由 invoke 直接挂起，不走分发"),
        }
    }

    /// 外部插件执行：adapter 直接执行（危险判定已在 invoke 前置完成）。
    async fn exec_external(
        &self,
        entry: &crate::plugin_registry::PluginEntry,
        name: &str,
        args: &serde_json::Value,
    ) -> SkillOutcome {
        let dir = entry
            .dir
            .clone()
            .unwrap_or_else(|| self.plugins.plugins_dir().to_path_buf());
        self.adapter.execute(&dir, &entry.manifest, name, args).await
    }

    /// 仅用于门控：判断该 pending 请求是否属于高危 builtin（ExecuteCommand / RunTests）。
    /// 高危操作保留 L4 亲密度门；文件读写/建工程等基础操作不走 L4。
    /// 当前待审批请求条数（缺件预检/空转排查用）。
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_pending_high_risk(&self, request_id: &str) -> bool {
        let Some(pending) = self.pending.get(request_id) else {
            return false;
        };
        let Some(entry) = self.plugins.entry(&pending.plugin_id) else {
            return false;
        };
        matches!(
            entry.builtin,
            Some(BuiltinKind::ExecuteCommand) | Some(BuiltinKind::RunTests)
        )
    }

    /// 前端确认回调：approve=true 执行，false 取消。
    ///
    /// 幂等：同一条确认被重复提交（连点「允许本次」、IPC 重试、两个窗口各点一次）时，
    /// 已跑过的回上次结果、取消过的不许再批准，都不二次执行。
    pub async fn approve(&mut self, request_id: &str, approve: bool) -> Result<String, SkillError> {
        let Some(pending) = self.pending.remove(request_id) else {
            let replay = self.replay_resolved(request_id, approve);
            // 诊断锚点：复现「点了允许却说没跑起来」时，一眼看出是重放还是真失效。
            eprintln!(
                "[qibai] approve: id={request_id} approve={approve} pending={} replay={}",
                self.pending.len(),
                match &replay {
                    Ok(_) => "ok",
                    Err(_) => "err",
                }
            );
            return replay;
        };
        eprintln!(
            "[qibai] approve: id={request_id} approve={approve} pending={} first-shot",
            self.pending.len()
        );
        if !approve {
            self.remember_resolved(request_id, Resolved::Cancelled);
            return Ok("用户已取消该操作".to_string());
        }
        let grant_key = pending.grant_key.clone();
        let result = self.run_pending(pending).await;
        match &result {
            Ok(text) => {
                self.remember_resolved(request_id, Resolved::RanOk(text.clone()));
                // 批准一次 = 本任务内该项不再重复询问：一条片子要连跑
                // schema→build→render→qa→mux，且每轮重排都会重写同一份 spec，
                // 逐步弹窗会被当成「卡住」。内置高危命令仍逐次批准（grant_key=None）。
                if let Some(k) = grant_key {
                    self.granted.insert(k);
                }
            }
            Err(e) => self.remember_resolved(request_id, Resolved::RanErr(e.to_string())),
        }
        result
    }

    /// 同一条确认的重复提交：回放上次结果，不二次执行。
    fn replay_resolved(&self, request_id: &str, approve: bool) -> Result<String, SkillError> {
        match self.resolved.get(request_id) {
            // 已执行成功：把上次的真实结果原样回给用户（不再重跑）。
            Some(Resolved::RanOk(text)) => {
                Ok(format!("{text}\n（这条确认刚才已经执行过，未重复执行）"))
            }
            // 已执行失败：同样的失败原因再报一次，不重试。
            Some(Resolved::RanErr(text)) => Err(SkillError::Exec(text.clone())),
            // 已取消：拒绝就直接回同义结果；再想批准必须先重新发起。
            Some(Resolved::Cancelled) => {
                if approve {
                    Err(SkillError::Exec(
                        "这条确认已经被取消，不会再执行；需要就重新发起一次".to_string(),
                    ))
                } else {
                    Ok("用户已取消该操作".to_string())
                }
            }
            None => Err(SkillError::NoSuchRequest(request_id.to_string())),
        }
    }

    /// 记下一条确认的最终结果（FIFO 上限 64，超出丢弃最旧的）。
    fn remember_resolved(&mut self, request_id: &str, outcome: Resolved) {
        const CAP: usize = 64;
        if self
            .resolved
            .insert(request_id.to_string(), outcome)
            .is_none()
        {
            self.resolved_order.push_back(request_id.to_string());
        }
        while self.resolved_order.len() > CAP {
            if let Some(old) = self.resolved_order.pop_front() {
                self.resolved.remove(&old);
            }
        }
    }

    /// 真正执行一条已批准的请求（approve 的执行体）。
    async fn run_pending(&mut self, pending: Pending) -> Result<String, SkillError> {
        let entry = self
            .plugins
            .entry(&pending.plugin_id)
            .cloned()
            .ok_or_else(|| SkillError::Exec(format!("pending 插件 {} 已不存在", pending.plugin_id)))?;
        match entry.builtin {
            Some(BuiltinKind::ExecuteCommand) => self
                .executor
                .execute_command(&pending.builtin_argv)
                .await
                .map_err(|e| SkillError::Exec(e)),
            Some(BuiltinKind::RunTests) => {
                // v24 编程技能：命令已在 invoke 白名单枚举（cargo test 等），批准后受限执行。
                self.executor
                    .execute_command(&pending.builtin_argv)
                    .await
                    .map_err(|e| SkillError::Exec(e))
            }
            Some(BuiltinKind::FileOps) => {
                // 审批通过：按 action 分发 delete/write/edit（二次消毒在各 perform_* 内）。
                let action = pending
                    .args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("delete");
                let path = pending
                    .args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let home = crate::fileops::default_home()
                    .ok_or_else(|| SkillError::Exec("无法定位用户主目录".to_string()))?;
                let home_s = home.to_string_lossy().to_string();
                match action {
                    "write" => {
                        let content = pending
                            .args
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        crate::fileops::perform_write(&path, &content, &home_s)
                            .await
                            .map_err(|e| SkillError::Exec(e))
                    }
                    "edit" => {
                        let content = pending
                            .args
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mode = pending
                            .args
                            .get("mode")
                            .and_then(|v| v.as_str())
                            .unwrap_or("append")
                            .to_string();
                        crate::fileops::perform_edit(&path, &content, &mode, &home_s)
                            .await
                            .map_err(|e| SkillError::Exec(e))
                    }
                    _ => {
                        crate::fileops::perform_delete(&path, &home_s)
                            .await
                            .map_err(|e| SkillError::Exec(e))
                    }
                }
            }
            Some(BuiltinKind::CreateProject) => {
                // 建工程审批通过：真正创建项目（骨架+上下文+索引）。
                let name = pending
                    .args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let parent = pending
                    .args
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let stack = pending
                    .args
                    .get("stack")
                    .and_then(|v| v.as_str())
                    .unwrap_or("python")
                    .to_string();
                let goal = pending
                    .args
                    .get("goal")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let home = crate::fileops::default_home()
                    .ok_or_else(|| SkillError::Exec("无法定位用户主目录".to_string()))?;
                let home_s = home.to_string_lossy().to_string();
                let git_init = pending
                    .args
                    .get("git_init")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let lang = normalize_lang(&stack);
                crate::project_scaffold::perform_create_project(
                    &name, &parent, &lang, &goal, git_init, &home_s,
                )
                .map_err(|e| SkillError::Exec(e))
            }
            Some(_) => Err(SkillError::Exec(format!(
                "pending 插件 {} 无执行体",
                pending.plugin_id
            ))),
            None => {
                // 外部插件（command/http/mcp 危险项）：批准后由 adapter 执行。
                let dir = entry.dir.clone().unwrap_or_default();
                match self
                    .adapter
                    .execute(&dir, &entry.manifest, &pending.fn_name, &pending.args)
                    .await
                {
                    SkillOutcome::Ok(msg) => Ok(msg),
                    SkillOutcome::Denied(e) => Err(SkillError::Exec(e)),
                    SkillOutcome::PendingApproval { .. } => {
                        Err(SkillError::Exec("unexpected pending state".to_string()))
                    }
                }
            }
        }
    }
}

/// 供 LLM function calling 使用的工具 schema（OpenAI 兼容格式）。
///
/// 形状与现状一字不差：默认注册表（仅内置、全启用）的工具表。
pub fn tool_schemas() -> Vec<serde_json::Value> {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": SKILL_LAUNCH_APP,
                "description": "在用户的电脑上打开一个应用或网址",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "target": {"type": "string", "description": "应用名或 URL"}
                    },
                    "required": ["target"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_OPEN_FOLDER,
                "description": "在文件管理器中打开一个文件夹路径",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "文件夹路径，空字符串表示主目录"}
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_SET_REMINDER,
                "description": "设置一个本地提醒，到时通知用户",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "minutes": {"type": "integer", "description": "多少分钟后提醒"},
                        "message": {"type": "string", "description": "提醒内容"}
                    },
                    "required": ["minutes", "message"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_CHECK_CODE,
                "description": "检查用户指定的本地代码文件/文件夹或代码片段，返回结构化检查报告摘要",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "本地代码文件或文件夹路径（与 snippet 二选一）"},
                        "snippet": {"type": "string", "description": "待检查的代码片段文本（与 path 二选一）"},
                        "language": {"type": "string", "description": "可选，代码语言名，如 javascript / python；path 模式忽略"}
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_OPEN_SOCIAL,
                "description": "打开联网社交页面（星球广场 / 同城灯塔）。用户表达「带我去星球逛逛」「星球广场」「去星球看看」「同城看看」「打开社交」等意图时调用",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "view": {"type": "string", "enum": ["planet", "city"], "description": "可选；planet=星球广场（默认），city=同城灯塔"}
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_WEB_SEARCH,
                "description": "搜索互联网获取实时/最新信息（新闻/天气/百科/知识等）。用户问实时、最新、不知道需要查证的事情时调用",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "搜索关键词，不超过 200 字"}
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_FETCH_PAGE,
                "description": "读取一个网页的正文内容（标题+正文纯文本）。搜索拿到结果后，需要深入了解某条内容时调用本工具抓正文再总结",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "网页完整地址（http/https），不超过 2048 字符"}
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": SKILL_FILE_OPS,
                "description": "浏览/打开/创建/重命名/移动/复制/写入/修改/删除本地文件或文件夹（限主目录内；write/edit/delete 需用户审批）",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["list", "open", "create", "rename", "move", "copy", "write", "edit", "delete"], "description": "操作类型；write=新建文件，edit=修改已有文件"},
                        "path": {"type": "string", "description": "目标文件或文件夹绝对路径，或 ~/ 开头（限主目录内）"},
                        "dest": {"type": "string", "description": "rename/move/copy 的目标路径"},
                        "content": {"type": "string", "description": "create/write/edit 时写入的文本内容"},
                        "mode": {"type": "string", "enum": ["append", "replace"], "description": "edit 的模式：append=追加，replace=全量替换（默认 append）"}
                    },
                    "required": ["action", "path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "resume_project",
                "description": "继续/恢复某个工程：返回工程目标、上次进度、文件清单与下一步建议。name 留空则返回最近一次任务。用户说「继续上次任务」「接着上次的工程做」「恢复项目」时调用",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "工程名（留空 = 最近一次任务）"},
                        "parent_dir": {"type": "string", "description": "工程父目录绝对路径或 ~/ 开头；name 为空时可省略"}
                    }
                }
            }
        }
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
}

/// v18.6 技能产物落盘（语音「帮我写…保存到 <路径>」）：安全校验 + 协议解析。
/// 校验只读、不写盘；写盘动作由调用方（src-tauri）在校验通过后执行。
/// 安全约束：路径必须在用户目录内、拒绝 .. 穿越、扩展名白名单、绝不覆盖已存在文件。
pub const SAVE_EXT_WHITELIST: [&str; 9] = ["py", "js", "html", "css", "sh", "json", "rs", "go", "ts"];

/// 解析技能输出中的 `@qibai-save:begin / path=… / content=<base64> / @qibai-save:end` 块。
pub fn parse_skill_save_block(out: &str) -> Option<(String, String)> {
    let begin = out.find("@qibai-save:begin")?;
    let after = &out[begin + "@qibai-save:begin".len()..];
    let end = after.find("@qibai-save:end")?;
    let block = &after[..end];
    let mut path: Option<String> = None;
    let mut content: Option<String> = None;
    for line in block.lines() {
        let line = line.trim();
        if let Some(p) = line.strip_prefix("path=") {
            path = Some(p.to_string());
        } else if let Some(c) = line.strip_prefix("content=") {
            content = Some(c.to_string());
        }
    }
    match (path, content) {
        (Some(p), Some(c)) => Some((p, c)),
        _ => None,
    }
}

/// 校验保存目标：~ 展开 → 绝对化 → 必须在 $HOME 内（防 .. 穿越）→ 扩展名白名单 → 不覆盖已有文件。
/// 返回规范化绝对路径；失败返回拒绝原因（中文，可直接朗读/展示）。
pub fn validate_save_target(raw: &str, home: &str) -> Result<String, String> {
    let t = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if t.is_empty() {
        return Err("保存路径为空，请说清楚保存到哪个文件".to_string());
    }
    let path = if t == "~" || t.starts_with("~/") {
        format!("{}/{}", home.trim_end_matches('/'), t.trim_start_matches("~/"))
    } else if t.starts_with('/') {
        t
    } else {
        return Err("保存路径必须是绝对路径（如 /home/你/xxx.py）或 ~/ 开头".to_string());
    };

    // 防穿越：逐段重建，拒绝 ParentDir
    let mut cur = std::path::PathBuf::new();
    for comp in std::path::Path::new(&path).components() {
        match comp {
            std::path::Component::ParentDir => {
                return Err("保存路径包含 .. 穿越，已拒绝".to_string());
            }
            std::path::Component::CurDir => {}
            _ => cur.push(comp.as_os_str()),
        }
    }
    let abs = cur;

    // 必须在用户目录内（父目录已存在 → canonicalize 精确校验；不存在 → 前缀 + 边界校验）
    let home_norm = std::path::Path::new(home)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(home));
    let home_str = home_norm.to_string_lossy().to_string();
    let abs_str = abs.to_string_lossy().to_string();
    if let Some(parent) = abs.parent() {
        if let Ok(pc) = parent.canonicalize() {
            if !pc.starts_with(&home_norm) {
                return Err("保存路径必须在用户目录内".to_string());
            }
        }
    }
    if !(abs_str == home_str || abs_str.starts_with(&format!("{home_str}/"))) {
        return Err("保存路径必须在用户目录内".to_string());
    }

    // 扩展名白名单
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !SAVE_EXT_WHITELIST.contains(&ext.as_str()) {
        return Err(format!(
            "仅支持保存 {} 等文本代码文件（当前扩展名 .{ext}）",
            SAVE_EXT_WHITELIST.join("/")
        ));
    }

    // 绝不覆盖已存在文件
    if abs.exists() {
        return Err(format!(
            "{} 已存在，我没有覆盖。可以说「覆盖保存」或换个文件名",
            abs.display()
        ));
    }
    Ok(abs.to_string_lossy().to_string())
}

/// create_project 技能的语言归一（manifest 允许 python/node/rust/web）。
fn normalize_lang(stack: &str) -> String {
    match stack.trim().to_ascii_lowercase().as_str() {
        "python" | "py" => "python".to_string(),
        "node" | "nodejs" | "js" | "javascript" => "node".to_string(),
        "rust" | "rs" => "rust".to_string(),
        "web" | "html" | "h5" => "web".to_string(),
        _ => "python".to_string(),
    }
}

/// 按工程名（或最近一次）查注册表工程目录。
fn find_project_by_name(name: &str) -> Option<std::path::PathBuf> {
    let records = crate::project_scaffold::load_registry();
    let rec = records
        .iter()
        .find(|r| r.name == name)
        .or_else(|| records.iter().max_by_key(|r| r.updated_at))?;
    Some(std::path::PathBuf::from(&rec.path))
}

/// v24 编程技能：git 工作区状态 + 最近 3 条提交（只读；git 不可用/非仓库给中文提示）。
fn git_status_text(dir: &str) -> Result<String, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|e| format!("git 不可用（{e}），请确认已安装 git"))
            .and_then(|o| {
                if o.status.success() {
                    Ok(String::from_utf8_lossy(&o.stdout).to_string())
                } else {
                    Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
                }
            })
    };
    // 先确认是 git 仓库（只读探测）。
    let is_repo = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !is_repo {
        return Err(format!("{dir} 不是一个 git 仓库（目录下没有 .git）"));
    }
    let status = run(&["status", "--short", "--branch"])
        .unwrap_or_else(|e| e);
    let log = run(&["log", "-3", "--oneline", "--decorate"]).unwrap_or_else(|e| e);
    let mut lines = Vec::new();
    if status.trim().is_empty() {
        lines.push("【工作区】干净，没有未提交的改动".to_string());
    } else {
        lines.push(format!("【工作区】\n{}", status.trim()));
    }
    if log.trim().is_empty() || log.contains("git 不可用") {
        lines.push("【最近提交】暂无提交记录".to_string());
    } else {
        lines.push(format!("【最近提交】\n{}", log.trim()));
    }
    Ok(lines.join("\n"))
}

/// v24 编程技能：递归扫描文本文件找关键字（只读；跳过 target/node_modules 等，单文件 >1MB 跳过）。
fn scan_code_text(dir: &str, query: &str) -> String {
    let mut hits: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(dir)];
    let mut visited: usize = 0;
    while let Some(p) = stack.pop() {
        if hits.len() >= 30 || visited > 4000 {
            break;
        }
        visited += 1;
        let rd = match std::fs::read_dir(&p) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            if hits.len() >= 30 {
                break;
            }
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                "target" | "node_modules" | ".git" | "dist" | "build" | ".venv" | "venv"
            ) {
                continue;
            }
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !matches!(
                    ext.as_str(),
                    "rs"
                        | "js"
                        | "ts"
                        | "py"
                        | "toml"
                        | "json"
                        | "html"
                        | "css"
                        | "md"
                        | "txt"
                        | "sh"
                        | "c"
                        | "cpp"
                        | "h"
                        | "go"
                        | "java"
                        | "vue"
                        | "jsx"
                        | "tsx"
                        | "yaml"
                        | "yml"
                ) {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                if meta.len() > 1024 * 1024 {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else { continue };
                for (i, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        let rel = path
                            .strip_prefix(dir)
                            .map(|r| r.to_string_lossy().to_string())
                            .unwrap_or_else(|_| path.to_string_lossy().to_string());
                        let snippet = line.trim().chars().take(80).collect::<String>();
                        hits.push(format!("{rel}:{i}: {snippet}"));
                        if hits.len() >= 30 {
                            break;
                        }
                    }
                }
            }
        }
    }
    if hits.is_empty() {
        format!("在 {dir} 里没有找到包含「{query}」的代码")
    } else {
        format!("找到 {} 处包含「{query}」的代码：\n{}", hits.len(), hits.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockExec {
        fail: bool,
        checked: std::sync::Arc<std::sync::Mutex<Vec<crate::codecheck::CheckRequest>>>,
    }

    impl MockExec {
        fn new(fail: bool) -> Self {
            Self {
                fail,
                checked: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl SkillExecutor for MockExec {
        async fn launch_app(&self, t: &str) -> Result<String, String> {
            if self.fail {
                Err("boom".into())
            } else {
                Ok(format!("launched {t}"))
            }
        }
        async fn open_folder(&self, p: &str) -> Result<String, String> {
            Ok(format!("opened {p}"))
        }
        async fn set_reminder(&self, m: u64, msg: &str) -> Result<String, String> {
            Ok(format!("reminder {m}min {msg}"))
        }
        async fn execute_command(&self, argv: &[String]) -> Result<String, String> {
            Ok(format!("ran {}", argv.join(",")))
        }
        async fn check_code(&self, req: crate::codecheck::CheckRequest) -> Result<String, String> {
            self.checked.lock().unwrap().push(req);
            if self.fail {
                Err("boom".into())
            } else {
                Ok("未发现问题".to_string())
            }
        }
    }


    /// 测试期强制走真实审批链路（debug 下 OPEN_ALL 默认全开，审批链路本来一条都走不到）。
    struct ForceApprovals;
    impl ForceApprovals {
        fn on() -> Self {
            unsafe { std::env::set_var("QIBAI_FORCE_APPROVALS", "1") };
            ForceApprovals
        }
    }
    impl Drop for ForceApprovals {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("QIBAI_FORCE_APPROVALS") };
        }
    }

    fn registry() -> SkillsRegistry {
        let r = SkillsRegistry::with_executor(SkillsSettings::default(), Box::new(MockExec::new(false)));
        let schemas = r.tool_schemas();
        let names: Vec<&str> = schemas.iter().filter_map(|sc| sc["function"]["name"].as_str()).collect();
        eprintln!("DBG schema names: {:?}", names);
        eprintln!("DBG status skills: {:?}", r.status()["skills"]);
        r
    }
    #[tokio::test]
    async fn whitelisted_skill_runs() {
        let mut r = registry();
        match r.invoke(SKILL_LAUNCH_APP, &serde_json::json!({"target":"firefox"})).await {
            SkillOutcome::Ok(msg) => assert!(msg.contains("firefox")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_skill_denied() {
        let mut r = registry();
        match r.invoke("hack_nasa", &serde_json::json!({})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn per_skill_disabled_denied() {
        let mut s = SkillsSettings::default();
        s.launch_app = false;
        let mut r = SkillsRegistry::with_executor(s, Box::new(MockExec::new(false)));
        match r.invoke(SKILL_LAUNCH_APP, &serde_json::json!({"target":"x"})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn global_toggle_disables_all() {
        let mut r = registry();
        r.set_total_enabled(false);
        match r.invoke(SKILL_OPEN_FOLDER, &serde_json::json!({"path":"/tmp"})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn dangerous_command_goes_pending_then_approval() {
        let _g = crate::test_support::env_lock();
        let _force = ForceApprovals::on();
        let mut r = registry();
        let out = r
            .invoke(
                SKILL_EXECUTE_COMMAND,
                &serde_json::json!({"argv":["rm","-rf","/"]}),
            )
            .await;
        let request_id = match out {
            SkillOutcome::PendingApproval { request_id, .. } => request_id,
            other => panic!("expected pending, got {other:?}"),
        };
        // 拒绝
        let msg = r.approve(&request_id, false).await.unwrap();
        assert!(msg.contains("取消"));
        // 已取消的不能再批准
        assert!(r.approve(&request_id, true).await.is_err());

        // 再来一次并批准
        let out = r
            .invoke(SKILL_EXECUTE_COMMAND, &serde_json::json!({"argv":["ls"]}))
            .await;
        if let SkillOutcome::PendingApproval { request_id, .. } = out {
            let res = r.approve(&request_id, true).await.unwrap();
            assert!(res.contains("ls"));
        } else {
            panic!("expected pending");
        }
    }

    /// 连点「允许本次」/ IPC 重试：第二次必须回放上次结果，而不是报「找不到请求」。
    #[tokio::test]
    async fn repeated_approve_replays_result_without_rerun() {
        let _g = crate::test_support::env_lock();
        let _force = ForceApprovals::on();
        let mut r = registry();
        let out = r
            .invoke(SKILL_EXECUTE_COMMAND, &serde_json::json!({"argv":["ls"]}))
            .await;
        let id = match out {
            SkillOutcome::PendingApproval { request_id, .. } => request_id,
            other => panic!("expected pending, got {other:?}"),
        };
        let first = r.approve(&id, true).await.unwrap();
        assert!(first.contains("ls"));
        assert_eq!(r.pending_len(), 0, "批准后不应残留 pending");

        // 同一条确认再提交一次（用户连点）：结果一致且标明未重复执行。
        let second = r.approve(&id, true).await.unwrap();
        expected_replay(&second, &first);
        assert_eq!(r.pending_len(), 0, "回放不应产生新的 pending");
    }

    fn expected_replay(second: &str, first: &str) {
        assert!(second.starts_with(first), "回放应带上原始结果：{second}");
        assert!(
            second.contains("未重复执行"),
            "回放要明说没重复执行：{second}"
        );
    }

    /// 未知 / 过期的 request_id：报「确认已失效」并给出下一步，不再像从前那样只说 not found。
    #[tokio::test]
    async fn unknown_request_id_explains_what_to_do() {
        let mut r = registry();
        let err = r
            .approve("approval-9999", true)
            .await
            .expect_err("未知请求必须报错")
            .to_string();
        assert!(err.contains("确认已失效"), "{err}");
        assert!(err.contains("重新发起"), "{err}");
        assert!(!err.contains("not found"), "不该再出现 not found：{err}");
    }

    /// 取消过的确认：再拒绝一次回同义结果，再批准一次明确拒绝（绝不执行）。
    #[tokio::test]
    async fn cancelled_request_is_stable_on_repeat() {
        let _g = crate::test_support::env_lock();
        let _force = ForceApprovals::on();
        let mut r = registry();
        let out = r
            .invoke(SKILL_EXECUTE_COMMAND, &serde_json::json!({"argv":["ls"]}))
            .await;
        let id = match out {
            SkillOutcome::PendingApproval { request_id, .. } => request_id,
            other => panic!("expected pending, got {other:?}"),
        };
        let first = r.approve(&id, false).await.unwrap();
        assert!(first.contains("取消"));
        assert!(r.approve(&id, false).await.unwrap().contains("取消"));
        let err = r.approve(&id, true).await.expect_err("已取消不能执行");
        assert!(err.to_string().contains("取消"), "{err}");
    }

    #[tokio::test]
    async fn executor_error_is_denied_not_panic() {        let mut r = SkillsRegistry::with_executor(
            SkillsSettings::default(),
            Box::new(MockExec::new(true)),
        );
        match r.invoke(SKILL_LAUNCH_APP, &serde_json::json!({"target":"x"})).await {
            SkillOutcome::Denied(e) => assert!(e.contains("boom")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn status_shape() {
        let r = registry();
        let v = r.status();
        assert!(v.get("total_enabled").unwrap().as_bool().unwrap());
        assert!(v["skills"].get(SKILL_LAUNCH_APP).unwrap().as_bool().unwrap());
        // check_code 纳入状态快照，默认 true
        assert!(v["skills"].get(SKILL_CHECK_CODE).unwrap().as_bool().unwrap());
    }

    // ---- check_code 技能（迭代①） ----

    /// recorder 直连：断言 path/snippet 两种路由与 language 透传。
    #[tokio::test]
    async fn check_code_records_path_and_snippet_requests() {
        use crate::codecheck::CheckSource;
        let exec = Box::new(MockExec::new(false));
        let recorder = exec.checked.clone();
        let mut r = SkillsRegistry::with_executor(SkillsSettings::default(), exec);

        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"/tmp/a.py"}))
            .await
        {
            SkillOutcome::Ok(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        match r
            .invoke(
                SKILL_CHECK_CODE,
                &serde_json::json!({"snippet":"def f(): pass","language":"python"}),
            )
            .await
        {
            SkillOutcome::Ok(_) => {}
            other => panic!("unexpected {other:?}"),
        }

        let reqs = recorder.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].source, CheckSource::Path);
        assert_eq!(reqs[0].path.as_deref().unwrap().to_string_lossy(), "/tmp/a.py");
        assert!(reqs[0].snippet.is_none());
        assert_eq!(reqs[1].source, CheckSource::Snippet);
        assert_eq!(reqs[1].snippet.as_deref().unwrap(), "def f(): pass");
        assert_eq!(reqs[1].language.as_deref(), Some("python"));
        assert!(reqs[1].path.is_none());
    }

    #[tokio::test]
    async fn check_code_rejects_both_or_neither_args() {
        let mut r = registry();
        // path 与 snippet 都给
        match r
            .invoke(
                SKILL_CHECK_CODE,
                &serde_json::json!({"path":"/a.js","snippet":"x"}),
            )
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        // 两者都不给
        match r.invoke(SKILL_CHECK_CODE, &serde_json::json!({})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        // 空字符串等价于不给
        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"  ","snippet":""}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_code_per_skill_toggle_and_approval_free() {
        // 默认开启
        let mut r = registry();
        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"/a.js"}))
            .await
        {
            SkillOutcome::Ok(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        // 用户关闭该技能 → Denied
        r.set_enabled(SKILL_CHECK_CODE, false).unwrap();
        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"/a.js"}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        // 总开关关闭同样 Denied
        r.set_total_enabled(false);
        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"/a.js"}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_code_executor_error_is_denied() {
        let mut r = SkillsRegistry::with_executor(
            SkillsSettings::default(),
            Box::new(MockExec::new(true)),
        );
        match r
            .invoke(SKILL_CHECK_CODE, &serde_json::json!({"path":"/a.js"}))
            .await
        {
            SkillOutcome::Denied(e) => assert!(e.contains("boom")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn check_code_tool_schema_present() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"check_code"));
        let sc = schemas
            .iter()
            .find(|s| s["function"]["name"] == "check_code")
            .unwrap();
        assert!(sc["function"]["description"].as_str().unwrap().contains("代码"));
        assert!(sc["function"]["parameters"]["properties"]["path"].is_object());
        assert!(sc["function"]["parameters"]["properties"]["snippet"].is_object());
        assert!(sc["function"]["parameters"]["properties"]["language"].is_object());
    }

    // ---- open_social 技能（迭代④，§9.2-5） ----

    #[test]
    fn open_social_in_whitelist_and_default_enabled() {
        assert!(ALL_SKILLS.contains(&SKILL_OPEN_SOCIAL));
        let r = registry();
        let v = r.status();
        assert!(v["skills"][SKILL_OPEN_SOCIAL].as_bool().unwrap());
    }

    #[tokio::test]
    async fn open_social_invoke_returns_open_text_with_view() {
        let mut r = registry();
        // 默认 planet
        match r.invoke(SKILL_OPEN_SOCIAL, &serde_json::json!({})).await {
            SkillOutcome::Ok(msg) => assert!(msg.contains("星球广场")),
            other => panic!("unexpected {other:?}"),
        }
        // city 视图
        match r
            .invoke(SKILL_OPEN_SOCIAL, &serde_json::json!({"view":"city"}))
            .await
        {
            SkillOutcome::Ok(msg) => assert!(msg.contains("同城灯塔")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_social_per_skill_toggle_denies() {
        let mut r = registry();
        r.set_enabled(SKILL_OPEN_SOCIAL, false).unwrap();
        match r
            .invoke(SKILL_OPEN_SOCIAL, &serde_json::json!({"view":"planet"}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn open_social_tool_schema_present() {
        let schemas = tool_schemas();
        let sc = schemas
            .iter()
            .find(|s| s["function"]["name"] == SKILL_OPEN_SOCIAL)
            .expect("open_social 应在工具表");
        let desc = sc["function"]["description"].as_str().unwrap();
        assert!(desc.contains("星球"));
        assert!(sc["function"]["parameters"]["properties"]["view"].is_object());
    }

    // ---- web_search / fetch_page 技能 ----

    #[test]
    fn fetch_page_in_whitelist_and_default_enabled() {
        assert!(ALL_SKILLS.contains(&SKILL_FETCH_PAGE));
        let r = registry();
        let v = r.status();
        assert!(v["skills"][SKILL_FETCH_PAGE].as_bool().unwrap());
    }

    #[test]
    fn fetch_page_tool_schema_present() {
        let schemas = tool_schemas();
        let sc = schemas
            .iter()
            .find(|s| s["function"]["name"] == SKILL_FETCH_PAGE)
            .expect("fetch_page 应在工具表");
        assert!(sc["function"]["description"].as_str().unwrap().contains("正文"));
        assert!(sc["function"]["parameters"]["properties"]["url"].is_object());
        let req = sc["function"]["parameters"]["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "url"));
    }

    #[tokio::test]
    async fn fetch_page_empty_url_denied_offline() {
        let mut r = registry();
        // 空参/缺参均优雅拒绝，不发网络请求
        match r.invoke(SKILL_FETCH_PAGE, &serde_json::json!({})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        match r
            .invoke(SKILL_FETCH_PAGE, &serde_json::json!({"url": "  "}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_page_non_http_url_denied_offline() {
        let mut r = registry();
        match r
            .invoke(SKILL_FETCH_PAGE, &serde_json::json!({"url": "ftp://x"}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }


    #[test]
    fn web_search_in_whitelist_and_default_enabled() {
        assert!(ALL_SKILLS.contains(&SKILL_WEB_SEARCH));
        let r = registry();
        let v = r.status();
        assert!(v["skills"][SKILL_WEB_SEARCH].as_bool().unwrap());
    }

    #[tokio::test]
    async fn web_search_empty_query_denied_offline() {
        let mut r = registry();
        // 空参/缺参均优雅拒绝，不发网络请求
        match r.invoke(SKILL_WEB_SEARCH, &serde_json::json!({})).await {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
        match r
            .invoke(SKILL_WEB_SEARCH, &serde_json::json!({"query": "  "}))
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn web_search_tool_schema_present() {
        let schemas = tool_schemas();
        let sc = schemas
            .iter()
            .find(|s| s["function"]["name"] == SKILL_WEB_SEARCH)
            .expect("web_search 应在工具表");
        assert!(sc["function"]["parameters"]["properties"]["query"].is_object());
        assert!(sc["function"]["parameters"]["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("query")));
    }

    // ---- file_ops 技能 ----

    #[test]
    fn file_ops_in_whitelist_and_default_enabled() {
        assert!(ALL_SKILLS.contains(&SKILL_FILE_OPS));
        let r = registry();
        let v = r.status();
        assert!(v["skills"][SKILL_FILE_OPS].as_bool().unwrap());
    }

    #[tokio::test]
    async fn file_ops_missing_action_denied() {
        let mut r = registry();
        match r
            .invoke(SKILL_FILE_OPS, &serde_json::json!({"path": "~/x"}))
            .await
        {
            SkillOutcome::Denied(e) => assert!(e.contains("action")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_ops_delete_outside_home_denied_before_approval() {
        let mut r = registry();
        // 越界路径在挂起审批之前就被消毒拒绝
        match r
            .invoke(
                SKILL_FILE_OPS,
                &serde_json::json!({"action": "delete", "path": "/etc/passwd"}),
            )
            .await
        {
            SkillOutcome::Denied(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_ops_delete_goes_pending_then_approval_executes() {
        let mut r = registry();
        // 在真实主目录里造一个待删目录（用后即删，无残留）。
        let home = crate::fileops::default_home().expect("home");
        let target = home.join(format!("qibai_fileops_del_{}", std::process::id()));
        std::fs::create_dir_all(&target).unwrap();
        let target_s = target.to_string_lossy().to_string();

        let out = r
            .invoke(
                SKILL_FILE_OPS,
                &serde_json::json!({"action": "delete", "path": target_s}),
            )
            .await;
        let request_id = match out {
            SkillOutcome::PendingApproval { request_id, description } => {
                assert!(description.contains("删除"));
                request_id
            }
            other => panic!("expected pending, got {other:?}"),
        };
        // 取消：目录保留
        let msg = r.approve(&request_id, false).await.unwrap();
        assert!(msg.contains("取消"));
        assert!(target.exists());
        // 再次请求并批准：真正删除
        let out = r
            .invoke(
                SKILL_FILE_OPS,
                &serde_json::json!({"action": "delete", "path": target_s}),
            )
            .await;
        if let SkillOutcome::PendingApproval { request_id, .. } = out {
            let res = r.approve(&request_id, true).await.unwrap();
            assert!(res.contains("已删除"), "实际: {res}");
        } else {
            panic!("expected pending");
        }
        assert!(!target.exists());
    }

    #[test]
    fn file_ops_tool_schema_present() {
        let schemas = tool_schemas();
        let sc = schemas
            .iter()
            .find(|s| s["function"]["name"] == SKILL_FILE_OPS)
            .expect("file_ops 应在工具表");
        let action = &sc["function"]["parameters"]["properties"]["action"];
        assert!(action["enum"].as_array().unwrap().contains(&serde_json::json!("delete")));
        assert!(sc["function"]["parameters"]["properties"]["dest"].is_object());
        assert!(sc["function"]["parameters"]["properties"]["content"].is_object());
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;

    fn tmp_home() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("qibai_save_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(d.join("sub"));
        d
    }

    #[test]
    fn parse_save_block_ok() {
        let out = "好的，代码生成啦。\n@qibai-save:begin\npath=/home/u/test.py\ncontent=QUJDRA==\n@qibai-save:end\n更多文字";
        let (p, c) = parse_skill_save_block(out).expect("block parsed");
        assert_eq!(p, "/home/u/test.py");
        assert_eq!(c, "QUJDRA==");
    }

    #[test]
    fn parse_save_block_absent() {
        assert!(parse_skill_save_block("没有保存块").is_none());
        assert!(parse_skill_save_block("@qibai-save:begin\npath=x\n@qibai-save:end").is_none());
    }

    #[test]
    fn validate_expands_tilde() {
        let home = tmp_home();
        let got = validate_save_target("~/sub/new.py", home.to_str().unwrap()).expect("ok");
        assert!(got.ends_with("/sub/new.py"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn validate_rejects_escape() {
        let home = tmp_home();
        assert!(validate_save_target("../evil.py", home.to_str().unwrap()).is_err());
        assert!(validate_save_target("~/../../evil.py", home.to_str().unwrap()).is_err());
        assert!(validate_save_target("/etc/passwd.py", home.to_str().unwrap()).is_err());
        assert!(validate_save_target("~/sub/evil.exe", home.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn validate_rejects_existing() {
        let home = tmp_home();
        let f = home.join("sub").join("exists.py");
        std::fs::write(&f, "x").unwrap();
        let got = validate_save_target(f.to_str().unwrap(), home.to_str().unwrap());
        assert!(got.is_err());
        let msg = got.unwrap_err();
        assert!(msg.contains("已存在"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn validate_ok_new_file() {
        let home = tmp_home();
        let f = home.join("sub").join("fresh.rs");
        let got = validate_save_target(f.to_str().unwrap(), home.to_str().unwrap()).expect("ok");
        assert!(got.ends_with("fresh.rs"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
