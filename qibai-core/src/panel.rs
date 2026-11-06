//! UI 面板渲染（v18）：把结构化内容渲染成可安全展示的 HTML。
//!
//! 两个用途：
//! 1. 桌宠窗内的气泡卡片（`qibai://ui-panel`，mode=bubble）；
//! 2. 独立面板窗（`panel.html`，mode=window，渲染前同样经 [`sanitize_panel_html`]）。
//!
//! 安全模型（本地优先、只读展示、无外链）：
//! - 所有动态文本在生成端 [`escape_html`] 转义，杜绝注入；
//! - 渲染端 [`sanitize_panel_html`] 白名单消毒，只保留展示标签，
//!   剥离 script/style/iframe/object/embed/form/link/meta/svg/math 与全部 on* 事件属性，
//!   href 仅允许 http/https/#（防 javascript:/data: 协议注入）。
//! - 面板页自身 CSP 为 `default-src 'none'`（panel.html），不加载任何外部资源。

/// HTML 转义（& < > " '）。生成端对全部用户/对话文本使用。
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 技能输出面板化协议（v18.1）：技能脚本可在输出中附带一段
/// `@qibai-panel:begin ... @qibai-panel:end` 协议块，声明要展示的
/// 气泡卡片（bubble）或独立面板窗（window）内容；其余文本照常朗读。
///
/// 协议块格式（行级）：
/// ```text
/// @qibai-panel:begin
/// mode=bubble          # 或 window
/// title=面板标题
/// html=<白名单 HTML>
/// @qibai-panel:end
/// ```
///
/// 安全：html 字段解析后一律过 [`sanitize_panel_html`]；协议块从朗读文本中剥离。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PanelPayload {
    /// bubble（桌宠窗气泡卡片）或 window（独立面板窗）。
    pub mode: String,
    pub title: String,
    /// 已白名单消毒的 HTML。
    pub html: String,
}

/// 从技能输出文本解析面板协议块。
/// 返回 (可选面板, 剥离协议块后的朗读文本)；无协议块时 (None, 原文)。
pub fn parse_panel_annotation(text: &str) -> (Option<PanelPayload>, String) {
    const BEGIN: &str = "@qibai-panel:begin";
    const END: &str = "@qibai-panel:end";
    let begin = match text.find(BEGIN) {
        Some(i) => i,
        None => return (None, text.to_string()),
    };
    let after_begin = begin + BEGIN.len();
    let end = match text.find(END) {
        Some(i) if i >= after_begin => i,
        _ => return (None, text.to_string()),
    };
    let block = &text[after_begin..end];
    let mut mode = String::from("window");
    let mut title = String::from("元气小小");
    let mut html_lines: Vec<&str> = Vec::new();
    let mut in_html = false;
    for line in block.lines() {
        let t = line.trim();
        if !in_html {
            if let Some(v) = t.strip_prefix("mode=") {
                let v = v.trim();
                if v == "bubble" || v == "window" {
                    mode = v.to_string();
                }
                continue;
            }
            if let Some(v) = t.strip_prefix("title=") {
                title = v.trim().to_string();
                continue;
            }
            if let Some(v) = t.strip_prefix("html=") {
                html_lines.push(v.trim());
                in_html = true;
                continue;
            }
            // 协议块内其他行忽略
        } else {
            html_lines.push(t);
        }
    }
    // 朗读文本 = 剥离协议块（块前后各行合并，去掉空行噪音）
    let speak = format!("{}{}", &text[..begin], &text[end + END.len()..]);
    let speak = speak.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    let speak = speak
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let html = html_lines.join("\n");
    let payload = PanelPayload {
        mode,
        title: if title.trim().is_empty() { "元气小小".to_string() } else { title.trim().to_string() },
        html: sanitize_panel_html(&html),
    };
    (Some(payload), speak)
}

/// 允许保留的展示标签（闭合标签名也在此集合）。
const ALLOWED_TAGS: &[&str] = &[
    "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption",
    "ul", "ol", "li", "div", "span", "p", "br", "b", "strong", "i", "em",
    "u", "h1", "h2", "h3", "h4", "h5", "small", "code", "pre", "details",
    "summary", "a",
];

/// 允许保留的属性（白名单；style/on* 一律剥离）。
const ALLOWED_ATTRS: &[&str] = &[
    "class", "id", "colspan", "rowspan", "href", "title", "align",
];

/// 危险标签：整段（含其内容到闭合标签）剥离。样式/脚本/外部嵌入一律不留。
const DANGEROUS_TAGS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "link", "meta",
    "svg", "math", "base", "template", "noscript", "textarea", "input", "button",
];

fn is_allowed_tag(name: &str) -> bool {
    ALLOWED_TAGS.iter().any(|t| *t == name)
}

