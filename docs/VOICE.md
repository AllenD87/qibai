# 元气小小 Qibai — 语音规格与调用文档（迭代②：TTS 发声陪伴）

> 本文是迭代②（TTS 发声陪伴 + STT 可选增强）的正式交付文档，面向接入方 / 真机验证者。
> 内容由冻结规格 `_VOICE_ITER2_SPEC.md` 整理而成，并补充三平台命令映射、环境变量、云端端点与隐私边界、STT 降级与真机验证清单。
> 工程根：本仓库根目录（下文简称 QIBAI）。
> IPC 命令形状另见 [`IPC_CONTRACT.md`](IPC_CONTRACT.md) §一·伍补。

---

## 1. 总体能力

- **对话回复可朗读**（可开关）；**提醒**（久坐 / 喝水 / 护眼）语音播报；**升级 / 彩蛋语音**与**守护开场语语音**为可选增强（仅在「朗读范围 = all」时触发）。
- **TTS 引擎可切换**：默认本地系统 TTS（Windows / macOS / Linux 平台命令），可配置云端 TTS（豆包 / 火山）。云端 key 由用户运行时提供，代码零硬编码。
- **引擎选择与参数持久化**进 `settings.json` 的 `voice` 块。
- **优雅降级**：无 key / 引擎不可用时自动降级（云端无 key → 本地 → 静音提示），绝不报错崩溃。
- **STT（可选增强，默认关）**：麦克风 → ASR；无 key 或设备不可用时明确提示并隐藏入口，不报错不打扰。
- **隐私硬约束**：语音数据本地处理；云端 TTS/ASR 仅发送当前朗读文本片段（不含对话历史）；key 仅存本地 `settings.json`，不上传、不打日志。
- **合规全龄无擦边**：朗读文本全部来自既有已审内容（提醒文案 / 人设回复 / 升级台词），不新增任何越界内容。

---

## 2. `settings.json` 的 `voice` 块（字段冻结）

```jsonc
"voice": {
  "enabled": true,            // 总开关（默认 true；无可用引擎时自动静音，不发声）
  "engine": "auto",           // "auto" | "local" | "cloud"（默认 auto）
  "volume": 80,               // 0-100，默认 80，落盘前 clamp
  "rate": 1.0,                // 0.5-2.0，默认 1.0，落盘前 clamp
  "scope": "all",             // "all" | "reminders" | "chat"（默认 all）
  "cloud_provider": "doubao", // "doubao" | "volcano"（默认 doubao）
  "cloud_voice": "",          // 音色 ID，留空用引擎默认
  "cloud_api_key": "",        // 云端 TTS key；空则运行时读环境变量（见 §5）
  "stt_enabled": false,       // STT 总开关（默认 false）
  "stt_provider": "doubao",   // "doubao" | "volcano"
  "stt_api_key": ""           // STT key；空则读环境变量（见 §5）
}
```

- 旧 `settings.json` 无 `voice` 块时**平滑加载默认值**（serde `default`，同 guard 先例），不崩溃、不整体回退。
- `cloud_api_key` / `stt_api_key` 为空字符串时 `skip_serializing_if`（同 `llm.api_key` 先例），落盘不写空 key。
- 保存设置时 `voice` 块**整表保存**（前端 collect 必须带全 voice 字段，防止保存覆盖丢失）。
- Rust 侧常量边界：`VOLUME_MIN=0, VOLUME_MAX=100, RATE_MIN=0.5, RATE_MAX=2.0`；`VoiceSettings::sanitize()` 在落盘前 clamp。

---

## 3. 引擎选择与朗读范围规则

### 3.1 引擎决策 `resolve_engine(settings, local_available)`

返回 `{ engine: Local | Cloud | Mute, reason: &'static str }`，规则冻结：

