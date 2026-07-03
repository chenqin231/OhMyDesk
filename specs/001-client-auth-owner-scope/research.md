# Research: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope` | **Date**: 2026-07-03
**方法**: 3 个只读研究 agent 分别挖 server WS/hub/广播、server data/login/迁移、client 登录落点。以下每项决策均有 file:line 依据。

---

## 决策清单

### D1 — 客户端登录通道：复用 ureq 调 `/api/login`
- **Decision**: 客户端登录用**已存在的 `ureq`** POST `https://<host>/api/login`，body `{"user","pass"}`，读响应 `{"token","user","tier","permissions"}`。复用 `update.rs:281-297` 的 `build_agent()`（已带 Windows SChannel / 非 Win rustls 的 TLS 分流）。
- **Rationale**: `ureq` 已在 `src/client/Cargo.toml:47,57` 且自动更新在用；服务端 `/api/login` 是现成公开端点（`http.rs:970,993-1035`）。零新依赖。
- **Alternatives**: 引入 reqwest（多余，ureq 够用）；WS 首帧做鉴权（服务端已有独立 HTTP 端点，无必要）。
- **注意**: 请求字段是 `user`/`pass`（非 `username`/`password`）；响应无 user_id（客户端也不需要，owner 由服务端从 token 派生）。

### D2 — WS 携带 token：URL 追加 `?token=`（非 Authorization header）
- **Decision**: 被控端 WS 反连时把 JWT 拼进 URL 查询串 `wss://<host>/ws?token=<jwt>`。
- **Rationale**: 服务端 `ws_handler` 现读 `Query<WsQuery>{ token }`（`main.rs:170-188`）并 `auth.validate` —— admin 连接本就走这条。被控端复用同一校验，**零服务端握手改动**。
- **Alternatives**: `Authorization: Bearer` header（服务端读 query 不读 header，要另改握手，更贵）。
- **落点**: `conn.rs:33 connect_async(server_url)` 的 `server_url` 由 token 拼装；token 经 `net::run`(`mod.rs:213`)→`connect_once`(`conn.rs:25`) 传入（新参数）。

### D3 — 开通被控端鉴权：agent 也 validate + bind_actor
- **Decision**: (a) `ws_handler` 对带 token 的 agent 连接同样 `validate`（已天然如此）；(b) 放开 `main.rs:289` 仅 admin 才 `bind_actor` 的限制，对**带有效 token 的 agent 连接也 `bind_actor`**（存入 `actors` DashMap，含 `user_id`）。
- **Rationale**: `ActorIdentity.user_id`（`hub.rs:36`）即 owner 来源。旧端无 token → `auth_user=None` → 不 bind → owner=None，且 `main.rs:277` 的 gate 只拦 `admin-` 前缀，agent 照常上线（**旧端不回归，FR-008**）。
- **Alternatives**: 为 agent 造独立鉴权路径（重复代码，复用 admin 的 validate 更省）。

### D4 — owner 存储：endpoint_registry 加列，服务端注入（不进 EndpointInfo）
- **Decision**: `endpoint_registry` 加 `owner_id TEXT` 列（软外键 → `users.id`，TEXT UUID）。`Registry::Entry`(`registry.rs:16`) 加 `owner: Option<String>`。`upsert` 增 owner 参数，由 Register 臂 `actor_of(env.from).user_id` 注入；**绝不读 `EndpointInfo` 里的自报字段**。
- **Rationale**: 反伪造（FR-002-E1）。且 `upsert` 每次 register/heartbeat 会整体重写 `info` JSON（`registry.rs:76-97`），owner 若混在 JSON 里会被 agent 覆盖。列存独立、服务端权威。
- **迁移**: 沿用 `add_column_if_missing`（`db.rs:111-127`，PRAGMA 幂等）在 `ensure_identity_columns` 加一行 `ALTER TABLE endpoint_registry ADD COLUMN owner_id TEXT`；基础 DDL `scripts/db/schema.sqlite.sql` 同步加列。存量行回填 NULL。
- **持久化**: `db_save`(`registry.rs:155`) 增第 4 列 bind；`db_load_all`(`registry.rs:178`) 增 `SELECT owner_id`。重启后离线终端保留上次 owner（**符合"注销保留归属"语义**）。
- **换绑**: 新账号登录 = 新连接 + 新 token → 新 Register → owner 覆盖为新用户（后登录覆盖，FR-004）。

### D5 — 列表隔离：Registry 加 owner 感知视图 + push_list 逐 admin 过滤
- **Decision**: `Registry` 新增 `views_visible_to(owner_id: Option<&str>, is_superadmin: bool) -> Vec<EndpointView>`。`push_list`(`hub.rs:162`) 从「一份 json 群发」改为**逐 admin 连接循环**：对每个 `admin-` 连接 `self.actors.get(conn_id)` 取身份 → 过滤 → 单独序列化 + `send_to`(`hub.rs:137`)。superadmin 见全量（含 owner=None 旧端）。
- **Rationale**: 推送级隔离（FR-005-E2），非首屏。三处触发（Register `hub.rs:283`、Heartbeat `hub.rs:289`、admin 首帧 `main.rs:304`）统一走新路径。
- **Alternatives**: 前端过滤（推送即泄露全量，违反安全动机，否决）。
- **代价**: O(admin 数 × 终端数) 每连接定制序列化，内网规模可接受。

