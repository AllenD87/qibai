/* ============================================================
 * 元气小小 Qibai — 语气 → 表情/动作 推断规则（v18.4）
 * 独立纯函数模块（无 DOM 依赖，node 可直接测试）。
 * pet.js 每句 pet-say 演出时调用：LLM 未带 emotion/action 时，
 * 按文本特征（关键词/标点/语气词/emoji）本地推断，
 * 保证「不同的话 → 不同表情 + 不同动作」。
 * 规则按优先级自上而下命中即返回。
 * ============================================================ */
(function (global) {
  'use strict';

  var EMOTIONS = ['smiling', 'happy', 'worried', 'sleepy', 'focused', 'sad', 'shy', 'surprised', 'angry', 'laugh', 'thinking', 'blush'];
  var ACTIONS = ['idle', 'wave', 'nod', 'shake', 'stretch', 'hands_on_hips', 'touch', 'pet', 'celebrate', 'bounce', 'tilt', 'wiggle', 'lean', 'point'];

  // v18.6 性格画像：同句话 → 不同形象不同表情/动作（同一情绪，动作与情绪烈度按性格调整）
  var PERSONALITY = {
    gudian: { // 古典温婉：情绪内敛、动作轻缓
      actionMap: { angry: 'shake', laugh: 'wave', sad: 'touch', surprised: 'nod', thinking: 'tilt', happy: 'wave', worried: 'lean', sleepy: 'nod', blush: 'touch', focused: 'nod', smiling: 'nod' },
      redirect: [ // 命中情绪但文本无强语气标记 → 收敛为重定向情绪
        { from: 'laugh', strong: /(笑死|太搞笑|哎哟喂|不行了|肚子疼)/, to: 'blush', action: 'touch' },
      ],
    },
    dushi: { // 都市知性：情绪克制、思考导向
      actionMap: { angry: 'shake', laugh: 'nod', sad: 'lean', surprised: 'tilt', thinking: 'point', happy: 'wave', worried: 'lean', sleepy: 'nod', blush: 'touch', focused: 'point', smiling: 'nod' },
      redirect: [
        { from: 'happy', strong: /(哈哈|耶|太棒|好开心|爱你|激动|加油|太好了|真棒)/, to: 'focused', action: 'nod' },
      ],
    },
    wenrou: { // 温柔恬静：安抚导向、动作轻柔
      actionMap: { angry: 'shake', laugh: 'wave', sad: 'pet', surprised: 'touch', thinking: 'tilt', happy: 'wave', worried: 'lean', sleepy: 'nod', blush: 'touch', focused: 'nod', smiling: 'nod' },
      redirect: [],
    },
    huopo: { // 活泼元气：情绪外放
      actionMap: { angry: 'bounce', laugh: 'bounce', sad: 'pet', surprised: 'stretch', thinking: 'tilt', happy: 'bounce', worried: 'lean', sleepy: 'nod', blush: 'touch', focused: 'nod', smiling: 'wave' },
      redirect: [],
    },
    reqing: { // 热情大方：感染力强、动作大开
      actionMap: { angry: 'point', laugh: 'celebrate', sad: 'pet', surprised: 'stretch', thinking: 'point', happy: 'celebrate', worried: 'lean', sleepy: 'nod', blush: 'touch', focused: 'point', smiling: 'wave' },
      redirect: [],
    },
  };

  function inferMood(text, persona) {
    var t = String(text || '');
    // 生气
    if (/(生气|气死|讨厌|不行|不许|烦人|恼火|可恶|居然敢)/.test(t)) {
      return applyPersonality({ emotion: 'angry', action: 'shake' }, text, persona);
    }
    // 大笑（强开心）
    if (/(哈哈哈哈哈|哈哈哈|笑死|太好笑|笑不活|哎哟喂)/.test(t)) {
      return applyPersonality({ emotion: 'laugh', action: 'bounce' }, text, persona);
    }
    // 难过 / 安慰
    if (/(难过|伤心|委屈|哭|呜呜|别怕|抱抱|心疼|想你了|好想)/.test(t)) {
      return applyPersonality({ emotion: 'sad', action: 'pet' }, text, persona);
    }
    // 惊讶
    if (/(哇|天哪|真的吗|不会吧|咦|没想到|居然|竟然|啊？！|！？|？！)/.test(t)) {
      return applyPersonality({ emotion: 'surprised', action: 'stretch' }, text, persona);
    }
    // 疑问 / 思考
    if (/(什么|怎么|为什么|如何|是不是|能不能|可以吗|怎么样|哪|呢$|吗$)/.test(t) || /[?？]/.test(t)) {
      return applyPersonality({ emotion: 'thinking', action: 'tilt' }, text, persona);
    }
    // 困倦 / 疲惫
    if (/(困|累|睡|晚安|休息|眯)/.test(t)) {
      return applyPersonality({ emotion: 'sleepy', action: 'nod' }, text, persona);
    }
    // 害羞 / 撒娇
    if (/(害羞|不好意思|脸红|羞|撒娇|扭捏|讨厌啦|人家)/.test(t)) {
      return applyPersonality({ emotion: 'blush', action: 'touch' }, text, persona);
    }
    // 开心 / 积极（关键词 + 感叹号 + 轻快语气词）
    if (/(开心|高兴|喜欢|爱你|哈哈|嘿嘿|加油|太好了|真棒|棒|耶|好呀|好嘞|早安|你好|嗨|啦|哦|嘿嘿)/.test(t) || /!/.test(t)) {
      return applyPersonality({ emotion: 'happy', action: 'wave' }, text, persona);
    }
    // 担忧 / 关切
    if (/(担心|怕|注意|小心|记得|别忘)/.test(t)) {
      return applyPersonality({ emotion: 'worried', action: 'lean' }, text, persona);
    }
    // 长句陈述 → 认真专注
    if (t.length > 40) {
      return applyPersonality({ emotion: 'focused', action: 'nod' }, text, persona);
    }
    // 默认：微笑 + 形象专属说话动作（action 为 default 时由播放器换 pickSpeakAction）
    return applyPersonality({ emotion: 'smiling', action: 'default' }, text, persona);
  }

  // v18.6 性格层：仅在显式传入 persona key 时生效；不传/未知形象 → 原规则不变
  function applyPersonality(base, text, persona) {
    var p = PERSONALITY[persona];
    if (!p) return base;
    // 1) 情绪收敛重定向：命中情绪弱表达时按性格重定向（如古典佳人把轻笑收敛为掩嘴笑）
    if (p.redirect && Array.isArray(p.redirect)) {
      for (var i = 0; i < p.redirect.length; i++) {
        var r = p.redirect[i];
        if (base.emotion === r.from && !r.strong.test(String(text || ''))) {
          return { emotion: r.to, action: r.action };
        }
      }
    }
    // 2) 动作映射：同情绪按形象换动作（生气：古典袖颤/活泼跺脚/热情前指）
    var a = p.actionMap[base.emotion];
    if (a) return { emotion: base.emotion, action: a };
    return base;
  }

  // 测试/扩展接口
  global.QibaiMood = {
    EMOTIONS: EMOTIONS,
    ACTIONS: ACTIONS,
    PERSONALITY: PERSONALITY,
    inferMood: inferMood,
  };
})(typeof window !== 'undefined' ? window : globalThis);

// node 直测兼容
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { EMOTIONS: ['smiling', 'happy', 'worried', 'sleepy', 'focused', 'sad', 'shy', 'surprised', 'angry', 'laugh', 'thinking', 'blush'], ACTIONS: ['idle', 'wave', 'nod', 'shake', 'stretch', 'hands_on_hips', 'touch', 'pet', 'celebrate', 'bounce', 'tilt', 'wiggle', 'lean', 'point'], PERSONALITY: (globalThis.QibaiMood || {}).PERSONALITY || {}, inferMood: function (t, persona) { return globalThis.QibaiMood ? globalThis.QibaiMood.inferMood(t, persona) : null; } };
}
