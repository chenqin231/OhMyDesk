# ui_glue.rs 拆分重构 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 1466 行的 `src/client/src/ui_glue.rs` 拆成 `ui_glue/` 模块目录，每文件 ≤300 行（对齐 `.agent/rules/modularity.md` 硬限），公共 API 与行为零变更。

**Architecture:** 纯代码搬运重构。`ui_glue.rs` → `ui_glue/mod.rs`，逐模块把代码块移进子文件。6 个采集回调组共享的句柄打包成 `UiCtx` 结构体，sub-wire 函数签名降为 `(ui, &cx)`。`consume_to_ui` 拆成 `ui_glue/ui_update/` 子目录（分发器 + 5 个 handler 文件）。公共函数用 `pub use` 从 `mod.rs` re-export，`main.rs` 零改。

**Tech Stack:** Rust 2021，Slint 1.17（`AppWindow`/`ChatNoticeWindow` 强句柄）、tokio mpsc/watch、cargo test/clippy/fmt。

---

## 全局约定（每个 Task 都适用）

**这是一次纯搬运，铁律：**
1. **逐字节搬运闭包/函数体**，只改两处：①代码住在哪个文件 ②句柄来源。**不改逻辑、不改语义、不顺手优化、不动既有 clippy 警告。**
2. **句柄来源转换规则**（仅 `wire_*` 采集回调适用）：原 `wire_ui_callbacks` 内各块顶部有 `let tx = from_ui_tx.clone();` / `let sess = cur_session.clone();` 等——把裸句柄名 `from_ui_tx`/`cur_session`/`ctrl_session`/`ended_session`/`activity`/`telemetry_tx` 改成 `cx.` 前缀（`cx.from_ui_tx.clone()` …）。闭包内部捕获的是本地 `tx`/`sess`，无需改。
3. **import 靠编译器引导**：每个新文件顶部先放已知 `use`（见各 Task），其余缺失的 `use super::...` / `use crate::...` 按 `cargo build` 报错逐个补齐。这是搬运的正常做法，不是占位。
4. **公共 API 签名一字不改**：`wire_ui_callbacks`、`wire_login_callbacks`、`wire_chat_notice_callbacks`、`wire_repaint_on_restore`、`consume_to_ui` 及 util 的 8 个 `pub fn` 签名全不变；靠 `mod.rs` 的 `pub use` 保证 `ui_glue::X` 旧路径仍解析。`main.rs` 不得改动。

**每个 Task 的验证门（除非该 Task 另注明）：**
- `cargo build -p client` 通过。
- `cargo test -p client` 通过，**测试数保持 187 passed**（搬运不增不减测试）。
- 该 Task 的 diff 是「近似纯移动」——无逻辑改动。

**背景事实（勿重新调研）：**
- `ui_glue.rs` 顶部 import：`use crate::{history, net, AppWindow, ChatNoticeWindow, FileEntry, HistoryItem, SharedSession};` + `use slint::{ComponentHandle, ModelRc, VecModel};` + `use std::sync::atomic::{AtomicBool, AtomicI32, Ordering as AtomicOrdering};`
- 文件级 static：`REFIT_PENDING`（11 行）、`LAST_RES_TIER`（13 行）——只被 Frame 处理用。
- `SharedSession` = `crate::SharedSession`（`main.rs:45` 定义的 `pub(crate) type`）。
- `main.rs` 调用点：`ui_glue::group_digits`、`ui_glue::build_history_model`、`ui_glue::build_file_model`、`ui_glue::wire_ui_callbacks`、`ui_glue::wire_chat_notice_callbacks`、`ui_glue::wire_login_callbacks`、`ui_glue::wire_repaint_on_restore`、`ui_glue::consume_to_ui`。
- `wire_login_callbacks` 参数是 `(ui, token_tx, server_url, active_server_url)`——**与 UiCtx 无关**，保持独立签名。
- `consume_to_ui` 参数 8 个：`(rx, ui_weak, chat_notice_weak, cur_session, ctrl_session, ended_session, activity, token_tx)`——**也不用 UiCtx**，其 handler 直接收所需 ref。

---

