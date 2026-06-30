# 远程工具集·计划②：Slint 客户端（命令 / 文件 / 即时消息 + 懒推流）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Slint 客户端补齐主控端发起「远程命令 / 远程文件（双栏，左本机右远端，下发+取回）/ 即时消息」的 UI 与上下行链路，并实现「顶部 4 标签整页同窗 + 桌面懒推流（切到桌面标签才推帧）」与被控端聊天回复面板。被控端命令执行（`exec.rs`）、文件收发/列目录（`transfer.rs`）已实现，本计划只补主控端发起侧 + 聊天两端 UI + 懒推流被控侧开关。

**Architecture:** UI（Slint，主线程）↔ net（tokio 后台）经 `FromUi`/`ToUi` 两条 mpsc 通道交互；net 内 `handle_uplink` 把 `FromUi` 映射为出站 `Envelope`，`handle_downlink` 把下行 `Message` 转 `ToUi` 投给 UI。新增能力沿用此双向链路：主控发起 → `FromUi` → `handle_uplink` → 出站协议消息；被控/对端回执 → 下行 → `handle_downlink` → `ToUi` → `ui_glue::consume_to_ui` 渲染。文件取回（pull 回流）需主控态在 `handle_downlink` 落盘，故新增全局 `PULL_TARGETS` map 记 `transfer_id → 主控本地保存目录`。懒推流复用计划①已落地的 `protocol::Message::SetCapture`：主控切标签 → `FromUi::SetCapture` → 出站；被控收 `SetCapture` → `CAPTURE_CTRL.send(Start/Stop)` 启停推帧（不动被控态）。

**Tech Stack:** Rust、Slint 1.17（软渲染，DSL 见 `.agent/skills/rust-remote-control-stack/references/slint.md`）、tokio、serde、base64、protocol crate。

**前置 / 依赖：**
- 在 `feature/remote-command-file-chat` 分支（spec：`docs/superpowers/specs/2026-06-30-remote-command-file-chat-design.md`）。
- **计划①必须先完成**（本计划引用其锁定的协议契约，不重定义）：
  - `protocol::Message::ChatMessage { session_id: String, msg_id: String, text: String }`
  - `protocol::Message::SetCapture { session_id: String, active: bool }`（懒推流开关，主控→被控，按 session 路由；server 已转发）
  - `protocol::AuditType::Chat`
  - 复用既有：`ExecRequest{session_id,exec_id,command,timeout_ms}`、`ExecResult{session_id,exec_id,exit_code:Option<i32>,stdout,stderr,truncated,duration_ms}`、`FileOpen{session_id,transfer_id,name,size,dir,dest:Option<String>}`、`FileChunk{session_id,transfer_id,seq,data,last}`、`FilePullRequest{session_id,transfer_id,path}`、`FileError{session_id,transfer_id,reason}`、`FileDone{session_id,transfer_id,path}`、`FileListRequest{session_id,transfer_id,path}`、`FileListResp{session_id,transfer_id,path,entries:Vec<FileEntry>,error:Option<String>}`、`FileDir::{Push,Pull}`、`FileEntry{name:String,is_dir:bool,size:u64}`

**风险 / 红线（动手前必读）：**
- **改 `.slint` 前先读 `.agent/skills/rust-remote-control-stack/references/slint.md`**：Slint DSL（1.17）在 Claude 语料里版本过时，组件用 `component Foo inherits Bar`、实例命名用 `name := Elem`、属性连字符在 Rust 侧转下划线（`set_active_tab`）、跨线程更新 UI 必须 `invoke_from_event_loop` + `Weak`。
- **键鼠捕获只能在「远程桌面」标签生效**：现有 `FocusScope`/`TouchArea` 捕获键鼠，标签重构后必须用 `if root.active_tab == 0` 门控，避免在命令/文件/消息标签输入串台。
- **懒推流不改被控态**：被控端收 `SetCapture` 只调 `CAPTURE_CTRL.send(Start/Stop)`，绝不动 `session.controlled`（否则注入/会话判活全乱）。
- **id 生成禁用不可复现 API**：`msg_id` 用进程内 `static AtomicU64` 计数器（`transfer_id`/`exec_id` 同理），不得用随机/`Date.now()`。
- **质量门（提交前）**：`cargo fmt` + `cargo clippy -p client -- -D warnings` 必过；不对外部输入 `unwrap()`；坏包/坏路径忽略不 panic。
- **client 测试 enigo 需串行**：跑 client 测试一律 `cargo test -p client <name> -- --test-threads=1`（注入器单测全局串行，避免 X11 句柄竞争）。

---

### Task 1: `net/mod.rs` 新增 FromUi / ToUi 变体（供后续任务引用，先建契约）

本任务只加枚举变体（不接线），让后续 TDD 任务能编译引用。加完 `cargo build -p client` 会因 `handle_uplink`/`handle_downlink` 的 `match` 非穷尽而**报错**——这是预期的，由 Task 2/4 补全分支后消除。

**Files:**
- Modify: `src/client/src/net/mod.rs:31-59`（`ToUi` 枚举末尾，`Disconnected` 之后）
- Modify: `src/client/src/net/mod.rs:62-108`（`FromUi` 枚举末尾，`CancelRemote` 之后）

- [ ] **Step 1: 给 `ToUi` 加 5 个变体**

在 `src/client/src/net/mod.rs` 的 `ToUi` 枚举里，`Disconnected,`（约 58 行）**之后、枚举闭合 `}` 之前**插入：

```rust
    /// 主控端收被控回执的命令执行结果（远程命令标签渲染）。
    ExecResult {
        exec_id: String,
        command: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        truncated: bool,
        duration_ms: u32,
    },
    /// 主控端收远端目录列表（远程文件标签右栏渲染）。path 为实际列出的绝对目录。
    RemoteEntries {
        path: String,
        entries: Vec<protocol::FileEntry>,
        /// 列目录失败原因（被控回 FileListResp.error）；None 表示成功。
        error: Option<String>,
    },
    /// 文件传输进度（下发/取回通用）：done/total 字节。total=0 表示未知。
    FileProgress {
        transfer_id: String,
        name: String,
        done: u64,
        total: u64,
    },
    /// 文件传输一次性通知（完成/失败提示，远程文件标签底部状态行）。
    FileNotice { text: String },
    /// 收到会话内即时消息（即时消息标签 / 被控聊天面板渲染，对端发来）。
    ChatIncoming {
        session_id: String,
        msg_id: String,
        text: String,
    },
```

- [ ] **Step 2: 给 `FromUi` 加 6 个变体**

在同文件 `FromUi` 枚举里，`CancelRemote { target: String },`（约 107 行）**之后、枚举闭合 `}` 之前**插入：

```rust
    /// 主控端发起一次性远程命令 → 发 ExecRequest 给被控端。
    ExecCommand { session_id: String, command: String },
    /// 主控端浏览远端目录 → 发 FileListRequest 给被控端。
    ListRemote { session_id: String, path: String },
    /// 主控端下发本机文件到远端当前目录（push）。
    PushFile {
        session_id: String,
        local_path: String,
        dest_dir: String,
    },
    /// 主控端从远端取回文件到本机目录（pull）：记 transfer_id→local_dir 后发 FilePullRequest。
    PullFile {
        session_id: String,
        remote_path: String,
        local_dir: String,
    },
    /// 会话内发送即时消息（主控/被控通用）→ 发 ChatMessage 给对端。
    SendChat { session_id: String, text: String },
    /// 切换桌面帧推流（懒推流）：主控切到/离开「远程桌面」标签 → 发 SetCapture 给被控端。
    SetCapture { session_id: String, active: bool },
```

- [ ] **Step 3: 确认编译报「match 非穷尽」（预期失败）**

Run: `cargo build -p client 2>&1 | head -30`
Expected: 编译错误 `non-exhaustive patterns`，指向 `dispatch.rs` 的 `handle_uplink`（缺 `ExecCommand/ListRemote/PushFile/PullFile/SendChat/SetCapture` 分支）。这证明变体已加入、待 Task 2 补分支。**本步不提交**（不留断编译的中间提交），与 Task 2 合并提交。

---

### Task 2: `handle_uplink` 上行映射（TDD）

为 6 个新 `FromUi` 变体补出站映射。其中 `ExecCommand/ListRemote/SendChat/SetCapture` 是纯 `Envelope` 映射（可测）；`PushFile` 委托 `transfer::send_file_push`（Task 3 实现，本任务先映射调用）；`PullFile` 需先记 `PULL_TARGETS`（Task 3 实现）再发 `FilePullRequest`。

> 顺序说明：`PushFile`/`PullFile` 依赖 Task 3 的 `transfer` 函数。本任务先实现 4 个纯映射 + 其测试并通过；`PushFile`/`PullFile` 分支在 Task 3 完成后回填（见 Task 3 Step 5）。本任务里先给这两个分支写**占位转发**以消除非穷尽错误（调用 Task 3 将补的函数签名——故本任务与 Task 3 顺序串行，不可并行）。

为避免「占位→回填」往返，本任务直接按最终形态写全 6 分支，并先在 Task 3 之前实现 `transfer` 侧函数签名的 `stub`。**实际执行时按 Task 3 → Task 2 的依赖顺序：先做 Task 3 的 transfer 函数，再回到本任务补 dispatch 分支。** 下面给出本任务最终代码与测试。

**Files:**
- Modify: `src/client/src/net/dispatch.rs:9`（`use` 引入 `PULL_TARGETS` 不需要——`transfer` 内部用；这里需引入计数器工具）
- Modify: `src/client/src/net/dispatch.rs:560`（`handle_uplink` 的 `match act` 末尾，`ScreenshotResp` 分支之后）
- Test: `src/client/src/net/dispatch.rs`（`uplink_tests` 模块）

- [ ] **Step 1: 写失败测试（4 个纯映射 + PullFile 记录目标 + PushFile 出站首包）**

在 `src/client/src/net/dispatch.rs` 的 `mod uplink_tests` 内（`cancel_remote_uplink_sends_cancel_request_with_target` 之后）追加：

