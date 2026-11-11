//! 插件注册表（T2，契约 §3.2）：扫描 / 校验 / 安装 / 卸载 / 启停 / 权限授予 / 动态工具 schema。
//!
//! - 扫描根：env `QIBAI_PLUGINS_DIR` 优先，否则 `data_dir()/plugins`。
//! - 坏 manifest 拒绝该目录并记录 error，不 panic、不中止整体扫描。
//! - id 冲突（内置 vs 外部 / 两个外部）→ 拒绝后到者。
//! - 内置 6 技能以"内置插件包"注册（信任包：grants 默认全授，无需安装确认）。
//! - 版本管理：记录 version 字符串；同版本重复安装 = no-op，跨版本 = 就地覆盖升级（保留启停/授权）。
//! - 重扫用 [`PluginRegistry::rescan`]：先摘旧外部条目再扫，避免"已注册 → 冲突"把好条目覆盖成 error。

use crate::plugin_manifest::{
    EntryKind, EntrySpec, FunctionSpec, Manifest, ManifestLoader, Permission,
};
pub use crate::plugin_manifest::PluginType;
use crate::settings::{PluginSettings, SkillsSettings};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// 内置技能分发枚举（仅在内置包定义处出现；注册表/分发层不按技能名字符串 match）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinKind {
    LaunchApp,
    OpenFolder,
    SetReminder,
    CheckCode,
    OpenSocial,
    ExecuteCommand,
    QueryWeather,
    ListSkills,
    WebSearch,
    /// v32 网页阅读器：抓取搜索结果 URL 的正文，供 LLM 总结（只读）。
    FetchPage,
    FileOps,
    CreateProject,
    ResumeProject,
    /// v24 编程技能：git 工作区状态（只读）。
    GitStatus,
    /// v24 编程技能：运行测试/检查命令（危险，需审批）。
    RunTests,
    /// v24 编程技能：代码内容/文件名搜索定位（只读）。
    CodeScan,
}

/// 注册表条目。
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: Manifest,
    pub installed: bool,
    pub enabled: bool,
    pub grants: HashSet<Permission>,
    pub builtin: Option<BuiltinKind>,
    /// 坏 manifest / 冲突 / 拒绝原因（plugin_list 展示）。
    pub error: Option<String>,
    /// 外部包目录（plugins/<id>）；内置为 None。
    pub dir: Option<PathBuf>,
}

impl PluginEntry {
    /// 是否可作为 LLM 工具暴露（execute_command 不进工具表——沿用现状）。
    pub fn is_exposed(&self) -> bool {
        !matches!(self.builtin, Some(BuiltinKind::ExecuteCommand))
    }
}

pub struct PluginRegistry {
    pub entries: HashMap<String, PluginEntry>,
    by_function: HashMap<String, String>,
    plugins_dir: PathBuf,
}