### D6 — 远控范围闸：加 target owner 交集
- **Decision**: `ConnectRequest` 闸（`hub.rs:293-331`）在现有 `use_remote` 布尔判定后，加 `reg.get_info(target).owner == actor.user_id || actor.is_superadmin`；不满足则拒绝并记审计（结果=被拒）。owner=None 的旧端仅 superadmin 可控。
- **Rationale**: FR-006。`self.reg` 在该作用域可达（`get_info` `registry.rs:131`），target 拿得到。

### D7 — 审计/会话过滤：走 session.to_id（**修正 spec 口径**）
- **Decision**: sessions 按 `to_id IN (SELECT id FROM endpoint_registry WHERE owner_id=?)` 过滤（`audit.rs:166`）。审计按 `session_id IN (SELECT id FROM sessions WHERE to_id IN (owned))` 过滤（`audit.rs:121`）。superadmin 全量。
- **修正原因**: `audit_logs.actor_id` 是**发起方**连接 id（操作员动作=`admin-xxx`），非目标终端。按 actor_id 过滤会漏掉所有管理员发起的远控审计。正确 owner 链路经 session 的目标终端。
- **已知取舍**: `ScreenshotReq` 审计首参是 `req_id` 非 session_id（`hub.rs:362`），这类行不 join → 普通用户看不到自己终端的截图审计（安全侧无泄露，功能侧轻微缺失，可接受）。

### D8 — 登录日志：无终端关联，需重定义语义（**待用户拍板**）
- **现状**: `login_log` 表只有 `username`（登录的管理员账号），**无任何终端关联列**（`login_log.rs:58`, schema）。无法按终端 owner 过滤。
- **方案 a（推荐）**: 重解释"隔离"为**只看自己的登录记录**（`WHERE username=<当前账号>` 过滤），superadmin 全量。语义合理——登录日志本就是账号维度。
- **方案 b**: 普通账号不显示登录日志页（superadmin-only）。
- **影响**: 需微调 spec FR-007 中"登录日志"一句的口径。

### D9 — 凭据持久化：新增 client `credential.rs` 仿 history.rs
- **Decision**: 新增 `src/client/src/credential.rs`，`Creds { token, user }` serde，存 `ohmydesk_state_dir()/credential.json`（敏感，放 state/data dir）；`load()/save()/clear()` 仿 `history.rs:31-61` 容错落盘。注销 = `clear()` 删文件。
- **Rationale**: FR-003 记住凭据 / FR-004 注销。`main.rs:15` mod 列表加 `mod credential;`。
- **注意**: 无现成原子写 helper（`history.rs:57` 是非原子 `fs::write`）；如需原子性自行「临时文件 + `fs::rename`」。Unix 建议 `0600`（现有代码未设文件权限，可作安全加固项）。

### D10 — 登录门时序：先读盘 token，无则登录页，登录后放行 net
- **Decision**: `main` 在 `AppWindow::new()` 后、`net::run` spawn 前读持久化 token：
  - 有 token → `logged_in=true`，`rt.spawn(net::run(..., token))` 照常。
  - 无 token → `logged_in=false`（Slint 门控显示登录页），`net::run` **暂不连**；`on_login` 回调后台 `ureq` 调 `/api/login` → 成功 `set_logged_in(true)` + `credential::save` + 启动/放行 net 连接。
- **Rationale**: token 必须先于 WS 连接就绪（D2）。`net::run` 现为启动即 spawn + 永不退出重连死循环（`mod.rs:222-230`），需改为「等 token 就绪信号再进重连循环」。
- **UI 落点**: `logged_in` 布尔（`app.slint:493` 旁）门控 `app.slint:609/901` 两主界面分支 + 新增 `if !logged_in` 全屏登录页（复用 `app.slint:1387` 模态 Card + `FieldInput`(`:183`)）；`on_login/on_logout` 回调仿 `refresh_password`(`ui_glue.rs:359`) + 后台 IO 仿 `list_local`(`ui_glue.rs:450`)。

---

## 影响 spec 的两处修正（需用户确认）

| # | 修正 | spec 原文 | 建议 |
|---|------|----------|------|
| C1 | 审计过滤走 session.to_id，截图审计行普通用户不可见 | FR-007-H1「操作对象为 A 名下终端」 | 接受（口径澄清，不改验收目标） |
| C2 | 登录日志无终端关联，重解释为「只看自己的登录记录」 | FR-007「登录日志…只含 A 名下终端相关」 | 采方案 a（username 过滤） |

## 未决 / 已消除的 NEEDS CLARIFICATION
- ✅ 客户端登录通道（D1 ureq）
- ✅ WS token 传输（D2 query 串）
- ✅ owner 存储位置（D4 独立列）
- ✅ 广播隔离机制（D5 逐 admin 过滤）
- ✅ 协议兼容写法（D4：owner 走列不走 EndpointInfo；若日后加 EndpointView.owner 用 `Option<T>` 兼容，同 `department`/`gpu` 先例）
- ⏳ C2 登录日志语义（待确认）
