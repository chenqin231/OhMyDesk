# 远程延迟梯队1 实现计划（Spec D）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** 消除远控"点击后画面慢"——帧生产侧 drop-stale（client + server 双 lane）、drag-aware 注入侧 move 合并、事件驱动抓帧。

**Architecture:** 把可丢的 Frame 与可靠的控制消息分到两条 lane：Frame 走 `watch`（单槽最新帧，drop-stale），控制消息走原有可靠 FIFO mpsc。客户端在 `conn.rs` 出站泵 select 双 lane；服务端用**附加式** `frame_clients`（不动现有 `clients`，blast radius 最小）。注入侧 drag-aware 合并悬停 move、拖拽全保真。抓帧改小粒度轮询 + `LAST_INPUT_MS` 事件触发。

**Tech Stack:** Rust / tokio（watch + mpsc + select）/ Slint client / axum server。

**对应 Spec:** `docs/superpowers/specs/2026-06-30-remote-latency-tier1-design.md`

**硬约束（贯穿全程）：仅 `Frame` 可合并丢弃；所有控制消息走可靠 FIFO，零丢失、不乱序。**

---

## Task 1（C）：drag-aware 注入侧 move 合并

**Files:**
- Modify: `src/client/src/workers.rs`（consume_inject 注入线程）

> 现状（workers.rs:30-53）：tokio 侧 `while rx.recv() → blk_tx.send(ev)`；std 线程 `while blk_rx.recv() → injector.apply(&ev)`。改造注入线程为批量抽干 + drag-aware 合并。纯本地逻辑、无跨模块耦合。

- [ ] **Step 1：写合并纯函数 + 单测（TDD RED→GREEN）**

在 `src/client/src/workers.rs` 顶部（`consume_inject` 之前）加纯函数 + 测试。该函数把一批事件按 drag-aware 规则压平成"实际要注入的序列"：

```rust
/// drag-aware 合并：把一批输入事件压平为实际注入序列。
/// 规则：buttons_down>0(拖拽中) 的 MouseMove 全保留；buttons_down==0(悬停) 的连续 MouseMove
/// 只保留每段最后一个；任何非 move 事件前先 flush 暂存的悬停 move（保证点击前光标到位）。
/// `buttons_down` 以引用传入并就地更新（跨批次保持按键状态）。
fn coalesce_inputs(
    batch: Vec<protocol::InputEvent>,
    buttons_down: &mut i32,
) -> Vec<protocol::InputEvent> {
    use protocol::InputEvent::*;
    let mut out = Vec::with_capacity(batch.len());
    let mut pending_move: Option<protocol::InputEvent> = None;
    for ev in batch {
        match &ev {
            MouseMove { .. } => {
                if *buttons_down > 0 {
                    out.push(ev); // 拖拽：全保真
                } else {
                    pending_move = Some(ev); // 悬停：覆盖暂存
                }
            }
            other => {
                if let Some(m) = pending_move.take() {
                    out.push(m); // 非 move 前 flush 悬停 move
                }
                if let MouseButton { down, .. } = other {
                    if *down {
                        *buttons_down += 1;
                    } else {
                        *buttons_down = (*buttons_down - 1).max(0);
                    }
                }
                out.push(ev);
            }
        }
    }
    if let Some(m) = pending_move.take() {
        out.push(m);
    }
    out
}

#[cfg(test)]
mod coalesce_tests {
    use super::coalesce_inputs;
    use protocol::InputEvent::{Key, MouseButton, MouseMove};

    #[test]
    fn 悬停连续move只留最后一个() {
        let mut b = 0;
        let out = coalesce_inputs(
            vec![
                MouseMove { x: 1, y: 1 },
                MouseMove { x: 2, y: 2 },
                MouseMove { x: 3, y: 3 },
            ],
            &mut b,
        );
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], MouseMove { x: 3, y: 3 }));
    }

    #[test]
    fn 拖拽中move全保留() {
        let mut b = 0;
        // down 后两 move 再 up：down/up 之间的 move 必须全保留（拖拽保真）
        let out = coalesce_inputs(
            vec![
                MouseButton { button: 0, down: true },
                MouseMove { x: 1, y: 1 },
                MouseMove { x: 2, y: 2 },
                MouseButton { button: 0, down: false },
            ],
            &mut b,
        );
        // down + move1 + move2 + up = 4 条，无一丢失
        assert_eq!(out.len(), 4);
        assert_eq!(b, 0, "按键状态应回到 0");
    }

    #[test]
    fn 点击前flush悬停move() {
        let mut b = 0;
        let out = coalesce_inputs(
            vec![
                MouseMove { x: 5, y: 5 },
                MouseMove { x: 9, y: 9 }, // 悬停合并到 9,9
                MouseButton { button: 0, down: true },
            ],
            &mut b,
        );
        // 应为 move(9,9) + button down，点击前光标到位
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], MouseMove { x: 9, y: 9 }));
        assert!(matches!(out[1], MouseButton { down: true, .. }));
    }

    #[test]
    fn 跨批次保持按键状态() {
        let mut b = 0;
        // 批1：按下
        let _ = coalesce_inputs(vec![MouseButton { button: 0, down: true }], &mut b);
        assert_eq!(b, 1);
        // 批2：仅 move——此时仍在拖拽，应保留
        let out = coalesce_inputs(vec![MouseMove { x: 1, y: 1 }, MouseMove { x: 2, y: 2 }], &mut b);
        assert_eq!(out.len(), 2, "跨批次拖拽中 move 应全保留");
    }

    #[test]
    fn key事件前也flush悬停move() {
        let mut b = 0;
        let out = coalesce_inputs(
            vec![MouseMove { x: 7, y: 7 }, Key { code: "a".into(), down: true }],
            &mut b,
        );
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], MouseMove { x: 7, y: 7 }));
    }
}
```

