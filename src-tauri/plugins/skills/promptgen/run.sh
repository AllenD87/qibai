#!/bin/sh
# 绘画提示词生成技能（纯本地关键词映射模板，无网络、无文件读写副作用）。
# 六道安全闸：
# ①命令注入——参数仅做字符串匹配/字段抽取，绝不拼接进 shell 命令执行；
# ②路径遍历——本脚本不涉及任何路径操作；
# ③数据外泄——不读任何本地文件、不发起任何网络请求；
# ④越权——只读运行，不写/不删/不改任何文件；
# ⑤硬编码密钥——全文无任何密钥/API key/token；
# ⑥异常输入——空参/过长输入给出友好中文提示后 exit 0。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"

# 抽字段：优先 subject，兼容 description；抽不到降级为纯文本。
SUBJ=$(printf '%s' "$ARGS" | sed -n 's/.*"subject"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$SUBJ" ]; then
  SUBJ=$(printf '%s' "$ARGS" | sed -n 's/.*"description"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
fi
if [ -z "$SUBJ" ]; then
  SUBJ=$(printf '%s' "$ARGS" | sed -e 's/^"//' -e 's/"$//' | tr -d '{}[]')
fi

if [ -z "$SUBJ" ]; then
  echo "告诉我你想画什么，例如：「帮我写个绘画描述——一只橘猫，水彩风，黄昏光」。"
  echo "可以说清四件事：主体 / 风格 / 光照 / 镜头，我来拼成英文 prompt 给你。"
  echo "（本地模板组合，未联网、未改动任何文件。）"
  exit 0
fi

# 展示用截断（防超长刷屏）
SHOW=$(printf '%s' "$SUBJ" | cut -c1-40)

# ---- 主体关键词映射（中英对照；未命中给通用风景模板）----
SUBJ_EN="a peaceful little scene with warm tones"
SUBJ_ZH="一处温暖色调的小景"
case "$SUBJ" in
  *猫*)      SUBJ_EN="a fluffy orange cat sitting by a window"; SUBJ_ZH="一只趴在窗边的橘猫" ;;
  *狗*)      SUBJ_EN="a cheerful golden retriever on grass";     SUBJ_ZH="草地上欢快的金毛犬" ;;
  *兔*)      SUBJ_EN="a fluffy white rabbit";                   SUBJ_ZH="一只毛茸茸的白兔" ;;
  *女孩*)    SUBJ_EN="a young girl in casual clothes looking aside"; SUBJ_ZH="一个穿便装、望向一旁的女孩" ;;
  *男孩*)    SUBJ_EN="a teenage boy with a backpack";            SUBJ_ZH="一个背书包的少年" ;;
  *山*|*雪山*) SUBJ_EN="misty snow mountains with pine trees";  SUBJ_ZH="雾气缭绕的雪山松林" ;;
  *海*)      SUBJ_EN="a quiet beach with gentle waves";         SUBJ_ZH="浪声轻柔的海滩" ;;
  *花*)      SUBJ_EN="a field of wildflowers in spring";        SUBJ_ZH="春天的一片野花田" ;;
  *城市*)    SUBJ_EN="a quiet city street at dusk";             SUBJ_ZH="黄昏时安静的街道" ;;
  *森林*)    SUBJ_EN="a lush green forest with sunlight";       SUBJ_ZH="洒着阳光的葱郁森林" ;;
  *龙*)      SUBJ_EN="a gentle fantasy dragon resting on clouds"; SUBJ_ZH="一条安卧云端的温和幻想龙" ;;
  *机器人*)  SUBJ_EN="a cute retro-style little robot";         SUBJ_ZH="一个可爱的复古小机器人" ;;
  *房子*|*小屋*) SUBJ_EN="a cozy cottage with warm light";      SUBJ_ZH="一盏暖灯的乡间小屋" ;;
esac