| settings.engine | 条件 | 决策 | reason |
|---|---|---|---|
| 任意 | `enabled=false` | **Mute** | 总开关已关闭 |
| `auto` | 有云端 key | **Cloud** | 云端已配置 |
| `auto` | 无 key 且本地可用 | **Local** | 本地系统 TTS |
| `auto` | 无 key 且本地不可用 | **Mute** | 无可用引擎（未配置云端 key 且本地无 TTS） |
| `local` | 本地可用 | **Local** | 本地系统 TTS |
| `local` | 本地不可用 | **Mute** | 本地 TTS 不可用 |
| `cloud` | 有云端 key | **Cloud** | 云端已配置 |
| `cloud` | 无 key 且本地可用 | **Local** | 云端无 key，降级本地 TTS |
| `cloud` | 无 key 且本地不可用 | **Mute** | 云端无 key 且本地无 TTS |

### 3.2 朗读范围 `scope_allows(scope, kind)`

`kind ∈ chat | reminder | level_up | guard`：

| scope | chat（对话回复） | reminder（久坐/喝水/护眼播报） | level_up（升级彩蛋台词） | guard（守护开场语） |
|---|---|---|---|---|
| `all` | ✅ | ✅ | ✅ | ✅ |
| `reminders` | ❌ | ✅ | ❌ | ❌ |
| `chat` | ✅ | ❌ | ❌ | ❌ |

- `tts_decide(kind, text)` 在以下情形返回 `None`（不朗读）：总开关关 / scope 不覆盖 / text 空白。
- 升级台词与守护开场语**仅在 `all` scope** 下朗读（可选增强）。

---

## 4. 三平台本地 TTS 命令表与音量 / 语速映射

本地命令由 `qibai-core/src/tts.rs` 的 `build_local_tts_cmd(platform, text, rate, volume)` **纯函数构造**（不加 `cfg`，三平台均可单测）；
探测 `detect_local_tts_available()` 用 `command -v` 依次查找对应二进制。

### 4.1 Linux

| 探测顺序 | 命令 | 命令形状 |
|---|---|---|
| 1 | `espeak-ng` | `["espeak-ng", "-v", "zh", "-s", "<wpm>", "-a", "<amp>", "<text>"]` |
| 2 | `espeak` | `["espeak", "-v", "zh", "-s", "<wpm>", "-a", "<amp>", "<text>"]` |
| 3 | `spd-say` | `["spd-say", "<text>"]`（简单回退，不映射音量语速） |

- `wpm = clamp(round(rate * 160), 80, 320)`（每分钟字数）；
- `amp = clamp(volume * 2, 0, 200)`（espeak 音量 0–200，默认 100）。

### 4.2 macOS

| 探测 | 命令 | 命令形状 |
|---|---|---|
| 1 | `say` | `["say", "-r", "<wpm>", "<text>"]` |

- `wpm = clamp(round(rate * 175), 80, 350)`；
- macOS `say` 不接受命令行音量参数，音量映射在真机以系统输出音量为准（本迭代不做逐次 `-v` 音色切换）。

### 4.3 Windows

| 探测 | 命令 | 命令形状 |
|---|---|---|
| 1 | `powershell`（存在即可，SAPI 内建） | `["powershell", "-NoProfile", "-Command", "Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Rate=<r>; $s.Volume=<v>; $s.Speak('<text>')"]` |

- `r = clamp(round((rate - 1.0) * 10), -10, 10)`（SAPI 语速 -10…+10）；
- `v = volume`（SAPI 音量 0–100）；
- text 中单引号 `'` 转义为 `''`。

### 4.4 映射速查

| 配置值 | espeak-ng/espeak | `say` (macOS) | PowerShell SAPI (Windows) |
|---|---|---|---|
| `volume=80`（默认） | `amp=160` | —（系统输出音量） | `-Volume=80` |
| `rate=1.0`（默认） | `wpm=160` | `wpm=175` | `-Rate=0` |
| `rate=0.5` | `wpm=80`（下限 clamp） | `wpm=88→80`（下限 clamp 到 80） | `-Rate=-5` |
| `rate=2.0` | `wpm=320`（上限 clamp） | `wpm=350`（上限 clamp） | `-Rate=10`（上限 clamp） |

### 4.5 长文本截断

本地朗读文本 **>200 字符截断加「……」**（`clip_text`，防命令行超限）；**云端不截断**。

---

## 5. 环境变量说明

