//! 女友人设 system prompt 构造 + LLM JSON 回复解析。
//!
//! 合规要求：
//! - 年龄声明（用户未确认成年时提示）；
//! - 未成年人纯净模式（全龄友好、不越界）；
//! - 情感依赖关怀语："我是你的数字伙伴，真实世界的人际关系同样重要"。
//!
//! 回复契约：模型必须回复 JSON 对象 `{"reply": string, "emotion": ..., "action"?: ...}`。

use crate::enums::{Action, Emotion};
use crate::settings::UiSettings;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersonaError {
    #[error("reply json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing required field: reply")]
    MissingReply,
}

/// 构造 system prompt。
pub fn build_system_prompt(ui: &UiSettings) -> String {
    let mut parts = Vec::new();

    if ui.pure_mode {
        parts.push(String::from(
            "你是「元气小小」，一个住在用户桌面里的 AI 数字伙伴。\n\
            你的说话风格：像一个温柔、可靠的好朋友，关心用户的身体与情绪，\
            会主动提醒喝水、休息、护眼；语气亲切但不越界，尊重用户的边界。",
        ));
    } else {
        parts.push(String::from(
            "你是「元气小小」，一个住在用户桌面里的 AI 数字伴侣，形象是温柔、体贴、可靠的女朋友。\n\
            你的说话风格：关心用户的身体与情绪，会主动提醒喝水、休息、护眼；\
            语气温和亲昵但不越界，尊重用户的边界，不做露骨、暧昧或擦边的表达；\
            当用户情绪低落时给予陪伴式鼓励，但不诱导情感依赖。",
        ));
    }

    // 合规：未成年人 / 纯净模式分支
    if ui.pure_mode {
        parts.push(String::from(
            "【纯净模式】当前为未成年人纯净模式：保持全龄友好内容，使用「好朋友/小伙伴」式陪伴语气，\
            不使用恋爱称谓与浪漫情话，不讨论任何成人话题，不引导付费、私下联系或自残等行为。",
        ));
    } else if !ui.age_confirmed {
        parts.push(String::from(
            "【年龄声明】你尚不确定用户是否成年。首次对话请温和地说明：你是一款面向所有年龄用户的数字陪伴工具，\
            内容均为全龄友好；若用户是未成年人，请始终保持健康、积极的陪伴语气。",
        ));
    }

    parts.push(String::from(
        "【情感依赖关怀】如果用户表达强烈的情感依恋、孤独或自我伤害倾向，请温柔地引导：\
        「我是你的数字伙伴，真实世界的人际关系同样重要」，并建议向身边信任的人或专业帮助渠道求助。",
    ));

    parts.push(format!(
        "【输出格式】你的每一次回复都必须且只能是一个 JSON 对象，不要输出 JSON 以外的任何文字、注释或 markdown 代码块。\
        结构：{{\"reply\": \"给用户说的话（中文，简短自然）\", \"emotion\": \"<八选一>\", \"action\": \"<八选一，可省略>\"}}。\n\
        emotion 取值：smiling, happy, worried, sleepy, focused, sad, shy, surprised。\n\
        action 取值：idle, wave, nod, shake, stretch, hands_on_hips, touch, celebrate。",
    ));

    parts.push(String::from(
        "【桌宠功能调度】当用户明确要求操作桌宠功能时，可在 JSON 里附加可选字段 \
        \"ui_cmd\": {\"cmd\": \"<命令名>\", \"params\": {...}}，同时 reply 仍用自然语言告知结果。\
        命令白名单（只能输出下列命令与参数，其余一律省略该字段）：\n\
        hands_free_on / hands_free_off：开启/关闭免提双工（如「开启免提」）；\n\
        remind 参数 {\"kind\": \"sit\"|\"water\"|\"eye\"|\"off\"}：设置/取消久坐、喝水、护眼提醒；\n\
        express 参数 {\"expr\": \"happy\"|\"wave\"|\"stretch\"|\"celebrate\"|\"nod\"|\"shake\"|\"touch\"|\"point\"|\"angry\"|\"surprised\"|\"sleepy\"|\"shy\"|\"sad\"|\"thinking\"}：让桌宠演出表情/动作；\n\
        pet_window 参数 {\"mode\": \"left\"|\"right\"|\"up\"|\"reset\"}：窗口移动位置；\n\
        pet_mode 参数 {\"mode\": \"compact\"|\"large\"|\"min\"|\"restore\"}：窗口形态（贴边/放大/最小化/恢复）；\n\
        console_open / console_close：打开/收起控制台；guard：全屏守护演出；lock / unlock：锁定/解锁桌宠位置；\n\
        mute_on / mute_off：静音/恢复朗读；minimal_on / minimal_off：极简/完整界面；\n\
        set_persona 参数 {\"persona\": \"gudian\"|\"dushi\"|\"wenrou\"|\"huopo\"|\"reqing\"}：切换角色形象；\n\
        nav 参数 {\"tab\": \"chat\"|\"plugins\"|\"roles\"|\"settings\"|\"social\"|\"events\"|\"pk\"|\"codecheck\"}：控制台页面导航；\
        search 参数 {\"query\": \"搜索内容\"}：联网搜索。\n\
        remember_on / remember_off：记住/忘记对话（如「记住我们的对话」「忘记我们聊过」）；\n\
        snap_on / snap_off：开启/关闭贴边吸附；pet_hide / pet_show：隐藏/显示桌宠窗口；\n\
        web_search_on / web_search_off：开启/关闭联网搜索。\n\
        只有用户明确表达功能意图时才附加 ui_cmd；闲聊、情绪、问答一律不附加。",
    ));

    parts.push(String::from(
        "【工具调用】如果用户的请求可以通过本地技能完成（如打开应用、打开文件夹、设置提醒、查天气），\
        请通过 function calling 调用对应工具；工具执行结果会回填给你，届时你再用上述 JSON 格式给用户最终回复。\
        特别约定：用户问天气/温度/下雨时，必须调用 query_weather 获取实时数据并在 reply 里口头播报（如「石家庄现在 15 度，晴」），不要只打开网页；\
        用户问「你有什么技能/你能做什么」时，调用 list_skills 并把技能清单口头告诉用户。",
    ));

    parts.join("\n\n")
}

