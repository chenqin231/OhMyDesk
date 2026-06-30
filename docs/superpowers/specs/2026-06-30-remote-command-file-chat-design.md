# 远程命令 / 远程文件 / 即时消息 — 设计文档

> **项目代号**：OhMyDesk
> **类型**：能力扩展（在 M2 远程控制之上）
> **状态**：设计待用户评审 → 进入实现规划
> **关系**：本文档扩展 [`2026-06-27-xinchuang-remote-control-design.md`](./2026-06-27-xinchuang-remote-control-design.md)。原设计把「文件传输、聊天」列为非目标（§3.2），本次为主动扩展，落地后需回写原文档目标/非目标章节。

---

## 1. 背景与问题

客户端（Slint Agent）当前主控端只能看远程桌面画面，缺命令行、文件传输；全平台缺会话内即时消息。代码调查关键结论（决定本次工作量分布）：

- **远程命令、文件传输的后端与 Web 端已存在**：协议层 `ExecRequest/ExecResult`、`FileOpen/FileChunk/FilePullRequest/FileError/FileDone/FileListRequest/FileListResp` 齐全；服务端 `route_to_peer` 路由 + `AuditType::Command/FileTransfer` 审计已就绪；**被控端**执行（`src/client/src/exec.rs`、`transfer.rs`）已实现；**admin-web 主控端**已有「远程控制 / 命令行 / 文件传输」三标签。
- **真正缺口在 Slint 客户端主控端**：其 `FromUi`/`ToUi` 枚举无任何 Exec/File 变体，会话工具条只有画质切换 + 断开——被控端早能执行/收发，但主控端没有发起这些操作的 UI 与上下行链路。
- **即时消息为全新功能**：全仓无任何聊天实现，需新增协议类型 + 服务端路由审计 + 两端 UI。

## 2. 目标 / 非目标

### 2.1 目标
1. Slint 客户端主控端（模式 B，客户端→客户端）补齐**远程命令**、**远程文件**，达到与 Web 端一致。
2. admin-web（模式 A，Web→客户端）**增强**现有命令/文件：文件传输进度条、命令历史回溯。
3. **即时消息**：模式 A、模式 B 远程会话内双向收发，全文落库审计、持久化 SQLite、管理后台可查。
4. 统一为「会话内多标签工作台」交互模型，桌面画面懒推流。

### 2.2 非目标（YAGNI）
- ❌ 交互式 shell / PTY（仍是一次性命令执行）
- ❌ 文件断点续传、文件夹递归传输、>50MB 大文件
- ❌ 会话外通讯录 / 离线消息 / 群聊
- ❌ 聊天图片、文件附件、表情、已读回执
- ❌ 真 P2P / 画质帧率优化

## 3. 架构总览：统一会话 + 懒推流

**核心模型**：一次授权建立会话后，桌面/命令/文件/消息四项能力全部可用；它们挂在**会话**（`session_id`）上，不挂在**桌面画面**上。

- **会话即授权通道**：建立会话需被控方授权（模式 B 对端同意 / 模式 A 管理员 `force` 强制 + `auto_accept`）。会话建立 = 四能力解锁。命令/文件/消息均门控在 active 会话内，按 `route_to_peer(session_id)` 路由。
- **桌面画面懒推流**：帧仅在主控端停留在「远程桌面」标签时推送；切到其他标签即暂停推流（省内网带宽 + 不必要地暴露被控方屏幕）。
  - 机制：主控端切标签时向被控端发「帧暂停/恢复」信号，被控端据此启停 `CAPTURE_CTRL`。实现复用现有 `SetQuality` 通道（新增暂停态）或新增轻量 `CaptureControl` 消息——二选一在实现规划阶段定。
- **授权入口不变**：沿用现有「请求远程控制」流程；连接成功默认落在「远程桌面」标签，用户自由切换。

```
                    一次授权 (route_to_peer 会话)
  主控端 ┌─────────────────────────────────────┐ 被控端
        │  [桌面]  帧懒推流 (仅桌面标签时)        │
        │  [命令]  ExecRequest → ExecResult     │
        │  [文件]  FileList/Open/Chunk/Pull     │
        │  [消息]  ChatMessage ⇄ ChatMessage    │
        └──────────────┬──────────────────────┘
                       │ 每条均落 audit_logs (SQLite)
                       ▼  Server: route_to_peer + audit.log
```

## 4. 协议层改动（`src/protocol/src/lib.rs`，单一事实源）

```rust
// Message 枚举新增（内部 tag #[serde(tag="type", rename_all="snake_case")]）
ChatMessage { session_id: String, msg_id: String, text: String },

// AuditType 新增（event_type 字符串 "chat"）
Chat,
```

- 命令/文件类型已存在，**不动**。
- 改完跑 `cargo test export_all` 自动导出 TS 到 `src/admin-web/src/lib/types/`（禁手改生成物）。
- `tests.rs` 补 `chat_message` 序列化 + tag 判别测试。
- 懒推流若选「新增 CaptureControl 消息」方案，则此处再加一个变体；若复用 SetQuality 则改 `QualityMode`。