| 环境变量 | 用途 | 优先级 |
|---|---|---|
| `QIBAI_TTS_KEY` | 云端 TTS key | `voice.cloud_api_key`（设置值）**→ `QIBAI_TTS_KEY` → `DOUBAO_API_KEY`** |
| `QIBAI_STT_KEY` | 云端 STT key | `voice.stt_api_key`（设置值）**→ `QIBAI_STT_KEY` → `DOUBAO_API_KEY`** |

- key 解析函数：`VoiceSettings::cloud_key()` / `stt_key()`，设置值非空时用设置值，否则按上表回退环境变量，均空则 `None`。
- **复用约定**：`QIBAI_TTS_KEY` / `QIBAI_STT_KEY` 缺省时回退到通用的 `DOUBAO_API_KEY`，与 LLM 同一个 key 源可直接复用，无需重复配置。
- key 仅进请求头（云端）/ 本地设置文件，**绝不进请求 body、不打日志**（隐私单测点：构造请求后断言 body 不含 key 字符串）。

---

## 6. 云端端点说明与隐私边界

### 6.1 云端 TTS 请求（`build_cloud_request`）

- **URL**：`https://openspeech.bytedance.com/api/v1/tts`（火山引擎语音开放平台）。
- **Headers**：`X-Access-Token: <key>`、`Content-Type: application/json`。
- **Body**：

```jsonc
{
  "text": "<当前朗读的文本片段>",
  "speaker": "<cloud_voice，留空则缺省>",
  "audio": { "format": "wav", "sample_rate": 24000, "encoding": "raw" }
}
```

- key 空 → `Err(TtsError::MissingCloudKey)`（结构化错误，不 panic）；
- 合成成功返回 WAV bytes，前端以 `data:audio/wav;base64,...` 经 `<audio>` 播放；
- 合成失败 → **自动降级**：本地可用则走 `run_local_tts`（返回 `degraded:true`），否则 `Mute` 并提示原因。

### 6.2 隐私边界

| 数据 | 是否离开本机 | 说明 |
|---|---|---|
| 全部对话历史 | ❌ 不上传 | 云端 TTS **仅发送当前这一段朗读文本**，不携带上下文 |
| 本地 TTS 朗读 | ❌ 不出本机 | 本地 espeak/say/SAPI 进程内合成 |
| key | ❌ 不上传 | 仅存本地 `settings.json` / 环境变量，云端请求只放请求头 |
| 设置 / 亲密度 / 社交数据 | ❌ 不上传 | 全部本地存储（见 `IPC_CONTRACT.md` §四） |
| STT 音频 | 仅在 `stt_enabled=true` 且配 key 时上传该次录音片段 | STT 默认关闭；无 key 时 `stt_status.available=false`，前端隐藏入口 |

---

## 7. STT 优雅降级说明

`stt_status()` 返回：

```jsonc
{
  "enabled": false,          // = settings.voice.stt_enabled
  "provider": "doubao",
  "key_ready": false,       // stt_api_key 或 QIBAI_STT_KEY/DOUBAO_API_KEY 任一非空
  "device_available": false, // Linux 检查 /dev/snd 存在；mac/windows 默认 true（真机项）
  "available": false,       // = enabled && key_ready && device_available
  "reason": "STT：未配置 STT Key（可通过环境变量 QIBAI_STT_KEY 提供）"
}
```

- **降级规则**：`available=false` 时前端在设置页 STT 区显示 `reason` 中文说明、隐藏 / 禁用麦克风入口，**不报错、不打扰**；
- 沙箱实测：无 `QIBAI_STT_KEY` / `DOUBAO_API_KEY` 且无麦克风设备 → `available=false`，提示文案即上图（截图 `artifacts/voice-stt-degraded.png`）；
- STT 本体（麦克风采集 → ASR）为**后续迭代占位能力**，本迭代仅交付「状态探测 + 优雅降级提示」链路。

---

## 8. 自动播报接线点（src-tauri/main.rs）