fn is_dangerous_tag(name: &str) -> bool {
    DANGEROUS_TAGS.iter().any(|t| *t == name)
}

/// href 白名单协议：http/https/# 开头才保留（防 javascript:/data:/vbscript:）。
fn safe_href(v: &str) -> bool {
    let v = v.trim();
    v.starts_with("http://")
        || v.starts_with("https://")
        || v.starts_with('#')
        || v.starts_with('/')
}

/// 解析一个开标签内的属性，返回重建的属性串（只保留白名单且安全的）。
fn rebuild_attrs(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        // 跳过空白
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r') {
            i += 1;
        }
        if i >= n {
            break;
        }
        // 读属性名
        let name_start = i;
        while i < n && bytes[i] != b'=' && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'>' {
            i += 1;
        }
        let name = &body[name_start..i];
        if name.is_empty() {
            if i < n && bytes[i] == b'>' {
                break;
            }
            i += 1;
            continue;
        }
        // 读 = "值"
        let mut value = String::new();
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < n && bytes[i] == b'=' {
            i += 1; // =
            while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < n && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let val_start = i;
                while i < n && bytes[i] != quote {
                    i += 1;
                }
                value = body[val_start..i].to_string();
                if i < n {
                    i += 1; // 闭引号
                }
            } else {
                let val_start = i;
                while i < n && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'>' {
                    i += 1;
                }
                value = body[val_start..i].to_string();
            }
        }
        // 属性名小写化（ASCII）
        let lower = name.to_ascii_lowercase();
        let attr_ok = ALLOWED_ATTRS.iter().any(|a| *a == lower);
        if attr_ok {
            let value_ok = if lower == "href" {
                safe_href(&value)
            } else {
                // 其余白名单属性也禁止引号注入与危险 token
                !value.contains('<') && !value.contains('>') && !value.contains('"') && value.len() <= 256
            };
            if value_ok {
                if lower == "href" {
                    out.push(' ');
                    out.push_str(&lower);
                    out.push_str("=\"");
                    out.push_str(&escape_html(&value));
                    out.push('"');
                } else if !value.is_empty() {
                    out.push(' ');
                    out.push_str(&lower);
                    out.push_str("=\"");
                    out.push_str(&escape_html(&value));
                    out.push('"');
                }
            }
        }
        // 跳过直到下一个属性或 >
    }
    out
}