- [ ] **Step 2：跑测试验证纯函数**

Run: `cargo test -p client coalesce`
Expected: 5 个测试通过。

- [ ] **Step 3：注入线程接入批量抽干 + 合并**

把 `consume_inject`（workers.rs:30-53）的 std 注入线程循环改为批量抽干 + coalesce。原 `while let Ok(ev) = blk_rx.recv()` 改为：

```rust
        // drag-aware 合并：每轮 recv 一个后抽干当前积压，压平悬停 move（拖拽保真），逐条注入。
        let mut buttons_down: i32 = 0;
        while let Ok(first) = blk_rx.recv() {
            let mut batch = vec![first];
            while let Ok(ev) = blk_rx.try_recv() {
                batch.push(ev);
            }
            for ev in coalesce_inputs(batch, &mut buttons_down) {
                // D：注入 button/key 后标记输入时刻，驱动事件驱动抓帧（见 Task 4）。
                let is_actionable = !matches!(ev, protocol::InputEvent::MouseMove { .. });
                match injector.apply(&ev) {
                    Ok(()) => {}
                    Err(e) => tracing::warn!("注入失败 ev={ev:?}：{e}"),
                }
                if is_actionable {
                    crate::workers::mark_input_now();
                }
            }
        }
```

> `mark_input_now()` 在 Task 4 定义（写 LAST_INPUT_MS）。本 Task 先实现到此（mark_input_now 还不存在则编译失败）——故 Task 1 与 Task 4 的 LAST_INPUT_MS 定义需合并提交，或先桩 `mark_input_now`。**实现顺序：先做 Task 4 的 LAST_INPUT_MS + mark_input_now 定义，再做本 Step 3。** 见 Task 4 说明。

- [ ] **Step 4：编译 + 注入相关测试**

Run: `cargo test -p client`（含 coalesce_tests）
Expected: 通过（mark_input_now 已在 Task 4 定义）。

- [ ] **Step 5：提交**

```bash
git add src/client/src/workers.rs
git commit -m "feat(client): drag-aware 注入侧 move 合并(拖拽保真)"
```

---

## Task 2（D-1）：LAST_INPUT_MS 信号 + capture 间隔

**Files:**
- Modify: `src/client/src/workers.rs`（LAST_INPUT_MS + mark_input_now + last_input_after）
- Modify: `src/client/src/capture.rs`（流畅档 interval 62→40）

> **本 Task 先于 Task 1 的 Step 3 实现**（提供 mark_input_now）。

- [ ] **Step 1：capture 流畅档间隔 62→40ms + 调整断言**

