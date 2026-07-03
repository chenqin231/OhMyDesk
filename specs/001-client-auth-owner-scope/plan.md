# Implementation Plan: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope` | **Date**: 2026-07-03 | **Spec**: `specs/001-client-auth-owner-scope/spec.md`
**Input**: Feature specification from `/specs/001-client-auth-owner-scope/spec.md`

## Summary

给 Rust 被控端加账号登录（复用 WEB `/api/login` 的账号体系），登录后本机以「当前账号=负责人（owner）」上线；owner 由服务端从校验过的 JWT 派生（不信客户端自报字段）。服务端为终端引入 owner 维度，并对终端列表/监控推送、远控闸、审计/会话/登录日志做按 owner 的行级隔离，superadmin 全量绕过。旧被控端无登录 → owner=NULL → 仅 superadmin 可见，现网不中断。

技术取向：**加性改造，不改架构风格**。协议向后兼容（新增字段用 `Option`/`#[serde(default)]`，旧端缺字段不丢消息）；最高风险在 hub 的「全量广播」改「按连接 owner 过滤推送」，与 src/client 首次改动的远控零回归。

## Technical Context

**Language/Version**: Rust 2021（client / server / protocol，workspace 0.4.9）；TypeScript + React 19（admin-web）
**Primary Dependencies**: Slint（client GUI，软渲染）、tokio-tungstenite（WS）、sqlx + SQLite、bcrypt、jsonwebtoken（HS256）；client 侧登录 = **复用已有 `ureq`** 调 `/api/login`（Phase 0 D1 已决，零新依赖）；admin-web: zustand + react-router 7 + Vite
**Storage**: SQLite（`users` / `endpoint_registry` / `audit_logs` / `sessions` / `login_logs`）
**Testing**: cargo test（Rust 单元/集成，含现有 registry/hub/auth 测试）；admin-web 现有前端测试
**Target Platform**: Windows / Linux 被控端；Linux 服务端（Docker）
**Project Type**: web + 多 crate 客户端/服务端（workspace: src/protocol, src/server, src/client + src/admin-web）
**Performance Goals**: 隔离过滤为 O(n) 内存过滤，不引入新的性能目标；不劣化现有远控帧率
**Constraints**: 协议向后兼容（旧端不回归）；src/client 截屏/注入/远控链路零回归；终端列表推送级隔离（非仅首屏）；owner 判定只认服务端 JWT
**Scale/Scope**: 内网规模（数十~数百终端 / 若干管理员账号 / 单 SQLite 单 server）

## Architecture Scope Assessment（架构规模评估）

> **必须决策**：根据 spec.md 需求内容，评估影响范围并选择对应的架构分析深度。

### 影响范围判断

| 维度 | 本次需求 |
|------|---------|
| 影响模块数 | 4+（protocol / server / client / admin-web） |
| 是否新建模块 | 新增文件（client 登录模块 + 凭据存储；server owner 过滤逻辑），不新增服务 |
| 接口变更范围 | 系统边界（WS 协议 Register 扩展 + 被控端连接携带凭据），但**加性向后兼容** |
| 数据模型变更 | 新关系（endpoint ↔ owner，新增 owner 字段/回填迁移） |

### 架构层级判定

本次需求属于：

- [x] **L2 跨模块（偏 L3）** — 4 模块协同 + 系统边界协议扩展，但为加性改造，**不新增服务、不改架构风格**，故按 L2 处理，仅在协议兼容与广播链路改造两处按 L3 谨慎度对待。

理由：无新架构风格、无新服务、无重大重构；核心是「加一个 owner 维度 + 沿数据出口加过滤 + 客户端加登录」。过度上升到 L3 全套架构选型不符合 YAGNI。

### 推荐架构 Skills

| 层级 | 加载 Skills | 关注点 |
|------|------------|--------|
| L2 | `design-patterns`（按需）+ `rust-remote-control-stack`（client Slint/连接改造参考） | owner 过滤的策略封装 + Slint 登录页/连接引导 |

**本次推荐加载**：
- **已自动加载**：`rust-patterns`（Rust 主语言规则，贯穿全程）
- **按需加载（到对应设计点再载）**：`rust-remote-control-stack`（client 登录页 + WS 连接引导改造时）；`design-patterns`（若 owner 过滤需要抽策略时）
- **admin-web 改动极小**（仅 auth store 补 user_id），不加载 `v0-to-project`

⏸ **GATE: 用户确认加载的 Skills 后继续**

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