/// 扫描根：env `QIBAI_PLUGINS_DIR` 优先，否则 `data_dir()/plugins`。
pub fn plugins_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("QIBAI_PLUGINS_DIR") {
        return PathBuf::from(dir);
    }
    let dir = crate::settings::data_dir()
        .map(|d| d.join("plugins"))
        .unwrap_or_else(|_| PathBuf::from("./plugins"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

impl PluginRegistry {
    /// 构造：注册内置包 → 扫描外部目录 → 应用设置（含迁移播种）。
    pub fn new(plugin_settings: &PluginSettings, legacy: &SkillsSettings) -> Self {
        let dir = plugins_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut reg = Self {
            entries: HashMap::new(),
            by_function: HashMap::new(),
            plugins_dir: dir,
        };
        reg.register_builtins();
        reg.scan();
        reg.apply_settings(plugin_settings, legacy);
        reg
    }

    pub fn plugins_dir(&self) -> &std::path::Path {
        &self.plugins_dir
    }

    pub fn entry(&self, id: &str) -> Option<&PluginEntry> {
        self.entries.get(id)
    }

    /// 按 function name 解析条目（LLM tool → 插件）。
    pub fn resolve_function(&self, name: &str) -> Option<&PluginEntry> {
        self.by_function
            .get(name)
            .and_then(|id| self.entries.get(id))
    }

    /// function name → 插件 id（专家白名单过滤用）。
    pub fn plugin_id_for_function(&self, name: &str) -> Option<&str> {
        self.by_function.get(name).map(|s| s.as_str())
    }

    /// 内置 function name / 插件 id → 插件 id（set_enabled 双向接受）。
    pub fn resolve_toggle_id(&self, name_or_id: &str) -> Option<String> {
        if self.entries.contains_key(name_or_id) {
            return Some(name_or_id.to_string());
        }
        self.plugin_id_for_function(name_or_id)
            .map(|s| s.to_string())
    }

    /// 插件 id / 函数名 / 对外展示名（manifest.name）→ 插件 id。
    ///
    /// 技能页按 `manifest.name` 展示（如「Animacode 动画成片」），插件页按 id
    /// （`animacode`）；两处开关要落到同一条目，就得能按展示名反查。
    pub fn resolve_by_display(&self, name_or_id: &str) -> Option<String> {
        if let Some(id) = self.resolve_toggle_id(name_or_id) {
            return Some(id);
        }
        self.entries
            .iter()
            .find(|(_, e)| e.manifest.name == name_or_id)
            .map(|(id, _)| id.clone())
    }

    // ---- 内置插件包（契约 §1） ----

    fn register_builtins(&mut self) {
        let defs: Vec<(Manifest, BuiltinKind)> = vec![
            (
                Manifest {
                    id: "com.qibai.launch_app".into(),
                    plugin_type: PluginType::Skill,
                    name: "打开应用".into(),
                    description: "跨平台打开应用或网址".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["system:launch".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "launch_app".into(),
                        description: "在用户的电脑上打开一个应用或网址".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "target": {"type": "string", "description": "应用名或 URL"}
                            },
                            "required": ["target"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::LaunchApp,
            ),
            (
                Manifest {
                    id: "com.qibai.open_folder".into(),
                    plugin_type: PluginType::Skill,
                    name: "打开文件夹".into(),
                    description: "在文件管理器中打开文件夹".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["system:launch".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "open_folder".into(),
                        description: "在文件管理器中打开一个文件夹。用户口语里的「下载」「桌面」「文档」「图片」「音乐」「视频」等称呼会被自动解析为主目录下的对应文件夹".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "文件夹：可写绝对路径、~/ 开头、或直接写「下载」「桌面」「文档」这类名字；留空表示主目录"}
                            },
                            "required": ["path"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::OpenFolder,
            ),
            (
                Manifest {
                    id: "com.qibai.set_reminder".into(),
                    plugin_type: PluginType::Skill,
                    name: "设置提醒".into(),
                    description: "本地定时提醒".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["reminder:set".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "set_reminder".into(),
                        description: "设置一个本地提醒，到时通知用户".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "minutes": {"type": "integer", "description": "多少分钟后提醒"},
                                "message": {"type": "string", "description": "提醒内容"}
                            },
                            "required": ["minutes", "message"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::SetReminder,
            ),
            (
                Manifest {
                    id: "com.qibai.check_code".into(),
                    plugin_type: PluginType::Skill,
                    name: "代码检查".into(),
                    description: "只读本地代码静态检查".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "check_code".into(),
                        description: "检查用户指定的本地代码文件/文件夹或代码片段，返回结构化检查报告摘要".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "本地代码文件或文件夹路径（与 snippet 二选一）"},
                                "snippet": {"type": "string", "description": "待检查的代码片段文本（与 path 二选一）"},
                                "language": {"type": "string", "description": "可选，代码语言名，如 javascript / python；path 模式忽略"}
                            }
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::CheckCode,
            ),
            (
                Manifest {
                    id: "com.qibai.open_social".into(),
                    plugin_type: PluginType::Skill,
                    name: "打开社交".into(),
                    description: "联网社交（只读）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["network:read".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "open_social".into(),
                        description: "打开联网社交页面（星球广场 / 同城灯塔）。用户表达「带我去星球逛逛」「星球广场」「去星球看看」「同城看看」「打开社交」等意图时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "view": {"type": "string", "enum": ["planet", "city"], "description": "可选；planet=星球广场（默认），city=同城灯塔"}
                            }
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::OpenSocial,
            ),
            (
                Manifest {
                    id: "com.qibai.execute_command".into(),
                    plugin_type: PluginType::Skill,
                    name: "执行命令".into(),
                    description: "执行系统命令（逐次批准，危险技能）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["command:run".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "execute_command".into(),
                        description: "执行系统命令（危险，逐次批准）".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "argv": {"type": "array", "items": {"type": "string"}, "description": "命令参数数组"}
                            }
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::ExecuteCommand,
            ),
            (
                Manifest {
                    id: "com.qibai.query_weather".into(),
                    plugin_type: PluginType::Skill,
                    name: "查询天气".into(),
                    description: "查询城市实时天气并口头播报".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["network:http".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "query_weather".into(),
                        description: "查询指定城市的实时天气（气温、体感、天气现象、湿度、风速）。用户问天气/温度/下雨时调用，并把结果直接口头告诉用户".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "city": {"type": "string", "description": "城市名，如 石家庄、北京、上海"}
                            },
                            "required": ["city"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::QueryWeather,
            ),
            (
                Manifest {
                    id: "com.qibai.list_skills".into(),
                    plugin_type: PluginType::Skill,
                    name: "技能清单".into(),
                    description: "列出元气小小可用技能".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec![],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "list_skills".into(),
                        description: "列出元气小小当前可用的全部技能清单。用户问「你有什么技能」「你能做什么」「你能干啥」时调用，并把技能清单口头告诉用户".into(),
                        parameters: serde_json::json!({"type": "object", "properties": {}}),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::ListSkills,
            ),
            (
                Manifest {
                    id: "com.qibai.web_search".into(),
                    plugin_type: PluginType::Skill,
                    name: "联网搜索".into(),
                    description: "搜索互联网获取实时/最新信息（搜狗→360→移动百度降级链）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["network:http".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "web_search".into(),
                        description: "搜索互联网获取实时/最新信息（新闻/天气/百科/知识等）。用户问实时、最新、不知道需要查证的事情时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "query": {"type": "string", "description": "搜索关键词，不超过 200 字"}
                            },
                            "required": ["query"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::WebSearch,
            ),
            (
                Manifest {
                    id: "com.qibai.fetch_page".into(),
                    plugin_type: PluginType::Skill,
                    name: "网页阅读器".into(),
                    description: "抓取指定网页正文（输入 URL 读取文章内容，供总结/转述；JS 渲染页或需登录页会提示）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["network:http".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "fetch_page".into(),
                        description: "读取一个网页的正文内容（标题+正文纯文本）。web_search 拿到结果后，需要深入了解某条内容时调用本工具抓正文再总结".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "url": {"type": "string", "description": "网页完整地址（http/https），不超过 2048 字符"}
                            },
                            "required": ["url"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::FetchPage,
            ),
            (
                Manifest {
                    id: "com.qibai.file_ops".into(),
                    plugin_type: PluginType::Skill,
                    name: "文件助手".into(),
                    description: "浏览/打开/创建/重命名/移动/复制/写入/编辑/删除本地文件或文件夹（限主目录，删除与写文件需审批）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into(), "files:write".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "file_ops".into(),
                        description: "浏览/打开/创建/重命名/移动/复制/写入/编辑/删除本地文件或文件夹（删除与写文件需审批）。write=把 content 写入 path 新建或覆盖文本文件；edit=修改已有文件（dest=append 追加或 overwrite 覆盖，content 为新内容）".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "action": {"type": "string", "enum": ["list", "open", "create", "rename", "move", "copy", "write", "edit", "delete"], "description": "操作类型：list=列出文件夹，open=系统打开，create=新建空文本文件，write=写入/覆盖文本文件内容（需审批），edit=修改已有文件（需审批），rename/move=重命名或移动，copy=复制，delete=删除（需审批）"},
                                "path": {"type": "string", "description": "目标文件或文件夹绝对路径，或 ~/ 开头（限主目录内）"},
                                "dest": {"type": "string", "description": "rename/move/copy 的目标路径；edit 时填 append（追加）或 overwrite（覆盖）"},
                                "content": {"type": "string", "description": "create/write 要写入的文本内容；edit 时的新内容"}
                            },
                            "required": ["action", "path"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::FileOps,
            ),
            (
                Manifest {
                    id: "com.qibai.project".into(),
                    plugin_type: PluginType::Skill,
                    name: "项目工程".into(),
                    description: "在本地电脑创建工程（参考豆包工作/WorkBuddy 项目制）：按技术栈生成项目骨架、维护项目上下文、制定方案并等待用户确认后开工".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into(), "files:write".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![
                        FunctionSpec {
                            name: "create_project".into(),
                            description: "在本地电脑创建工程：选目录（限主目录内）+ 按技术栈生成骨架（Python/Node/Rust/通用 HTML）+ 写入项目上下文，创建前会请你确认。用户说「创建/新建一个 XX 项目」时调用".into(),
                            parameters: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string", "description": "项目名（将作为文件夹名，不允许路径穿越）"},
                                    "parent": {"type": "string", "description": "父目录绝对路径或 ~/ 开头，如 ~/Downloads"},
                                    "stack": {"type": "string", "description": "技术栈：python/node/rust/html（默认 python）"},
                                    "goal": {"type": "string", "description": "项目目标一句话（写入项目上下文）"}
                                },
                                "required": ["name", "parent"]
                            }),
                        },
                        FunctionSpec {
                            name: "project_context".into(),
                            description: "读取指定项目的上下文（目标/技术栈/当前方案/状态/任务清单），用于延续项目任务。用户提到某个已有项目并要接着做时，先调用此函数再回答".into(),
                            parameters: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string", "description": "项目名"}
                                },
                                "required": ["name"]
                            }),
                        },
                        FunctionSpec {
                            name: "project_plan".into(),
                            description: "为已有项目制定解决方案：把理解到的用户目标与解决方案写入项目上下文（状态待确认），等用户确认后开工".into(),
                            parameters: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string", "description": "项目名"},
                                    "goal": {"type": "string", "description": "本次任务目标"},
                                    "plan": {"type": "string", "description": "解决方案步骤（简明）"}
                                },
                                "required": ["name", "goal", "plan"]
                            }),
                        },
                        FunctionSpec {
                            name: "project_confirm".into(),
                            description: "用户确认方案后标记项目状态为 approved（开始执行）。用户说「确认/可以/开工/就这么办」时调用".into(),
                            parameters: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string", "description": "项目名"}
                                },
                                "required": ["name"]
                            }),
                        },
                    ],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::CreateProject,
            ),
            (
                Manifest {
                    id: "com.qibai.resume_project".into(),
                    plugin_type: PluginType::Skill,
                    name: "续作项目".into(),
                    description: "读取工程上下文摘要，跨会话继续上次任务（只读）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "resume_project".into(),
                        description: "继续/恢复某个工程：返回工程目标、上次进度、文件清单与下一步建议。name 留空则返回最近一次任务。用户说「继续上次任务」「接着上次的工程做」「恢复项目」时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "description": "工程名（留空 = 最近一次任务）"},
                                "parent_dir": {"type": "string", "description": "工程父目录绝对路径或 ~/ 开头；name 为空时可省略"}
                            }
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::ResumeProject,
            ),
            (
                Manifest {
                    id: "com.qibai.git_status".into(),
                    plugin_type: PluginType::Skill,
                    name: "Git 状态".into(),
                    description: "查看 git 仓库工作区状态与最近提交（只读，不修改任何内容）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "git_status".into(),
                        description: "查看指定目录的 git 仓库状态（git status 简版 + 最近 3 条提交）。用户问「看看代码改了什么」「git 状态」「最近提交」「工作区干不干净」时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "项目目录路径，省略时用当前项目目录"}
                            }
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::GitStatus,
            ),
            (
                Manifest {
                    id: "com.qibai.run_tests".into(),
                    plugin_type: PluginType::Skill,
                    name: "运行测试".into(),
                    description: "运行项目测试或编译检查命令（cargo test / cargo check / node --check / npm test；执行命令需用户确认）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["shell:execute".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "run_tests".into(),
                        description: "运行项目的测试或检查命令：cargo test 跑 Rust 测试、cargo check 编译检查、node --check 检查 JS 语法、npm test 跑前端测试。执行前需用户确认。用户说「跑一下测试」「检查代码能不能编译」「全量自测」「node 语法检查」时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "command": {"type": "string", "enum": ["cargo test", "cargo check", "node --check", "npm test"], "description": "要执行的测试/检查命令"}
                            },
                            "required": ["command"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::RunTests,
            ),
            (
                Manifest {
                    id: "com.qibai.code_scan".into(),
                    plugin_type: PluginType::Skill,
                    name: "代码扫描".into(),
                    description: "在项目代码中按关键字搜索文件与内容定位（只读，不修改代码）".into(),
                    icon: None,
                    version: "1.0.0".into(),
                    author: Some("qibai".into()),
                    permissions: vec!["files:read".into()],
                    entry: Some(EntrySpec {
                        kind: EntryKind::Builtin,
                        command: None,
                        http: None,
                        http_headers: std::collections::BTreeMap::new(),
                        mcp: None,
                    }),
                    functions: vec![FunctionSpec {
                        name: "code_scan".into(),
                        description: "在项目代码中搜索关键字定位文件与行号（类似 grep）。用户问「这个功能在哪」「找找 XX 的代码」「哪个文件里写了 XX」时调用".into(),
                        parameters: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "query": {"type": "string", "description": "要搜索的关键字（函数名/变量名/文案等）"},
                                "path": {"type": "string", "description": "项目目录路径，省略时用当前项目目录"}
                            },
                            "required": ["query"]
                        }),
                    }],
                    system_prompt: None,
                    skills: vec![],
                    persona_binding: None,
                },
                BuiltinKind::CodeScan,
            ),
        ];
        for (m, kind) in defs {
            let grants: HashSet<Permission> = m
                .permissions
                .iter()
                .filter_map(|p| Permission::from_str(p))
                .collect();
            let id = m.id.clone();
            for f in &m.functions {
                self.by_function.insert(f.name.clone(), id.clone());
            }
            self.entries.insert(
                id,
                PluginEntry {
                    manifest: m,
                    installed: true,
                    enabled: true, // 迁移时由 apply_settings 按旧 bool 覆盖
                    grants,
                    builtin: Some(kind),
                    error: None,
                    dir: None,
                },
            );
        }
    }

    // ---- 外部包扫描（契约 §3.2） ----

    /// 扫描 plugins_dir 下每个 `<id>/qibai-plugin.json`；坏 manifest 记录 error 不中止。
    pub fn scan(&mut self) {
        let Ok(rd) = std::fs::read_dir(&self.plugins_dir) else {
            return;
        };
        for d in rd.flatten() {
            let path = d.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()) else {
                continue;
            };
            self.scan_one(&dir_name, &path);
        }
    }

    /// 重扫外部目录：先摘掉旧的外部条目再扫。
    ///
    /// 早先 `install()` 落盘后直接调 `scan()`，此时 entries 里已有同 id 条目，
    /// `scan_one` 走"已注册 → 冲突"分支，把刚装好的条目**覆盖成 error 条目**
    /// （installed=false / error 有值），结果是装完就调不动，界面显示
    /// 「id 冲突：animacode 已注册，该目录被忽略」。这里改成重建外部条目，
    /// 并把旧条目的启停/授权按 id 带回去。
    pub fn rescan(&mut self) {
        let mut keep: std::collections::HashMap<String, (bool, HashSet<Permission>)> =
            std::collections::HashMap::new();
        for (id, e) in self.entries.iter() {
            if e.builtin.is_none() && e.installed && e.error.is_none() {
                keep.insert(id.clone(), (e.enabled, e.grants.clone()));
            }
        }
        let external: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.builtin.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        for id in external {
            self.entries.remove(&id);
        }
        let alive: std::collections::HashSet<String> = self.entries.keys().cloned().collect();
        self.by_function.retain(|_, v| alive.contains(v));
        self.scan();
        for (id, (on, grants)) in keep {
            if let Some(e) = self.entries.get_mut(&id) {
                if e.error.is_none() {
                    e.enabled = on;
                    e.grants = grants;
                }
            }
        }
    }

    fn scan_one(&mut self, dir_name: &str, path: &std::path::Path) {
        // 同一目录重复扫描：保留现值，别误记"已注册"冲突（rescan 之外的重入也要安全）。
        if let Some(prev) = self.entries.get(dir_name) {
            if prev.dir.as_deref() == Some(path) {
                return;
            }
        }
        // 已存在同名条目（内置或已安装）→ 冲突，拒绝后到者。
        if self.entries.contains_key(dir_name) {
            self.record_bad(
                dir_name,
                Some(path.to_path_buf()),
                format!("id 冲突：{dir_name} 已注册，该目录被忽略"),
            );
            return;
        }
        match ManifestLoader::load(path) {
            Ok(m) => {
                if m.id != dir_name {
                    self.record_bad(
                        dir_name,
                        Some(path.to_path_buf()),
                        format!("清单 id {} 与目录名 {dir_name} 不一致", m.id),
                    );
                    return;
                }
                // 外部插件声明 kind=builtin → 拒绝（builtin 仅限内置包）。
                let is_builtin_kind = m
                    .entry
                    .as_ref()
                    .map(|e| e.kind == EntryKind::Builtin)
                    .unwrap_or(false);
                if is_builtin_kind {
                    self.record_bad(
                        dir_name,
                        Some(path.to_path_buf()),
                        "外部插件声明 kind=builtin 被拒绝（builtin 仅限内置包）".to_string(),
                    );
                    return;
                }
                // 函数名冲突 → 拒绝后到者。
                for f in &m.functions {
                    if self.by_function.contains_key(&f.name) {
                        self.record_bad(
                            dir_name,
                            Some(path.to_path_buf()),
                            format!("函数名 {} 与已注册插件冲突", f.name),
                        );
                        return;
                    }
                }
                for f in &m.functions {
                    self.by_function.insert(f.name.clone(), m.id.clone());
                }
                self.entries.insert(
                    m.id.clone(),
                    PluginEntry {
                        manifest: m,
                        installed: true,
                        enabled: false,
                        grants: HashSet::new(),
                        builtin: None,
                        error: None,
                        dir: Some(path.to_path_buf()),
                    },
                );
            }
            Err(e) => {
                self.record_bad(dir_name, Some(path.to_path_buf()), e.to_string());
            }
        }
    }

    fn record_bad(&mut self, id: &str, dir: Option<PathBuf>, error: String) {
        let manifest = Manifest {
            id: id.to_string(),
            plugin_type: PluginType::Skill,
            name: id.to_string(),
            description: String::new(),
            icon: None,
            version: "0.0.0".into(),
            author: None,
            permissions: vec![],
            entry: None,
            functions: vec![],
            system_prompt: None,
            skills: vec![],
            persona_binding: None,
        };
        self.entries.insert(
            id.to_string(),
            PluginEntry {
                manifest,
                installed: false,
                enabled: false,
                grants: HashSet::new(),
                builtin: None,
                error: Some(error),
                dir,
            },
        );
    }

    // ---- 安装 / 卸载 / 启停 / 授权 ----

    /// 安装：复制源目录到 plugins/<id>/。
    /// 幂等：同 id 同版本 → no-op；**同 id 跨版本 → 覆盖升级**（保留启停与已授权限）。
    pub fn install(&mut self, from_dir: &std::path::Path) -> Result<Manifest, String> {
        let m = ManifestLoader::load(from_dir).map_err(|e| e.to_string())?;
        let mut carry: Option<(bool, HashSet<Permission>)> = None;
        match self.entries.get(&m.id) {
            Some(e) if e.builtin.is_some() => {
                return Err(format!("{} 为内置插件，不可安装", m.id));
            }
            Some(e) if e.installed && e.error.is_none() => {
                if e.manifest.version == m.version {
                    return Ok(m); // 幂等 no-op
                }
                // 跨版本：覆盖升级，不再要求"先卸载再安装"。
                carry = Some((e.enabled, e.grants.clone()));
            }
            _ => {}
        }
        // 外部 builtin-kind 拒绝。
        let is_builtin_kind = m
            .entry
            .as_ref()
            .map(|e| e.kind == EntryKind::Builtin)
            .unwrap_or(false);
        if is_builtin_kind {
            return Err("外部插件声明 kind=builtin 被拒绝（builtin 仅限内置包）".to_string());
        }
        // 函数名冲突（升级时同名函数属自己，先摘掉自己的旧映射再查，免得自撞）。
        self.by_function.retain(|_, v| v != &m.id);
        for f in &m.functions {
            if self.by_function.contains_key(&f.name) {
                return Err(format!("函数名 {} 与已注册插件冲突", f.name));
            }
        }
        let target = self.plugins_dir.join(&m.id);
        let _ = std::fs::remove_dir_all(&target);
        crate::skill_adapter::copy_dir_all(from_dir, &target)
            .map_err(|e| format!("复制插件目录失败: {e}"))?;
        // 复制后按目标目录重载清单：`{PLUGIN_DIR}` 要按最终落点解析，
        // 否则 entry 里存的还是源目录/临时目录路径，装完就指向不存在的文件。
        let m = ManifestLoader::load(&target)
            .map_err(|e| format!("目标目录清单重载失败: {e}"))?;
        for f in &m.functions {
            self.by_function.insert(f.name.clone(), m.id.clone());
        }
        let (enabled, grants) = carry.unwrap_or((false, HashSet::new()));
        self.entries.insert(
            m.id.clone(),
            PluginEntry {
                manifest: m.clone(),
                installed: true,
                enabled,
                grants,
                builtin: None,
                error: None,
                dir: Some(target),
            },
        );
        Ok(m)
    }

    /// 卸载：幂等（不存在 → ok）；内置插件拒绝。
    pub fn uninstall(&mut self, id: &str) -> Result<(), String> {
        match self.entries.get(id) {
            Some(e) if e.builtin.is_some() => {
                return Err(format!("{id} 为内置插件，不可卸载"));
            }
            None => return Ok(()),
            _ => {}
        }
        if let Some(e) = self.entries.remove(id) {
            if let Some(dir) = &e.dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            self.by_function.retain(|_, v| v != id);
        }
        Ok(())
    }

    /// 启停：幂等；未知 id → 错误。
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), String> {
        let e = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("未知插件 {id}"))?;
        e.enabled = on;
        Ok(())
    }

    /// 权限授予/撤销（幂等）。files:write 对外部插件硬拒（T8 红线）。
    pub fn grant_permission(
        &mut self,
        id: &str,
        perm: Permission,
        approve: bool,
    ) -> Result<(), String> {
        let e = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("未知插件 {id}"))?;
        if e.builtin.is_none() && perm == Permission::FilesWrite {
            return Err("files:write 对外部插件永久拒绝（本地优先红线）".to_string());
        }
        if approve {
            e.grants.insert(perm);
        } else {
            e.grants.remove(&perm);
        }
        Ok(())
    }

    /// 运行时权限强制（§3.5 第 4 步）：返回缺失的第一个权限；None = 授权覆盖。
    pub fn missing_permission(&self, id: &str) -> Option<Permission> {
        let e = self.entries.get(id)?;
        for p in &e.manifest.permissions {
            let Some(perm) = Permission::from_str(p) else {
                continue;
            };
            if !e.grants.contains(&perm) {
                return Some(perm);
            }
        }
        None
    }

    // ---- 设置应用（含迁移，幂等） ----

    /// 应用 PluginSettings：enabled 为空 → 从旧 SkillsSettings 播种内置启停（迁移不写盘）；
    /// 非空 → 以之为准。内置 grants 默认为全授（信任包）。
    pub fn apply_settings(&mut self, ps: &PluginSettings, legacy: &SkillsSettings) {
        if ps.enabled.is_empty() {
            // 迁移播种
            for e in self.entries.values_mut() {
                e.enabled = match e.builtin {
                    Some(BuiltinKind::LaunchApp) => legacy.launch_app,
                    Some(BuiltinKind::OpenFolder) => legacy.open_folder,
                    Some(BuiltinKind::SetReminder) => legacy.set_reminder,
                    Some(BuiltinKind::CheckCode) => legacy.check_code,
                    Some(BuiltinKind::OpenSocial) => legacy.open_social,
                    Some(BuiltinKind::ExecuteCommand) => true,
                    Some(BuiltinKind::QueryWeather) => true,
                    Some(BuiltinKind::ListSkills) => true,
                    Some(BuiltinKind::WebSearch) => legacy.web_search,
                    Some(BuiltinKind::FetchPage) => legacy.fetch_page,
                    Some(BuiltinKind::FileOps) => legacy.file_ops,
                    Some(BuiltinKind::CreateProject) => true,
                    Some(BuiltinKind::ResumeProject) => true,
                    Some(BuiltinKind::GitStatus) => legacy.git_status,
                    Some(BuiltinKind::RunTests) => legacy.run_tests,
                    Some(BuiltinKind::CodeScan) => legacy.code_scan,
                    None => false,
                };
            }
        } else {
            for e in self.entries.values_mut() {
                if let Some(on) = ps.enabled.get(&e.manifest.id) {
                    e.enabled = *on;
                }
                // 内置缺省保持注册默认 true；外部缺省 false。
            }
        }
        // grants：内置 = manifest 声明全授；外部 = settings.grants 落盘值。
        for e in self.entries.values_mut() {
            if e.builtin.is_some() {
                e.grants = e
                    .manifest
                    .permissions
                    .iter()
                    .filter_map(|p| Permission::from_str(p))
                    .collect();
            }
        }
        for e in self.entries.values_mut() {
            if e.builtin.is_none() {
                if let Some(perms) = ps.grants.get(&e.manifest.id) {
                    e.grants = perms.iter().filter_map(|p| Permission::from_str(p)).collect();
                }
            }
        }
    }

    // ---- 动态工具 schema（§3.2：仅 enabled 条目） ----

    pub fn tool_schemas(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for e in self.entries.values() {
            if !e.enabled || e.error.is_some() {
                continue;
            }
            // v27：MCP 生态打通——LLM 工具表放行 Skill 与 Connector(mcp) 条目。
            // 外部插件经 exec_external → adapter 按 entry.kind 执行，MCP 工具
            // 首连做 tools/list 校验，函数名与 manifest functions 必须吻合。
            let mcp_ok = e.manifest.plugin_type == PluginType::Connector
                && e
                    .manifest
                    .entry
                    .as_ref()
                    .map(|en| en.kind == EntryKind::Mcp)
                    .unwrap_or(false);
            if e.manifest.plugin_type != PluginType::Skill && !mcp_ok {
                continue;
            }
            if !e.is_exposed() {
                continue;
            }
            for f in &e.manifest.functions {
                out.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": f.name,
                        "description": f.description,
                        "parameters": f.parameters,
                    }
                }));
            }
        }
        out
    }

    /// expert 类型条目（供 ExpertRegistry 装配）。
    pub fn expert_manifests(&self) -> Vec<Manifest> {
        self.entries
            .values()
            .filter(|e| {
                e.manifest.plugin_type == PluginType::Expert
                    && e.error.is_none()
                    && e.installed
            })
            .map(|e| e.manifest.clone())
            .collect()
    }

    /// plugin_list IPC 形状（契约 §3.6）。
    pub fn list_json(&self) -> Vec<serde_json::Value> {
        let mut ids: Vec<&String> = self.entries.keys().collect();
        ids.sort();
        ids.into_iter()
            .map(|id| {
                let e = &self.entries[id];
                serde_json::json!({
                    "id": e.manifest.id,
                    "type": e.manifest.plugin_type.as_str(),
                    "name": e.manifest.name,
                    "description": e.manifest.description,
                    "icon": e.manifest.icon,
                    "version": e.manifest.version,
                    "author": e.manifest.author,
                    "permissions": e.manifest.permissions,
                    "granted": e.grants.iter().map(|p| p.as_str().to_string()).collect::<Vec<_>>(),
                    "enabled": e.enabled,
                    "installed": e.installed,
                    "entry_kind": e.manifest.entry.as_ref().map(|k| k.kind.as_str()),
                    "functions": e.manifest.functions.iter().map(|f| serde_json::json!({
                        "name": f.name, "description": f.description
                    })).collect::<Vec<_>>(),
                    "error": e.error,
                    "builtin": e.builtin.is_some(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SkillsSettings;
    use crate::test_support::{env_lock, unique_temp_dir};

    fn write_plugin(dir: &std::path::Path, id: &str, manifest: serde_json::Value) {
        let p = dir.join(id);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("qibai-plugin.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn builtins_register_with_permissions_and_schemas() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-builtin");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        // 16 个内置（v20 web_search/file_ops；并行方 create_project；本批 resume_project；v24 git_status/run_tests/code_scan；v32 fetch_page）
        assert_eq!(reg.entries.len(), 16);
        // by_function 含 10 个函数名（execute_command 也可解析）
        assert!(reg.resolve_function("launch_app").is_some());
        assert!(reg.resolve_function("execute_command").is_some());
        assert!(reg.resolve_function("query_weather").is_some());
        assert!(reg.resolve_function("fetch_page").is_some());
        assert!(reg.resolve_function("list_skills").is_some());
        assert!(reg.resolve_function("web_search").is_some());
        assert!(reg.resolve_function("file_ops").is_some());
        // tool_schemas = 18 个（execute_command 不进表；v24 新增 git_status/run_tests/code_scan；v32 新增 fetch_page）
        let schemas = reg.tool_schemas();
        // v43：open_folder 真机默认关闭，工具表只剩 17 个；避免误开文件管理器。
        assert_eq!(schemas.len(), 17);
        let mut names: Vec<&str> = schemas.iter().map(|s| s["function"]["name"].as_str().unwrap()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["check_code", "code_scan", "create_project", "fetch_page", "file_ops", "git_status", "launch_app", "list_skills", "open_social", "project_confirm", "project_context", "project_plan", "query_weather", "resume_project", "run_tests", "set_reminder", "web_search"]
        );
        // 内置 grants 默认全授
        assert!(reg.missing_permission("com.qibai.launch_app").is_none());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn migration_seeds_from_legacy_when_enabled_empty() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-migrate");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let mut legacy = SkillsSettings::default();
        legacy.launch_app = false; // 用户关过
        let reg = PluginRegistry::new(&PluginSettings::default(), &legacy);
        assert!(!reg.entry("com.qibai.launch_app").unwrap().enabled);
        // v43：open_folder 默认关闭，防止误触/拖拽后自动弹文件管理器。
        assert!(!reg.entry("com.qibai.open_folder").unwrap().enabled);
        // execute_command 无旧开关 → 默认 true
        assert!(reg.entry("com.qibai.execute_command").unwrap().enabled);
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn nonempty_enabled_takes_precedence_and_is_idempotent() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-enabled");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let mut ps = PluginSettings::default();
        ps.enabled.insert("com.qibai.open_folder".into(), false);
        let reg1 = PluginRegistry::new(&ps, &SkillsSettings::default());
        assert!(!reg1.entry("com.qibai.open_folder").unwrap().enabled);
        assert!(reg1.entry("com.qibai.launch_app").unwrap().enabled);
        // 重复加载结果一致（幂等）
        let reg2 = PluginRegistry::new(&ps, &SkillsSettings::default());
        assert_eq!(
            reg1.entry("com.qibai.open_folder").unwrap().enabled,
            reg2.entry("com.qibai.open_folder").unwrap().enabled
        );
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn bad_manifest_is_recorded_not_fatal() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-bad");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        // 坏目录：非法 JSON
        let p = tmp.join("com.bad.x");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("qibai-plugin.json"), "{oops").unwrap();
        // 正常目录
        write_plugin(&tmp, "com.good.y", serde_json::json!({
            "id": "com.good.y", "type": "skill", "name": "y", "version": "1.0.0",
            "entry": {"kind": "command", "command": "a.sh"}, "functions": [{"name": "fy"}]
        }));
        let reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        assert!(reg.entry("com.bad.x").unwrap().error.is_some());
        assert!(reg.entry("com.good.y").unwrap().error.is_none());
        assert!(reg.resolve_function("fy").is_some());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn external_builtin_kind_is_rejected() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-extbuiltin");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        write_plugin(&tmp, "com.hack.builtin", serde_json::json!({
            "id": "com.hack.builtin", "type": "skill", "name": "h", "version": "1.0.0",
            "entry": {"kind": "builtin"}, "functions": [{"name": "fhack"}]
        }));
        let reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        let e = reg.entry("com.hack.builtin").unwrap();
        assert!(e.error.as_ref().unwrap().contains("builtin"));
        assert!(reg.resolve_function("fhack").is_none());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn install_uninstall_idempotent_and_version_locked() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-install");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let src = unique_temp_dir("reg-src");
        write_plugin(&src, "com.x.demo", serde_json::json!({
            "id": "com.x.demo", "type": "skill", "name": "d", "version": "1.0.0",
            "entry": {"kind": "command", "command": "run.sh"}, "functions": [{"name": "fdemo"}]
        }));
        let from = src.join("com.x.demo");
        let mut reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        reg.install(&from).unwrap();
        assert!(reg.entry("com.x.demo").unwrap().installed);
        assert!(!reg.entry("com.x.demo").unwrap().enabled); // 外部默认关
        // 幂等 no-op
        reg.install(&from).unwrap();
        // 改版本 → 覆盖升级（v45.14：不再要求先卸载），版本要更新
        std::fs::remove_dir_all(&from).unwrap();
        write_plugin(&src, "com.x.demo", serde_json::json!({
            "id": "com.x.demo", "type": "skill", "name": "d", "version": "2.0.0",
            "entry": {"kind": "command", "command": "run.sh"}, "functions": [{"name": "fdemo"}]
        }));
        let up = reg.install(&src.join("com.x.demo")).unwrap();
        assert_eq!(up.version, "2.0.0", "同 id 跨版本应就地升级");
        assert!(reg.entry("com.x.demo").unwrap().installed);
        // 卸载幂等
        reg.uninstall("com.x.demo").unwrap();
        reg.uninstall("com.x.demo").unwrap();
        assert!(reg.entry("com.x.demo").is_none());
        // 内置卸载拒绝
        assert!(reg.uninstall("com.qibai.launch_app").is_err());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    #[test]
    fn files_write_hard_reject_for_external_plugins() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-grant");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let src = unique_temp_dir("reg-grant-src");
        write_plugin(&src, "com.x.w", serde_json::json!({
            "id": "com.x.w", "type": "skill", "name": "w", "version": "1.0.0",
            "entry": {"kind": "command", "command": "run.sh"}, "functions": [{"name": "fw"}]
        }));
        let mut reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        reg.install(&src.join("com.x.w")).unwrap();
        // command:run 可授
        reg.grant_permission("com.x.w", Permission::CommandRun, true).unwrap();
        assert!(reg.entry("com.x.w").unwrap().grants.contains(&Permission::CommandRun));
        // files:write 硬拒
        assert!(reg.grant_permission("com.x.w", Permission::FilesWrite, true).is_err());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// 装完后再扫一遍不能把条目毒化成 error（"id 已注册"那条报错的根因）。
    #[test]
    fn rescan_after_install_keeps_entry_usable() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-rescan");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let src = unique_temp_dir("reg-rescan-src");
        write_plugin(&src, "animacode", serde_json::json!({
            "id": "animacode", "type": "connector", "name": "Animacode 动画成片", "version": "2.2.0",
            "entry": {"kind": "mcp", "command": "node", "args": ["{PLUGIN_DIR}/mcp/server.mjs"]},
            "permissions": ["system:launch"],
            "functions": [{"name": "anima_build"}, {"name": "anima_render"}]
        }));
        let mut reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        reg.install(&src.join("animacode")).unwrap();
        reg.rescan();
        let e = reg.entry("animacode").expect("重扫后条目不能消失");
        assert!(e.error.is_none(), "重扫不能记冲突：{:?}", e.error);
        assert!(e.installed, "重扫后应仍是已安装状态");
        // 启用后工具必须进表（否则"导入成功但调不动"）
        reg.set_enabled("animacode", true).unwrap();
        let names: Vec<String> = reg
            .tool_schemas()
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(names.contains(&"anima_build".to_string()));
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }

    /// 同 id 跨版本重装：保留启停与授权，不要求先卸载。
    #[test]
    fn upgrade_keeps_enabled_and_grants() {
        let _g = env_lock();
        let tmp = unique_temp_dir("reg-upgrade");
        unsafe { std::env::set_var("QIBAI_PLUGINS_DIR", &tmp) };
        let src = unique_temp_dir("reg-upgrade-src");
        let mut reg = PluginRegistry::new(&PluginSettings::default(), &SkillsSettings::default());
        let write = |v: &str| {
            let _ = std::fs::remove_dir_all(src.join("animacode"));
            write_plugin(&src, "animacode", serde_json::json!({
                "id": "animacode", "type": "connector", "name": "Animacode 动画成片", "version": v,
                "entry": {"kind": "mcp", "command": "node", "args": ["{PLUGIN_DIR}/mcp/server.mjs"]},
                "permissions": ["system:launch"],
                "functions": [{"name": "anima_build"}]
            }));
        };
        write("2.1.0");
        let m = reg.install(&src.join("animacode")).unwrap();
        assert_eq!(m.version, "2.1.0");
        reg.set_enabled("animacode", true).unwrap();
        reg.grant_permission("animacode", Permission::SystemLaunch, true).unwrap();
        write("2.2.0");
        let m = reg.install(&src.join("animacode")).unwrap();
        assert_eq!(m.version, "2.2.0", "同 id 跨版本应就地升级");
        let e = reg.entry("animacode").unwrap();
        assert!(e.enabled, "升级应保留启用状态");
        assert!(e.grants.contains(&Permission::SystemLaunch), "升级应保留已授权限");
        assert!(e.error.is_none());
        unsafe { std::env::remove_var("QIBAI_PLUGINS_DIR") };
    }
}
