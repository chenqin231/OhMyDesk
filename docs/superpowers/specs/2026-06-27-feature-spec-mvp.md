# OhMyDesk 功能需求规格说明书（MVP）

> **范围**：demo MVP（死线 2026-06-28 中午），细化设计文档 5 大模块的核心功能为可验收的功能点。
> **配套**：[产品设计](./2026-06-27-xinchuang-remote-control-design.md) ｜ 规范 `.agent/user.md`
> **优先级**：`P0` = demo 必须闭环（不做=演示断链）；`P1` = 时间富余增强。
> **功能编号**：`F-M{模块}-{序号}`，便于 tasks 拆解与追踪。

---

## 0. 全局共享契约（所有模块依赖，单一事实源）

### 0.1 端与角色
| 端 | 角色 | 技术 |
|----|------|------|
| Agent 客户端 | 既是被控端，也能作主控端 | Slint + Rust |
| 管理 Web 端 | 上帝视角：看终端、批量、远控、审计、AI 问答 | React |
| 服务端 Relay | 中枢：注册表/鉴权/路由/转发/审计落库 | axum + Rust |
| MCP Server | 把管控数据以 MCP 工具暴露给 AI | TS 薄层 |

### 0.2 核心数据实体
```
Endpoint { id, name(使用人), ip, mac, os{name,type}, cpu{model,cores,arch},
           ram{total,used}, gpu{model,vram}, online, lastSeen, password(临时6位), agentVersion }
Session  { id, mode(A|B), fromId, toId, startAt, endAt, status(active|ended|rejected), durationSec }
AuditLog { id, sessionId, ts, actorId, type(connect|auth_fail|reject|screenshot|input|disconnect), text }
```

### 0.3 消息信封（WS）
统一 `{ type, from, to, payload, ts }`。MVP 消息类型：
`register / register_ack / heartbeat / endpoint_list / connect_request / auth_result / connect_ack / reject / frame / input / screenshot_req / screenshot_resp / session_end`

### 0.4 两种远程模式（贯穿 M2/M3/M4）
- **模式 A**：管理 Web 端 → 客户端（管理员控员工终端）
- **模式 B**：客户端 → 客户端（点对点协助，输入对方 ID+密码）

---

## M1 终端资产管理

**目标**：内网每台终端装 Agent，反连服务端注册并持续上报硬件资产，管理端实时可视。

| 功能 | 优先级 | 描述 / 输入 → 处理 → 输出 | 消息·数据 |
|------|:---:|---|---|
| **F-M1-1 Agent 注册上线** | P0 | Agent 启动 → 反连服务端 WS → 生成本机 6 位临时密码 → 注册。服务端存入内存注册表。 | `register` → `register_ack`；写 Endpoint |
| **F-M1-2 硬件资产采集** | P0 | Agent 用 sysinfo 采 CPU(型号/核数/arch)、内存、GPU(型号/显存)、IP、MAC，随注册与周期上报。 | `register`/`heartbeat` payload |
| **F-M1-3 心跳与在线态** | P0 | Agent 每 5s 心跳；服务端超时(如 15s)未收则标记离线。 | `heartbeat`；更新 online/lastSeen |
| **F-M1-4 信创标识识别** | P0 | 由 os.name/cpu.arch 推断信创标识（麒麟/统信/Windows × 龙芯LoongArch/鲲鹏aarch64/x86），打标签。 | Endpoint.os/cpu 派生 |
| **F-M1-5 管理端终端列表** | P0 | 管理端表格：状态/使用人/IP/OS(信创图标)；点行 → 右侧抽屉看 CPU/内存/GPU/MAC 详情。 | `endpoint_list` 推送 |

**MVP 验收**：起服务端 + ≥2 个 Agent → 管理端列表显示 2 台在线、含真实硬件 + 信创标识；杀掉一个 Agent → 15s 内变离线。
**不做**：资产历史趋势、自定义分组、Agent 自动升级。