`.specify/memory/constitution.md` 为未填充模板 → 采用项目 CLAUDE.md 铁律作为 gate：

| Gate | 要求 | 本方案符合性 |
|------|------|------------|
| TDD 先测试 | 实现前先写失败测试 | ✅ 每 FR 有可断言 AC；server 侧有现成 registry/hub/auth 测试框架可扩 |
| 向后兼容 | 协议演进不破坏旧端 | ✅ 新增字段 `Option`/`serde(default)`，旧端缺字段不丢消息（吸取 [[protocol-evolution-breaks-old-endpoints]] 教训） |
| 零回归 | src/client 远控链路不劣化 | ✅ 登录为前置门，远控/截屏/注入代码路径不改动语义 |
| KISS / YAGNI | 不过度设计 | ✅ owner 为单值字段（非多对多），不引入分配 UI、不引入分组维度 |
| 模块 ≤500 行 | 单文件行数受控 | ✅ 见 Module Decomposition，登录逻辑独立成文件 |
| 防御性交付 | 交付前 4 件套自检 | ✅ 纳入 Post-Design AC 验证 + 测试清单 |

**结论**：无违反项，Complexity Tracking 留空。可进入 Phase 0。

**Post-Design 复检（Phase 1 后）**：设计未引入新抽象层——owner 为单值 `Option<String>` 列、复用 `add_column_if_missing` 迁移、复用 `ActorIdentity.user_id`、复用 `ureq`/`FieldInput`。9 个模块均 ≤220 行、职责单一、无循环依赖。协议兼容用 `Option<T>` 先例。**仍无违反项**。

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # /speckit.tasks command output (NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
src/
├── protocol/src/lib.rs            # EndpointView 加 owner_id: Option<String>（ts-rs 导出）
├── server/src/
│   ├── main.rs                    # ws_handler 对 agent 也 validate；放开 bind_actor 限制（D3）
│   ├── hub.rs                     # Register 臂注入 owner；push_list 逐 admin 过滤；ConnectRequest 加 owner 闸
│   ├── registry.rs               # Entry.owner + upsert 增参 + db_save/db_load_all 带 owner_id + views_visible_to
│   ├── db.rs                      # ensure_identity_columns 加 endpoint_registry.owner_id 迁移
│   ├── audit.rs                   # query_audit / query_sessions 加 owner 过滤（经 session.to_id）
│   ├── login_log.rs              # query 加 username 过滤重载
│   └── http.rs                    # 审计/会话/登录日志 handler 传入 AuthUser 归属；superadmin 分支
├── client/src/
│   ├── main.rs                    # 启动读盘 token；登录门时序（D10）；mod credential
│   ├── credential.rs             # 🆕 token 持久化（load/save/clear，仿 history.rs）
│   ├── login.rs                  # 🆕 ureq 调 /api/login（复用 update::build_agent）
│   ├── ui_glue.rs                # on_login / on_logout 回调（仿 refresh_password + list_local 后台 IO）
│   ├── net/{mod.rs,conn.rs}      # net::run/connect_once 加 token 参数；WS URL 拼 ?token=；等 token 就绪再连
│   └── ui/app.slint             # logged_in 门控 + S1 登录页 + S2 顶栏账号条 + S3 注销 Modal
├── admin-web/src/store/auth.ts    # 补存 user_id（支撑前端可选 owner 展示）
└── scripts/db/schema.sqlite.sql   # endpoint_registry 基础 DDL 加 owner_id 列
```

**Structure Decision**: 沿用现有 workspace（protocol / server / client）+ admin-web，不新增 crate/模块目录。改动分布在 4 模块，client 侧新增 2 个小文件（credential.rs / login.rs）保持单文件职责单一、≤200 行。

## UI/UX Design（界面与交互设计）

> 目标为 **Slint 原生桌面端**，视觉风格被现有 `app.slint` 深色 token 锁定（Card `#18181c`、遮罩 `#000000bb`、`FieldInput`/`PrimaryButton` 现成）。**不加载 ui-ux-pro-max**（其 web 风格库不适用）；设计约束 = 与现有客户端 100% 一致，新增即复用，零新组件族。

### Style & Stack Selection（风格与技术栈选型）

| 决策项 | 选择 | 理由 |
|--------|------|------|
| Visual Style | 沿用现有深色扁平（Card + 留白） | 一致性；登录页不与主界面割裂 |
| UI Stack | Slint（复用 Card / FieldInput / PrimaryButton） | 零新依赖，`app.slint` 现成组件 |
| Interaction Pattern | 全屏门控页（登录）+ 居中模态（注销确认） | 复用 `app.slint:1387` 模态范式；登录为前置门，全屏最清晰 |

