# docs/superpowers 需求与设计评审结论

## 结论

当前状态：**NoGo**。

`docs/superpowers` 的总体方向可执行，M1-M5 需求也基本都有任务覆盖；但当前文档仍存在核心协议、数据库、模式 B 验收口径和并行任务边界的阻塞级不一致。若直接进入并行开发，server/client/frontend/mcp 四线很可能实现出不同接口，导致联调返工。

## 阻塞问题

| 级别 | 问题 | 依据 | 建议 |
|---|---|---|---|
| 🔴 CRITICAL | WS 信封结构有两套：需求写 `{ type, from, to, payload, ts }`，实现计划/编排改成 `payload.type` 内部 tag。 | `feature-spec-mvp.md` §0.3；`parallel-dev-orchestration.md` §4；`ohmydesk-mvp-implementation.md` Phase 0 | 以 `Envelope { from,to,ts,payload }` + `payload.type` 为准，回改需求文档和代码注释。 |
| 🔴 CRITICAL | 数据库选型 SQLite/MySQL 漂移。需求和设计残留 SQLite，实现计划和项目规范用 MySQL。 | `feature-spec-mvp.md` M4/M5；`xinchuang-remote-control-design.md` §5；`ohmydesk-mvp-implementation.md` Phase 6 | 全部统一为 MySQL；不要放到 Wave3 才清理，因为会影响 server/mcp/frontend 契约。 |
| 🔴 CRITICAL | `audit_logs.type` 已被标成阻塞问题，但实现计划 SQL 仍使用 `type VARCHAR(16)`。 | `parallel-dev-orchestration.md` B-DB1；`ohmydesk-mvp-implementation.md` Task 6.1 | 立即改为 `event_type`，同步 INSERT/查询字段。 |
| 🔴 CRITICAL | 模式 B 验收口径冲突：需求要求客户端到客户端完整远控，编排又把 Web 主控定为演示口径并把 Slint 主控降为 P1。 | `feature-spec-mvp.md` M2/§7；`parallel-dev-orchestration.md` P-DOC1；`ohmydesk-mvp-implementation.md` Self-Review | 明确二选一：保持客户端到客户端为 P0，或把需求验收改成“Web 主控 + 模式 B 密码校验/拒连”。 |

## 高风险问题

| 级别 | 问题 | 依据 | 建议 |
|---|---|---|---|
| 🟠 HIGH | Phase0 声称新增 `AuditLog`/`Session` 并导出，但协议代码片段只定义到 `EndpointView`，mock/frontend 又直接依赖这些类型。 | `ohmydesk-mvp-implementation.md` Phase 0；`mock-api-contract-and-adapters.md` §0/§2.2 | Phase0 必须显式定义并导出 `Session`、`AuditLog`、`AuditType`。 |
| 🟠 HIGH | 并行任务边界在实现计划里跨端混写：一个 Task 同时改 frontend/client，或 server/frontend。 | `ohmydesk-mvp-implementation.md` Task 4.3/4.4/6.2 | 按 agent 拆成 server/client/frontend 子任务，集成任务只负责验收和胶水。 |
| 🟠 HIGH | 截屏/注入尺寸规则冲突：编排要求真实 `w/h` 和等比缩放，计划代码仍写死 1280×720。 | `parallel-dev-orchestration.md` P-CLI4；`ohmydesk-mvp-implementation.md` Task 4.2/4.4 | 计划代码片段改成返回实际缩放后尺寸，注入按 `real_w/frame_w`、`real_h/frame_h` 计算。 |

## 中风险问题

| 级别 | 问题 | 依据 | 建议 |
|---|---|---|---|
| 🟡 MEDIUM | MCP 范围在 4 个/5 个工具、内置/独立 TS 进程之间漂移。 | `xinchuang-remote-control-design.md` M5/技术栈；`ohmydesk-mvp-implementation.md` Phase 7 | 写死：P0 四个工具，P1 `get_screenshots`；部署形态为独立 TS MCP 进程读 HTTP。 |
| 🟡 MEDIUM | 隔离策略自相矛盾：角色表写 worktree，随后又建议同一工作树。 | `parallel-dev-orchestration.md` §2 | 选择一种：死线冲刺建议同一工作树 + 目录 owner + integrator 串行合并。 |
| 🟡 MEDIUM | 包管理器不统一：项目规范和前端用 pnpm，MCP 计划用 npm。 | `.agent/user.md`；`parallel-dev-orchestration.md` §0；`ohmydesk-mvp-implementation.md` Phase 7 | MCP 也统一 `pnpm init` / `pnpm add`。 |