| 触发点 | kind | 文本来源 |
|---|---|---|
| 用户发消息后 assistant 回复 | `chat` | 回复正文（前端 `tts_speak` 触发） |
| 桌宠气泡喇叭按钮 `#btnSpeak` | `chat` | 当前气泡文本（`lastSpokenText`） |
| 提醒 tick（久坐/喝水/护眼） | `reminder` | `ReminderEvent.message`（系统通知后朗读） |
| 亲密度升级（`intimacy_event` 且 `leveled_up=true`） | `level_up` | `"亲密度提升啦！现在是 L{level} {level_name}，要继续一起走下去呀～"`（仅 all scope） |
| 全屏守护开场（`start_guard_performance`，`perform_fullscreen`） | `guard` | `start.message`（仅 all scope） |

朗读成功（非 Mute）后 emit `qibai://tts-speak {kind, engine, volume, rate, state:"speaking"}`，前端据此刻画「正在朗读」指示与桌宠气泡 speaking 脉冲。

---

## 9. 沙箱受限项与替代验证（如实声明）

| 受限项 | 沙箱实测 | 替代验证 |
|---|---|---|
| 本地 TTS 命令 | 沙箱无 `espeak`/`espeak-ng`/`say`/`spd-say`/`powershell`，`detect_local_tts_available()` 返回 false | 单测覆盖命令构造（三平台 wpm/amp/r/volume 映射 + 引号转义）与无引擎降级返回 `Mute`；`piper-tts` 安装因网络失败 |
| `src-tauri` 编译 | 缺 `gdk-3.0`/`webkit2gtk-4.1` 系系统库，`cargo check` 仅因此失败 | 代码接线 + 静态审查为准；真机编译为验证项 |
| 云端 TTS/STT 端到端 | 无 key 未实际合成 | 单测覆盖请求构造（URL/headers/body、key 只进 header、空 key `MissingCloudKey`）与降级路径 |

---

## 10. 真机验证清单

> 以下各项在沙箱无法完成，需在装齐系统依赖、有扬声器 / 麦克风的真机上逐项打勾：

- [ ] **Linux 本地 TTS**：`apt install espeak-ng`（或 `speech-dispatcher`）后，`tts_speak{kind:"chat"}` 实际出声，音量 / 语速随设置生效；
- [ ] **macOS 本地 TTS**：`say` 默认存在，朗读中文不报错（必要时 `say -v ?` 查看中文音色）；
- [ ] **Windows 本地 TTS**：PowerShell SAPI `SpeechSynthesizer` 实际出声，中文音色正常；
- [ ] **云端 TTS**：配置 `QIBAI_TTS_KEY`（或设置页 key）后 engine=cloud，朗读返回 `audio_data_url` 并由 `<audio>` 播放；断网 / key 失效时自动降级本地或 Mute；
- [ ] **STT 麦克风**：`stt_enabled=true` + 配 `QIBAI_STT_KEY` + 系统授权麦克风后，`stt_status.available=true`；拒绝授权 / 拔麦时 `available=false` 且提示 reason；
- [ ] **真机编译**：装齐 `docs/BUILD_NOTE.md` 所列依赖后 `cd src-tauri && cargo build --release` 通过，`tts_speak` / `tts_engine_status` / `stt_status` 三命令注册生效；
- [ ] **提醒播报**：触发一次久坐提醒，系统通知与语音播报同帧出现；
- [ ] **隐私核验**：抓包确认云端请求仅含当前朗读文本片段、key 在 header、无对话历史外泄。

---

## 11. 相关文件与证据

| 类别 | 位置 |
|---|---|
| Rust 核心 | `qibai-core/src/tts.rs`（配置 / 引擎选择 / 平台命令构造 / 云端请求 / 17 个新单测）、`settings.rs`（voice 块）、`engine.rs`（`tts_decide` / `tts_status` / `stt_status`） |
| Tauri 接线 | `src-tauri/src/main.rs`（`tts_speak` / `tts_engine_status` / `stt_status` 命令 + 提醒 / 升级 / 守护开场播报） |
| 前端 | `src/console.*`（设置页「语音」card）、`src/pet.*`（喇叭按钮 `#btnSpeak` + speaking 脉冲）、`src/tauri-bridge.js`（mock 层 `speechSynthesis` 降级）、`src/smoke.html`（朗读一句 / 语音状态控制） |
| IPC 契约 | [`IPC_CONTRACT.md`](IPC_CONTRACT.md) §一·伍补 |
| 冒烟证据 | `artifacts/voice-settings.png` / `voice-read-trigger.png` / `voice-speaker-btn.png` / `voice-reminder-broadcast.png` / `voice-stt-degraded.png`；记录见 `SMOKE_TEST.md` §14 |
| 单测结论 | `cargo test -p qibai-core`：**138 passed / 0 failed**（既有 121 全保持 + 新增 17） |