```rust
    /// 远程命令上行：ExecCommand → ExecRequest（带 session_id/command，timeout_ms 用封顶值）。
    #[tokio::test]
    async fn exec_command_uplink_sends_exec_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::ExecCommand {
                session_id: "s-1".into(),
                command: "whoami".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 ExecRequest");
        assert!(s.contains("\"type\":\"exec_request\""), "缺 exec_request: {s}");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::ExecRequest { session_id, command, .. } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(command, "whoami");
            }
            other => panic!("应为 ExecRequest，实际 {other:?}"),
        }
    }

    /// 远端目录浏览上行：ListRemote → FileListRequest（path 透传）。
    #[tokio::test]
    async fn list_remote_uplink_sends_file_list_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::ListRemote {
                session_id: "s-1".into(),
                path: "/home".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 FileListRequest");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileListRequest { session_id, path, .. } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(path, "/home");
            }
            other => panic!("应为 FileListRequest，实际 {other:?}"),
        }
    }

    /// 即时消息上行：SendChat → ChatMessage（带 msg_id，text 透传）。
    #[tokio::test]
    async fn send_chat_uplink_sends_chat_message() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::SendChat {
                session_id: "s-1".into(),
                text: "你好".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 ChatMessage");
        assert!(s.contains("\"type\":\"chat_message\""), "缺 chat_message: {s}");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::ChatMessage { session_id, msg_id, text } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(text, "你好");
                assert!(!msg_id.is_empty(), "msg_id 必须非空（AtomicU64 计数器）");
            }
            other => panic!("应为 ChatMessage，实际 {other:?}"),
        }
    }

    /// 懒推流上行：SetCapture → SetCapture（active 透传）。
    #[tokio::test]
    async fn set_capture_uplink_sends_set_capture() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::SetCapture {
                session_id: "s-1".into(),
                active: true,
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 SetCapture");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(env.payload, Message::SetCapture { active: true, .. }));
    }

    /// 取回上行：PullFile 先把 transfer_id→local_dir 记入 PULL_TARGETS，再发 FilePullRequest。
    #[tokio::test]
    async fn pull_file_uplink_records_target_and_sends_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let tmp = std::env::temp_dir().join("ohmydesk-pull-dst");
        std::fs::create_dir_all(&tmp).unwrap();
        handle_uplink(
            FromUi::PullFile {
                session_id: "s-1".into(),
                remote_path: "/etc/hostname".into(),
                local_dir: tmp.to_string_lossy().to_string(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 FilePullRequest");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FilePullRequest { session_id, transfer_id, path } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(path, "/etc/hostname");
                // 取回目标已登记：取出应等于本地目录
                let got = crate::transfer::take_pull_target(&transfer_id);
                assert_eq!(got.as_deref(), Some(tmp.as_path()), "transfer_id 应登记 local_dir");
            }
            other => panic!("应为 FilePullRequest，实际 {other:?}"),
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client -- --test-threads=1 exec_command_uplink list_remote_uplink send_chat_uplink set_capture_uplink pull_file_uplink 2>&1 | tail -20`
Expected: 编译失败——`FromUi` 变体在 `handle_uplink` 的 `match` 未覆盖（非穷尽），且 `crate::transfer::take_pull_target` 未定义。

> 注：`crate::transfer::take_pull_target`/`send_file_push` 在 Task 3 实现，故本测试在 Task 3 完成前不可能编译通过。**实际执行先做 Task 3，再做本任务 Step 3。**

- [ ] **Step 3: 在 `handle_uplink` 补 6 个分支**

先在 `src/client/src/net/dispatch.rs` 顶部新增 id 计数器（文件级，紧跟 `use` 块之后，约第 9 行下方）：

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内自增 id 计数器（exec_id/transfer_id/msg_id 用，禁随机/时间以保证可复现）。
static SEQ: AtomicU64 = AtomicU64::new(1);

/// 生成下一个带前缀的进程内唯一 id（如 "exec-12"）。
fn next_id(prefix: &str) -> String {
    format!("{prefix}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}
```

在 `handle_uplink` 的 `match act` 里，`FromUi::ScreenshotResp { .. } => Envelope { ... }`（约 559 行，闭合 `},` 之后）之后追加 6 个分支：

```rust
        FromUi::ExecCommand { session_id, command } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session 路由给被控端
            ts: now(),
            payload: Message::ExecRequest {
                session_id,
                exec_id: next_id("exec"),
                command,
                timeout_ms: crate::exec::MAX_TIMEOUT_MS,
            },
        },
        FromUi::ListRemote { session_id, path } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::FileListRequest {
                session_id,
                transfer_id: next_id("ls"),
                path,
            },
        },
        FromUi::SendChat { session_id, text } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::ChatMessage {
                session_id,
                msg_id: next_id("msg"),
                text,
            },
        },
        FromUi::SetCapture { session_id, active } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::SetCapture { session_id, active },
        },
        // 下发（push）：直接委托 transfer::send_file_push 在独立任务里读本机文件分块出站，
        // 本分支不构造 Envelope（已在子任务里发首包+块），故提前 return。
        FromUi::PushFile { session_id, local_path, dest_dir } => {
            tokio::spawn(crate::transfer::send_file_push(
                out_tx.clone(),
                self_id.to_string(),
                session_id,
                next_id("tx"),
                local_path,
                dest_dir,
            ));
            return;
        }
        // 取回（pull）：先登记 transfer_id→本地保存目录（被控回流首包据此落盘），再发请求。
        FromUi::PullFile { session_id, remote_path, local_dir } => {
            let transfer_id = next_id("tx");
            crate::transfer::set_pull_target(&transfer_id, std::path::PathBuf::from(local_dir));
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::FilePullRequest {
                    session_id,
                    transfer_id,
                    path: remote_path,
                },
            }
        }
```

> 说明：`PushFile` 分支用 `return`（不落入函数末尾的统一 `serde_json::to_string + send`），因为 `send_file_push` 自己发多条消息。其余 5 个分支构造 `Envelope` 走统一出站。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client -- --test-threads=1 exec_command_uplink list_remote_uplink send_chat_uplink set_capture_uplink pull_file_uplink 2>&1 | tail -15`
Expected: 5 个测试 PASS。

- [ ] **Step 5: 提交（与 Task 1、Task 3 合并：见 Task 3 Step 6）**

> 本任务不单独提交：Task 1（枚举）+ Task 3（transfer 函数）+ Task 2（dispatch 分支）构成一个可编译、测试通过的最小闭环，统一在 Task 3 Step 6 提交。

---

### Task 3: `transfer.rs` 新增 `send_file_push` + `PULL_TARGETS`（TDD）

主控端「下发」镜像被控端 `send_file`（但 `dir=Push`、`dest=Some(dest_dir)`、文件读自本机）；「取回」需全局 map 记 `transfer_id → 主控本地目录`，供 Task 4 的下行落盘取用。

**Files:**
- Modify: `src/client/src/transfer.rs:32-33`（`RECEIVERS` 静态之后加 `PULL_TARGETS`）
- Modify: `src/client/src/transfer.rs:307`（`send_file` 之后加 `send_file_push`）
- Test: `src/client/src/transfer.rs`（`mod tests`）

- [ ] **Step 1: 写失败测试**

在 `src/client/src/transfer.rs` 的 `mod tests` 内（`接收_超声明大小被拒并清理` 之后）追加：

```rust
    #[test]
    fn pull_target_存取与取出清除() {
        let tid = "t-pull-1";
        let dir = std::env::temp_dir().join("ohmydesk-pull-target");
        set_pull_target(tid, dir.clone());
        // 取出一次即移除（落盘完成后不应残留）
        assert_eq!(take_pull_target(tid), Some(dir));
        assert_eq!(take_pull_target(tid), None, "二次取出应为 None");
    }

    #[tokio::test]
    async fn 下发_推送首包为_file_open_push_带_dest() {
        // 造一个本机小文件
        let base = std::env::temp_dir().join("ohmydesk-push-src");
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("up.txt");
        std::fs::write(&f, b"push-payload").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file_push(
            tx,
            "ep-self".into(),
            "s-1".into(),
            "tx-1".into(),
            f.to_string_lossy().to_string(),
            "/remote/dest/dir".into(),
        )
        .await;

        // 首包：FileOpen{dir:push, dest:Some, name=up.txt, size=12}
        let s = rx.recv().await.expect("应有 FileOpen 首包");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileOpen { dir, dest, name, size, .. } => {
                assert_eq!(dir, FileDir::Push);
                assert_eq!(dest.as_deref(), Some("/remote/dest/dir"));
                assert_eq!(name, "up.txt");
                assert_eq!(size, 12);
            }
            other => panic!("首包应为 FileOpen，实际 {other:?}"),
        }
        // 次包：FileChunk last=true（小文件单块）
        let s = rx.recv().await.expect("应有 FileChunk");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileChunk { last, data, .. } => {
                assert!(last, "12 字节单块即末块");
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .unwrap();
                assert_eq!(raw, b"push-payload");
            }
            other => panic!("次包应为 FileChunk，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn 下发_文件不存在回_file_error() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file_push(
            tx,
            "ep-self".into(),
            "s-1".into(),
            "tx-err".into(),
            "/no/such/ohmydesk/file.bin".into(),
            "/remote/dir".into(),
        )
        .await;
        let s = rx.recv().await.expect("应有 FileError");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(env.payload, Message::FileError { .. }));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client -- --test-threads=1 pull_target 下发_ 2>&1 | tail -15`
Expected: 编译失败——`set_pull_target`/`take_pull_target`/`send_file_push` 未定义。

- [ ] **Step 3: 加 `PULL_TARGETS` 静态 + 存取函数**

在 `src/client/src/transfer.rs` 的 `RECEIVERS` 静态（约 32-33 行）之后插入：

```rust
/// 主控端取回保存目录：transfer_id → 本地保存目录。
/// PullFile 上行时登记，被控端 FileOpen{dir:pull} 回流首包到达时取出（一次取用即移除）。
static PULL_TARGETS: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 登记一次取回的本地保存目录（主控端发 FilePullRequest 前调用）。
pub fn set_pull_target(transfer_id: &str, dir: PathBuf) {
    PULL_TARGETS
        .lock()
        .unwrap()
        .insert(transfer_id.to_string(), dir);
}

/// 取出并移除取回目标目录（回流首包到达时调用）。未登记返回 None。
pub fn take_pull_target(transfer_id: &str) -> Option<PathBuf> {
    PULL_TARGETS.lock().unwrap().remove(transfer_id)
}
```

- [ ] **Step 4: 加 `send_file_push`（镜像 `send_file`，dir=Push + dest）**

在同文件 `send_file` 函数（约 307 行，闭合 `}` 之后）之后插入：

```rust
/// push 下发：读取主控本机 `local_path` 文件，以 `FileOpen{dir:push, dest}` + `FileChunk` 流给被控端。
/// 镜像 [`send_file`]，区别：方向为 Push、带目标目录 dest、文件读自主控本机。失败回 `FileError`。
/// 在独立任务中调用（读 ≤50MB 文件进内存再分块）。
pub async fn send_file_push(
    out_tx: UnboundedSender<String>,
    self_id: String,
    session_id: String,
    transfer_id: String,
    local_path: String,
    dest_dir: String,
) {
    let err = |reason: String| {
        send(
            &out_tx,
            &self_id,
            Message::FileError {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                reason,
            },
        );
    };

    let p = Path::new(&local_path);
    match tokio::fs::metadata(p).await {
        Ok(m) if !m.is_file() => return err("目标不是常规文件".into()),
        Ok(m) if m.len() > MAX_FILE => {
            return err(format!("文件超过上限 {}MB", MAX_FILE / 1024 / 1024))
        }
        Err(e) => return err(format!("无法读取文件: {e}")),
        _ => {}
    }
    let bytes = match tokio::fs::read(p).await {
        Ok(b) => b,
        Err(e) => return err(format!("读取失败: {e}")),
    };
    let name = safe_name(p.file_name().and_then(|s| s.to_str()).unwrap_or("file.bin"));

    send(
        &out_tx,
        &self_id,
        Message::FileOpen {
            session_id: session_id.clone(),
            transfer_id: transfer_id.clone(),
            name,
            size: bytes.len() as u64,
            dir: FileDir::Push,
            dest: Some(dest_dir),
        },
    );

    if bytes.is_empty() {
        send(
            &out_tx,
            &self_id,
            Message::FileChunk {
                session_id,
                transfer_id,
                seq: 0,
                data: String::new(),
                last: true,
            },
        );
        return;
    }
    let total = bytes.len();
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        let last = (i + 1) * CHUNK >= total;
        send(
            &out_tx,
            &self_id,
            Message::FileChunk {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                seq: i as u64,
                data: STANDARD.encode(chunk),
                last,
            },
        );
    }
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p client -- --test-threads=1 pull_target 下发_ 2>&1 | tail -15`
Expected: `pull_target_存取与取出清除`、`下发_推送首包为_file_open_push_带_dest`、`下发_文件不存在回_file_error` 全 PASS。

