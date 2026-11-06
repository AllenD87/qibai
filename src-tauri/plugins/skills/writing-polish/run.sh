#!/bin/sh
# 写作润色技能（纯文本检查，无网络、无文件读写副作用）。
# 六道安全闸：
#  ① 命令注入——text 只做字符串匹配与回显，绝不拼进 shell 命令；
#  ② 路径遍历——本技能不涉及任何本地路径；
#  ③ 数据外泄——不联网，原文不出本机；
#  ④ 越权——只读检查，不写文件、不修改原文；
#  ⑤ 硬编码密钥——无任何密钥/API key/token；
#  ⑥ 异常输入——空参/超长/非法格式均友好中文提示后 exit 0。
# 脚本只出检查报告、不修改原文；润色改稿由主模型按 SKILL.md 方法论完成。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
TEXT=$(printf '%s' "$ARGS" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$TEXT" ]; then
  TEXT=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
TEXT=$(printf '%s' "$TEXT" | tr -s ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [ -z "$TEXT" ]; then
  echo "请把要润色或检查的这段话发给我，例如：帮我改下这段话：今天天气很好,我们去公园走走。"
  exit 0
fi

# 超长输入截断（保护输出长度；仅截断分析对象，不改动原文）
TEXT=$(printf '%s' "$TEXT" | cut -c 1-2000)

echo "【写作润色 · 基础检查报告】（脚本仅检查，未修改原文）"

# ---------- ④ 字数统计 ----------
CNT=$(printf '%s' "$TEXT" | grep -o '[一-龥]' | wc -l | tr -d ' ')
TOTAL=$(printf '%s' "$TEXT" | wc -m | tr -d ' ')
echo "字数：约 ${CNT} 个汉字（全文字符约 ${TOTAL}）"

# ---------- ① 全角/半角标点混用 ----------
echo "---"
echo "① 标点混用检查（中文语境疑似误用半角 , . ; : ? !）："
PUNCT=$(printf '%s' "$TEXT" | grep -oE '[一-龥][,.;:?!]|[,.;:?!][一-龥]' | head -5)
if [ -n "$PUNCT" ]; then
  echo "$PUNCT" | sed 's/^/   · 片段：「/;s/$/」 建议：中文语境改用全角「，。；：？！」/'
else
  echo "   未发现明显混用。"
fi

# ---------- ② 连续重复字（笔误） ----------
echo "---"
echo "② 连续重复字检查（的的/了了/我我 等疑似笔误）："
REPEAT=$(printf '%s' "$TEXT" | grep -oE '(的的|了了|我我|你你|他他|她她|它它|是是|在在|有有|就就|都都|也也|很很|又又|不不|但但|被被|把把|这这|那那|会会|要要|能能|想想|去去|说说|看看)' | sort | uniq -c | sort -rn)
if [ -n "$REPEAT" ]; then
  printf '%s\n' "$REPEAT" | sed 's/^/   · /'
else
  echo "   未发现连续重复字。"
fi

# ---------- ③ 疑似叠词/重复短语告警（可能是笔误，也可能是修辞，需人工确认） ----------
echo "---"
echo "③ 疑似重复短语（请人工确认：笔误 or 修辞）："
DOUBLE=$(printf '%s' "$TEXT" | grep -oE '(非常非常|特别特别|真的真的|然后然后|就是就是|那个那个|这个这个|因为因为|所以所以|如果如果|但是但是|可能可能|应该应该|已经已经)' | sort | uniq -c)
if [ -n "$DOUBLE" ]; then
  printf '%s\n' "$DOUBLE" | sed 's/^/   · /'
else
  echo "   未发现疑似重复短语。"
fi

# ---------- ⑤ 常见口语词提示 ----------
echo "---"
echo "④ 口语词提示（正式文稿可考虑删改）："
COLLOQ=$(printf '%s' "$TEXT" | grep -oE '(我觉得|就是说|然后|那个|其实|基本上|大概|好像|嗯|嘛|就是)' | sort | uniq -c | sort -rn | head -5)
if [ -n "$COLLOQ" ]; then
  printf '%s\n' "$COLLOQ" | sed 's/^/   · /'
else
  echo "   未发现常见口语词。"
fi

echo "---"
echo "结论：脚本只做确定性检查、不改原文；润色改稿由主模型按 SKILL.md 方法论完成（改动守恒、缺事实打 [NEED:…] 不编造）。"

# v18.2 面板协议块：window 润色检查报告
echo "@qibai-panel:begin"
echo "mode=window"
echo "title=润色检查报告"
echo "html=<div><h3>润色检查报告</h3><p>字数：约 <b>$CNT</b> 个汉字（全文 $TOTAL 字符）。</p><p>检查项：</p><ul>"
[ -n "$REPEAT" ] && echo "<li>连续重复字：发现（疑似笔误）。</li>" || echo "<li>连续重复字：未发现。</li>"
[ -n "$DOUBLE" ] && echo "<li>疑似重复短语：发现（笔误 or 修辞，需人工确认）。</li>" || echo "<li>疑似重复短语：未发现。</li>"
[ -n "$COLLOQ" ] && echo "<li>口语词：发现（正式文稿可考虑删改）。</li>" || echo "<li>口语词：未发现。</li>"
echo "</ul><p><small>脚本只做确定性检查、不改原文；改稿由主模型完成。</small></p></div>"
echo "@qibai-panel:end"