## Task 1: 转成目录模块（ui_glue.rs → ui_glue/mod.rs）

**Files:** rename `src/client/src/ui_glue.rs` → `src/client/src/ui_glue/mod.rs`

- [ ] **Step 1: git mv**

```bash
mkdir -p src/client/src/ui_glue
git mv src/client/src/ui_glue.rs src/client/src/ui_glue/mod.rs
```

- [ ] **Step 2: 验证 build+test**

`main.rs` 的 `mod ui_glue;` 会自动解析到 `ui_glue/mod.rs`，无需改。
Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 编译通过；`test result: ok. 187 passed`。

- [ ] **Step 3: 提交**

```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): ui_glue.rs 转为 ui_glue/ 目录模块（纯移动，零改动）"
```

---

## Task 2: 抽出 util.rs（纯函数 + 对应单测）

**Files:**
- Create: `src/client/src/ui_glue/util.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

**移入 util.rs 的函数**（从 mod.rs 剪切，源行号以 Task 1 后的 mod.rs 为准，内容不变）：
`group_digits`、`rel_time`、`build_history_model`、`build_file_model`、`resolve_path_arg`、`parent_of`、`join_path`、`append_line`（原 15–116 段）。

**移入 util.rs 测试模块的单测**（从 mod.rs 底部 `mod tests` 剪切）：
`路径父级_unix与windows`、`路径拼接_按分隔符`、`指令串解析_up与cd`、`聊天行追加`。

- [ ] **Step 1: 建 util.rs**

文件头：
```rust
//! ui_glue 纯工具：id 分组、相对时间、Slint model 构造、路径运算、聊天行追加。
//! 全部无副作用，可单测。

use crate::{history, net, FileEntry, HistoryItem};
use slint::{ModelRc, VecModel};
```
把上述 8 个 `pub fn` 原样粘入（保持 `pub`）。底部加：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    // 粘入上列 4 个 #[test] 函数，内容不变
}
```
若测试调用了 `net::now()` 等，`use super::*;` + 已有 import 即可；缺啥按编译报错补。

- [ ] **Step 2: 改 mod.rs**

删掉 mod.rs 中已移走的 8 个 fn 与 4 个测试。在 mod.rs 顶部（import 之后）加：
```rust
mod util;
pub use util::{
    append_line, build_file_model, build_history_model, group_digits, join_path, parent_of,
    rel_time, resolve_path_arg,
};
```
mod.rs 内其余代码若调用这些 fn，`pub use` 后同模块内可直接用名字（已 re-export 到本模块作用域）；如报未解析，改成 `util::NAME` 或确认 `pub use` 生效。

- [ ] **Step 3: 验证**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。

- [ ] **Step 4: 提交**

```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 抽出 ui_glue/util.rs（纯工具+单测，零改动）"
```

---

## Task 3: 引入 UiCtx + 抽出 control.rs

