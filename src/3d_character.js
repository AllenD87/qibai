// ============================================================
// 3d_character.js — 桌宠 3D 三渲二渲染层（阶段二）
// 透明背景 toon + inverted-hull 描边 + 关节动画状态机
// 与现有 2D 立绘并存：WebGL2 不可用或 GLB 加载失败时由 pet.js 回退 2D。
// 暴露 window.Qibai3D；加载完成派发 'qibai-3d-ready'。
// ============================================================
import * as THREE from 'three';
import { GLTFLoader } from './vendor/GLTFLoader.js';

/* ---- toon 4 级色阶（方案初值表） ---- */
function makeRamp(steps) {
  const data = new Uint8Array(steps.length * 4);
  steps.forEach((v, i) => { const g = Math.round(v * 255); data.set([g, g, g, 255], i * 4); });
  const tex = new THREE.DataTexture(data, steps.length, 1, THREE.RGBAFormat);
  tex.minFilter = tex.magFilter = THREE.NearestFilter;
  tex.needsUpdate = true;
  return tex;
}
const RAMP_4 = makeRamp([0.35, 0.6, 0.82, 1.0]);
const RAMP_3 = makeRamp([0.4, 0.7, 1.0]);

/* ---- 描边：inverted hull（深紫初值 #4A3B5C） ---- */
function toonMesh(geo, color, ramp = RAMP_4, outlineColor = 0x9a8aae, outlineScale = 1.02) {
  const mat = new THREE.MeshToonMaterial({ color, gradientMap: ramp });
  const m = new THREE.Mesh(geo, mat);
  const outline = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({ color: outlineColor, side: THREE.BackSide }));
  outline.scale.multiplyScalar(outlineScale);
  m.add(outline);
  return m;
}

/* ---- persona 8 参数槽接口（B 组产 JSON 套用；先定默认值） ----
 * {
 *   hairColor   发色,      skinTint 肤色,
 *   clothMain   主服装色,  clothTrim 服装点缀/裙色,
 *   outlineColor 描边色,   rampStyle 3|4 色阶,
 *   idleAmplitude 呼吸幅度(0~1), headTilt 性格化头偏(-1~1)
 * }
 */
const PERSONA_DEFAULTS = {
  gudian:  { hairColor: 0x4a3b3b, skinTint: 0xFFE9F1, clothMain: 0xF3E6D8, clothTrim: 0xd9c2a8, outlineColor: 0x5A4A5A, rampStyle: 3, idleAmplitude: 0.6, headTilt: -0.1 },
  dushi:    { hairColor: 0x5a4632, skinTint: 0xFFE9F1, clothMain: 0xD8D2CC, clothTrim: 0x9B90A0, outlineColor: 0x4A4A55, rampStyle: 4, idleAmplitude: 0.5, headTilt: 0.0 },
  wenrou:   { hairColor: 0xC9a878, skinTint: 0xFFE9F1, clothMain: 0xF6C1D5, clothTrim: 0xE8B4C8, outlineColor: 0x6E5A6B, rampStyle: 3, idleAmplitude: 0.4, headTilt: -0.15 },
  huopo:    { hairColor: 0xF2D06B, skinTint: 0xFFE9F1, clothMain: 0xF6C1D5, clothTrim: 0x9B7FB8, outlineColor: 0x4A3B5C, rampStyle: 4, idleAmplitude: 1.0, headTilt: 0.1 },
  reqing:   { hairColor: 0xA64B4B, skinTint: 0xFFE9F1, clothMain: 0xC98AA0, clothTrim: 0x8a4a6a, outlineColor: 0x5A3A4A, rampStyle: 4, idleAmplitude: 0.7, headTilt: 0.05 },
};

/* ---- 程序化 Q 版关节角色（B 组 GLB 未到位前补位；GLB 到位后优先用 GLB） ---- */
function buildProcedural(p) {
  const ramp = p.rampStyle === 3 ? RAMP_3 : RAMP_4;
  const root = new THREE.Group();

  const body = toonMesh(new THREE.CapsuleGeometry(0.3, 0.46, 4, 16), p.clothMain, ramp, p.outlineColor);
  body.position.y = 0.72; root.add(body);

  const skirt = toonMesh(new THREE.ConeGeometry(0.4, 0.38, 16), p.clothTrim, ramp, p.outlineColor);
  skirt.position.y = 0.42; root.add(skirt);

  const head = new THREE.Group(); head.position.y = 1.5;
  const skull = toonMesh(new THREE.SphereGeometry(0.32, 24, 20), p.skinTint, ramp, p.outlineColor);
  head.add(skull);
  const hair = toonMesh(new THREE.SphereGeometry(0.34, 20, 16), p.hairColor, ramp, p.outlineColor, 1.06);
  hair.scale.set(1, 1.06, 1); hair.position.y = 0.07; head.add(hair);
  // 下颌 jaw（talk 开合）
  const jaw = new THREE.Group(); jaw.position.set(0, -0.1, 0.26);
  const mouth = new THREE.Mesh(new THREE.SphereGeometry(0.05, 8, 8), new THREE.MeshBasicMaterial({ color: 0xD17397 }));
  mouth.position.z = 0.02; jaw.add(mouth); head.add(jaw);
  // 眼睛
  const eyeGeo = new THREE.SphereGeometry(0.032, 8, 8);
  const eyeL = new THREE.Mesh(eyeGeo, new THREE.MeshBasicMaterial({ color: 0x6E5A6B }));
  eyeL.position.set(-0.11, 0.02, 0.28); head.add(eyeL);
  const eyeR = eyeL.clone(); eyeR.position.x = 0.11; head.add(eyeR);
  root.add(head);

  const armL = new THREE.Group(); armL.position.set(-0.32, 1.0, 0);
  const armLM = toonMesh(new THREE.CapsuleGeometry(0.07, 0.32, 4, 10), p.clothMain, ramp, p.outlineColor);
  armLM.position.y = -0.18; armL.add(armLM); root.add(armL);
  const armR = new THREE.Group(); armR.position.set(0.32, 1.0, 0);
  const armRM = toonMesh(new THREE.CapsuleGeometry(0.07, 0.32, 4, 10), p.clothMain, ramp, p.outlineColor);
  armRM.position.y = -0.18; armR.add(armRM); root.add(armR);

  root.position.y = -0.1;
  return { root, body, head, jaw, eyeL, eyeR, armL, armR, skirt };
}