## 5. 需求一·远程命令（Slint 主控端）

被控端 `exec.rs` 已实现（`cmd /C` / `sh -c`、GBK/UTF-8 解码、超时封顶 120s、输出截断 64KB）。**仅补主控端链路**：

| 层 | 改动 |
|----|------|
| `net/mod.rs` | `FromUi::ExecCommand{session_id, command}`；`ToUi::ExecResult{exec_id, command, exit_code, stdout, stderr, truncated, duration_ms}` |
| `net/dispatch.rs` | `handle_uplink` 映射 `ExecCommand` → 发 `ExecRequest`；`handle_downlink` 收 `ExecResult` → `ToUi::ExecResult` |
| `app.slint` | 「远程命令」标签页：命令输入行 + 输出卡片列表（执行中/退出码/耗时/截断标记）；命令历史 ↑↓ 回溯 |
| `ui_glue.rs` | callback 接线 |

**平台差异对用户透明**：被控端自动按 `cfg!(target_os)` 选 shell；主控端输入框占位提示 `whoami / ipconfig / ls`（信创终端为 Linux 命令）。

## 6. 需求一·远程文件（Slint 主控端，真实本地磁盘）

复用被控端 `transfer.rs`（`list_dir/open_recv/write_chunk/send_file/abort`）与全部 File* 协议。**左栏=本机真实目录树**（Slint 可读本地盘，优于浏览器暂存区），**右栏=远端目录**。

| 层 | 改动 |
|----|------|
| `net/mod.rs` | `FromUi`: `ListRemote{session_id, path}` / `PushFile{session_id, local_path, dest_dir}` / `PullFile{session_id, remote_path}`；`ToUi`: `RemoteEntries{path, entries}` / `FileProgress{name, done, total}` / `FileNotice{text}` |
| `net/dispatch.rs` | 上行映射 `FileListRequest/FileOpen+FileChunk*+FileDone/FilePullRequest`；下行处理 `FileListResp/FileChunk(取回回流)/FileError/FileDone` |
| `app.slint` | 「远程文件」标签页：双栏目录树 + 下发/取回按钮 + 传输进度条 |
| 路径处理 | 分隔符按远端 OS 适配（移植 `remote-tools.tsx` 的 `sep()/joinPath/parentPath/childPath/isDriveRoot` 逻辑到 Slint 侧） |

单文件 ≤ 50MB（沿用现限）。本地枚举用标准库 `std::fs::read_dir`。

## 7. 需求一·Web 端增强（`src/admin-web`）

admin-web 命令/文件已可用，补三处（保持标签式，不改弹窗）：

1. **文件传输进度条**：`FilePanel` 现仅文字 `fileNotice`，改为 per-file 百分比进度（store 增 `fileProgress` 状态，下行 `FileChunk/FileDone` 累计）。
2. **命令历史回溯**：`CommandPanel` 输入框支持 ↑↓ 调出历史命令（组件内 history 数组）。
3. **会话消息标签**：见 §8。

## 8. 需求二·即时消息（双向，会话内）

数据流（主控↔被控对称，模式 A/B 统一）：

```
发送方 ChatMessage(to:None, session_id) → Server
   Server: route_to_peer(session_id) → 对端
           audit.log(Chat, actor_id=from, text=消息全文)
```

| 端 | 改动 |
|----|------|
| `src/server/hub.rs` | `route_to_peer` 加 `ChatMessage` 分支 + `audit.log(...Chat...)` |
| `src/server/audit.rs` | `audit_type_str` 与 `AuditLogRow→AuditLog` 两处补 `Chat` 映射 |
| Slint 主控端 | 第 4 标签「即时消息」整页：聊天记录 + 输入行；非当前标签时标签显未读红点 |
| Slint 被控端 | 被控提示条加「聊天」入口，弹紧凑面板可回复（被控方是真人，需双向） |
| admin-web | 会话视图加第 4 标签「会话消息」`ChatPanel`；store 加 `sendChat`/`chatMessages` + 下行 `chat_message` 处理 |

消息全文写入 `audit_logs.text`，`actor_id` 区分发送方，天然支撑双向审计。

## 9. UX 设计（designer 人格：Rams「尽可能少」+ 一致性）

**风格基线**：沿用现有深色 token 与 shadcn 组件，**不引入新视觉风格**（一致性 > 个人创意）。Slint 端复用现有控件样式。

### 9.1 用户旅程

- **主控发起**：输入目标 ID+密码 → 授权通过 → 落「远程桌面」标签 → 自由切换四标签工作 → 断开。
- **被控收消息**：被控期间提示条「聊天」徽标亮起 → 点开面板看消息 → 回复 → 折叠。

### 9.2 线框图

