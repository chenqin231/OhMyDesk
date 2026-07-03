---
description: "Task list for 客户端账号认证与按用户数据隔离"
---

# Tasks: 客户端账号认证与按用户数据隔离

**Input**: `/specs/001-client-auth-owner-scope/`（spec.md / plan.md / research.md / data-model.md / contracts/ / quickstart.md）
**Tests**: 已请求 TDD —— 每个 User Story 内先写失败测试（RED）再实现。
**Organization**: 按 User Story 分组，可独立实现与验证。

## Format: `[ID] [P?] [Story] Description`
- **[P]**: 可并行（不同文件、无未完成依赖）
- **[Story]**: US1/US2/US3；Setup/Foundational/Polish 无 Story 标签

## User Story 映射（FR → US）
- **US1（P1）客户端登录与归属绑定** — FR-001 / FR-002 / FR-003 / FR-004 🎯 MVP
- **US2（P1）按 owner 数据隔离与范围管控** — FR-005 / FR-006 / FR-007
- **US3（P2）旧端兼容不回归** — FR-008

## 关键共享文件（串行约束）
- `src/server/src/hub.rs` — T009(US1) / T019,T020(US2) / T027(US3) 共享 → **串行**
- `src/server/src/main.rs` — T008,T013(US1) / T027(US3) 共享 → **串行**
- `src/client/ui/app.slint` — T014 独占

---

## Phase 1: Setup

- [ ] T001 建立 worktree 并记录基线：`cargo build --workspace` + `cargo test -p server` 现状全绿存档（crate 名已定板：`server`/`client`/`protocol`）；确认 `src/client/Cargo.toml` 的 `ureq`/TLS 依赖就位（无需新增）
  - **独占文件**: 无（只读校验 + worktree）

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ 阻塞**：owner 数据地基未就绪前，US1/US2 均不可开工。

- [ ] T002 [P] 【RED】Registry owner 单测：在 `src/server/src/registry.rs` 测试模块加 `upsert(info,pw,now,Some("A"))` → `views_visible_to(Some("A"),false)` 含该 ep、`views_visible_to(Some("B"),false)` 不含、`views_visible_to(None,true)` 含全部、owner=None 项仅 `is_superadmin=true` 可见；落库回灌保留 owner_id。**先失败**（owner 字段/方法未实现）
  - **独占文件**: `src/server/src/registry.rs`（测试模块）
- [ ] T003 [P] 协议：`EndpointView` 加 `owner_id: Option<String>`（`#[ts(export)]`，用 `Option` 兼容旧端 serde）
  - **独占文件**: `src/protocol/src/lib.rs`
- [ ] T004 迁移：`endpoint_registry` 基础 DDL 加 `owner_id TEXT`（`scripts/db/schema.sqlite.sql`）+ `ensure_identity_columns` 加 `add_column_if_missing(pool,"endpoint_registry","owner_id","ALTER TABLE endpoint_registry ADD COLUMN owner_id TEXT")`（`src/server/src/db.rs`）
  - **共享文件**: `db.rs`（与 M5 无交集，独立）；`schema.sqlite.sql`
- [ ] T005 Registry 实现（转绿 T002）：`Entry.owner: Option<String>` + `upsert` 增 owner 参 + `db_save`/`db_load_all` 带 `owner_id` 列 + 新增 `views_visible_to(owner_id: Option<&str>, is_superadmin: bool) -> Vec<EndpointView>`（依赖 T003/T004）
  - **独占文件**: `src/server/src/registry.rs`

**Checkpoint**: owner 地基就绪，T002 转绿，US1/US2 可开工。

---

## Phase 3: User Story 1 - 客户端登录与归属绑定 (P1) 🎯 MVP

**Goal**: 被控端用 WEB 账号登录并以「当前账号=负责人」上线；记住凭据自动上线；注销/换绑。

**Independent Test**: 客户端用账号 A 登录 → 服务端 `endpoint_registry.owner_id == A.id`；错密码显示 inline 错误；重启自动上线；注销清凭据并回登录页。

### Tests for US1（RED first）
- [ ] T006 [P] [US1] 【RED】服务端集成测试：带 token=A 的连接发 Register → 落库 `owner_id==A.id`；同一 Register 的 `info` 夹带伪造 `owner=B` → 落库仍 `A.id`（反伪造）
  - **独占文件**: `src/server/src/hub.rs`（测试模块）
  - **覆盖 AC**: AC-002-H1, AC-002-E1