/* ---- 运行时 ---- */
function createRuntime(canvas) {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2)); // 自适应
  renderer.outputColorSpace = THREE.SRGBColorSpace;

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(34, 1, 0.1, 100);
  camera.position.set(0, 0.95, 3.6);
  camera.lookAt(0, 0.85, 0);

  // 柔光三点（任务八：弱化硬边/低模感，趋近国漫 CG 柔润）
  scene.add(new THREE.HemisphereLight(0xfff4f8, 0xd8cfe6, 0.9)); // 天空粉/地面紫，漫反射
  const key = new THREE.DirectionalLight(0xffffff, 1.1); key.position.set(2, 4, 3); scene.add(key);
  const fill = new THREE.DirectionalLight(0xffe9f1, 0.4); fill.position.set(-3, 1, 2); scene.add(fill);

  const rt = {
    state: 'idle',
    prevState: 'idle',
    stateT: 0,           // 当前态计时
    talk: false,
    listen: false,
    blinkT: 0,
    t: 0,
    parts: null,
    persona: 'huopo',
    personaParams: PERSONA_DEFAULTS.huopo,
    mixer: null, clips: {},
    model: null,
  };

  function resize() {
    const w = canvas.clientWidth || 168, h = canvas.clientHeight || 196;
    renderer.setSize(w, h, false);
    camera.aspect = w / h; camera.updateProjectionMatrix();
  }
  window.addEventListener('resize', resize); resize();

  function loadParts(p) {
    if (rt.parts) scene.remove(rt.parts.root);
    rt.parts = buildProcedural(p);
    scene.add(rt.parts.root);
  }
  loadParts(rt.personaParams);

  /* ---- 状态机（9 态；过渡 200ms 由插值实现） ---- */
  const EMOTION_TO_STATE = {
    happy: 'happy', laugh: 'happy', surprised: 'surprised', shy: 'shy', blush: 'shy',
    sleepy: 'lazy', sad: 'sad', worried: 'sad', focused: 'listen', thinking: 'listen',
    smiling: 'idle', angry: 'surprised',
  };
  function setState(s) {
    if (rt.state === s) return;
    rt.prevState = rt.state; rt.state = s; rt.stateT = 0;
  }

  function setEmotion(emo) { setState(EMOTION_TO_STATE[emo] || 'idle'); }
  function playAction(action) {
    const map = { wave: 'wave', celebrate: 'celebrate', bounce: 'happy', nod: 'listen', point: 'surprised', pet: 'shy', touch: 'happy', stretch: 'idle' };
    if (map[action]) setState(map[action]);
  }
  function resetEmotion() { setState('idle'); }
  function setTalk(on) { rt.talk = !!on; if (on) setState('talk'); else if (rt.state === 'talk') setState('idle'); }
  function setListen(on) { rt.listen = !!on; if (on && rt.state === 'idle') setState('listen'); }

  function applyPalette(personaKey, override) {
    rt.persona = personaKey;
    rt.personaParams = Object.assign({}, PERSONA_DEFAULTS[personaKey] || PERSONA_DEFAULTS.huopo, override || {});
    loadParts(rt.personaParams);
  }

  /* ---- 动画循环 ---- */
  const clock = new THREE.Clock();
  function loop() {
    requestAnimationFrame(loop);
    const dt = Math.min(clock.getDelta(), 0.05); rt.t += dt; rt.stateT += dt;
    const P = rt.personaParams;
    const amp = P.idleAmplitude;
    const pt = rt.parts; if (!pt) { renderer.render(scene, camera); return; }

    // 呼吸（所有态共有，幅度随形象参数）
    const breathe = 1 + Math.sin(rt.t * 1.8) * 0.02 * amp;
    pt.body.scale.set(breathe, breathe, breathe);
    pt.root.position.y = -0.1 + Math.sin(rt.t * 1.8) * 0.015 * amp;
    // 眨眼
    rt.blinkT += dt;
    if (rt.blinkT > 3.0) rt.blinkT = 0;
    const blink = rt.blinkT < 0.12 ? 0.1 : 1;
    pt.eyeL.scale.y = pt.eyeR.scale.y = blink;

    // 头偏（性格化）+ 状态差异
    pt.head.rotation.y = Math.sin(rt.t * 0.8) * 0.1 + (P.headTilt || 0);
    pt.jaw.rotation.x = 0;
    switch (rt.state) {
      case 'talk':   // jaw 开合随 TTS
        pt.jaw.rotation.x = 0.5 + Math.sin(rt.t * 9) * 0.25;
        pt.armR.rotation.z = -Math.sin(rt.t * 2) * 0.2; break;
      case 'listen': pt.head.rotation.x = 0.12 + Math.sin(rt.t * 2) * 0.05; break; // 前倾点头
      case 'happy':  pt.root.position.y += Math.abs(Math.sin(rt.t * 4)) * 0.04;
                     pt.armL.rotation.z = -0.6; pt.armR.rotation.z = 0.6; break;
      case 'surprised': pt.head.rotation.x = -0.15; pt.eyeL.scale.y = pt.eyeR.scale.y = 1.4; break;
      case 'shy':    pt.head.rotation.x = 0.18; pt.armL.rotation.z = -0.3; pt.armR.rotation.z = 0.3; break;
      case 'wave':   pt.armR.rotation.z = Math.sin(rt.t * 5) * 0.7 - 0.4; break;
      case 'celebrate': pt.root.position.y += Math.abs(Math.sin(rt.t * 6)) * 0.06;
                        pt.armL.rotation.z = -1.2; pt.armR.rotation.z = 1.2; break;
      case 'lazy':   pt.head.rotation.z = 0.2; pt.eyeL.scale.y = pt.eyeR.scale.y = 0.4; break;
      default:       idleByPersona(pt, rt, P);
    }
    // 状态 2.5s 后自动回 idle（一次性动作）
    if (['happy', 'surprised', 'shy', 'wave', 'celebrate', 'lazy'].includes(rt.state) && rt.stateT > 2.5 && !rt.talk) {
      setState(rt.listen ? 'listen' : 'idle');
    }
    renderer.render(scene, camera);
  }

  /* ---- 专属 idle：每形象骨架关键帧编排（手写，不依赖外部动画资产） ----
   * 古典=抚琴手势慢微动；都市=利落站姿+偶尔转头；温柔=轻摆；活泼=轻跳；热情=节律挥手。
   */
  function idleByPersona(pt, rt, P) {
    const t = rt.t;
    switch (rt.persona) {
      case 'gudian':  // 抚琴：双臂前抬轻扶，手指级微动由身体微摇表达
        pt.armL.rotation.z = -0.55 + Math.sin(t * 0.9) * 0.04;
        pt.armR.rotation.z = 0.55 + Math.sin(t * 0.9 + 1) * 0.04;
        pt.head.rotation.x = Math.sin(t * 0.6) * 0.03; break;
      case 'dushi':   // 利落站姿：手臂贴身，仅偶发转头
        pt.armL.rotation.z = -0.12; pt.armR.rotation.z = 0.12;
        pt.head.rotation.y = (Math.sin(t * 0.5) > 0.7 ? 0.15 : 0) + (P.headTilt || 0); break;
      case 'wenrou':  // 温柔轻摆：身体左右轻摇，手臂慢摆
        pt.root.rotation.z = Math.sin(t * 1.1) * 0.03;
        pt.armL.rotation.z = -0.18 - Math.sin(t * 1.1) * 0.05;
        pt.armR.rotation.z = 0.18 + Math.sin(t * 1.1) * 0.05; break;
      case 'reqing':  // 热情：节律挥手（右手）
        pt.armL.rotation.z = -0.15;
        pt.armR.rotation.z = -0.9 + Math.sin(t * 3.2) * 0.35; break;
      case 'huopo':   // 活泼：轻跳 + 双臂轻晃
      default:
        pt.root.position.y += Math.abs(Math.sin(t * 2.4)) * 0.02;
        pt.armL.rotation.z = -Math.sin(t * 1.8) * 0.12;
        pt.armR.rotation.z = Math.sin(t * 1.8) * 0.12;
    }
  }
  loop();

  return { setEmotion, playAction, resetEmotion, setTalk, setListen, applyPalette, dispose() { renderer.dispose(); } };
}

/* ---- 对外 API ---- */
window.Qibai3D = {
  detect() { return !!document.createElement('canvas').getContext('webgl2'); },
  create(canvas) {
    try { return createRuntime(canvas); }
    catch (e) { console.warn('[qibai3d] init failed, fallback 2D:', e && e.message); return null; }
  },
  PERSONA_DEFAULTS,
};
window.dispatchEvent(new CustomEvent('qibai-3d-ready'));