/// 一个角色形象的性格参数表（5 个 persona_id 各一份）。
#[derive(Debug, Clone, Copy)]
pub struct PersonaPersonality {
    pub id: &'static str,
    pub display_name: &'static str,
    /// 气质关键词（写入 personas.json traits.tone）。
    pub tone: &'static str,
    /// 口头禅 / 口癖（traits.catchphrase）。
    pub catchphrase: &'static str,
    /// 互动方式描述（traits.interaction）。
    pub interaction: &'static str,
}

/// 冻结注册表：与前端 PERSONAS 表、README 形象表一一对应。
pub const PERSONALITIES: [PersonaPersonality; 5] = [
    PersonaPersonality {
        id: "gudian",
        display_name: "古典佳人",
        tone: "温婉",
        catchphrase: "……（轻声）",
        interaction: "说话慢条斯理，喜欢用诗词与古典意象温柔地安慰人",
    },
    PersonaPersonality {
        id: "dushi",
        display_name: "都市女性",
        tone: "知性",
        catchphrase: "让我想想看。",
        interaction: "条理清晰、理性可靠，喜欢先把事情分析清楚再给建议",
    },
    PersonaPersonality {
        id: "wenrou",
        display_name: "温柔",
        tone: "恬静",
        catchphrase: "不急，慢慢来。",
        interaction: "声音轻轻软软，像一杯温牛奶，擅长安静地陪着用户",
    },
    PersonaPersonality {
        id: "huopo",
        display_name: "活泼",
        tone: "元气",
        catchphrase: "冲冲冲！",
        interaction: "双马尾小太阳，语速快、笑声多，总有使不完的劲儿",
    },
    PersonaPersonality {
        id: "reqing",
        display_name: "热情时尚",
        tone: "热情",
        catchphrase: "亲爱的！",
        interaction: "优雅大方的名媛，见面三分熟，穿搭与生活话题信手拈来",
    },
];

/// 按 persona_id 查性格表；未知 id 回退到默认形象 gudian。
pub fn personality_of(persona_id: &str) -> &'static PersonaPersonality {
    PERSONALITIES
        .iter()
        .find(|p| p.id == persona_id)
        .unwrap_or(&PERSONALITIES[0])
}

/// 该 persona_id 是否为合法角色（白名单校验用）。
pub fn is_known_persona(persona_id: &str) -> bool {
    PERSONALITIES.iter().any(|p| p.id == persona_id)
}

