//! 语音 UI 指令表：把用户说的中文话匹配成确定性 UI/系统动作。
//!
//! 设计约束：
//! - **纯核心层**：不依赖 tauri / 窗口 / engine，只做「文本 → 意图」匹配，
//!   执行器在 src-tauri 层（listener.rs 语音链路 / main.rs voice_ui_dispatch 命令）。
//! - **顺序敏感**：精确/窄规则在前，宽泛规则在后；同义易混词按
//!   「静音 > 控制台 > 导航 > 文件 > 搜索 > 启动 > 退出」优先级排列，
//!   防误伤（例：「打开声音」先命中 MuteOff，绝不能落到 ConsoleOpen）。
//! - **闲聊放行**：问候 / 自由闲聊一律返回 None，交给 LLM 对话链路。
//! - 不引入 regex 依赖（Cargo.toml 无 regex），全部手写字符串扫描。

use serde_json::json;

/// 一条可被 UI/系统层直接执行的语音指令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCmd {
    /// 打开控制台窗（ensure_console）。
    ConsoleOpen,
    /// 关闭控制台窗。
    ConsoleClose,
    /// 控制台内页面导航：参数 = tab id（chat/plugins/roles/settings/vault/social/events/pk/codecheck）。
    UiNav(String),
    /// 触发一次全屏守护演出（guard_trigger）。
    GuardTrigger,
    /// 切换主角色：参数 = persona id（gudian/dushi/wenrou/huopo/reqing）。
    SetPersona(String),
    /// 锁定 active persona 桌宠位置/窗口。
    Lock,
    /// 解除桌宠锁定。
    Unlock,
    /// 全局静音（关闭 TTS 出声）。
    MuteOn,
    /// 取消静音。
    MuteOff,
    /// 极简模式（只留桌宠，隐藏面板/控制台装饰）。
    MinimalOn,
    /// 完整界面恢复。
    MinimalOff,
    /// 桌宠窗口形态切换：compact=贴边小窗 / large=居中大窗 / min=最小化 / restore=恢复贴边小窗。
    PetMode(String),
    /// 联网搜索：参数 = query（走 web_search 技能）。
    Search(String),
    /// 文件操作：action(path/dest/content 捕获后走 file_ops 技能)。
    FileOp {
        action: String,
        path: String,
        dest: Option<String>,
        content: Option<String>,
    },
    /// 打开网站/网页：参数 = 目标（URL 或站名）。
    Launch(String),
    /// 请求退出（需二次确认，不直接退）。
    Exit,
    /// 确认退出（真退）。
    ExitConfirm,
    /// 批准上一条待确认操作（file_ops/create_project 的 PendingApproval）。
    ApproveLast,
    /// 否决上一条待确认操作。
    DenyLast,
    /// 创建本地工程：name/parent_dir/lang/goal（走 create_project 技能）。
    CreateProject {
        name: String,
        parent_dir: String,
        lang: String,
        goal: String,
    },
    /// 继续上次/指定工程：name 为空 = 最近任务（走 resume_project 技能）。
    ResumeProject { name: String, parent_dir: String },
    /// 开启免提双工（常开聆听，唤醒词+VAD；= 前端 set_wake_enabled(true)）。
    HandsFreeOn,
    /// 关闭免提双工（回到按住说话 PTT；= 前端 set_wake_enabled(false)）。
    HandsFreeOff,
    /// 设置/取消提醒：参数 kind = sit/water/eye 或 "off"（取消某类）。
    Remind(String),
    /// 显式表情/动作演出：参数 = emotion 或 action 名（happy/wave/stretch…）。
    Express(String),
    /// 桌宠窗口调整：参数 = large/small/left/right/up/reset。
    PetWindow(String),
    /// v24 UI 开关：记住对话（settings.llm.remember_chat=true）。
    RememberOn,
    /// v24 UI 开关：忘记对话（settings.llm.remember_chat=false）。
    RememberOff,
    /// v24 UI 开关：开启贴边吸附（settings.ui.snap_to_edge=true）。
    SnapOn,
    /// v24 UI 开关：关闭贴边吸附（settings.ui.snap_to_edge=false）。
    SnapOff,
    /// v24 UI 开关：隐藏桌宠窗口（emit qibai://pet-hide）。
    HidePet,
    /// v24 UI 开关：显示桌宠窗口（emit qibai://pet-show）。
    ShowPet,
    /// v24 UI 开关：开启联网搜索技能（settings.skills.web_search=true）。
    WebSearchOn,
    /// v24 UI 开关：关闭联网搜索技能（settings.skills.web_search=false）。
    WebSearchOff,
    // 预留：后续 MCP 意图（连接知识库 / 调用 xx 智能体）在此扩展。
}

/// 匹配结果：命令 + 要朗读/落库的回复 + 捕获参数（供执行器取用）。
#[derive(Debug, Clone)]
pub struct UiCmdMatch {
    pub cmd: UiCmd,
    pub reply: String,
    pub args: serde_json::Value,
}

/// 取某个关键词之后的尾巴（最后一次出现位置），去标点与填充词。
fn tail_after(text: &str, kw: &str) -> String {
    let Some(pos) = text.rfind(kw) else {
        return String::new();
    };
    let rest = &text[pos + kw.len()..];
    strip_filler(rest)
}

/// 去掉尾巴里的标点与无信息填充词（路径/query 提取共用）。
fn strip_filler(s: &str) -> String {
    let mut s = s.trim();
    for ch in ['，', '。', '！', '？', '；', '：', ',', '.', '!', '?', ';', ':', ' ', '\t', '\n', '"', '\'', '“', '”', '‘', '’'] {
        s = s.trim_matches(ch);
    }
    for w in ["一下", "帮我", "请", "吧", "呢", "啊"] {
        s = s.strip_prefix(w).unwrap_or(s);
    }
    s.to_string()
}

/// 角色关键词表：(persona_id, 别名列表)。与 listener.rs try_persona_intent 保持一致。
const PERSONA_MAP: &[(&str, &[&str])] = &[
    ("gudian", &["古典佳人", "古典", "古风", "gudian"]),
    ("dushi", &["都市女性", "都市", "知性", "dushi"]),
    ("wenrou", &["温柔", "wenrou"]),
    ("huopo", &["活泼", "元气", "huopo"]),
    ("reqing", &["热情时尚", "热情", "时尚", "reqing"]),
];

/// 导航页关键词表：(tab_id, 页面关键词)。
const NAV_MAP: &[(&str, &[&str])] = &[
    ("chat", &["聊天", "对话"]),
    ("plugins", &["技能", "插件", "专家"]),
    ("roles", &["角色", "形象"]),
    ("settings", &["设置"]),
    ("social", &["社交", "星球"]),
    ("events", &["诊断", "状态"]),
    ("pk", &["猜拳"]),
    ("codecheck", &["代码检查", "代码审查", "代码体检"]),
];

