//! 天气查询（wttr.in 免费接口，无需 API Key）。
//! 供 query_weather 技能调用：返回可直接播报的中文天气文本。

pub async fn fetch_weather(city: &str) -> Result<String, String> {
    let city = city.trim();
    if city.is_empty() {
        return Err("城市名为空".to_string());
    }
    let url = format!("https://wttr.in/{}?format=j1&lang=zh", city);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", "qibai-desktop/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("请求天气服务失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("天气服务返回 HTTP {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| format!("读取天气响应失败: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析天气数据失败: {e}"))?;
    Ok(format_weather(&v, city))
}

/// 把 wttr.in j1 JSON 解析成中文播报文本（抽为纯函数以便沙盒离线单测）。
/// 各字段缺失均有兜底（"?" / 空串），不 panic。
pub fn format_weather(v: &serde_json::Value, fallback_city: &str) -> String {
    let cc = &v["current_condition"][0];
    let area = v["nearest_area"][0]["areaName"][0]["value"]
        .as_str()
        .unwrap_or(fallback_city);
    let desc = cc["weatherDesc"][0]["value"].as_str().unwrap_or("未知");
    let temp = cc["temp_C"].as_str().unwrap_or("?");
    let feels = cc["FeelsLikeC"].as_str().unwrap_or("?");
    let hum = cc["humidity"].as_str().unwrap_or("?");
    let wind = cc["windspeedKmph"].as_str().unwrap_or("?");
    let dir = cc["winddir16Point"].as_str().unwrap_or("");
    let mut out = format!(
        "{} 当前{}，气温 {}°C，体感 {}°C，湿度 {}%",
        area, desc, temp, feels, hum
    );
    if !dir.is_empty() && wind != "?" {
        out.push_str(&format!("，{}风 {} 公里/小时", dir, wind));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_weather_full_fields() {
        // 全字段路径：area/desc/temp/体感/湿度/风向 都拼进播报。
        let v = json!({
            "nearest_area": [{"areaName":[{"value":"石家庄"}]}],
            "current_condition": [{"weatherDesc":[{"value":"晴"}],"temp_C":"18",
                "FeelsLikeC":"17","humidity":"44","windspeedKmph":"12","winddir16Point":"东北"}]
        });
        let out = format_weather(&v, "fallback");
        assert!(out.starts_with("石家庄 当前晴，气温 18°C"), "实际: {out}");
        assert!(out.contains("体感 17°C"));
        assert!(out.contains("湿度 44%"));
        assert!(out.contains("东北风 12 公里/小时"), "实际: {out}");
    }

    #[test]
    fn format_weather_missing_fields_fall_back() {
        // 缺字段兜底：area 用 fallback、温度 "?"、无风（不拼风向段）。
        let v = json!({});
        let out = format_weather(&v, "邢台");
        assert!(out.starts_with("邢台 当前未知，气温 ?°C"), "实际: {out}");
        assert!(!out.contains("风 "), "无风时不应拼风向段: {out}");
    }
}
