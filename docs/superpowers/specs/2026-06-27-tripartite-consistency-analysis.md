# 需求 × 原型 × 设计 — 三方一致性分析

> ⚠️ **历史分析快照（2026-06-27）**：本文裁决已**全部回流**进 `plans/ohmydesk-mvp-implementation.md`（单一 TDD 入口）+ `parallel-dev-orchestration.md` + `review/`。C-1~C-4 / A-1 / O·G·D 系列均已落实，文中 SQLite/Tauri/旧消息类型描述为**当时快照**，现状以 plan/design/protocol 为准，**勿据本文回改**。
>
> **目的**：把三个事实源放一起做三角对照，暴露分歧、给出裁决与收敛行动。
> **三方**：
> - **需求** = `2026-06-27-feature-spec-mvp.md`（M1–M5 / F-Mx-n / §7 验收）
> - **原型** = `v0/`（Next.js 实现，已评估）
> - **设计** = `crates/protocol` 契约 + `*-design.md` + 实现计划 + 并行编排

---

## 0. 结论 + 裁决原则

**三方在「功能范围」上约 85% 一致，在「数据契约」上仅约 40% 一致（漂移密集），设计内部另有已知文档债。** 原型补全了需求的 UI 形态，但它自造的类型几乎全与 protocol 漂移，且**多做了 2 处需求明确不做的功能**（文件传输、录制标记），**少做了 M2 核心数据面**。

**单一事实源优先级（裁决时谁说了算）：**
- **数据契约**（字段/枚举/结构）：`protocol` > `feature-spec` > `design` > 原型。**原型只是 UI 参考，从不充当契约源。**
- **功能范围**（做不做）：`feature-spec` > `design` > 原型。**原型超出 spec 的功能一律砍**，除非 spec 补议。

---

## 1. 三方一致性矩阵（M1–M5）

判定：🟢 三方一致 ｜ 🟡 形一致但有漂移/缺逻辑 ｜ 🔴 三方有实质分歧

| 模块 | 需求(spec) | 原型(v0) | 设计(protocol/design) | 判定 | 核心分歧 |
|------|-----------|---------|----------------------|:---:|---------|
| **M1 资产** | 注册/采集/心跳/信创标识/列表+抽屉 | 列表+抽屉+信创视觉完整(mock) | EndpointView/EndpointInfo | 🟡 | 原型多 `department`、抽屉展示「临时密码」(设计不下发)；os/arch 扁平 vs 嵌套、缺 linux/other；ram GB vs 字节 |
| **M2 远控** | A/B 发起/截屏/渲染/注入/结束/过场 | UI 壳全有，**帧渲染+键鼠回传缺失**；授权弹窗完整 | ConnectRequest/Frame/Input/AuthResult | 🔴 | 原型**缺**帧渲染/键鼠/模式B拒连态；原型**多**「录制标记」(spec 不录像)；Web 授权弹窗 vs 设计被控端=Slint |
| **M3 批量** | 网格/一键批量截图/截图墙 | 三态动效完整(mock 截图) | ScreenshotReq/Resp | 🟢 | 仅数据未接：`desktop` 静态路径 → `ScreenshotResp.data` |
| **M4 审计** | 连接/操作(聚合计数)/查询/鉴权失败 | 列表+时间线完整，**时间范围筛选有UI无逻辑** | AuditLog 事件流 + Session | 🔴 | 原型**会话聚合视图** vs 设计**事件流**；audit type **三方枚举各不同**；原型多 `transfer`(spec不做)；mode 大小写、result 枚举漂移 |
| **M5 AI** | AI 自然语言问答 + MCP 4 tools | 聊天UI+工具Chip完整，**应答是关键词 mock** | MCP Server + 5 tools，数据不出网 | 🟡 | 原型 AI 是前端 mock vs 设计真 MCP/LLM；展示类型无契约对应(一致) |

---

## 2. 分歧清单与裁决

### 2.1 原型超出需求 → 砍（原型做了 spec 明确不做的）

| # | 分歧 | 三方立场 | 裁决 | 归属 |
|---|------|---------|------|------|
| **O-1** | 审计 timeline 含 `transfer`（文件传输） | 原型有；spec §M2「不做文件传输」；protocol audit type 无 transfer | **砍**：删 transfer 节点，原型 mock 移除 | frontend |
| **O-2** | 远控画面「录制标记」UI | 原型有；spec §M4「纯文本审计、不录像」 | **砍**：删录制标记，避免误导评委以为有录像 | frontend |
| **O-3** | 终端详情抽屉展示「临时连接密码」 | 原型展示；设计 `EndpointView` 故意不下发密码（安全） | **砍**：删密码展示（模式B密码走专门交互，不在资产抽屉暴露） | frontend |

### 2.2 需求/设计有、原型缺 → 补（原型只给壳）

