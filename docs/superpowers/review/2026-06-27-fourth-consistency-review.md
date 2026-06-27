# docs/superpowers 第四轮一致性评审

## 结论

当前判定：**NoGo（仅剩少量文档一致性阻塞）**。

本轮更新已把“模式 B = client→client P0，Web 主控仅兜底”修正回主线，`design` 的 SQLite/Tauri/旧消息类型残留也已清理，`plan` 已补 Task 4.6 覆盖 Slint 主控端发起、贴帧和键鼠捕获。整体已经接近可执行状态。

但仍有 1 个协议级阻塞：`feature-spec` 的 WS 信封仍写旧结构 `{ type, from, to, payload, ts }`，与 `design`、`orchestration`、`plan` 的 `Envelope { from, to, ts, payload }` + `payload.type` 冲突。协议冻结前必须统一。

## 已修复项

| 项 | 当前状态 | 判定 |
|---|---|---|
| 模式 B 被错误降级为 Web 主控兜底 | `plan` 增加 P-CLI5 和 Task 4.6；`orchestration` 的 P-DOC1/I2/风险说明均改为 client→client P0，Web 主控只作兜底。 | 已修复 |
| `design` SQLite/Tauri/旧消息类型残留 | `design` §7/§8 已对齐 MySQL、`AuditType`、`payload.type` 信封；路线图已回到 Slint。 | 已修复 |
| `feature-spec` SQLite 数据源 | F-M4-3/F-M5-4 已改为 MySQL。 | 已修复 |
| `mock-api-contract` I2 口径 | I2 已改为 client→client P0，Web 主控兜底。 | 已修复 |
| Phase 4 对模式 B 的实现覆盖 | Task 4.3 Step 2 标为 P0，Task 4.6 覆盖 Slint 发起面板、画面消费、键鼠回传。 | 已修复 |

## 阻塞问题

| 级别 | 问题 | 证据 | 影响 | 建议 |
|---|---|---|---|---|
| CRITICAL | `feature-spec` 的 WS 信封仍是旧结构 `{ type, from, to, payload, ts }`，而其他执行文档已统一为 `Envelope { from, to, ts, payload }` 且消息类型在 `payload.type`。 | `feature-spec-mvp.md:28-30`；`xinchuang-remote-control-design.md:195-198`；`parallel-dev-orchestration.md:164`；`ohmydesk-mvp-implementation.md:337-341` | protocol-owner 若按需求文档写，会做顶层 `type`；frontend/server 若按 plan 写，会做 `payload.type`，序列化契约不兼容。 | 将 `feature-spec` §0.3 改为：统一信封 `{ from, to, ts, payload }`，`payload` 内部 tag `type` 判别消息，前端按 `env.payload.type` 判别。 |

## 高风险问题

| 级别 | 问题 | 证据 | 建议 |
|---|---|---|---|
| HIGH | MCP 包管理器仍与项目约束冲突。编排要求 pnpm，Phase 7 仍写 `npm init -y && npm i ...`。 | `parallel-dev-orchestration.md:46`；`ohmydesk-mvp-implementation.md:1493-1494` | 改为 `pnpm init` / `pnpm add @modelcontextprotocol/sdk zod`，避免生成 npm lockfile。 |
| HIGH | Phase 4/6 仍有跨 owner 任务。Task 4.3 同时改 Web 和 client；Task 4.4 同时改 frontend 和 client；Task 4.5 同时改 client/server/frontend。 | `ohmydesk-mvp-implementation.md:1173-1196`、`1199-1258`、`1261-1283` | 若真要多 agent 并行，应拆成 owner 子任务；若主线串行执行，可以保留但需在编排里明确“Phase 4 由 integrator 串行落地”。 |
| HIGH | `orchestration` 的隔离策略仍有两种说法：角色表写 worktree，正文又建议同一工作树按目录分工。 | `parallel-dev-orchestration.md:90-99` | 选择一种默认策略。若采用同一工作树，角色表“隔离”列改为“目录 owner”。 |

