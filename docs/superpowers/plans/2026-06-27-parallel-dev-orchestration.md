# OhMyDesk MVP — Agents Team 并行开发编排

> **配套**：实现计划 [2026-06-27-ohmydesk-mvp-implementation.md](./2026-06-27-ohmydesk-mvp-implementation.md)（逐 Task 代码细节）｜ 本文件只管**怎么并行、谁做什么、何时集成、必修项归属**。
> **死线**：2026-06-28 中午。**编排原则**：协议先冻结 → 四线并行开发 → 按 M1→M5 滚动集成 → 收尾彩排。

---

## 0. 评审必修项清单（两份评审去重，开发时必须应用）

> 来源：可执行性评审 + 完整性评审（architect subagent）。每条标注【归属线】【Phase】，开发 agent 执行对应 Task 时**同时应用修正**。

### 🔴 阻塞（不改则编译/建表失败）
| 编号 | 修正 | 归属 | Phase |
|------|------|------|-------|
| **B-DB1** | `audit_logs.type` 是 MySQL 保留字 → 列名改 `event_type`（schema.sql + INSERT 同步） | server | 6 |

### 🟠 必补（不补则 §7 某条在开发期/演示期断链）
| 编号 | 修正 | 归属 | Phase |
|------|------|------|-------|
| **M-SRV1** | DB 连接失败**降级为 `Option<Db>`=None**，审计落库 best-effort 跳过 + 告警 log；实时链路 M1/M2/M3 不受 MySQL 影响 | server | 6 |
| **M-SRV2** | server router 挂 `CorsLayer::permissive()`（admin :5173 跨端口 fetch `/api/*`） | server | 1/6 |
| **M-SRV3** | `http.rs` 的 axum `State` 同时持 `Arc<Hub>`（→`reg.views()`）+ `Db`；`/api/endpoints` 读注册表、`/api/audit\|sessions` 读 DB | server | 6 |
| **M-SRV4** | hub 转发 `Message::Input` 时对该 session 的 `InputAggregator.bump()`；`session_end` 落聚合 text（否则审计输入计数恒 0） | server | 6 |
| **M-CLI1** | `net.rs` 用 **mpsc 出站泵**（与 server `handle_socket` 同构）：`out_tx` 统一发注册/心跳/下行回发；**不要**把 `write` move 进心跳 task（否则 Phase 4 回发 Frame/Input 撞所有权墙、推倒重来） | client | 2 |
| **M-CLI2** | `net.rs` 外层包**断线重连循环**（断开 sleep 3s 重连重注册），否则演示中途掉线要手动重启 agent | client | 2 |
| **M-CLI3** | 补 `rand_6()`/`now()`/`cur_ram()` 实现 + `rand="0.8"` 依赖（计划示例引用了但未定义） | client | 2 |

### 🟡 准必补（design 明确要求 / 影响演示完成度）
| 编号 | 修正 | 归属 | Phase |
|------|------|------|-------|
| **P-SRV5** | server `ServeDir` 托管 `apps/admin-web/dist` + SPA fallback（design §11「一个内网 URL 给评委」） | server | 收尾 |
| **P-CLI4** | 截屏**等比缩放**（非写死 1280×720 拉伸）；`Frame` 带真实 `w/h`；注入按 `real_w/frame_w` 缩放（非写死 1280），否则非 16:9 屏坐标偏 | client | 4 |
| **P-DOC1（用户裁决修正 2026-06-27）** | **§7 第 3 条模式 B = client→client（P0，F-M2-2/4/5）**：client 主控端 Slint 发起 UI + 贴帧 + 键鼠捕获**必做**（plan Task 4.6）；Web 主控降为 Slint 翻车**兜底**，非替代 | client+integrator | 4 |
| **P-MCP1** | 锁 `@modelcontextprotocol/sdk` 版本，按该版核对 `tool`/`registerTool` 签名（最新版签名可能与计划示例不符，TS 侧最易翻车点） | mcp | 7 |
| **P-MCP2** | `/api/endpoints` 契约 = 返回 `EndpointView[]` 裸数组，与 MCP `all.filter` 对齐 | server+mcp | 6/7 |