### Page / Screen Inventory（页面清单）

| Screen | Purpose | Entry Point | Exit Point |
|--------|---------|-------------|------------|
| S1 登录页（全屏） | 未登录态输账号密码 | `!logged_in` | 登录成功 → 主界面 |
| S2 顶栏账号条 | 已登录展示 + 注销入口 | `logged_in` | 注销 → 回 S1 |
| S3 注销确认 Modal | 破坏性操作二次确认 | 点「注销」 | 取消 / 确定 |

### Component Breakdown（组件拆解）

| Screen | Component | States | Notes |
|--------|-----------|--------|-------|
| S1 | Card(380px)：标题 / 账号 FieldInput / 密码 FieldInput(password) / 服务器地址(折叠「高级」) / PrimaryButton / Inline 错误区 | empty(初始) / loading(「登录中…」禁用) / error(红字 inline) / success(整页消失) | 复用 `FieldInput`(`app.slint:183`)、`PrimaryButton`(`:129`)、`Card`(`:67`) |
| S2 | 顶栏：`已登录:<用户名>` + ●在线状态点 + `[注销]` TextButton | 在线/离线状态点变色 | 加在现有空闲态主界面顶部 |
| S3 | 遮罩 + Card：标题文案 + `[取消]` `[确定]`(确定高亮) | 展示/取消/确定 | 照抄 `app.slint:1387-1496` 模态范式 |

### Message & Feedback Components（消息反馈组件设计 · 客户端 Slint）

| 反馈类型 | 触发场景 | 默认时长 | 位置 | 消失方式 | 样式规则 |
|---------|---------|---------|------|---------|---------|
| Inline (错误) | 登录失败 / 网络失败 / 空值 | 持续 | 字段或按钮下方 | 重新输入 / 重试时消失 | 红色文字 |
| Modal (Confirm) | 注销前 | — | 居中遮罩 | 点取消 / 确定 | Card + 标题 + 双按钮，确定高亮 |
| 状态条 | 登录态展示 | 持续 | 主界面顶栏 | 注销后消失 | 文本 + ●状态点 |

**文案规范**：错误 = 问题 + 恢复建议（「无法连接服务器，请检查网络后重试」）；确认 = 后果 + 询问（「注销后本机将下线，确定?」）。

> **两端澄清**：FR-006「无权远控该终端」提示属 **admin-web**（web 端）。web **无 Toast 库**，实为复用现有拒连结果卡片 `RejectedCard{reason}`（`control-client.tsx:50` `remoteRejectReason`）——服务端拒绝时回带 reason 文案即可，**不在** Slint 客户端。客户端仅上述三类反馈。

### UX Flow（交互流程）

**启动 → 登录 → 主界面**
1. 启动读盘 token → 有效 → 跳过 S1 直接主界面在线态
2. 无 / 失效 → 显示 S1 登录页

**登录**
1. 输入账号密码 → 回车或点「登录」→ 按钮转 loading 禁用
2. 后台 `ureq` 调 `/api/login` → 成功：`credential::save` + 启动/放行 WS 连接 + 进主界面在线
3. 失败：inline 红字错误 + 按钮恢复可点

**注销**
1. 点「注销」→ S3 Modal
2. 确定：断 WS + `credential::clear` + 回 S1；取消：关 Modal，保持在线态

**Error Recovery / Edge Paths**
- 网络失败 → inline「无法连接服务器，请检查网络后重试」，按钮恢复
- token 过期（自动上线失败）→ 回 S1 + inline「登录已过期，请重新登录」
- 账号被禁用 → inline「账号已被禁用，请联系管理员」
- 服务器地址默认折叠在「高级」，仅需改址时展开
- 窗口关闭 / 返回：标准关闭行为，不额外拦截

---

## Module Decomposition（模块划分）

> **强制要求**：遵循"8️⃣ 模块化设计原则"，每个模块预估行数不得超过 500 行

