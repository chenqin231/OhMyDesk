# Spec D 梯队1：远程延迟根治（帧 drop-stale + 拖拽安全的输入合并 + 事件驱动抓帧）

> 日期：2026-06-30
> 范围：功能① 第一梯队（基于纠偏后的证据，已过架构师 + design-patterns 审视）
> 状态：已确认设计，待转实现计划

---

## 1. 背景与根因（经代码证实并订正）

远控"点击后画面慢"是串行流水线累积延迟。**上轮"输入与视频共用同一队列导致队头阻塞"的根因经代码证伪**——按收发方向：被控端 `out_tx`（conn.rs:38）只走帧（被控只收 input 本地注入、不回发）；主控端 `out_tx` 只走 input（只收帧、不发帧）；server→被控队列走 input 无帧；server→主控队列走帧无 input。**输入不被帧阻塞。**

**真正根因（实证 workers.rs:167 + main.rs:186）**：帧生产侧无 drop-stale。被控抓帧线程每 62/100ms 把帧塞进 `from_ui_tx`→`out_tx`→server 转发队列，全部 unbounded、无丢旧。点击后那张"显示结果"的新帧排在若干陈旧帧之后才发出——主控消费侧虽有丢帧追新（ui_glue），生产侧/server 侧不丢，延迟已在上游累积。叠加固定轮询抓帧间隔与软件整屏编码，构成可感延迟。

---

## 2. 方案总览与关键决策（已过架构审视）

四项改动，全在 client + server，**不动 admin-web、不改协议（帧仍 JSON）**：

| 项 | 改动 | 价值 |
|----|------|------|
| **A** | 被控端帧 drop-stale：`conn.rs` 出站泵拆 `control_tx`（可靠 FIFO）+ `frame_watch`（单槽最新帧）双 lane | 核心：消灭"新帧排陈旧帧后" |
| **B** | server→主控帧 coalesce：`hub.rs`/`main.rs` 每客户端出站同构拆双 lane | 保护 admin 走慢链路场景（内网 ROI 较低，据实标注） |
| **C** | **drag-aware** 注入侧 move 合并：`workers.rs` 注入线程跟踪按键态，悬停合并、拖拽不合并 | 降低光标滞后，且不破坏拖拽/绘图 |
| **D** | 抓帧间隔 62→40ms + 事件驱动：`AtomicU64 LAST_INPUT_MS` 桥接注入↔抓帧，输入后即时抓帧、coalesce ≤1 次/16ms | 点击后近即时出新帧，空闲不浪费 CPU |

### 硬约束（架构审视产出，spec 级不变量）
1. **仅 `Frame` 可合并丢弃**；所有控制消息（`auth_result`/`connect_ack`/`session_end`/`reject`/`input`/`exec_*`/`file_*`/`chat`/`heartbeat`/`register` 等）**必须走可靠 FIFO，零丢失**。lane 分类只认 Frame。
2. **C 必须 drag-aware**：`buttons_down>0`（拖拽中）注入**所有** move 不合并；`buttons_down==0`（悬停）才合并到最新；任何 button/key 前先 flush 待发 move（保证点击前光标到位）。**不做主控发送侧节流**（会同样误伤拖拽）。
3. **不引入跨 crate 强抽象**：A（client）与 B（server）用同一"control_tx + frame_watch + select 泵"模式，**镜像实现**即可（类型不同，DRY 限于心智模型）。
4. **D 的 `LAST_INPUT_MS` 是唯一新增跨线程耦合点**，仅传时间戳信号（非业务态），代码注释标注。

### 砍掉/推迟
- **砍掉**：输入优先级队列拆分（已证伪，给非问题加复杂度）。
- **推迟到梯队2**：二进制帧（纯带宽优化、且触及 admin-web 解码，风险/面更大）；脏区检测；硬件/帧间编码。

---

## 3. 组件设计

### 3.1 A — 被控端帧 drop-stale（`src/client/src/net/conn.rs`）

- `connect_once` 新增每连接 `let (frame_tx, frame_rx) = tokio::sync::watch::channel::<Option<String>>(None);`（持已序列化的帧 JSON）。
- **出站泵改造**（原 conn.rs:38-45 单 `while out_rx.recv()`）：
  ```
  loop tokio::select! {
    Some(text) = out_rx.recv()  => write control（可靠）
    Ok(()) = frame_rx.changed() => 取 frame_rx.borrow() 最新帧并 write（陈旧已被覆盖）
    else => break
  }
  ```
- **net 主循环**收到 `FromUi::Frame`：不再 `out_tx.send(json)`，改 `frame_tx.send_replace(Some(json))`（覆盖旧帧）。其余 `FromUi::*` 仍走 `out_tx`（control lane）。
- 帧序列化位置：维持现状在 net 循环序列化后投 frame_tx（保持 handle_uplink 结构最小改动）。
- 会话结束迟到帧：主控现有 `frame_belongs_to_ended`（ui_glue）已丢弃，无需额外处理。

### 3.2 B — server→主控帧 coalesce（`src/server/src/hub.rs` + `main.rs`）