/// 语音指令主匹配入口：先精确后宽泛；未命中返回 None（交给 LLM 闲聊）。
pub fn match_ui_command(raw: &str) -> Option<UiCmdMatch> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }

    // 0) 退出确认（必须最前：「确认退出」不能落到普通 Exit 再追问一次）。
    if text.contains("确认退出") || text.contains("是的退出") || text.contains("确认关闭") {
        return Some(UiCmdMatch {
            cmd: UiCmd::ExitConfirm,
            reply: "好，我这就退下啦，想我了再打开我哦～".to_string(),
            args: json!({}),
        });
    }

    // 1) 静音 / 取消静音（先判 Off：「取消静音」含「静音」字样，必须先截走；
    // 再判一切「打开/关闭」类规则，防「打开声音」误开控制台）。
    if contains_any(text, &["取消静音", "解除静音", "打开声音", "恢复声音", "把声音打开"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::MuteOff,
            reply: "声音打开啦，我又能陪你聊天咯～".to_string(),
            args: json!({}),
        });
    }
    if contains_any(text, &["静音", "关闭声音", "别说话", "把声音关掉", "关掉声音", "不要出声"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::MuteOn,
            reply: "好，我静音啦，有事打字告诉我哦～".to_string(),
            args: json!({}),
        });
    }

    // 1.5) 免提双工开关（先判关：「取消免提」含「免提」；开启词更具体）。
    if contains_any(text, &["关闭免提", "关掉免提", "取消免提", "不用免提", "退出免提", "免提关"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::HandsFreeOff,
            reply: "好，免提关掉啦，之后按住说话就行～".to_string(),
            args: json!({}),
        });
    }
    if contains_any(text, &["开启免提", "打开免提", "免提模式", "免提聆听", "双工模式", "免提说话", "免提对话", "开始免提"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::HandsFreeOn,
            reply: "好，免提开启啦，我随时听你说话～".to_string(),
            args: json!({}),
        });
    }

    // 2) 守护触发。
    if contains_any(text, &["守护我", "开始守护", "守护模式", "守护屏幕", "帮我守着", "守着屏幕"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::GuardTrigger,
            reply: "收到，我帮你守着屏幕，注意休息呀～".to_string(),
            args: json!({}),
        });
    }

    // 2.5) 提醒设置 / 取消（先判取消：停用类词含「提醒」字样）。
    if contains_any(text, &["取消提醒", "关掉提醒", "关闭提醒", "停掉提醒", "不要提醒"]) {
        let kind = if contains_any(text, &["喝水", "饮水"]) {
            "water"
        } else if contains_any(text, &["久坐", "坐下"]) {
            "sit"
        } else if contains_any(text, &["护眼", "眼睛"]) {
            "eye"
        } else {
            "off"
        };
        let reply = match kind {
            "water" => "好，喝水提醒关掉啦，记得自己多喝水～".to_string(),
            "sit" => "好，久坐提醒关掉啦，也要记得起身活动哦～".to_string(),
            "eye" => "好，护眼提醒关掉啦，注意让眼睛休息～".to_string(),
            _ => "好，提醒都关掉啦～".to_string(),
        };
        return Some(UiCmdMatch {
            cmd: UiCmd::Remind(kind.to_string()),
            reply,
            args: json!({"kind": kind}),
        });
    }
    if text.contains("提醒") {
        let kind = if contains_any(text, &["喝水", "饮水"]) {
            Some("water")
        } else if contains_any(text, &["久坐", "坐太久", "起身", "活动一下"]) {
            Some("sit")
        } else if contains_any(text, &["护眼", "眼睛", "远眺"]) {
            Some("eye")
        } else {
            None
        };
        if let Some(k) = kind {
            let reply = match k {
                "water" => "好，我记着啦，到点提醒你喝水～".to_string(),
                "sit" => "好，我盯着呢，久坐到点提醒你起来活动～".to_string(),
                _ => "好，到点提醒你护眼休息～".to_string(),
            };
            return Some(UiCmdMatch {
                cmd: UiCmd::Remind(k.to_string()),
                reply,
                args: json!({"kind": k}),
            });
        }
    }

    // 2.7) 显式表情/动作演出（用户点名要表情/动作时直接演，不依赖 LLM 推断）。
    const EXPR_INTENTS: &[(&str, &[&str])] = &[
        ("happy", &["开心一点", "开心些", "高兴一点", "高兴些", "笑一个", "笑一笑", "笑一下", "开心起来"]),
        ("wave", &["挥挥手", "招招手", "打个招呼", "挥手", "挥一下"]),
        ("stretch", &["伸个懒腰", "伸展一下", "舒展一下", "拉伸一下", "伸懒腰"]),
        ("angry", &["生气一下", "不高兴", "哼一声", "生气给我看"]),
        ("surprised", &["惊讶一下", "吓一跳", "哇一下", "演个惊讶"]),
        ("sleepy", &["打个哈欠", "好困", "困一下", "演个困"]),
        ("shy", &["害羞一下", "不好意思一下", "演个害羞"]),
        ("sad", &["难过一下", "伤心一下", "演个难过"]),
        ("celebrate", &["做个动作", "表演一下", "秀一下", "动一动", "庆祝一下"]),
        ("thinking", &["思考一下", "想想看", "演个思考"]),
    ];
    for (expr, kws) in EXPR_INTENTS {
        if contains_any(text, kws) {
            return Some(UiCmdMatch {
                cmd: UiCmd::Express(expr.to_string()),
                reply: "来啦，表演给你看～".to_string(),
                args: json!({"expr": expr}),
            });
        }
    }

    // 3) 角色切换（头部动词 + 角色别名，两者都要命中，防「我要温柔对待别人」误伤）。
    let has_switch_head = contains_any(text, &["切换到", "切换成", "换到", "换成", "换上", "变到", "变成"]);
    if has_switch_head {
        for (id, keys) in PERSONA_MAP {
            if keys.iter().any(|k| text.contains(k)) {
                let names = ["古典佳人", "都市女性", "温柔形象", "活泼形象", "热情时尚"];
                let idx = PERSONA_MAP.iter().position(|(i, _)| i == id).unwrap_or(0);
                return Some(UiCmdMatch {
                    cmd: UiCmd::SetPersona(id.to_string()),
                    reply: format!("好呀，换上{}～", names[idx]),
                    args: json!({"persona": id}),
                });
            }
        }
    }

    // 4) 桌宠锁定 / 解锁。
    if text.contains("解锁") || (text.contains("取消") && text.contains("锁定")) {
        return Some(UiCmdMatch {
            cmd: UiCmd::Unlock,
            reply: "好啦，桌宠解锁了，你可以把它拖来拖去咯～".to_string(),
            args: json!({}),
        });
    }
    if text.contains("锁定") && !text.contains("解锁") {
        return Some(UiCmdMatch {
            cmd: UiCmd::Lock,
            reply: "已锁定桌宠位置，它不会乱跑啦～".to_string(),
            args: json!({}),
        });
    }

    // 4.5) 桌宠窗口形态切换（先于极简/完整匹配：形态词更具体，防「小窗」落极简）。
    //      compact=贴边小窗 / large=居中大窗 / min=最小化 / restore=恢复贴边小窗。
    //      含「吸附」的文本走 4.9 吸附开关（贴边吸附/靠边吸附），这里排除，避免形态/开关歧义。
    if contains_any(text, &["贴边", "靠边", "小窗", "小窗口", "缩小点", "变小点", "缩到角落", "靠边站"])
        && !text.contains("吸附")
    {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetMode("compact".to_string()),
            reply: "好，我贴到边边去啦，不挡你～".to_string(),
            args: json!({"mode": "compact"}),
        });
    }
    if contains_any(text, &["大窗", "大窗口", "放大窗口", "完整窗口", "放大点", "变大点"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetMode("large".to_string()),
            reply: "好，放大界面，陪你聊得舒服点～".to_string(),
            args: json!({"mode": "large"}),
        });
    }
    if contains_any(text, &["最小化", "缩到最小", "先退下"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetMode("min".to_string()),
            reply: "好，我最小化啦，有事叫我～".to_string(),
            args: json!({"mode": "min"}),
        });
    }
    if contains_any(text, &["恢复窗口", "还原窗口", "恢复大小", "还原大小", "回来陪我", "重新出来"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetMode("restore".to_string()),
            reply: "回来啦，继续陪你～".to_string(),
            args: json!({"mode": "restore"}),
        });
    }

    // 4.7) 窗口位置移动（向左/向右/向上/回到原位）。
    if contains_any(text, &["移到左边", "移到左面", "往左移", "挪到左边", "窗口靠左", "移到最左"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetWindow("left".to_string()),
            reply: "好，我挪到左边去啦～".to_string(),
            args: json!({"mode": "left"}),
        });
    }
    if contains_any(text, &["移到右边", "移到右面", "往右移", "挪到右边", "窗口靠右", "移到最右"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetWindow("right".to_string()),
            reply: "好，我挪到右边去啦～".to_string(),
            args: json!({"mode": "right"}),
        });
    }
    if contains_any(text, &["移到上面", "移到上边", "往上游", "挪到上面", "移到顶部", "窗口靠上"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetWindow("up".to_string()),
            reply: "好，我挪到上面去啦～".to_string(),
            args: json!({"mode": "up"}),
        });
    }
    if contains_any(text, &["回到原位", "回到原位置", "窗口复位", "位置复位", "回到原来位置"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::PetWindow("reset".to_string()),
            reply: "好，我回到贴边位置啦～".to_string(),
            args: json!({"mode": "reset"}),
        });
    }

    // 4.8) v24 UI 开关：记忆（记住/忘记对话）。
    if contains_any(text, &["记住我们的对话", "记住我们聊过", "记住我说的话", "记住对话", "别忘记我们聊过", "记住我们说的"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::RememberOn,
            reply: "好，我会记住我们聊过的话～".to_string(),
            args: json!({"on": true}),
        });
    }
    if contains_any(text, &["忘记我们的对话", "忘记我们聊过", "忘记对话", "别记了", "关掉记忆", "不要记住", "忘掉我们聊的"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::RememberOff,
            reply: "好，我不记啦，从这轮开始重新算～".to_string(),
            args: json!({"on": false}),
        });
    }
    // 4.9) v24 UI 开关：贴边吸附（「贴边」裸词已被 compact 形态规则占用，开关用「吸附」系触发词）。
    if contains_any(text, &["贴边吸附", "吸附到边缘", "靠边吸附", "吸附边缘", "打开吸附", "开启吸附", "吸附功能"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::SnapOn,
            reply: "好，贴边吸附开啦，拖我到边缘会自动贴住～".to_string(),
            args: json!({"on": true}),
        });
    }
    if contains_any(text, &["取消吸附", "关闭吸附", "吸附关掉", "关掉吸附", "不要吸附", "不吸附", "关闭吸附功能"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::SnapOff,
            reply: "好，贴边吸附关掉啦～".to_string(),
            args: json!({"on": false}),
        });
    }
    // 4.10) v24 UI 开关：窗口隐藏 / 显示。
    if contains_any(text, &["藏起来", "先躲起来", "隐身", "窗口藏起来", "躲一会", "藏一下", "藏一会", "收起来"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::HidePet,
            reply: "好，我先藏起来，需要的时候叫我～".to_string(),
            args: json!({}),
        });
    }
    if contains_any(text, &["出来吧", "现形", "显示出来", "回来吧", "窗口显示", "出来一下", "别藏了"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::ShowPet,
            reply: "我回来啦～".to_string(),
            args: json!({}),
        });
    }
    // 4.11) v24 UI 开关：联网搜索技能开关。
    if contains_any(text, &["关闭联网", "关掉联网", "不要联网", "关闭联网搜索", "关掉联网搜索", "关闭网络搜索", "别联网了"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::WebSearchOff,
            reply: "好，联网搜索关掉啦，只靠我自己的知识回答你～".to_string(),
            args: json!({"on": false}),
        });
    }
    if contains_any(text, &["打开联网", "开启联网", "联网搜索", "打开联网搜索", "开启联网搜索", "恢复联网"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::WebSearchOn,
            reply: "好，联网搜索开啦，遇到新消息我能上网查～".to_string(),
            args: json!({"on": true}),
        });
    }

    // 5) 极简模式 / 完整界面。
    if text.contains("极简") || text.starts_with("只留") || contains_any(text, &["只留角色", "只留人物"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::MinimalOn,
            reply: "好，切换极简模式，只留我陪着你～".to_string(),
            args: json!({"on": true}),
        });
    }
    if contains_any(text, &["完整界面", "完整模式", "显示菜单", "恢复界面", "退出极简"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::MinimalOff,
            reply: "完整界面回来啦，菜单都在咯～".to_string(),
            args: json!({"on": false}),
        });
    }

    // 5.5) 否决待确认操作。注意「取消静音」（MuteOff）/「取消锁定」（Unlock）已在前面
    // 更精确地命中，这里只接 file_ops/create_project 类操作的取消。
    if contains_any(text, &["拒绝", "不同意", "取消写入", "取消操作", "取消上一步"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::DenyLast,
            reply: "好的，请告诉我希望怎么调整，我再出方案～".to_string(),
            args: json!({}),
        });
    }

    // 6) 保险箱/密码箱「打开」→ 导航 vault（优先于通用「打开」，防落到 ConsoleOpen）。
    if (text.contains("保险箱") || text.contains("密码箱"))
        && contains_any(text, &["打开", "看看", "查看", "进入"])
    {
        return Some(UiCmdMatch {
            cmd: UiCmd::UiNav("vault".to_string()),
            reply: "好，带你打开保险箱～".to_string(),
            args: json!({"tab": "vault"}),
        });
    }

    // 6.5) 语音写代码：「写一个 hello.py 实现 xxx」→ FileOp write（generate_code + file_ops）。
    if let Some(m) = match_write_code(text) {
        return Some(m);
    }
    // 6.6) 语音改代码：「给 xx.py 追加 xxx」「修改 xx.py 把 xxx」→ FileOp edit。
    if let Some(m) = match_edit_code(text) {
        return Some(m);
    }
    // 6.7) 创建本地工程（create_project 技能）。
    if let Some(m) = match_create_project(text) {
        return Some(m);
    }
    // 6.8) 继续上次/指定工程（resume_project 技能）。
    if let Some(m) = match_resume_project(text) {
        return Some(m);
    }

    // 7) 文件操作（打开文件/列目录/创建/重命名/移动/复制/删除）。
    if let Some(m) = match_file_op(text) {
        return Some(m);
    }

    // 8) 控制台开关（「控制台」+ 动作词组合判定，兼容「把控制台关了」这类倒装）。
    if text.contains("控制台") && contains_any(text, &["关闭", "收起", "关掉", "隐藏", "关了"]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::ConsoleClose,
            reply: "好，控制台收起来啦，需要时叫我再打开～".to_string(),
            args: json!({}),
        });
    }
    if text.contains("控制台") && contains_any(text, &["打开", "调出", "显示"])
        || contains_any(text, &["打开面板", "调出面板"])
    {
        return Some(UiCmdMatch {
            cmd: UiCmd::ConsoleOpen,
            reply: "控制台已经打开啦".to_string(),
            args: json!({}),
        });
    }

    // 8.7) 打开常用目录（口语说法：「打开下载」「打开桌面文件夹」「看看我的文档」）。
    //      v45 修：此前只认"打开文件夹 xxx"，用户说"打开下载"就匹配不到、被当作不理解。
    if !text.replace("文件夹", "").contains("文件")
        && contains_any(text, &["打开", "进入", "看看", "看一下"])
    {
        for key in ["下载", "桌面", "文档", "文稿", "图片", "照片", "音乐", "视频", "主目录", "家目录"] {
            if text.contains(key) {
                return Some(file_op_match(
                    "open",
                    key.to_string(),
                    None,
                    None,
                    "好，帮你打开了～",
                ));
            }
        }
    }

    // 9) 控制台内页面导航（「打开 xx 页」；与 ConsoleOpen 不重叠：带 tab 词才算导航）。
    let nav_head = contains_any(text, &["打开", "调出", "进入", "切到", "切换到"]);
    if nav_head {
        for (tab, keys) in NAV_MAP {
            if keys.iter().any(|k| text.contains(k)) {
                return Some(UiCmdMatch {
                    cmd: UiCmd::UiNav(tab.to_string()),
                    reply: format!("好呀，带你去{tab}页看看～"),
                    args: json!({"tab": tab}),
                });
            }
        }
    }

    // 10) 搜索（捕获 query；须在导航之后，避免「打开聊天」被吞）。
    if let Some(m) = match_search(text) {
        return Some(m);
    }

    // 11) 打开网站/网页。
    if contains_any(text, &["打开网站", "打开网页", "打开网址", "访问网站", "访问网页", "访问网址", "打开浏览器"]) {
        let target = [
            "打开网站", "打开网页", "打开网址", "访问网站", "访问网页", "访问网址", "打开浏览器",
        ]
        .iter()
        .find_map(|kw| {
            let t = tail_after(text, kw);
            if t.is_empty() { None } else { Some(t) }
        })
        .unwrap_or_else(|| "default".to_string());
        return Some(UiCmdMatch {
            cmd: UiCmd::Launch(target.clone()),
            reply: format!("好，帮你打开{target}～"),
            args: json!({"target": target}),
        });
    }

    // 11.5) 批准待确认操作（file_ops/create_project 的 PendingApproval 收尾）。
    // 顺序要点：「开始守护」（GuardTrigger）、「取消静音」（MuteOff）等精确意图已在前面
    // 命中，「开始/开工/可以/没问题/确认」这类兜底词放在最后，不误伤既有意图。
    if contains_any(text, &[
        "确认写入", "确认写", "同意", "批准", "开工", "就这么干", "开始", "可以", "没问题", "确认", "确定",
    ]) {
        return Some(UiCmdMatch {
            cmd: UiCmd::ApproveLast,
            reply: "好，正在写入～".to_string(),
            args: json!({}),
        });
    }

    // 12) 退出应用（需二次确认；ExitConfirm 已在第 0 步截走）。
    if contains_any(text, &["退出", "关闭桌宠", "关闭元气小小", "关闭应用", "关掉桌宠", "关掉元气小小"])
        && !text.contains("保险箱")
    {
        return Some(UiCmdMatch {
            cmd: UiCmd::Exit,
            reply: "真的要退出吗？再说一次『确认退出』我就走哦～".to_string(),
            args: json!({}),
        });
    }

    None
}

