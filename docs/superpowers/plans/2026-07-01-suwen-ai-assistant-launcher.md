# 客户端「AI助手」按钮(素问检测/静默安装/拉起) 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 OhMyDesk 客户端右下角新增「AI助手」悬浮按钮,点击后确保素问已安装(检测 `suwen-daemon.exe`,缺则下载 `suwen-setup.exe` 静默 `/S` 安装)并拉起 `suwen-gui.exe`。

**Architecture:** 新增单一模块 `src/client/src/suwen.rs`,内部为一条线性状态机(检测→[下载→安装→等待]→拉起),全程后台线程执行、经 `slint::invoke_from_event_loop` 回写 UI 相位;复用 `update.rs` 的 ureq(SChannel) Agent 与 `CapReader`,零新依赖。UI 侧照抄现有「检查更新」按钮范式。

**Tech Stack:** Rust + Slint;ureq 2.12 + native-tls(SChannel);tempfile;std `Command`(`creation_flags` 静默)。仅 Windows 有实体逻辑,其余平台按钮隐藏。

**设计依据:** `docs/superpowers/specs/2026-07-01-suwen-ai-assistant-launcher-design.md`

**关键前置事实(已核实):**
- 客户端启动即 `elevate::ensure_elevated()`(`main.rs:73`)→ 已管理员运行 → 安装器子进程继承令牌,无二次 UAC。
- `update::CapReader` 已 `pub`(`update.rs:197`);`update::build_agent` 现为私有(`update.rs:282/292`),需提为 `pub(crate)`。
- `CREATE_NO_WINDOW = 0x0800_0000`(参 `exec.rs:38`)。
- Slint 回调注册在 `wire_ui_callbacks(ui: &AppWindow, ...)`(`ui_glue.rs:112`);跨线程回写范式见 `ui_glue.rs:936`。
- 客户端二进制名 `client`;本地已装 `x86_64-pc-windows-gnu`(与 CI 一致),Windows 专属代码用 `cargo check --target x86_64-pc-windows-gnu` 验证。

---

## 文件结构

| 文件 | 责任 | 改动 |
|---|---|---|
| `src/client/src/suwen.rs` | 素问检测/下载/安装/拉起 + 状态机编排 | **新建** |
| `src/client/src/main.rs` | 注册模块 | 加 `mod suwen;` |
| `src/client/src/update.rs` | 复用 HTTP Agent | `build_agent` 提为 `pub(crate)` |
| `src/client/ui/app.slint` | 3 property + 1 callback + 右下角按钮 | 两处插入 |
| `src/client/src/ui_glue.rs` | 绑定回调 + 置平台支持位 | `wire_ui_callbacks` 内新增一块 |

---

## Task 1: `suwen.rs` 纯逻辑核心 + 单元测试 + 注册模块

**Files:**
- Create: `src/client/src/suwen.rs`
- Modify: `src/client/src/main.rs`(在 `mod elevate;` 附近加 `mod suwen;`)
- Test: 同文件 `#[cfg(test)] mod tests`(纯函数,可在 Linux 宿主跑)

- [ ] **Step 1: 写失败测试(先建文件含测试与被测纯函数签名)**

创建 `src/client/src/suwen.rs`,内容如下(本步即含实现,但先跑测试确认可编译通过——纯逻辑用 TDD 的"实现+测试同提交"最小闭环):