> 完成后回到 **Task 2 Step 3/4**：补 `handle_uplink` 6 分支（`PushFile`/`PullFile` 现可引用 `send_file_push`/`set_pull_target`），跑 Task 2 测试至 PASS。

- [ ] **Step 6: 统一提交 Task 1 + 2 + 3**

```bash
cargo fmt
cargo clippy -p client -- -D warnings
cargo test -p client -- --test-threads=1 2>&1 | tail -5
git add src/client/src/net/mod.rs src/client/src/net/dispatch.rs src/client/src/transfer.rs
git commit -m "feat(client): 主控端命令/文件/聊天/懒推流上行链路 + transfer push/pull 目标登记"
```

Expected: clippy 零警告；client 全部测试 PASS。

---

### Task 4: `handle_downlink` 主控侧处理（ExecResult / FileListResp / pull 回流落盘 / FileDone / FileError / Chat）（TDD 能测部分）

被控回执到达主控端时，现有 `handle_downlink` 对 `ExecResult`/`FileListResp`/`FileDone`/`ChatMessage` 落在 `_ => {}`（未处理，spec 标 P1），且 pull 回流（`FileOpen{dir:Pull}`+`FileChunk`）在主控态被忽略。本任务补全。

可测部分：pull 回流落盘（`take_pull_target` + `open_recv` + `write_chunk` 链路，纯逻辑）。`ExecResult`/`FileListResp`/`ChatMessage` → `ToUi` 是单向投递，用 `to_ui` 通道断言。

**Files:**
- Modify: `src/client/src/net/dispatch.rs:248-295`（`FileOpen`/`FileChunk` 分支：补主控态 pull 回流）
- Modify: `src/client/src/net/dispatch.rs:213-246` 区域附近（`ExecResult`/`FileListResp`/`FileDone`/`ChatMessage` 新分支，插在 `ExecRequest` 分支之前的被控分支组旁）
- Test: `src/client/src/net/dispatch.rs`（`mod tests`）

- [ ] **Step 1: 写失败测试（下行 → ToUi 投递 + pull 回流落盘）**

在 `src/client/src/net/dispatch.rs` 的 `mod tests` 内（`auth_accept_uplink_enters_controlled_and_starts_capture` 之后）追加。先在该模块顶部确保引入 `ToUi`（`use super::*;` 已带入 `ToUi`，无需额外）：

```rust
    /// 辅助：把一条 payload 包成 Envelope 文本，喂给 handle_downlink。
    fn env_text(from: &str, payload: Message) -> String {
        serde_json::to_string(&Envelope {
            from: from.into(),
            to: None,
            ts: 0,
            payload,
        })
        .unwrap()
    }

    /// 主控端收 ExecResult → 投 ToUi::ExecResult（exec_id/exit_code/stdout 透传）。
    #[tokio::test]
    async fn downlink_exec_result_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let t = env_text(
            "ep-peer",
            Message::ExecResult {
                session_id: "s-1".into(),
                exec_id: "exec-7".into(),
                exit_code: Some(0),
                stdout: "root".into(),
                stderr: String::new(),
                truncated: false,
                duration_ms: 12,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::ExecResult") {
            ToUi::ExecResult { exec_id, exit_code, stdout, .. } => {
                assert_eq!(exec_id, "exec-7");
                assert_eq!(exit_code, Some(0));
                assert_eq!(stdout, "root");
            }
            other => panic!("应为 ExecResult，实际 {other:?}"),
        }
    }

    /// 主控端收 FileListResp → 投 ToUi::RemoteEntries（path + entries + error 透传）。
    #[tokio::test]
    async fn downlink_file_list_resp_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let t = env_text(
            "ep-peer",
            Message::FileListResp {
                session_id: "s-1".into(),
                transfer_id: "ls-1".into(),
                path: "/home/me".into(),
                entries: vec![protocol::FileEntry {
                    name: "docs".into(),
                    is_dir: true,
                    size: 0,
                }],
                error: None,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::RemoteEntries") {
            ToUi::RemoteEntries { path, entries, error } => {
                assert_eq!(path, "/home/me");
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "docs");
                assert!(error.is_none());
            }
            other => panic!("应为 RemoteEntries，实际 {other:?}"),
        }
    }

    /// 主控端收对端 ChatMessage → 投 ToUi::ChatIncoming（text 透传）。
    #[tokio::test]
    async fn downlink_chat_message_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let t = env_text(
            "ep-peer",
            Message::ChatMessage {
                session_id: "s-1".into(),
                msg_id: "msg-3".into(),
                text: "看下报错".into(),
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::ChatIncoming") {
            ToUi::ChatIncoming { msg_id, text, .. } => {
                assert_eq!(msg_id, "msg-3");
                assert_eq!(text, "看下报错");
            }
            other => panic!("应为 ChatIncoming，实际 {other:?}"),
        }
    }

    /// 主控态收 pull 回流（FileOpen{dir:Pull} + FileChunk last）→ 据 PULL_TARGETS 落盘到本机目录。
    #[tokio::test]
    async fn downlink_pull_flow_writes_local_file() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        // 主控态：controlling = s-pull
        session.lock().await.controlling = Some("s-pull".into());

        // 模拟：上行 PullFile 已登记 transfer_id→本地目录
        let dst = std::env::temp_dir().join("ohmydesk-pull-recv");
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(&dst).unwrap();
        crate::transfer::set_pull_target("tx-pull", dst.clone());

        // 回流首包：FileOpen{dir:Pull}
        let open = env_text(
            "ep-peer",
            Message::FileOpen {
                session_id: "s-pull".into(),
                transfer_id: "tx-pull".into(),
                name: "report.log".into(),
                size: 5,
                dir: protocol::FileDir::Pull,
                dest: None,
            },
        );
        handle_downlink(&open, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        // 回流末块：FileChunk last=true
        let chunk = env_text(
            "ep-peer",
            Message::FileChunk {
                session_id: "s-pull".into(),
                transfer_id: "tx-pull".into(),
                seq: 0,
                data: STANDARD.encode(b"hello"),
                last: true,
            },
        );
        handle_downlink(&chunk, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();

        // 落盘到本机目录
        let saved = dst.join("report.log");
        assert!(saved.exists(), "取回文件应落到本机目录");
        assert_eq!(std::fs::read(&saved).unwrap(), b"hello");
        // 应投 FileDone/FileProgress 类通知给 UI
        let mut saw_done = false;
        while let Ok(ev) = to_ui_rx.try_recv() {
            if matches!(ev, ToUi::FileNotice { .. } | ToUi::FileProgress { .. }) {
                saw_done = true;
            }
        }
        assert!(saw_done, "应投 FileNotice/FileProgress 通知 UI");
        let _ = std::fs::remove_dir_all(&dst);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client -- --test-threads=1 downlink_exec_result downlink_file_list_resp downlink_chat_message downlink_pull_flow 2>&1 | tail -15`
Expected: 断言失败（消息落入 `_ => {}`，`try_recv` 拿不到 ToUi / 文件未落盘）。

- [ ] **Step 3a: 补主控侧下行新分支（ExecResult / FileListResp / FileDone / ChatMessage）**

在 `src/client/src/net/dispatch.rs` 的 `match env.payload` 里，被控分支组之前——`Message::ExecRequest { .. }` 分支（约 214 行）**之前**插入：

```rust
        // ── 主控端：收被控回执的命令执行结果 → 投 UI 渲染 ─────────────────────
        Message::ExecResult {
            exec_id,
            exit_code,
            stdout,
            stderr,
            truncated,
            duration_ms,
            ..
        } => {
            let _ = to_ui.send(ToUi::ExecResult {
                exec_id,
                // command 由 UI 侧按 exec_id 关联展示；下行不带 command 原文，留空由 UI 回填。
                command: String::new(),
                exit_code,
                stdout,
                stderr,
                truncated,
                duration_ms,
            });
        }
        // ── 主控端：收被控回执的远端目录列表 → 投 UI（右栏渲染）──────────────────
        Message::FileListResp {
            path, entries, error, ..
        } => {
            let _ = to_ui.send(ToUi::RemoteEntries { path, entries, error });
        }
        // ── 主控端：收对端即时消息 → 投 UI（即时消息标签 / 被控聊天面板）──────────
        Message::ChatMessage {
            session_id, msg_id, text,
        } => {
            let _ = to_ui.send(ToUi::ChatIncoming { session_id, msg_id, text });
        }
```

> 注意：`ExecResult.command` 下行不含原文。主控端在发 `ExecCommand` 时本地按 `exec_id` 暂存 command，回执到达后 UI 侧关联——但 `ExecResult` 下行不带 exec→command 映射的话无法在 net 层回填。简化：UI 端维护「执行中卡片」按发送顺序匹配回执（命令一次一条、串行展示）；net 层 `command` 留空，UI 渲染时用本地输入历史的最后一条未完成命令填充（见 Task 7 命令标签设计）。

- [ ] **Step 3b: 改 `FileOpen` 分支——补主控态 pull 回流（打开本地接收）**

把现有 `Message::FileOpen { .. }` 分支（约 249-267 行）整体替换为：

```rust
        // ── FileOpen：被控态收 push 首包(落盘准备) / 主控态收 pull 回流首包(本地落盘准备)──
        Message::FileOpen {
            session_id,
            transfer_id,
            name,
            size,
            dir,
            dest,
        } => {
            let ctx = session.lock().await;
            let controlled = ctx.controlled.as_deref() == Some(session_id.as_str());
            let controlling = ctx.controlling.as_deref() == Some(session_id.as_str());
            drop(ctx);
            if controlled && dir == protocol::FileDir::Push {
                // 被控端：push 下发首包 → 打开接收文件（失败回 FileError）
                if let Err(reason) =
                    crate::transfer::open_recv(&transfer_id, &name, size, dest.as_deref())
                {
                    send_file_error(out_tx, self_id, session_id, transfer_id, reason);
                }
            } else if controlling && dir == protocol::FileDir::Pull {
                // 主控端：pull 回流首包 → 取出本地保存目录，打开本地接收文件
                let local_dir = crate::transfer::take_pull_target(&transfer_id)
                    .map(|p| p.to_string_lossy().to_string());
                match crate::transfer::open_recv(&transfer_id, &name, size, local_dir.as_deref()) {
                    Ok(_) => {
                        let _ = to_ui.send(ToUi::FileProgress {
                            transfer_id: transfer_id.clone(),
                            name: name.clone(),
                            done: 0,
                            total: size,
                        });
                    }
                    Err(reason) => {
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("取回 {name} 失败：{reason}"),
                        });
                    }
                }
            }
        }
```