---

## M2 远程控制（基础，demo 核心）

**目标**：打通 A/B 两种模式的"授权 → 连接 → 画面 → 操作 → 断开"闭环。

| 功能 | 优先级 | 描述 / 流程 | 消息·数据 |
|------|:---:|---|---|
| **F-M2-1 模式A 发起（Web→客户端）** | P0 | 管理端点终端"远程" → 服务端转发请求 → 目标客户端弹窗授权 → 通过则建会话。 | `connect_request`(mode=A) → `auth_result` → `connect_ack`；建 Session |
| **F-M2-2 模式B 发起（客户端→客户端）** | P0 | 主控客户端输入对方 ID+密码 → 服务端校验密码 → 正确则目标弹窗授权 → 建会话；密码错返回失败。 | `connect_request`(mode=B) → `auth_result`/`reject`；建 Session |
| **F-M2-3 被控端截屏帧推送** | P0 | 会话建立后被控端 xcap 周期截屏(默认 2–3fps) → 编码 → 推帧。 | `frame` payload(base64/JPEG, w, h, seq) |
| **F-M2-4 主控端画面渲染** | P0 | 主控端收帧渲染：Web 端 canvas/`<img>`；Slint 端 `SharedPixelBuffer→Image::from_rgba8`。 | 消费 `frame` |
| **F-M2-5 键鼠回传与注入** | P0 | 主控端捕获鼠标坐标/点击/按键 → 回传 → 被控端 enigo 注入（锁 X11）。坐标按被控屏尺寸映射。 | `input` payload(type, x/y/key) |
| **F-M2-6 会话结束** | P0 | 任一端点"断开"或掉线 → 关闭会话、停止推帧、回写 endAt/durationSec。 | `session_end`；更新 Session |
| **F-M2-7 连接中过场态** | P1 | 发起后"协商中…"loading；被控端常驻"当前被 XX 远程"提示条（可治理感）。 | UI 态 |

**MVP 验收**：模式 A 与模式 B 各跑通一次——授权弹窗 → 看到对端实时画面 → 鼠标点击在对端生效 → 断开。密码错误时模式 B 拒绝连接。
**不做**：多显示器、文件传输、剪贴板、画质自适应、真 P2P。

---

## M3 管理平台批量

**目标**：管理端"一眼看全网屏幕"，制造主视觉记忆点。

| 功能 | 优先级 | 描述 / 流程 | 消息·数据 |
|------|:---:|---|---|
| **F-M3-1 在线终端网格视图** | P0 | 管理端以卡片网格展示所有在线终端（名称/IP/OS/状态点）。 | 复用 `endpoint_list` |
| **F-M3-2 一键批量截图** | P0 | 点"批量截图" → 服务端向所有在线 Agent 广播截图指令 → 各 Agent 截一帧回传。 | `screenshot_req`(广播) → `screenshot_resp` |
| **F-M3-3 截图墙展示** | P0 | 收齐的截图以缩略图墙呈现；点缩略图可放大 / 直接发起远控。 | 消费 `screenshot_resp` |
| **F-M3-4 截图降级开关** | P1 | 可调分辨率/质量（720p + JPEG q0.6），弱网兜底。 | req 参数 |

**MVP 验收**：点一次"批量截图" → 数秒内看到所有在线终端当前屏幕缩略图墙；点其一可放大。
**不做**：定时巡检截图、批量脚本下发、批量唤醒(WoL)。

---

## M4 会话审计（纯文本）

**目标**：每次远控可追溯——谁、何时、控了谁、做了什么（文本，不录像）。