- [ ] T007 [P] [US1] 【RED】客户端单测：`credential::{save,load,clear}` 往返——save 后 load 得同 token；clear 后 load 得 None；文件损坏时 load 返回 None 不 panic
  - **独占文件**: `src/client/src/credential.rs`（测试模块）
  - **覆盖 AC**: AC-003-H1, AC-004-H1

### Implementation for US1
- [ ] T008 [US1] 服务端：`ws_handler` 对 agent 连接也 `validate` token；放开 `main.rs:289` 仅 `admin-` 才 `bind_actor` 的限制 → 带有效 token 的 agent 也 `bind_actor`（存 `user_id`）；无 token 连接不受影响（`main.rs:277` gate 只拦 `admin-`）
  - **共享文件**: `src/server/src/main.rs`（与 T027 共享 → 串行）
  - **覆盖 AC**: AC-002-H1, AC-002-E2, AC-008-H1
  - **完成标准**:
    - 正常：token=A 的 agent 连上 → `actors[conn]` 存在且 `user_id==A.id`
    - 异常：token 无效 → close 1008（现有行为不变）；无 token → 不 bind、照常上线
- [ ] T009 [US1] 服务端：Register 臂 `let owner = self.actor_of(&env.from).map(|a| a.user_id)` 注入 `reg.upsert(info, pw, now, owner)`；**忽略 `info` 内任何自报归属**（转绿 T006）
  - **共享文件**: `src/server/src/hub.rs`（与 T019/T020/T027 共享 → 串行，先做）
  - **覆盖 AC**: AC-002-H1, AC-002-E1
  - **完成标准**:
    - 正常：token=A 的 Register → owner_id=A.id
    - 异常（反伪造）：info 含伪造 owner=B → 落库仍 A.id
- [ ] T010 [P] [US1] 客户端：新建 `src/client/src/credential.rs`——`Creds{token,user}` serde + `load/save/clear`（仿 `history.rs`，落 `ohmydesk_state_dir()/credential.json`，容错落盘）（转绿 T007）
  - **独占文件**: `src/client/src/credential.rs`
  - **覆盖 AC**: AC-003-H1, AC-004-H1
  - **完成标准**:
    - 正常：save → load 往返一致；clear 删文件
    - 异常：文件缺失/损坏 → load 返回 None，不 panic
- [ ] T011 [P] [US1] 客户端：新建 `src/client/src/login.rs`——`login(server,user,pass) -> Result<Creds,LoginErr>`，复用 `update::build_agent` 的 ureq，POST `/api/login` body `{user,pass}`，解析 `{token,user,...}`；区分 401/网络错/禁用
  - **独占文件**: `src/client/src/login.rs`
  - **覆盖 AC**: AC-001-H1, AC-001-E1, AC-001-E2, AC-001-E3
  - **完成标准**:
    - 正常：有效账号密码 → 返回 `Creds{token}`
    - 异常1：401 → `LoginErr::BadCredential`（UI 映射「账号或密码错误」）
    - 异常2：连接失败 → `LoginErr::Network`（UI 映射「无法连接服务器，请检查网络后重试」）
- [ ] T012 [US1] 客户端：`net::run`/`connect_once` 加 `token` 参数；WS URL 拼 `?token=<jwt>`；重连循环改为「等 token 就绪信号再进循环」
  - **共享文件**: `src/client/src/net/mod.rs`, `src/client/src/net/conn.rs`
  - **覆盖 AC**: AC-001-H1, AC-002-E2
  - **完成标准**:
    - 正常：带 token → WS 以 `?token=` 建连、发 Register
    - 异常：无 token → 不进重连循环（等登录）；token 失效被 close → 通知 UI 回登录页
- [ ] T013 [US1] 客户端：`main.rs` 启动读盘 token（`credential::load`）→ 有效则 `logged_in=true` 并放行 net；无则 `logged_in=false`；`mod credential; mod login;` 声明
  - **共享文件**: `src/client/src/main.rs`
  - **覆盖 AC**: AC-003-H1, AC-003-E1, AC-003-E2
  - **完成标准**:
    - 正常：有效凭据 → 跳过登录页自动上线
    - 异常：凭据过期/账号不可用 → 回登录页 + inline「登录已过期，请重新登录」/「账号不可用，请重新登录」