/// LLM 智能调度：把 LLM 返回的 `ui_cmd` 名称+参数校验为白名单命令（防幻觉/防越权）。
/// 未命中白名单或参数非法 → None（忽略该字段，LLM 回复照常走聊天演出）。
/// 与 match_ui_command（语音权威路由）共享同一枚举与参数枚举校验。
pub fn ui_cmd_from_llm(name: &str, params: &serde_json::Value) -> Option<UiCmd> {
    match name {
        "hands_free_on" => Some(UiCmd::HandsFreeOn),
        "hands_free_off" => Some(UiCmd::HandsFreeOff),
        "remind" => {
            let k = params.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match k {
                "sit" | "water" | "eye" | "off" => Some(UiCmd::Remind(k.to_string())),
                _ => None,
            }
        }
        "express" => {
            const EXPRS: &[&str] = &[
                "happy", "angry", "surprised", "laugh", "thinking", "sad", "shy", "worried",
                "sleepy", "wave", "nod", "shake", "stretch", "celebrate", "touch", "point",
            ];
            let e = params.get("expr").and_then(|v| v.as_str()).unwrap_or("");
            if EXPRS.contains(&e) {
                Some(UiCmd::Express(e.to_string()))
            } else {
                None
            }
        }
        "pet_window" => {
            const MODES: &[&str] = &["left", "right", "up", "reset"];
            let m = params.get("mode").and_then(|v| v.as_str()).unwrap_or("");
            if MODES.contains(&m) {
                Some(UiCmd::PetWindow(m.to_string()))
            } else {
                None
            }
        }
        "pet_mode" => {
            const MODES: &[&str] = &["compact", "large", "min", "restore"];
            let m = params.get("mode").and_then(|v| v.as_str()).unwrap_or("");
            if MODES.contains(&m) {
                Some(UiCmd::PetMode(m.to_string()))
            } else {
                None
            }
        }
        "mute_on" => Some(UiCmd::MuteOn),
        "mute_off" => Some(UiCmd::MuteOff),
        "console_open" => Some(UiCmd::ConsoleOpen),
        "console_close" => Some(UiCmd::ConsoleClose),
        "guard" => Some(UiCmd::GuardTrigger),
        "lock" => Some(UiCmd::Lock),
        "unlock" => Some(UiCmd::Unlock),
        "minimal_on" => Some(UiCmd::MinimalOn),
        "minimal_off" => Some(UiCmd::MinimalOff),
        "set_persona" => {
            let id = params.get("persona").and_then(|v| v.as_str()).unwrap_or("");
            if PERSONA_MAP.iter().any(|(pid, _)| *pid == id) {
                Some(UiCmd::SetPersona(id.to_string()))
            } else {
                None
            }
        }
        "nav" => {
            let tab = params.get("tab").and_then(|v| v.as_str()).unwrap_or("");
            if NAV_MAP.iter().any(|(t, _)| *t == tab) {
                Some(UiCmd::UiNav(tab.to_string()))
            } else {
                None
            }
        }
        "search" => {
            let q = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            if !q.is_empty() {
                Some(UiCmd::Search(q.to_string()))
            } else {
                None
            }
        }
        // v24 UI 开关白名单：记忆/贴边/窗口显隐/联网开关。
        "remember_on" => Some(UiCmd::RememberOn),
        "remember_off" => Some(UiCmd::RememberOff),
        "snap_on" => Some(UiCmd::SnapOn),
        "snap_off" => Some(UiCmd::SnapOff),
        "pet_hide" => Some(UiCmd::HidePet),
        "pet_show" => Some(UiCmd::ShowPet),
        "web_search_on" => Some(UiCmd::WebSearchOn),
        "web_search_off" => Some(UiCmd::WebSearchOff),
        _ => None,
    }
}