- [ ] **Step 3c: 改 `FileChunk` 分支——补主控态 pull 回流落盘**

把现有 `Message::FileChunk { .. }` 分支（约 270-295 行）整体替换为：

```rust
        // ── FileChunk：被控态收 push 块(落盘) / 主控态收 pull 回流块(本地落盘)──────
        Message::FileChunk {
            session_id,
            transfer_id,
            data,
            last,
            ..
        } => {
            let ctx = session.lock().await;
            let controlled = ctx.controlled.as_deref() == Some(session_id.as_str());
            let controlling = ctx.controlling.as_deref() == Some(session_id.as_str());
            drop(ctx);
            if controlled {
                // 被控端：push 块落盘，末块回 FileDone(带最终路径)，失败回 FileError
                match crate::transfer::write_chunk(&transfer_id, &data, last) {
                    Ok(Some(path)) => send_file_done(
                        out_tx,
                        self_id,
                        session_id,
                        transfer_id,
                        path.to_string_lossy().to_string(),
                    ),
                    Ok(None) => {}
                    Err(reason) => {
                        send_file_error(out_tx, self_id, session_id, transfer_id, reason)
                    }
                }
            } else if controlling {
                // 主控端：pull 回流块落盘到本机；末块完成 → 投 FileNotice 告知本机最终路径
                match crate::transfer::write_chunk(&transfer_id, &data, last) {
                    Ok(Some(path)) => {
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("已取回到本机：{}", path.to_string_lossy()),
                        });
                    }
                    Ok(None) => {}
                    Err(reason) => {
                        crate::transfer::abort(&transfer_id);
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("取回失败：{reason}"),
                        });
                    }
                }
            }
        }
```

- [ ] **Step 3d: 补 `FileDone` 主控态（下发回执 → UI 通知）**

现有 `Message::FileError { transfer_id, .. }` 分支只清理在途接收。`FileDone` 当前落 `_ => {}`。在 `Message::FileError { .. }` 分支（约 371 行）之前插入：

```rust
        // ── 主控端：收被控回执的下发完成（push）→ 投 UI 告知远端最终路径 ──────────
        Message::FileDone { path, .. } => {
            let _ = to_ui.send(ToUi::FileNotice {
                text: format!("已下发到远端：{path}"),
            });
        }
```

并把 `Message::FileError { transfer_id, .. }` 分支改为同时通知 UI：

```rust
        // ── 传输失败：清理在途接收 + 通知 UI ───────────────────────────────────
        Message::FileError { transfer_id, reason, .. } => {
            crate::transfer::abort(&transfer_id);
            let _ = to_ui.send(ToUi::FileNotice {
                text: format!("传输失败：{reason}"),
            });
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client -- --test-threads=1 downlink_exec_result downlink_file_list_resp downlink_chat_message downlink_pull_flow 2>&1 | tail -15`
Expected: 4 个测试 PASS。

- [ ] **Step 5: 提交**

```bash
cargo fmt
cargo clippy -p client -- -D warnings
git add src/client/src/net/dispatch.rs
git commit -m "feat(client): 主控端下行处理 ExecResult/FileListResp/Chat + pull 回流落盘 + 传输通知"
```

---

### Task 5: 被控端处理 `SetCapture`（懒推流，TDD）

被控端收主控 `SetCapture{active}` → 据 active 调 `CAPTURE_CTRL.send(Start/Stop)`，**仅启停推帧，不动 `controlled` 态**。

**Files:**
- Modify: `src/client/src/net/dispatch.rs:185-195` 区域（`SetQuality` 分支之后插入 `SetCapture` 分支）
- Test: `src/client/src/net/dispatch.rs`（`mod tests`）

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内追加：

```rust
    /// 被控端收 SetCapture{active:false} → CAPTURE_CTRL.Stop；active:true → Start。controlled 态不变。
    #[tokio::test]
    async fn downlink_set_capture_toggles_capture_only() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, _to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        session.lock().await.controlled = Some("s-cap".into());

        let (cap_tx, mut cap_rx) = mpsc::unbounded_channel::<CaptureCtrl>();
        CAPTURE_CTRL.init(cap_tx);

        // active:false → Stop
        let t = env_text(
            "ep-ctrl",
            Message::SetCapture {
                session_id: "s-cap".into(),
                active: false,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        assert!(matches!(cap_rx.try_recv(), Ok(CaptureCtrl::Stop)));
        // controlled 态绝不被 SetCapture 改动
        assert_eq!(session.lock().await.controlled.as_deref(), Some("s-cap"));

        // active:true → Start
        let t = env_text(
            "ep-ctrl",
            Message::SetCapture {
                session_id: "s-cap".into(),
                active: true,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match cap_rx.try_recv() {
            Ok(CaptureCtrl::Start { session_id }) => assert_eq!(session_id, "s-cap"),
            other => panic!("应为 Start，实际 {other:?}"),
        }
    }
```

> 注：`CAPTURE_CTRL` 是全局 `OnceLock`，与 `auth_accept_uplink_enters_controlled_and_starts_capture` 共用。`OnceLock::set` 二次调用返回 Err（忽略），故 `--test-threads=1` 串行下首个初始化生效、后续 `try_recv` 仍从同一通道收——本测试独立断言 Stop/Start，不依赖通道空。若与既有测试串扰，二者都在串行下各自先发各自的信号、紧接 `try_recv`，不交叉。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client -- --test-threads=1 downlink_set_capture 2>&1 | tail -15`
Expected: 编译失败——`Message::SetCapture` 在 `match` 未覆盖（落 `_ => {}` 则 `cap_rx.try_recv()` 为 Err，断言失败）。

- [ ] **Step 3: 加 `SetCapture` 下行分支**

在 `src/client/src/net/dispatch.rs` 的 `Message::SetQuality { session_id, mode } => { ... }` 分支（约 186-195 行，闭合 `}` 之后）之后插入：

```rust
        // 被控端收主控懒推流开关 → 据 active 启停推帧（仅启停采集，不动 controlled 态）
        Message::SetCapture { session_id, active } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
                if active {
                    CAPTURE_CTRL.send(CaptureCtrl::Start {
                        session_id: session_id.clone(),
                    });
                } else {
                    CAPTURE_CTRL.send(CaptureCtrl::Stop);
                }
                tracing::info!("被控应用懒推流开关 active={active} session={session_id}");
            }
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client -- --test-threads=1 downlink_set_capture 2>&1 | tail -10`
Expected: PASS。

- [ ] **Step 5: 全 client 测试 + 提交**

```bash
cargo fmt
cargo clippy -p client -- -D warnings
cargo test -p client -- --test-threads=1 2>&1 | tail -5
git add src/client/src/net/dispatch.rs
git commit -m "feat(client): 被控端处理 SetCapture 懒推流开关(仅启停采集)"
```

Expected: client 全部测试 PASS；clippy 零警告。逻辑层（net/transfer）至此完成，后续为 UI（手动验证）。

---

### Task 6: `app.slint` 顶部 4 标签重构（手动验证）

把主控画面态（`remote_active`，617-824 行）重构为：顶部 4 标签条「远程桌面 / 远程命令 / 远程文件 / 即时消息」+ 标签下整页内容区（同窗同页切换），桌面贴帧+键鼠捕获**门控在 `active_tab == 0`**。本任务先搭标签骨架 + 把现有桌面内容收进 tab 0，命令/文件/消息三页留占位（Task 7/8 填充）。

> 改前必读 `.agent/skills/rust-remote-control-stack/references/slint.md`（DSL 1.17）。

**Files:**
- Modify: `src/client/ui/app.slint:318-356`（属性/回调声明区，加 `active_tab` 等）
- Modify: `src/client/ui/app.slint:617-824`（`if root.remote_active` 块重构）

- [ ] **Step 1: 加标签/聊天/命令/文件相关属性与回调声明**

在 `src/client/ui/app.slint` 的「主控端状态」声明区，`callback set_quality(bool /*high*/);`（约 333 行）之后插入：

```slint
    // ── 主控工作台标签（0=远程桌面 1=远程命令 2=远程文件 3=即时消息）──
    in-out property <int> active_tab: 0;
    in-out property <bool> chat_unread: false;       // 即时消息标签未读红点
    callback tab_changed(int /*tab*/);               // 切标签 → Rust 侧发 SetCapture（懒推流）

    // ── 远程命令 ──
    in-out property <string> cmd_input;              // 命令输入行
    in property <string> cmd_output;                 // 输出区累积文本（Rust 侧 append）
    callback run_command(string /*command*/);        // 执行命令

    // ── 远程文件 ──
    in property <[FileEntry]> local_entries: [];     // 左栏本机目录条目
    in property <string> local_path;                 // 左栏当前本机绝对路径
    in property <[FileEntry]> remote_entries: [];    // 右栏远端目录条目
    in property <string> remote_path;                // 右栏当前远端绝对路径
    in property <string> file_notice;                // 传输状态行
    callback list_local(string /*path*/);            // 浏览本机目录
    callback list_remote(string /*path*/);           // 浏览远端目录
    callback push_file(string /*local_name*/);       // 下发左栏选中文件到右栏当前目录
    callback pull_file(string /*remote_name*/);      // 取回右栏选中文件到左栏当前目录

    // ── 即时消息 ──
    in property <string> chat_log;                   // 聊天记录累积文本（Rust 侧 append）
    in-out property <string> chat_input;             // 输入行
    callback send_chat(string /*text*/);             // 发送消息
