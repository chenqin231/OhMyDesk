# ui_glue.rs 拆分重构 — 设计文档

> 日期：2026-07-10
> 状态：已批准设计，待写实现计划
> 分支：`refactor/ui-glue-split`（基于 `feature/remote-control-chinese-ime` HEAD）
> 范围：仅 `src/client/src/ui_glue.rs`（1466 行）拆成模块目录。**不碰 `app.slint`（另列未来项）。**

## 1. 问题

`src/client/src/ui_glue.rs` 1466 行，超 800 行阈值（code-review 标 MEDIUM）。根因是两个巨函数：
- `wire_ui_callbacks()`（119-665，**546 行**）：50+ 个 `ui.on_*` 回调绑定塞一处，横向散、纵向臃。
- `consume_to_ui()`（858-1304，**445 行**）：13 个网络事件 match arm，每 arm 20-50 行。

其余已按函数分好（`wire_login_callbacks`/`wire_chat_notice_callbacks`/`wire_repaint_on_restore`），只是同挤一个文件。

## 2. 决策：拆模块目录 + `UiCtx` 句柄打包（Approach A）

`ui_glue.rs` → `ui_glue/` 目录。**公共 API 签名全不变**（main.rs 零改），靠 `mod.rs` re-export。共享句柄用 `UiCtx` 结构体打包传递。

**为何 `UiCtx`**：`wire_ui_callbacks` 的 6 个句柄（`from_ui_tx` / `cur_session` / `ctrl_session` / `ended_session` / `activity` / `telemetry_tx`）几乎每个闭包都捕获。打包成一个可 clone 的结构体，sub-wire 函数签名从「7 参数」降到 `fn wire_input(ui: &AppWindow, cx: &UiCtx)`，闭包内按需 `cx.field.clone()`。真可读性收益，非纯搬运。

**否决 Approach B**（直传 6 参数）：签名啰嗦、每处重复 6 参数。
**否决「同文件仅 extract-method」**：函数变小但文件仍 1466，不解决 >800 行问题。

## 3. 目标文件布局

| 文件 | 内容 | 来源行 |
|---|---|---|
| `ui_glue/mod.rs` | `UiCtx` 定义 + `wire_ui_callbacks`（建 ctx → 调 sub-wire）+ `pub use` re-export（util 全部 + login/chat_notice/restore/ui_update 的公共入口） | 119-127 壳 |
| `ui_glue/util.rs` | `group_digits`/`rel_time`/`build_history_model`/`build_file_model`/`resolve_path_arg`/`parent_of`/`join_path`/`append_line` **+ 对应单测** | 16-116 + 测试 |
| `ui_glue/control.rs` | `wire_control(ui, cx)`：授权同意/拒绝、远程 B 发起/断开/取消、被控断开 | 128-226,431-461 |
| `ui_glue/input.rs` | `wire_input(ui, cx)`：键盘(on_key_ev/on_text)、指针移动/按键、滚轮、剪贴板、画质 SetQuality | 227-424 |
| `ui_glue/features.rs` | `wire_features(ui, cx)`：文件浏览/传输、命令执行、聊天双角色、更新检查、素问、tab 切换、密码刷新、渲染模式、遥测 | 462-664 |
| `ui_glue/login.rs` | `wire_login_callbacks(...)`（签名不变） | 690-765 |
| `ui_glue/chat_notice.rs` | `wire_chat_notice_callbacks(...)`（签名不变） | 767-801 |
| `ui_glue/restore.rs` | `wire_repaint_on_restore(...)`（签名不变） | 803-842 |
| `ui_glue/ui_update.rs` | `consume_to_ui(...)`（签名不变）分发器 + 13 个 per-event 私有 handler 函数 | 858-1304 |

**结果**：`wire_ui_callbacks` 546 → 薄分发器（建 `UiCtx`，调 3 个 wire_*）；`consume_to_ui` 445 → 薄 match（每 arm 调一 handler）；最大文件 `ui_update.rs` ~450 行，其余 <250。

## 4. `UiCtx` 定义

```rust
/// 采集侧回调共享句柄打包。全字段皆 Arc/Sender，clone 廉价。
struct UiCtx {
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: SharedSession,
    ctrl_session: SharedSession,
    ended_session: SharedSession,
    activity: std::sync::Arc<crate::activity::ClientActivityState>,
    telemetry_tx: tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
}
```

`wire_ui_callbacks` 签名不变，函数体内先由入参构造 `UiCtx`，再依次 `control::wire(ui, &cx)` / `input::wire(ui, &cx)` / `features::wire(ui, &cx)`。各 sub-wire 内每个闭包 `let tx = cx.from_ui_tx.clone(); let sess = cx.cur_session.clone();` —— 与现有闭包内 clone 模式一致，纯搬运。

## 5. 铁律：纯行为保持（零行为变更）

- **每个闭包体逐字节搬运**，只改「住在哪个文件」+「句柄从哪来（入参→`cx.field`）」。不动逻辑、不改语义、不顺手优化。
- 公共 API 签名（第 3 节标"签名不变"的 5 个 + util 8 个）**一字不改**，main.rs 零改。
- `mod.rs` 用 `pub use` 保证 `ui_glue::group_digits` / `ui_glue::consume_to_ui` 等旧路径全部仍解析。

## 6. 测试与验证

- **既有单测跟随搬运**：util 的 10 个单测（path/frame/session 逻辑）搬进 `ui_glue/util.rs`（或就近模块），**必须保持全绿**——它们是「纯搬运没搞坏」的护栏。
- **验证门**（CI/Linux 可全跑，无需真机）：
  1. `cargo build -p client` 通过。
  2. `cargo test -p client` 全绿（含搬运后的既有测试）。
  3. `cargo clippy -p client`：不得**新增**警告（既有无关警告不管）。
  4. `cargo fmt -p client -- --check` 通过。
- **审查**：diff 应呈现为「近似纯移动」——审查者重点核对无逻辑改动、无闭包捕获错配（如 clone 错 Arc）。

## 7. 分支与合并序

- 分支 `refactor/ui-glue-split` 基于 `feature/remote-control-chinese-ime` HEAD（因 `ui_glue.rs` 已含 IME 的 `on_key_ev`/`on_text` 绑定，必须基于当前版本，否则搬运会丢这些改动）。
- **合并序：IME 分支先合（过 Windows 验收门后），本重构分支再合。** 二者改同一文件，耦合固有。

## 8. 非目标（YAGNI）

- 不碰 `app.slint`（GUI 单体拆分风险高、需真机验收，另开 spec）。
- 不改任何回调的行为/语义/信号。
- 不改公共 API 签名。
- 不引入 `UiCtx` 之外的新抽象（不做 trait 化、不做宏化回调注册）。
- 不顺手修既有 clippy 警告（保持 diff 纯净）。