---

## 12. 迭代⑤ · 每角色专属音色

> 本节为迭代⑤新增能力，冻结于 `qibai-core/src/voice_profile.rs`；上文 §1–§11 的迭代②内容保持冻结不变。
> 核心思想：**每个角色（persona）带一份自己的音色档案 `voice_profile`**，角色切换后，对话 / 提醒 / 升级 / 守护台词都按对应角色音色播放；全局 `voice.rate` 仍作为叠加系数参与最终语速。

### 12.1 `voice_profile` 字段与默认值

`personas.json`（及其 Rust 端 `personas.rs` 内置默认表）中每个角色实体新增 `voice_profile` 字段：

```jsonc
"voice_profile": {
  "mode": "auto",                       // "auto" | "custom"（默认 auto）
  "pitch": 0.0,                         // f32，范围 -10..10，默认 0.0
  "rate": 1.0,                          // f32，范围 0.5..2.0，默认 1.0
  "engine_voice_id": "",                // 本地引擎指定音色（espeak -v / say -v / SAPI SelectVoice），留空用系统默认
  "cloud_speaker": ""                   // 云端 speaker id，非空时优先于全局 settings.voice.cloud_voice
}
```

- **迁移幂等**：旧 `personas.json` 缺 `voice_profile`（或缺其子字段）时，serde `default` 自动补 `{ mode:"auto", pitch:0, rate:1.0, engine_voice_id:"", cloud_speaker:"" }`，不崩溃、不整体回退；重复加载结果一致。
- `mode="auto"`：使用 5 角色内置默认映射（见 §12.2）；`mode="custom"`：使用用户在设置页音色卡片里调好的 pitch / rate / engine_voice_id / cloud_speaker。

### 12.2 五角色默认音色映射（auto 模式生效）

| 角色 id | 性格 tone | 默认 pitch | 默认 rate | 云 speaker id（火山引擎） | 音色名 | auto 标签 |
|---|---|---|---|---|---|---|
| `gudian` 古典佳人 | 温婉 | -2.0 | 1.0 | `zh_female_wenrouxiaoya_moon_bigtts` | 温柔小雅 | 温婉 · 中速 · 微低 |
| `dushi` 都市女性 | 知性 | 0.0 | 1.1 | `zh_female_shuangkuaisisi_emo_v2_mars_bigtts` | 爽快思思 | 知性 · 轻快 · 中性 |
| `wenrou` 温柔 | 恬静 | -1.0 | 0.8 | `ICL_zh_female_ruanmengtuanzi_cs_tob` | 软萌团子 | 恬静 · 慢速 · 微低 |
| `huopo` 活泼 | 元气 | 3.0 | 1.2 | `ICL_zh_female_wuxi_tob` | 元气甜妹 | 元气 · 轻快 · 偏高 |
| `reqing` 热情时尚 | 热情 | 2.0 | 1.05 | `ICL_zh_female_reqingaina_cs_tob` | 热情艾娜 | 热情 · 中速 · 偏高 |

- 云端 speaker id 取自火山引擎官方音色列表（https://www.volcengine.com/docs/6561/1257544）；**用户可按自己账号实际可用音色覆盖**（设置页卡片改 `cloud_speaker` 即可）。
- 无云端 key 时**降级本地音色并明确提示**（见 §12.4）。

### 12.3 生效规则与三平台 pitch / `engine_voice_id` 参数映射

**最终参数（与迭代②全局设置叠加）**：

```
eff_rate     = clamp(全局 voice.rate × profile.rate, 0.5, 2.0)
pitch        = profile.pitch
cloud_speaker = profile.cloud_speaker 非空 ? profile.cloud_speaker : settings.voice.cloud_voice
```