`src/client/src/capture.rs` `params_for` 的 Smooth 分支（:45-50）`interval_ms: 62` 改为 `interval_ms: 40`。

同步改 `画质档位_参数符合预期` 测试（capture.rs:262）的注释无需改，但确认 `sm.interval_ms < hq.interval_ms`（40 < 100）仍成立——无需改断言。

- [ ] **Step 2：workers.rs 加 LAST_INPUT_MS + 辅助 + 单测**

在 `src/client/src/workers.rs` 顶部 `use` 区下方加：

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// 最近一次"可见输入"(button/key)注入的 Unix 毫秒时刻。
/// 注入线程写、抓帧线程读——事件驱动抓帧的唯一跨线程信号（KISS：只传时间戳，非业务态）。
static LAST_INPUT_MS: AtomicU64 = AtomicU64::new(0);

/// 注入线程在注入 button/key 后调用，标记"刚发生输入"。
pub fn mark_input_now() {
    LAST_INPUT_MS.store(now_ms(), Ordering::Relaxed);
}

/// LAST_INPUT_MS 是否晚于给定时刻（抓帧线程判断"上次抓帧后是否有新输入"）。
fn last_input_after(since_ms: u64) -> bool {
    LAST_INPUT_MS.load(Ordering::Relaxed) > since_ms
}

/// 当前 Unix 毫秒。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
```

并加单测：

```rust
#[cfg(test)]
mod input_signal_tests {
    use super::{last_input_after, mark_input_now, now_ms};

    #[test]
    fn mark_后_last_input_after_为真() {
        let before = now_ms().saturating_sub(1);
        mark_input_now();
        assert!(last_input_after(before), "mark 后应晚于此前时刻");
        // 远未来时刻：不应晚于
        assert!(!last_input_after(now_ms() + 10_000));
    }
}
```

- [ ] **Step 3：跑测试**

Run: `cargo test -p client input_signal && cargo test -p client 画质档位`
Expected: 通过。

- [ ] **Step 4：提交（与 Task 1 Step 3 协调）**

> 因 Task 1 Step 3 调用 `mark_input_now`，本 Task 的定义必须先编译通过。建议提交顺序：本 Task（Step 1-3）先提交，再做 Task 1 Step 3。

```bash
git add src/client/src/workers.rs src/client/src/capture.rs
git commit -m "feat(client): LAST_INPUT_MS 输入信号 + 流畅档间隔 40ms"
```

---

## Task 3（D-2）：事件驱动抓帧

**Files:**
- Modify: `src/client/src/workers.rs`（consume_capture 抓帧线程循环）

> 现状（workers.rs:109-183）：抓帧线程 `loop { sleep(interval_ms); ... 截帧 ... }`。改为小粒度轮询 + 事件触发，并 coalesce（单轮最多一帧，防 input 洪泛 DoS）。

- [ ] **Step 1：改抓帧循环为小粒度轮询 + 事件触发**

把抓帧线程主循环（workers.rs:109-112 的 `loop { let qp=...; sleep(qp.interval_ms); ... }`）改为：

```rust
            let mut last_cap_ms: u64 = 0;
            const TICK_MS: u64 = 16; // 轮询粒度（≤60fps 上限，coalesce input 洪泛）
            loop {
                std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                let qp = capture::current_params();
                let now = now_ms();
                // 满间隔 或 上次抓帧后有新输入 → 抓一帧；否则轻睡继续。
                let due = now.saturating_sub(last_cap_ms) >= qp.interval_ms;
                let input_driven = last_input_after(last_cap_ms);
                if !due && !input_driven {
                    continue;
                }
                last_cap_ms = now;
                let sid = match active.lock().unwrap().clone() {
                    Some(s) => s,
                    None => continue, // 未在被控态，空转
                };
                // ... 以下抓帧/编码/发送逻辑保持不变（fake / Wayland / 懒构造 / frame_q / from_ui_tx.send）...
```

> 注意：原循环体内 `let sid = match active.lock()...` 已存在（workers.rs:113），改造后**不要重复**——把新的「轮询节流 + due/input 判定 + last_cap_ms 更新」插在 sleep 与取 sid 之间，删掉原 `let qp = current_params(); sleep(qp.interval_ms);` 两行。其余截帧逻辑（fake/Wayland/capturer 懒构造/frame_q/from_ui_tx.send）原样保留。

- [ ] **Step 2：编译验证（抓帧逻辑无法纯单测，靠编译 + 后续真机）**

Run: `cargo build -p client`
Expected: 编译通过。

- [ ] **Step 3：提交**

```bash
git add src/client/src/workers.rs
git commit -m "feat(client): 事件驱动抓帧(输入后即时, coalesce<=60fps)"
```

---

## Task 4（A）：被控端帧 drop-stale（conn.rs 双 lane 泵）

**Files:**
- Modify: `src/client/src/net/conn.rs`

> **风险最高**：重构出站泵。硬约束：控制消息走 `out_tx`（可靠 FIFO），**仅 Frame** 走 `frame_tx`（watch 单槽最新）。

- [ ] **Step 1：connect_once 建 frame watch + 改造出站泵**

在 `src/client/src/net/conn.rs` `connect_once` 内，把出站泵（conn.rs:37-45）替换为双 lane：

```rust
    // ── 出站泵：控制消息走可靠 FIFO out_tx；帧走 frame watch（单槽最新，drop-stale）──
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased; // 控制消息优先（input/心跳/控制绝不被帧延迟）
                ctrl = out_rx.recv() => {
                    match ctrl {
                        Some(text) => {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        None => break, // 控制通道关闭 = 连接结束
                    }
                }
                changed = frame_rx.changed() => {
                    if changed.is_err() {
                        // frame_tx 已 drop：转入纯控制循环直到关闭（不丢控制消息）
                        while let Some(text) = out_rx.recv().await {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        break;
                    }
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some(text) = latest {
                        if write.send(WsMsg::Text(text)).await.is_err() { break; }
                    }
                }
            }
        }
    });
