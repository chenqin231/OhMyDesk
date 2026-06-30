# Spec B：客户端全平台图标

> 日期：2026-06-30
> 范围：功能⑤ 给客户端加品牌图标（裁剪 O.png 落地为各平台图标）
> 状态：已确认设计，待转实现计划

---

## 1. 背景与目标

客户端（`src/client`，Rust + Slint）当前**三平台均无自定义图标**：Windows exe 无图标、Slint 窗口/任务栏无图标、Linux `.desktop` 用系统通用 `utilities-terminal`、macOS 用系统通用图标（`packaging/macos/build-macos.sh` 支持 `.icns` 但源文件 `AppIcon.png` 不存在）。

目标：用品牌图 `O.png`（256×256 RGBA 盾牌+O，非透明区 x∈[14,242] 近满幅）生成各平台图标并接入，覆盖 Windows / Linux / macOS 的 exe、窗口/任务栏、应用菜单图标。

---

## 2. 关键决策（已确认）

| # | 决策 | 理由 |
|---|------|------|
| 1 | 图标资产**预生成并提交**（ImageMagick `convert`），构建不依赖图像转换工具 | 信创/CI 环境工具不全，预生成最稳；mingw/macos runner 无需装额外工具 |
| 2 | macOS 用 O.png 上采样到 1024 作 `AppIcon.png` | 源仅 256，简单盾形图上采样软化可接受 |
| 3 | Windows winres 交叉编译有效性以 CI 验证 | 本机可能无 mingw `windres`，不阻塞其余三平台；warn-not-fail 保证缺工具时构建不崩 |
| 4 | 资产统一放 `src/client/icons/`（macOS 单独 `packaging/macos/AppIcon.png`） | 集中管理；Slint/winres 就近引用 |

---

## 3. 资产清单（预生成提交）

由 `O.png` 经 `convert` 生成并提交：

| 文件 | 尺寸 | 用途 | 生成命令 |
|------|------|------|----------|
| `src/client/icons/app.ico` | 多尺寸 16/32/48/64/128/256 | Windows exe 嵌入 | `convert O.png -define icon:auto-resize=256,128,64,48,32,16 src/client/icons/app.ico` |
| `src/client/icons/app-icon.png` | 256×256 | Slint 窗口 + Linux 桌面 | `convert O.png -resize 256x256 src/client/icons/app-icon.png` |
| `packaging/macos/AppIcon.png` | 1024×1024 | macOS `.icns`（脚本生成） | `convert O.png -resize 1024x1024 packaging/macos/AppIcon.png` |

---

## 4. 接线（4 平台）

### 4.1 Windows exe 图标（winresource + build.rs）

- `src/client/Cargo.toml` `[build-dependencies]` 加 `winresource = "0.1"`。
- `src/client/build.rs` 在 `slint_build::compile` 之后，按 **target**（非 host）门控嵌入：
  ```rust
  if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
      let mut res = winresource::WindowsResource::new();
      res.set_icon("icons/app.ico");
      if let Err(e) = res.compile() {
          println!("cargo:warning=winresource 图标嵌入失败: {e}");
      }
  }
  ```
- 说明：build.rs 运行在 host（Linux 交叉编译），必须用 `CARGO_CFG_TARGET_OS` 判断目标平台；`winresource` 在 windows-gnu 下调用 mingw `windres`（`build-windows.sh` 已装 mingw-w64）。warn-not-fail 保证缺工具时不中断构建。

### 4.2 Slint 窗口/任务栏图标

- `src/client/ui/app.slint` 的 `AppWindow inherits Window`（第 476 行附近）加：
  ```
  icon: @image-url("../icons/app-icon.png");
  ```
- `@image-url` 由 slint-build 在**编译期**读取并嵌入像素（路径相对 .slint 文件 → `src/client/icons/app-icon.png`），无运行时图像解码依赖。winit 后端据此设置窗口/任务栏图标。

### 4.3 Linux .desktop 图标

- `packaging/deb/build-deb.sh`：
  - 把 `.desktop` 的 `Icon=utilities-terminal` 改为 `Icon=ohmydesk-client`。
  - 新增安装步骤：把 `src/client/icons/app-icon.png` 复制到包内 `usr/share/icons/hicolor/256x256/apps/ohmydesk-client.png`。

### 4.4 macOS .app 图标

- 提交 `packaging/macos/AppIcon.png`（1024）。`build-macos.sh` 现有逻辑（第 42-57 行）检测到该文件即 `iconutil` 生成 `.icns` 并放入 `.app/Contents/Resources`，**零代码改动**。

---

## 5. 验证

- 本机：`cargo build -p client`（Linux 本地 target）通过——build.rs 的 winresource 分支不触发（target_os=linux），Slint 图标嵌入成功（slint-build 编译期）。运行客户端目视确认窗口/任务栏出现盾牌图标。
- 资产：`file src/client/icons/app.ico` 应为 `MS Windows icon resource`，含多尺寸；`AppIcon.png` 为 1024×1024。
- Windows 交叉编译：若本机有 mingw（`x86_64-w64-mingw32-windres`）则 `bash packaging/windows/build-windows.sh` 验证 exe 带图标；否则留待 CI（`release.yml` windows job）确认产物图标。
- 不破坏现有构建：`cargo build -p client` 在改 build.rs 后仍通过。

---

## 6. 涉及文件清单

**新增（资产）**：`src/client/icons/app.ico`、`src/client/icons/app-icon.png`、`packaging/macos/AppIcon.png`

**修改**：
- `src/client/Cargo.toml`（winresource build-dep）
- `src/client/build.rs`（winres 嵌入）
- `src/client/ui/app.slint`（Window icon）
- `packaging/deb/build-deb.sh`（Icon= + 安装 png）

**不改**：`packaging/macos/build-macos.sh`（现有逻辑提交 AppIcon.png 即生效）