```

并在文件顶部 `struct HistoryItem` 之后（约 38 行）新增一个与 `protocol::FileEntry` 对齐的 Slint struct（Slint 不直接认 Rust struct，需声明同形）：

```slint
// 一个目录条目（与 protocol::FileEntry 同形：Rust 侧 build 后 set_*_entries）。
struct FileEntry {
    name: string,
    is_dir: bool,
    size: int,    // 字节（u64 在 Slint 用 int 承载，仅展示用，超 2^31 极少见）
}
```

- [ ] **Step 2: 重构 `if root.remote_active` 块为标签骨架**

把 `src/client/ui/app.slint` 的整个 `if root.remote_active: Rectangle { ... }` 块（617-824 行）替换为下面结构。**核心：顶部 44px 标签条 + 内容区按 `active_tab` 切换；tab 0 保留原桌面贴帧/键鼠/工具条（键鼠捕获只在 tab 0 渲染，自然门控）。**

```slint
    // ⑤ 主控工作台：顶部 4 标签 + 整页内容（同窗同页切换）
    if root.remote_active: Rectangle {
        background: #050506;

        VerticalLayout {
            // ── 顶部标签条 ──
            Rectangle {
                height: 44px;
                background: #0b0b0ddd;
                HorizontalLayout {
                    padding-left: 8px;
                    padding-right: 10px;
                    spacing: 4px;
                    // 4 个标签
                    for tab[idx] in [
                        { t: "远程桌面", i: 0 },
                        { t: "远程命令", i: 1 },
                        { t: "远程文件", i: 2 },
                        { t: "即时消息", i: 3 },
                    ]: Rectangle {
                        width: 96px;
                        VerticalLayout {
                            alignment: center;
                            HorizontalLayout {
                                alignment: center;
                                spacing: 4px;
                                Text {
                                    text: tab.t;
                                    color: root.active_tab == tab.i ? Theme.fg : Theme.fg-muted;
                                    font-size: 13px;
                                    font-weight: root.active_tab == tab.i ? 700 : 500;
                                    vertical-alignment: center;
                                }
                                // 即时消息未读红点
                                if tab.i == 3 && root.chat_unread: Rectangle {
                                    width: 7px;
                                    height: 7px;
                                    border-radius: 4px;
                                    background: Theme.danger;
                                }
                            }
                            // 选中下划线
                            Rectangle {
                                height: 2px;
                                background: root.active_tab == tab.i ? Theme.primary : transparent;
                            }
                        }
                        TouchArea {
                            mouse-cursor: pointer;
                            clicked => {
                                root.active_tab = tab.i;
                                if (tab.i == 3) { root.chat_unread = false; }
                                root.tab_changed(tab.i);
                            }
                        }
                    }
                    Rectangle { } // 撑开
                    // 断开（常驻标签条右侧）
                    VerticalLayout {
                        alignment: center;
                        DangerButton {
                            label: "断开";
                            clicked => {
                                root.disconnect_remote();
                            }
                        }
                    }
                }
            }

            // ── 内容区 ──
            Rectangle {
                vertical-stretch: 1;

                // tab 0：远程桌面（贴帧 + 键鼠捕获，键鼠只在此 if 内渲染，天然门控）
                if root.active_tab == 0: Rectangle {
                    background: #050506;
                    init => { fs.focus(); }
                    property <float> frame_scale: root.frame_w <= 1 || root.frame_h <= 1 ? 1 : (
                        (self.width / (root.frame_w * 1px)) < (self.height / (root.frame_h * 1px))
                            ? ((self.width / (root.frame_w * 1px)) < 1 ? (self.width / (root.frame_w * 1px)) : 1)
                            : ((self.height / (root.frame_h * 1px)) < 1 ? (self.height / (root.frame_h * 1px)) : 1));
                    property <length> frame_display_w: root.frame_w <= 1 ? self.width : (root.frame_w * 1px * self.frame_scale);
                    property <length> frame_display_h: root.frame_h <= 1 ? self.height : (root.frame_h * 1px * self.frame_scale);
                    property <length> frame_display_x: (self.width - self.frame_display_w) / 2;
                    property <length> frame_display_y: (self.height - self.frame_display_h) / 2;

                    if root.frame_w <= 1: VerticalLayout {
                        alignment: center;
                        HorizontalLayout {
                            alignment: center;
                            VerticalLayout {
                                alignment: center;
                                spacing: 10px;
                                Text {
                                    text: "远程桌面已就绪";
                                    color: Theme.fg;
                                    font-size: 14px;
                                    font-weight: 500;
                                    horizontal-alignment: center;
                                }
                                Text {
                                    text: "正在接收对端画面…";
                                    color: Theme.fg-muted;
                                    font-size: 12px;
                                    horizontal-alignment: center;
                                }
                            }
                        }
                    }

                    frame_img := Image {
                        source: root.frame;
                        image-fit: fill;
                        x: parent.frame_display_x;
                        y: parent.frame_display_y;
                        width: parent.frame_display_w;
                        height: parent.frame_display_h;
                    }

                    ta := TouchArea {
                        x: parent.frame_display_x;
                        y: parent.frame_display_y;
                        width: parent.frame_display_w;
                        height: parent.frame_display_h;
                        mouse-cursor: pointer;
                        moved => {
                            root.on_pointer_move(
                                (self.mouse-x / self.width * root.frame_w * 1px) / 1px,
                                (self.mouse-y / self.height * root.frame_h * 1px) / 1px);
                        }
                        pointer-event(ev) => {
                            if (ev.kind == PointerEventKind.down) {
                                fs.focus();
                            }
                            if (ev.kind == PointerEventKind.down || ev.kind == PointerEventKind.up) {
                                root.on_pointer_button(
                                    (self.mouse-x / self.width * root.frame_w * 1px) / 1px,
                                    (self.mouse-y / self.height * root.frame_h * 1px) / 1px,
                                    ev.button == PointerEventButton.left ? 0
                                        : ev.button == PointerEventButton.middle ? 1 : 2,
                                    ev.kind == PointerEventKind.down);
                            }
                        }
                    }

                    // 画质档位切换（右下角悬浮）
                    Rectangle {
                        x: parent.width - self.width - 12px;
                        y: parent.height - self.height - 12px;
                        width: 132px;
                        height: 28px;
                        border-radius: 8px;
                        background: #1a1a1edd;
                        HorizontalLayout {
                            padding: 2px;
                            spacing: 2px;
                            Rectangle {
                                border-radius: 6px;
                                background: !root.high_quality ? Theme.emerald : transparent;
                                Text {
                                    text: "流畅";
                                    color: white;
                                    font-size: 11px;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }
                                TouchArea {
                                    mouse-cursor: pointer;
                                    clicked => {
                                        root.high_quality = false;
                                        root.set_quality(false);
                                    }
                                }
                            }
                            Rectangle {
                                border-radius: 6px;
                                background: root.high_quality ? Theme.emerald : transparent;
                                Text {
                                    text: "高清";
                                    color: white;
                                    font-size: 11px;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }
                                TouchArea {
                                    mouse-cursor: pointer;
                                    clicked => {
                                        root.high_quality = true;
                                        root.set_quality(true);
                                    }
                                }
                            }
                        }
                    }

                    fs := FocusScope {
                        width: 0px;
                        key-pressed(ev) => {
                            root.on_key(root.key_code(ev.text), true);
                            accept
                        }
                        key-released(ev) => {
                            root.on_key(root.key_code(ev.text), false);
                            accept
                        }
                    }
                }

                // tab 1/2/3 占位（Task 7/8 填充）
                if root.active_tab == 1: Text {
                    text: "远程命令（待填充）";
                    color: Theme.fg-muted;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
                if root.active_tab == 2: Text {
                    text: "远程文件（待填充）";
                    color: Theme.fg-muted;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
                if root.active_tab == 3: Text {
                    text: "即时消息（待填充）";
                    color: Theme.fg-muted;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }
        }
    }
```

- [ ] **Step 2: 编译验证（Slint 编译期校验）**

Run: `cargo build -p client 2>&1 | tail -20`
Expected: 编译通过（Slint 语法正确）。若报 `unknown property`/`syntax error`，对照 slint.md 修正（常见：struct 字段类型、`for x[idx] in [...]` 语法、`property` 必须在元素内声明）。

> 此时 `tab_changed`/`run_command`/`list_*`/`push_file`/`pull_file`/`send_chat` 回调尚未在 Rust 侧 `on_*` 注册——Slint 允许未注册回调（调用即 no-op），不影响编译。Rust 接线在 Task 9。

- [ ] **Step 3: 手动验证（标签切换 + 键鼠门控）**

Run（需两台/两进程，A 主控 B 被控；本机可起两个进程，OHMYDESK_FAKE_CAPTURE=1 验链路）:
```bash
# 终端1（被控 B）
OHMYDESK_FAKE_CAPTURE=1 cargo run -p client -- userB@hostB
# 终端2（主控 A）
cargo run -p client -- userA@hostA
```
手动验：
1. A 输入 B 的 ID + 密码 → 授权通过 → 进入工作台，默认在「远程桌面」标签，能看到 B 的画面（fake 占位帧滚动）。
2. 顶部 4 标签可见，点击「远程命令/远程文件/即时消息」分别切到占位文字页，画面消失；点回「远程桌面」画面恢复。
3. 在「远程命令」标签敲键盘/移动鼠标 → B 端不应收到任何注入（tab 0 外无键鼠捕获）。回「远程桌面」标签敲键 → B 端有反应。
4. 右上「断开」可断开。

- [ ] **Step 4: 提交**

```bash
cargo fmt
git add src/client/ui/app.slint
git commit -m "feat(client-ui): 主控工作台顶部4标签重构 + 桌面键鼠门控tab0"
```

---

### Task 7: 远程命令 / 远程文件 标签页 UI（手动验证）

填充 tab 1（命令）与 tab 2（文件双栏）。命令：输入行 + 输出区；文件：左栏本机（`list_local`）+ 右栏远端（`list_remote`）+ 下发/取回按钮 + 状态行。

**Files:**
- Modify: `src/client/ui/app.slint`（替换 Task 6 的 tab 1 / tab 2 占位）

- [ ] **Step 1: 替换 tab 1 占位为命令页**

把 Task 6 里 `if root.active_tab == 1: Text { ... }` 整块替换为：

```slint
                // tab 1：远程命令
                if root.active_tab == 1: VerticalLayout {
                    padding: 14px;
                    spacing: 10px;
                    Text {
                        text: "远程命令（被控端 Shell 一次性执行）";
                        color: Theme.fg-muted;
                        font-size: 11px;
                        font-weight: 600;
                        letter-spacing: 1px;
                    }
                    // 输出区（可滚动累积）
                    Rectangle {
                        vertical-stretch: 1;
                        border-radius: 10px;
                        background: #0d0d10;
                        border-width: 1px;
                        border-color: Theme.card-border;
                        Flickable {
                            width: 100%;
                            height: 100%;
                            viewport-width: self.width;
                            viewport-height: out_text.preferred-height + 20px;
                            out_text := Text {
                                x: 12px;
                                y: 10px;
                                width: parent.width - 24px;
                                text: root.cmd_output == "" ? "输入命令并回车执行，如 whoami / ls / ipconfig" : root.cmd_output;
                                color: root.cmd_output == "" ? Theme.fg-muted : Theme.fg;
                                font-size: 13px;
                                font-family: "monospace";
                                wrap: word-wrap;
                            }
                        }
                    }
                    // 输入行
                    HorizontalLayout {
                        spacing: 8px;
                        FieldInput {
                            horizontal-stretch: 1;
                            placeholder: "whoami / ls / ipconfig …";
                            text <=> root.cmd_input;
                            accepted => {
                                if (root.cmd_input != "") {
                                    root.run_command(root.cmd_input);
                                    root.cmd_input = "";
                                }
                            }
                        }
                        PrimaryButton {
                            label: "执行";
                            enabled: root.cmd_input != "";
                            clicked => {
                                root.run_command(root.cmd_input);
                                root.cmd_input = "";
                            }
                        }
                    }
                }
```

- [ ] **Step 2: 替换 tab 2 占位为文件双栏页**

先在文件顶部通用控件区（`component HistoryRow` 之后，约 287 行）新增一个目录栏组件：

```slint
// 文件浏览单栏：标题 + 路径行（含上级/刷新）+ 条目列表 + 操作（下发/取回）
component FilePane inherits Rectangle {
    in property <string> title;
    in property <string> path;
    in property <[FileEntry]> entries;
    in property <string> action_label;          // "下发→" 或 "←取回"
    callback open_dir(string /*path*/);          // 进入子目录/上级
    callback refresh();
    callback act(string /*entry_name*/);         // 对选中文件执行下发/取回
    border-radius: 10px;
    background: #0d0d10;
    border-width: 1px;
    border-color: Theme.card-border;
    VerticalLayout {
        padding: 10px;
        spacing: 8px;
        Text {
            text: root.title;
            color: Theme.fg-muted;
            font-size: 11px;
            font-weight: 600;
            letter-spacing: 1px;
        }
        // 路径行
        HorizontalLayout {
            spacing: 6px;
            Text {
                text: root.path == "" ? "/" : root.path;
                color: Theme.fg;
                font-size: 12px;
                font-family: "monospace";
                vertical-alignment: center;
                horizontal-stretch: 1;
                overflow: elide;
            }
            GhostButton {
                label: "上级";
                clicked => {
                    root.open_dir(parent_path(root.path));
                }
            }
            GhostButton {
                label: "刷新";
                clicked => {
                    root.refresh();
                }
            }
        }
        // 条目列表
        Rectangle {
            vertical-stretch: 1;
            Flickable {
                width: 100%;
                height: 100%;
                viewport-width: self.width;
                viewport-height: list_col.preferred-height;
                list_col := VerticalLayout {
                    spacing: 1px;
                    if root.entries.length == 0: Text {
                        text: "（空目录或加载中）";
                        color: Theme.fg-muted;
                        font-size: 12px;
                    }
                    for e in root.entries: Rectangle {
                        height: 32px;
                        border-radius: 6px;
                        background: row_ta.has-hover ? #ffffff0a : transparent;
                        HorizontalLayout {
                            padding-left: 8px;
                            padding-right: 8px;
                            spacing: 8px;
                            Text {
                                text: e.is_dir ? "[目录]" : "[文件]";
                                color: e.is_dir ? Theme.primary : Theme.fg-muted;
                                font-size: 11px;
                                vertical-alignment: center;
                            }
                            Text {
                                text: e.name;
                                color: Theme.fg;
                                font-size: 13px;
                                font-family: "monospace";
                                vertical-alignment: center;
                                horizontal-stretch: 1;
                                overflow: elide;
                            }
                            // 文件才显示下发/取回按钮
                            if !e.is_dir: GhostButton {
                                label: root.action_label;
                                clicked => {
                                    root.act(e.name);
                                }
                            }
                        }
                        row_ta := TouchArea {
                            mouse-cursor: pointer;
                            clicked => {
                                // 点目录进入；点文件不动（用右侧按钮）
                                if (e.is_dir) {
                                    root.open_dir(child_path(root.path, e.name));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

并在 `AppWindow` 内新增路径拼接的 pure function（放在 `key_code` 函数之后，约 356 行）。这些把 Web 端 `sep()/joinPath/parentPath` 逻辑移植到 Slint，按路径是否含 `\` 判 Windows：

```slint
    // 路径分隔符：远端/本机绝对路径含 '\' 视为 Windows，否则 '/'。
    pure function path_is_win(p: string) -> bool {
        return p.contains("\\");
    }
    // 上级目录：去掉最后一段；到根则回空串（被控端空路径=home/盘符列表）。
    pure function parent_path(p: string) -> string {
        if (p == "" || p == "/") {
            return "";
        }
        // 简化：交给 Rust 侧精确处理——Slint 仅触发回调，Rust list 时归一。
        // 这里返回特殊标记 "<up>"，Rust 端据当前 path 计算父目录（见 ui_glue）。
        return "<up>:" + p;
    }
    // 子目录：当前路径拼接子名。同样交 Rust 精确拼接，避免 Slint 字符串能力不足。
    pure function child_path(p: string, name: string) -> string {
        return "<cd>:" + p + "|" + name;
    }
```

> 设计决策：Slint 字符串处理能力弱（无 split/rsplit），故 `parent_path`/`child_path` 只产出**带标记的指令串**（`<up>:当前路径` / `<cd>:当前路径|子名`），由 Rust 侧 `ui_glue` 的回调解析、用标准库 `PathBuf` 精确算父/子目录再 `list_*`（见 Task 9）。这把平台分隔符逻辑收敛到 Rust（已有 `transfer::list_dir` 的 canonicalize 归一），Slint 不重复实现。

把 tab 2 占位替换为双栏：

```slint
                // tab 2：远程文件（左本机 / 右远端）
                if root.active_tab == 2: VerticalLayout {
                    padding: 14px;
                    spacing: 10px;
                    HorizontalLayout {
                        spacing: 12px;
                        FilePane {
                            horizontal-stretch: 1;
                            title: "本机（主控）";
                            path: root.local_path;
                            entries: root.local_entries;
                            action_label: "下发→";
                            open_dir(p) => { root.list_local(p); }
                            refresh => { root.list_local(root.local_path); }
                            act(name) => { root.push_file(name); }
                        }
                        FilePane {
                            horizontal-stretch: 1;
                            title: "远端（被控）";
                            path: root.remote_path;
                            entries: root.remote_entries;
                            action_label: "←取回";
                            open_dir(p) => { root.list_remote(p); }
                            refresh => { root.list_remote(root.remote_path); }
                            act(name) => { root.pull_file(name); }
                        }
                    }
                    // 传输状态行
                    if root.file_notice != "": Text {
                        text: root.file_notice;
                        color: Theme.fg-muted;
                        font-size: 12px;
                        font-family: "monospace";
                        wrap: word-wrap;
                    }
                }
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p client 2>&1 | tail -20`
Expected: 编译通过。常见错误：`overflow: elide` 需 Text 有固定宽度（已给 `horizontal-stretch`/`width`）；`for e in` 列表项内嵌 `if !e.is_dir` 合法。

- [ ] **Step 4: 手动验证（接线后再完整验，本步先确认 UI 渲染不崩）**

Run: `cargo run -p client`，无对端时进不去工作台——故本步只确认 `cargo build` 通过 + Task 9 接线后在 Task 10 完整验。本任务提交即可。

- [ ] **Step 5: 提交**

```bash
cargo fmt
git add src/client/ui/app.slint
git commit -m "feat(client-ui): 远程命令页 + 远程文件双栏页(本机/远端,下发/取回)"
```

---

### Task 8: 即时消息标签页 + 被控端聊天面板（手动验证）

填充 tab 3（主控聊天整页）+ 被控提示条加「聊天」入口弹紧凑回复面板。

**Files:**
- Modify: `src/client/ui/app.slint`（替换 tab 3 占位；被控提示条加聊天入口 + 面板；加被控聊天相关属性）

- [ ] **Step 1: 加被控端聊天属性/回调声明**

在「被控端状态」声明区，`callback stop_being_controlled();`（约 316 行）之后插入：

```slint
    // 被控端聊天面板（被控期间真人可回复）
    in-out property <bool> chat_panel_open: false;   // 面板展开
    in property <bool> controlled_chat_unread: false;// 被控聊天入口红点
    in property <string> controlled_chat_log;        // 被控侧聊天记录累积
    in-out property <string> controlled_chat_input;  // 被控侧输入
    callback send_controlled_chat(string /*text*/);  // 被控发送（复用 SendChat，session=被控会话）
```

- [ ] **Step 2: 替换 tab 3 占位为聊天整页**

把 Task 6 里 `if root.active_tab == 3: Text { ... }` 整块替换为：

```slint
                // tab 3：即时消息（主控侧整页）
                if root.active_tab == 3: VerticalLayout {
                    padding: 14px;
                    spacing: 10px;
                    // 聊天记录（可滚，自动累积）
                    Rectangle {
                        vertical-stretch: 1;
                        border-radius: 10px;
                        background: #0d0d10;
                        border-width: 1px;
                        border-color: Theme.card-border;
                        Flickable {
                            width: 100%;
                            height: 100%;
                            viewport-width: self.width;
                            viewport-height: chat_text.preferred-height + 20px;
                            chat_text := Text {
                                x: 12px;
                                y: 10px;
                                width: parent.width - 24px;
                                text: root.chat_log == "" ? "暂无消息，发送一条试试" : root.chat_log;
                                color: root.chat_log == "" ? Theme.fg-muted : Theme.fg;
                                font-size: 13px;
                                wrap: word-wrap;
                            }
                        }
                    }
                    HorizontalLayout {
                        spacing: 8px;
                        FieldInput {
                            horizontal-stretch: 1;
                            placeholder: "输入消息…";
                            text <=> root.chat_input;
                            accepted => {
                                if (root.chat_input != "") {
                                    root.send_chat(root.chat_input);
                                    root.chat_input = "";
                                }
                            }
                        }
                        PrimaryButton {
                            label: "发送";
                            enabled: root.chat_input != "";
                            clicked => {
                                root.send_chat(root.chat_input);
                                root.chat_input = "";
                            }
                        }
                    }
                }
```

- [ ] **Step 3: 被控提示条加「聊天」入口 + 弹面板**

在 `src/client/ui/app.slint` 的被控提示条里，「我要断开」`GhostButton`（约 387-392 行）之前插入聊天入口按钮：

```slint
                GhostButton {
                    label: root.controlled_chat_unread ? "聊天 ●" : "聊天";
                    clicked => {
                        root.chat_panel_open = !root.chat_panel_open;
                    }
                }
```

并在空闲态 `if !root.remote_active:` 的 `VerticalLayout` 内、被控提示条 `Rectangle` 之后（约 394 行 `}` 之后）插入被控聊天紧凑面板（仅 `being_controlled && chat_panel_open` 显示）：

```slint
        // 被控端聊天紧凑面板（被控期间真人回复）
        if root.being_controlled && root.chat_panel_open: Rectangle {
            height: 220px;
            background: #131316;
            border-width: 1px;
            border-color: Theme.card-border;
            VerticalLayout {
                padding: 10px;
                spacing: 8px;
                HorizontalLayout {
                    Text {
                        text: "与 " + root.peer_name + " 的会话";
                        color: Theme.fg;
                        font-size: 13px;
                        font-weight: 600;
                        vertical-alignment: center;
                        horizontal-stretch: 1;
                    }
                    GhostButton {
                        label: "收起";
                        clicked => {
                            root.chat_panel_open = false;
                        }
                    }
                }
                Rectangle {
                    vertical-stretch: 1;
                    border-radius: 8px;
                    background: #0d0d10;
                    Flickable {
                        width: 100%;
                        height: 100%;
                        viewport-width: self.width;
                        viewport-height: cc_text.preferred-height + 16px;
                        cc_text := Text {
                            x: 10px;
                            y: 8px;
                            width: parent.width - 20px;
                            text: root.controlled_chat_log == "" ? "暂无消息" : root.controlled_chat_log;
                            color: root.controlled_chat_log == "" ? Theme.fg-muted : Theme.fg;
                            font-size: 13px;
                            wrap: word-wrap;
                        }
                    }
                }
                HorizontalLayout {
                    spacing: 8px;
                    FieldInput {
                        horizontal-stretch: 1;
                        placeholder: "回复…";
                        text <=> root.controlled_chat_input;
                        accepted => {
                            if (root.controlled_chat_input != "") {
                                root.send_controlled_chat(root.controlled_chat_input);
                                root.controlled_chat_input = "";
                            }
                        }
                    }
                    PrimaryButton {
                        label: "发送";
                        enabled: root.controlled_chat_input != "";
                        clicked => {
                            root.send_controlled_chat(root.controlled_chat_input);
                            root.controlled_chat_input = "";
                        }
                    }
                }
            }
        }
```

- [ ] **Step 4: 编译验证**

Run: `cargo build -p client 2>&1 | tail -20`
Expected: 编译通过。

- [ ] **Step 5: 提交**

```bash
cargo fmt
git add src/client/ui/app.slint
git commit -m "feat(client-ui): 即时消息整页(主控) + 被控端聊天回复面板"
```

---

### Task 9: `ui_glue.rs` 全接线 + `main.rs`/`workers` 接 tab 切换发 SetCapture（手动验证）

把 Task 6-8 的 Slint 回调接到 `FromUi`，把 Task 4 的 `ToUi` 新变体渲染到 UI 属性。命令/文件/聊天状态在 UI 线程维护累积文本；tab 切换发 `SetCapture` 实现懒推流。

**Files:**
- Modify: `src/client/src/ui_glue.rs:47-258`（`wire_ui_callbacks`：加新回调接线）
- Modify: `src/client/src/ui_glue.rs:266-428`（`consume_to_ui`：渲染新 ToUi）

- [ ] **Step 1: `wire_ui_callbacks` 加新回调接线**

在 `src/client/src/ui_glue.rs` 的 `wire_ui_callbacks` 末尾（`ui.on_cancel_remote(...)` 块之后，函数闭合 `}` 之前，约 257 行）插入。注意保持现有 `cur_session`（主控会话）语义：

```rust
    // ── tab 切换 → 懒推流：tab 0(远程桌面)发 SetCapture{active:true}，其余 false ──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_tab_changed(move |tab| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SetCapture {
                    session_id: sid,
                    active: tab == 0,
                });
            }
        });
    }
    // ── 远程命令：执行 ──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_run_command(move |command| {
            let command = command.to_string();
            if command.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::ExecCommand {
                    session_id: sid,
                    command,
                });
            }
        });
    }
    // ── 远程文件：浏览本机目录（左栏，复用 transfer::list_dir 列本机任意路径）──
    {
        let ui_weak = ui.as_weak();
        ui.on_list_local(move |arg| {
            let arg = arg.to_string();
            let ui_weak = ui_weak.clone();
            // 解析 Slint 传来的指令串（<up>:/<cd>: 标记）→ 目标绝对路径
            let cur = ui_weak.upgrade().map(|u| u.get_local_path().to_string()).unwrap_or_default();
            let target = resolve_path_arg(&arg, &cur);
            // 列目录是阻塞 IO，放后台线程，完成后投回 UI 线程 set。
            std::thread::spawn(move || {
                let listed = crate::transfer::list_dir(&target);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match listed {
                            Ok((dir, entries)) => {
                                ui.set_local_path(dir.into());
                                ui.set_local_entries(build_file_model(&entries));
                            }
                            Err(reason) => {
                                ui.set_file_notice(format!("本机目录读取失败：{reason}").into());
                            }
                        }
                    }
                });
            });
        });
    }
    // ── 远程文件：浏览远端目录（右栏）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_list_remote(move |arg| {
            let arg = arg.to_string();
            let cur = ui_weak.upgrade().map(|u| u.get_remote_path().to_string()).unwrap_or_default();
            let target = resolve_path_arg(&arg, &cur);
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::ListRemote {
                    session_id: sid,
                    path: target,
                });
            }
        });
    }
    // ── 远程文件：下发（左栏选中文件 → 右栏当前目录）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_push_file(move |name| {
            let name = name.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let local_dir = ui.get_local_path().to_string();
                let dest_dir = ui.get_remote_path().to_string();
                let local_path = join_path(&local_dir, &name);
                if let Some(sid) = sess.lock().unwrap().clone() {
                    let _ = tx.send(net::FromUi::PushFile {
                        session_id: sid,
                        local_path,
                        dest_dir,
                    });
                }
            }
        });
    }
    // ── 远程文件：取回（右栏选中文件 → 左栏当前目录）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_pull_file(move |name| {
            let name = name.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let remote_dir = ui.get_remote_path().to_string();
                let local_dir = ui.get_local_path().to_string();
                let remote_path = join_path(&remote_dir, &name);
                if let Some(sid) = sess.lock().unwrap().clone() {
                    let _ = tx.send(net::FromUi::PullFile {
                        session_id: sid,
                        remote_path,
                        local_dir,
                    });
                }
            }
        });
    }
    // ── 即时消息：主控发送（本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_chat_log().to_string();
                    ui.set_chat_log(append_line(&log, "我", &text).into());
                }
            }
        });
    }
    // ── 即时消息：被控发送（用被控会话 ctrl_session，本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_controlled_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_controlled_chat_log().to_string();
                    ui.set_controlled_chat_log(append_line(&log, "我", &text).into());
                }
            }
        });
    }
