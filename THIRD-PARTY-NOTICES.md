# 第三方组件与许可声明 / Third-Party Notices

[中文](#中文) ｜ [English](#english)

---

<a id="中文"></a>
## 中文

本仓库自身源码以 **MIT** 发布（见 `LICENSE`，纯 MIT 正文，便于平台自动识别）。

> **许可范围划分**（`LICENSE` 只覆盖代码）：
> - 美术素材（立绘 / 动作帧 / 表情 / 图标）**不适用 MIT**，授权范围见 `docs/ASSETS.md`；
> - 下列第三方组件适用其**各自许可**。

下列组件由第三方提供，适用其**各自许可**。仓库不打包这些组件的二进制与权重，仅提供获取脚本或运行时调用方式。

> ✅ 语音链路 4 条与 `sonora` 的许可已按上游 LICENSE / 模型页核对（2026-11）。仍建议公开前复核心版本内附 LICENSE；本表为事实记录，不构成法律结论。

### 一、语音链路

| 组件 | 版本 / 形态 | 预期许可 | 仓库内是否分发 | 说明 |
|---|---|---|---|---|
| **whisper.cpp** | 编译产物 `libwhisper.so.1.9.5`、`whisper-server`、`whisper-cli` | **MIT**（已核对上游 `ggml-org/whisper.cpp` 的 LICENSE 原文） | ❌ 由 `fetch_resources.sh` 从官方 release（tag `b5454`）下载 | 本项目**未改造源码、非自建 fork**，只用官方预编译二进制；以独立进程 / 动态库调用 |
| **Whisper 模型权重** `ggml-base.bin` / `ggml-small.bin` | 142 MB / ~466 MB | MIT（OpenAI Whisper 权重） | ❌ 脚本从 `hf-mirror.com/ggerganov/whisper.cpp` 下载 | 首次运行必须下载 |
| **sherpa-onnx / Paraformer** | `paraformer.onnx`（233 MB）+ `tokens.txt`、`vad.onnx`、`punc.onnx` | **Apache-2.0**（已核对 `k2-fsa/sherpa-onnx` 的 LICENSE；模型页标注 `license: apache-2.0`） | ❌ 不入库，由 `fetch_resources.sh` 下载并逐文件 sha256 校验 | 备用中文离线识别引擎（paraformer-zh int8） |
| **piper** 引擎 | 可执行程序 | MIT | ❌ | 本地离线 TTS |
| **piper 音色** `zh_CN-huayan-medium.onnx` | 61 MB | 仓库整体 MIT；但该音色 MODEL_CARD 记载训练数据集 `PlayVoice/HuaYan_TTS` 许可为 **Unknown** | ❌ 不入库、不随安装包分发；用户自行从 `rhasspy/piper-voices` 下载并自判用途 | **结论：不做商用授权保证**；本项目默认 TTS 走 espeak-ng（捆绑）+ edge-tts（在线），piper 仅作可选本地女声 |
| **bge-small-zh-v1.5**（向量模型） | `model.onnx` 91 MB + `vocab.txt` | MIT（BAAI，上游 `BAAI/bge-small-zh-v1.5`） | ❌ 不进安装包、不入库 | 知识库语义检索用；用户在应用内按需下载（Release `kb-model-v1` 仅做镜像分发，权重版权归原作者） |
| **espeak-ng** | 二进制 + `espeak-ng-data` 语音数据（约 26 MB，解包到 `src-tauri/tts-espeak/`） | **GPL-3.0** | ❌ 不入库，`fetch_resources.sh` 从 Debian 官方包（`deb.debian.org`，1.52.0）解包组装，无需 sudo | 以独立可执行程序调用；若你选择把该二进制一起分发，须同时提供 GPL-3.0 全文与源码获取地址（https://github.com/espeak-ng/espeak-ng） |

### 二、Rust 依赖（主要项，均由 `Cargo.lock` 锁定）

| Crate | 用途 | 预期许可 |
|---|---|---|
| `tauri` / `tauri-build` / `tauri-plugin-*` | 应用壳、托盘、通知、自启、剪贴板、更新 | MIT / Apache-2.0 |
| `serde` / `serde_json` | 序列化 | MIT / Apache-2.0 |
| `tokio` / `tokio-tungstenite` / `futures-util` | 异步运行时、WebSocket | MIT |
| `reqwest`（`rustls-tls`） | HTTP 客户端 | MIT / Apache-2.0 |
| `cpal` / `rodio` / `rubato` | 音频采集、播放、重采样 | MIT / Apache-2.0 |
| `sonora` | WebRTC AEC3 纯 Rust 移植（回声消除） | **BSD-3-Clause**（crates.io `sonora 0.3.0`） |
| `tract-onnx` | 纯 Rust ONNX 推理（本地向量模型，免 C/C++ 依赖） | MIT / Apache-2.0 |
| `aes-gcm` / `pbkdf2` / `sha2` / `rand` / `zeroize` | 本地保险箱加密 | MIT / Apache-2.0 |
| `keyring` | 系统钥匙串 | MIT / Apache-2.0 |
| `directories` / `thiserror` / `async-trait` / `base64` / `zip` | 路径、错误、trait、编码、打包 | MIT / Apache-2.0 |
| `gtk` / `gdk`（仅 Linux） | X11 窗口表面重绘 | MIT |

### 三、Node 依赖（`server/`）

| 包 | 用途 | 许可 |
|---|---|---|
| `ws` | WebSocket 服务端 | MIT |
| `node:sqlite` | 内置数据库（Node ≥ 22.5 自带） | 随 Node 分发（MIT 类） |

### 四、美术素材

立绘、动作帧、表情、图标由生成式模型产出，**不属于第三方开源组件**，权利与授权见 `docs/ASSETS.md`。

### 五、分发时的义务清单

如果你要发布**安装包**（而不只是源码），额外需要：

1. 附上本文件与 `LICENSE`；
2. 若打包了 espeak-ng 二进制 → 附 GPL-3.0 全文 + 源码获取地址；
3. 若打包了 piper 音色 → 确认其许可允许再分发；
4. 若打包了 whisper 二进制 → 附 MIT 版权声明；
5. 若打包了模型权重 → 附来源与许可说明。

---

<a id="english"></a>
## English

This repository's own source is released under **MIT** (see `LICENSE`, which contains the plain MIT text so platforms can detect it automatically).

> **How the licence scopes divide** (`LICENSE` covers code only):
> - Artwork (character art / motion frames / expressions / icons) is **not** covered by MIT — see `docs/ASSETS.md`;
> - The third-party components below follow **their own licences**.

The components below are provided by third parties and follow **their own licences**. This repository does not bundle their binaries or weights — it only provides fetch scripts or a runtime calling convention.

> ✅ Licences for the four speech-pipeline items and `sonora` were checked against upstream LICENSE files / model pages (Nov 2026). Re-check the bundled LICENSE of the exact version before publishing; this table is a factual record, not a legal conclusion.

### 1. Speech pipeline

| Component | Version / form | Expected licence | Distributed in repo | Notes |
|---|---|---|---|---|
| **whisper.cpp** | Build artefacts `libwhisper.so.1.9.5`, `whisper-server`, `whisper-cli` | **MIT** (upstream `ggml-org/whisper.cpp` LICENSE checked) | ❌ downloaded by `fetch_resources.sh` from the official release (tag `b5454`) | **No source fork or modification** — official prebuilt binaries only, called as a separate process / shared library |
| **Whisper model weights** `ggml-base.bin` / `ggml-small.bin` | 142 MB / ~466 MB | MIT (OpenAI Whisper weights) | ❌ downloaded by script from `hf-mirror.com/ggerganov/whisper.cpp` | Required on first run |
| **sherpa-onnx / Paraformer** | `paraformer.onnx` (233 MB) + `tokens.txt`, `vad.onnx`, `punc.onnx` | **Apache-2.0** (`k2-fsa/sherpa-onnx` LICENSE checked; model card states `license: apache-2.0`) | ❌ not in repo; downloaded and sha256-verified by `fetch_resources.sh` | Alternative Chinese offline recogniser (paraformer-zh int8) |
| **piper** engine | Executable | MIT | ❌ | Local offline TTS |
| **piper voice** `zh_CN-huayan-medium.onnx` | 61 MB | Repo as a whole is MIT; this voice's MODEL_CARD lists its training dataset `PlayVoice/HuaYan_TTS` as **Unknown** | ❌ not in repo, not shipped in installers; users download from `rhasspy/piper-voices` and judge their own use | **Conclusion: no commercial clearance is asserted**; default TTS is espeak-ng (bundled) + edge-tts (online); piper is optional |
| **bge-small-zh-v1.5** (embedding model) | `model.onnx` 91 MB + `vocab.txt` | MIT (BAAI, upstream `BAAI/bge-small-zh-v1.5`) | ❌ not in the installer, not in repo | Used for knowledge-base semantic retrieval; downloaded on demand in-app (the `kb-model-v1` release only mirrors it; copyright stays with the original authors) |
| **espeak-ng** | Binary + `espeak-ng-data` voice data (~26 MB, unpacked into `src-tauri/tts-espeak/`) | **GPL-3.0** | ❌ not in repo; `fetch_resources.sh` assembles it from official Debian packages (`deb.debian.org`, 1.52.0) without sudo | Called as a separate executable; if you choose to ship that binary too, include the full GPL-3.0 text and the source URL (https://github.com/espeak-ng/espeak-ng) |

### 2. Rust dependencies (main ones, all pinned by `Cargo.lock`)

| Crate | Purpose | Expected licence |
|---|---|---|
| `tauri` / `tauri-build` / `tauri-plugin-*` | App shell, tray, notifications, autostart, clipboard, updater | MIT / Apache-2.0 |
| `serde` / `serde_json` | Serialisation | MIT / Apache-2.0 |
| `tokio` / `tokio-tungstenite` / `futures-util` | Async runtime, WebSocket | MIT |
| `reqwest` (`rustls-tls`) | HTTP client | MIT / Apache-2.0 |
| `cpal` / `rodio` / `rubato` | Audio capture, playback, resampling | MIT / Apache-2.0 |
| `sonora` | Pure-Rust port of WebRTC AEC3 (echo cancellation) | **BSD-3-Clause** (crates.io `sonora 0.3.0`) |
| `tract-onnx` | Pure-Rust ONNX inference (local embedding model, no C/C++ dependency) | MIT / Apache-2.0 |
| `aes-gcm` / `pbkdf2` / `sha2` / `rand` / `zeroize` | Local vault encryption | MIT / Apache-2.0 |
| `keyring` | System keychain | MIT / Apache-2.0 |
| `directories` / `thiserror` / `async-trait` / `base64` / `zip` | Paths, errors, traits, encoding, packaging | MIT / Apache-2.0 |
| `gtk` / `gdk` (Linux only) | X11 window surface redraw | MIT |

### 3. Node dependencies (`server/`)

| Package | Purpose | Licence |
|---|---|---|
| `ws` | WebSocket server | MIT |
| `node:sqlite` | Built-in database (ships with Node ≥ 22.5) | Distributed with Node (MIT-like) |

### 4. Artwork

Character art, motion frames, expressions and icons are produced by generative models and are **not third-party open-source components**; rights and permissions are described in `docs/ASSETS.md`.

### 5. Obligations when distributing

If you publish an **installer** (not just source), you additionally need to:

1. Include this file and `LICENSE`;
2. If you bundled the espeak-ng binary → include the full GPL-3.0 text and where to obtain the source;
3. If you bundled piper voices → confirm their licences permit redistribution;
4. If you bundled the whisper binary → include the MIT copyright notice;
5. If you bundled model weights → include their origin and licence.
