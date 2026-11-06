#!/bin/sh
# 翻译助手技能（纯本地词表参考，无网络、无文件读写副作用）。
# 六道安全闸：
#  ① 命令注入——text 只做字符串匹配与输出回显，绝不拼进 shell 命令；
#  ② 路径遍历——本技能不涉及任何本地路径；
#  ③ 数据外泄——不联网、不读取/上传任何文件，原文只在本机内存处理；
#  ④ 越权——只读处理，不写文件、不删文件；
#  ⑤ 硬编码密钥——无任何密钥/API key/token；
#  ⑥ 异常输入——空参/超长/非法格式均友好中文提示后 exit 0。
# 设计原则（取自素材 §2）：整句翻译由主模型完成，脚本只查词表做参考与校验，
# 绝不做子串自动替换（旧版自动代换曾把 debug 误替换成「程式碼錯誤」）。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
TEXT=$(printf '%s' "$ARGS" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$TEXT" ]; then
  # 非 JSON 场景：去掉外壳字符后按纯文本处理
  TEXT=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
TEXT=$(printf '%s' "$TEXT" | tr -s ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [ -z "$TEXT" ]; then
  echo "请告诉我要翻译的内容，例如：翻译一下「你好」，或 这个用英语怎么说：会议几点开始"
  exit 0
fi

# 超长输入截断（保护输出长度）
TEXT=$(printf '%s' "$TEXT" | cut -c 1-500)

# ---------- 方向识别：含汉字即中→英，否则英→中 ----------
if printf '%s' "$TEXT" | grep -q '[一-龥]'; then
  DIR="中 → 英"
else
  DIR="英 → 中"
fi

# ---------- 本地常用中英对照表（格式：原文|译文，共 60 条） ----------
ZH_EN='你好|Hello / Hi
谢谢|Thank you / Thanks
不客气|You are welcome
早上好|Good morning
晚安|Good night
再见|Goodbye / See you
对不起|Sorry / I am sorry
没关系|Never mind / It does not matter
请稍等|Please wait a moment
没问题|No problem
好的|Okay / All right
我明白了|I see / I understand
请问|Excuse me / May I ask
多少钱|How much is it
请再说一遍|Please say it again
很高兴认识你|Nice to meet you
祝你愉快|Have a nice day
我同意|I agree
我不同意|I disagree
请回复|Please reply
会议几点开始|What time does the meeting start
邮件已收到|The email has been received
我稍后回复你|I will get back to you later
文件在哪里|Where is the file
有什么问题吗|Any questions
辛苦啦|Thanks for your hard work
请注意|Please note / Attention please
明天见|See you tomorrow
一路平安|Have a safe trip
加油|Keep it up / Go for it'

EN_ZH='hello|你好
thank you|谢谢你
thanks|谢谢
good morning|早上好
good night|晚安
sorry|对不起
please|请
okay|好的
goodbye|再见
see you|再见
welcome|欢迎
excuse me|打扰一下
no problem|没问题
i agree|我同意
i disagree|我不同意
help|帮助
email|邮件
file|文件
meeting|会议
deadline|截止日期
project|项目
report|报告
schedule|日程安排
confirm|确认
review|评审 / 复核
update|更新
urgent|紧急
as soon as possible|尽快
let me know|请告诉我
i will get back to you|我稍后回复你'

# ---------- 整句精确匹配（仅整句命中才给译文；绝不子串替换） ----------
# 纯命令替换完成，不落临时文件（只读无副作用）
HIT=""
case "$DIR" in
  "中 → 英")
    HIT=$(printf '%s\n' "$ZH_EN" | while IFS='|' read -r zh en; do
          [ -z "$zh" ] && continue
          if [ "$zh" = "$TEXT" ]; then printf '%s\n' "$en"; fi
        done | head -1)
    ;;
  *)
    LOW=$(printf '%s' "$TEXT" | tr 'A-Z' 'a-z')
    HIT=$(printf '%s\n' "$EN_ZH" | while IFS='|' read -r en zh; do
          [ -z "$en" ] && continue
          if [ "$en" = "$LOW" ]; then printf '%s\n' "$zh"; fi
        done | head -1)
    ;;
esac

# ---------- 术语参考：列出词表中被原文包含的词条（仅供参考，不回写整句） ----------
REF=""
if [ "$DIR" = "中 → 英" ]; then
  REF=$(printf '%s\n' "$ZH_EN" | while IFS='|' read -r zh en; do
        [ -z "$zh" ] && continue
        case "$TEXT" in
          *"$zh"*) printf '%s→%s；' "$zh" "$en" ;;
        esac
      done)
else
  LOW=$(printf '%s' "$TEXT" | tr 'A-Z' 'a-z')
  REF=$(printf '%s\n' "$EN_ZH" | while IFS='|' read -r en zh; do
        [ -z "$en" ] && continue
        case "$LOW" in
          *"$en"*) printf '%s→%s；' "$en" "$zh" ;;
        esac
      done)
fi
[ -z "$REF" ] && REF="（无）"

# ---------- 输出 ----------
echo "【翻译助手 · 本地词表参考】（未联网，原文未发送到任何外部服务）"
echo "方向：$DIR"
echo "原文：$TEXT"
if [ -n "$HIT" ]; then
  echo "词表整句命中：译文：$HIT"
  echo "可改进提示：词表已覆盖整句，可直接播报；主模型可按语境微调语气，专业/法律/医疗文本仍需人工复核。"
else
  echo "词表整句命中：（词表未收录该句——脚本不做子串自动替换，请主模型整句翻译）"
  echo "术语参考（仅供核对，禁止直接拼接成句）：$REF"
  echo "可改进提示：请主模型按语境整句翻译，术语以参考为准；专业/法律/医疗文本需提示用户人工复核。"
fi
# v18.2 面板协议块：气泡展示翻译参考
echo "@qibai-panel:begin"
echo "mode=bubble"
echo "title=翻译参考"
if [ -n "$HIT" ]; then
  echo "html=<div><b>译文参考</b>：$HIT<br/><small>$DIR · 本地词表 · 未联网</small></div>"
else
  echo "html=<div><b>术语参考</b>：$REF<br/><small>$DIR · 词表未整句命中，由主模型整句翻译</small></div>"
fi
echo "@qibai-panel:end"