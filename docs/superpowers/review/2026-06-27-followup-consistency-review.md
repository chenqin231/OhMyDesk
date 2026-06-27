# docs/superpowers 第三轮一致性与可执行性评审

## 结论

当前判定：**NoGo（跨文档一致性未达标）**。

实现计划已经修复上一轮的大部分执行级问题：`AuditLog`/`Session` 已进入 Phase 0，`event_type` 已替代 MySQL 保留字 `type`，截屏与注入也已改成等比缩放和真实 `w/h`。但 `feature-spec`、设计文档、并行编排、实现计划之间仍有核心验收口径冲突；如果按多 agent 并行开发，仍可能出现不同 agent 按不同事实源实现。

## 已修复项

| 上轮问题 | 当前状态 | 判定 |
|---|---|---|
| Phase 0 缺 `AuditLog` / `Session` / `AuditType` | `ohmydesk-mvp-implementation.md` Phase 0 已定义 `Session`、`SessionStatus`、`AuditLog`、`AuditType`，并在 ts-rs 导出测试中显式导出。 | 已修复 |
| `audit_logs.type` 使用 MySQL 保留字 | Task 6.1 schema 已改为 `event_type`，审计写入说明也同步为 `event_type`。 | 已修复 |
| 截屏与注入写死 1280x720 | Task 4.2 已按比例缩放并返回真实帧尺寸；Task 4.4 注入按 `real_w/frame_w`、`real_h/frame_h` 缩放。 | 已修复 |
| DB 失败会拖垮实时链路 | Task 6.1 已要求 `Option<Db>`，DB 连接失败时审计 no-op + 告警，M1/M2/M3 不依赖 DB。 | 已修复 |

## 阻塞问题

| 级别 | 问题 | 证据 | 影响 | 建议 |
|---|---|---|---|---|
| CRITICAL | 模式 B 的 P0 验收口径仍有两套。`feature-spec` 和设计文档要求“客户端 A 用 ID+密码远控客户端 B”，且模式 B 也要看到画面、点击生效、断开；并行编排和实现计划又把 P0 演示钉成“Web 主控 + 密码校验 + 拒连”，Slint 主控贴帧降为可选。 | `feature-spec-mvp.md:34,62,69,141`；`xinchuang-remote-control-design.md:36,57`；`parallel-dev-orchestration.md:33,147`；`ohmydesk-mvp-implementation.md:46,1181,1548,1558` | client-dev 可能实现客户端主控，frontend-dev/integrator 可能只验 Web 主控，验收口径不可判定。 | 立即二选一：1. 若 P0 只演 Web 主控，则回改 feature-spec/design 的 M2 和 §7；2. 若客户端到客户端仍是 P0，则撤销 P-DOC1 降级，并给 Slint 主控输入捕获完整任务。 |
| CRITICAL | WS 信封结构仍跨文档不一致。实现计划和编排采用 `Envelope { from,to,ts,payload }` + `payload.type`，但 feature-spec 和设计文档仍写顶层 `{ type, from, to, payload, ts }`。 | `feature-spec-mvp.md:28-30`；`xinchuang-remote-control-design.md:197-198`；`parallel-dev-orchestration.md:164`；`ohmydesk-mvp-implementation.md:337-341` | mock/frontend/server 可能按不同 JSON 解析；这是协议级分歧，不应留到收尾。 | 将 feature-spec/design 的信封描述改为 `Envelope { from, to, ts, payload }`，消息类型在 `payload.type`。 |
| CRITICAL | 数据库选型仍跨文档不一致。实现计划和项目规范为 MySQL，但 feature-spec/design 仍有 SQLite 残留，且涉及 M4/M5 数据源。 | `feature-spec-mvp.md:98,115`；`xinchuang-remote-control-design.md:99,122,125`；`ohmydesk-mvp-implementation.md:1342-1400` | MCP、审计查询、部署脚本的依赖会被不同 agent 理解成 SQLite 或 MySQL。 | 不建议等 Wave3 清理；在 Wave0 前统一所有需求/设计叙述为 MySQL。 |

## 高风险问题