| 模块 | 职责 | 文件路径 | 预估改动行数 | 依赖 |
|--------|------|---------|---------|---------|
| M1 owner 数据地基 | endpoint_registry 加列 + Entry.owner + upsert/db_save/db_load 带 owner + views_visible_to | `db.rs` `registry.rs` `schema.sqlite.sql` | ~120 | 无 |
| M2 鉴权开通 | agent 也 validate + bind_actor；Register 臂注入 owner | `main.rs` `hub.rs` | ~60 | M1 |
| M3 列表隔离 | push_list 逐 admin 按 owner 过滤（3 触发点） | `hub.rs` | ~70 | M1,M2 |
| M4 远控范围闸 | ConnectRequest 加 target owner 交集 | `hub.rs` | ~30 | M1,M2 |
| M5 审计/会话/日志过滤 | query_audit/query_sessions 经 session.to_id；login_log 按 username；handler 传归属 | `audit.rs` `login_log.rs` `http.rs` | ~120 | M1 |
| M6 协议字段 | EndpointView 加 owner_id: Option（ts-rs 导出） | `protocol/src/lib.rs` | ~10 | 无 |
| M7 客户端登录 | credential.rs + login.rs + 登录门时序 + net 传 token + WS 拼 ?token= | `client/{credential.rs,login.rs,main.rs,net/*}` | ~220 | 无（对接服务端 C2） |
| M8 客户端 UI | logged_in 门控 + S1/S2/S3 三屏 + on_login/on_logout | `client/ui/app.slint` `ui_glue.rs` | ~200 | M7 |
| M9 web auth store | 补存 user_id | `admin-web/src/store/auth.ts` | ~15 | 无 |

### 模块依赖关系

```
M1 owner地基 ─┬─ M2 鉴权开通 ─┬─ M3 列表隔离
             │               └─ M4 远控闸
             └─ M5 审计/会话/日志过滤
M6 协议字段（独立）
M7 客户端登录 ── M8 客户端 UI
M9 web store（独立）
```

**循环依赖检测**：无。服务端链 M1→M2→{M3,M4} 与 M5 单向；客户端 M7→M8 单向；M6/M9 独立。

## Parallel Development Feasibility（并行开发可行性）