## 覆盖与可执行性

- M1-M5 的需求都有对应 Phase/Task，覆盖面基本完整。
- Wave0 协议冻结、Wave1 四线并行、Wave2 滚动集成的编排方向合理。
- 当前主要风险不是“有没有任务”，而是“同一任务的事实源不唯一”。

## 并行边界判定

边界“概念上清晰”，但“执行级任务不够清晰”。

`parallel-dev-orchestration.md` 的 owner 划分是可用的；`ohmydesk-mvp-implementation.md` 里的 Task 仍需要拆到单 owner，否则 frontend-dev/client-dev/server-dev 会在同一个里程碑里互相等待或同时修改跨端文件。

## 建议下一步

先做一次文档收敛补丁，再启动 Wave0：

1. 统一 WS 信封为 `Envelope { from, to, ts, payload }` + `payload.type`。
2. 统一数据库为 MySQL，移除 SQLite 残留。
3. 修正 `audit_logs.type` 为 `audit_logs.event_type`。
4. 决定模式 B 的 P0 验收口径。
5. 在 Phase0 补齐 `Session`、`AuditLog`、`AuditType`。
6. 把跨端 Task 拆成单 owner 子任务。
7. 统一 MCP 为独立 TS 进程、P0 四工具、pnpm 包管理。

---

# 第二轮评审（架构师人格 + design-patterns，2026-06-27）

> **评审对象**：裁决回流后的单一 TDD 入口 `plans/...-mvp-implementation.md`（commit 27c3085 后），交叉 `parallel-dev-orchestration.md` / `tripartite-consistency-analysis.md` / `design.md`。
> **评审视角**：① 复核上一轮 4 个 CRITICAL 是否真解决；② design-patterns 架构合理性。

## 结论：**Conditional Go（有条件放行）**

真阻塞从上一轮 **4 → 0**。回流把协议层/server 必修项落进了代码示例，AuditLog/Session/event_type/模式B口径全部就位。本轮发现的不是阻塞，而是 **2 处「回流裂缝」**（plan 顶部清单已声明、代码示例没同步——正是上一轮回流要消除的「事实源不唯一」）+ **2 条架构设计纪律**。前者已在本轮就地修复，后者钉进 Wave 0/Phase 4 即可进场。

## 一、上一轮 4 CRITICAL 复核

| 上轮问题 | 现状 | 判定 |
|---|---|---|
| WS 信封双套 | plan W0-3 + 代码 `#[serde(tag="type")]` 已统一 `Envelope{from,to,ts,payload}` | ✅ 代码层闭环（feature-spec §0.3 叙述残留→收尾） |
| SQLite/MySQL 漂移 | plan Phase 6 全 MySQL；但 **design §99/§122/§125 + feature-spec F-M4-3/F-M5-4 仍写 SQLite** | ⚠️ 契约层已 MySQL，**文档叙述残留**（C-2，收尾清理；Wave 0 公告口头钉死） |
| `audit_logs.type` 保留字 | Task 6.1 schema 已 `event_type` | ✅ |
| 模式 B 验收口径 | **用户裁决推翻评审降级**：client→client 恢复 **P0**（F-M2-2/4/5），Slint 主控端必做（plan Task 4.6），Web 主控降为兜底 | 🔄 已修正（详见「四、用户裁决」） |

上一轮 HIGH：AuditLog/Session 已在 Phase 0 完整定义+导出 ✅；跨端 Task 由 orchestration 按 owner 拆线 ✅；截屏尺寸写死见下 R1。

## 二、本轮新发现

### 🔴 回流裂缝（已就地修复）
- **R1 — P-CLI4 回流不彻底**：顶部清单声明「等比缩放+真实 w/h」，但 Task 4.2 `resize(&img,1280,720)` 写死拉伸、Task 4.4 `scale_x: real_w/1280.0` 写死分母。非 16:9 屏画面拉伸 + 注入坐标偏。**已改**：capture 按长边≤1280 等比缩放返回真实 w/h；`Injector::new(real,frame)` 按 `real/frame` 换算；Web/被控注释同步。
- **R2 — `git add -A` 违反用户硬约束**：Task 4.5 Step 4 用 `git add -A`（用户红线：禁 `add -A/.`，须显式列文件）。**已改**为显式三文件 add。

