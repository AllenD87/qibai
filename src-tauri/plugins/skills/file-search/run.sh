#!/bin/sh
# 文件搜索技能（只读查找，无写入、无网络、无副作用）。
# 职责边界：脚本只做确定性检索（Glob 文件名通道），相关性判断与播报交给主模型；
# 搜不到就明说，绝不编造路径（素材库 capture 技能的诚信红线）。
#
# 六道安全闸（逐条实现）：
#   ① 命令注入：keyword 只作为引号包裹的 find 参数传入，绝不拼进 shell 命令串；
#   ② 路径遍历：关键词拒绝含「..」；find 根固定为 $HOME，maxdepth 6 限制深度；
#   ③ 数据外泄：无任何 curl/wget/网络调用，结果只打印给本会话；
#   ④ 越权：只读 find，绝不写/删/改任何文件；权限仅 files:read；
#   ⑤ 硬编码密钥：本脚本无任何密钥/token；
#   ⑥ 异常输入：空参/超长/非法写法均给出友好中文提示后 exit 0。
# 依赖：GNU find / grep / sed（Linux 桌面自带）。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"

# ---------- 抽取关键词（sed 抽 JSON 字段，失败降级为纯文本） ----------
KW=$(printf '%s' "$ARGS" | sed -n 's/.*"keyword"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$KW" ]; then
  KW=$(printf '%s' "$ARGS" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
fi
if [ -z "$KW" ]; then
  KW=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
KW=$(printf '%s' "$KW" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

# ---------- ⑥ 空参友好提示 ----------
if [ -z "$KW" ]; then
  echo "请告诉我要找的文件名关键词，例如：找一下「简历」或「report」。我只在你的用户目录里按文件名查找，全程只读、不改任何文件。"
  exit 0
fi

# ---------- ② 路径遍历防护：拒绝 .. 与越界写法 ----------
case "$KW" in
  *..*)
    echo "关键词里不能带「..」这类路径写法。请直接给我文件名里的几个字，例如「发票」「合同」，我只在你的用户目录内查找。"
    exit 0
    ;;
  /*)
    # 用户误给绝对路径：不跟随、不进入，仅取最后一段作为关键词
    KW=$(printf '%s' "$KW" | sed 's|^.*/||')
    ;;
esac

# ---------- ⑥ 超长关键词截断提示 ----------
LEN_B=$(printf '%s' "$KW" | wc -c | tr -d ' ')
if [ "$LEN_B" -gt 50 ]; then
  echo "关键词有点长，挑文件名里最有代表性的两三个字再试一次吧，比如「发票」「plan」。"
  exit 0
fi

# ---------- 根目录校验 ----------
ROOT="${HOME:-}"
if [ -z "$ROOT" ] || [ ! -d "$ROOT" ]; then
  echo "没找到你的用户主目录（HOME 环境为空），无法执行搜索。"
  exit 0
fi

# ---------- 执行只读检索 ----------
# 排除：依赖与构建目录 node_modules/.git/target/.cache；
# 隐私目录 .ssh/.gnupg 不进入；名称含 secret/password/credential/token 的目录一律跳过。
# keyword 全程引号包裹传参（闸①）；find 不跟随符号链接（GNU find 默认行为）。
RES=$(find "$ROOT" -maxdepth 6 \
  \( -name node_modules -o -name .git -o -name target -o -name .cache \
     -o -name .ssh -o -name .gnupg \
     -o -name '*secret*' -o -name '*password*' -o -name '*credential*' -o -name '*token*' \
  \) -prune \
  -o -type f -iname "*$KW*" -print 2>/dev/null | head -20)

# ---------- 输出：命中或「未找到」，均不编造 ----------
if [ -z "$RES" ]; then
  echo "未找到文件名里包含「$KW」的文件（已在你的用户目录下、最深 6 层范围内检索）。"
  echo "建议：换个更短的关键词试试，比如文件名里的两三个字；或告诉我文件大概类型（文档/表格/图片），我帮你缩小范围。"
  echo "（全程只读，未改动任何文件；以上为真实检索结果，未编造任何路径。）"
else
  CNT=$(printf '%s\n' "$RES" | grep -c . )
  echo "【文件搜索】在你的用户目录下，找到这些文件名包含「$KW」的文件（共 $CNT 条，已列前 20）："
  printf '%s\n' "$RES" | sed 's/^/  - /'
  echo "（按文件名模糊匹配；已排除 node_modules/.git/target/.cache、.ssh 等隐私目录与含 secret/password 字样的目录；全程只读，未改动任何文件。）"
fi
# v18.2 面板协议块：window 搜索结果
if [ -n "$RES" ]; then
  echo "@qibai-panel:begin"
  echo "mode=window"
  echo "title=文件搜索结果 · $KW"
  echo "html=<div><h3>文件搜索 · «$KW»</h3><p>共 <b>$CNT</b> 条（列前 20）：</p><ul>"
  printf '%s\n' "$RES" | sed 's|^|  <li>|; s|$|</li>|'
  echo "</ul><p><small>只读搜索，未改动任何文件；已排除隐私与依赖目录。</small></p></div>"
  echo "@qibai-panel:end"
else
  echo "@qibai-panel:begin"
  echo "mode=window"
  echo "title=文件搜索结果 · $KW"
  echo "html=<div><h3>文件搜索 · «$KW»</h3><p>未找到匹配文件。</p></div>"
  echo "@qibai-panel:end"
fi