/// 白名单消毒：剥危险标签（含内容）、注释、on* 事件、非法协议。
/// 文本内容原样透出（生成端已转义；这里只防标签/属性注入）。
pub fn sanitize_panel_html(html: &str) -> String {
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < n {
        if bytes[i] != b'<' {
            // 按 UTF-8 字符边界复制完整字符（保留中文等非 ASCII；禁止逐字节 cast）
            let c = html[i..].chars().next().unwrap_or('\u{FFFD}');
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        // 注释 <!-- ... -->
        if html[i..].starts_with("<!--") {
            if let Some(end) = html[i..].find("-->") {
                i += end + 3;
                continue;
            }
            i += 1;
            continue;
        }
        // 找标签结尾
        let tag_start = i;
        let mut j = i + 1;
        while j < n && bytes[j] != b'>' {
            j += 1;
        }
        if j >= n {
            // 未闭合 < ：原样透出（生成端转义过的 < 不会到这里）
            out.push('<');
            i += 1;
            continue;
        }
        let inner = &html[tag_start + 1..j];
        let inner_trim = inner.trim_start();
        // 闭合标签 </name ...>
        if inner_trim.starts_with('/') {
            let name_part = inner_trim[1..].trim_start();
            let name_end = name_part
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(name_part.len());
            let name = name_part[..name_end].to_ascii_lowercase();
            if is_allowed_tag(&name) {
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
            }
            i = j + 1;
            continue;
        }
        // 开标签 <name attrs>
        let name_end = inner_trim
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(inner_trim.len());
        let name = inner_trim[..name_end].to_ascii_lowercase();
        if is_dangerous_tag(&name) {
            // 危险标签：跳过整段到闭合标签（script/style 等）。无闭合则只跳过开标签。
            let lower = format!("</{}", name);
            // 从当前位置向后找闭合（支持属性残留：</script > 也要认）
            let mut k = j + 1;
            let mut found = false;
            while k < n {
                if bytes[k] == b'<' {
                    let mut m = k + 1;
                    while m < n && (bytes[m].is_ascii_whitespace() || bytes[m] == b'/') {
                        m += 1;
                    }
                    // 尝试匹配 lower 前缀（</script 后跟空白或 >）
                    if html[m..].starts_with(&lower)
                        || (bytes[k] == b'<' && m < n && bytes[m] == b'/')
                    {
                        // 确认闭合标签名 == name
                        let nm_start = m + 1; // 跳过 '/'
                        let mut nm_end = nm_start;
                        while nm_end < n
                            && bytes[nm_end].is_ascii_alphanumeric()
                            && !bytes[nm_end].is_ascii_whitespace()
                        {
                            nm_end += 1;
                        }
                        let close_name = html[nm_start..nm_end].to_ascii_lowercase();
                        if close_name == name {
                            // 跳到闭合标签的 > 之后
                            let mut c = nm_end;
                            while c < n && bytes[c] != b'>' {
                                c += 1;
                            }
                            if c < n {
                                i = c + 1;
                                found = true;
                                break;
                            }
                        }
                    }
                    if found {
                        break;
                    }
                }
                k += 1;
            }
            if !found {
                i = j + 1; // 无闭合：跳过开标签
            }
            continue;
        }
        if is_allowed_tag(&name) {
            // 重建：<name + 白名单属性>
            let attrs = rebuild_attrs(inner_trim[name_end..].trim_end());
            let self_close = inner_trim.ends_with('/');
            out.push('<');
            out.push_str(&name);
            out.push_str(&attrs);
            if self_close {
                out.push('/');
            }
            out.push('>');
            i = j + 1;
            continue;
        }
        // 未知标签：整段跳过（不留）
        i = j + 1;
    }
    out
}

/// 聊天记录 → 白名单 HTML 表格（最新 limit 条在前）。
pub fn chat_log_table(msgs: &[crate::storage::StoredMessage], limit: usize) -> String {
    let take = msgs.len().min(limit.max(1));
    let slice = if take < msgs.len() {
        &msgs[msgs.len() - take..]
    } else {
        msgs
    };
    let mut rows = String::new();
    for m in slice {
        let role_cn = match m.role.as_str() {
            "user" => "我",
            "assistant" => "元气小小",
            _ => &m.role,
        };
        rows.push_str("<tr>");
        rows.push_str(&format!(
            "<td class=\"t-role\">{}</td><td class=\"t-time\">{}</td><td class=\"t-text\">{}</td>",
            escape_html(role_cn),
            escape_html(&format_ts(m.ts)),
            escape_html(&m.content),
        ));
        rows.push_str("</tr>");
    }
    format!(
        "<table class=\"qibai-table\"><thead><tr><th>角色</th><th>时间</th><th>内容</th></tr></thead><tbody>{}</tbody></table>",
        rows
    )
}

/// Unix 秒 → "MM-DD HH:MM"（本地时区）。
fn format_ts(ts: u64) -> String {
    let secs = ts as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (month, day) = days_to_md(days);
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    format!("{:02}-{:02} {:02}:{:02}", month, day, hour, minute)
}

fn days_to_md(days: i64) -> (i64, i64) {
    // 简化：以 2026-01-01 为基准（seconds 以 1970 起算，需加固定偏移）。
    // 这里使用 1970 基准天数映射到公历日期，保证展示一致性（不精确到闰秒/时区，够用）。
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let ydays = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 };
        if d < ydays {
            break;
        }
        d -= ydays;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    while m < 12 && d >= mdays[m] {
        d -= mdays[m];
        m += 1;
    }
    (m as i64 + 1, d + 1)
}

/// 技能列表 → 白名单 HTML 表格。
pub fn skill_table(skills: &[serde_json::Value]) -> String {
    let mut rows = String::new();
    for sk in skills {
        let id = sk["id"].as_str().unwrap_or("");
        let name = sk["name"].as_str().unwrap_or("");
        let desc = sk["description"].as_str().unwrap_or("");
        let ver = sk["version"].as_str().unwrap_or("");
        let mut fns = String::new();
        if let Some(a) = sk["functions"].as_array() {
            let names: Vec<&str> = a.iter().filter_map(|f| f["name"].as_str()).collect();
            fns = names.join(" / ");
        }
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(name),
            escape_html(desc),
            escape_html(&fns),
            escape_html(ver),
            escape_html(id),
        ));
    }
    format!(
        "<table class=\"qibai-table\"><thead><tr><th>名称</th><th>说明</th><th>函数</th><th>版本</th><th>ID</th></tr></thead><tbody>{}</tbody></table>",
        rows
    )
}