本地 TTS 在迭代②三平台命令（§4）基础上新增 **pitch** 与 **可指定音色** 两个差异化参数：

| 平台 | pitch 映射 | 可指定音色（`engine_voice_id`） |
|---|---|---|
| Linux `espeak` / `espeak-ng` | 加 `-p <val>`，`val = clamp(50 + pitch*3, 0, 99)`（0..99） | 指定 `-v <engine_voice_id>` |
| macOS `say` | 文本前缀 `[[pbas <round(pitch*3)>]]`（`pbas` 为相对半音，round 取整） | 指定 `-v <engine_voice_id>` |
| Windows SAPI（PowerShell） | 文本包 `<pitch absmiddle="<v>"/>`，`v = clamp(5 + pitch/2, 0, 10)`，保留一位小数 | `SelectVoice(<engine_voice_id>)` |

- **向后逐字节兼容**：`pitch=0` 且 `engine_voice_id` 为空时，本地命令与迭代②旧行为**逐字节一致**（`espeak` 不加 `-p`、`say` 不加前缀、SAPI 不加 `<pitch>`、不 SelectVoice）。

### 12.4 云端 speaker 解析链与无 key 降级提示

- 云端请求由 `build_cloud_request_with_speaker` 构造，speaker 取值链：
  **`profile.cloud_speaker`（非空）→ 否则 `settings.voice.cloud_voice`（全局）→ 否则引擎缺省**。
- 角色切换后，对话 / 提醒 / 升级 / 守护台词均按对应角色音色播放——`tts_speak` 新增可选 `persona_id` 参数；**提醒播报固定用当前主角色**音色。
- **无 key 降级**：有 `cloud_speaker` 但未配置云端 key（`cloud_ready=false`）时，不报错，自动降级本地音色，并在设置页对应角色音色卡状态位明确红字提示：
  「云端音色「\<speaker_id\>」未配置 key，当前用本地音色」。

### 12.5 音色标签算法

聊天 tab / 角色 chip 上展示的短标签由纯函数计算：

- **语速档**：`rate < 0.9` → 慢速；`rate < 1.1` → 中速；`rate ≥ 1.1` → 轻快。
- **音调档**：`pitch < -0.5` → 微低；`pitch ≤ 0.5` → 中性；`pitch ≤ 1.5` → 明亮；`pitch > 1.5` → 偏高。
- **合成**：
  - `mode="auto"` → `"{性格tone} · {rate_desc} · {pitch_desc}"`，性格 tone ∈ {温婉 / 知性 / 恬静 / 元气 / 热情}；
  - `mode="custom"` → `"自定义 · {rate_desc} · {pitch_desc}"`。

### 12.6 新 IPC 与真机验证项

- **新 IPC 命令**：`set_persona_voice { persona_id, voice_profile }`（保存某角色音色档案，落盘 personas.json）；`tts_speak` 增可选 `persona_id`。
- **真实发声为真机项**：沙箱无 `espeak`/`say`/SAPI、headless `speechSynthesis` 不可用，本地发声仅 simulated；三平台真实出声（pitch / `-v` / `<pitch>` 实际听感差异）需在装齐系统 TTS 的真机逐项验证（并入 §10 真机清单）。
- **真实云端为真机项**：沙箱无云端 key，云端 speaker 仅模拟、不发外部请求；配置 `QIBAI_TTS_KEY` 后按 `profile.cloud_speaker` 真实合成并由 `<audio>` 播放、断网/key 失效降级本地，需真机抓包验证（隐私边界同 §6.2，仅发送当前朗读文本片段）。
- 冒烟证据：`artifacts/voice-card.png`（5 张音色卡 + 标签 + 无 key 降级文案）、`voice-switch.png`（切主角为活泼后 chip=「元气 · 轻快 · 偏高」且 tts_speak 返回对应 pitch/eff_rate/cloud_speaker）、`voice-degrade.png`（custom 模式 reqing 卡红字降级提示）；记录见 `artifacts/SMOKE_RECORD_VOICE_FRONTEND.md`。
