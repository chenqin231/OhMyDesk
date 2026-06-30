# 客户端全平台图标 实现计划（Spec B）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** 用 `O.png` 生成各平台图标并接入 Windows exe / Slint 窗口 / Linux .desktop / macOS .app。

**Architecture:** ImageMagick `convert` 预生成图标资产并提交（构建不依赖转换工具）；Windows 用 `winresource` build-dep + build.rs 按 target 门控嵌入；Slint 用 `@image-url` 编译期嵌入窗口图标；Linux/macOS 改打包脚本与提交 AppIcon.png。

**Tech Stack:** Rust + Slint 1.17 + winresource + ImageMagick(convert) + bash 打包脚本。

**对应 Spec:** `docs/superpowers/specs/2026-06-30-client-app-icon-design.md`

---

## Task 1：生成并提交图标资产

**Files:**
- Create: `src/client/icons/app.ico`、`src/client/icons/app-icon.png`、`packaging/macos/AppIcon.png`

- [ ] **Step 1：生成三个资产**

工作目录 `/data/code/OhMyDesk`，源图 `O.png`（256×256）。`src/client/icons/` 目录已存在。

```bash
convert O.png -define icon:auto-resize=256,128,64,48,32,16 src/client/icons/app.ico
convert O.png -resize 256x256 src/client/icons/app-icon.png
convert O.png -resize 1024x1024 packaging/macos/AppIcon.png
```

- [ ] **Step 2：校验资产**

```bash
file src/client/icons/app.ico        # 期望: MS Windows icon resource ... 6 icons
file src/client/icons/app-icon.png   # 期望: PNG image data, 256 x 256
file packaging/macos/AppIcon.png     # 期望: PNG image data, 1024 x 1024
```
Expected: `.ico` 为 MS Windows icon resource 且含 6 个尺寸；两个 PNG 尺寸正确。

- [ ] **Step 3：提交**

```bash
git add src/client/icons/app.ico src/client/icons/app-icon.png packaging/macos/AppIcon.png
git commit -m "feat(client): 生成品牌图标资产(ico/png/AppIcon)"
```

---

## Task 2：Windows exe 图标（winresource + build.rs）

**Files:**
- Modify: `src/client/Cargo.toml`（build-dependencies 加 winresource）
- Modify: `src/client/build.rs`

- [ ] **Step 1：Cargo.toml 加 build-dep**

在 `src/client/Cargo.toml` 的 `[build-dependencies]` 段（现有 `slint-build = "1.17"`）下加一行：

```toml
[build-dependencies]
slint-build = "1.17"
winresource = "0.1"
```

- [ ] **Step 2：build.rs 嵌入图标（按 target 门控）**

把 `src/client/build.rs` 改为：

```rust
// 编译期把 ui/app.slint 编译为 Rust 代码（由 slint::include_modules!() 引入）。
// Windows target 额外把 app.ico 嵌入 exe（任务栏/资源管理器图标）。
fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // build.rs 运行在 host（交叉编译时 host=Linux），必须按 target 而非 host 判断。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/app.ico");
        if let Err(e) = res.compile() {
            // warn-not-fail：缺 windres 等工具时不中断构建，仅告警（图标不嵌入）。
            println!("cargo:warning=winresource 图标嵌入失败: {e}");
        }
    }
}
```

- [ ] **Step 3：本机 Linux target 构建验证（winres 分支不触发）**

Run: `cargo build -p client`
Expected: 编译通过（本机 target_os=linux，winresource 分支跳过；仅验证 build.rs 改动不破坏现有构建）

- [ ] **Step 4：提交**

```bash
git add src/client/Cargo.toml src/client/build.rs
git commit -m "feat(client): build.rs 嵌入 Windows exe 图标(winresource)"
```

---

## Task 3：Slint 窗口/任务栏图标

**Files:**
- Modify: `src/client/ui/app.slint`（AppWindow 加 icon）

- [ ] **Step 1：定位 AppWindow 并加 icon 属性**

在 `src/client/ui/app.slint` 找到 `export component AppWindow inherits Window {`（第 476 行附近），在其属性区（已有 `title:` 那几行旁）加：

```slint
    icon: @image-url("../icons/app-icon.png");
```

> 路径相对 .slint 文件（`src/client/ui/app.slint`）→ 解析到 `src/client/icons/app-icon.png`。slint-build 编译期读取并嵌入像素。

- [ ] **Step 2：构建验证（图标被 slint-build 嵌入）**

Run: `cargo build -p client`
Expected: 编译通过。若路径错误，slint-build 会报 `@image-url` 找不到文件并 FAIL——此时检查相对路径。

- [ ] **Step 3：目视确认（可选，有图形环境时）**

Run: `cargo run -p client` 启动客户端，观察窗口标题栏/任务栏是否显示盾牌图标。
Expected: 窗口图标为 O.png 盾牌图（无图形环境则跳过，依赖 CI/真机）。

- [ ] **Step 4：提交**

```bash
git add src/client/ui/app.slint
git commit -m "feat(client): Slint 窗口图标(app-icon.png)"
```

---

## Task 4：Linux .desktop 图标

**Files:**
- Modify: `packaging/deb/build-deb.sh`

- [ ] **Step 1：确认脚本变量与 .desktop 位置（已勘明）**

`packaging/deb/build-deb.sh` 已知事实：`REPO_ROOT`=仓库根（第 6 行）、`STAGE`=包根（第 22 行）、`.desktop` 由 heredoc 写在第 54-63 行（`Icon=utilities-terminal` 在第 60 行）。

- [ ] **Step 2a：改 .desktop 的 Icon 名**

把第 60 行：
```
Icon=utilities-terminal
```
改为：
```
Icon=ohmydesk-client
```

- [ ] **Step 2b：安装图标 PNG 到包内 hicolor 主题**

在 `.desktop` heredoc 结束（第 63 行 `EOF`）之后、`# 文档`（第 65 行）之前，插入：
```bash
# 应用图标（桌面环境从 hicolor 主题按 .desktop 的 Icon=ohmydesk-client 查找）
install -Dm644 "$REPO_ROOT/src/client/icons/app-icon.png" \
  "$STAGE/usr/share/icons/hicolor/256x256/apps/ohmydesk-client.png"
```
> `install -Dm644` 会自动创建 `hicolor/256x256/apps/` 父目录。源用 `$REPO_ROOT/src/client/icons/app-icon.png`（Task 1 已提交），目标用包根 `$STAGE`。

- [ ] **Step 3：脚本语法自检**

Run: `bash -n packaging/deb/build-deb.sh`
Expected: 无语法错误（`bash -n` 仅解析不执行）。

- [ ] **Step 4：提交**

```bash
git add packaging/deb/build-deb.sh
git commit -m "feat(packaging): Linux deb 安装应用图标 + Icon= 指向"
```

---

## 验收（全部 Task 完成后）

- [ ] `cargo build -p client` 通过（build.rs + Slint 图标改动不破坏构建）
- [ ] `file src/client/icons/app.ico` 为 MS Windows icon resource（6 尺寸）；`AppIcon.png` 为 1024×1024
- [ ] `bash -n packaging/deb/build-deb.sh` 无语法错误
- [ ] 有图形环境/真机：客户端窗口与任务栏显示盾牌图标
- [ ] Windows 交叉编译：本机有 mingw 则跑 `packaging/windows/build-windows.sh` 验 exe 图标；否则留待 CI（release.yml windows job）确认
- [ ] macOS：`packaging/macos/AppIcon.png` 已就位，build-macos.sh 无需改动即可生成 .icns