**Files:**
- Create: `src/client/src/ui_glue/control.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

本 Task 确立 `UiCtx` 模式。`control.rs` 承载：授权同意/拒绝、远程 B 发起/断开/取消、被控断开（原 128–226 与 431–461 段的回调），外加纯函数 `validate_remote_target`（原 1330–1342）及其单测 `远控目标校验_拒绝自连并保留空目标提示`。

- [ ] **Step 1: 在 mod.rs 定义 UiCtx，改造 wire_ui_callbacks 为分发器**

在 mod.rs（`wire_ui_callbacks` 上方）加：
```rust
/// 采集侧回调共享句柄打包。全字段皆 Arc/Sender，clone 廉价。
pub(crate) struct UiCtx {
    pub from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    pub cur_session: SharedSession,
    pub ctrl_session: SharedSession,
    pub ended_session: SharedSession,
    pub activity: std::sync::Arc<crate::activity::ClientActivityState>,
    pub telemetry_tx: tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
}
```
把 `wire_ui_callbacks` 函数体改为：先由入参构造 `UiCtx`，再调 sub-wire。本 Task 先只接 control，其余仍留在函数体内：
```rust
pub fn wire_ui_callbacks(
    ui: &AppWindow,
    from_ui_tx: &tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    telemetry_tx: &tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
    let cx = UiCtx {
        from_ui_tx: from_ui_tx.clone(),
        cur_session: cur_session.clone(),
        ctrl_session: ctrl_session.clone(),
        ended_session: ended_session.clone(),
        activity: activity.clone(),
        telemetry_tx: telemetry_tx.clone(),
    };
    control::wire(ui, &cx);
    // ↓ 其余回调（input/files/exec/chat/misc）暂仍留此，后续 Task 逐个外迁。
    //   这些块现引用 cx.<field>，把原裸句柄名改成 cx. 前缀（全局约定 2）。
    // ...（input/files/exec/chat/misc 的原有代码，句柄改 cx.）
}
```
注意：留在函数体内的其余块，其顶部 `let tx = from_ui_tx.clone();` 需改为 `cx.from_ui_tx.clone()`（因入参已被 move 进 cx；或改为在构造 cx 前不 move、构造后用 cx）。统一走 cx 最简。

- [ ] **Step 2: 建 control.rs**

文件头：
```rust
//! 采集回调·控制域：授权同意/拒绝、远程发起/断开/取消、被控断开。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 粘入原 128–226 与 431–461 的回调块（句柄改 cx.）
}
```
把 `validate_remote_target`（原 1330–1342，含 `#[cfg(test)]` 之外的定义）移到 control.rs 作 `fn validate_remote_target(...)`（若被 control 回调用则加 `pub(super)` 视情况），并把测试 `远控目标校验_...` 放入 control.rs 的 `#[cfg(test)] mod tests`。

- [ ] **Step 3: mod.rs 声明子模块**

在 mod.rs 加 `mod control;`。删掉已移走的 `validate_remote_target` 及其测试、以及移进 control::wire 的授权/断开回调块。

- [ ] **Step 4: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 引入 UiCtx + 抽出 ui_glue/control.rs（零改动）"
```

---

## Task 4: 抽出 input.rs

**Files:**
- Create: `src/client/src/ui_glue/input.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

`input.rs` 承载：键盘（`on_on_key_ev`/`on_on_text`）、指针移动/按键、滚轮、画质 SetQuality（原 227–388 段）。

- [ ] **Step 1: 建 input.rs**

```rust
//! 采集回调·输入域：键盘(on_key_ev/on_text)、指针、滚轮、画质。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 粘入原 227–388 的回调块（句柄改 cx.；含调用 crate::key_route::key_route 的键盘块）
}
```
键盘块内 `crate::key_route::key_route(...)` 路径不变。

- [ ] **Step 2: mod.rs 接线**

mod.rs 加 `mod input;`；在 `wire_ui_callbacks` 分发器中 `control::wire(ui, &cx);` 之后加 `input::wire(ui, &cx);`；从函数体删掉已移走的输入块。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 抽出 ui_glue/input.rs（键鼠采集，零改动）"
```

---

## Task 5: 抽出 files.rs + exec.rs + chat.rs

**Files:**
- Create: `src/client/src/ui_glue/files.rs`、`src/client/src/ui_glue/exec.rs`、`src/client/src/ui_glue/chat.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

分域：`files.rs`=文件浏览本地/远程+传输 push/pull（原 504–596）；`exec.rs`=远程命令执行（原 476–503）；`chat.rs`=聊天双角色（原 597–640）。

- [ ] **Step 1: 建三个文件**

每个文件同构（`XXX` 替换为对应回调块，句柄改 `cx.`）：
```rust
//! 采集回调·<域名>。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 粘入对应域的回调块
}
```
- `files.rs` ← 原 504–596（浏览+传输）。若用到 `super::util::build_file_model` / `resolve_path_arg` / `parent_of` / `join_path`，加 `use super::util::{build_file_model, resolve_path_arg, parent_of, join_path};`。
- `exec.rs` ← 原 476–503。
- `chat.rs` ← 原 597–640。若用到 `super::util::append_line`，加 `use super::util::append_line;`。

- [ ] **Step 2: mod.rs 接线**