```rust
//! 「AI助手」按钮后端:检测/静默安装/拉起素问(Suwen)。仅 Windows 有实体逻辑。
//!
//! 流程:检测 suwen-daemon.exe 是否存在 → 不存在则下载 suwen-setup.exe 静默安装(/S)
//! → 等安装器退出 + 轮询 daemon.exe 落盘 → 拉起 suwen-gui.exe。
//! 客户端启动即提权(main.rs ensure_elevated),安装器子进程继承管理员令牌,不二次弹 UAC。
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;

/// 素问安装器下载地址(带 /S 静默安装)。
pub const SETUP_URL: &str = "https://ai-agent.guoziweb.com/downloads/client/suwen-setup.exe";
/// 素问守护进程名(检测锚点)。
pub const DAEMON_EXE: &str = "suwen-daemon.exe";
/// 素问 GUI 进程名(拉起目标)。
pub const GUI_EXE: &str = "suwen-gui.exe";

// UI 相位:与 app.slint `suwen_phase` 取值一一对应。
pub const PHASE_IDLE: i32 = 0;
pub const PHASE_DOWNLOAD: i32 = 1;
pub const PHASE_INSTALL: i32 = 2;
pub const PHASE_LAUNCH: i32 = 3;
pub const PHASE_FAILED: i32 = 4;

/// 由 %ProgramFiles% / %ProgramFiles(x86)% 推出素问安装目录候选(纯函数,便于测试)。
/// 主目录在前、兜底 x86 在后;跳过 None/空串;去重。
pub fn candidate_dirs(pf: Option<String>, pf86: Option<String>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in [pf, pf86].into_iter().flatten() {
        if base.is_empty() {
            continue;
        }
        let d = PathBuf::from(base).join("Suwen");
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

/// 读环境变量得到安装目录候选。
fn install_dirs() -> Vec<PathBuf> {
    candidate_dirs(
        std::env::var("ProgramFiles").ok(),
        std::env::var("ProgramFiles(x86)").ok(),
    )
}

/// 在候选目录中查找已存在的指定 exe;返回第一个存在者。
fn find_exe(name: &str) -> Option<PathBuf> {
    install_dirs()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

/// 素问是否已安装(以 daemon.exe 存在为锚点)。
pub fn is_installed() -> bool {
    find_exe(DAEMON_EXE).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_dirs_prefers_program_files_then_x86() {
        let dirs = candidate_dirs(
            Some(r"C:\Program Files".into()),
            Some(r"C:\Program Files (x86)".into()),
        );
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], PathBuf::from(r"C:\Program Files").join("Suwen"));
        assert_eq!(dirs[1], PathBuf::from(r"C:\Program Files (x86)").join("Suwen"));
    }

    #[test]
    fn candidate_dirs_skips_missing_and_empty() {
        assert!(candidate_dirs(None, Some(String::new())).is_empty());
        assert_eq!(
            candidate_dirs(Some(r"C:\PF".into()), None),
            vec![PathBuf::from(r"C:\PF").join("Suwen")]
        );
    }

    #[test]
    fn candidate_dirs_dedups_identical_bases() {
        let dirs = candidate_dirs(Some("/opt".into()), Some("/opt".into()));
        assert_eq!(dirs.len(), 1);
    }
}
```

然后在 `src/client/src/main.rs` 的模块声明区(`mod elevate;` 一行附近)新增:

```rust
mod suwen;
```

- [ ] **Step 2: 跑测试确认失败/编译**

Run: `cargo test -p client suwen::tests 2>&1 | tail -20`
Expected: 首次可能因 `mod suwen;` 未加或路径笔误而编译失败;修正后进入 Step 3。

- [ ] **Step 3: 修正至通过(实现已在 Step 1)**

确保 `mod suwen;` 已加、文件无语法错误。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client suwen::tests 2>&1 | tail -20`
Expected: PASS,3 个测试全绿。

- [ ] **Step 5: 宿主编译确认(非 Windows 分支不炸)**

Run: `cargo check -p client 2>&1 | tail -20`
Expected: 通过(`suwen.rs` 未被 windows 代码引用的项由 `#![cfg_attr(not(windows), allow(dead_code))]` 压警)。

- [ ] **Step 6: 提交**

```bash
git add src/client/src/suwen.rs src/client/src/main.rs
git commit -m "feat(suwen): AI助手模块纯逻辑核心(路径候选/检测)+单测+注册"
```

---

## Task 2: 复用 `update::build_agent`(提为 `pub(crate)`)