### 🟢 协议层（Wave 0 一次定死，避免后续广播式返工）
| 编号 | 修正 | Phase |
|------|------|-------|
| **W0-1** | `register_ack` 闭环：server `Register` 分支回发 `RegisterAck`（契约不留死类型） | 1 |
| **W0-2** | `audit` 事件 type 枚举**定死含 `input`**：`connect\|auth_fail\|reject\|screenshot\|input\|disconnect`，三端统一 | 0 |
| **W0-3** | 修正注释：`#[serde(tag="type")]` 是**内部 tag**，type 在 `payload` 对象内（非信封顶层）；前端按 `env.payload.type` 判别 | 0 |

### ⚪ 收尾文档债 / 一致性（不阻断 demo）
- 清理残留：design **§12「Tauri」**、§5/§10 **SQLite** 字样、feature-spec 的 SQLite、design §8 消息类型对齐 protocol。
- 命令统一 **pnpm**（计划部分写了 npm，与 user.md 冲突）。
- ts-rs 双导出：`#[ts(export)]` 会额外生成 `crates/protocol/bindings/` → `.gitignore` 或去掉 `#[ts(export)]`。
- sqlx 加 `tls-rustls` feature（连生产内网 MySQL；本地 docker 库可暂免）。
- **MSRV 警示**：sysinfo 0.39 需 Rust ≥1.95、enigo 0.6 需 ≥1.87；**loongarch64 交叉编译须验证工具链 ≥1.95，现场 demo 机优先 x86_64/aarch64**。

### 可选增强（不影响 §7 达成）
截图墙 N/M loading+3s 超时聚合 ｜ admin ws `onclose` 重连 ｜ 信创 Lucide 图标映射 ｜ 端口/地址走环境变量 ｜ 授权弹窗倒计时（spec 无要求）。

---

## 1. 并行可行性与依赖图

**能并行的前提：协议契约冻结。** 冻结后，server/client/admin/mcp 四线可基于「契约 + mock」独立开发各自模块，集成时才需对方真实就绪。

```
                    ┌─────────────────────────┐
                    │  Wave 0: protocol 冻结    │  ← 硬 barrier，阻塞一切
                    │  Rust 类型 + ts-rs TS 类型 │
                    └────────────┬─────────────┘
          ┌──────────────┬───────┴───────┬──────────────┐
          ▼              ▼               ▼              ▼
   ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐
   │ server-dev │ │ client-dev │ │frontend-dev│ │  mcp-dev   │   Wave 1（并行）
   │ 中枢/瓶颈   │ │ 采集/远控  │ │ v0接入/mock │ │ mock HTTP  │
   └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └─────┬──────┘
         └──────────────┴───────┬───────┴──────────────┘
                                ▼
                    ┌─────────────────────────┐
                    │ Wave 2: 滚动集成          │  integrator 主导
                    │ I1 M1→I2 M2→I3 M3/4→I4 M5 │
                    └────────────┬─────────────┘
                                 ▼
                    ┌─────────────────────────┐
                    │ Wave 3: 收尾/托管/彩排    │
                    └─────────────────────────┘
```

**运行时依赖（集成期才生效）**：client→server（注册）、admin→server（WS+HTTP）、mcp→server（HTTP）。
**瓶颈**：server 是所有集成的对手方 → **server-dev 在 Wave 1 内优先级最高**，须最先交付可联调的骨架（注册+列表+WS 路由），解锁其他线集成。

---

## 2. Agents Team 角色

| Agent 角色 | 负责范围（crate/app） | 依赖 | 工作模式（同一工作树） |
|-----------|---------------------|------|------|
| **protocol-owner** | `crates/protocol`（契约 + ts-rs 导出） | 无（地基） | 主线串行 |
| **server-dev** | `crates/server` 全部 + `scripts/db/schema.sql` | protocol | 目录 owner |
| **client-dev** | `crates/client` 全部（Slint/采集/网络/截屏/注入） | protocol | 目录 owner |
| **frontend-dev** | `apps/admin-web`（v0 接入/store/ws/5 页） | protocol（TS 类型） | 目录 owner |
| **mcp-dev** | `apps/mcp`（5 tool） | protocol/HTTP 契约 | 目录 owner |
| **integrator** | 跨端联调 + 集成里程碑验收 + 收尾 + 文档残留 | 全部 | 主线 |

> **隔离策略（已定死单一方案）**：四线在**同一工作树按目录 owner 分工**（server=`crates/server`、client=`crates/client`、frontend=`apps/admin-web`、mcp=`apps/mcp`，互不交叉写入），**不使用 worktree 并行写入**。本仓库是同一 Cargo workspace，跨 crate 编译共享根 `Cargo.toml`，由 protocol-owner 在 Wave 0 一次配齐 members/deps。**Phase 4 远控链路跨端耦合高，Task 4.3/4.4/4.5/4.6 由 integrator 串行落地**，不拆多 agent 并行（消除跨 owner 任务冲突）。