```

- [ ] **Step 2：net 主循环把 Frame 路由到 frame_tx**

在 `connect_once` 主循环的 `act = from_ui.recv()` 分支（conn.rs:111-134），在 `Some(FromUi::RefreshPassword) => {...}` 之后、`Some(a) => handle_uplink(...)` 之前，插入 Frame 专路：

```rust
                    // 帧走单槽 watch：网络慢时陈旧帧被覆盖，只发最新（drop-stale 核心）。
                    Some(FromUi::Frame { session_id, data, w, h, seq }) => {
                        let env = Envelope {
                            from: id.clone(),
                            to: None,
                            ts: now(),
                            payload: Message::Frame { session_id, data, w, h, seq },
                        };
                        if let Ok(s) = serde_json::to_string(&env) {
                            let _ = frame_tx.send_replace(Some(s));
                        }
                    }
```

> 这样 `handle_uplink` 不再处理 Frame（其 Frame 臂变为不可达，但保留以维持穷尽匹配，无行为影响）。

- [ ] **Step 3：清理时显式 drop frame_tx**

`connect_once` 末尾清理（conn.rs:138-142）由：
```rust
    hb.abort();
    drop(out_tx);
    let _ = pump.await;
    result
```
改为：
```rust
    hb.abort();
    drop(out_tx);
    drop(frame_tx); // 关帧 lane，泵转入纯控制收尾后退出
    let _ = pump.await;
    result
```

- [ ] **Step 4：编译 + 客户端全测**

Run: `cargo test -p client`
Expected: 通过（conn 改动不破坏 dispatch/mod 既有测试）。

- [ ] **Step 5：提交**

```bash
git add src/client/src/net/conn.rs
git commit -m "feat(client): 被控帧 drop-stale(出站泵拆 control/frame 双lane)"
```

---

## Task 5（B）：server→主控帧 coalesce（附加式 frame_clients）

**Files:**
- Modify: `src/server/src/hub.rs`（frame_clients + send_frame_to + Frame 分支）
- Modify: `src/server/src/main.rs`（handle_socket 建 frame watch + 注册 + 泵 select 双 lane）

> **附加式**：不动现有 `clients: DashMap<id, UnboundedSender<String>>`（全部 hub 测试不改），新增独立 `frame_clients`，仅 Frame 路由走它。

- [ ] **Step 1：hub.rs 加 frame_clients + 方法**

`src/server/src/hub.rs`：`Hub` 结构（:17-23）加字段：

```rust
    /// 帧专用 lane（drop-stale）：endpoint_id/admin_id → 单槽最新帧 watch。与 clients 并存（附加式）。
    frame_clients: DashMap<String, tokio::sync::watch::Sender<Option<String>>>,