```
Slint 主控端会话（顶部标签 · 整页 · 同一窗口）
┌────────────────────────────────────────────────┐
│ ●远程桌面 │ 远程命令 │ 远程文件 │ 即时消息 ③  [断开]│
├────────────────────────────────────────────────┤
│ 远程文件页:                                       │
│  本地(本机)            │  远端(被控)               │
│  /home/me        [↻]  │  C:\Users\         [↑][↻]│
│   📁 docs/            │   📁 Desktop/            │
│   📄 a.txt    [下发→] │   📄 b.log    [←取回]    │
│  ──────────────────────────────────────────────│
│  传输中 a.txt ▓▓▓▓▓░░░ 62%                       │
└────────────────────────────────────────────────┘

被控端（被控期间）
┌ 正在被 张三 控制 ───────────[聊天 ●]─[断开]┐
└──────────────────────────────────────────┘
 点开:
┌─ 与 张三 ───────────────────────[×]┐
│ 张三: 看下这个报错                   │
│ 我:   好的 ───────────────────────  │
│ [______________________________] [发送]│
└─────────────────────────────────────┘
```

### 9.3 状态设计（每个工具覆盖空/加载/成功/错误）

- **命令**：空（提示语）→ 执行中（spinner）→ 成功（输出+退出码+耗时）→ 错误（stderr 红字/超时）。
- **文件**：空目录提示 / 加载中 spinner / 列表 / 错误（点刷新恢复）；传输：进度条 → 完成/失败提示。
- **消息**：空（无消息提示）→ 消息流（左对端/右本端）→ 发送中/失败重试。

### 9.4 交互规范

- 标签切换即时无动画过场；非活跃工具标签有新内容时显小红点（未读消息/命令完成）。
- 文件传输进度条平滑过渡；命令输出区自动滚到底；聊天新消息自动滚底。
- 切到桌面标签恢复推流，切走暂停（懒推流，对用户无感）。

## 10. 审计与持久化

- 三类操作全部复用 `audit_logs` 表（SQLite，**无需改表**）。聊天写 `event_type="chat", text=消息全文, actor_id=发送方`。
- DB 连不上时审计 best-effort 降级，实时链路不受影响（沿用现有策略）。
- **审计页扩展**（`src/admin-web`）：`lib/adapters/audit.ts` 的 `summarize` 现仅统计 screenshot/input，需扩展识别 command/file/chat；`audit-cells.tsx` 加对应图标与文案；timeline 渲染分类补全。

## 11. 平台差异处理

| 维度 | 处理 |
|------|------|
| Shell | 被控端 `cfg!(target_os="windows")` → `cmd /C`，否则 `sh -c`（信创 Linux 终端）。已实现。 |
| 控制台编码 | 中文 Windows cmd 输出 GBK → 失败回退 GBK 解码；类 Unix lossy UTF-8。已实现。 |
| 路径分隔符 | 按远端绝对路径含 `\` 判 Windows，Slint 侧移植 Web 端 `sep()` 系列逻辑。 |
| 盘符根 | Windows `C:\` 向上回「此电脑」（空路径列盘符）。已在 Web 端实现，移植。 |

## 12. 错误处理与边界

- 协议坏包忽略不 panic；不对外部输入 `unwrap()`（项目红线）。
- 命令超时/启动失败 → 落 stderr 回传，不崩。
- 文件传输中途断开 → `FileError` + 清理半成品；进度条转失败态。
- 切标签/会话结束时正确启停推流与传输；会话结束清空各标签状态。

## 13. 测试策略（务实降标，不追 80%）

- **协议契约**：`ChatMessage` 序列化 + tag 判别；`cargo test export_all` 重导出 TS。
- **服务端**：chat `route_to_peer` 路由 + 审计落库单测；`audit_type_str` 往返。
- **被控端**：`exec.rs` 现有测试沿用。
- **E2E 手测**：模式 A / 模式 B 各跑「桌面 / 命令 / 文件下发取回 / 双向消息 / 懒推流切标签」闭环。

## 14. 文件改动清单

**协议**：`src/protocol/src/lib.rs`、`src/protocol/src/tests.rs`
**服务端**：`src/server/src/hub.rs`、`src/server/src/audit.rs`
**Slint 客户端**：`src/client/ui/app.slint`、`src/client/src/net/mod.rs`、`src/client/src/net/dispatch.rs`、`src/client/src/ui_glue.rs`（被控端聊天面板亦在 `app.slint`）
**admin-web**：`src/admin-web/src/components/control/remote-session.tsx`、`remote-tools.tsx`、`src/admin-web/src/store.ts`、`src/admin-web/src/lib/adapters/audit.ts`、`src/admin-web/src/components/audit/audit-cells.tsx`
**文档**：本文件 + 回写 `2026-06-27-...-design.md` §3.2

## 15. 风险

- Slint `.slint` DSL / enigo / xcap API 易写过时——开发前查项目 skill `rust-remote-control-stack`。
- 顶部标签 + 懒推流改动触及会话状态机（`remote_active`），需保证键鼠捕获仅在桌面标签生效，避免输入串台。
- 被控端聊天面板为新 UI 面，需覆盖「被控中收消息」的提示与折叠交互。