> **F4 拆分**：T014/T014b/T014c 均改 `app.slint`（同文件 → 顺序子提交，逐屏落地降单次 diff 风险）。
- [ ] T014 [US1] 客户端 UI-S1：`app.slint` 加 `logged_in` 布尔门控现有主界面分支（`:609/:901` 前置 `logged_in &&`）+ 新增 `if !logged_in` 全屏 S1 登录页（复用 Card/FieldInput/PrimaryButton；账号框 / 密码框 password / 服务器地址折叠「高级」/ 登录按钮 / inline 错误区）
  - **独占文件**: `src/client/ui/app.slint`（子提交 1）
  - **覆盖 AC**: AC-001-H1, AC-001-E1, AC-001-E4
  - **完成标准**:
    - 正常：未登录显示 S1；登录成功后 `logged_in=true` 切回主界面
    - 异常：空账号/密码 → 空框下 inline「请输入账号」/「请输入密码」，焦点定位首个空框；错密码 → inline「账号或密码错误」
- [ ] T014b [US1] 客户端 UI-S2：`app.slint` 主界面顶栏加账号条「已登录:<user> · ●在线 · [注销]」（在线/离线状态点变色）
  - **共享文件**: `src/client/ui/app.slint`（子提交 2，串行于 T014）
  - **覆盖 AC**: AC-004-H1（顶栏注销入口）
  - **完成标准**:
    - 正常：已登录态顶栏显示账号名 + 在线状态点 + 注销按钮
- [ ] T014c [US1] 客户端 UI-S3：`app.slint` 加 S3 注销确认 Modal（复用 `:1387` 遮罩+Card 范式，文案「注销后本机将下线,确定?」+「取消」/「确定」双按钮，确定高亮）
  - **共享文件**: `src/client/ui/app.slint`（子提交 3，串行于 T014b）
  - **覆盖 AC**: AC-004-H1, AC-004-E1
  - **完成标准**:
    - 正常：点注销 → 弹 S3 Modal
    - 异常：点取消 → Modal 关闭，`logged_in` 与在线态不变
- [ ] T015 [US1] 客户端：`ui_glue.rs` 加 `on_login`（后台线程 ureq 调 `login::login` + `invoke_from_event_loop` 回填，成功 `set_logged_in(true)`+`credential::save`+放行 net，失败 `set_login_error`）+ `on_logout`（S3 确定 → 断 WS + `credential::clear` + `set_logged_in(false)`）
  - **独占文件**: `src/client/src/ui_glue.rs`
  - **覆盖 AC**: AC-001-H1, AC-001-E1, AC-001-E2, AC-001-E3, AC-004-H1, AC-004-H2
  - **完成标准**:
    - 正常：登录成功进在线态并存盘；注销确定断连清盘回登录页
    - 异常：登录失败按错误类型 set 对应 inline 文案，登录按钮 loading 恢复

**Checkpoint**: US1 可独立验证 —— 客户端登录/归属绑定/记住凭据/注销换绑闭环（服务端 owner 落库正确）。

---

## Phase 4: User Story 2 - 按 owner 数据隔离与范围管控 (P1)

**Goal**: 普通账号只看到/只能远控/只能审计自己负责的终端；superadmin 全量。

**Independent Test**: 直接给两台终端置 owner=A/B（或经 US1 登录），以 A 登录管理端：列表/监控/审计/会话/登录日志均不含 B；A 远控 B 被拒。

### Tests for US2（RED first）
- [ ] T016 [P] [US2] 【RED】服务端集成测试：`push_list` 给 admin(A) 的 EndpointList 仅含 owner==A.id；superadmin 收全量（含 owner=NULL）；B 终端上线触发 push 后 A 通道消息仍不含 B
  - **独占文件**: `src/server/src/hub.rs`（测试模块）
  - **覆盖 AC**: AC-005-H1, AC-005-H2, AC-005-E2
- [ ] T017 [P] [US2] 【RED】服务端集成测试：`ConnectRequest` actor=A、target.owner=B → 拒绝（不建会话）+ 审计 result=被拒；target.owner=A → 放行；superadmin → 放行；target.owner=NULL 仅 superadmin 放行
  - **独占文件**: `src/server/src/hub.rs`（测试模块）
  - **覆盖 AC**: AC-006-H1, AC-006-E1, AC-006-E2