```

- [ ] **Step 2: 加辅助函数（路径解析/拼接 + 文件模型构造 + 聊天行追加）**

`build_file_model` 需要 `FileEntry`（Slint 生成类型，`slint::include_modules!` 产出，与 `HistoryItem` 同源）。在 `src/client/src/ui_glue.rs` 顶部 `use` 处补 `FileEntry`：

```rust
use crate::{history, net, AppWindow, FileEntry, HistoryItem, SharedSession};
```

在 `build_history_model` 函数（约 44 行）之后追加：

```rust
/// 把 protocol::FileEntry 列表构造为 Slint 列表模型（必须在 UI 线程调用）。
pub fn build_file_model(items: &[protocol::FileEntry]) -> ModelRc<FileEntry> {
    let rows: Vec<FileEntry> = items
        .iter()
        .map(|e| FileEntry {
            name: e.name.clone().into(),
            is_dir: e.is_dir,
            // u64→i32：仅展示用，超 i32 的文件大小极少见，饱和截断不影响功能。
            size: e.size.min(i32::MAX as u64) as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// 解析 Slint 传来的路径指令串 → 目标绝对路径。
/// "<up>:当前路径" → 父目录；"<cd>:当前路径|子名" → 子目录；其余原样（首次/直填路径）。
pub fn resolve_path_arg(arg: &str, cur: &str) -> String {
    if let Some(rest) = arg.strip_prefix("<up>:") {
        parent_of(rest)
    } else if let Some(rest) = arg.strip_prefix("<cd>:") {
        match rest.split_once('|') {
            Some((base, name)) => join_path(base, name),
            None => cur.to_string(),
        }
    } else {
        arg.to_string()
    }
}

/// 父目录：去掉最后一段；到顶（无分隔或仅根）返回空串（被控端空路径=home/盘符列表）。
pub fn parent_of(path: &str) -> String {
    let win = path.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let trimmed = path.trim_end_matches(sep);
    match trimmed.rsplit_once(sep) {
        // Windows 盘根 "C:" → 空（回此电脑）；Unix 根的父 → 空
        Some((head, _)) if head.is_empty() => String::new(),
        Some((head, _)) if win && head.ends_with(':') => format!("{head}{sep}"),
        Some((head, _)) => head.to_string(),
        None => String::new(),
    }
}

/// 拼接目录 + 子名（按 base 是否含 '\' 选分隔符）。base 为空时返回 name 本身。
pub fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }
    let win = base.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let base = base.trim_end_matches(sep);
    format!("{base}{sep}{name}")
}

/// 聊天记录追加一行（"发送者: 文本"），保持纯文本累积（Slint Text 渲染）。
pub fn append_line(log: &str, who: &str, text: &str) -> String {
    if log.is_empty() {
        format!("{who}: {text}")
    } else {
        format!("{log}\n{who}: {text}")
    }
}
```

- [ ] **Step 3: `consume_to_ui` 渲染新 ToUi 变体**

在 `src/client/src/ui_glue.rs` 的 `consume_to_ui` 的 `match ev` 里，`net::ToUi::Disconnected => { ... }` 分支（约 418-425 行）之后插入 5 个新分支：

```rust
            net::ToUi::ExecResult {
                exit_code, stdout, stderr, truncated, duration_ms, ..
            } => {
                let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "无(超时/未启动)".into());
                let mut block = format!("退出码 {code} · 耗时 {duration_ms}ms");
                if !stdout.is_empty() {
                    block.push_str(&format!("\n{stdout}"));
                }
                if !stderr.is_empty() {
                    block.push_str(&format!("\n[stderr] {stderr}"));
                }
                if truncated {
                    block.push_str("\n[输出已截断]");
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let prev = ui.get_cmd_output().to_string();
                        let next = if prev.is_empty() { block } else { format!("{prev}\n\n{block}") };
                        ui.set_cmd_output(next.into());
                    }
                });
            }
            net::ToUi::RemoteEntries { path, entries, error } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match error {
                            Some(reason) => ui.set_file_notice(format!("远端目录读取失败：{reason}").into()),
                            None => {
                                ui.set_remote_path(path.into());
                                ui.set_remote_entries(build_file_model(&entries));
                            }
                        }
                    }
                });
            }
            net::ToUi::FileProgress { name, done, total, .. } => {
                let pct = if total > 0 { done * 100 / total } else { 0 };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(format!("传输中 {name} {pct}%").into());
                    }
                });
            }
            net::ToUi::FileNotice { text } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(text.into());
                    }
                });
            }
            net::ToUi::ChatIncoming { session_id, text, .. } => {
                // 据当前会话角色决定渲染到主控聊天页还是被控聊天面板。
                let is_controlling = cur_session.lock().unwrap().as_deref() == Some(session_id.as_str());
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        if is_controlling {
                            let log = ui.get_chat_log().to_string();
                            ui.set_chat_log(append_line(&log, "对方", &text).into());
                            if ui.get_active_tab() != 3 {
                                ui.set_chat_unread(true);
                            }
                        } else {
                            let log = ui.get_controlled_chat_log().to_string();
                            ui.set_controlled_chat_log(append_line(&log, "对方", &text).into());
                            if !ui.get_chat_panel_open() {
                                ui.set_controlled_chat_unread(true);
                            }
                        }
                    }
                });
            }
