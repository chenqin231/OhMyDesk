# 远程工具集·计划①：协议 + 服务端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在协议层新增即时消息(`ChatMessage`)与懒推流开关(`SetCapture`)消息、`AuditType::Chat` 审计类型,并在服务端中枢路由聊天(落审计)与转发懒推流信号——为客户端/Web 端三功能提供契约与中转底座。

**Architecture:** 单一事实源在 `src/protocol`(serde + ts-rs 自动导出 TS)。聊天/懒推流都是「会话内消息」,沿用现有 `route_to_peer(session_id)` 对端路由;聊天额外复用 `audit.log` 落 SQLite。命令/文件相关协议与路由已存在,本计划零改动。

**Tech Stack:** Rust、serde、ts-rs、sqlx(SQLite)、tokio、dashmap。

**前置:** 已在 `feature/remote-command-file-chat` 分支(spec 见 `docs/superpowers/specs/2026-06-30-remote-command-file-chat-design.md`)。

---

### Task 1: 协议新增 `ChatMessage` 变体

**Files:**
- Modify: `src/protocol/src/lib.rs:237-240`(在 `ClipboardSync` 后插入)
- Test: `src/protocol/src/tests.rs`

- [ ] **Step 1: 写失败测试**

在 `src/protocol/src/tests.rs` 末尾(`export_all` 之前)追加:

```rust
#[test]
fn chat_message_tagged() {
    let env = Envelope {
        from: "ep-1".into(),
        to: None,
        ts: 0,
        payload: Message::ChatMessage {
            session_id: "s-1".into(),
            msg_id: "m-1".into(),
            text: "你好".into(),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"chat_message\""));
    assert!(json.contains("\"text\":\"你好\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.payload, Message::ChatMessage { .. }));
}
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `cargo test -p protocol chat_message_tagged`
Expected: 编译错误 `no variant named ChatMessage`。

- [ ] **Step 3: 加变体**

在 `src/protocol/src/lib.rs` 的 `ClipboardSync { ... }` 块(约 237-240 行)之后插入:

```rust
    /// 会话内即时消息(双向,主控↔被控)。按 session 对端路由(同 ClipboardSync);
    /// server 转发同时落 AuditType::Chat 审计(全文)。
    ChatMessage {
        session_id: String,
        msg_id: String,
        text: String,
    },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p protocol chat_message_tagged`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/protocol/src/lib.rs src/protocol/src/tests.rs
git commit -m "feat(protocol): 新增 ChatMessage 会话内即时消息变体"
```

---

### Task 2: 协议新增 `SetCapture` 懒推流开关

**Files:**
- Modify: `src/protocol/src/lib.rs:226-229`(在 `SetQuality` 后插入)
- Test: `src/protocol/src/tests.rs`

- [ ] **Step 1: 写失败测试**

在 `src/protocol/src/tests.rs` 追加:

```rust
#[test]
fn set_capture_tagged() {
    let env = Envelope {
        from: "ep-1".into(),
        to: None,
        ts: 0,
        payload: Message::SetCapture {
            session_id: "s-1".into(),
            active: false,
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"set_capture\""));
    assert!(json.contains("\"active\":false"));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.payload, Message::SetCapture { active: false, .. }));
}
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `cargo test -p protocol set_capture_tagged`
Expected: 编译错误 `no variant named SetCapture`。

- [ ] **Step 3: 加变体**

在 `src/protocol/src/lib.rs` 的 `SetQuality { ... }` 块(约 226-229 行)之后插入:

```rust
    /// 主控→被控:会话内帧推流开关(懒推流——主控仅在「远程桌面」标签需要帧)。
    /// active=false 暂停采集推帧, true 恢复。按 session 对端路由(同 SetQuality);不审计(纯传输优化)。
    SetCapture {
        session_id: String,
        active: bool,
    },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p protocol set_capture_tagged`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/protocol/src/lib.rs src/protocol/src/tests.rs
git commit -m "feat(protocol): 新增 SetCapture 懒推流开关变体"
```

---

### Task 3: 协议 + 服务端新增 `AuditType::Chat`

**Files:**
- Modify: `src/protocol/src/lib.rs:413-422`(`AuditType` 枚举)
- Modify: `src/server/src/audit.rs:174-185`(`audit_type_str`)、`src/server/src/audit.rs:215-236`(`From<AuditLogRow>`)
- Test: `src/server/src/audit.rs`(新增 `#[cfg(test)] mod tests`)