- [ ] T018 [P] [US2] 【RED】服务端集成测试：`query_audit(A)` 仅返回 `session_id∈(sessions.to_id∈owned(A))`（含 B 数=0）；`query_sessions(A)` 仅 `to_id∈owned(A)`；`login_log.query(username=A)` 仅 A 的行；superadmin 全量
  - **独占文件**: `src/server/src/audit.rs`, `src/server/src/login_log.rs`（测试模块）
  - **覆盖 AC**: AC-007-H1, AC-007-H2, AC-007-H3
- [ ] T019 [US2] 服务端：`push_list` 从 `broadcast_admins` 群发改逐 admin 循环——每个 `admin-` 连接取 `actors[conn]` → `reg.views_visible_to(user_id, is_superadmin)` → 单独序列化 `send_to`；三触发点（Register/Heartbeat/admin 首帧）统一走新路径（转绿 T016）
  - **共享文件**: `src/server/src/hub.rs`（串行：T009 后）
  - **覆盖 AC**: AC-005-H1, AC-005-H2, AC-005-E2
  - **完成标准**:
    - 正常：普通 admin 收自己 owner 的列表；superadmin 收全量
    - 异常（推送级）：他人终端上线，普通 admin 通道任何时刻不含
- [ ] T020 [US2] 服务端：`ConnectRequest` 闸在 `use_remote` 布尔后加 `reg.get_info(target).owner_id == actor.user_id || actor.is_superadmin`，否则拒绝并记审计；拒绝时向发起方回**带 reason 的拒连消息**，reason 文案=「无权远控该终端」（复用现有 `remotePhase="rejected"` + `RejectedCard{reason}` 渲染链路，web 无 Toast）（转绿 T017）
  - **共享文件**: `src/server/src/hub.rs`（串行：T019 后）
  - **覆盖 AC**: AC-006-H1, AC-006-E1, AC-006-E2
  - **完成标准**:
    - 正常：A 控自有终端放行
    - 异常：A 控他人/NULL 终端 → 拒绝 + 审计 result=被拒 + 发起方 RejectedCard 显示「无权远控该终端」
- [ ] T021 [P] [US2] 服务端：`query_audit`/`query_sessions` 加 owner 过滤——sessions `WHERE to_id IN (SELECT id FROM endpoint_registry WHERE owner_id=?)`；audit `WHERE session_id IN (SELECT id FROM sessions WHERE to_id IN (owned))`；superadmin 走无过滤分支（转绿 T018 前两项）
  - **独占文件**: `src/server/src/audit.rs`
  - **覆盖 AC**: AC-007-H1, AC-007-H2
- [ ] T022 [P] [US2] 服务端：`login_log.query` 加按 `username` 过滤重载（普通账号只看自己登录记录；superadmin 全量）（转绿 T018 第三项）
  - **独占文件**: `src/server/src/login_log.rs`
  - **覆盖 AC**: AC-007-H3
- [ ] T023 [US2] 服务端 HTTP：审计/会话/登录日志 handler 从 `AuthUser` 取归属传入 query（superadmin 无过滤分支）（依赖 T021/T022）
  - **独占文件**: `src/server/src/http.rs`
  - **覆盖 AC**: AC-007-H1, AC-007-H2, AC-007-H3, AC-007-E1
  - **完成标准**:
    - 正常：普通账号请求三类数据 → 仅返回自己范围；superadmin → 全量
    - 异常（空态）：A 无记录 → 返回空集（前端呈现「暂无记录」）
- [ ] T024 [P] [US2] web：`admin-web/src/store/auth.ts` 补存 `user_id`（`loadMe`/login 写入），支撑 superadmin 视图可选展示 owner（ts-rs 类型经 T003 已含 owner_id）
  - **独占文件**: `src/admin-web/src/store/auth.ts`
  - **覆盖 AC**: AC-005-H2（superadmin 展示辅助）
- [ ] T024b [P] [US2] web 空态文案（F2）：`terminal-assets.tsx:350-353` 当前空列表显示「正在加载终端列表…」/「未找到匹配的终端」——需区分「已加载但名下无终端」→ 显示「暂无你负责的终端」；`audit-log.tsx:242` 空态显示「暂无记录」。核对远控拒连 `RejectedCard{reason}` 链路（`control-client.tsx:50` `remoteRejectReason`）能透传服务端 reason
  - **独占文件**: `src/admin-web/src/components/assets/terminal-assets.tsx`, `src/admin-web/src/components/audit/audit-log.tsx`
  - **覆盖 AC**: AC-005-E1, AC-007-E1
  - **完成标准**:
    - 正常：名下有终端正常列出
    - 异常（空态）：普通账号名下 0 终端 → 显示「暂无你负责的终端」（非「正在加载…」）；审计 0 记录 → 「暂无记录」