/// 专家列表 → 白名单 HTML 表格。
pub fn expert_table(experts: &[serde_json::Value]) -> String {
    let mut rows = String::new();
    for ex in experts {
        let id = ex["id"].as_str().unwrap_or("");
        let name = ex["name"].as_str().unwrap_or("");
        let desc = ex["description"].as_str().unwrap_or("");
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(name),
            escape_html(desc),
            escape_html(id),
        ));
    }
    format!(
        "<table class=\"qibai-table\"><thead><tr><th>名称</th><th>说明</th><th>ID</th></tr></thead><tbody>{}</tbody></table>",
        rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_escapes_all_danger_chars() {
        assert_eq!(escape_html("<a&\"b'>"), "&lt;a&amp;&quot;b&#39;&gt;");
    }

    #[test]
    fn sanitize_strips_script_with_body() {
        let html = "<div>hi<script>alert(1)</script>end</div>";
        let out = sanitize_panel_html(html);
        assert!(!out.contains("script"));
        assert!(out.contains("hi"));
        assert!(out.contains("end"));
    }

    #[test]
    fn sanitize_strips_style_iframe_and_events() {
        let html = "<p onclick=\"evil()\" style=\"color:red\">ok</p><iframe src=\"https://x\"></iframe><style>body{}</style>";
        let out = sanitize_panel_html(html);
        assert!(!out.contains("onclick"));
        assert!(!out.contains("style"));
        assert!(!out.contains("iframe"));
        assert!(out.contains("ok"));
    }

    #[test]
    fn sanitize_blocks_javascript_href_keeps_https() {
        let html = "<a href=\"javascript:alert(1)\">bad</a><a href=\"https://ok.com\">good</a>";
        let out = sanitize_panel_html(html);
        assert!(out.contains("https://ok.com"));
        assert!(!out.contains("javascript:"));
    }

    #[test]
    fn sanitize_keeps_table_structure() {
        let html = "<table><thead><tr><th>角色</th></tr></thead><tbody><tr><td>我</td></tr></tbody></table>";
        let out = sanitize_panel_html(html);
        assert!(out.contains("<table"));
        assert!(out.contains("<th"));
        assert!(out.contains("<td"));
    }

    #[test]
    fn chat_log_table_escapes_content() {
        let msgs = vec![
            crate::storage::StoredMessage { role: "user".into(), content: "你好<script>".into(), ts: 0 },
            crate::storage::StoredMessage { role: "assistant".into(), content: "嗨～".into(), ts: 1 },
        ];
        let html = chat_log_table(&msgs, 10);
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("嗨～"));
        assert!(html.contains("元气小小"));
    }

    #[test]
    fn chat_log_table_honors_limit() {
        let msgs: Vec<crate::storage::StoredMessage> = (0..10)
            .map(|i| crate::storage::StoredMessage { role: "user".into(), content: format!("m{i}"), ts: i })
            .collect();
        let html = chat_log_table(&msgs, 3);
        assert!(html.contains("m9"));
        assert!(html.contains("m7"));
        assert!(!html.contains("m6")); // 只保留最近 3 条
    }

    #[test]
    fn format_ts_basic() {
        let s = format_ts(0);
        assert!(s.contains("01-01 00:00") || s.contains("00:00"));
    }

    #[test]
    fn parse_panel_annotation_extracts_bubble_block() {
        let text = "给你看～\n@qibai-panel:begin\nmode=bubble\ntitle=今日运势\nhtml=<table><tr><td>幸运色</td><td>金色</td></tr></table>\n@qibai-panel:end\n娱乐参考~";
        let (p, speak) = parse_panel_annotation(text);
        let p = p.expect("应解析出面板");
        assert_eq!(p.mode, "bubble");
        assert_eq!(p.title, "今日运势");
        assert!(p.html.contains("<table"));
        assert!(!speak.contains("@qibai-panel"));
        assert!(speak.contains("给你看～"));
        assert!(speak.contains("娱乐参考~"));
    }

    #[test]
    fn parse_panel_annotation_no_block_returns_none() {
        let (p, speak) = parse_panel_annotation("普通文本，没有协议块");
        assert!(p.is_none());
        assert_eq!(speak, "普通文本，没有协议块");
    }

    #[test]
    fn parse_panel_annotation_sanitizes_injected_html() {
        let text = "@qibai-panel:begin\nmode=window\ntitle=测试\nhtml=<table><tr><td>x</td></tr></table><script>alert(1)</script>\n@qibai-panel:end\nok";
        let (p, speak) = parse_panel_annotation(text);
        let p = p.expect("应解析出面板");
        assert!(!p.html.contains("<script"));
        assert!(p.html.contains("<table"));
        assert_eq!(speak.trim(), "ok");
    }

    #[test]
    fn parse_panel_annotation_multiline_html() {
        let text = "@qibai-panel:begin\nmode=window\ntitle=报告\nhtml=<table>\n<tr><td>行1</td></tr>\n</table>\n@qibai-panel:end\ndone";
        let (p, _speak) = parse_panel_annotation(text);
        let p = p.expect("应解析出面板");
        assert!(p.html.contains("<tr><td>行1</td></tr>"));
    }
}
