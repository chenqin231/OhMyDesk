# Bug：被控端主动发聊天，主控（尤其 WEB）收不到

> **状态**：根因已定位（系统化调试 Phase 1 完成）；按用户选定 **A 节奏**分两步修复
> **关联**：`remote-tools-feature-plan`（远程命令/文件/消息功能）

---

## 症状

WEB 远控一台客户端时，被控端**主动**发聊天消息，WEB 主控端收不到；即使双方来回发过几次仍如此。期望：主控/被控无论谁先发，对端都能收到。

## 现场证据（用户确认的 3 个观测）

1. 被控点「发送」后，被控自己的聊天面板**有本地回显**（「我: …」）。
2. 反方向 **WEB→被控 正常**（被控能收到主控的消息）。
3. WEB 通过**管理后台强制远程**（mode A force = auto_accept）连接。

加上：**远程画面（帧）正常**。

## 根因

**被控端把「当前被控会话 id」存了两份，维护条件不一致，会漂移：**

| 事实源 | 谁在用 | set 点 | clear 点 |
|---|---|---|---|
| `SessionCtx.controlled`（权威） | 帧 capture、注入、收消息门控 | `dispatch.rs` IncomingControl/AuthResult | SessionEnd **按 session_id 门控**（`dispatch.rs:240`，只清匹配的） |
| `ctrl_session`（UI 侧副本，`SharedSession`） | **只有被控聊天发送**（`ui_glue.rs:507` `send_controlled_chat`） | `ui_glue.rs` ControlRequest/BeingControlled | SessionEnd **无条件清**（`ui_glue.rs:694`） |

被控聊天发送（`send_controlled_chat`）读 `ctrl_session` 取 session_id 上行（`FromUi::SendChat`→`ChatMessage{session_id}`）。一旦 `ctrl_session` 与权威会话漂移（多会话 / 重控 / 迟到 SessionEnd / `SetCapture` 只更新 capture 不碰 ctrl_session），被控就带着**失效的 session_id** 上行 → 服务端 `route_to_peer`→`peer_of(失效id, 被控)` 返回 `None` → **静默丢弃**（修复前）。

**证据闭合**：
- 本地回显有 = 消息确实发出（走了 `out_tx`，`handle_uplink` 已 send）；
- WEB 不显示 = 服务端丢了（admin-web `store.ts:275` 收 `chat_message` 是**无条件 `appendChat`**，到了必显示；`msg-N` 与 `c-…` 前缀不可能去重撞）；
- 帧正常 = 帧走**权威 capture 会话**，没漂移；
- WEB→被控 正常 = 被控**收**消息按 `cur_session` 角色判定（`ui_glue.rs:828`），**根本不读 `ctrl_session`**。

**一处确凿的不对称**（坐证「两份会漂移」真实存在）：SessionEnd 清理时 `dispatch.rs:240` 清 `controlled` **按 session_id 门控**，而 `ui_glue.rs:694` 清 `ctrl_session` **无条件**；set 侧 `SetCapture` 更新 capture 会话也不碰 `ctrl_session`。多会话/迟到 SessionEnd/懒推流切换下两者必然不同步。

## client→client 是否也有（用户第二问）

**同样受影响，不是 WEB 专属。** 被控发送路径 `send_controlled_chat→ctrl_session` 对「客户端控制」与「WEB 控制」**完全相同**；是否触发只取决于控制序列（重控/多会话），与控制端类型无关。WEB 易复现是因为「强制远程 + 反复测试」更易制造漂移序列。

## 修复（按用户选定 A：先坐实，再根治）

### 第一步（已做，随下次部署上线）
服务端 `route_to_peer` 路由失败由**静默丢弃改 `warn!`**（`hub.rs`）：查无对端 / 对端离线都记 `session=… from=…`。这既是该修的缺陷（路由失败不该静默吞），也用于**坐实确切漂移触发序列**——下次部署后发一次被控聊天，日志出现 `route_to_peer 丢弃: 查无对端 session=<失效id> from=<被控>` 即实锤。

### 第二步（坐实后做，带回归测试）
**消除第二事实源**：被控聊天发送统一用权威被控会话 id。两条可选，确认触发序列后定：
- 让 `ctrl_session` 的 set/clear 与 `SessionCtx.controlled` **逐点对齐**（最小改动：SessionEnd 在 ui_glue 侧也按 session_id 门控；补 SetCapture/重控路径的同步）；或
- 被控聊天不再依赖 UI 侧 `ctrl_session`，改由权威 `controlled` 派生 session_id（去掉副本，单一事实源，更彻底）。

**回归测试**：
- 服务端：`route_to_peer` 对「被控(to_id)→主控(from_id=admin-*)」方向的 ChatMessage 正确投递（补 hub 测试当前缺的 to_id→from_id + admin 对端方向）。
- 客户端：构造「会话漂移序列」（控制 S1 → 重控 S2 / 迟到 SessionEnd{S1}）后，被控发聊天所用 session_id 必须等于权威 `controlled`。

## 非目标

- 不改聊天消息格式/协议。
- 不动 admin-web 收侧（已无条件追加，正确）。
