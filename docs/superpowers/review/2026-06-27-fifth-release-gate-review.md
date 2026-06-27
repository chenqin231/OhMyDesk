# docs/superpowers 第五轮放行条件复核

## 结论

当前判定：**Go for Wave0**。

你给出的结论基本正确：第四轮提出的 4 项放行条件已经全部清掉。`feature-spec` §0.3 已改为 `Envelope { from, to, ts, payload }` + `payload.type`；MCP Phase 7 已改用 pnpm；并行策略已定为同一工作树目录 owner，且 Phase 4 由 integrator 串行落地；`tripartite-consistency-analysis.md` 已标注为历史快照，禁止据此回改。

剩余问题均为 MEDIUM，不阻断 Wave0 TDD，但建议在 Wave0 冻结公告或收尾阶段明确处理。

## 第四轮放行条件复核

| # | 放行条件 | 复核结果 | 判定 |
|---|---|---|---|
| 1 | `feature-spec` §0.3 WS 信封改为 `{ from, to, ts, payload }` + `payload.type` | `feature-spec-mvp.md:29` 已写明 `payload` 内部 tag `type`，并对齐 protocol `Envelope`。 | 通过 |
| 2 | MCP Phase 7 命令改为 pnpm | `ohmydesk-mvp-implementation.md:1494` 已改为 `pnpm init && pnpm add @modelcontextprotocol/sdk zod`。 | 通过 |
| 3 | 并行策略只能保留一种 | `parallel-dev-orchestration.md:90-99` 已改为同一工作树目录 owner，并明确不使用 worktree 并行写入；Phase 4 跨端远控链路由 integrator 串行落地。 | 通过 |
| 4 | `tripartite-consistency-analysis.md` 标注历史状态 | 文件头已标注“历史分析快照”，并说明 SQLite/Tauri/旧消息类型描述是当时快照，现状以 plan/design/protocol 为准。 | 通过 |

## 关键一致性复核

| 维度 | 当前状态 | 判定 |
|---|---|---|
| WS 信封 | `feature-spec`、`design`、`orchestration`、`plan` 均统一为 `Envelope { from, to, ts, payload }` + `payload.type`。 | 一致 |
| 模式 B | `feature-spec` 保持 client→client；`plan` 增加 P-CLI5/Task 4.6；`orchestration` I2 改为 client→client P0，Web 主控仅兜底。 | 一致 |
| 数据库 | `design`/`feature-spec` 的实体性 SQLite/Tauri/旧消息类型残留已清；剩余 grep 命中是历史说明或清理任务描述。 | 一致 |
| 并行边界 | 目录 owner + Phase 4 integrator 串行，避免跨 owner 写同一远控链路。 | 可执行 |

## 非阻塞跟踪项

| 级别 | 问题 | 证据 | 建议 |
|---|---|---|---|
| MEDIUM | MCP 工具数量仍有表述噪音：部分位置写 `5 tool`，执行任务写 P0 四个。 | `parallel-dev-orchestration.md:96`、`xinchuang-remote-control-design.md:183`、`ohmydesk-mvp-implementation.md:1484-1486` | Wave0 公告钉死：MVP P0 只做 4 个工具；`get_screenshots` 是 P1/预留。 |
| MEDIUM | `feature-spec` 只写 JSON/API 层 `AuditLog.type`，未解释 Rust `kind` 与 DB `event_type` 的三层映射。 | `feature-spec-mvp.md:25`；`design`/`plan` 已解释 `event_type` 和 Rust `kind`。 | 可在 Wave0 协议冻结公告补一句：JSON 字段 `type`，Rust 字段 `kind`，DB 列 `event_type`。 |

## 放行建议

可以进入 **Wave0 TDD / Phase 0 protocol**。

进入前建议在冻结公告中明确两句话：

1. MCP MVP 只交付 4 个 P0 工具，`get_screenshots` 不阻断 demo。
2. 审计类型三层命名：JSON/API = `type`，Rust = `kind`，MySQL = `event_type`。

