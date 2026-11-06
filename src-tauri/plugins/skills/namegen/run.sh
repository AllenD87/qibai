#!/bin/sh
# 创意命名技能（纯本地词根库规则组合，无网络、无文件读写副作用）。
# 六道安全闸：
# ①命令注入——参数仅做字符串匹配/字段抽取，绝不拼接进 shell 命令执行；
# ②路径遍历——本脚本不涉及任何路径操作；
# ③数据外泄——不读任何本地文件、不发起任何网络请求；
# ④越权——只读运行，不写/不删/不改任何文件；
# ⑤硬编码密钥——全文无任何密钥/API key/token；
# ⑥异常输入——空参/非法场景给出友好中文提示后 exit 0。
# 合规边界：只做「寓意/音韵」注解，不碰八字/三才五格类迷信内容。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
SCENE=$(printf '%s' "$ARGS" | sed -n 's/.*"scene"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$SCENE" ]; then
  SCENE=$(printf '%s' "$ARGS" | sed -e 's/^"//' -e 's/"$//' | tr -d '{}[]')
fi

SEED=$(date +%s)

# ---- 五组词根库：前缀字|寓意 与 后缀字|寓意（每库 8 词根）----
# 科技
PRE_TECH='云|轻盈广阔
极|顶尖突破
智|聪慧机敏
星|闪耀引路
蓝|冷静信任
光|明亮高效
跃|跃迁进取
简|克制简约'
SUF_TECH='枢|运转中心
核|核心动力
链|连接协作
界|开拓疆域
阁|沉淀思考
塔|登高望远
坊|动手创造
芽|生长起点'

# 文艺
PRE_ART='墨|书卷气息
清|通透干净
晚|温柔时分
浅|轻描淡写
山|沉静安稳
听|细腻感知
叙|娓娓道来
知|求知若渴'
SUF_ART='书|文字陪伴
吟|吟咏性情
笺|信札传情
野|自在天地
灯|夜色暖意
辞|文雅表达
时|光阴故事
路|脚步远方'

# 萌宠
PRE_PET='奶|软嫩入口
糯|黏糊可爱
圆|胖乎讨喜
毛|绒感满满
豆|小巧玲珑
桃|粉嫩讨喜
糖|甜在心头
麻|俏皮灵动'
SUF_PET='团|抱团圆
球|圆滚滚
叽|叽叽喳喳
包|揣怀里
仔|小家伙
糕|软乎乎
爪|小爪子
泡|圆泡泡'

# 游戏
PRE_GAME='斩|爽快利落
御|守护一方
破|突破重围
逐|追风前行
幻|天马行空
烬|余焰不息
骑|冲锋在前
影|迅疾无声'
SUF_GAME='刃|锋利在手
境|打开世界
歌|江湖传说
界|地图疆域
师|职业风范
塔|策略布阵
踪|来去如风
契|命运羁绊'

# 国风
PRE_CN='松|苍劲长青
鹤|飘逸出尘
江|开阔悠远
溪|灵动清澈
玉|温润含蓄
云|自在随心
清|高洁自持
墨|古韵书香'
SUF_CN='风|风骨气度
岚|山间薄雾
舟|行旅江湖
砚|文房清雅
辞|古典文采
澜|波澜不惊
笙|雅乐和鸣
阙|高处从容'

# ---- 场景命中（大小写兼容中英文关键词；未命中回退随机风格）----
STYLE_NAME=""
case "$SCENE" in
  *萌宠*|*宠物*|*猫*|*狗*|*兔*|*仓鼠*|pet*|*Pet*)
    PREV="$PRE_PET"; SUFV="$SUF_PET"; STYLE_NAME="萌宠" ;;
  *科技*|*互联网*|*软件*|*产品*|app*|*App*|tech*|*Tech*)
    PREV="$PRE_TECH"; SUFV="$SUF_TECH"; STYLE_NAME="科技" ;;
  *文艺*|*文创*|*书*|*咖啡*|艺术*|*Art*)
    PREV="$PRE_ART"; SUFV="$SUF_ART"; STYLE_NAME="文艺" ;;
  *游戏*|*电竞*|*桌游*|game*|*Game*)
    PREV="$PRE_GAME"; SUFV="$SUF_GAME"; STYLE_NAME="游戏" ;;
  *国风*|*古风*|*中式*|*中国*)
    PREV="$PRE_CN"; SUFV="$SUF_CN"; STYLE_NAME="国风" ;;
esac

if [ -z "$STYLE_NAME" ]; then
  case $((SEED % 5)) in
    0) PREV="$PRE_PET";   SUFV="$SUF_PET";   STYLE_NAME="萌宠" ;;
    1) PREV="$PRE_TECH";  SUFV="$SUF_TECH";  STYLE_NAME="科技" ;;
    2) PREV="$PRE_ART";   SUFV="$SUF_ART";   STYLE_NAME="文艺" ;;
    3) PREV="$PRE_GAME";  SUFV="$SUF_GAME";  STYLE_NAME="游戏" ;;
    4) PREV="$PRE_CN";    SUFV="$SUF_CN";    STYLE_NAME="国风" ;;
  esac
  NOTE="（未指定风格，随机抽中「${STYLE_NAME}」；也可以说科技/文艺/萌宠/游戏/国风换一种。）"
else
  NOTE=""
fi

NAMES=""
echo "【创意命名 · ${STYLE_NAME}风】五个候选名："
[ -n "$NOTE" ] && echo "$NOTE"
echo ""

i=1
while [ "$i" -le 5 ]; do
  # 步长 3/5 与词库 8 条互质错开，5 个候选两两不重复
  pi=$(( (SEED + i * 3) % 8 ))
  si=$(( (SEED + i * 5) % 8 ))
  pl=$(printf '%s\n' "$PREV" | sed -n "$((pi + 1))p")
  sl=$(printf '%s\n' "$SUFV" | sed -n "$((si + 1))p")
  pc=${pl%%|*}; pm=${pl#*|}
  sc=${sl%%|*}; sm=${sl#*|}
  echo "$i. 「$pc$sc」——「$pc」$pm，「$sc」$sm；两字组合，好念好记。"
  NAMES="$NAMES<li>「$pc$sc」——「$pc」$pm，「$sc」$sm</li>
"
  i=$((i + 1))
done

echo "---"
echo "说明：名字由本地词根库随机组合，仅供灵感；正式商用前请自行核查商标与域名，本脚本不做任何八字/五格测算。（未联网、未改动任何文件。）"
# v18.2 面板协议块：气泡候选名
echo "@qibai-panel:begin"
echo "mode=bubble"
echo "title=创意命名 · ${STYLE_NAME}风"
echo "html=<div><ul>"
printf '%s' "$NAMES"
echo "</ul><p><small>本地词根库随机组合，仅供灵感；商用前请核查商标与域名。</small></p></div>"
echo "@qibai-panel:end"