mod.rs 加 `mod files; mod exec; mod chat;`；分发器在 input 之后加 `files::wire(ui, &cx); exec::wire(ui, &cx); chat::wire(ui, &cx);`；删掉函数体内已移走的块。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 抽出 ui_glue/{files,exec,chat}.rs（零改动）"
```

---

## Task 6: 抽出 misc.rs（收尾 wire_ui_callbacks）

**Files:**
- Create: `src/client/src/ui_glue/misc.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

`misc.rs` 承载剩余采集回调：剪贴板、更新检查、素问、tab 切换、密码刷新、渲染模式、遥测（原 389–475、641–664 段）。

- [ ] **Step 1: 建 misc.rs**

```rust
//! 采集回调·杂项：剪贴板、更新检查、素问、tab 切换、密码刷新、渲染模式、遥测。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 粘入原 389–475、641–664 的回调块（句柄改 cx.）
}
```

- [ ] **Step 2: mod.rs 接线**

mod.rs 加 `mod misc;`；分发器末尾加 `misc::wire(ui, &cx);`；删净函数体内剩余回调块。此后 `wire_ui_callbacks` 只剩「构造 cx + 6 行 sub-wire 调用」。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 抽出 ui_glue/misc.rs，wire_ui_callbacks 收薄为分发器（零改动）"
```

---

## Task 7: 抽出 login.rs + chat_notice.rs + restore.rs

**Files:**
- Create: `src/client/src/ui_glue/login.rs`、`src/client/src/ui_glue/chat_notice.rs`、`src/client/src/ui_glue/restore.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

这三个是已独立的公共函数，直接整体外迁（签名不变，非 UiCtx）：
- `login.rs` ← `wire_login_callbacks`（原 694–765）+ 测试 `登录服务器地址_高级项覆盖默认地址`。
- `chat_notice.rs` ← `wire_chat_notice_callbacks`（原 767–801）+ 测试 `被控端新消息_仅面板未打开时触发自绘通知`、`被控消息通知_自绘常驻并贴工作区右下角`（若这俩测试依赖 `show_controlled_chat_notice`，见 Task 9——本 Task 先把它们留在 mod.rs，Task 9 随 `show_controlled_chat_notice` 一起归位到 ui_update/chat_update.rs）。
- `restore.rs` ← `wire_repaint_on_restore`（原 809–842）。

- [ ] **Step 1: 建三文件**

各文件把对应 `pub fn` 原样粘入（保持 `pub`），文件头按需 `use crate::{...}; use slint::ComponentHandle;`。`login.rs` 的 `active_server_url: crate::SharedServerUrl` 类型路径不变。

- [ ] **Step 2: mod.rs 声明 + re-export**

mod.rs 加：
```rust
mod chat_notice;
mod login;
mod restore;
pub use chat_notice::wire_chat_notice_callbacks;
pub use login::wire_login_callbacks;
pub use restore::wire_repaint_on_restore;
```
删掉 mod.rs 中已移走的三个 fn 与 `登录服务器地址_...` 测试。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): 抽出 ui_glue/{login,chat_notice,restore}.rs（零改动）"
```

---

## Task 8: consume_to_ui → ui_update/ 子目录（整体移动）

**Files:**
- Create: `src/client/src/ui_glue/ui_update/mod.rs`
- Modify: `src/client/src/ui_glue/mod.rs`

先把 `consume_to_ui` 整体连同其私有依赖搬进 `ui_update/mod.rs`（本 Task 不切 handler，只搬家）：
- `consume_to_ui`（原 858–1304，签名不变）。
- 文件级 static `REFIT_PENDING`、`LAST_RES_TIER`（原 11、13）。
- 私有 fn `decode_frame_rgba`（原 1344–1352）、`show_controlled_chat_notice`（原 1306–1328）。
- 相关测试：`已断开会话的迟到帧应被丢弃`、`next_ctrl_session_门控清理`（若 `next_ctrl_session` 私有 fn 也在此文件被用）、`被控端新消息_...`、`被控消息通知_...`。

- [ ] **Step 1: 建 ui_update/mod.rs**

文件头：
```rust
//! ToUi 流消费：拉网络事件逐条应用到 UI（invoke_from_event_loop），维护会话 id、帧解码。
use crate::{net, AppWindow, ChatNoticeWindow, SharedSession};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering as AtomicOrdering};
```
把上列 static、`consume_to_ui`、`decode_frame_rgba`、`show_controlled_chat_notice` 及相关测试原样粘入。若这些代码调用 util 的 `build_file_model`/`build_history_model`/`append_line` 等，加 `use super::util::{...};`。若用到 `super::UiCtx` 则不需要（consume_to_ui 不吃 UiCtx）。

- [ ] **Step 2: mod.rs 声明 + re-export**

ui_glue/mod.rs 加：
```rust
mod ui_update;
pub use ui_update::consume_to_ui;
```
从 mod.rs 删掉已移走的 static、两个私有 fn、`consume_to_ui`、及相关测试。**注意 static 移走后**，mod.rs 顶部若不再用 `AtomicBool/AtomicI32`，删掉那行 `use std::sync::atomic::...`（避免 unused import 警告——这属于「移动导致的 import 清理」，非改逻辑）。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): consume_to_ui 迁入 ui_glue/ui_update/ 子目录（整体移动，零改动）"
```

