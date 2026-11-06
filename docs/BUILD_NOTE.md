# 元气小小 Qibai — 构建说明（三平台系统依赖前置）

本项目分两部分：

- **qibai-core**（纯 Rust 核心库）：不依赖 tauri / webkit2gtk / gtk3 等系统库，
  在任意 Rust 工具链下可独立 `cargo build` / `cargo test`。
- **src-tauri**（Tauri 2 应用壳）：依赖 tauri 及其 sys 系 crate，需要对应平台的系统库。

---

## 0. 安装 Rust 工具链（三平台通用）

```bash
# 无 sudo 环境下装到 Home
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"
```

---

## 1. Linux（Debian/Ubuntu 系）

### 系统依赖（必须先装）

```bash
sudo apt update
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libsoup-3.0-dev \
  javascriptcoregtk-4.1-dev \
  build-essential curl wget file libssl-dev pkg-config librsvg2-dev \
  libasound2-dev libpcaudio0 libsonic0
```

> **v14 起新增：`libasound2-dev`（ALSA）**——免提聆听用 cpal 采集麦克风，
> Linux 后端依赖 ALSA 开发头文件，缺它会在 `alsa-sys` 自定义构建命令处编译失败
> （报错形如 `failed to run custom build command for 'alsa-sys v0.3.1'`）。
> `libpcaudio0 libsonic0` 为随包捆绑 espeak-ng 的运行库（供离线发声）。

> 关键库对照：
> - `libwebkit2gtk-4.1-dev` — Tauri WebView 后端
> - `libgtk-3-dev` — GTK3 窗口
> - `libsoup-3.0-dev` — HTTP 客户端（tauri 内部）
> - `javascriptcoregtk-4.1-dev` — JS 引擎

### 构建

```bash
cd src-tauri
cargo build --release
# 产物：target/release/qibai-tauri
```

### Linux 运行时额外依赖（托盘）

Linux 上系统托盘需要 AppIndicator 运行时库（**仅运行时需要，不参与编译**）：

```bash
sudo apt install -y libayatana-appindicator3-1
# 或老发行版：libappindicator3-1
```

> Windows / macOS 的托盘由系统原生实现，**无需**此库。

### 跨平台插件说明

本壳启用了两个 Tauri 官方插件，全部跨三平台、无需额外系统包：

- `tauri-plugin-notification`：系统通知（久坐/喝水/护眼提醒触发时弹窗）。
- `tauri-plugin-autostart`：开机自启。
  - Windows：写注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
  - macOS：写入 `~/Library/LaunchAgents/<bundle-id>.plist`（LaunchAgent）。
  - Linux：写入 `~/.config/autostart/<bundle-id>.desktop`（freedesktop autostart 规范）。

> 在本开发机上的冒烟记录：由于沙箱系统目录只读、禁 sudo/apt，
> 上述系统库全部缺失，`cargo check` 在 src-tauri 内必然失败（缺 `webkit2gtk-4.1` pkg-config 包）。
> 这是**环境限制**，不是代码问题。代码层面 tauri.conf.json、命令注册、托盘/自启/通知插件均已接线。

---

## 2. Windows

### 系统依赖

- **WebView2 Runtime**：Windows 11 已内置；Windows 10 需手动安装
  [Evergreen Bootstrapper](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)。
- **Microsoft C++ Build Tools**（含 MSVC、Windows SDK）。

### 构建

```powershell
# 用 MSVC 工具链（rustup default stable-msvc）
cd src-tauri
cargo build --release
```

---

## 3. macOS

### 系统依赖

- **Xcode Command Line Tools**：`xcode-select --install`
- （可选）完整 Xcode：用于 Notarization / 签名。

### 构建

```bash
cd src-tauri
cargo build --release
# 产物：target/release/qibai-tauri（.app bundle 需 `cargo tauri build`）
```

---

## 4. 仅验证核心库（无需系统 GUI 库）

```bash
cd qibai-core    # 或仓库根目录
cargo build
cargo test
```

根目录 `cargo test` 只会编译 `qibai-core`（workspace members = ["qibai-core"]）；
`src-tauri` 自带独立 `[workspace]`，不会被拖入根构建。

---

## 5. 常见问题

- **`error: pkg-config could not find webkit2gtk-4.1`**：Linux 未装 `libwebkit2gtk-4.1-dev`。
- **`error: linking with `cc` failed`**：缺 `build-essential` / `libssl-dev` / `pkg-config`。
- **Windows 上 WebView2 白屏**：WebView2 Runtime 未安装。
- **macOS 上 `cargo:warning=The linking profile was overridden`**：正常提示，不影响产物。