## 中风险问题

| 级别 | 问题 | 证据 | 建议 |
|---|---|---|---|
| MEDIUM | MCP 工具数量表述仍混用“5 个只读 tool”和“P0 四个”。 | `xinchuang-remote-control-design.md:75,183`；`ohmydesk-mvp-implementation.md:1484-1486`；`parallel-dev-orchestration.md:96,137` | 明确写成：P0 四个工具，P1 `get_screenshots`，`apps/mcp` 目录预留第五个。 |
| MEDIUM | `tripartite-consistency-analysis.md` 仍保留 SQLite/Tauri/旧消息类型等旧分歧描述。 | `tripartite-consistency-analysis.md:75-77,93,102` | 若该文件作为历史评审可保留，但文件头应标注“历史分析，已由后续 plan/orchestration 修正”；否则同步更新，避免新 agent 误读。 |
| MEDIUM | `feature-spec` 的 `AuditLog.type`、Rust `AuditLog.kind`、DB `event_type` 三层命名关系只在 design/plan 中解释，需求侧未解释。 | `feature-spec-mvp.md:25`；`xinchuang-remote-control-design.md:193`；`ohmydesk-mvp-implementation.md:402-408` | 在 `feature-spec` §0.2 加一句：JSON/API 字段为 `type`，Rust 内部字段名为 `kind`，DB 列名为 `event_type`。 |

## 可执行性判断

- 功能覆盖：M1-M5 P0 均有任务覆盖；模式 B 已恢复到 client→client P0。
- 架构可执行性：星型 Relay、protocol 单一事实源、Transport/Adapter、DB 降级策略均合理。
- 并行可执行性：Wave0 后四线并行方向成立，但 Phase 4 远控链路耦合高，建议半并行：server/client 主链先行，frontend/mcp 并行跟进，I2 由 integrator 串行验收。
- 当前阻塞：仅 WS 信封跨文档不一致达到 CRITICAL；修正后可进入 Wave0。

## 放行条件

满足以下 4 项后可进入 Wave0 TDD：

1. 修正 `feature-spec` §0.3 WS 信封为 `Envelope { from, to, ts, payload }` + `payload.type`。
2. 修正 MCP Phase 7 命令为 pnpm。
3. 明确并行策略：同一工作树目录 owner，或 worktree；不要两套并存。
4. 给 `tripartite-consistency-analysis.md` 标注历史状态，或同步更新旧分歧。

---

## 放行条件落实（2026-06-27，本会话修正）→ **Go**

第四轮 4 项放行条件已全部完成，具备进入 Wave0 条件：

| # | 放行条件 | 落实 |
|---|---------|------|
| 1 (CRITICAL) | feature-spec §0.3 信封改 `{from,to,ts,payload}` + payload.type | ✅ `feature-spec-mvp.md:29` 已改（含「对齐 protocol `Envelope`」） |
| 2 (HIGH) | MCP Phase 7 命令改 pnpm | ✅ Task 7.1 → `pnpm init && pnpm add @modelcontextprotocol/sdk zod` |
| 3 (HIGH) | 并行策略定死（不两套并存） | ✅ 角色表「工作模式」列 + §2「同一工作树目录 owner，不用 worktree」；**Phase 4 由 integrator 串行落地 Task 4.3/4.4/4.5/4.6**（同时消除 HIGH 跨 owner 任务冲突） |
| 4 (文档) | tripartite 标注历史状态 | ✅ 文件头加「历史快照，裁决已回流，勿据此回改」 |

**剩余 MEDIUM（不阻断 Wave0，留 Wave0 公告/收尾）**：MCP 工具数量统一表述（P0 四个 + P1 `get_screenshots`）；feature-spec §0.2 补 `type`(JSON) / `kind`(Rust) / `event_type`(DB) 命名注解。

**判定更新：NoGo → Go。** 可从 Wave 0 = Phase 0 protocol 进 TDD。