| # | 缺口 | 裁决 | 归属 | 工作量 |
|---|------|------|------|:---:|
| **G-1** | M2 主控画面帧渲染（canvas/`<img data:>` 消费 `frame`） | 补（plan Task 4.3） | frontend+client | 大 |
| **G-2** | M2 键鼠回传（监听+坐标映射+发 `Input`） | 补（plan Task 4.4） | frontend+client | 大 |
| **G-3** | 模式 B 密码错 → 拒连结果态 | 补 UI 分支 | frontend | 小 |
| **G-4** | 审计时间范围筛选（today/3d/7d 有 UI 无逻辑） | 补过滤逻辑或转 `from/to` 参数 | frontend | 小 |
| **G-5** | M5 AI 真实应答（替换关键词 mock） | 接真实 MCP/LLM **或**保留预录降级脚本（spec §M5 允许断网降级） | frontend+mcp | 中/小 |

### 2.3 三方契约漂移 → 统一（以 protocol 为准，前端写适配层）

> 这些**不改 protocol**，前端在适配层消化。protocol 是唯一契约源。

| # | 漂移 | protocol(权威) | 原型 | 适配 |
|---|------|---------------|------|------|
| **D-1** | OS/CPU 结构 | `info.os.kind` / `info.cpu.arch`（嵌套，枚举含 linux/other） | 扁平 `os`/`arch`，缺 linux/other | 适配函数拍平 + fallback |
| **D-2** | 在线态 | `online: bool` | `status: "online"\|"offline"` | bool → badge |
| **D-3** | 内存单位 | `ram` 字节(u64) | `memGb` number | ÷1024³ |
| **D-4** | 时间戳 | `last_seen: i64` epoch | 格式化字符串 | epoch → 相对时间 |
| **D-5** | 命名风格 | snake_case（ts-rs 生成） | camelCase | 适配层映射 |
| **D-6** | 会话模式 | `mode` serde lowercase `"a"/"b"` | 大写 `"A"/"B"` | 统一小写 |
| **D-7** | 审计结构 | `AuditLog[]` 事件流 + `Session` | 会话聚合视图(内嵌 timeline) | 前端按 session_id 聚合，**或 server 加会话聚合接口** |
| **D-8** | result 枚举 | `Session.status: active\|ended\|rejected` | `success\|rejected\|auth_failed`(无 active) | 适配 + 补「进行中」态 |

### 2.4 设计内部不一致 → 清理（design vs spec vs plan 自相矛盾）

| # | 矛盾 | 以谁为准 | 行动 |
|---|------|---------|------|
| **C-1** | **audit type 三方各异**：spec=`connect\|auth_fail\|reject\|screenshot\|input\|disconnect`；design §7=`connect\|screenshot\|click\|disconnect`；原型=`connect\|screenshot\|input\|transfer\|disconnect\|error` | **protocol/spec** | 统一为 spec 集合；design §7 `click`→`input`、删；原型 `transfer` 砍、`error`→`auth_fail/reject` |
| **C-2** | design §5/§10 残留 **SQLite**；feature-spec §M5-4 数据源写 SQLite | 决策 = **MySQL** | 收尾清理 |
| **C-3** | design §12 残留 **Tauri** | 决策 = **Slint** | 收尾清理 |
| **C-4** | design §8 消息类型 `auth_challenge`/`mouse` | protocol = `auth_result`/`input` | design §8 对齐 protocol |

### 2.5 原型合理、设计可纳入 → 议（不是砍而是考虑补设计）

| # | 项 | 说明 | 建议 |
|---|---|------|------|
| **A-1** | `department` 部门字段 | 原型有，protocol 无。对 B 端终端管理（按部门分组/审计「财务部电脑」）**有真实价值**，且 M5 示例问答正是「谁在控财务部电脑」 | **建议 protocol 补 `EndpointInfo.department: Option<String>`**（低成本，增强 B 端叙事）。归 Wave 0 协议冻结一并定 |

---

## 3. 收敛行动项（谁改什么）

**砍原型（frontend，Wave 1）**：O-1 transfer、O-2 录制标记、O-3 密码展示。
**补原型（frontend，Wave 1/2）**：G-1/G-2 M2 帧+键鼠（最大）、G-3 拒连态、G-4 审计时间筛选、G-5 AI 真实/降级。
**适配层（frontend，Wave 1）**：D-1~D-8，尤其 D-7 审计聚合（最重）。
**协议定夺（protocol-owner，Wave 0 一次定死）**：C-1 audit type 统一含 `input`、A-1 议 `department`。
**清理设计债（integrator，Wave 3 收尾）**：C-2 SQLite、C-3 Tauri、C-4 §8 消息类型。

---

## 4. 单一事实源固化建议

1. **protocol 是数据契约的唯一源**：spec/design/原型 与 protocol 冲突时，改后三者，不改 protocol（除非 protocol 本身设计缺陷，如 A-1 department 走正式补议）。
2. **原型不进契约**：v0 的 `lib/*.ts` 类型仅作 UI 起点，接入即被 ts-rs 生成类型 + 适配层取代。
3. **Wave 0 把 C-1/A-1 一次定死**，避免协议二次变更广播四线返工（编排文档已强调）。
4. **收尾统一清理 design 残留**（C-2/C-3/C-4 + 编排已列的 §12 Tauri / SQLite），让 design 与 protocol/决策最终一致。

*配套：[原型评估] 见上次 v0 完整度报告 ｜ [必修项] 见 `2026-06-27-parallel-dev-orchestration.md` §0。*
