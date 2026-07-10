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

## 3. 目标文件布局（对齐 modularity.md：每文件 ≤300，目标 100-200）

> 依据 `.agent/rules/modularity.md`：硬限 >500 阻塞、强制 301-500、目标 100-200；单函数 >50 行提独立文件；多职责必拆。故 `consume_to_ui`(445) 与多职责的 features 组必须再拆细，不能停在「≤450」。

| 文件路径 | 职责（一句话） | 预估行数 | 新建/修改 | 来源行 |
|---|---|---|---|---|
| `ui_glue/mod.rs` | `UiCtx` 定义 + `wire_ui_callbacks`（建 ctx → 调 6 个 sub-wire）+ `pub use` re-export | ~90 | 新建 | 119-127 壳 |
| `ui_glue/util.rs` | 纯工具：id 分组/相对时间/两个 model 构造/path 三函数/append_line **+ 既有单测（测试豁免计数）** | ~110 | 新建 | 16-116 |
| `ui_glue/control.rs` | `wire_control`：授权同意/拒绝、远程 B 发起/断开/取消、被控断开 | ~140 | 新建 | 128-226,431-461 |
| `ui_glue/input.rs` | `wire_input`：键盘(on_key_ev/on_text)、指针移动/按键、滚轮、画质 SetQuality | ~170 | 新建 | 227-388 |
| `ui_glue/files.rs` | `wire_files`：文件浏览本地/远程、传输 push/pull | ~100 | 新建 | 504-596 |
| `ui_glue/exec.rs` | `wire_exec`：远程命令执行 | ~40 | 新建 | 476-503 |
| `ui_glue/chat.rs` | `wire_chat`：聊天双角色 | ~50 | 新建 | 597-640 |
| `ui_glue/misc.rs` | `wire_misc`：剪贴板、更新检查、素问、tab 切换、密码刷新、渲染模式、遥测 | ~120 | 新建 | 389-475,641-664 |
| `ui_glue/login.rs` | `wire_login_callbacks`（签名不变） | ~80 | 新建 | 690-765 |
| `ui_glue/chat_notice.rs` | `wire_chat_notice_callbacks`（签名不变） | ~40 | 新建 | 767-801 |
| `ui_glue/restore.rs` | `wire_repaint_on_restore`（签名不变） | ~40 | 新建 | 803-842 |
| `ui_glue/ui_update/mod.rs` | `consume_to_ui`（签名不变）薄分发器：解帧丢弃逻辑 + match → 各 handler | ~90 | 新建 | 858-890 |
| `ui_glue/ui_update/conn.rs` | 连接类事件：Registered/ControlRequest/BeingControlled/RemoteAck/RemoteRejected/Disconnected/AuthExpired | ~160 | 新建 | 891-973,1139-1159 |
| `ui_glue/ui_update/frame.rs` | 画面：Frame JPEG 解码+渲染+窗口重排、Cursor 叠加 | ~140 | 新建 | 975-1080 |
| `ui_glue/ui_update/session.rs` | SessionEnded 清理 | ~60 | 新建 | 1082-1137 |
| `ui_glue/ui_update/transfer.rs` | 执行/文件类：ExecResult/RemoteEntries/FileProgress/FileNotice/PaneRefresh | ~120 | 新建 | 1162-1246 |
| `ui_glue/ui_update/chat_update.rs` | ChatIncoming/UpdateAvailable/UpdateStatus | ~80 | 新建 | 1248-1300 |

**结果**：`wire_ui_callbacks` 546 → ~90 薄分发器；`consume_to_ui` 445 → `ui_update/mod.rs` ~90 薄分发 + 5 个 handler 文件；**每个产出文件 ≤~170 行，全部满足 ≤300、多数落在 100-200 或以下**。行数为预估，实现时以实际 ≤300 为硬门。`ui_update/` 因 >3 个关注点建子目录（modularity.md 决策树）。

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

`wire_ui_callbacks` 签名不变，函数体内先由入参构造 `UiCtx`，再依次调 6 个 sub-wire：`control::wire(ui, &cx)` / `input::wire(ui, &cx)` / `files::wire(ui, &cx)` / `exec::wire(ui, &cx)` / `chat::wire(ui, &cx)` / `misc::wire(ui, &cx)`。各 sub-wire 内每个闭包 `let tx = cx.from_ui_tx.clone(); let sess = cx.cur_session.clone();` —— 与现有闭包内 clone 模式一致，纯搬运。

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
  5. **行数合规（modularity.md 硬门）**：每个产出 `.rs` 文件 `wc -l` ≤300（测试文件豁免）；`ui_glue.rs` 旧单文件删除后不复存在。
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

## 9. 遗留债务与建议

- **`app.slint`（2065 行）仍违反 modularity.md 硬限**（4 倍），本次范围外，另开 spec（GUI 拆分需真机验收）。列为已知债务，勿视作合规。
- **流程账**：本轮 IME 功能往已超限的 `ui_glue.rs`/`app.slint` 直接追加代码，违反 modularity.md「禁止在超限文件追加、改 >300 行文件先拆再改」。正确序应先拆后加；已做反。IME 已审已过不回炉，本重构即补拆分欠账。
- **建议（防再漂）**：加一个 line-count 强制 hook（pre-commit 或 CI），对非豁免 `.rs`/`.slint` 超 500 行阻断——规则未机器化是历史漂移的根因。此建议独立于本重构，待用户决定是否单独做。