**Files:**
- Modify: `src/client/src/update.rs:282`(windows 变体)与 `src/client/src/update.rs:292`(非 windows 变体)

- [ ] **Step 1: 改可见性**

把 `update.rs` 两处函数签名的 `fn build_agent` 改为 `pub(crate) fn build_agent`:

```rust
#[cfg(windows)]
pub(crate) fn build_agent(connect_s: u64, read_s: u64) -> ureq::Agent {
```
```rust
#[cfg(not(windows))]
pub(crate) fn build_agent(connect_s: u64, read_s: u64) -> ureq::Agent {
```

- [ ] **Step 2: 编译确认(两平台)**

Run: `cargo check -p client 2>&1 | tail -5 && cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -5`
Expected: 均通过。

- [ ] **Step 3: 提交**

```bash
git add src/client/src/update.rs
git commit -m "refactor(update): build_agent 提为 pub(crate) 供 suwen 复用"
```

---

## Task 3: Windows 下载/安装/等待/拉起(`win` 子模块)

**Files:**
- Modify: `src/client/src/suwen.rs`(在 `is_installed()` 之后、`#[cfg(test)]` 之前插入 `#[cfg(windows)] mod win`)

- [ ] **Step 1: 加 `win` 子模块实现**

在 `suwen.rs` 的 `pub fn is_installed()` 函数之后插入:

```rust
#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 同 exec.rs:38,静默不弹黑框
    const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);
    const POLL_INTERVAL: Duration = Duration::from_millis(500);
    const DOWNLOAD_CAP: u64 = 50 * 1024 * 1024;

    /// 下载素问安装器到 %TEMP% 临时文件,返回临时路径。
    /// 复用 update::build_agent(必须显式 SChannel,否则 ureq 报 "no TLS backend")+ CapReader(50MB 上限)。
    pub fn download_setup() -> anyhow::Result<tempfile::TempPath> {
        let agent = crate::update::build_agent(10, 300);
        let resp = agent.get(SETUP_URL).call()?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".suwen-setup-")
            .suffix(".exe")
            .tempfile_in(std::env::temp_dir())?;
        let mut reader = crate::update::CapReader::new(resp.into_reader(), DOWNLOAD_CAP);
        std::io::copy(&mut reader, tmp.as_file_mut())?;
        Ok(tmp.into_temp_path())
    }

    /// 运行安装器静默安装(/S)。客户端已提权,子进程继承管理员令牌。等待退出并校验退出码。
    pub fn run_installer(setup: &Path) -> anyhow::Result<()> {
        let status = Command::new(setup)
            .arg("/S")
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        if !status.success() {
            anyhow::bail!("安装器退出码非 0:{status}");
        }
        Ok(())
    }

    /// 轮询等待 daemon.exe 落盘(安装真正完成),超时报错。
    /// /S 为异步:单看安装器退出不足以保证文件就绪,故双条件(退出成功 + 文件出现)。
    pub fn wait_installed() -> anyhow::Result<()> {
        let deadline = Instant::now() + INSTALL_TIMEOUT;
        loop {
            if is_installed() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("安装超时:{}s 内未见 {DAEMON_EXE}", INSTALL_TIMEOUT.as_secs());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 拉起素问 GUI(不 wait、不加窗口 flag)。工作目录设为安装目录。
    pub fn launch_gui() -> anyhow::Result<()> {
        let gui = find_exe(GUI_EXE).ok_or_else(|| anyhow::anyhow!("未找到 {GUI_EXE}"))?;
        let mut cmd = Command::new(&gui);
        if let Some(dir) = gui.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn()?;
        Ok(())
    }
}
```

- [ ] **Step 2: Windows 交叉编译确认**

Run: `cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -20`
Expected: 通过(`win` 模块暂未被调用,dead_code 由顶层 `allow` 压警)。

- [ ] **Step 3: 宿主编译确认(non-windows 不含 win 模块)**

