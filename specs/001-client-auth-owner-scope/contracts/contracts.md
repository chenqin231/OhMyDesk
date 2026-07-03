# Contracts: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope` | **Date**: 2026-07-03

> 本功能不新增 REST 端点，契约集中在：① 复用的登录 HTTP、② WS 鉴权约定、③ 服务端数据范围过滤不变量。以下均为**行为契约**，实现须满足。

## C1 — 登录 HTTP（复用现有 `/api/login`，客户端新调用方）

```
POST /api/login          （公开，无需鉴权）
Content-Type: application/json
Request:  { "user": "<账号>", "pass": "<密码>" }
Response 200: { "token": "<JWT>", "user": "<username>", "tier": "superadmin|user", "permissions": ["..."] }
Response 401: { "error": "账号或密码错误" }
```
- 字段名是 `user`/`pass`（非 username/password）——客户端须对齐。
- 客户端只需 `token`；owner 由服务端从 token 派生，客户端不发送归属。
- 禁用账号：`verify_login` 返回 None → 401（客户端 inline「账号已被禁用…」需服务端能区分——若现有仅返回统一 401，客户端按「账号或密码错误」兜底，禁用细分为可选增强）。

## C2 — WS 鉴权约定（被控端连接携带 token）

```
连接: wss://<host>/ws?token=<JWT>
```
- 服务端 `ws_handler` 现有 `Query<WsQuery>{ token }` 校验路径**不改**（`main.rs:170`）。
- **不变量 A（旧端兼容）**：无 `token` 的连接照常升级（`auth_user=None`），只要 `from` 不以 `admin-` 前缀即不被 `main.rs:277` gate 拦截 → 旧端正常上线，owner=NULL。
- **不变量 B（owner 派生）**：带**有效** token 的连接，服务端为其 `bind_actor`（含 `user_id`）；agent 连接的 `bind_actor` 限制从「仅 admin-」放开到「凡带有效 token」。
- **不变量 C（token 无效）**：带 token 但校验失败 → close 1008（现有行为，不变）。客户端据此回登录页提示「登录已过期」。

## C3 — Register 报文（wire 不变，语义收紧）

```
Message::Register { info: EndpointInfo, password: String }   // wire 结构不变
```
- **不变量 D（反伪造）**：服务端处理 Register 时，owner **只取** `actor_of(env.from).user_id`，忽略 `info` 内任何自报归属。即使客户端在 `EndpointInfo` 里注入伪造 owner，也不生效（对应 SC-003=100%）。

## C4 — 终端列表推送过滤（`push_list` 逐连接）

**契约**：对每个 `admin-*` 连接，推送的 `EndpointList` 满足：
```
若 actor.is_superadmin → 全量（含 owner=NULL）
否则               → { ep | ep.owner_id == actor.user_id }
```
- **不变量 E（推送级隔离）**：过滤发生在服务端每次推送前（Register/Heartbeat/admin 首帧三触发点），前端收到即已过滤，任何时刻不含他人终端（SC-001、AC-005-E2）。

## C5 — 远控范围闸（`ConnectRequest`）

**契约**：现有 `use_remote` 布尔判定通过后，追加：
```
allow = actor.is_superadmin
     || (reg.get_info(target).owner_id == actor.user_id)
拒绝 → 不建会话 + 审计记录（结果=被拒）
```
- **不变量 F**：owner=NULL 的终端仅 superadmin 可远控（SC-002=100% 阻断）。

## C6 — 审计 / 会话 / 登录日志过滤

| 数据 | 普通账号过滤 | superadmin |
|---|---|---|
| sessions | `WHERE to_id IN (SELECT id FROM endpoint_registry WHERE owner_id=?)` | 全量 |
| audit_logs | `WHERE session_id IN (SELECT id FROM sessions WHERE to_id IN (owned))` | 全量 |
| login_log | `WHERE username = <当前账号>` | 全量 |

- **不变量 G**：三类查询的 `?` 绑定当前请求 `AuthUser.id`/`username`；superadmin 走无过滤分支。
- 已知取舍：截图类审计（`actor_id`/`req_id`，无 session_id）不 join → 普通账号不呈现（C1 修正，无泄露）。

## 契约测试要点（映射 SC）
- C3-D → SC-003：伪造 owner 的 Register，落库 owner_id 仍为 token.sub。
- C4-E → SC-001：普通账号 WS 推送的列表不含他人 owner 终端。
- C5-F → SC-002：普通账号 ConnectRequest 打他人终端，返回拒绝 + 审计。
- C2-A → SC-004：无 token 旧端 Register 成功上线，owner=NULL。
- C6-G → SC-001：审计/会话/登录日志查询结果按上表过滤。
