#!/bin/sh
# 代码审查技能（只读静态扫描，绝不写文件）：对用户指定目录做危险模式清单扫描。
# 六道安全闸：
#  ① 命令注入——path 先经白名单校验，之后仅作为 grep 的位置参数（双引号包裹），绝不拼进命令字符串；
#  ② 路径遍历——拒绝含 ".." 的路径；相对路径按 $HOME 解析，绝对路径必须在 $HOME 内；
#  ③ 数据外泄——不联网，只把扫描结果打到 stdout，不读取/上传任何内容；
#  ④ 越权——只读 grep，无任何写/删/改操作；
#  ⑤ 硬编码密钥——脚本本身无密钥；扫描清单反而专门找用户代码里的硬编码密钥；
#  ⑥ 异常输入——空参/目录不存在/路径越界均友好中文提示后 exit 0。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
TARGET=$(printf '%s' "$ARGS" | sed -n 's/.*"path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$TARGET" ]; then
  TARGET=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
TARGET=$(printf '%s' "$TARGET" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [ -z "$TARGET" ]; then
  echo "请告诉我要审查的代码目录，例如：代码审查 projects/my-app（相对路径按用户目录解析）"
  exit 0
fi

# 闸②a：拒绝路径遍历
case "$TARGET" in
  *..*)
    echo "已拒绝：路径中不允许包含「..」（防路径遍历）。只读检查，未做任何改动。"
    exit 0
    ;;
esac

# 闸②b：绝对路径必须在用户目录 $HOME 内；相对路径按 $HOME 解析
case "$TARGET" in
  /*)
    case "$TARGET" in
      "$HOME"|"$HOME"/*) ;;
      *)
        echo "已拒绝：只允许扫描用户目录（$HOME）内的路径，收到：$TARGET（只读安全校验）。"
        exit 0
        ;;
    esac
    ;;
  *)
    TARGET="$HOME/$TARGET"
    ;;
esac
TARGET=$(printf '%s' "$TARGET" | sed 's|/$||')

# 目录存在性检查
if [ ! -d "$TARGET" ]; then
  echo "目录不存在或不是目录：$TARGET（只读检查，未做任何改动）。请确认路径是否正确后再试。"
  exit 0
fi

# 扫描范围：常见代码文件后缀；排除依赖/构建产物/版本库
INCLUDES="--include=*.py --include=*.js --include=*.ts --include=*.jsx --include=*.tsx --include=*.sh --include=*.java --include=*.go --include=*.rb --include=*.php --include=*.c --include=*.cc --include=*.cpp --include=*.h --include=*.rs --include=*.sql --include=*.vue --include=*.yml --include=*.yaml"
EXCLUDES="--exclude-dir=node_modules --exclude-dir=.git --exclude-dir=venv --exclude-dir=.venv --exclude-dir=__pycache__ --exclude-dir=dist --exclude-dir=build --exclude-dir=.next --exclude-dir=target"

HAS=no
N_FOUND=0
FOUND_TYPES=""
echo "【代码审查 · 只读静态扫描】（未改动任何文件）"
echo "扫描目录：$TARGET"
echo "风险清单："

# 扫描函数：$1=风险类型  $2=修复建议  $3=正则模式
scan() {
  hits=$(grep -rInE $INCLUDES $EXCLUDES "$3" "$TARGET" 2>/dev/null | head -10)
  if [ -n "$hits" ]; then
    HAS=yes
    N_FOUND=$((N_FOUND+1))
    FOUND_TYPES="$FOUND_TYPES<li><b>$1</b>：$2</li>
"
    printf '🔴 [%s] 修复建议：%s\n' "$1" "$2"
    printf '%s\n' "$hits" | sed 's/^/   /'
  fi
}

scan "eval 动态执行" "避免把外部输入交给 eval；改用查表/白名单分发，或用 JSON 解析替代" \
     'eval[[:space:]]*\('

scan "命令执行/拼接" "避免 os.system / subprocess shell=True；外部输入勿拼进 shell 命令，改用参数数组调用" \
     '(subprocess[^[:space:]]*[[:space:]]*shell[[:space:]]*=[[:space:]]*True|os\.system[[:space:]]*\(|exec[[:space:]]*\()'

scan "危险删除 rm -rf" "rm -rf 属于不可逆操作；确认路径、加交互确认或改走 trash 机制" \
     'rm[[:space:]]+-[a-zA-Z]*[rf][a-zA-Z]*[[:space:]]'

scan "硬编码密钥/口令" "立即移出代码：改用环境变量/配置中心，并轮换已泄露的密钥" \
     'sk-[A-Za-z0-9_-]{10,}|AKIA[0-9A-Z]{16}|password[[:space:]]*=[[:space:]]*["'"'"'][^"'"'"']{4,}["'"'"']'

scan "curl|sh 远程脚本管道执行" "禁止 curl 后直接管道给 sh/bash；先下载审查再执行" \
     'curl[[:space:]][^|]*\|[[:space:]]*(sh|bash|zsh)'

# 危险 SQL：DELETE/DROP 行内不含 WHERE 的才算高危
SQLHITS=$(grep -rInEi $INCLUDES $EXCLUDES '(delete[[:space:]]+from|drop[[:space:]]+table)' "$TARGET" 2>/dev/null | while IFS= read -r line; do
  case "$line" in
    *where*|*WHERE*) ;;
    *) printf '%s\n' "$line" ;;
  esac
done | head -10)
if [ -n "$SQLHITS" ]; then
  HAS=yes
  printf '🔴 [危险 SQL（DELETE/DROP 缺 WHERE）] 修复建议：补 WHERE 条件与备份，生产环境先在事务里试跑\n'
  printf '%s\n' "$SQLHITS" | sed 's/^/   /'
fi

if [ "$HAS" = no ]; then
  echo "   未发现高危模式（只读扫描，未改动任何文件）。"
else
  echo "---"
  echo "提示：以上为确定性模式命中，是否真风险请结合上下文判断；建议 Critical（硬编码密钥/rm -rf/eval）立即修复。"
fi

# v18.1 面板协议块：window 独立面板窗展示审查报告
if [ "$HAS" = yes ]; then
  echo "@qibai-panel:begin"
  echo "mode=window"
  echo "title=代码审查报告"
  echo "html=<div><h3>代码审查 · $TARGET</h3><p>发现 <b>$N_FOUND</b> 类风险模式（确定性扫描，需人工复核）：</p><ul>"
  printf '%s' "$FOUND_TYPES"
  echo "</ul></div>"
  echo "@qibai-panel:end"
else
  echo "@qibai-panel:begin"
  echo "mode=window"
  echo "title=代码审查报告"
  echo "html=<div><h3>代码审查 · $TARGET</h3><p>未发现高危模式（只读扫描，未改动任何文件）。</p></div>"
  echo "@qibai-panel:end"
fi
