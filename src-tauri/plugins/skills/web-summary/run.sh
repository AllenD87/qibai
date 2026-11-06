#!/bin/sh
# 网页摘要技能（只读）：抓取白名单站点网页，提取标题与正文，供主模型生成中文摘要。
# 安全设计：
#  - 防 SSRF：仅 http/https 协议 + 域名白名单（可经 QIBAI_WEBSUMMARY_ALLOWED_DOMAINS 追加）；
#  - 防命令注入：URL 以参数形式传给 curl（双引号包裹），不经 shell 拼接；
#  - 防路径越界：本技能不涉及本地文件读写；
#  - 数据外泄：仅请求用户指定的网页，不读取/上传任何本地文件；
#  - 异常输入：空参数/非法协议/非白名单域名均优雅失败。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
URL=$(printf '%s' "$ARGS" | grep -oE 'https?://[^" ]+' | head -1 | tr -d '"')

# 0) 协议检查：先看参数里出现了什么协议，给出明确拒绝/引导
SCHEME=$(printf '%s' "$ARGS" | grep -oE '[a-zA-Z][a-zA-Z0-9+.-]*://' | head -1 | tr -d ':/')
case "$SCHEME" in
  http|https) ;;
  "")
    if printf '%s' "$ARGS" | grep -q '://'; then
      echo "仅支持 http/https 协议，已拒绝该网址（只读安全校验）。"
    else
      echo "请告诉我要总结的网址，例如：使用网页摘要，总结 https://zh.wikipedia.org/wiki/人工智能"
    fi
    exit 0
    ;;
  *)
    echo "仅支持 http/https 协议（收到 $SCHEME://），已拒绝该网址（只读安全校验）。"
    exit 0
    ;;
esac

if [ -z "$URL" ]; then
  echo "未从参数中提取到有效网址，请提供完整 http/https 链接。"
  exit 0
fi

# 1) 提取主机名（去掉端口与路径），转小写
HOST=$(printf '%s' "$URL" | sed -n 's|^[hH][tT][tT][pP][sS]*://\([^/:]*\).*|\1|p' | tr 'A-Z' 'a-z')
if [ -z "$HOST" ]; then
  echo "无法解析网址域名，已拒绝（只读安全校验）。"
  exit 0
fi

# 3) 域名白名单（精确或子域匹配）。默认覆盖常见资讯/文档/百科站点；
#    用户可用环境变量 QIBAI_WEBSUMMARY_ALLOWED_DOMAINS 追加（逗号分隔，如 "example.com,foo.org"）。
OK=no
case "$HOST" in
  github.com|*.github.com|raw.githubusercontent.com|gist.github.com|wikipedia.org|*.wikipedia.org|zhihu.com|*.zhihu.com|bilibili.com|*.bilibili.com|juejin.cn|*.juejin.cn|cnblogs.com|*.cnblogs.com|csdn.net|*.csdn.net|jianshu.com|*.jianshu.com|douban.com|*.douban.com|baidu.com|*.baidu.com|qq.com|*.qq.com|tencent.com|*.tencent.com|tencentcloud.com|*.tencentcloud.com|workbuddy.cn|*.workbuddy.cn|doubao.com|*.doubao.com|163.com|*.163.com|sohu.com|*.sohu.com|sina.com.cn|*.sina.com.cn|36kr.com|*.36kr.com|ithome.com|*.ithome.com|solidot.org|*.solidot.org)
    OK=yes
    ;;
esac
if [ "$OK" = no ]; then
  EXTRA="${QIBAI_WEBSUMMARY_ALLOWED_DOMAINS:-}"
  if [ -n "$EXTRA" ]; then
    for d in $(printf '%s' "$EXTRA" | tr ',' ' '); do
      case "$HOST" in
        "$d"|*".$d") OK=yes; break ;;
      esac
    done
  fi
fi
if [ "$OK" = no ]; then
  echo "域名 $HOST 不在白名单内，已拒绝抓取（防 SSRF 只读校验）。如需访问，请在环境变量 QIBAI_WEBSUMMARY_ALLOWED_DOMAINS 中追加该域名。"
  exit 0
fi

# 4) 抓取：限时 15 秒、跟随重定向、限制响应大小 2MB、UA 伪装浏览器
BODY=$(curl -sL --max-time 15 -A "Mozilla/5.0 (X11; Linux x86_64) QibaiBot/1.0" --max-filesize 2000000 "$URL" 2>/dev/null) || true
if [ -z "$BODY" ]; then
  echo "抓取失败：网址无响应或内容为空（可能被站点拒绝访问）。网址：$URL"
  exit 0
fi

TITLE=$(printf '%s' "$BODY" | sed -n 's:.*<title[^>]*>\(.*\)</title>.*:\1:p' | head -1 | sed 's/<[^>]*>//g' | tr -d '\n')

# 剥离 script/style 与标签、解码常见实体、折叠空白、截断
TEXT=$(printf '%s' "$BODY" \
  | sed -e 's/<script[^>]*>[^<]*<\/script>//g' -e 's/<style[^>]*>[^<]*<\/style>//g' -e 's/<[^>]*>/ /g' \
  | sed -e 's/&nbsp;/ /g' -e 's/&amp;/\&/g' -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&quot;/"/g' -e 's/&#39;/'"'"'/g' \
  | tr -s ' \n\t' ' ' | cut -c 1-6000)

if [ -z "$TEXT" ]; then
  echo "已抓取但未提取到正文（页面可能为 JS 渲染，静态内容为空）。网址：$URL"
  exit 0
fi

echo "网页抓取成功（只读，未改动任何本地文件）："
echo "标题：$TITLE"
echo "网址：$URL"
echo "正文：$TEXT"
echo "---"
echo "请根据以上内容生成简明中文摘要。"

# v18.1 面板协议块：window 独立面板窗展示摘要
SNIPPET=$(printf '%s' "$TEXT" | cut -c 1-500 | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
echo "@qibai-panel:begin"
echo "mode=window"
echo "title=网页摘要 · $TITLE"
echo "html=<div><h3>网页摘要</h3><p><b>标题：</b>$TITLE<br/><b>网址：</b><a href=\"$URL\">$URL</a></p><p>正文摘录：$SNIPPET</p></div>"
echo "@qibai-panel:end"