**Checkpoint**: US1 + US2 独立可用 —— 两账号数据全面隔离，越权远控被拦。

---

## Phase 5: User Story 3 - 旧端兼容不回归 (P2)

**Goal**: 存量无登录旧端继续上线，归 NULL，仅 superadmin 可见，功能零回归。

**Independent Test**: 一台旧端（无 token）上线 → 在线、owner=NULL、普通账号列表不可见、superadmin 可见且可远控；旧 info JSON 缺字段不报错。

### Tests for US3（RED first）
- [ ] T025 [P] [US3] 【RED】服务端单测：无 token 连接发 Register → 上线成功、`owner_id=NULL`；`views_visible_to(Some("A"),false)` 不含、`views_visible_to(None,true)` 含
  - **独占文件**: `src/server/src/hub.rs`（测试模块）
  - **覆盖 AC**: AC-008-H1
- [ ] T026 [P] [US3] 【RED】协议单测：旧 `EndpointView` JSON（缺 `owner_id` 键）反序列化 → `owner_id=None`，其余字段完整、不报错（owner_id 仅加于 EndpointView，`EndpointInfo` 本无此字段，不测）
  - **独占文件**: `src/protocol/src/lib.rs`（测试模块）
  - **覆盖 AC**: AC-008-E1
- [ ] T027 [US3] 守卫核对与兜底（转绿 T025/T026）——逐条断言点：
  - (a) `main.rs:277` gate 条件为 `id.starts_with("admin-") && !authed`，即无 token 的 agent（非 admin- 前缀）不被 break → 断言无 token Register 上线成功
  - (b) `views_visible_to(Some(uid), false)` 对 owner=None 项返回 false；`(None, true)` 返回全含 → 断言 owner=None 仅 superadmin 可见
  - (c) T020 远控闸对 `owner_id==None` 且非 superadmin 返回拒绝 → 断言旧端普通账号不可控
  - **共享文件**: `src/server/src/main.rs`, `src/server/src/hub.rs`（串行：US1/US2 后）
  - **覆盖 AC**: AC-008-H1, AC-008-E1
  - **完成标准**:
    - 正常：旧端上线 owner=NULL，仅 superadmin 可见/可控（断言 a/b/c 全过）
    - 异常：旧 JSON 缺字段不丢消息、不 panic

**Checkpoint**: 三个 User Story 全部独立可用。

---

## Phase 6: Polish & Cross-Cutting

- [ ] T028 [P] 跑 `quickstart.md` 全 6 场景真机 E2E（隔离/越权/反伪造/旧端/记住凭据/换绑），逐条对 SC-001~006 打勾
- [ ] T029 远控零回归回归：superadmin 远控自有终端，截屏/键鼠注入/文件/聊天链路帧率与功能对齐改动前基线（防 src/client 首改劣化）
- [ ] T030 [P] 文档：README/发版说明标注「客户端需登录」；重新生成 ts-rs 类型核对 `admin-web` 编译通过
- [ ] T031 [P] **安全加固（强制 · F1）**：`credential.json` Unix 权限落 `0600`（写盘后 `set_permissions`）；确认 token 不出现在日志/界面明文（grep 日志 + 审查 UI 绑定）。纳入 DoD——满足 Security NFR「本机身份受保护存储、不明文可直接读取」，非可选项
- [ ] T032 defensive-delivery 四件套自检（潜在 Bug / 测试清单 / 交付自检 / 实现回顾）

---

## Dependencies & Execution Order

### Phase 依赖
- Setup(P1) → Foundational(P2，阻塞) → US1/US2/US3 → Polish
- **Foundational 未完成前禁止 US 开工**

### User Story 依赖
- **US1(P1)**: Foundational 后即可；服务端 owner 绑定 + 客户端登录
- **US2(P1)**: Foundational 后即可，**可与 US1 并行**（US2 测试直接置 owner，不依赖 US1 客户端）
- **US3(P2)**: 多为 US1/US2 设计副产物，其守卫核对 T027 需在 US1/US2 服务端改动后做

### 关键串行链（同文件）
- `hub.rs`: T009(US1) → T019(US2) → T020(US2) → T027(US3)
- `server/main.rs`: T008(US1) → T027(US3)

