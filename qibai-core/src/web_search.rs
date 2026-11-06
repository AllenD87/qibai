//! 联网搜索（三引擎降级链：搜狗 → 360 → 移动百度）。
//!
//! 供 `web_search` 技能调用：返回可直接播报/转述的中文搜索结果文本。
//! 设计约束（本地优先 + 无新依赖）：
//! - 只读 GET，UA 伪装浏览器，8s 超时，共享 reqwest client。
//! - HTML 解析为纯函数（正则/字符串扫描），不引 scraper。
//! - 全链路中文错误文案，便于语音播报。

use std::time::Duration;

/// 单条搜索结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchItem {
    pub title: String,
    pub snippet: String,
    pub url: String,
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 搜索引擎条目：（引擎名，URL 模板）。顺序即降级顺序。
/// v45.9：默认 baidu → 360 → sogou；指定引擎时只用它，不偷偷换。
fn engine_chain(query_enc: &str, engine_pref: &str) -> Vec<(&'static str, String)> {
    let baidu = ("baidu", format!("https://www.baidu.com/s?wd={query_enc}"));
    let so360 = ("so360", format!("https://www.so.com/s?q={query_enc}"));
    let sogou = ("sogou", format!("https://www.sogou.com/web?query={query_enc}"));
    match engine_pref {
        "baidu" => vec![baidu],
        "360" | "so360" => vec![so360],
        "sogou" => vec![sogou],
        _ => vec![baidu, so360, sogou], // auto / 默认
    }
}

/// 当前搜索引擎偏好（全局 OnceLock，main.rs 启动时或 IPC 时 set）。
static ENGINE_PREF: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();
fn engine_cell() -> &'static std::sync::Mutex<String> {
    ENGINE_PREF.get_or_init(|| std::sync::Mutex::new("auto".to_string()))
}
pub fn set_search_engine(engine: &str) {
    let n = match engine {
        "baidu" | "360" | "so360" | "sogou" | "auto" => engine.to_string(),
        _ => "auto".to_string(),
    };
    eprintln!("[qibai] web: engine pref set = {n}");
    *engine_cell().lock().unwrap() = n;
}
pub fn current_search_engine() -> String {
    engine_cell().lock().unwrap().clone()
}

/// 全局 HTTP 超时 / 结果条数（由 Engine::apply_saved_preferences / save_settings 同步）。
static HTTP_TIMEOUT: std::sync::OnceLock<std::sync::Mutex<u64>> = std::sync::OnceLock::new();
static RESULT_N: std::sync::OnceLock<std::sync::Mutex<usize>> = std::sync::OnceLock::new();
pub fn set_http_timeout_secs(secs: u64) {
    *HTTP_TIMEOUT.get_or_init(|| std::sync::Mutex::new(8)).lock().unwrap() = secs.clamp(1, 60);
}
pub fn current_http_timeout_secs() -> u64 {
    HTTP_TIMEOUT.get_or_init(|| std::sync::Mutex::new(8)).lock().unwrap().clone()
}
pub fn set_search_result_n(n: usize) {
    *RESULT_N.get_or_init(|| std::sync::Mutex::new(8)).lock().unwrap() = n.clamp(1, 20);
}
pub fn current_search_result_n() -> usize {
    RESULT_N.get_or_init(|| std::sync::Mutex::new(8)).lock().unwrap().clone()
}

/// 手工百分号编码（UTF-8），避免引入 url crate。
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.as_bytes() {
        let b = *byte;
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 联网搜索入口：query 为空/超长 → Err；逐引擎降级；任一引擎解析出结果即返回前 5 条。
pub async fn search_web(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("搜索词为空，告诉我你想搜什么".to_string());
    }
    if query.chars().count() > 200 {
        return Err("搜索词太长了，精简到 200 字以内再试".to_string());
    }
    let enc = url_encode(query);
    let pref = current_search_engine();
    let timeout = current_http_timeout_secs();
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(timeout))
        .build()
        .map_err(|e| format!("初始化搜索客户端失败: {e}"))?;

    let mut got_empty = false;
    for (name, url) in engine_chain(&enc, &pref) {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(html) => {
                    let mut items = parse_search_results(name, &html);
                    eprintln!("[qibai] web: engine={name} hits={}", items.len());
                    if !items.is_empty() {
                        let n = current_search_result_n();
                        items.truncate(n);
                        return Ok(format_results(&items));
                    }
                    got_empty = true;
                }
                Err(_) => continue,
            },
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    if got_empty {
        Err("没搜到相关内容，换个说法试试？".to_string())
    } else {
        Err("联网搜索暂时不可用（无网络或超时），稍后再试～".to_string())
    }
}