- 每客户端出站从 `UnboundedSender<String>` 改为结构 `ClientTx { control: UnboundedSender<String>, frame: watch::Sender<Option<String>> }`（存 `Hub.clients`）。
- `send_to`（控制消息）走 `control`；hub 的 **Frame 路由分支**（route_to_peer Frame 时）走 `frame.send_replace`。
- `main.rs` 每连接泵 select `control_rx.recv()` + `frame_rx.changed()`，同 A。
- 其余路由（endpoint_list/ack/reject 等）一律 control lane。

### 3.3 C — drag-aware 注入侧 move 合并（`src/client/src/workers.rs` consume_inject 注入线程）

- 注入线程（workers.rs:31-48 的 `while blk_rx.recv()`）改为**批量抽干 + drag-aware 合并**：
  - 维护 `buttons_down: i32`（MouseButton down +1 / up -1，下限 0）。
  - 每轮 `recv()` 一个事件后 `try_recv()` 抽干当前积压，得到一批事件，按序处理：
    - `MouseMove`：若 `buttons_down>0` → 立即 `apply`（拖拽全保真）；否则暂存 `pending_move`（覆盖，悬停合并）。
    - `MouseButton`/`Key`/`Text`：先 flush `pending_move`（若有）再 `apply` 本事件；`MouseButton` 更新 `buttons_down`。
  - 批末 flush `pending_move`。
- 不改主控发送侧（admin-web 与 Slint 主控均原样全量发，被控侧统一收敛）。

### 3.4 D — 抓帧间隔 + 事件驱动（`src/client/src/workers.rs` consume_capture + `inject.rs` + `capture.rs`）

- `capture.rs`：流畅档 `interval_ms` 62→40（25fps）；高清档维持或微调（实现时定，不在本 spec 钉死具体值，但记录变更）。
- 新增模块级 `static LAST_INPUT_MS: AtomicU64`（放 workers.rs 或 inject.rs，单一定义）。注入线程每次 `apply` 一个**button/key**（及拖拽 move）后写入当前 ms。
- 抓帧线程（workers.rs:109 循环）：改为小粒度（~16ms）轮询，每轮判断「距上次抓帧 ≥ interval_ms」**或**「`LAST_INPUT_MS` > 上次抓帧时刻」→ 抓一帧；否则继续轻睡。**coalesce**：单轮最多抓一帧，input 洪泛无法把抓帧/编码频率拉过 ~60fps 上限（防 CPU DoS）。
- 与 A/B 协同：抓帧率提高时若 CPU/网络跟不上，drop-stale 只丢帧不堆积，延迟不累积。

---

## 4. 安全与边界

- **无新增外部面**：不加端点、不改协议、攻击面不变。
- **输入注入无新能力**：合并只丢/不新增事件；drag-aware 保留拖拽语义；授权会话内才有 input。
- **输入洪泛 DoS**：D 抓帧 coalesce ≤1 次/16ms，恶意主控狂发 input 无法拉爆被控抓帧/编码 CPU（有界）。
- **控制消息零丢失**：lane 分类只认 Frame；A/B 的 watch 仅用于 Frame。回归必须验证 input/exec/file/chat/session_end 在高帧率下不丢不乱序。
- **依赖方向**：coalescing 属传输层关注点，留在 conn.rs 泵 / hub-main 泵；capture/inject 不感知传输合并，**唯一例外**是 D 的 `LAST_INPUT_MS` 跨线程信号（已标注）。

---

## 5. 回归清单（关键，合并前必过）

- [ ] 远控基本链路：admin-web 与 Slint 主控分别连一台被控，画面正常刷新、断开正常。
- [ ] **控制消息零丢失**：会话内执行命令（exec）、下发/取回文件（file_*）、即时消息（chat）、切画质（set_quality）、结束会话（session_end）全部正常——验证帧 lane 不吞控制消息。
- [ ] **拖拽/绘图保真**：被控端打开画图工具，主控按住拖动画一条曲线——应为曲线非直线（drag-aware 生效）；滑块拖动平滑。
- [ ] **点击响应**：点击后画面更新明显比改前快（主观 + 若可埋点量化）。
- [ ] **mode B 双向**：同一 Slint 端既被控又主控（如有该用例）时，frame_watch 与 control_tx 并存不串。
- [ ] 弱环境：Wayland 回执、截屏不可用回执、懒推流（SetCapture 暂停/恢复）仍正常。
- [ ] `cargo test -p client -p server -p protocol` 全绿；`cargo build` workspace 通过。

---

## 6. 涉及文件清单

**修改**：
- `src/client/src/net/conn.rs`（出站泵双 lane + frame_watch）
- `src/client/src/net/mod.rs`（如需调整 FromUi 路由/wiring）
- `src/client/src/workers.rs`（consume_inject drag-aware 合并；consume_capture 事件驱动抓帧 + LAST_INPUT_MS）
- `src/client/src/inject.rs`（apply 后写 LAST_INPUT_MS；或在 workers 注入线程写）
- `src/client/src/capture.rs`（interval_ms 调整）
- `src/server/src/hub.rs`（ClientTx 双 lane + Frame 分支走 frame lane）
- `src/server/src/main.rs`（每连接泵 select 双 lane）

**不改**：`src/protocol/*`（帧格式不变）、`src/admin-web/*`（被控侧统一收敛，主控不动）。