### 并行机会
- **Foundational**: T002 / T003 可并行（不同文件）
- **US1**: T010 / T011（credential.rs / login.rs 独占）可并行；测试 T006 / T007 可并行
- **US2**: T021 / T022 / T024（audit.rs / login_log.rs / auth.ts 独占）可并行；测试 T016 / T017 / T018 可并行
- **跨线并行**: 客户端线（T010-T015）与服务端线（T008-T009,T016-T023）与 web（T024）三线并行；`hub.rs` 内部串行

---

## Parallel Example: US2 测试与独占实现

```bash
# US2 RED 测试并行：
Task: "T016 push_list owner 过滤集成测试 (hub.rs 测试模块)"
Task: "T017 ConnectRequest owner 闸集成测试 (hub.rs 测试模块)"
Task: "T018 审计/会话/登录日志过滤集成测试 (audit.rs/login_log.rs)"

# US2 独占实现并行（hub.rs 之外）：
Task: "T021 audit/sessions owner 过滤 (audit.rs)"
Task: "T022 login_log username 过滤 (login_log.rs)"
Task: "T024 auth store 补 user_id (auth.ts)"
```

---

## Implementation Strategy

### MVP First
1. Phase 1 Setup → Phase 2 Foundational（owner 地基）
2. Phase 3 US1（客户端登录 + 归属绑定）→ **STOP 验证**：客户端登录后 DB owner 正确、错误路径提示到位
3. 可 demo：机器绑到人

### Incremental
1. Foundational → 地基就绪
2. +US1 → 登录归属闭环（MVP）
3. +US2 → 数据隔离生效（核心合规价值）
4. +US3 → 旧端兼容坐实
5. Polish → 真机 E2E + 零回归 + 交付自检

### 风险（RAID）
- **R1 高**：`hub.rs` 广播改造（T019）是回归高危区 → 集成测试 T016 先行 + T029 真机核对
- **R2 中**：src/client 首改（T012-T015）可能扰动远控链路 → T029 专项回归
- **R3 中**：协议加字段（T003）→ T026 旧端反序列化测试守护向后兼容
- **D1 依赖**：客户端登录禁用账号细分文案依赖服务端 401 是否可区分 → 不可区分则兜底「账号或密码错误」

---

## Requirements Coverage Matrix

| Requirement | Source | Covered By (Task IDs) | Status |
|-------------|--------|-----------------------|--------|
| FR-001 被控端登录 | spec §US1 | T011, T014, T015 | ⬜ |
| FR-002 上线绑定归属（反伪造） | spec §US1 | T006, T008, T009 | ⬜ |
| FR-003 记住凭据自动上线 | spec §US1 | T007, T010, T013 | ⬜ |
| FR-004 注销/换绑 | spec §US1 | T010, T014b, T014c, T015 | ⬜ |
| FR-005 终端列表按归属隔离 | spec §US2 | T016, T019, T024b | ⬜ |
| FR-006 远控范围闸 | spec §US2 | T017, T020 | ⬜ |
| FR-007 审计/会话/登录日志隔离 | spec §US2 | T018, T021, T022, T023, T024b | ⬜ |
| FR-008 旧被控端兼容 | spec §US3 | T025, T026, T027 | ⬜ |
| owner 数据地基（支撑全部） | plan §Foundational | T002, T003, T004, T005 | ⬜ |
| NFR-Security 凭据受保护存储（F1） | spec §NFR | T031 | ⬜ |
| SC-001 隔离有效性 | spec | T016, T018, T028 | ⬜ |
| SC-002 越权阻断率 | spec | T017, T020, T028 | ⬜ |
| SC-003 反伪造 | spec | T006, T009, T028 | ⬜ |
| SC-004 现网不中断 | spec | T025, T027, T028, T029 | ⬜ |
| SC-005 一次登录 | spec | T007, T013, T028 | ⬜ |
| SC-006 换绑生效 | spec | T015, T028 | ⬜ |

**Status 图例**: ⬜ 未完成 / ✅ 全关联任务已勾 / ⏭ 已跳过（登记原因）

## Skipped Tasks

| Task ID | Reason | FR Still Covered By |
|---------|--------|---------------------|
| *(none)* | | |

---

## Notes
- [P] = 不同文件、无未完成依赖
- US 任务均含「覆盖 AC + 完成标准（正常+异常）」；含消息反馈的完成标准写明形式+文案
- TDD：先跑 RED 测试确认失败再实现
- 每任务或逻辑组完成后提交
- 测试用例细目见 `test-plan.md`（测试工程师人格产出）
