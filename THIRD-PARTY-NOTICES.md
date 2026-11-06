# 第三方组件与许可声明 / Third-Party Notices

本仓库自身源码以 **MIT** 发布（见 `LICENSE`，纯 MIT 正文，便于平台自动识别）。

> **许可范围划分**（`LICENSE` 只覆盖代码）：
> - 美术素材（立绘 / 动作帧 / 表情 / 图标）**不适用 MIT**，授权范围见 `docs/ASSETS.md`；
> - 下列第三方组件适用其**各自许可**。

下列组件由第三方提供，适用其**各自许可**。仓库不打包这些组件的二进制与权重，仅提供获取脚本或运行时调用方式。

> ⚠️ **标【待核实】的条目，公开前请以你实际使用的版本内附 LICENSE 为准**，本表只记录预期的上游许可，不作为法律结论。

---

## 一、语音链路

| 组件 | 版本 / 形态 | 预期许可 | 仓库内是否分发 | 说明 |
|---|---|---|---|---|
| **whisper.cpp** | 编译产物 `libwhisper.so.1.9.5`、`whisper-server`、`whisper-cli` | MIT（上游 `ggml-org/whisper.cpp`）【待核实你所用 fork】 | ❌ 由 `fetch_resources.sh` 从官方 release 下载 | 调用方式为独立进程 / 动态库，非源码内联 |
| **Whisper 模型权重** `ggml-base.bin` / `ggml-small.bin` | 142 MB / ~466 MB | MIT（OpenAI Whisper 权重） | ❌ 脚本从 `hf-mirror.com/ggerganov/whisper.cpp` 下载 | 首次运行必须下载 |
| **sherpa-onnx / Paraformer** | `paraformer.onnx`（233 MB） | Apache-2.0【待核实】 | ❌ 不入库 | 备用中文流式识别引擎 |
| **piper** 引擎 | 可执行程序 | MIT | ❌ | 本地离线 TTS |
| **piper 音色** `zh_CN-huayan-medium.onnx` | 61 MB | **各音色独立许可，部分限非商用**【必须核实】 | ❌ 不入库，建议用户自行从 `rhasspy/piper-voices` 下载 | **若该音色限非商用，不得随本项目安装包分发** |
| **bge-small-zh-v1.5**（向量模型） | `model.onnx` 91 MB + `vocab.txt` | MIT（BAAI，上游 `BAAI/bge-small-zh-v1.5`） | ❌ 不进安装包、不入库 | 知识库语义检索用；用户在应用内按需下载（Release `kb-model-v1` 仅做镜像分发，权重版权归原作者） |
| **espeak-ng** | 二进制 + `share/` 语音数据（13 MB） | **GPL-3.0** | ❌ 不入库，建议用户用系统包管理器安装 | 以独立可执行程序调用；若你选择分发二进制，须同时提供许可文本与源码获取方式 |

## 二、Rust 依赖（主要项，均由 `Cargo.lock` 锁定）

| Crate | 用途 | 预期许可 |
|---|---|---|
| `tauri` / `tauri-build` / `tauri-plugin-*` | 应用壳、托盘、通知、自启、剪贴板、更新 | MIT / Apache-2.0 |
| `serde` / `serde_json` | 序列化 | MIT / Apache-2.0 |
| `tokio` / `tokio-tungstenite` / `futures-util` | 异步运行时、WebSocket | MIT |
| `reqwest`（`rustls-tls`） | HTTP 客户端 | MIT / Apache-2.0 |
| `cpal` / `rodio` / `rubato` | 音频采集、播放、重采样 | MIT / Apache-2.0 |
| `sonora` | WebRTC AEC3 纯 Rust 移植（回声消除） | 【待核实】 |
| `tract-onnx` | 纯 Rust ONNX 推理（本地向量模型，免 C/C++ 依赖） | MIT / Apache-2.0 |
| `aes-gcm` / `pbkdf2` / `sha2` / `rand` / `zeroize` | 本地保险箱加密 | MIT / Apache-2.0 |
| `keyring` | 系统钥匙串 | MIT / Apache-2.0 |
| `directories` / `thiserror` / `async-trait` / `base64` / `zip` | 路径、错误、trait、编码、打包 | MIT / Apache-2.0 |
| `gtk` / `gdk`（仅 Linux） | X11 窗口表面重绘 | MIT |

## 三、Node 依赖（`server/`）

| 包 | 用途 | 许可 |
|---|---|---|
| `ws` | WebSocket 服务端 | MIT |
| `node:sqlite` | 内置数据库（Node ≥ 22.5 自带） | 随 Node 分发（MIT 类） |

## 四、美术素材

立绘、动作帧、表情、图标由生成式模型产出，**不属于第三方开源组件**，权利与授权见 `docs/ASSETS.md`。

---

## 五、分发时的义务清单

如果你要发布**安装包**（而不只是源码），额外需要：

1. 附上本文件与 `LICENSE`；
2. 若打包了 espeak-ng 二进制 → 附 GPL-3.0 全文 + 源码获取地址；
3. 若打包了 piper 音色 → 确认其许可允许再分发；
4. 若打包了 whisper 二进制 → 附 MIT 版权声明；
5. 若打包了模型权重 → 附来源与许可说明。