# ---- 风格关键词映射 ----
STYLE_EN="digital illustration"
STYLE_ZH="数字插画"
case "$SUBJ" in
  *水彩*)     STYLE_EN="watercolor painting, soft washes";        STYLE_ZH="水彩画" ;;
  *油画*)     STYLE_EN="oil painting, rich brushstrokes";        STYLE_ZH="油画" ;;
  *赛博*)     STYLE_EN="cyberpunk style, neon reflections";      STYLE_ZH="赛博朋克风" ;;
  *国风*|*水墨*|*古风*) STYLE_EN="traditional Chinese ink wash painting"; STYLE_ZH="中国水墨风" ;;
  *动漫*|*动画*) STYLE_EN="soft anime style, clean line art";   STYLE_ZH="柔和动漫风" ;;
  *像素*)     STYLE_EN="pixel art, 16-bit style";                STYLE_ZH="像素画" ;;
  *写实*|*照片*) STYLE_EN="photorealistic, sharp details";      STYLE_ZH="写实照片风" ;;
  *手绘*)     STYLE_EN="hand-drawn sketch, pencil texture";      STYLE_ZH="手绘速写" ;;
esac

# ---- 光照关键词映射 ----
LIGHT_EN="soft diffused lighting"
LIGHT_ZH="柔和漫射光"
case "$SUBJ" in
  *黄昏*|*傍晚*) LIGHT_EN="warm golden hour lighting";  LIGHT_ZH="黄昏金色暖光" ;;
  *清晨*|*早晨*) LIGHT_EN="soft morning light";         LIGHT_ZH="清晨柔光" ;;
  *夜晚*|*晚上*) LIGHT_EN="moonlit night lighting";     LIGHT_ZH="夜晚月光" ;;
  *霓虹*)       LIGHT_EN="neon lighting";               LIGHT_ZH="霓虹灯光" ;;
  *逆光*)       LIGHT_EN="backlighting with soft rim light"; LIGHT_ZH="逆光轮廓光" ;;
  *阴天*)       LIGHT_EN="overcast soft light";         LIGHT_ZH="阴天柔光" ;;
esac

# ---- 镜头关键词映射 ----
SHOT_EN="cinematic wide shot"
SHOT_ZH="电影感全景"
case "$SUBJ" in
  *特写*)  SHOT_EN="close-up shot";              SHOT_ZH="特写镜头" ;;
  *微距*)  SHOT_EN="macro shot";                 SHOT_ZH="微距镜头" ;;
  *俯拍*)  SHOT_EN="top-down view";              SHOT_ZH="俯拍视角" ;;
  *平视*)  SHOT_EN="eye-level view";             SHOT_ZH="平视视角" ;;
  *远景*|*全景*) SHOT_EN="wide panoramic shot";   SHOT_ZH="宽幅全景" ;;
esac

echo "【绘画提示词】（你的描述：$SHOW）"
echo ""
echo "English prompt（可直接喂给 SD/MJ 类工具）："
echo "$SUBJ_EN, $STYLE_EN, $LIGHT_EN, $SHOT_EN, masterpiece, best quality, highly detailed, 8k"
echo ""
echo "Negative prompt（负面词）："
echo "lowres, bad anatomy, bad hands, text, error, missing fingers, extra digit, fewer digits, cropped, worst quality, jpeg artifacts, signature, watermark, username, blurry, nsfw, nude, naked, sexual, weapon, blood, gore"
echo ""
echo "中文对照：主体=$SUBJ_ZH ｜ 风格=$STYLE_ZH ｜ 光照=$LIGHT_ZH ｜ 镜头=$SHOT_ZH"
echo "---"
echo "小提示：想更贴合可以补充细节（颜色/季节/情绪氛围）。脚本未联网、未改动任何文件。"

# v18.2 面板协议块：window 提示词
echo "@qibai-panel:begin"
echo "mode=window"
echo "title=绘画提示词"
echo "html=<div><h3>绘画提示词 · $SHOW</h3><p><b>English prompt：</b></p><pre>$SUBJ_EN, $STYLE_EN, $LIGHT_EN, $SHOT_EN, masterpiece, best quality, highly detailed, 8k</pre><p><b>Negative prompt：</b></p><pre>lowres, bad anatomy, bad hands, text, error, missing fingers, extra digit, fewer digits, cropped, worst quality, jpeg artifacts, signature, watermark, username, blurry, nsfw, nude, naked, sexual, weapon, blood, gore</pre><p>中文对照：主体=$SUBJ_ZH ｜ 风格=$STYLE_ZH ｜ 光照=$LIGHT_ZH ｜ 镜头=$SHOT_ZH</p></div>"
echo "@qibai-panel:end"