```

> 注：`ChatIncoming` 分支用到 `cur_session`，而 `consume_to_ui` 闭包里已 `let ui_weak = ui_weak.clone();`（约 285 行）。`cur_session` 在 `consume_to_ui` 入参里（约 269 行），可直接 `.lock()`。`controlled_chat_unread` 是 `in property`，但本处需 set——把它在 app.slint 改为 `in-out property`（Task 8 已声明为 `in property`，此处需改）。**修正**：把 Task 8 Step 1 的 `in property <bool> controlled_chat_unread` 改为 `in-out property <bool> controlled_chat_unread`，`controlled_chat_log` 改为 `in-out property <string> controlled_chat_log`，`chat_log`/`cmd_output`/`file_notice`/`local_*`/`remote_*` 同理需可 set——见 Step 4 统一核对。

- [ ] **Step 4: 统一核对 app.slint 属性方向（Rust 需 set 的必须 in-out）**

逐项确认下列属性在 `app.slint` 为 `in-out`（Rust 侧 `set_*` 调用要求非 `out`-only；`in` 属性 Rust 可 set，但若 Slint 内也写则需 `in-out`）。Slint 规则：`in` = 外部可写组件内只读，Rust `set_*` 合法；本计划这些属性 Rust 写、Slint 只读，**保持 `in` 即可**，唯独被 Slint 内部也改写的（`active_tab`/`chat_unread`/`cmd_input`/`chat_input`/`chat_panel_open`/`controlled_chat_input`）须 `in-out`。核对清单：

Run: `grep -nE 'property <.*> (active_tab|chat_unread|cmd_input|cmd_output|chat_input|chat_log|chat_panel_open|controlled_chat_unread|controlled_chat_log|controlled_chat_input|local_entries|local_path|remote_entries|remote_path|file_notice)' src/client/ui/app.slint`
Expected: `active_tab`/`chat_unread`/`cmd_input`/`chat_input`/`chat_panel_open`/`controlled_chat_input` 为 `in-out`；其余（Rust 单向写、Slint 只读）为 `in`。若 `controlled_chat_unread`/`controlled_chat_log` 误设 `in` 但需 Rust set——`in` 允许 Rust set，无需改（Slint 内未写它们）。结论：仅确保被 Slint `=` 赋值的属性是 `in-out`，Rust set 对 `in` 合法。

- [ ] **Step 5: 进入工作台时初始化文件双栏（首次列目录）**

主控进入工作台（`RemoteAck`/首帧）后，左右栏需各列一次默认目录。在 `consume_to_ui` 的 `net::ToUi::RemoteAck { session_id }` 分支（约 324 行）的 `invoke_from_event_loop` 闭包里，`ui.set_remote_active(true);` 之后追加触发首次列目录（经回调，复用接线）：

```rust
                        // 进入工作台：左栏列本机 home、右栏列远端默认目录（空路径=被控 home）
                        ui.invoke_list_local("".into());
                        ui.invoke_list_remote("".into());
