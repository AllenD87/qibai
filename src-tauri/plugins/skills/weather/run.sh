#!/bin/sh
# 查天气技能：wttr.in 免费公开接口（无需 key）。
# 安全设计：只读 GET、10s 内超时、无外发用户数据（仅城市名）、无文件副作用。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
CITY=$(printf '%s' "$ARGS" | sed -n 's/.*"city"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$CITY" ]; then
  CITY=$(printf '%s' "$ARGS" | sed -e 's/^{"city"://' -e 's/}$//' -e 's/^"//' -e 's/"$//' | tr -d '{}[]"')
fi
[ -z "$CITY" ] && { echo '请告诉我城市名，比如：使用查天气，看看北京今天怎么样。'; exit 0; }

CITY_ENC=$(printf '%s' "$CITY" | python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.stdin.read().strip()))')
DATA=$(curl -s --max-time 8 "https://wttr.in/${CITY_ENC}?format=j1&lang=zh")

if [ -z "$DATA" ]; then
  echo "天气查询没成功（可能没网络），稍后再试试？"
  exit 0
fi
if ! printf '%s' "$DATA" | python3 -c 'import json,sys;json.load(sys.stdin)' 2>/dev/null; then
  echo "没查到「$CITY」的天气，换个城市名试试？"
  exit 0
fi

printf '%s' "$DATA" | python3 -c '
import json, sys

WEATHER_ZH = {
    "Sunny": "晴", "Clear": "晴", "Partly cloudy": "多云转晴", "Cloudy": "多云",
    "Overcast": "阴", "Mist": "薄雾", "Fog": "雾", "Freezing fog": "冻雾",
    "Light rain": "小雨", "Moderate rain": "中雨", "Heavy rain": "大雨",
    "Light drizzle": "毛毛雨", "Patchy rain possible": "零星小雨",
    "Thundery outbreaks possible": "可能雷阵雨", "Light snow": "小雪",
    "Moderate snow": "中雪", "Heavy snow": "大雪", "Sleet": "雨夹雪",
    "Light rain shower": "小阵雨", "Moderate or heavy rain shower": "中到大阵雨",
    "Windy": "大风", "Blizzard": "暴风雪", "Freezing rain": "冻雨",
}
try:
    d = json.load(sys.stdin)
    c = d.get("current_condition", [{}])[0]
    area = d.get("nearest_area", [{}])[0]
    name = area.get("areaName", [{}])[0].get("value", "") or "当地"
    temp = c.get("temp_C", "?")
    feel = c.get("FeelsLikeC", "?")
    desc_en = (c.get("weatherDesc", [{}])[0].get("value", "") or "").strip()
    desc = WEATHER_ZH.get(desc_en, desc_en or "未知天气")
    hum = c.get("humidity", "?")
    wind = c.get("windspeedKmph", "?")
    print(f"{name} 现在 {temp}°C（体感 {feel}°C），{desc}，湿度 {hum}%，风速 {wind} km/h")
except Exception:
    print("天气数据解析失败，稍后再试试？")
'