fn contains_any(text: &str, kws: &[&str]) -> bool {
    kws.iter().any(|k| text.contains(k))
}

/// 搜索意图：关键词 + 捕获 query。query 提取失败（如只说「搜一下」）时仍命中但 query 为空。
fn match_search(text: &str) -> Option<UiCmdMatch> {
    let kws = ["搜一下", "搜索", "帮我搜", "查一下", "帮我查", "百度", "搜搜", "搜搜看"];
    for kw in kws {
        if text.contains(kw) {
            let mut q = tail_after(text, kw);
            for w in ["关于", "有关", "一下"] {
                q = q.strip_prefix(w).unwrap_or(&q).to_string();
            }
            let reply = if q.is_empty() {
                "想查点什么？把要搜的词告诉我，我帮你查～".to_string()
            } else {
                format!("好，我帮你搜搜「{q}」～")
            };
            return Some(UiCmdMatch {
                cmd: UiCmd::Search(q.clone()),
                reply,
                args: json!({"query": q}),
            });
        }
    }
    None
}

/// 文件操作意图：动作关键词 → (action, path)。路径为关键词后的尾巴，尽量剥填充词。
fn match_file_op(text: &str) -> Option<UiCmdMatch> {
    // 删除（独立处理：删除最危险，关键词要窄）。
    for kw in ["删除文件", "删除这个文件", "删掉文件", "删除"] {
        if text.contains(kw) {
            let path = tail_after(text, kw);
            return Some(file_op_match("delete", path, None, None, "好，我帮你删掉它，需要的话请在控制台确认授权哦～"));
        }
    }
    // 移动：「把 a 移动到 b」/「移动 a 到 b」。
    if text.contains("移动") {
        let rest = tail_after(text, "移动");
        let (path, dest) = match rest.split_once("到") {
            Some((p, d)) => (strip_filler(p), strip_filler(d)),
            None => (rest, String::new()),
        };
        return Some(file_op_match("move", path, Some(dest), None, "好，帮你移动文件～"));
    }
    // 复制：「复制 a 到 b」。
    if text.contains("复制") {
        let rest = tail_after(text, "复制");
        let (path, dest) = match rest.split_once("到") {
            Some((p, d)) => (strip_filler(p), strip_filler(d)),
            None => (rest, String::new()),
        };
        return Some(file_op_match("copy", path, Some(dest), None, "好，帮你复制文件～"));
    }
    // 重命名：「把 a 重命名为 b」。
    if text.contains("重命名") {
        let rest = tail_after(text, "重命名");
        let (path, dest) = match rest.split_once("为") {
            Some((p, d)) => (strip_filler(p), strip_filler(d)),
            None => match rest.split_once("成") {
                Some((p, d)) => (strip_filler(p), strip_filler(d)),
                None => (rest, String::new()),
            },
        };
        return Some(file_op_match("rename", path, Some(dest), None, "好，帮你改名字～"));
    }
    // 创建 / 新建文件。
    for kw in ["创建文件", "新建文件", "新建一个文件", "创建一个文件"] {
        if text.contains(kw) {
            let path = tail_after(text, kw);
            return Some(file_op_match("create", path, None, None, "好，帮你创建文件～"));
        }
    }
    // 列出 / 看看目录。
    for kw in ["列出目录", "看看目录", "查看目录", "列出文件夹", "看看文件夹", "打开文件夹", "打开目录", "打开文件"] {
        if text.contains(kw) {
            let path = tail_after(text, kw);
            let action = if kw.starts_with("打开") { "open" } else { "list" };
            return Some(file_op_match(action, path, None, None, "好，我看看目录里有什么～"));
        }
    }
    None
}