### 🟠 架构设计纪律（钉进 plan，非阻塞）
- **A1 — Hub 有 God-method 演进风险**：`Hub::handle()` 是 Mediator（星型中枢路由，模式选对了），但当前 match 已揉「连接路由 + 会话建立 + 列表广播」，Phase 4 还要加鉴权、Phase 5 加截图广播、Phase 6 加审计 bump——会长成单一巨型 match。**纪律**：Hub 只做纯路由/广播；`ConnectRequest/AuthResult` 委托 `session.rs`（plan 已规划 Task 4.1 建），`Input` 转发触发审计 `bump`，别全堆 `handle()`。守住即可，不需重构。
- **A2 — Session 拒因语义（已钉死）**：`SessionStatus{Active,Ended,Rejected}` 把「密码错 auth_fail」与「被控拒 reject」两种拒因合并到一个 `Rejected`，而 `AuditType` 区分两者。**已在 Phase 0 加注释钉死**：status 只记终态，拒因细分查 `AuditLog.kind`，避免四线各端理解不一。

### ✅ 模式选择正确（无过度设计，符合 KISS/YAGNI）
- 前端 `Transport`（mock/real 同接口运行时切换）= **Strategy**；适配层 D-1~D-8（ts-rs 类型→组件视图）= **Adapter**；`AuditStore{db:Option<Db>}` 降级 = **Null Object** 轻量变体；Hub = **Mediator**。四个模式都用在真问题上，没有为模式而模式。

## 三、放行条件（进 Wave 0 前）

1. ✅ R1/R2 已修（本轮完成）。
2. Wave 0 协议冻结公告时**口头钉死 MySQL**（C-2 文档残留留待收尾，但四线认知须先统一）。
3. A1/A2 作为 Phase 4 编码纪律执行（A2 已写进协议注释）。

> 满足后即可从 **Wave 0 = Phase 0 protocol** 进 TDD。

## 四、用户裁决修正（2026-06-27，推翻评审 P-DOC1 降级）

- **client→client 恢复 P0**：评审/编排曾把模式 B 的 client 主控端（Slint 发起 UI + 键鼠捕获）降为 P1、用 Web 主控替代演示——这与需求 **F-M2-2/F-M2-4/F-M2-5（均 P0）+ §MVP 验收「模式 A 与模式 B 各跑通一次」** 冲突。经用户确认，**模式 B = client→client 是 P0**，client 主控端必做（新增 plan **Task 4.6**：Slint 发起面板 + 贴帧 + 键鼠捕获回传）。
- **执行策略 = P0 + Web 主控兜底**：先用模式 A（Web 主控）打通整条远控链路（截屏/注入/会话/审计被控端+server 完全共用），再加 client 主控端表现层；Slint 主控若现场翻车，Web 主控兜底演示模式 B 的鉴权/拒连/审计，**兜底≠替代 P0 目标**。
- **联动修正**：plan（P-CLI5 + Task 4.6 + Phase 2.3 app.slint 发起面板 + Self-Review）、orchestration（P-DOC1/I2/I2 风险/client-dev）、本评审表「模式 B」行同步更新。

## 五、文档残留清理（C-2/C-3/C-4，本轮一并落实，不留 Wave 3）

- **C-2 SQLite→MySQL**：design §5「无重型数据库/SQLite」改「实时态内存 + 历史态 MySQL」、§6 目录树 server/mcp 注释、feature-spec F-M4-3/F-M5-4 数据源——全部清为 MySQL。
- **C-3 Tauri→Slint**：design §12 路线图 P2「Tauri 客户端/集成」改 Slint。
- **C-4 §8 消息类型对齐 protocol**：design §8 信封 `{type,from,to,payload,ts}`→`{from,to,ts,payload}`+内部 tag；`auth_challenge/mouse`→`auth_result/input`，补 `register_ack/reject`；§7 `AuditLog.type` 枚举对齐 `AuditType`（含 `auth_fail/reject/input`）。
