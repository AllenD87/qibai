#!/bin/sh
# 星座运势技能（纯本地规则生成，无网络、无文件读写副作用）。
# 安全设计：无硬编码密钥、无外发数据、无系统命令；内容全龄合规、娱乐性质、不绝对化预测。
# 确定性随机：以「年中第几天 + 星座序号」做种子，同一天同一星座结果稳定。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
SIGN=$(printf '%s' "$ARGS" | sed -n 's/.*"sign"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$SIGN" ]; then
  SIGN=$(printf '%s' "$ARGS" | sed -e 's/^{"sign"://' -e 's/}$//' -e 's/^"//' -e 's/"$//' | tr -d '{}[]"')
fi

SIGNS="白羊座 金牛座 双子座 巨蟹座 狮子座 处女座 天秤座 天蝎座 射手座 摩羯座 水瓶座 双鱼座"
IDX=0
MATCHED=no
for s in $SIGNS; do
  IDX=$((IDX + 1))
  if [ "$s" = "$SIGN" ]; then MATCHED=yes; break; fi
done
if [ "$MATCHED" != yes ]; then
  echo "请告诉我是哪个星座，例如：使用星座运势，看看白羊座今天怎么样。支持的星座：$SIGNS"
  exit 0
fi

DOY=$(date +%j | sed 's/^0*//')
[ -z "$DOY" ] && DOY=1
SEED=$((DOY * 7 + IDX * 13))

# 总体运势评分 40-90
SCORE=$((40 + SEED % 51))

# 幸运色（按种子取）
case $((SEED % 8)) in
  0) COLOR="暖橙色" ;; 1) COLOR="天蓝色" ;; 2) COLOR="草绿色" ;; 3) COLOR="淡紫色" ;;
  4) COLOR="米黄色" ;; 5) COLOR="珊瑚红" ;; 6) COLOR="湖水青" ;; 7) COLOR="樱花粉" ;;
esac

# 幸运数字
LUCKY=$((1 + SEED % 9))

# 关键词
case $((SEED % 6)) in
  0) KEY="稳中有进" ;; 1) KEY="灵感闪现" ;; 2) KEY="贵人相助" ;;
  3) KEY="小有波折" ;; 4) KEY="渐入佳境" ;; 5) KEY="蓄势待发" ;;
esac

# 事业/爱情/健康三栏建议（正向、通用、全龄合规）
case $((SEED % 5)) in
  0) CAREER="适合推进手头积压的任务，把大目标拆成小步骤，完成一项就给自己一点肯定。" ;;
  1) CAREER="适合沟通与协作，主动同步进度会带来意外帮助；避免同时接过多新任务。" ;;
  2) CAREER="适合专注与复盘，整理手头资料，优化工作方法，效率会比想象中高。" ;;
  3) CAREER="适合尝试新方法，换个思路处理旧问题会有突破；小步验证，别一次押太多。" ;;
  4) CAREER="适合收尾与交接，把未完成的事理清优先级，先做最重要的一件。" ;;
esac
case $((SEED % 5)) in
  0) LOVE="和亲近的人多说一句关心的话，平淡日常里的小温暖最珍贵。" ;;
  1) LOVE="适合坦诚表达想法，把期待说清楚，误会自然会少很多。" ;;
  2) LOVE="适合一起做件简单的小事，比如散步、做饭，陪伴本身就很治愈。" ;;
  3) LOVE="给在乎的人留点时间，倾听比给建议更重要。" ;;
  4) LOVE="适合经营小惊喜，一句问候或一个小礼物，能让关系升温。" ;;
esac
case $((SEED % 5)) in
  0) HEALTH="记得喝水与拉伸，久坐之后站起来活动五分钟。" ;;
  1) HEALTH="今天适合早点休息，睡饱觉就是最好的充电。" ;;
  2) HEALTH="注意用眼与肩颈，工作间隙远眺放松一下。" ;;
  3) HEALTH="适合轻度运动，散步或快走会让心情更明亮。" ;;
  4) HEALTH="三餐规律一些，少熬夜，身体状态会稳稳在线。" ;;
esac

echo "【$SIGN 今日运势】（$DOY 日 · 娱乐参考）"
echo "总体：$SCORE 分（满分 100）｜今日关键词：$KEY"
echo "幸运色：$COLOR ｜幸运数字：$LUCKY"
echo "事业：$CAREER"
echo "爱情：$LOVE"
echo "健康：$HEALTH"
echo "---"
echo "温馨提示：运势仅供娱乐，请以积极心态面对每一天，你的努力才是最重要的运势。"

# v18.1 面板协议块：bubble 气泡卡片展示运势
echo "@qibai-panel:begin"
echo "mode=bubble"
echo "title=$SIGN 今日运势"
echo "html=<div><b>$SIGN 今日运势</b> · $SCORE 分 ｜ 关键词：$KEY<br/>幸运色：$COLOR ｜ 幸运数字：$LUCKY<br/><br/>事业：$CAREER<br/>爱情：$LOVE<br/>健康：$HEALTH</div>"
echo "@qibai-panel:end"