---

## 3. 波次编排

### Wave 0 — 协议冻结（串行，protocol-owner，~30min）
**唯一目标**：把契约一次定全定死，之后**禁止随意改协议**（改一处广播四线返工）。

- 实现计划 **Phase 0 全部**（Task 0.1~0.4）。
- 应用 **W0-1/W0-2/W0-3**：`RegisterAck` 保留、audit type 枚举含 `input`、内部 tag 注释修正。
- 根 `Cargo.toml` 一次配齐四线所需 `workspace.dependencies`（含 `rand`、sqlx features 等），避免 Wave 1 各线反复改根清单冲突。
- **交付物（barrier）**：`cargo test -p protocol` 全绿 + `apps/admin-web/src/lib/types/*.ts` 生成。
- **冻结公告**：protocol-owner 在此点通知四线「契约已冻结，可开工」。

### Wave 1 — 四线并行开发（protocol 冻结后）

> 各线把自己模块做到「单元可测 / mock 可跑」，**不等对方**。集成留 Wave 2。

**① server-dev（优先级最高，瓶颈）** — 实现计划 Phase 1 + Phase 4(server 侧) + Phase 5(server 侧) + Phase 6
- Phase 1：registry / hub / 列表广播（应用 **W0-1** register_ack、**M-SRV2** CORS）。
- Phase 4 server 侧：session 建立/鉴权 A/B、Input/Frame 路由（应用 **M-SRV4** bump 埋点）。
- Phase 5 server 侧：ScreenshotReq 广播 + Resp 聚合。
- Phase 6：db.rs（**M-SRV1** DB 降级 None、**B-DB1** event_type）、audit.rs、http.rs（**M-SRV3** State 双源、**P-MCP2** 裸数组契约）。
- **里程碑产出**：server 起得来，可被 websocat/测试脚本驱动跑通注册→列表→会话路由→截图广播→审计落库。

**② client-dev** — 实现计划 Phase 2 + Phase 4(client 侧)
- Phase 2：asset.rs 采集、net.rs（**M-CLI1** mpsc 泵、**M-CLI2** 重连、**M-CLI3** helper）、最小 Slint UI。
- Phase 4 client 侧：capture.rs（**P-CLI4** 等比缩放 + 真实 w/h）、inject.rs（坐标按 frame_w 缩放）、Slint 贴帧、授权弹窗联动；**client→client 主控端（Task 4.6，P-CLI5 P0）：Slint 发起面板 + 键鼠捕获回传**。
- **里程碑产出**：client 能采真实硬件、反连注册、被控截屏推帧、收 input 注入（先用本地 server 桩或 server-dev 就绪后联调）。

**③ frontend-dev** — 实现计划 Phase 3 + Phase 4/5/6 的 admin 页
- 脚手架（**pnpm**）+ ws.ts + store；按 skill `v0-to-project` 接入 v0 五页（Assets/Grid/Remote/Audit/Assistant）。
- **先用 mock 数据**渲染五页（不阻塞于 server），再在 Wave 2 接真实 WS/HTTP。
- 审计页 fetch 依赖 **M-SRV2** CORS（server 线提供）。
- **里程碑产出**：五页 mock 可视、组件接 store、类型用 ts-rs 生成物。

**④ mcp-dev** — 实现计划 Phase 7.1
- 锁 SDK 版本（**P-MCP1**），4 个只读 tool 先打 **mock HTTP**（按 `/api/*` 契约）开发，inspector 验证结构。
- **里程碑产出**：MCP server 起得来，tool 结构正确（接 mock，Wave 2 换真实 server HTTP）。

### Wave 2 — 滚动集成（integrator 主导，各 dev 配合）

> 按 §7 验收顺序滚动，每个里程碑是一次跨线联调 + 验收。前一个绿了再推下一个。

| 里程碑 | 集成内容 | 参与线 | 验收（§7） |
|--------|---------|--------|-----------|
| **I1 = M1** | client 反连注册 + admin 接 WS 收 endpoint_list 渲染真实终端 + 抽屉 | server+client+frontend | §7-1：2+ 台真实硬件+信创标识+离线 |
| **I2 = M2** | 模式 A 远控闭环（授权→帧→注入→断开）；模式 B = **client→client**（A 的 Slint 主控控 B：授权/密码校验/拒连/键鼠注入，P-DOC1 P0）；Web 主控为兜底 | 三线 | §7-2、§7-3 |
| **I3 = M3+M4** | 一键批量截图墙；远控产生审计（**M-SRV4** input 计数）+ 审计页查询 | server+client+frontend | §7-4、§7-5 |
| **I4 = M5** | mcp 换真实 server HTTP；AI 自然语言问答（录降级视频） | mcp+frontend | §7-6 |