---

## Task 9: 拆 ui_update handler（conn/frame/session/transfer/chat_update）

**Files:**
- Create: `src/client/src/ui_glue/ui_update/{conn,frame,session,transfer,chat_update}.rs`
- Modify: `src/client/src/ui_glue/ui_update/mod.rs`

把 `consume_to_ui` 里 `match ev { ... }` 的各 arm 按域提取为 handler 函数，`mod.rs` 的 match 只留「调 handler」。每个 handler 收它实际需要的 ref（`ui_weak: &slint::Weak<AppWindow>`、`chat_notice_weak`、`cur_session`/`ctrl_session`/`ended_session`、`activity`、`token_tx` 等——按各 arm 原本用到的取）。

分域（arm → 文件）：
- `conn.rs`：`Registered`、`ControlRequest`、`BeingControlled`、`RemoteAck`、`RemoteRejected`、`Disconnected`、`AuthExpired`。
- `frame.rs`：`Frame`（JPEG 解码+渲染+窗口重排）、`Cursor`。**把 static `REFIT_PENDING`/`LAST_RES_TIER` 与 `decode_frame_rgba` 移到此文件**（它们只服务 Frame）。
- `session.rs`：`SessionEnded`。**把 `next_ctrl_session` 私有 fn（若存在）与测试 `next_ctrl_session_门控清理`、`已断开会话的迟到帧应被丢弃` 移此**。
- `transfer.rs`：`ExecResult`、`RemoteEntries`、`FileProgress`、`FileNotice`、`PaneRefresh`。
- `chat_update.rs`：`ChatIncoming`、`UpdateAvailable`、`UpdateStatus`。**把 `show_controlled_chat_notice` 与测试 `被控端新消息_...`、`被控消息通知_...` 移此**。

- [ ] **Step 1: 建 5 个 handler 文件**

每个文件头示例（`frame.rs`）：
```rust
//! ui_update·画面事件：Frame 解码渲染重排、Cursor 叠加。
use crate::AppWindow;
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering as AtomicOrdering};

static REFIT_PENDING: AtomicBool = AtomicBool::new(false);
static LAST_RES_TIER: AtomicI32 = AtomicI32::new(-1);

// decode_frame_rgba 移入此处（私有）
// pub(super) fn handle_frame(ui_weak: &slint::Weak<AppWindow>, ...) { <原 Frame arm 体> }
// pub(super) fn handle_cursor(...) { <原 Cursor arm 体> }
```
其余文件同构，只放对应域 handler。每个 handler 的**函数体 = 原 match arm 的花括号内代码逐字节搬**，参数取该 arm 实际引用的变量。

- [ ] **Step 2: 改 ui_update/mod.rs 的 match 为薄分发**