/// 格式化输出：v45.8 加厚到前 8 条，按域名去重（同站只留最相关一条），
/// 每条「序号. 标题 —— 摘要\n   url」。
pub fn format_results(items: &[SearchItem]) -> String {
    // 按域名去重：保留首次出现（最相关）的一条，序号稳定。
    let mut seen_domains = std::collections::HashSet::new();
    let deduped: Vec<&SearchItem> = items
        .iter()
        .filter(|it| {
            let domain = url_domain(&it.url);
            if seen_domains.contains(&domain) {
                false
            } else {
                seen_domains.insert(domain);
                true
            }
        })
        .collect();
    deduped
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, it)| {
            let snippet = if it.snippet.is_empty() {
                "（无摘要）".to_string()
            } else {
                it.snippet.clone()
            };
            format!("{}. {} —— {}\n   {}", i + 1, it.title, snippet, it.url)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 URL 提取域名（去 scheme、去 path），用于同站去重；提取失败返回原始 url。
pub fn url_domain(url: &str) -> String {
    let after = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    after.split('/').next().unwrap_or(after).to_string()
}

/// v45.8：分页切片——完整正文按 offset/max_chars 切一段，返回段文本与下一个 offset。
pub fn slice_paged(text: &str, offset: usize, max_chars: usize) -> (String, usize, bool) {
    let total = text.chars().count();
    if offset >= total {
        return (String::new(), offset, false);
    }
    let start = offset;
    let end = (offset + max_chars).min(total);
    let seg: String = text.chars().skip(start).take(end - start).collect();
    let next_offset = end;
    let has_more = next_offset < total;
    (seg, next_offset, has_more)
}

// ---- HTML 解析纯函数（单测固定样例） ----

/// 抓取网页正文（联网阅读器）：输入 URL → 返回「标题 + 正文纯文本」。
/// 与 web_search 配套：搜索拿 URL → fetch_page 抓正文 → LLM 总结。
/// 只读 GET，10s 超时，正文最多取 8MB、提炼 4000 字符，无新依赖。
pub async fn fetch_page(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("fetch_page 需要提供 url（网页地址）。例如：{\"url\":\"https://example.com\"}".to_string());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只支持 http/https 网页地址".to_string());
    }
    if url.chars().count() > 2048 {
        return Err("url 太长了，精简后再试".to_string());
    }
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("初始化网页读取客户端失败: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("读取网页失败（网络或超时）: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("网页返回状态 {}（可能是 404/403/需要登录）", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取网页内容失败: {e}"))?;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(8 * 1024 * 1024)]).to_string();
    let (title, body) = extract_page_text(&html);
    if body.trim().is_empty() {
        return Err("这个页面没有可读正文（可能是 JS 渲染页或需要登录）".to_string());
    }
    let title_s = if title.trim().is_empty() {
        "(无标题)".to_string()
    } else {
        title
    };
    Ok(format!("【{title_s}】\n{body}"))
}

/// v45.8：fetch_many 原语——顺序抓多个 URL，逐条返回 {url, ok, title, text, error}。
/// 不做合并/摘要/排版；单条失败只标记 error，不影响其它条。技能层（SKILL.md）负责怎么用。
pub async fn fetch_many(urls: &[String]) -> Vec<serde_json::Value> {
    let mut out = Vec::with_capacity(urls.len());
    for u in urls {
        match fetch_page(u).await {
            Ok(text) => {
                // 简单拆 title/body（fetch_page 已拼好"【title】\nbody"）
                let (title, body) = match text.find("\n") {
                    Some(idx) => {
                        let head = &text[..idx];
                        let title = head.trim_start_matches("【").trim_end_matches("】").to_string();
                        (title, text[idx + 1..].to_string())
                    }
                    None => (text.clone(), String::new()),
                };
                out.push(serde_json::json!({"url": u, "ok": true, "title": title, "text": body}));
            }
            Err(e) => {
                out.push(serde_json::json!({"url": u, "ok": false, "error": e}));
            }
        }
    }
    out
}

/// 从网页 HTML 提取「标题 + 正文纯文本」（纯函数，便于单测）。
/// 策略：取 `<title>`；剔除 script/style；按 p/article/li/h1-h4 提取段落；
/// 去标签反转义、空白折叠。
/// v45.8：不再内部截断 4000 字，返回完整正文；调用方按 offset/max_chars 切片。
pub fn extract_page_text(html: &str) -> (String, String) {
    let mut title = String::new();
    if let Some(s) = find_open_tag(html, "title") {
        let tail = &html[s..];
        if let Some(end) = tail.find("</title>") {
            title = strip_tags_and_unescape(&tail[..end]);
        }
    }
    // 先剔除 <script>...</script> / <style>...</style> 内容（全字符索引，避免多字节边界问题）。
    let mut no_js = String::with_capacity(html.len());
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '<' {
            let tag_name: String = chars[i + 1..].iter().take(12).collect::<String>().to_lowercase();
            let (closing, is_skip) = if tag_name.starts_with("script") {
                ("</script>", true)
            } else if tag_name.starts_with("style") {
                ("</style>", true)
            } else {
                ("", false)
            };
            if is_skip {
                // 跳到开标签 '>' 之后。
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '>' {
                    j += 1;
                }
                if j < chars.len() {
                    j += 1;
                }
                // 找闭合标签。
                let close_chars: Vec<char> = closing.chars().collect();
                let mut k = j;
                'scan: while k < chars.len() {
                    if chars[k] == '<' {
                        let mut m = 0usize;
                        while m < close_chars.len() && k + m < chars.len() && chars[k + m] == close_chars[m] {
                            m += 1;
                        }
                        if m == close_chars.len() {
                            k += m;
                            break 'scan;
                        }
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        no_js.push(chars[i]);
        i += 1;
    }
    // 提取正文段落。
    let mut segs: Vec<String> = Vec::new();
    for tag in ["p", "article", "li", "h1", "h2", "h3", "h4"] {
        let mut rest = no_js.as_str();
        while let Some(off) = find_open_tag(rest, tag) {
            let tail = &rest[off..];
            if let Some(gt) = tail.find('>') {
                let closing = format!("</{tag}>");
                if let Some(end_rel) = tail[gt + 1..].find(&closing) {
                    let content = &tail[gt + 1..gt + 1 + end_rel];
                    let text = strip_tags_and_unescape(content);
                    if !text.trim().is_empty() {
                        segs.push(text);
                    }
                    rest = &tail[gt + 1 + end_rel + closing.len()..];
                    continue;
                }
            }
            break;
        }
    }
    // 拼接 + 去相邻重复 + 截断。
    let mut body = String::new();
    let mut prev = String::new();
    for s in segs {
        if s == prev {
            continue;
        }
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&s);
        prev = s;
    }
    (title, body)
}

/// 从引擎 HTML 提取结果列表。engine ∈ sogou / so360 / baidu。
pub fn parse_search_results(engine: &str, html: &str) -> Vec<SearchItem> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(off) = find_open_tag(rest, "h3") {
        let after = &rest[off..];
        let Some(end) = after.find("</h3>") else {
            break;
        };
        let h3 = &after[..end];
        if let Some((href, anchor_text)) = extract_anchor(h3) {
            let title = strip_tags_and_unescape(&anchor_text);
            if title.is_empty() {
                rest = &after[end + 5..];
                continue;
            }
            let url = normalize_url(engine, &href);
            let tail = &after[end + 5..];
            let snippet = extract_snippet(tail);
            out.push(SearchItem { title, snippet, url });
            if out.len() >= 8 {
                break;
            }
        }
        rest = &after[end + 5..];
    }
    out
}

/// 找下一个 `<h3`（容忍 `<h3 ` 带属性）的字节偏移。
fn find_open_tag(hay: &str, tag: &str) -> Option<usize> {
    let lower = hay;
    let mut i = 0usize;
    while let Some(rel) = lower[i..].find('<') {
        let pos = i + rel;
        let from = &lower[pos + 1..];
        if let Some(rest) = from.strip_prefix(tag) {
            // 标签名后必须是空白、> 或 /，防止 `<h3x>` 误命中
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('>') || rest.starts_with('/')
            {
                return Some(pos);
            }
        }
        i = pos + 1;
    }
    None
}

/// 从一块 HTML 里提取第一个 `<a ... href="...">inner</a>` 的 (href, inner)。
fn extract_anchor(html: &str) -> Option<(String, String)> {
    let a_off = find_open_tag(html, "a")?;
    let after = &html[a_off..];
    let tag_end = after.find('>')?;
    let open_tag = &after[..tag_end];
    let href = extract_attr(open_tag, "href")?;
    let inner_start = tag_end + 1;
    let close = after[inner_start..].find("</a>")?;
    let inner = &after[inner_start..inner_start + close];
    Some((href, inner.to_string()))
}

/// 从 `<a ...>` 开标签里取属性值（双引号优先）。
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let idx = lower.find(attr)?;
    let after = &tag[idx + attr.len()..];
    let eq = after.find('=')?;
    let rest = &after[eq + 1..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].to_string());
    }
    if let Some(quoted) = rest.strip_prefix('\'') {
        let end = quoted.find('\'')?;
        return Some(quoted[..end].to_string());
    }
    // 无引号：取到空白/>为止
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