fn file_op_match(
    action: &str,
    path: String,
    dest: Option<String>,
    content: Option<String>,
    reply: &str,
) -> UiCmdMatch {
    UiCmdMatch {
        cmd: UiCmd::FileOp {
            action: action.to_string(),
            path: path.clone(),
            dest,
            content,
        },
        reply: reply.to_string(),
        args: json!({"action": action, "path": path}),
    }
}

/// 语音指令速览（前端 / README 展示用）。
pub fn list_command_hints() -> Vec<&'static str> {
    vec![
        "打开控制台 / 收起控制台",
        "打开聊天页 / 角色页 / 设置页 / 保险箱",
        "守护我 / 开始守护",
        "切换到温柔形象 / 换成活泼的",
        "锁定桌宠 / 解锁",
        "静音 / 打开声音",
        "极简模式 / 完整界面",
        "搜一下 今天天气 / 帮我查 北京房价",
        "写一个 hello.py 实现斐波那契（自动生成代码并写入）",
        "在下载文件夹创建 python 天气助手项目 / 继续天气助手项目",
        "打开网站 bilibili",
        "退出（再说「确认退出」真退）",
    ]
}

/// 从文本里找代码文件名（xxx.py/.js/.rs/.sh/.html，可带目录前缀）。
/// 返回 (文件名, char 起点, char 终点)。
fn find_code_filename(text: &str) -> Option<(String, usize, usize)> {
    const EXTS: [&str; 5] = ["py", "js", "rs", "sh", "html"];
    let v: Vec<char> = text.chars().collect();
    for i in 1..v.len() {
        if v[i] != '.' {
            continue;
        }
        let rest: String = v[i + 1..].iter().take(8).collect();
        let ext: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !EXTS.contains(&ext.as_str()) {
            continue;
        }
        // 回溯文件名起点：允许字母数字 / _ - . / / ~（不含中文，防把「写一个」卷进去）。
        let mut start = i;
        while start > 0 {
            let c = v[start - 1];
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '~' {
                start -= 1;
            } else {
                break;
            }
        }
        let end = i + 1 + ext.len();
        return Some((v[start..end].iter().collect(), start, end));
    }
    None
}

/// 「写一个 hello.py 实现 xxx」→ FileOp write（spec = 需求文本）。
fn match_write_code(text: &str) -> Option<UiCmdMatch> {
    if !contains_any(text, &["写一个", "写个", "写段", "写份", "创建文件", "新建文件"]) {
        return None;
    }
    let (path, s, e) = find_code_filename(text)?;
    let mut t = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if !(idx >= s && idx < e) {
            t.push(ch);
        }
    }
    for kw in ["写一个", "写个", "写段", "写份", "帮我", "一下"] {
        t = t.replace(kw, "");
    }
    let spec = strip_filler(&t);
    Some(UiCmdMatch {
        cmd: UiCmd::FileOp {
            action: "write".to_string(),
            path: path.clone(),
            dest: None,
            content: Some(spec),
        },
        reply: format!("好的，我来写 {path}～"),
        args: json!({"action": "write", "path": path}),
    })
}

/// 「给 xx.py 追加 xxx」（append）/「修改 xx.py 把 xxx」（replace）→ FileOp edit。
fn match_edit_code(text: &str) -> Option<UiCmdMatch> {
    // append：给 + 追加/加上/添加 + 文件名
    if text.contains('给') && contains_any(text, &["追加", "加上", "添加"]) {
        let (path, _, _) = find_code_filename(text)?;
        let kw = ["追加", "加上", "添加"].iter().find(|k| text.contains(**k))?;
        let content = tail_after(text, kw);
        return Some(UiCmdMatch {
            cmd: UiCmd::FileOp {
                action: "edit".to_string(),
                path,
                dest: Some("append".to_string()),
                content: Some(content),
            },
            reply: "好，我来补充内容～".to_string(),
            args: json!({"action": "edit", "mode": "append"}),
        });
    }
    // replace：修改/改一下 + 文件名 + 要求
    if contains_any(text, &["修改", "改一下"]) {
        let (path, _, _) = find_code_filename(text)?;
        let kw = ["改一下", "修改"].iter().find(|k| text.contains(**k))?;
        let mut spec = tail_after(text, kw);
        spec = spec.replace(&path, "");
        let spec = strip_filler(&spec);
        return Some(UiCmdMatch {
            cmd: UiCmd::FileOp {
                action: "edit".to_string(),
                path,
                dest: Some("replace".to_string()),
                content: Some(spec),
            },
            reply: "好，我来改～".to_string(),
            args: json!({"action": "edit", "mode": "replace"}),
        });
    }
    None
}

/// 「在下载文件夹创建 python 工程天气助手实现天气预报」→ CreateProject。
fn match_create_project(text: &str) -> Option<UiCmdMatch> {
    if !contains_any(text, &["创建", "新建", "做一个", "开发"]) {
        return None;
    }
    // 必须是工程语境：有「项目/工程」或明确技术栈词。
    if !contains_any(text, &["项目", "工程"])
        && !contains_any(text, &["python", "node", "rust", "web", "网页"])
    {
        return None;
    }
    let mut t = text.to_string();
    // 父目录：在（下载|桌面|文档|图片|音乐|视频|主目录|home）→ ~/xxx；缺省 ~/Downloads。
    let mut parent = "~/Downloads".to_string();
    for (kw, dir) in [
        ("下载", "~/Downloads"), ("桌面", "~/Desktop"), ("文档", "~/Documents"),
        ("图片", "~/Pictures"), ("音乐", "~/Music"), ("视频", "~/Videos"),
        ("主目录", "~"), ("home", "~"),
    ] {
        if t.contains(kw) {
            parent = dir.to_string();
            t = t.replace(kw, "");
            break;
        }
    }
    for v in ["文件夹", "目录", "在"] {
        t = t.replace(v, " ");
    }
    // 统一小写：中文不受影响，但 "Python"→"python" 才能被下方 replace 命中删除。
    t = t.to_lowercase();
    // 技术栈：缺省 web。
    let mut lang = "web".to_string();
    let low = t.clone();
    for (kw, l) in [
        ("python3", "python"), ("python", "python"), ("py", "python"),
        ("nodejs", "node"), ("node", "node"), ("rust", "rust"), ("cargo", "rust"),
        ("网页", "web"), ("web", "web"), ("html", "web"), ("通用", "web"),
    ] {
        if low.contains(kw) {
            lang = l.to_string();
            t = t.replace(kw, "");
            break;
        }
    }
    for v in ["项目", "工程"] {
        t = t.replace(v, " ");
    }
    for v in ["创建", "新建", "做一个", "开发", "帮我", "一个"] {
        t = t.replace(v, " ");
    }
    let trimmed = t.trim();
    // 目标（goal）从「实现/功能/用来」切开：前面是 name，后面整段是 goal。
    let (head, goal) = ["实现", "功能", "用来"]
        .iter()
        .find_map(|m| trimmed.find(m).map(|pos| (&trimmed[..pos], trimmed[pos..].to_string())))
        .unwrap_or((trimmed, String::new()));
    let name: String = head
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(30)
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(UiCmdMatch {
        cmd: UiCmd::CreateProject {
            name: name.clone(),
            parent_dir: parent,
            lang,
            goal,
        },
        reply: "好的，我先出个方案给你确认～".to_string(),
        args: json!({"name": name}),
    })
}