```

`Hub::new`（:26-33）初始化 `frame_clients: DashMap::new(),`。

加方法（紧邻 add_client/remove_client/send_to）：

```rust
    pub fn add_frame_client(&self, id: String, frame_tx: tokio::sync::watch::Sender<Option<String>>) {
        self.frame_clients.insert(id, frame_tx);
    }

    /// 帧定向推送（drop-stale）：覆盖目标的单槽最新帧，陈旧未发帧被丢弃。
    pub fn send_frame_to(&self, id: &str, json: &str) {
        if let Some(tx) = self.frame_clients.get(id) {
            let _ = tx.send_replace(Some(json.to_string()));
        }
    }
```

`remove_client`（:39-41）加一行同时移除帧 lane：

```rust
    pub fn remove_client(&self, id: &str) {
        self.clients.remove(id);
        self.frame_clients.remove(id);
    }
```

- [ ] **Step 2：hub.handle 把 Frame 拆出走 frame lane**

`hub.rs` 的合并 arm（:196-202）当前把 `Frame | RemoteNotice | SetQuality | SetCapture | ClipboardSync` 一起 route_to_peer。把 **Frame 单独拆出**走帧 lane：

```rust
            // Frame：走帧 lane（drop-stale），按 session 对端路由
            Message::Frame { session_id, .. } => {
                if let Some(peer) = self.sessions.peer_of(session_id, &env.from) {
                    if let Ok(json) = serde_json::to_string(&env) {
                        self.send_frame_to(&peer, &json);
                    }
                }
            }
            // 其余会话内控制消息：可靠 route_to_peer（control lane）
            Message::RemoteNotice { session_id, .. }
            | Message::SetQuality { session_id, .. }
            | Message::SetCapture { session_id, .. }
            | Message::ClipboardSync { session_id, .. } => {
                self.route_to_peer(session_id, &env);
            }
```

- [ ] **Step 3：hub 帧 lane 单测**

在 hub.rs `mod tests` 加（验证 Frame 走 frame lane、coalesce 留最新）：

```rust
    #[tokio::test]
    async fn frame_routed_to_frame_lane_latest_wins() {
        let hub = test_hub();
        let (a_tx, _a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, _b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);
        let (bf_tx, bf_rx) = tokio::sync::watch::channel::<Option<String>>(None);
        hub.add_frame_client("ep-b".into(), bf_tx);

        let sid = "sess-f".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(), mode: Mode::B, from_id: "ep-a".into(), to_id: "ep-b".into(),
            start_at: 100, end_at: None, status: SessionStatus::Active,
        });

        // ep-a 连发两帧（seq 0,1）→ frame lane 只保留最新
        for seq in 0u64..2 {
            let env = Envelope {
                from: "ep-a".into(), to: None, ts: 200,
                payload: Message::Frame { session_id: sid.clone(), data: format!("d{seq}"), w: 1, h: 1, seq },
            };
            hub.handle(env, 200).await;
        }
        let latest = bf_rx.borrow().clone().expect("帧 lane 应有最新帧");
        let env: Envelope = serde_json::from_str(&latest).unwrap();
        match env.payload {
            Message::Frame { seq, .. } => assert_eq!(seq, 1, "drop-stale：应保留最新 seq=1"),
            other => panic!("应为 Frame，实际 {other:?}"),
        }
    }
```

- [ ] **Step 4：main.rs handle_socket 双 lane 泵**

`src/server/src/main.rs` `handle_socket`（:171-246）：在建 `(tx, mut rx)`（:186）后加帧 watch，并把出站泵（:189-195）改为 select 双 lane；注册时同时 `add_frame_client`。

把 `:186-195` 段：
```rust
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let pump = tokio::spawn(async move {
        while let Some(s) = rx.recv().await {
            if sink.send(WsMsg::Text(s)).await.is_err() { break; }
        }
    });