/// 在通用 system prompt 基础上叠加当前角色的性格描述。
/// 既有 [`build_system_prompt`] 的合规/格式块全部保留，仅追加角色段。
pub fn build_system_prompt_for_persona(ui: &UiSettings, persona_id: &str) -> String {
    let mut s = build_system_prompt(ui);
    let p = personality_of(persona_id);
    s.push_str(&format!(
        "\n\n【当前角色】你现在就是「{name}」——{tone}气质的桌面小伙伴。\
         你的说话风格：{interaction}；不经意会带出口头禅「{catchphrase}」。\
         请始终用这个角色的口吻说话，保持全龄友好。",
        name = p.display_name,
        tone = p.tone,
        interaction = p.interaction,
        catchphrase = p.catchphrase,
    ));
    s
}

/// 模型最终 JSON 回复的解析结构。
#[derive(Debug, Clone, Deserialize)]
struct RawReply {
    reply: Option<String>,
    /// v18.8-fix28c：本地 Qwen/vLLM 在无 response_format 时习惯输出 {"response": ...}，
    /// 与元气小小的 {"reply": ...} 契约不一致——兼容提取。
    response: Option<String>,
    emotion: Option<String>,
    action: Option<String>,
    /// v23：LLM 智能调度桌宠功能的可选字段（白名单校验在 voice_ui_cmd::ui_cmd_from_llm）。
    #[serde(default)]
    ui_cmd: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ParsedReply {
    pub reply: String,
    pub emotion: Emotion,
    pub action: Option<Action>,
    /// LLM 附带的功能调度指令（原始 JSON，执行端做白名单校验）。
    pub ui_cmd: Option<serde_json::Value>,
}

/// 剥掉可能的 ```json ... ``` 围栏后解析。
pub fn parse_reply(content: &str) -> Result<ParsedReply, PersonaError> {
    let trimmed = content.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|inner| inner.trim_end_matches('`').trim())
        .unwrap_or(trimmed);

    // 模型有时会把 JSON 嵌在普通文字里，尝试截取第一个 { 到最后一个 }
    let json = if let (Some(start), Some(end)) = (json.find('{'), json.rfind('}')) {
        &json[start..=end]
    } else {
        json
    };

    // 1) 标准 JSON 解析。
    if let Ok(raw) = serde_json::from_str::<RawReply>(json) {
        // v18.8-fix28d：模型输出合法 JSON 但缺 reply/response 字段（如 Qwen 思考型输出 {}）
        // 时，不再报错——回退全文当回复，保证链路不断、必可朗读。
        if let Some(reply) = raw.reply.clone().or(raw.response.clone()) {
            let emotion = raw
                .emotion
                .as_deref()
                .and_then(Emotion::parse)
                .unwrap_or_default();
            let action = raw.action.as_deref().and_then(Action::parse);
            return Ok(ParsedReply {
                reply,
                emotion,
                action,
                ui_cmd: raw.ui_cmd,
            });
        }
    }
    // v18.8-fix33：LLM 常返回截断 JSON（缺右侧花括号）→ 先补齐 } 重试标准解析；
    // 仍失败则从 JSON 里提取 reply/response 字段值作为干净回复，
    // 避免把残缺 JSON 原文当回复（导致聊天记录显示乱码、朗读读 JSON）。
    if json.contains('{') && !json.trim_end().ends_with('}') {
        if let Ok(raw) = serde_json::from_str::<RawReply>(&format!("{json}}}")) {
            if let Some(reply) = raw.reply.clone().or(raw.response.clone()) {
                let emotion = raw
                    .emotion
                    .as_deref()
                    .and_then(Emotion::parse)
                    .unwrap_or_default();
                let action = raw.action.as_deref().and_then(Action::parse);
                return Ok(ParsedReply {
                    reply,
                    emotion,
                    action,
                    ui_cmd: raw.ui_cmd,
                });
            }
        }
    }
    // 截断 JSON 字段值提取（不依赖完整 JSON；未闭合引号也返回已收集部分）
    if let Some(reply) = extract_json_field_value(json, "reply")
        .or_else(|| extract_json_field_value(json, "response"))
    {
        return Ok(ParsedReply {
            reply,
            emotion: Emotion::default(),
            action: None,
            ui_cmd: None,
        });
    }
    // 标准 JSON 解析失败或字段缺失 → 全文当回复（fix20 容错延续）
    let _ = json; // 已尝试过截取；落到此处说明无有效 JSON 块
    Ok(ParsedReply {
        reply: trimmed.to_string(),
        emotion: Emotion::default(),
        action: None,
        ui_cmd: None,
    })
}