`consume_to_ui` 内 `match ev` 各 arm 改成一行调用，例如：
```rust
net::ToUi::Frame { .. } => frame::handle_frame(&ui_weak, /* 原 arm 用到的其余变量 */),
net::ToUi::Cursor { .. } => frame::handle_cursor(&ui_weak, /* ... */),
net::ToUi::SessionEnded { .. } => session::handle_session_ended(/* ... */),
// ...每个事件一行
```
mod.rs 顶部加 `mod conn; mod frame; mod session; mod transfer; mod chat_update;`。删掉已移走的 static、`decode_frame_rgba`、`show_controlled_chat_notice`、相关测试。

> 提示：arm 内原本 `match ev { ToUi::Frame { a, b, .. } => { ... } }` 是带绑定的解构。改为 handler 调用时，在 arm 里解构出字段再传参：`net::ToUi::Frame { data, w, h, .. } => frame::handle_frame(&ui_weak, data, w, h, ...)`。保持字段与用法一致，不改语义。

- [ ] **Step 3: 验证 + 提交**

Run: `cargo build -p client && cargo test -p client 2>&1 | grep "test result"`
Expected: 通过；`187 passed`。
```bash
git add -A src/client/src/ui_glue
git commit -m "refactor(client): ui_update 拆 5 个 handler 文件，consume_to_ui 收薄为分发（零改动）"
```

---

## Task 10: 合规终验 + 收尾

**Files:** 只读校验 + 可能的 `cargo fmt` 改动。

- [ ] **Step 1: 行数硬门（modularity.md）**

Run:
```bash
find src/client/src/ui_glue -name '*.rs' -exec wc -l {} \; | sort -rn
```
Expected: 每个文件 ≤300 行。**若某文件仍 >300**（最可能是 `conn.rs` 或 `frame.rs`），按域再拆一层（如 frame 的 Frame 体单独成 `frame.rs`、Cursor 归 `cursor.rs`），重跑本步直至全绿。测试文件不豁免此处（这些是含 `#[cfg(test)]` 的实现文件，非独立 `*test*` 文件）。

- [ ] **Step 2: 确认旧文件消失 + 全门**

Run:
```bash
test ! -f src/client/src/ui_glue.rs && echo "旧单文件已不存在"
cargo build -p client
cargo test -p client 2>&1 | grep "test result"
cargo clippy -p client 2>&1 | grep -c "^warning"
cargo fmt -p client -- --check || cargo fmt -p client
```
Expected: 旧文件不存在；build 通过；`187 passed`；clippy 警告数**不高于重构前基线**（重构前基线：记录 Task 0 前的 `cargo clippy -p client 2>&1 | grep -c "^warning"`，终值不得更高）；fmt 干净或已格式化。

- [ ] **Step 3: 提交（若 fmt 有改动）**

```bash
git add -A src/client/src/ui_glue && git commit -m "chore(client): ui_glue 重构收尾 fmt" || echo "无需 fmt 提交"
```

- [ ] **Step 4: 最终自检**

- `main.rs` 未被改动（`git diff --stat da464f1..HEAD -- src/client/src/main.rs` 应只含 IME 分支的 `mod key_route;` 那一行，无本重构新增改动）。
- `ui_glue/` 下所有 `.rs` ≤300 行。
- 全程无逻辑改动，测试数恒为 187。

---

## 自查（Self-Review 已过）

- **spec 覆盖**：spec §3 的 17 文件全部对应 Task 2–9 的产出；§4 UiCtx→Task 3；§5 纯移动铁律→全局约定；§6 验证门→各 Task 验证步 + Task 10；§7 分支→已在 `refactor/ui-glue-split`；§8 非目标（不碰 app.slint/不改签名/不引额外抽象）→计划未触及；§9 债务→计划范围外。
- **占位符**：无 TBD/TODO；移动类步骤以「源行号段 + fn/测试名 + 转换规则」精确指定，非占位（搬运不适合粘贴 1400 行原体）。
- **签名一致**：`UiCtx` 字段、`wire(ui, &cx)` sub-wire 签名、re-export 名（`wire_login_callbacks`/`consume_to_ui` 等）全计划一致。
- **风险点**：Task 9 最重（match arm 解构→传参），提示已给「解构字段再传参、不改语义」。Task 10 Step 1 兜底「若仍 >300 再拆一层」。