Run: `cargo check -p client 2>&1 | tail -5`
Expected: 通过。

- [ ] **Step 4: 提交**

```bash
git add src/client/src/suwen.rs
git commit -m "feat(suwen): Windows 下载(SChannel)/静默安装/等待/拉起 win 子模块"
```

---

## Task 4: 状态机编排 `ensure_and_launch` + 跨线程回写 + 非 Windows 空实现

**Files:**
- Modify: `src/client/src/suwen.rs`(在 `#[cfg(windows)] mod win` 之后、`#[cfg(test)]` 之前插入)

- [ ] **Step 1: 加编排函数(Windows)与非 Windows 空实现**

在 `suwen.rs` 的 `mod win` 块之后插入:

```rust
/// UI 线程调用:后台线程编排「检测→[下载→安装→等待]→拉起」,全程回写 suwen_phase/suwen_status。
/// 防重入:UI 侧按钮 enabled 仅在 phase 0/4 可点;此处 AtomicBool 二次兜底。
#[cfg(windows)]
pub fn ensure_and_launch(ui: slint::Weak<crate::AppWindow>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return; // 已有任务在跑,忽略连点
    }
    std::thread::spawn(move || {
        match run_flow(&ui) {
            Ok(()) => set_state(&ui, PHASE_IDLE, String::new()),
            Err(e) => {
                tracing::warn!("素问部署失败:{e:#}");
                set_state(&ui, PHASE_FAILED, format!("失败:{e}"));
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// 线性状态机主体。已安装则跳过下载/安装直接拉起。
#[cfg(windows)]
fn run_flow(ui: &slint::Weak<crate::AppWindow>) -> anyhow::Result<()> {
    if !is_installed() {
        set_state(ui, PHASE_DOWNLOAD, "正在下载素问…".into());
        let setup = win::download_setup()?;
        set_state(ui, PHASE_INSTALL, "正在安装素问…".into());
        win::run_installer(&setup)?;
        win::wait_installed()?;
        // setup(TempPath)在此 drop:安装已完成,可安全删除临时安装包。
    }
    set_state(ui, PHASE_LAUNCH, "正在启动素问…".into());
    win::launch_gui()?;
    Ok(())
}

/// 跨线程回写 UI 相位与状态文本(AppWindow 句柄非 Send,必须经事件循环)。
#[cfg(windows)]
fn set_state(ui: &slint::Weak<crate::AppWindow>, phase: i32, status: String) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui.upgrade() {
            // set_suwen_* 为 Slint 生成的固有方法;upgrade() 为 Weak 固有方法,均无需 ComponentHandle。
            ui.set_suwen_phase(phase);
            ui.set_suwen_status(status.into());
        }
    });
}

/// 非 Windows:按钮已被 suwen_supported=false 隐藏,此处兜底空实现(保证跨平台可编译)。
#[cfg(not(windows))]
pub fn ensure_and_launch(_ui: slint::Weak<crate::AppWindow>) {}
```

> 注:`set_suwen_phase` / `set_suwen_status` 由 Task 5 在 `app.slint` 声明的 `in property` 自动生成;故本 Task 的 Windows 交叉编译**依赖 Task 5 先落**。若按序执行,先做 Task 5 再回来编译本 Task,或本步与 Task 5 合并验证。执行顺序见末尾说明。

- [ ] **Step 2: 宿主编译确认(非 Windows 空实现)**

Run: `cargo check -p client 2>&1 | tail -10`
Expected: 通过(非 Windows 只编译空实现,不触碰 `set_suwen_*`)。

- [ ] **Step 3: 提交**

```bash
git add src/client/src/suwen.rs
git commit -m "feat(suwen): ensure_and_launch 状态机编排+跨线程回写+非Windows空实现"
```

---

## Task 5: Slint UI —— 属性/回调 + 右下角「AI助手」按钮

**Files:**
- Modify: `src/client/ui/app.slint`(两处插入)

- [ ] **Step 1: 声明属性与回调**