- [ ] **Step 1: 写失败测试**

在 `src/server/src/audit.rs` 文件末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_audit_type_str_and_back() {
        // 枚举 → 字符串
        assert_eq!(audit_type_str(AuditType::Chat), "chat");
        // 字符串行 → 枚举(往返)
        let row = AuditLogRow {
            id: "a1".into(),
            session_id: "s1".into(),
            ts: 0,
            actor_id: "ep-1".into(),
            event_type: "chat".into(),
            text: "你好".into(),
        };
        let log = AuditLog::from(row);
        assert!(matches!(log.kind, AuditType::Chat));
        assert_eq!(log.text, "你好");
    }
}
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `cargo test -p server chat_audit_type_str_and_back`
Expected: 编译错误 `no variant named Chat`。

- [ ] **Step 3a: 协议加枚举值**

在 `src/protocol/src/lib.rs` 的 `AuditType` 枚举(`FileTransfer` 行后,约 421 行)追加:

```rust
    Chat,         // 会话内即时消息
```

- [ ] **Step 3b: 服务端补两处映射**

在 `src/server/src/audit.rs` 的 `audit_type_str` 匹配臂(`AuditType::FileTransfer => "file_transfer",` 后)追加:

```rust
        AuditType::Chat => "chat",
```

在同文件 `impl From<AuditLogRow> for AuditLog` 的 `match r.event_type.as_str()`(`"file_transfer" => AuditType::FileTransfer,` 后)追加:

```rust
            "chat" => AuditType::Chat,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p server chat_audit_type_str_and_back`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/protocol/src/lib.rs src/server/src/audit.rs
git commit -m "feat(protocol,server): 新增 AuditType::Chat 审计类型与映射"
```

---

### Task 4: 服务端中枢路由 `ChatMessage`(转发 + 落审计)

**Files:**
- Modify: `src/server/src/hub.rs:275`(在 `// server 单向发出` 臂之前插入新分支)
- Test: `src/server/src/hub.rs`(`mod tests`)

- [ ] **Step 1: 写失败测试**

在 `src/server/src/hub.rs` 的 `mod tests` 内(`file_list_request_forwarded_to_controlled_peer` 之后)追加:

```rust
    /// 即时消息:必须按 session 路由给对端,且落审计不 panic,不回发给发送方。
    #[tokio::test]
    async fn chat_message_forwarded_to_peer() {
        let hub = test_hub();
        let (a_tx, mut a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-chat".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
        });

        let env = Envelope {
            from: "ep-a".into(),
            to: None,
            ts: 200,
            payload: Message::ChatMessage {
                session_id: sid.clone(),
                msg_id: "m-1".into(),
                text: "你好".into(),
            },
        };
        hub.handle(env, 200).await;

        let got = b_rx.try_recv().expect("对端应收到 ChatMessage");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::ChatMessage { text, .. } => assert_eq!(text, "你好"),
            other => panic!("应为 ChatMessage,实际 {other:?}"),
        }
        assert!(a_rx.try_recv().is_err(), "不应回发给发送方");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server chat_message_forwarded_to_peer`
Expected: 编译失败(`ChatMessage` 未在 `match` 覆盖会触发非穷尽错误)或断言失败。

- [ ] **Step 3: 加路由分支**

在 `src/server/src/hub.rs` 的 `handle()` 中,`// server 单向发出的消息` 注释(约 276 行)**之前**插入:

```rust
            // ── 会话内即时消息:按 session 对端路由 + 落 Chat 审计(全文)──────────
            Message::ChatMessage {
                session_id, text, ..
            } => {
                self.audit
                    .log(session_id, &env.from, AuditType::Chat, text)
                    .await;
                self.route_to_peer(session_id, &env);
            }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p server chat_message_forwarded_to_peer`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/server/src/hub.rs