| 功能 | 优先级 | 描述 | 消息·数据 |
|------|:---:|---|---|
| **F-M4-1 连接记录落库** | P0 | 建会话/拒绝/结束时写一条：发起方、目标、模式、起止、时长、结果。 | 写 AuditLog(connect/reject/disconnect) |
| **F-M4-2 操作记录落库** | P0 | 会话内关键事件落文本：发起截图、断开；键鼠**按会话聚合计数**（"输入操作 N 次"），不逐条爆量。 | 写 AuditLog(screenshot/input) |
| **F-M4-3 审计列表查询** | P0 | 管理端审计页：列表 + 按终端/时间/结果筛选；点会话看其操作时间线。 | 读 SQLite |
| **F-M4-4 鉴权失败留痕** | P1 | 模式 B 密码错误记 `auth_fail`，体现"未授权可追溯"。 | 写 AuditLog(auth_fail) |

**MVP 验收**：完成 1 次远控后，审计页出现该连接记录（含起止/时长）+ 至少"发起截图/断开"等操作文本；可按终端筛选。
**不做**：录屏/视频回放、屏幕水印、防篡改链、合规报表导出。

---

## M5 MCP Server（AI 时代亮点）

**目标**：平台管控数据以标准 MCP 工具暴露，管理员用 AI 自然语言查全网态势；数据不出网。

| 功能 | 优先级 | 描述（MCP 工具 = 只读） | 数据源 |
|------|:---:|---|---|
| **F-M5-1 `list_endpoints`** | P0 | 列终端，支持 filter（在线/离线/按 OS·arch）。 | 内存注册表/HTTP |
| **F-M5-2 `get_endpoint_detail`** | P0 | 单终端硬件画像。 | 注册表 |
| **F-M5-3 `get_active_sessions`** | P0 | 当前进行中的远控会话（谁在控谁）。 | Session |
| **F-M5-4 `query_audit_log`** | P0 | 查连接/操作审计（按终端/时间/结果）。 | SQLite |
| **F-M5-5 `get_screenshots`** | P1 | 取指定在线终端最近截图。 | 截图缓存 |
| **F-M5-6 AI 自然语言问答** | P0 | 管理端内嵌 AI 助手框（或外部 Claude 连 MCP）：管理员问"几台麒麟在线""谁在控财务部电脑" → AI 调 MCP 工具作答。 | 经 MCP → 上述工具 |

**MVP 验收**：MCP Server 启动后，每个 P0 工具能返回正确结构数据；通过 AI 助手提问 ≥2 个自然语言问题（如"列出在线的麒麟终端""今天有哪些远程连接"）得到基于实时数据的正确回答。
**演示降级**：现场断网 → 切预录的 AI 问答脚本；M1–M4 不依赖 AI 仍是完整 demo。
**不做**：MCP 写操作（远程下发/断会话）、私有化大模型部署（demo 用 Claude API，注明生产替换）。

---

## 6. 功能优先级与依赖总览

```
P0 关键路径（必须按序打通）：
  M1(注册+资产+列表)  →  M2(A/B 远控闭环)  →  M4(审计落库)
                      ↘  M3(批量截图)
  M5(MCP+AI) 依赖 M1/M4 的数据，最后接入

依赖：M2 依赖 M1(终端存在) ；M3 依赖 M1(在线列表) ；
      M4 依赖 M2(会话事件) ；M5 依赖 M1+M2+M4(数据齐备)
```

## 7. MVP 总验收清单（demo 通过标准）

- [ ] 2+ 台 Agent 在线，管理端显示真实硬件 + 信创标识（M1）
- [ ] 模式 A：管理端远控某客户端，授权→画面→操作→断开（M2）
- [ ] 模式 B：客户端 A 用 ID+密码远控客户端 B，密码错被拒（M2）
- [ ] 一键批量截图 → 截图墙呈现所有在线终端屏幕（M3）
- [ ] 审计页出现上述连接记录 + 操作文本，可筛选（M4）
- [ ] AI 助手用自然语言查到实时终端/会话/审计数据（M5）

---

*下一步：基于本规格用 writing-plans / speckit.tasks 拆解为 P0 优先的实现任务，按设计文档 §12 时间盒推进。*