在 `app.slint` 中 `callback check_update();`(约 `:572`)一行**之后**插入:

```slint
    // ── 被控端「AI助手」:检测/静默安装/拉起素问(仅 Windows)──
    in property <bool> suwen_supported: false;   // 仅 Windows 为 true,控制按钮可见
    in property <int> suwen_phase: 0;            // 0空闲 1下载中 2安装中 3启动中 4失败
    in property <string> suwen_status;           // 进度/失败详情(日志与排障用)
    callback launch_suwen();                     // 点击:确保素问已装并拉起
```

- [ ] **Step 2: 加右下角悬浮按钮**

在 `app.slint` 尾部,被控端弹卡的 `Timer { ... running: root.controlled_toast_visible; ... }` 块**结束之后**、`function try-start()`(约 `:1706`)**之前**,插入一个 AppWindow 顶层子元素:

```slint
    // 被控端右下角「AI助手」悬浮按钮:仅 idle 面板 + Windows 显示。
    // 占满窗口仅为把按钮定位到右下角;TouchArea 只在按钮 Rectangle 内,不遮挡背景。
    if !root.remote_active && root.suwen_supported: Rectangle {
        VerticalLayout {
            alignment: end;
            HorizontalLayout {
                alignment: end;
                padding: 16px;
                Rectangle {
                    width: 128px;
                    height: 40px;
                    border-radius: 10px;
                    background: sta.has-hover ? Theme.primary-hover : Theme.primary;
                    border-width: root.suwen_phase == 4 ? 1px : 0px;
                    border-color: Theme.amber;
                    animate background { duration: 120ms; }
                    Text {
                        text: root.suwen_phase == 1 ? "下载中…"
                            : root.suwen_phase == 2 ? "安装中…"
                            : root.suwen_phase == 3 ? "启动中…"
                            : root.suwen_phase == 4 ? "失败,点击重试"
                            : "AI助手";
                        color: white;
                        font-size: 13px;
                        font-weight: 600;
                        horizontal-alignment: center;
                        vertical-alignment: center;
                    }
                    sta := TouchArea {
                        mouse-cursor: pointer;
                        // 进行中(1/2/3)禁用防重入;空闲(0)或失败(4)可点
                        enabled: root.suwen_phase == 0 || root.suwen_phase == 4;
                        clicked => { root.launch_suwen(); }
                    }
                }
            }
        }
    }
```

- [ ] **Step 3: Windows 交叉编译确认(Slint 生成器 + Task 4 setter 打通)**

Run: `cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -20`
Expected: 通过(`set_suwen_phase`/`set_suwen_status` 已由新 `in property` 生成,Task 4 的 Windows 代码此时可编译)。

- [ ] **Step 4: 宿主编译确认**

Run: `cargo check -p client 2>&1 | tail -5`
Expected: 通过。

- [ ] **Step 5: 提交**

```bash
git add src/client/ui/app.slint
git commit -m "feat(suwen): 右下角AI助手按钮+suwen相位/状态属性与回调"
```

---

## Task 6: `ui_glue.rs` 绑定回调 + 置平台支持位

**Files:**
- Modify: `src/client/src/ui_glue.rs`(在 `wire_ui_callbacks` 内,`on_check_update` 绑定块 `:308-318` 之后插入)

- [ ] **Step 1: 绑定 `on_launch_suwen` 并设 `suwen_supported`**

在 `ui_glue.rs` 的 `on_check_update` 绑定块(以 `crate::update::nudge();` 结尾、约 `:318` 的 `}` )**之后**插入:

```rust
    // AI助手:检测/静默安装/拉起素问。仅 Windows 显示按钮(其余平台 supported=false 隐藏)。
    {
        ui.set_suwen_supported(cfg!(windows));
        let ui_weak = ui.as_weak();
        ui.on_launch_suwen(move || {
            crate::suwen::ensure_and_launch(ui_weak.clone());
        });
    }
```