| 任务组 | 涉及文件 | 与其他任务冲突？ | 可并行？ |
|--------|---------|----------------|---------|
| M1 owner 地基 | db.rs, registry.rs, schema.sqlite.sql | 服务端链前置 | 否，最先做 |
| M6 协议字段 | protocol/lib.rs | 独立 | 是（可与 M1 并行） |
| M7+M8 客户端 | client/* | 独立于服务端 | 是（整条客户端线可与服务端线并行） |
| M9 web store | admin-web/store/auth.ts | 独立 | 是 |
| M2/M3/M4 | main.rs, hub.rs | **共享 hub.rs**，且依赖 M1 | 否，M1 后串行（M3/M4 同文件不同函数，谨慎并行） |
| M5 审计过滤 | audit.rs, login_log.rs, http.rs | 依赖 M1，文件独立于 hub | M1 后可与 M2 并行 |

**并行策略**：服务端线（M1→M2→M3/M4，M5）与客户端线（M7→M8）、协议（M6）、web（M9）**三线并行**。hub.rs 是服务端串行瓶颈（M2/M3/M4 集中于此），建议单人顺序改，避免同文件冲突（worktree 隔离对同文件无效）。

## File Structure Inventory（文件清单）

| File Path | Responsibility | Est. Lines | New/Modify | Dependencies |
|-----------|---------------|-----------|------------|-------------|
| `src/protocol/src/lib.rs` | EndpointView 加 `owner_id: Option<String>` | +10 | Modify | — |
| `scripts/db/schema.sqlite.sql` | endpoint_registry 基础 DDL 加 owner_id 列 | +1 | Modify | — |
| `src/server/src/db.rs` | ensure_identity_columns 加 owner_id 迁移 | +5 | Modify | schema |
| `src/server/src/registry.rs` | Entry.owner + upsert 增参 + db_save/load 带列 + views_visible_to | +90 | Modify | db.rs |
| `src/server/src/main.rs` | agent 也 validate + 放开 bind_actor | +25 | Modify | registry |
| `src/server/src/hub.rs` | Register 注入 owner + push_list 逐 admin 过滤 + ConnectRequest owner 闸 | +90 | Modify | registry, main |
| `src/server/src/audit.rs` | query_audit/query_sessions 加 owner 过滤 | +40 | Modify | registry |
| `src/server/src/login_log.rs` | query 加 username 过滤重载 | +20 | Modify | — |
| `src/server/src/http.rs` | 审计/会话/登录日志 handler 传 AuthUser 归属 + superadmin 分支 | +45 | Modify | audit, login_log |
| `src/client/src/credential.rs` | token 持久化 load/save/clear | ~70 | **New** | — |
| `src/client/src/login.rs` | ureq 调 /api/login（复用 build_agent） | ~60 | **New** | update.rs |
| `src/client/src/main.rs` | 读盘 token + 登录门时序 + mod 声明 | +30 | Modify | credential, login |
| `src/client/src/net/mod.rs` | net::run 加 token 参数 + 等 token 就绪再连 | +25 | Modify | — |
| `src/client/src/net/conn.rs` | connect_once 加 token + WS URL 拼 ?token= | +15 | Modify | — |
| `src/client/src/ui_glue.rs` | on_login / on_logout 回调 | +70 | Modify | login, credential |
| `src/client/ui/app.slint` | logged_in 门控 + S1/S2/S3 三屏 | +200 | Modify | — |
| `src/admin-web/src/store/auth.ts` | 补存 user_id | +15 | Modify | — |

**校验清单**：
- [x] 每文件 ≤300 行改动（最大 app.slint +200，仍在同文件既有 1600+ 行内，新增块 <300）
- [x] 每文件单一职责
- [x] 无循环依赖（见模块依赖图）
- [x] 同文件多任务已标串行（hub.rs 的 M2/M3/M4）

## AC Verification Design（验收标准验证设计）

> **强制要求**：每条 spec AC 必须映射到可执行的技术断言。由 `/speckit.plan` Step 4.9 自动填充。
> **覆盖度要求**：每个涉及 UI 的 FR 必须同时有 Happy + Error 行；消息反馈必须断言具体文案。

| Spec AC | Coverage Type | Source | Verification Type | Technical Assertion | Verification Method |
|---------|--------------|--------|------------------|--------------------|--------------------|
| AC-001-H1 | Happy | spec | Manual | 有效账号密码 → WS 以 `?token=` 建连成功 + `logged_in=true` + 顶栏文本含 `已登录:<user>` | 客户端手动登录，观察进入在线态 |
| AC-001-E1 | Error+Message | spec | Manual | 错密码 → login 收 401 → `login_error=="账号或密码错误"`，密码框 text 清空、账号保留 | 输错密码，断言 inline 文案 |
| AC-001-E2 | Error+Message | spec | Manual | 断网 → ureq 连接 Err → `login_error=="无法连接服务器，请检查网络后重试"`，登录按钮 enabled 恢复 | 断网点登录 |
| AC-001-E3 | Error+Message | spec | Manual | 禁用账号 → 401 → `login_error=="账号已被禁用，请联系管理员"`（服务端不可区分时兜底 E1 文案） | 禁用账号后登录 |
| AC-001-E4 | Error+Message | spec | Manual | 空账号/密码 → 不发请求 → 空框下 `请输入账号`/`请输入密码`，焦点定位首个空框 | 空值点登录 |
| AC-002-H1 | Happy | spec | Integration | token=A 连接 Register → `endpoint_registry.owner_id == A.id`；`views_visible_to(B,false)` 不含该 ep | cargo test 模拟 A token 注册 |
| AC-002-E1 | Error（反伪造） | spec | Unit | Register 的 `info` 含伪造 `owner=B`，连接 token=A → 落库 `owner_id == A.id`（≠B） | 单测构造伪造 info |
| AC-002-E2 | Error+Message | spec | Manual | 过期 token 连接 → WS close 1008 → 客户端回 S1 + inline `登录已过期，请重新登录` | 用过期 token 连 |
| AC-003-H1 | Happy | spec | Manual | credential.json 有效 → 启动跳过 S1，`logged_in=true` 自动上线 | 登录后重启客户端 3 次 |
| AC-003-E1 | Error+Message | spec | Manual | 过期 token → 自动上线失败 → 回 S1 + inline `登录已过期，请重新登录` | token 过期后重启 |
| AC-003-E2 | Error+Message | spec | Manual | 账号被删/禁 → validate None → 回 S1 + inline `账号不可用，请重新登录` | 删账号后重启客户端 |
| AC-004-H1 | Happy+Message | spec | Manual | 注销 → Modal 文案 `注销后本机将下线,确定?` → 确定 → WS 断 + credential.json 被删；服务端该 ep offline 且 `owner_id` 仍=A | 登录后注销，查文件删除 + DB owner 保留 |
| AC-004-H2 | Happy | spec | Integration | 注销后 B 登录 → ep `owner_id` 由 A 覆盖为 B；`views_visible_to(A)` 不含、`(B)` 含 | A 注销→B 登录，两视图核对 |
| AC-004-E1 | Error+Message | spec | Manual | Modal 点取消 → `logged_in` 仍 true、WS 在线，无状态变化 | 点注销再取消 |
| AC-005-H1 | Happy | spec | Integration | `push_list` 给 admin(A) 的 EndpointList 仅含 `owner_id==A.id`，含 B 项数=0 | cargo test：A/B 各 1 终端断言推送 |
| AC-005-H2 | Happy | spec | Integration | superadmin 连接收全量（含 owner=NULL 旧端） | cargo test superadmin 分支 |
| AC-005-E1 | Error+Message | spec | Manual | A 无终端 → 列表空态 `暂无你负责的终端`，无他人项 | 无归属终端时以 A 登录 web |
| AC-005-E2 | Error（推送级） | spec | Integration | B 终端上线触发 push_list → A 连接通道收到的列表仍不含 B | cargo test：B 上线后查 A 通道消息 |
| AC-006-H1 | Happy | spec | Integration | actor=A、target.owner=A → ConnectRequest 进入建会话路径 | cargo test owner==actor |
| AC-006-E1 | Error+Message | spec | Integration+Manual | actor=A、target.owner=B → 闸返回拒绝（不建会话）+ audit result=被拒；web Toast `无权远控该终端` 显示 5 秒 | cargo test 拒绝路径+审计；web 手动断言 toast |
| AC-006-E2 | Happy（superadmin） | spec | Integration | superadmin + 任意 target → 放行 | cargo test superadmin |
| AC-007-H1 | Happy | spec | Integration | `query_audit(A)` 仅返回 `session_id ∈ (sessions.to_id ∈ owned(A))`，含 B 终端审计数=0 | cargo test 造 A/B 会话+审计 |
| AC-007-H2 | Happy | spec | Integration | `query_sessions(A)` 仅 `to_id ∈ owned(A)` | cargo test |
| AC-007-H3 | Happy | spec | Integration | `login_log.query(A)` 仅 `username==A` 的行 | cargo test |
| AC-007-E1 | Error+Message | spec | Manual | A 无审计 → 审计页空态 `暂无记录`，无他人行 | 手动 web |
| AC-008-H1 | Happy | spec | Integration | 无 token 连接 Register → 上线成功 + `owner_id=NULL`；superadmin 可见、普通 A 不可见 | cargo test 无 token 注册路径 |
| AC-008-E1 | Error（兼容） | spec | Unit | 旧端 info 缺新字段 → EndpointView `owner_id` 反序列化为 None，不报错不丢消息 | cargo test 旧 JSON 反序列化 |
| SC-001 | Happy | spec | Integration+Manual | 他人终端记录数=0（列表/监控/审计/会话/登录日志五处） | quickstart 场景1 + 审计核对 |
| SC-002 | Error | spec | Integration | A 对 B 终端远控 ≥5 次拒绝率=100% + 全部记审计 | quickstart 场景2 |
| SC-003 | Error | spec | Unit | 伪造 owner 的 Register，落库 `owner_id==token.sub`=100% | quickstart 场景3 + `SELECT owner_id` |
| SC-004 | Happy | spec | Integration+Manual | 旧端升级前后在线 + superadmin 可控不变；上线成功率≥基线 | quickstart 场景4 |
| SC-005 | Happy | spec | Manual | 记住凭据机器重启 3 次，人工输入次数=0 | quickstart 场景5 |
| SC-006 | Happy | spec | Manual+Integration | 换绑后归属翻转，旧账号 ≤1 次刷新不再可见 | quickstart 场景5 |

**Coverage Type 图例**：
- **Happy** — 正常操作路径验证
- **Error** — 异常操作路径验证（输入异常/状态异常/外部异常）
- **Message** — 消息反馈验证（Toast/Modal/Inline/Notification/页面跳转/状态变更 的文案、时机、消失方式）

**验证通过条件**：
- [ ] 每条 FR AC 至少 1 行映射（零覆盖 → 返回 spec 补充）
- [ ] 每条 SC 至少 1 行映射
- [ ] 每个 FR 同时有 Happy + Error 行；UI 相关 FR 额外需要消息反馈断言行
- [ ] 消息反馈断言包含具体文案（如 `toast[text="保存成功"]`、`redirect to /result`），禁止仅写 "toast appears"
- [ ] Technical Assertion 列无模糊词（optimize/improve/ensure/加强/完善 → 重写）
- [ ] Manual verification 行包含逐步操作步骤（非仅 "manually check"）

---

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