```

> `invoke_<callback>` 是 Slint 为回调生成的「主动触发」方法，等价于在 UI 线程调用该回调闭包——复用 Task 9 Step 1 的接线逻辑，不重复写列目录代码。

- [ ] **Step 6: 编译验证**

Run: `cargo build -p client 2>&1 | tail -25`
Expected: 编译通过。常见错误：`invoke_list_local` 名称（Slint 回调 `list_local` → `invoke_list_local`）；`FileEntry` 未从 `crate::` 导入；`get_*`/`set_*` 名称连字符转下划线。

- [ ] **Step 7: 单元测试路径辅助函数（TDD 补测，纯函数可测）**

在 `src/client/src/ui_glue.rs` 的 `mod tests`（约 453 行）内追加：

```rust
    #[test]
    fn 路径父级_unix与windows() {
        assert_eq!(parent_of("/home/me/docs"), "/home/me");
        assert_eq!(parent_of("/home"), "");
        assert_eq!(parent_of(r"C:\Users\me"), r"C:\Users");
        assert_eq!(parent_of(r"C:\"), ""); // 盘根回此电脑
    }

    #[test]
    fn 路径拼接_按分隔符() {
        assert_eq!(join_path("/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join_path(r"C:\Users", "a.txt"), r"C:\Users\a.txt");
        assert_eq!(join_path("", "a.txt"), "a.txt");
    }

    #[test]
    fn 指令串解析_up与cd() {
        assert_eq!(resolve_path_arg("<up>:/home/me/docs", ""), "/home/me");
        assert_eq!(resolve_path_arg("<cd>:/home/me|docs", ""), "/home/me/docs");
        assert_eq!(resolve_path_arg("/etc", "/home"), "/etc"); // 直填原样
    }

    #[test]
    fn 聊天行追加() {
        assert_eq!(append_line("", "我", "hi"), "我: hi");
        assert_eq!(append_line("我: hi", "对方", "yo"), "我: hi\n对方: yo");
    }
```

Run: `cargo test -p client -- --test-threads=1 路径 指令串 聊天行 2>&1 | tail -10`
Expected: 4 个测试 PASS。

- [ ] **Step 8: 提交**

```bash
cargo fmt
cargo clippy -p client -- -D warnings
git add src/client/src/ui_glue.rs src/client/ui/app.slint
git commit -m "feat(client): ui_glue 全接线命令/文件/聊天 + tab切换发SetCapture懒推流 + 路径辅助"
```

---

### Task 10: 全链路 A/B 手测 + 质量门

**Files:** 无（验证 + 修复）

- [ ] **Step 1: 工作区质量门**

Run:
```bash
cargo fmt --check
cargo clippy -p client -- -D warnings
cargo test -p client -- --test-threads=1 2>&1 | tail -8
```
Expected: fmt 无差异；clippy 零警告；client 全部测试 PASS。

- [ ] **Step 2: 起 A/B 两进程**

```bash
# 终端1（被控 B，无真实屏用占位帧验链路）
OHMYDESK_FAKE_CAPTURE=1 OHMYDESK_SERVER=wss://rc.guoziweb.com/ws cargo run -p client -- userB@hostB
# 终端2（主控 A）
OHMYDESK_SERVER=wss://rc.guoziweb.com/ws cargo run -p client -- userA@hostA
```
（或用本地 server：先 `cargo run -p server`，两端设 `OHMYDESK_SERVER=ws://127.0.0.1:<port>/ws`。）

- [ ] **Step 3: 逐功能点验（A 主控 / B 被控）**

1. **授权 + 桌面**：A 填 B 的 ID+密码 → B 弹授权 → 同意 → A 进工作台「远程桌面」标签见画面。
2. **懒推流**：A 切到「远程命令」标签 → B 端推帧停止（看 B 日志 `懒推流开关 active=false` 或 CPU 降）；切回「远程桌面」→ 画面恢复（`active=true`）。
3. **远程命令**：A 在命令标签输入 `whoami`（Linux）/`ipconfig`（Win）回车 → 输出区显示退出码+耗时+stdout。输入错误命令 → 显示 stderr。
4. **远程文件·浏览**：A 切「远程文件」→ 左栏自动列 A 本机 home、右栏列 B home；点目录进入、点「上级」回退、点「刷新」。
5. **远程文件·下发**：A 左栏选一文件点「下发→」→ B 端 recv/远端当前目录出现该文件 → A 状态行显示「已下发到远端：<路径>」。
6. **远程文件·取回**：A 右栏选 B 的一文件点「←取回」→ A 本机左栏当前目录出现该文件 → 状态行「已取回到本机：<路径>」。
7. **即时消息**：A 在「即时消息」标签发消息 → B 被控提示条「聊天 ●」红点亮 → B 点开面板见消息 → B 回复 → A 标签若不在消息页则显红点，切过去见回复。
8. **键鼠不串台**：A 在命令/文件/消息标签敲键 → B 无注入；回桌面标签敲键 → B 有反应。
9. **断开**：A 点「断开」→ 双方回到空闲态，B 提示条消失。

- [ ] **Step 4: 修复手测暴露的问题（如有）**

按 `superpowers:systematic-debugging` 定位；每个修复补对应单测（逻辑层）或记录手测复验（UI 层）。修复后重跑 Step 1 质量门。

- [ ] **Step 5: 最终提交（若 Step 4 有改动）**

```bash
cargo fmt
git add -A
git commit -m "fix(client): 修复全链路手测暴露的问题（命令/文件/消息/懒推流）"
```

---

## Self-Review

**Spec 逐条覆盖（对照 spec §3 懒推流、§5 命令、§6 文件、§8 消息、§9 UX 标签、§11 路径、§12 边界）：**
- §3 懒推流：主控切 tab 发 SetCapture → Task 9 Step 1（`on_tab_changed`）；被控收 SetCapture 启停采集 → Task 5 ✓
- §5 远程命令（Slint 主控端）：FromUi::ExecCommand → ExecRequest（Task 2）、下行 ExecResult → ToUi（Task 4）、命令标签 UI（Task 7）、ui_glue 接线（Task 9）✓
- §6 远程文件（左本机/右远端、下发、取回）：transfer::send_file_push + PULL_TARGETS（Task 3）、主控 pull 回流落盘（Task 4）、双栏 UI（Task 7）、list_local 用 transfer::list_dir 列本机（Task 9 Step 1）✓
- §8 即时消息双向：SendChat → ChatMessage（Task 2）、下行 ChatMessage → ToUi（Task 4）、主控整页 + 被控面板（Task 8）、双向接线含本地回显（Task 9）✓
- §9 UX 顶部 4 标签整页同窗 + 未读红点 + 键鼠仅桌面标签：Task 6（标签骨架 + tab0 门控键鼠）✓
- §11 路径分隔符按远端 OS：parent_of/join_path 按是否含 `\` 判（Task 9 Step 2 + Step 7 测试）✓；本机列目录复用 transfer::list_dir 的 canonicalize 归一 ✓
- §12 边界：坏路径回 FileNotice 不 panic（Task 4）、命令超时落 stderr（被控既有 exec.rs）、传输失败 abort + 通知（Task 4 Step 3d）、断开清状态（既有 SessionEnded 流，Task 6 保留）✓
- 非目标（YAGNI）未逾矩：无断点续传/递归/PTY/群聊/已读回执 ✓

**占位符扫描：** 全文无 TBD/TODO/「类似上面」。tab 1/2/3 在 Task 6 是**明确的临时占位 Text**（带「待填充」字样），Task 7/8 给出完整替换代码，非遗留占位。每个代码步骤可直接粘贴。

**类型一致性（FromUi/ToUi 字段名跨任务核对）：**
- FromUi 新变体字段：`ExecCommand{session_id,command}`、`ListRemote{session_id,path}`、`PushFile{session_id,local_path,dest_dir}`、`PullFile{session_id,remote_path,local_dir}`、`SendChat{session_id,text}`、`SetCapture{session_id,active}` — Task 1 定义、Task 2 映射、Task 9 接线三处字段名完全一致 ✓
- ToUi 新变体字段：`ExecResult{exec_id,command,exit_code,stdout,stderr,truncated,duration_ms}`、`RemoteEntries{path,entries,error}`、`FileProgress{transfer_id,name,done,total}`、`FileNotice{text}`、`ChatIncoming{session_id,msg_id,text}` — Task 1 定义、Task 4 构造、Task 9 渲染三处一致 ✓
- 复用协议字段与计划①/protocol 一致：`ExecResult.exit_code: Option<i32>`、`FileOpen.dest: Option<String>`、`FileEntry{name,is_dir,size:u64}`、`FileDir::{Push,Pull}` ✓
- Slint `struct FileEntry{name,is_dir,size:int}` 与 Rust `build_file_model` 的 u64→i32 饱和映射对应（Task 6 声明、Task 9 构造）✓

**依赖顺序提醒（执行者必读）：** Task 2 的 `PushFile/PullFile` 分支依赖 Task 3 的 `send_file_push/set_pull_target` —— 实际编码顺序为 **Task 1 → Task 3 → Task 2 → Task 4 → Task 5**（逻辑层），统一在 Task 3 Step 6 提交；再 Task 6 → 7 → 8 → 9 → 10（UI 层，逐任务提交）。CAPTURE_CTRL 全局 OnceLock 在测试中跨用例共享，client 测试一律 `--test-threads=1`。

**红线核对：** 提交前 fmt + clippy -D warnings（每个提交步骤含）；不对外部输入 unwrap（路径/命令/消息均 best-effort，坏路径回 Notice）；id 用 AtomicU64 计数器（Task 2 `next_id`）非随机/时间；改 Slint 前提示读 rust-remote-control-stack（开头风险节）✓