**集成期协议变更纪律**：若集成暴露协议缺陷，由 protocol-owner 统一改 + 重导 TS + 通知四线，**禁止各线私改协议类型**。

### Wave 3 — 收尾（integrator）
- **P-SRV5** server ServeDir 托管 admin/dist（一个内网 URL）。
- 文档残留清理（design §12 Tauri / §5·§10 SQLite / feature-spec SQLite / §8 消息类型）。
- 联调全链路 6 条 §7 逐条过；深色主题+信创标识美化；彩排 2 遍 + 兜底（客户端打包翻车→浏览器模拟；AI 断网→播录像）。

---

## 4. 集成交接契约（减少联调摩擦）

各线在 Wave 1 必须**严格遵守**这些跨线接口，集成才不返工：

1. **WS 端点**：`ws://<server>:8765/ws`，统一 `Envelope{from,to,ts,payload}`，`payload` 内部 tag `type`。
2. **连接登记**：每个连接首条消息的 `from` 即其 id；`admin-*` 前缀触发 server 推 `endpoint_list`。
3. **HTTP 契约**（server 线提供，mcp/frontend 消费）：
   - `GET /api/endpoints` → `EndpointView[]`（裸数组，读内存注册表）
   - `GET /api/sessions` → 进行中会话数组
   - `GET /api/audit?endpoint=&from=&to=&result=` → `AuditLog[]`（读 MySQL）
   - 全部挂 `CorsLayer::permissive()`。
4. **帧格式**：`Frame{session_id, data(base64 JPEG), w, h, seq}`；`w/h` 是**实际帧尺寸**（等比缩放后），注入侧据此换算。
5. **TS 类型**：frontend/mcp 一律 import `apps/admin-web/src/lib/types/`，**不手写实体**。

---

## 5. 风险与串行回退点

| 风险 | 触发 | 回退 |
|------|------|------|
| **server 瓶颈拖慢全线** | server-dev 未及时交付可联调骨架 | server 线最高优先；I1 联调可用 websocat 桩先验 client/admin 各自 |
| **远控集成（I2）最高风险** | 帧/注入/坐标/会话状态多端耦合；**client→client Slint 主控键鼠捕获是新风险点** | 预留缓冲；先模式 A（Web 主控）打通全链路（截屏/注入/会话/审计），再加 client 主控端（Task 4.6）；**Slint 主控翻车则 Web 主控兜底演示模式 B**（被控+server 链路一致，鉴权/拒连/审计照样达成） |
| **协议中途要改** | 集成暴露契约缺陷 | 只走 protocol-owner 统一改 + 重导；Wave 0 尽量定全（W0-*） |
| **MySQL 拖垮实时链路** | DB 连不上 | **M-SRV1** 已要求降级 None；演示前确认 docker 库起着 |
| **MCP SDK 版本漂移** | npm 装最新签名不符 | **P-MCP1** 锁版本；I4 失败有录像兜底，且 M1-M4 不依赖 AI |
| **loongarch64 工具链 <1.95** | 交叉编译 sysinfo 失败 | 现场 demo 机优先 x86_64/aarch64；交叉编译仅作架构论证 |

---

## 6. 启动方式（二选一）

- **A. 真并行（Agents Team）**：Wave 0 主线跑完冻结协议 → Wave 1 派 4 个后台 dev agent（server/client/frontend/mcp）并行 → integrator 主线按 I1~I4 滚动集成。适合最大化吞吐，但需主线协调集成。
- **B. 半并行（务实稳妥）**：Wave 0 + server-dev 主线先行（瓶颈），client/frontend 用 1-2 个并行 agent 跟进，mcp 最后接。集成风险更可控，适合死线冲刺。

> 推荐 **B**：远控集成耦合密集，server 不就绪时其他线集成是空转；先把「protocol→server→client→admin」主链打通（I1/I2），再并行收 M3/M4/M5。

---

*下一步：确认启动方式（A/B）→ 先执行 Wave 0 冻结协议（含 W0-* 修正）→ 按编排推进。*