```
改为：
```rust
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased; // 控制消息优先
                ctrl = rx.recv() => {
                    match ctrl {
                        Some(s) => { if sink.send(WsMsg::Text(s)).await.is_err() { break; } }
                        None => break,
                    }
                }
                changed = frame_rx.changed() => {
                    if changed.is_err() {
                        while let Some(s) = rx.recv().await {
                            if sink.send(WsMsg::Text(s)).await.is_err() { break; }
                        }
                        break;
                    }
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some(s) = latest {
                        if sink.send(WsMsg::Text(s)).await.is_err() { break; }
                    }
                }
            }
        }
    });
```

在登记连接 id 处（`hub.add_client(id.clone(), tx.clone());`，:223）紧随其后加：
```rust
            hub.add_frame_client(id.clone(), frame_tx.clone());
```

> `frame_tx` 是 watch::Sender（**不可 Clone**）。故不能 `frame_tx.clone()`。改法：`add_frame_client` 在首条消息登记时调用，把 `frame_tx` **move** 进去——但 frame_rx 已 move 进 pump，frame_tx 仍在 handle_socket 作用域。由于登记只发生一次（`if my_id.is_none()`），用 `Option<watch::Sender>` 包装并 `take()`：在 `:186` 处 `let mut frame_tx_opt = Some(frame_tx);`（frame_tx 已 move 进 pump？否）。**正确做法见 Step 5 修正**。

- [ ] **Step 5：修正 frame_tx 所有权（watch::Sender 不可 Clone）**

watch::Sender 不可 Clone 且 frame_rx 已 move 进 pump。让 pump 只拿 `frame_rx`，`frame_tx` 留在 handle_socket，注册时 move 进 hub：

- `:186` 后：`let (frame_tx, frame_rx) = tokio::sync::watch::channel::<Option<String>>(None);`（frame_rx 给 pump，frame_tx 留下）。pump 闭包 `move` 捕获 `frame_rx`（和 sink、rx）。
- 用 `let mut frame_tx = Some(frame_tx);` 便于在登记分支 `take()`。
- 登记处（`if my_id.is_none()` 块内，add_client 之后）：
  ```rust
              if let Some(ftx) = frame_tx.take() {
                  hub.add_frame_client(id.clone(), ftx);
              }
  ```

> 这样 frame_tx 在唯一登记点 move 进 hub.frame_clients；连接断开时 `remove_client` 移除该 sender → pump 的 `frame_rx.changed()` 返回 Err → 转纯控制收尾。

- [ ] **Step 6：server 全测 + 编译**

Run: `cargo test -p server`
Expected: 既有 hub 测试全过（clients 未动）+ 新增 frame lane 测试过。

- [ ] **Step 7：提交**

```bash
git add src/server/src/hub.rs src/server/src/main.rs
git commit -m "feat(server): 帧 lane coalesce(附加式 frame_clients, drop-stale)"
```

---

## 验收（全部 Task 完成后）

- [ ] `cargo test -p client -p server -p protocol` 全绿；`cargo build --workspace` 通过。
- [ ] **控制消息零丢失**（真机/集成）：exec / file push+pull / chat / set_quality / session_end 在高帧率下全部正常。
- [ ] **拖拽/绘图保真**：被控开画图，主控按住拖动 → 曲线（非直线）；滑块拖动平滑。
- [ ] **点击响应**：点击后画面更新明显快于改前。
- [ ] **mode B 双向**：同一 Slint 端既被控又主控时 frame_watch 与 out_tx 并存不串。
- [ ] 弱环境：Wayland 回执、截屏不可用回执、懒推流（SetCapture）正常。

---

## 实现顺序（依赖）

1. **Task 2**（LAST_INPUT_MS + mark_input_now + interval）→ 提供 Task 1/3 依赖的信号定义。
2. **Task 1**（drag-aware 合并 + 注入接入 mark_input_now）。
3. **Task 3**（事件驱动抓帧，用 last_input_after）。
4. **Task 4**（client 帧 drop-stale 泵）。
5. **Task 5**（server 帧 lane）。

> Task 1-3 全在 workers.rs/capture.rs（client 注入与抓帧侧），Task 4 在 conn.rs，Task 5 在 server。Task 4/5 是并发热路径核心，需最谨慎 + 双审。