| 级别 | 问题 | 证据 | 建议 |
|---|---|---|---|
| HIGH | 实现计划仍把跨端工作塞进单个 Task。Task 4.3 同时改 Web 和 Slint client，Task 4.4 同时改 frontend/client，Task 6.2 同时改 server/frontend。 | `ohmydesk-mvp-implementation.md:1171-1194`、`1197-1245`、`1418-1433` | 按 owner 拆成 `4.3a frontend 渲染`、`4.3b client Slint 贴帧(P1/可选)`、`4.4a frontend 输入回传`、`4.4b client 注入`、`6.2a server HTTP`、`6.2b frontend 审计页`。 |
| HIGH | MCP 包管理器仍不统一。编排要求 pnpm，Phase 7 仍写 `npm init` / `npm i`。 | `parallel-dev-orchestration.md:46`；`ohmydesk-mvp-implementation.md:1462-1463` | MCP 线会生成 npm lockfile，与项目约束冲突。 | 改为 `pnpm init` / `pnpm add @modelcontextprotocol/sdk zod`。 |
| HIGH | 收尾清理项引用不准。实现计划写“design §11/§13 的 tauri”，实际 Tauri 残留在设计文档 §12；SQLite 和 §8 消息类型残留也未列全。 | `ohmydesk-mvp-implementation.md:1537`；`xinchuang-remote-control-design.md:99,122,125,193,197,231,238` | integrator 可能漏清理关键残留，导致最终文档仍不一致。 | 收尾项改成明确列表：feature-spec SQLite、design SQLite、design §8 消息、design §12 Tauri。 |

## 中风险问题

| 级别 | 问题 | 证据 | 建议 |
|---|---|---|---|
| MEDIUM | MCP 工具数量叙述仍不够清晰：设计文档写 5 个工具，Phase 7 标题写 4 个只读 tool，目标写“5 个只读 tool（P0 四个）”。 | `xinchuang-remote-control-design.md:70-75,183`；`ohmydesk-mvp-implementation.md:1453-1455` | 不阻断 P0，但会让 mcp-dev 误做 `get_screenshots`。 | 统一写法：P0 只做 4 个；`get_screenshots` 为 P1，不进入 Wave1/MVP 必交。 |
| MEDIUM | 并行隔离策略仍有两种说法：角色表写 worktree，正文又建议同一工作树按目录分工。 | `parallel-dev-orchestration.md:90-99` | 不一定阻塞，但会影响 agent 启动方式。 | 明确推荐一种默认策略；若推荐同一工作树，则角色表“隔离”列改为“目录 owner”。 |
| MEDIUM | `feature-spec` 的 `AuditLog.type` 与实现计划 Rust 字段 `kind` + serde rename 存在命名解释成本。 | `feature-spec-mvp.md:25`；`ohmydesk-mvp-implementation.md:400-407` | JSON 层可以保持 `type`，Rust 层用 `kind` 合理；但需要在契约文档写清。 | 在协议说明中补一句：JSON/API 字段为 `type`，Rust 内部字段名为 `kind`，DB 列名为 `event_type`。 |

## 覆盖与可执行性

- 功能覆盖：M1-M5 的 P0 功能仍都有实现任务覆盖。
- 实现计划可执行性：若以 `ohmydesk-mvp-implementation.md` 作为唯一入口，技术步骤基本可执行。
- 跨文档一致性：未达标，主要卡在模式 B、WS 信封、数据库三项。
- 并行开发边界：`parallel-dev-orchestration.md` 的角色划分方向正确，但实现计划的 Task 粒度仍跨 owner，不适合直接分派给并行 agent。

## 建议的最小修复顺序

1. 先定模式 B P0 口径，并同步 `feature-spec`、设计文档、并行编排和实现计划。
2. 同步 WS 信封结构，所有文档统一为 `payload.type`。
3. 同步 DB 选型，删除 SQLite 残留。
4. 拆分跨端 Task，让每个并行 agent 只拥有自己的目录和交付物。
5. 修正 MCP 包管理器为 pnpm，并把 P0/P1 工具范围写清。

## 放行条件

满足以下条件后可从 Wave0 进入实现：

- 模式 B 验收口径在所有文档一致。
- WS 信封和数据库选型在所有文档一致。
- Phase 4/6 的跨端 Task 已拆成单 owner 子任务。
- MCP 线统一 pnpm，P0 只做四个工具。