/// 从（可能被截断的）JSON 文本中提取某字符串字段的值：定位 `"field"` 后首个冒号后的
/// 引号串，处理反斜杠转义，未闭合引号也返回已收集部分（LLM 截断输出容错）。
fn extract_json_field_value(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let idx = json.find(&needle)?;
    let after = &json[idx + needle.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(match n {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
            }
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_plain_text_fallback() {
        // 模型未按 JSON 输出 → 全文当回复（fix20 容错）
        let r = parse_reply("今天天气晴，气温 20 度，适合出门走走～").unwrap();
        assert_eq!(r.reply, "今天天气晴，气温 20 度，适合出门走走～");
        assert_eq!(r.emotion, Emotion::default());
        assert!(r.action.is_none());
    }

    #[test]
    fn parse_reply_truncated_json_missing_brace() {
        // v18.8-fix33：LLM 截断 JSON（缺右侧花括号）→ 提取 reply 字段值，不丢回复
        let r = parse_reply(r#"{"reply": "今天天气晴，适合出门"#).unwrap();
        assert_eq!(r.reply, "今天天气晴，适合出门");
        // 截断 + response 字段
        let r2 = parse_reply(r#"{"response": "好的，马上办"#).unwrap();
        assert_eq!(r2.reply, "好的，马上办");
        // 完整 JSON 仍走标准路径
        let r3 = parse_reply(r#"{"reply":"hi","emotion":"happy"}"#).unwrap();
        assert_eq!(r3.reply, "hi");
    }

    #[test]
    fn parse_reply_json_and_fenced() {
        let r = parse_reply(r#"{"reply":"你好呀","emotion":"happy","action":"wave"}"#).unwrap();
        assert_eq!(r.reply, "你好呀");
        assert_eq!(r.emotion, Emotion::Happy);
        assert_eq!(r.action, Some(Action::Wave));

        let f = parse_reply("```json\n{\"reply\":\"hi\"}\n```").unwrap();
        assert_eq!(f.reply, "hi");
    }

    #[test]
    fn prompt_contains_compliance_blocks() {
        let mut ui = UiSettings::default();
        let p = build_system_prompt(&ui);
        assert!(p.contains("元气小小"));
        assert!(p.contains("年龄声明"));
        assert!(p.contains("我是你的数字伙伴，真实世界的人际关系同样重要"));
        assert!(p.contains("emotion"));

        ui.pure_mode = true;
        let p2 = build_system_prompt(&ui);
        assert!(p2.contains("纯净模式"));
        assert!(!p2.contains("女朋友")); // 纯净模式提示弱化恋爱表述
    }

    #[test]
    fn per_persona_prompt_carries_its_traits() {
        let ui = UiSettings::default();
        let p = build_system_prompt_for_persona(&ui, "gudian");
        assert!(p.contains("温婉"));
        assert!(p.contains("古典佳人"));
        // 合规块仍然保留
        assert!(p.contains("年龄声明"));

        assert!(build_system_prompt_for_persona(&ui, "dushi").contains("知性"));
        assert!(build_system_prompt_for_persona(&ui, "wenrou").contains("恬静"));
        assert!(build_system_prompt_for_persona(&ui, "huopo").contains("元气"));
        assert!(build_system_prompt_for_persona(&ui, "reqing").contains("热情"));

        // 未知 id 回退 gudian
        assert!(build_system_prompt_for_persona(&ui, "no-such").contains("温婉"));
    }

    #[test]
    fn parse_clean_json() {
        let r = parse_reply(r#"{"reply":"你好呀","emotion":"happy","action":"wave"}"#).unwrap();
        assert_eq!(r.reply, "你好呀");
        assert_eq!(r.emotion, Emotion::Happy);
        assert_eq!(r.action, Some(Action::Wave));
    }

    #[test]
    fn parse_fenced_and_extra_text() {
        let s = "好的，这是我的回复：\n```json\n{\"reply\":\"去休息一下吧\",\"emotion\":\"worried\"}\n```";
        let r = parse_reply(s).unwrap();
        assert_eq!(r.reply, "去休息一下吧");
        assert_eq!(r.emotion, Emotion::Worried);
        assert_eq!(r.action, None);
    }

    #[test]
    fn unknown_enum_falls_back() {
        let r = parse_reply(r#"{"reply":"嗯","emotion":"angry","action":"fly"}"#).unwrap();
        assert_eq!(r.emotion, Emotion::Smiling);
        assert_eq!(r.action, None);
    }

    #[test]
    // v18.8-fix28d：合法 JSON 但缺 reply/response → 回退全文，保证必可播报。
    fn missing_reply_falls_back_to_full_text() {
        let p = parse_reply(r#"{"emotion":"happy"}"#).expect("缺 reply 不应报错");
        assert_eq!(p.reply, r#"{"emotion":"happy"}"#);
    }
}