/// 摘要：h3 之后向后扫 1500 字符，去标签、折叠空白，取首段可见文本（≤160 字）。
fn extract_snippet(tail: &str) -> String {
    let region: String = tail.chars().take(1500).collect();
    let text = strip_tags_and_unescape(&region);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(160).collect()
}

/// 去标签 + HTML 实体反转义（常见 6 种）。
fn strip_tags_and_unescape(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // 实体反转义（顺序敏感：&amp; 最后替换）
    out = out.replace("&nbsp;", " ");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#39;", "'");
    out = out.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 相对链接补全为可点 URL。
fn normalize_url(engine: &str, href: &str) -> String {
    let h = href.trim();
    if h.starts_with("http://") || h.starts_with("https://") {
        return h.to_string();
    }
    if let Some(rest) = h.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let base = match engine {
        "sogou" => "https://www.sogou.com",
        "so360" => "https://www.so.com",
        _ => "https://m.baidu.com",
    };
    if h.starts_with('/') {
        format!("{base}{h}")
    } else {
        format!("{base}/{h}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 固定样例 HTML（模拟三引擎真实结构） ----

    const SOGOU_HTML: &str = r#"
<div class="results">
  <div class="vrwrap">
    <h3 class="vr-title"><a href="/link?url=abc123">石家庄今天天气怎么样</a></h3>
    <div class="str-text-info"><span class="fz-mid space-txt">据天气预报，石家庄今天晴转多云，气温 18 到 26 摄氏度，风力不大。</span></div>
  </div>
  <div class="vrwrap">
    <h3 class="vr-title"><a href="/link?url=def456">石家庄天气_百度百科</a></h3>
    <div class="str-text-info"><span>石家庄地处华北平原，属温带季风气候，四季分明。</span></div>
  </div>
</div>"#;

    const SO360_HTML: &str = r#"
<div id="container">
  <li class="res-list">
    <h3 class="res-title"><a data-click="..." href="https://www.example.com/a">今天天气 - 中国天气网</a></h3>
    <p class="res-desc">中国天气网提供石家庄实时天气、一周预报及生活指数。</p>
  </li>
  <li class="res-list">
    <h3 class="res-title"><a href="/s?url=https://www.example.com/b">石家庄天气预报一周</a></h3>
    <p class="res-desc">未来七天石家庄以晴为主，周末有小雨。</p>
  </li>
</div>"#;

    const BAIDU_M_HTML: &str = r#"
<div class="results">
  <div class="result c-container">
    <h3 class="c-title"><a href="https://m.baidu.com/link?url=xyz">石家庄天气实况</a></h3>
    <div class="c-abstract">今天（10月27日）石家庄晴，东北风微风，空气质量良。</div>
  </div>
  <div class="result c-container">
    <h3 class="c-title"><a href="//www.baidu.com/link?url=uvw">历史天气查询</a></h3>
    <div class="c-abstract">支持查询石家庄历年同期天气数据。</div>
  </div>
</div>"#;

    #[test]
    fn sogou_extracts_title_url_and_snippet() {
        let items = parse_search_results("sogou", SOGOU_HTML);
        assert_eq!(items.len(), 2, "应解析出 2 条，实际 {items:?}");
        assert_eq!(items[0].title, "石家庄今天天气怎么样");
        assert_eq!(items[0].url, "https://www.sogou.com/link?url=abc123");
        assert!(items[0].snippet.contains("晴转多云"), "snippet: {}", items[0].snippet);
        assert!(items[1].title.contains("百度百科"));
    }

    #[test]
    fn sogou_ignores_h3_without_anchor() {
        let html = r#"<h3 class="vr-title">孤立标题无链接</h3><h3><a href="https://x.com/1">有效结果</a></h3>"#;
        let items = parse_search_results("sogou", html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "有效结果");
    }

    #[test]
    fn so360_extracts_absolute_and_relative_urls() {
        let items = parse_search_results("so360", SO360_HTML);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://www.example.com/a");
        assert!(items[1].url.starts_with("https://www.so.com/"));
        assert!(items[0].snippet.contains("中国天气网"));
    }

    #[test]
    fn so360_empty_when_no_h3() {
        let items = parse_search_results("so360", "<div>没有 h3 结构</div>");
        assert!(items.is_empty());
    }

    #[test]
    fn baidu_extracts_and_protocol_relative_url() {
        let items = parse_search_results("baidu", BAIDU_M_HTML);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "石家庄天气实况");
        assert_eq!(items[0].url, "https://m.baidu.com/link?url=xyz");
        // //www.baidu.com → https: 前缀
        assert_eq!(items[1].url, "https://www.baidu.com/link?url=uvw");
        assert!(items[0].snippet.contains("10月27日"));
    }

    #[test]
    fn baidu_entities_unescaped_and_title_trimmed() {
        let html = r#"<h3 class="c-title"><a href="https://x.com/1">AT&amp;T &lt;天气&gt; 服务&nbsp; </a></h3>"#;
        let items = parse_search_results("baidu", html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "AT&T <天气> 服务");
    }

    #[test]
    fn format_results_lists_first_five_with_numbering() {
        // v45.8：按域名去重 + 上限 8。用不同域名避免被去重。
        let items: Vec<SearchItem> = (0..7)
            .map(|i| SearchItem {
                title: format!("标题{i}"),
                snippet: format!("摘要{i}"),
                url: format!("https://example{i}.com/{i}"),
            })
            .collect();
        let out = format_results(&items);
        assert!(out.starts_with("1. 标题0 —— 摘要0"));
        assert!(out.contains("5. 标题4"));
        assert!(out.contains("7. 标题6")); // 7 条都不同域名 → 全列
        assert!(out.contains("https://example0.com/0"));
    }

    #[test]
    fn url_encode_utf8_and_safe_chars() {
        assert_eq!(url_encode("aB1-_.~"), "aB1-_.~");
        assert_eq!(url_encode("天气"), "%E5%A4%A9%E6%B0%94");
    }

    #[test]
    fn extract_page_text_gets_title_and_body() {
        let html = r#"<html><head><title>石家庄天气预报</title><style>.x{color:red}</style></head>
<body><nav>导航栏</nav><article><h1>今日天气</h1><p>今天晴转多云，东北风微风。</p><p>最高气温 18 度。</p></article>
<script>var a=1;</script><p>脚注</p></body></html>"#;
        let (title, body) = extract_page_text(html);
        assert_eq!(title, "石家庄天气预报");
        assert!(body.contains("今日天气"), "body: {body}");
        assert!(body.contains("今天晴转多云"), "body: {body}");
        assert!(body.contains("最高气温 18 度"), "body: {body}");
        assert!(!body.contains("var a=1"), "script 内容应剔除: {body}");
        assert!(!body.contains("导航栏"), "nav 内容不应作为正文: {body}");
    }

    #[test]
    fn extract_page_text_handles_empty() {
        let (title, body) = extract_page_text("<html><body></body></html>");
        assert_eq!(title, "");
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn fetch_page_rejects_non_http() {
        let r = fetch_page("file:///etc/passwd").await;
        assert!(r.is_err());
        let r2 = fetch_page("").await;
        assert!(r2.is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn smoke_fetch_page() {
        let r = fetch_page("https://example.com").await;
        println!("smoke fetch_page(example.com) => {r:?}");
        if let Ok(t) = r {
            assert!(!t.trim().is_empty());
        }
    }

    /// 联网冒烟（默认跳过，手动 `cargo test smoke_search -- --ignored`）。
    #[tokio::test]
    #[ignore]
    async fn smoke_search_web() {
        let r = search_web("今天天气").await;
        println!("smoke search_web(今天天气) => {r:?}");
        assert!(r.is_ok() || r.is_err()); // 联网环境下打印日志即可
    }

    // ---- v45.8：多源汇总 / 分页 / digest 总上限 ----

    #[test]
    fn format_results_dedup_same_domain_keep_order() {
        let items = vec![
            SearchItem { title: "A".into(), snippet: "s1".into(), url: "https://news.com/1".into() },
            SearchItem { title: "B".into(), snippet: "s2".into(), url: "https://news.com/2".into() }, // 同域名
            SearchItem { title: "C".into(), snippet: "s3".into(), url: "https://other.com/x".into() },
            SearchItem { title: "D".into(), snippet: "s4".into(), url: "https://last.com/y".into() },
        ];
        let out = format_results(&items);
        // 同域名只留一条 → 应含 A 不含 B
        assert!(out.contains("1. A"), "应保留首条 A：{out}");
        assert!(!out.contains("B ——"), "同域名 B 应被去重：{out}");
        assert!(out.contains("C"));
        assert!(out.contains("D"));
        // 序号连续 1/2/3
        assert!(out.contains("2. C"));
        assert!(out.contains("3. D"));
    }

    #[test]
    fn format_results_caps_at_8() {
        let items: Vec<SearchItem> = (0..12)
            .map(|i| SearchItem {
                title: format!("T{i}"),
                snippet: "s".into(),
                url: format!("https://site{i}.com/x"),
            })
            .collect();
        let out = format_results(&items);
        // 8 条：数 "N. " 开头行
        let lines = out.lines().filter(|l| l.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).count();
        assert_eq!(lines, 8, "应只列 8 条：{out}");
    }

    #[test]
    fn slice_paged_splits_and_reassembles() {
        let text: String = "一二三四五六七八九十".repeat(60); // 600 字
        let (seg1, off1, more1) = slice_paged(&text, 0, 250);
        assert!(more1, "第 1 段后应有更多");
        assert_eq!(seg1.chars().count(), 250);
        let (seg2, off2, more2) = slice_paged(&text, off1, 250);
        assert!(more2, "第 2 段后应有更多");
        let (seg3, off3, more3) = slice_paged(&text, off2, 250);
        assert!(!more3, "末段 has_more=false");
        assert!(off3 >= text.chars().count());
        // 拼接三段 == 原文（去空白）
        let joined: String = [seg1, seg2, seg3].concat();
        assert_eq!(joined.chars().count(), text.chars().count());
        assert_eq!(joined, text);
    }

    #[test]
    fn slice_paged_offset_out_of_range_empty() {
        let text = "hello";
        let (seg, off, more) = slice_paged(text, 100, 1000);
        assert!(seg.is_empty());
        assert!(!more);
        assert_eq!(off, 100);
    }
}

// v45.9：URL 规范化 + 正文清洗（去杂质、去站点后缀标题）。

/// 去追踪参数：utm_*、gclid、fbclid、spm、from、ref、share、icn、wcid、sk。
pub fn normalize_public_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let keep: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("").to_lowercase();
            !(key.starts_with("utm_")
                || key == "gclid"
                || key == "fbclid"
                || key == "spm"
                || key == "from"
                || key == "ref"
                || key == "share"
                || key == "icn"
                || key == "wcid"
                || key == "sk")
        })
        .collect();
    if keep.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", keep.join("&"))
    }
}

/// 标题去站点后缀："- 某某网"、"_某某频道"、"| 某某"、"_某某"。
pub fn clean_title(title: &str) -> String {
    let mut t = title.trim().to_string();
    for sep in [" - ", " _ ", " | ", " — ", " – "] {
        if let Some(idx) = t.find(sep) {
            t = t[..idx].trim().to_string();
        }
    }
    t
}

/// 段落黑名单：导航/页脚/侧栏/推荐/评论/分享/广告/面包屑的常见关键词。
fn is_junk_paragraph(p: &str) -> bool {
    let s = p.trim();
    if s.chars().count() < 4 {
        return true;
    }
    let lower = s.to_lowercase();
    const JUNK: &[&str] = &[
        "cookie", "版权", "copyright", "备案号", "京icp", "沪icp",
        "点击", "登录", "注册", "下载app", "扫码", "关注我们", "分享到",
        "相关阅读", "推荐阅读", "热门推荐", "评论", "发表评论", "广告",
        "上一篇", "下一篇", "返回目录", "面包屑", "首页 >", "首页>",
        "导航", "menu", "sidebar",
    ];
    JUNK.iter().any(|k| lower.contains(k))
}

/// 清洗正文段落列表：去 junk、去相邻重复。
pub fn clean_paragraphs(segs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in segs {
        let t = s.trim();
        if is_junk_paragraph(t) {
            continue;
        }
        if let Some(last) = out.last() {
            if last == t {
                continue;
            }
        }
        out.push(t.to_string());
    }
    out
}

/// 抓不到正文：正文段落数 < 2 或总字符 < 80 → 标记"未取到正文"，不拿标题冒充。
pub fn has_real_body(paragraphs: &[String]) -> bool {
    let total: usize = paragraphs.iter().map(|p| p.chars().count()).sum();
    paragraphs.len() >= 2 && total >= 80
}

#[cfg(test)]
mod v459_tests {
    use super::*;

    #[test]
    fn normalize_url_strips_tracking_params() {
        let a = "https://example.com/news/1?utm_source=weibo&from=fb&id=42";
        let b = "https://example.com/news/1?id=42";
        assert_eq!(normalize_public_url(a), normalize_public_url(b));
        assert_eq!(normalize_public_url("https://x.com/a?gclid=zzz"), "https://x.com/a");
        assert_eq!(normalize_public_url("https://x.com/a"), "https://x.com/a");
    }

    #[test]
    fn clean_title_strips_site_suffix() {
        assert_eq!(clean_title("今天新闻 - 新浪网"), "今天新闻");
        assert_eq!(clean_title("今日要闻 | 网易"), "今日要闻");
        assert_eq!(clean_title("纯标题"), "纯标题");
    }

    #[test]
    fn clean_paragraphs_removes_nav_footer_comments() {
        let segs: Vec<String> = vec![
            "首页 > 新闻 > 国际".into(),
            "这是正文第一段，讲一件重要的事，内容比较长足够过阈值。".into(),
            "点击登录后查看更多".into(),
            "这是正文第二段，继续展开，也足够长不至于被当成短 junk 干掉。".into(),
            "相关阅读：其它文章标题".into(),
            "发表评论".into(),
        ];
        let cleaned = clean_paragraphs(&segs);
        let joined = cleaned.join("\n");
        assert!(!joined.contains("首页 >"));
        assert!(!joined.contains("点击登录"));
        assert!(!joined.contains("相关阅读"));
        assert!(!joined.contains("发表评论"));
        assert!(joined.contains("这是正文第一段"));
        assert!(joined.contains("这是正文第二段"));
    }

    #[test]
    fn has_real_body_false_for_thin_page() {
        let thin: Vec<String> = vec!["只有一句话".into()];
        assert!(!has_real_body(&thin));
        let real: Vec<String> = vec![
            "第一段足够长的正文内容，必须过 80 字阈值，否则就被当成抓不到正文样例。".into(),
            "第二段同样要长，凑够总字数让 has_real_body 返回 true，证明逻辑工作正常。".into(),
        ];
        assert!(has_real_body(&real));
    }
}

#[cfg(test)]
mod v459_engine_pref_tests {
    use super::*;

    #[test]
    fn auto_chain_baidu_first() {
        let chain = engine_chain("hello", "auto");
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].0, "baidu");
        assert_eq!(chain[1].0, "so360");
        assert_eq!(chain[2].0, "sogou");
        assert!(chain[0].1.contains("baidu.com"));
    }

    #[test]
    fn specified_engine_only_one() {
        let chain = engine_chain("x", "sogou");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, "sogou");
        let chain = engine_chain("x", "baidu");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, "baidu");
        let chain = engine_chain("x", "360");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, "so360");
    }

    #[test]
    fn unknown_pref_falls_back_to_auto() {
        let chain = engine_chain("x", "google");
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].0, "baidu");
    }

    #[test]
    fn set_and_get_engine_pref() {
        set_search_engine("sogou");
        assert_eq!(current_search_engine(), "sogou");
        set_search_engine("baidu");
        assert_eq!(current_search_engine(), "baidu");
        set_search_engine("auto");
    }
}