/// 「继续天气助手项目」/「继续上次任务」→ ResumeProject（name 空 = 最近任务）。
fn match_resume_project(text: &str) -> Option<UiCmdMatch> {
    if !contains_any(text, &["继续", "接着"]) {
        return None;
    }
    if !contains_any(text, &["项目", "工程", "任务"]) {
        return None;
    }
    let kw = if text.contains("继续") { "继续" } else { "接着" };
    let mut rest = tail_after(text, kw);
    rest = rest.replace("上次", "").replace("任务", "").replace("项目", "").replace("工程", "");
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(30)
        .collect();
    Some(UiCmdMatch {
        cmd: UiCmd::ResumeProject {
            name,
            parent_dir: "~/Downloads".to_string(),
        },
        reply: "好的，我看看这个工程的进度～".to_string(),
        args: json!({}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_of(text: &str) -> Option<UiCmd> {
        match_ui_command(text).map(|m| m.cmd)
    }

    #[test]
    fn console_open_close() {
        assert_eq!(cmd_of("打开控制台"), Some(UiCmd::ConsoleOpen));
        assert_eq!(cmd_of("帮我调出控制台"), Some(UiCmd::ConsoleOpen));
        assert_eq!(cmd_of("收起控制台"), Some(UiCmd::ConsoleClose));
        assert_eq!(cmd_of("把控制台关了"), Some(UiCmd::ConsoleClose));
    }

    #[test]
    fn nav_tabs() {
        assert_eq!(cmd_of("打开聊天页"), Some(UiCmd::UiNav("chat".into())));
        assert_eq!(cmd_of("打开对话"), Some(UiCmd::UiNav("chat".into())));
        assert_eq!(cmd_of("打开技能页"), Some(UiCmd::UiNav("plugins".into())));
        assert_eq!(cmd_of("打开角色页"), Some(UiCmd::UiNav("roles".into())));
        assert_eq!(cmd_of("打开设置"), Some(UiCmd::UiNav("settings".into())));
        assert_eq!(cmd_of("打开社交"), Some(UiCmd::UiNav("social".into())));
        assert_eq!(cmd_of("打开诊断"), Some(UiCmd::UiNav("events".into())));
        assert_eq!(cmd_of("打开猜拳"), Some(UiCmd::UiNav("pk".into())));
        assert_eq!(cmd_of("打开代码检查"), Some(UiCmd::UiNav("codecheck".into())));
    }

    #[test]
    fn vault_nav_beats_generic_open() {
        assert_eq!(cmd_of("打开保险箱"), Some(UiCmd::UiNav("vault".into())));
        assert_eq!(cmd_of("看看密码箱"), Some(UiCmd::UiNav("vault".into())));
    }

    #[test]
    fn guard_trigger() {
        let m = match_ui_command("守护我").unwrap();
        assert_eq!(m.cmd, UiCmd::GuardTrigger);
        assert!(m.reply.contains("守着屏幕"));
        assert_eq!(cmd_of("开始守护模式"), Some(UiCmd::GuardTrigger));
    }

    #[test]
    fn persona_switch() {
        assert_eq!(cmd_of("切换到温柔形象"), Some(UiCmd::SetPersona("wenrou".into())));
        assert_eq!(cmd_of("换成活泼的"), Some(UiCmd::SetPersona("huopo".into())));
        assert_eq!(cmd_of("换到古典佳人"), Some(UiCmd::SetPersona("gudian".into())));
        assert_eq!(cmd_of("切换到dushi"), Some(UiCmd::SetPersona("dushi".into())));
        assert_eq!(cmd_of("变成热情时尚"), Some(UiCmd::SetPersona("reqing".into())));
        // 无头动词的闲聊不命中（防误伤：不是所有含「温柔」的话都要换形象）。
        assert!(cmd_of("我很喜欢你的温柔").is_none());
    }

    #[test]
    fn lock_unlock() {
        assert_eq!(cmd_of("锁定"), Some(UiCmd::Lock));
        assert_eq!(cmd_of("锁定桌宠位置"), Some(UiCmd::Lock));
        assert_eq!(cmd_of("解锁"), Some(UiCmd::Unlock));
        assert_eq!(cmd_of("取消锁定"), Some(UiCmd::Unlock));
    }

    #[test]
    fn mute_order_before_console() {
        assert_eq!(cmd_of("静音"), Some(UiCmd::MuteOn));
        assert_eq!(cmd_of("别说话"), Some(UiCmd::MuteOn));
        assert_eq!(cmd_of("关闭声音"), Some(UiCmd::MuteOn));
        // 关键防误伤：打开声音 必须是 MuteOff，绝不能落到 ConsoleOpen/UiNav。
        assert_eq!(cmd_of("打开声音"), Some(UiCmd::MuteOff));
        assert_eq!(cmd_of("取消静音"), Some(UiCmd::MuteOff));
    }

    #[test]
    fn hands_free_switch() {
        assert_eq!(cmd_of("开启免提"), Some(UiCmd::HandsFreeOn));
        assert_eq!(cmd_of("打开免提聆听"), Some(UiCmd::HandsFreeOn));
        assert_eq!(cmd_of("双工模式"), Some(UiCmd::HandsFreeOn));
        assert_eq!(cmd_of("免提说话"), Some(UiCmd::HandsFreeOn));
        assert_eq!(cmd_of("关闭免提"), Some(UiCmd::HandsFreeOff));
        assert_eq!(cmd_of("取消免提"), Some(UiCmd::HandsFreeOff));
        assert_eq!(cmd_of("不用免提了"), Some(UiCmd::HandsFreeOff));
    }

    #[test]
    fn remind_set_cancel() {
        assert_eq!(cmd_of("提醒我喝水"), Some(UiCmd::Remind("water".into())));
        assert_eq!(cmd_of("帮我设置喝水提醒"), Some(UiCmd::Remind("water".into())));
        assert_eq!(cmd_of("提醒我久坐"), Some(UiCmd::Remind("sit".into())));
        assert_eq!(cmd_of("设置护眼提醒"), Some(UiCmd::Remind("eye".into())));
        assert_eq!(cmd_of("取消喝水提醒"), Some(UiCmd::Remind("water".into())));
        assert_eq!(cmd_of("关掉提醒"), Some(UiCmd::Remind("off".into())));
        // 不含具体提醒类型的「提醒」不误伤（如「谢谢你提醒我」）。
        assert!(cmd_of("谢谢你提醒我").is_none());
    }

    #[test]
    fn express_emotion_action() {
        assert_eq!(cmd_of("开心一点"), Some(UiCmd::Express("happy".into())));
        assert_eq!(cmd_of("笑一个"), Some(UiCmd::Express("happy".into())));
        assert_eq!(cmd_of("挥挥手"), Some(UiCmd::Express("wave".into())));
        assert_eq!(cmd_of("伸个懒腰"), Some(UiCmd::Express("stretch".into())));
        assert_eq!(cmd_of("做个动作"), Some(UiCmd::Express("celebrate".into())));
        // 闲聊含「高兴」无动作动词不命中（防「我今天很高兴」误触发演出）。
        assert!(cmd_of("我今天很高兴").is_none());
    }

    #[test]
    fn pet_window_move() {
        assert_eq!(cmd_of("移到左边"), Some(UiCmd::PetWindow("left".into())));
        assert_eq!(cmd_of("窗口靠右"), Some(UiCmd::PetWindow("right".into())));
        assert_eq!(cmd_of("挪到上面"), Some(UiCmd::PetWindow("up".into())));
        assert_eq!(cmd_of("窗口复位"), Some(UiCmd::PetWindow("reset".into())));
    }

    #[test]
    fn v24_ui_switches() {
        // 记忆
        assert_eq!(cmd_of("记住我们的对话"), Some(UiCmd::RememberOn));
        assert_eq!(cmd_of("记住我们聊过"), Some(UiCmd::RememberOn));
        assert_eq!(cmd_of("忘记对话"), Some(UiCmd::RememberOff));
        assert_eq!(cmd_of("别记了"), Some(UiCmd::RememberOff));
        // 贴边（「贴边」裸词归 compact 形态规则，吸附开关用「吸附」系）
        assert_eq!(cmd_of("贴边吸附"), Some(UiCmd::SnapOn));
        assert_eq!(cmd_of("吸附到边缘"), Some(UiCmd::SnapOn));
        assert_eq!(cmd_of("取消吸附"), Some(UiCmd::SnapOff));
        assert_eq!(cmd_of("关闭吸附"), Some(UiCmd::SnapOff));
        // 窗口显隐
        assert_eq!(cmd_of("藏起来"), Some(UiCmd::HidePet));
        assert_eq!(cmd_of("先躲起来"), Some(UiCmd::HidePet));
        assert_eq!(cmd_of("出来吧"), Some(UiCmd::ShowPet));
        assert_eq!(cmd_of("别藏了"), Some(UiCmd::ShowPet));
        // 联网开关
        assert_eq!(cmd_of("关闭联网"), Some(UiCmd::WebSearchOff));
        assert_eq!(cmd_of("关掉联网"), Some(UiCmd::WebSearchOff));
        assert_eq!(cmd_of("打开联网"), Some(UiCmd::WebSearchOn));
        assert_eq!(cmd_of("联网搜索"), Some(UiCmd::WebSearchOn));
        // 防误伤：随意的“记得”不命中开关（无明确功能词）
        assert_eq!(cmd_of("我记得你上次说的"), None);
        assert_eq!(cmd_of("别躲着我说话"), None);
    }

    #[test]
    fn v24_llm_ui_switch_whitelist() {
        use serde_json::json;
        let p = json!({});
        assert_eq!(ui_cmd_from_llm("remember_on", &p), Some(UiCmd::RememberOn));
        assert_eq!(ui_cmd_from_llm("remember_off", &p), Some(UiCmd::RememberOff));
        assert_eq!(ui_cmd_from_llm("snap_on", &p), Some(UiCmd::SnapOn));
        assert_eq!(ui_cmd_from_llm("snap_off", &p), Some(UiCmd::SnapOff));
        assert_eq!(ui_cmd_from_llm("pet_hide", &p), Some(UiCmd::HidePet));
        assert_eq!(ui_cmd_from_llm("pet_show", &p), Some(UiCmd::ShowPet));
        assert_eq!(ui_cmd_from_llm("web_search_on", &p), Some(UiCmd::WebSearchOn));
        assert_eq!(ui_cmd_from_llm("web_search_off", &p), Some(UiCmd::WebSearchOff));
        // 未知命令仍被拒
        assert_eq!(ui_cmd_from_llm("hide", &p), None);
        assert_eq!(ui_cmd_from_llm("remember", &p), None);
    }

    #[test]
    fn llm_whitelist() {
        use serde_json::json;
        let p = json!({});
        assert_eq!(ui_cmd_from_llm("hands_free_on", &p), Some(UiCmd::HandsFreeOn));
        assert_eq!(ui_cmd_from_llm("mute_off", &p), Some(UiCmd::MuteOff));
        assert_eq!(ui_cmd_from_llm("console_open", &p), Some(UiCmd::ConsoleOpen));
        // 参数校验：白名单外/非法参数 → None
        assert_eq!(ui_cmd_from_llm("remind", &json!({"kind": "water"})), Some(UiCmd::Remind("water".into())));
        assert_eq!(ui_cmd_from_llm("remind", &json!({"kind": "bed"})), None);
        assert_eq!(ui_cmd_from_llm("express", &json!({"expr": "wave"})), Some(UiCmd::Express("wave".into())));
        assert_eq!(ui_cmd_from_llm("express", &json!({"expr": "fly"})), None);
        assert_eq!(ui_cmd_from_llm("pet_window", &json!({"mode": "left"})), Some(UiCmd::PetWindow("left".into())));
        assert_eq!(ui_cmd_from_llm("pet_window", &json!({"mode": "diag"})), None);
        assert_eq!(ui_cmd_from_llm("set_persona", &json!({"persona": "gudian"})), Some(UiCmd::SetPersona("gudian".into())));
        assert_eq!(ui_cmd_from_llm("set_persona", &json!({"persona": "xxx"})), None);
        assert_eq!(ui_cmd_from_llm("nav", &json!({"tab": "chat"})), Some(UiCmd::UiNav("chat".into())));
        assert_eq!(ui_cmd_from_llm("nav", &json!({"tab": "root"})), None);
        // 完全未知命令 → None
        assert_eq!(ui_cmd_from_llm("rm -rf", &p), None);
        assert_eq!(ui_cmd_from_llm("shell_exec", &json!({"cmd": "ls"})), None);
    }

    #[test]
    fn minimal_mode() {
        assert_eq!(cmd_of("极简模式"), Some(UiCmd::MinimalOn));
        assert_eq!(cmd_of("只留角色"), Some(UiCmd::MinimalOn));
        assert_eq!(cmd_of("完整界面"), Some(UiCmd::MinimalOff));
        assert_eq!(cmd_of("显示菜单"), Some(UiCmd::MinimalOff));
    }

    #[test]
    fn pet_mode_switch() {
        assert_eq!(cmd_of("贴边"), Some(UiCmd::PetMode("compact".into())));
        assert_eq!(cmd_of("靠边站"), Some(UiCmd::PetMode("compact".into())));
        assert_eq!(cmd_of("小窗"), Some(UiCmd::PetMode("compact".into())));
        assert_eq!(cmd_of("大窗"), Some(UiCmd::PetMode("large".into())));
        assert_eq!(cmd_of("放大窗口"), Some(UiCmd::PetMode("large".into())));
        assert_eq!(cmd_of("最小化"), Some(UiCmd::PetMode("min".into())));
        // v24：收起来/藏起来 归隐藏开关（HidePet），最小化只留形态词。
        assert_eq!(cmd_of("收起来"), Some(UiCmd::HidePet));
        assert_eq!(cmd_of("藏起来"), Some(UiCmd::HidePet));
        assert_eq!(cmd_of("恢复窗口"), Some(UiCmd::PetMode("restore".into())));
        assert_eq!(cmd_of("回来陪我"), Some(UiCmd::PetMode("restore".into())));
        // 防误伤：极简/完整界面 仍是原语义，不被形态词截走。
        assert_eq!(cmd_of("极简模式"), Some(UiCmd::MinimalOn));
        assert_eq!(cmd_of("完整界面"), Some(UiCmd::MinimalOff));
    }

    #[test]
    fn search_captures_query() {
        let m = match_ui_command("帮我搜一下今天天气怎么样").unwrap();
        match m.cmd {
            UiCmd::Search(q) => assert!(q.contains("今天天气"), "query={q}"),
            other => panic!("expected Search, got {other:?}"),
        }
        assert_eq!(m.args["query"], serde_json::json!("今天天气怎么样"));
        let m2 = match_ui_command("查一下北京房价").unwrap();
        match m2.cmd {
            UiCmd::Search(q) => assert_eq!(q, "北京房价"),
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn file_op_actions() {
        let m = match_ui_command("删除文件 ~/a.txt").unwrap();
        assert!(matches!(m.cmd, UiCmd::FileOp { ref action, .. } if action == "delete"));
        let m = match_ui_command("列出目录 ~/docs").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, path, .. } => {
                assert_eq!(action, "list");
                assert!(path.contains("docs"), "path={path}");
            }
            _ => panic!(),
        }
        let m = match_ui_command("创建文件 ~/note.txt").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, .. } => assert_eq!(action, "create"),
            _ => panic!(),
        }
        let m = match_ui_command("打开文件夹 ~/work").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, .. } => assert_eq!(action, "open"),
            _ => panic!(),
        }
        // v45：口语说法不写"文件夹"也要能打开
        for say in ["打开下载", "打开下载文件夹", "打开我的桌面", "看看文档"] {
            let m = match_ui_command(say).unwrap_or_else(|| panic!("{say} 未匹配"));
            match m.cmd {
                UiCmd::FileOp { ref action, ref path, .. } => {
                    assert_eq!(action, "open", "{say} 应走 open");
                    assert!(!path.is_empty(), "{say} 应带出目录名");
                }
                _ => panic!("{say} 应匹配 FileOp"),
            }
        }
        // 移动捕获 dest
        let m = match_ui_command("移动 a.txt 到 b.txt").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, dest, .. } => {
                assert_eq!(action, "move");
                assert_eq!(dest, Some("b.txt".to_string()));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn launch_url() {
        let m = match_ui_command("打开网站 bilibili").unwrap();
        match m.cmd {
            UiCmd::Launch(t) => assert!(t.contains("bilibili"), "t={t}"),
            _ => panic!(),
        }
        let got = cmd_of("访问网页 github").map(|c| match c {
            UiCmd::Launch(t) => t,
            other => panic!("expected Launch, got {other:?}"),
        });
        assert_eq!(got, Some("github".to_string()));
    }

    #[test]
    fn exit_needs_confirmation_first() {
        let m = match_ui_command("退出").unwrap();
        assert_eq!(m.cmd, UiCmd::Exit);
        assert!(m.reply.contains("确认退出"));
        let m2 = match_ui_command("关闭元气小小").unwrap();
        assert_eq!(m2.cmd, UiCmd::Exit);
        // 再说一次 → ExitConfirm（真退）
        let m3 = match_ui_command("确认退出").unwrap();
        assert_eq!(m3.cmd, UiCmd::ExitConfirm);
        let m4 = match_ui_command("是的退出").unwrap();
        assert_eq!(m4.cmd, UiCmd::ExitConfirm);
    }

    #[test]
    fn chitchat_falls_through_to_llm() {
        assert!(match_ui_command("你好呀").is_none());
        assert!(match_ui_command("今天过得怎么样").is_none());
        assert!(match_ui_command("给我讲个笑话").is_none());
        assert!(match_ui_command("").is_none());
    }

    #[test]
    fn hints_nonempty() {
        let h = list_command_hints();
        assert!(h.len() >= 8);
    }

    #[test]
    fn anti_mistake_ordering() {
        // 「打开声音」不允许落到 ConsoleOpen / UiNav settings。
        assert_eq!(cmd_of("打开声音"), Some(UiCmd::MuteOff));
        // 「取消静音」不允许被「静音」先吞成 MuteOn。
        assert_eq!(cmd_of("取消静音"), Some(UiCmd::MuteOff));
        // 「打开文件 xx」是文件操作，不是导航/控制台。
        match cmd_of("打开文件 ~/a.txt") {
            Some(UiCmd::FileOp { action, .. }) => assert_eq!(action, "open"),
            other => panic!("expected FileOp open, got {other:?}"),
        }
        // 「关闭控制台」不能落到通用 Exit。
        assert_eq!(cmd_of("关闭控制台"), Some(UiCmd::ConsoleClose));
        // 闲聊不命中。
        assert!(cmd_of("你觉得今天心情怎么样").is_none());
    }

    // ---- 追加批：语音工作流（写代码 / 建工程 / 续工程 / 确认批准） ----

    #[test]
    fn write_code_captures_path_and_spec() {
        let m = match_ui_command("写一个 hello.py 实现斐波那契数列").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, path, dest, content } => {
                assert_eq!(action, "write");
                assert_eq!(path, "hello.py");
                assert_eq!(dest, None);
                let spec = content.unwrap();
                assert!(spec.contains("斐波那契"), "spec={spec}");
            }
            other => panic!("expected FileOp write, got {other:?}"),
        }
        assert!(m.reply.contains("hello.py"));
    }

    #[test]
    fn write_without_code_ext_falls_to_create() {
        // 无代码扩展名 → 不命中 write，回落既有 create。
        match cmd_of("创建文件 ~/note.txt") {
            Some(UiCmd::FileOp { action, .. }) => assert_eq!(action, "create"),
            other => panic!("expected create, got {other:?}"),
        }
    }

    #[test]
    fn edit_append_intent() {
        let m = match_ui_command("给 app.py 追加一行调试日志").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, path, dest, content } => {
                assert_eq!(action, "edit");
                assert_eq!(path, "app.py");
                assert_eq!(dest, Some("append".to_string()));
                assert!(content.unwrap().contains("调试日志"));
            }
            other => panic!("expected edit append, got {other:?}"),
        }
    }

    #[test]
    fn edit_replace_intent() {
        let m = match_ui_command("修改 main.py 把日志输出到文件").unwrap();
        match m.cmd {
            UiCmd::FileOp { action, path, dest, .. } => {
                assert_eq!(action, "edit");
                assert_eq!(path, "main.py");
                assert_eq!(dest, Some("replace".to_string()));
            }
            other => panic!("expected edit replace, got {other:?}"),
        }
    }

    #[test]
    fn create_project_full_capture() {
        let m = match_ui_command("在下载文件夹创建 Python 工程天气助手实现天气预报").unwrap();
        match m.cmd {
            UiCmd::CreateProject { name, parent_dir, lang, goal } => {
                assert_eq!(name, "天气助手");
                assert_eq!(parent_dir, "~/Downloads");
                assert_eq!(lang, "python");
                assert!(goal.contains("天气预报"), "goal={goal}");
            }
            other => panic!("expected CreateProject, got {other:?}"),
        }
        assert!(m.reply.contains("方案"));
    }

    #[test]
    fn create_project_default_dir_and_lang() {
        let m = match_ui_command("创建 天气助手 项目").unwrap();
        match m.cmd {
            UiCmd::CreateProject { name, parent_dir, lang, goal } => {
                assert_eq!(name, "天气助手");
                assert_eq!(parent_dir, "~/Downloads");
                assert_eq!(lang, "web");
                assert_eq!(goal, "");
            }
            other => panic!("expected CreateProject, got {other:?}"),
        }
    }

    #[test]
    fn create_project_rust_and_desktop() {
        let m = match_ui_command("在桌面目录新建 rust 项目博客").unwrap();
        match m.cmd {
            UiCmd::CreateProject { name, parent_dir, lang, .. } => {
                assert_eq!(parent_dir, "~/Desktop");
                assert_eq!(lang, "rust");
                assert_eq!(name, "博客");
            }
            other => panic!("expected CreateProject, got {other:?}"),
        }
    }

    #[test]
    fn resume_project_named() {
        let m = match_ui_command("继续天气助手项目").unwrap();
        match m.cmd {
            UiCmd::ResumeProject { name, parent_dir } => {
                assert_eq!(name, "天气助手");
                assert_eq!(parent_dir, "~/Downloads");
            }
            other => panic!("expected ResumeProject, got {other:?}"),
        }
    }

    #[test]
    fn resume_project_nameless() {
        let m = match_ui_command("继续上次任务").unwrap();
        match m.cmd {
            UiCmd::ResumeProject { name, .. } => assert_eq!(name, ""),
            other => panic!("expected ResumeProject, got {other:?}"),
        }
        let m2 = match_ui_command("接着上次工程").unwrap();
        assert_eq!(m2.cmd, UiCmd::ResumeProject { name: String::new(), parent_dir: "~/Downloads".into() });
    }

    #[test]
    fn approve_deny_last() {
        assert_eq!(cmd_of("确认写入"), Some(UiCmd::ApproveLast));
        assert_eq!(cmd_of("就这么干"), Some(UiCmd::ApproveLast));
        assert_eq!(cmd_of("开工"), Some(UiCmd::ApproveLast));
        assert_eq!(cmd_of("同意"), Some(UiCmd::ApproveLast));
        assert_eq!(cmd_of("拒绝"), Some(UiCmd::DenyLast));
        assert_eq!(cmd_of("取消写入"), Some(UiCmd::DenyLast));
    }

    #[test]
    fn approve_does_not_steal_precise_intents() {
        // 「开始守护」仍命中 GuardTrigger，不能被 ApproveLast 的「开始」吞掉。
        assert_eq!(cmd_of("开始守护"), Some(UiCmd::GuardTrigger));
        // 「确认退出」仍是 ExitConfirm（step 0 先命中）。
        assert_eq!(cmd_of("确认退出"), Some(UiCmd::ExitConfirm));
        // 「取消静音」仍是 MuteOff。
        assert_eq!(cmd_of("取消静音"), Some(UiCmd::MuteOff));
    }
}