git commit -m "feat(server): 中枢路由 ChatMessage 并落 Chat 审计"
```

---

### Task 5: 服务端中枢转发 `SetCapture`(懒推流信号)

**Files:**
- Modify: `src/server/src/hub.rs:200-205`(并入 `route_to_peer` 分支组)
- Test: `src/server/src/hub.rs`(`mod tests`)

- [ ] **Step 1: 写失败测试**

在 `src/server/src/hub.rs` 的 `mod tests` 内追加:

```rust
    /// 懒推流信号:SetCapture 必须按 session 路由给对端(被控端据此启停采集)。
    #[tokio::test]
    async fn set_capture_forwarded_to_peer() {
        let hub = test_hub();
        let (a_tx, _a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-cap".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
        });

        let env = Envelope {
            from: "ep-a".into(),
            to: None,
            ts: 200,
            payload: Message::SetCapture {
                session_id: sid.clone(),
                active: false,
            },
        };
        hub.handle(env, 200).await;

        let got = b_rx.try_recv().expect("对端应收到 SetCapture");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        assert!(matches!(env.payload, Message::SetCapture { active: false, .. }));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server set_capture_forwarded_to_peer`
Expected: 编译失败(`match` 非穷尽,`SetCapture` 未覆盖)。

- [ ] **Step 3: 并入路由分支组**

在 `src/server/src/hub.rs` 的 `handle()` 中,把 `SetCapture` 加入既有 `Frame/RemoteNotice/SetQuality/ClipboardSync` 的 `route_to_peer` 分支组(约 200-205 行):

```rust
            // ── Frame / RemoteNotice / SetQuality / SetCapture / Clipboard：按 session 对端路由 ──
            Message::Frame { session_id, .. }
            | Message::RemoteNotice { session_id, .. }
            | Message::SetQuality { session_id, .. }
            | Message::SetCapture { session_id, .. }
            | Message::ClipboardSync { session_id, .. } => {
                self.route_to_peer(session_id, &env);
            }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p server set_capture_forwarded_to_peer`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/server/src/hub.rs
git commit -m "feat(server): 中枢转发 SetCapture 懒推流信号"
```

---

### Task 6: 重新导出 TS 类型 + 工作区质量门

新协议类型(`ChatMessage`/`SetCapture` 在 `Message` 链、`AuditType::Chat` 在 `AuditLog` 链)由现有 `export_all` 自动带出,无需改 `export_all`;但**必须重跑**以更新 admin-web 生成物。

**Files:**
- 自动生成: `src/admin-web/src/lib/types/*.ts`(禁手改)

- [ ] **Step 1: 重新导出 TS 类型**

Run: `cargo test -p protocol export_all`
Expected: PASS;`git status` 显示 `src/admin-web/src/lib/types/` 下文件有变更(`Message.ts`/`AuditType.ts` 等含 `ChatMessage`/`SetCapture`/`"chat"`)。

- [ ] **Step 2: 全工作区质量门(项目红线)**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: fmt 无改动残留、clippy 零警告、全部测试 PASS。

> enigo 相关测试需串行:若 client 包测试受影响,用 `cargo test -p client -- --test-threads=1`。本计划不动 client,通常无需。

- [ ] **Step 3: 提交**

```bash
git add src/admin-web/src/lib/types
git commit -m "chore(admin-web): 重新导出 ts-rs 类型(ChatMessage/SetCapture/Chat)"
```

---

## Self-Review

**Spec coverage(对照 spec §4、§5/§6 复用、§8、§10、§3 懒推流):**
- §4 协议 `ChatMessage` → Task 1 ✓;`AuditType::Chat` → Task 3 ✓;懒推流信号(选定 `SetCapture` 方案,而非改 `SetQuality`)→ Task 2 ✓
- §8 服务端 chat 路由 + 审计 → Task 4 ✓;audit.rs 两处映射 → Task 3 ✓
- §3 懒推流服务端转发 → Task 5 ✓
- ts-rs 重导出(§4 末)→ Task 6 ✓
- 命令/文件协议与服务端路由:已存在,本计划不动(spec §5/§6 标注后端复用)✓

**Placeholder scan:** 无 TBD/TODO;每步含完整代码与精确命令。

**Type consistency:** `ChatMessage{session_id, msg_id, text}`、`SetCapture{session_id, active}`、`AuditType::Chat`/`"chat"` 在 Task 1/2/3/4/5 引用一致;测试构造的 `AuditLogRow` 字段(id/session_id/ts/actor_id/event_type/text)与 audit.rs 定义一致。

**懒推流方案锁定:** spec 留的二选一,本计划锁定为「新增 `SetCapture` 消息」(不污染 `QualityMode` 画质选择器),Plan② 被控端据此启停采集、Plan② 主控端切标签时发送、Plan③ admin-web 切标签时发送。