- [ ] **Step 2: 宿主编译确认**

Run: `cargo check -p client 2>&1 | tail -10`
Expected: 通过(非 Windows 调用空实现)。

- [ ] **Step 3: Windows 交叉编译确认**

Run: `cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -20`
Expected: 通过(整链打通:按钮→launch_suwen→ensure_and_launch→win 模块)。

- [ ] **Step 4: 提交**

```bash
git add src/client/src/ui_glue.rs
git commit -m "feat(suwen): ui_glue 绑定 launch_suwen + 按平台置 suwen_supported"
```

---

## Task 7: 全量验证 + Windows 真机手动测试

**Files:** 无代码改动(仅验证)。

- [ ] **Step 1: 单元测试 + 双平台编译门**

Run:
```bash
cargo test -p client suwen 2>&1 | tail -15
cargo check -p client 2>&1 | tail -5
cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -5
```
Expected: 测试全绿;两平台 check 均通过。

- [ ] **Step 2: Windows 真机手动测试(在已装 Windows 客户端上)**

逐项核对(每项通过打勾):
1. **未装素问** → 点「AI助手」→ 按钮文案依次「下载中…→安装中…→启动中…」→ `C:\Program Files\Suwen\` 出现 `suwen-daemon.exe` 与 `suwen-gui.exe` → 素问 GUI 拉起。
2. **已装素问** → 点按钮 → 跳过下载,直接「启动中…」→ 素问 GUI 拉起。
3. **断网** → 点按钮 → 文案变「失败,点击重试」(红边),按钮恢复可点;`RUST_LOG`/tracing 有 `素问部署失败:...`。
4. **连点按钮** → 进行中(1/2/3)按钮禁用,仅触发一次(AtomicBool + enabled 双闸)。
5. **主控画面态**(`remote_active=true`)→ 按钮隐藏,不干扰工作台。
6. **兜底目录**:若素问实际装在 `Program Files (x86)\Suwen`,检测与拉起仍成功。

- [ ] **Step 3: 无二次 UAC 验证**

因客户端启动已提权,点按钮触发安装时**不应**再弹 UAC。若弹了,说明客户端未提权运行,回查 `ensure_elevated`。

- [ ] **Step 4: 收尾提交(若手动测试中有微调)**

```bash
git add -A && git commit -m "test(suwen): Windows 真机手动测试通过,微调收尾"
```

---

## 执行顺序说明(重要)

Task 4 的 Windows 代码引用 `set_suwen_phase/set_suwen_status`(由 Task 5 的 Slint `in property` 生成)。因此 **Windows 交叉编译门在 Task 5 落地后才会绿**。两种可行执行法:
- **推荐**:按 1→2→3→4→5→6 顺序写代码,Task 4 只跑「宿主 check」(Step 2),把 Task 4 的「Windows check」延到 Task 5 Step 3 一并验证。
- 或:把 Task 4 与 Task 5 合并为一次提交后再跑 Windows check。

## 自审记录(spec 覆盖 / 占位符 / 类型一致性)

- **spec 覆盖**:§3 路径/检测/提权/完成判定→Task1+Task3;§4 模块拆分→Task1/3/4;§5 状态机→Task4;§6 UI→Task5/6;§7 错误处理→Task4 `set_state(PHASE_FAILED)`;§8 跨平台→Task4 空实现 + Task6 `cfg!(windows)`;§10 测试→Task1/Task7。§9 验签(可选)与 §11 `DETACHED_PROCESS`(可选)按设计**不在 v1**,未列任务(符合 YAGNI)。
- **占位符**:无 TODO/TBD;每步含完整可编译代码与确切命令。
- **类型一致性**:`ensure_and_launch(slint::Weak<crate::AppWindow>)`、`PHASE_*: i32`、`suwen_phase: int`、setter 名 `set_suwen_phase/set_suwen_status/set_suwen_supported`、回调名 `launch_suwen`/`on_launch_suwen` 全链一致。
