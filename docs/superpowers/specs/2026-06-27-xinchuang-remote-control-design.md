# 信创内网终端远程安全管控平台 — 设计文档

> **项目代号**：OhMyDesk
> **类型**：AI 编程大赛参赛作品（演示工具）
> **死线**：2026-06-28 中午前提交
> **状态**：设计已锁定，待进入实现规划
> **本文档同时承担**：产品设计 spec + 项目 SOP 框架（需求/可行性/架构/技术栈/规范/测试/部署）

---

## 1. 背景与真实问题（创新的现实依据）

赛题为"终端安全管理领域的远程控制演示工具"。经 5 轮竞品/市场调研，结论是**远控市场按网络边界裂成两半，中间塌陷出一块没人管好的空白**：

- **消费级远控**（RustDesk 117k★ / TeamViewer / 向日葵）：体验好，但依赖公网中转、**内网物理隔离网直接用不了**、无 B 端审计与批量管控。
- **服务器侧特权访问**（堡垒机 / Teleport 20.5k★）：审计强，但**只管服务器、够不着员工终端 PC**。

**这块空白 = 企业内网里一大批员工终端 PC 的「远程协助 + 安全管控 + 合规审计」**，叠加信创国产化（党政 2026 年底 80% 国产化）带来的结构性需求：麒麟/统信 UOS + 龙芯/鲲鹏/飞腾架构上，主流远控适配缺位。

GitHub 上最接近答案的是 MeshCentral（6.8k★，Agent 反连 + 自托管 Web + 设备分组 + 审计），其架构范式是本项目的主要借鉴对象；但它 star 低、无信创适配、无 AI 能力。

## 2. 产品定位

> **信创内网终端远程安全管控平台** —— 把"远程控制"重新定义为"内网终端安全管控"，避开与 TeamViewer 卷体验的红海。

核心差异化（每条都对应一条真实痛点）：
- **数据不出网**：Agent 反连自托管，纯内网部署 → 对应"物理隔离网主流远控用不了"。
- **信创原生**：终端标注麒麟/统信/龙芯/鲲鹏 → 对应"国产栈远控缺位"。
- **管控审计闭环**：连接/操作文本审计 → 对应"堡垒机管不到终端 PC"。
- **AI 时代数据消费**：平台内置 MCP Server，管控数据供 AI 自然语言查询 → 蹭满 AI 大赛，且 MCP 与 AI 均在内网，不破护城河。

## 3. 目标 / 非目标

### 3.1 目标（demo 必须呈现）
1. 终端 Agent 上报硬件资产 + 信创标识，管理端可视
2. 远程控制基础连通：**Web 管理端→客户端**、**客户端→客户端** 两种模式
3. 管理平台批量能力：批量查看在线 + **一键批量截图**所有在线终端
4. 会话审计（**纯文本**）：连接记录 + 操作记录
5. MCP Server：管控数据以 MCP 工具暴露，管理员用 AI 自然语言查全网态势

### 3.2 非目标（YAGNI，明确不做）
- ❌ 录像回放、屏幕水印、敏感操作实时拦截
- ❌ 违规外联发现、批量脚本下发、跨网段 WoL 唤醒
- ❌ 真 P2P / NAT 穿透 / 画质帧率优化
- ❌ 多显示器、通讯录（注：**文件传输 / 远程命令 / 即时消息已于 2026-06-30 追加实现**，见 [`2026-06-30-remote-command-file-chat-design.md`](./2026-06-30-remote-command-file-chat-design.md)，本条非目标已部分作废）
- ❌ 真实国产 OS/CPU 适配打包（demo 用 Linux/Windows 模拟，界面标注信创即可）

## 4. 功能范围（5 大模块）

### M1 终端资产管理
- Agent 启动 → WS 反连服务端注册 → 周期上报：IP、MAC、使用人、OS、CPU、内存、GPU+显存、在线状态
- 信创标识：OS（麒麟/统信/Windows）、CPU 架构（龙芯/鲲鹏/x86）图标与角标
- 管理端：终端列表 + 在线/离线状态；点行展开右侧抽屉看硬件画像

### M2 远程控制（基础）
- **模式 A — Web 管理端 → 客户端**：管理员在 Web 点"远程"，目标 Agent 截屏 → base64 → WS → Web 渲染；鼠标坐标回传
- **模式 B — 客户端 → 客户端**：客户端 A 输入 B 的设备 ID + 临时密码 → 服务端校验路由 → 建立会话，画面流同上
- 连接建立前：目标端弹窗授权（简单确认即可）

### M3 管理平台批量
- 批量查看：在线终端网格视图
- **一键批量截图**：管理端下发指令 → 所有在线 Agent 各截当前屏 → base64 回传 → 网格墙展示（"一眼看全网屏幕"，主视觉记忆点）

### M4 会话审计（纯文本）
- **连接记录**：发起方、目标、模式(A/B)、起止时间、时长、结果
- **操作记录**：会话内文本日志（如"发起截图""鼠标点击 (x,y)""断开连接"）
- 管理端审计列表 + 按终端/时间筛选；**不做录像、不做视频回放**

### M5 MCP Server（AI 时代亮点）
- 平台服务端内置 MCP Server，暴露只读工具：
  - `list_endpoints(filter?)` — 查在线/离线/按 OS 架构筛终端
  - `get_endpoint_detail(id)` — 单终端硬件画像
  - `get_active_sessions()` — 当前进行中的远程会话（谁在控谁）
  - `query_audit_log(filter?)` — 查连接/操作审计记录
  - `get_screenshots(ids?)` — 取在线终端截图
- 管理员通过 AI（平台内嵌助手 / Claude）自然语言提问 → AI 调 MCP 工具 → 实时作答
  - 示例："现在几台麒麟终端在线？""谁在远程控制财务部电脑？""列出今天所有远程连接记录"
- **数据不出网**：MCP Server 与 AI 客户端均部署在内网；demo 阶段 LLM 可用 Claude API 演示，生产环境替换为内网私有化大模型（文档须诚实标注此假设）

## 5. 架构设计

```
┌──────────────┐                ┌──────────────────────────────┐
│ Web 管理端    │◄──── WS ──────►│      内网服务端 (Relay)        │
│ React + Vite  │ 控制/截图/审计  │  ① WS 中转 + 终端注册表(内存)   │
│ 态势/资产/批量 │                │  ② 审计存储 (MySQL 文本)       │
│ 截图/审计/AI   │                │  ③ MCP Server (官方 TS SDK)    │◄── AI
└──────────────┘                └──────────────┬───────────────┘
                                                │ WS 注册/被控/截图
                       ┌────────────────────────┴─────────┐
                       │ Agent 客户端 (Slint + Rust)        │
                       │ 上报硬件 / 截屏 / 被控 / 也能主控   │
                       └──────────────────────────────────┘
全链路内网，数据不出网
```

- **星型拓扑**：服务端是唯一中枢，负责注册表、ID+密码鉴权、会话路由、帧转发、审计落库、MCP 暴露。
- **Agent 反向连接**：被控端主动反连服务端（借鉴 MeshCentral），穿透内网防火墙、天然支持"一台控制台管一批终端"。
- **实时态在内存，历史态落库**：终端注册表用内存 Map；会话/审计历史用 **MySQL 8**（`sqlx`，`utf8mb4`）持久化；DB 连不上时审计 best-effort 降级，实时链路（注册/远控/截图）不受影响。
- **协议契约优先**：统一 JSON 信封 `{from, to, ts, payload}`（`payload` 内部 tag `type` 判别消息），三端共享 TS 类型（`ts-rs` 导出），是全项目第一优先级产物。

## 6. 技术栈

> **统一栈决策（2026-06-27 选型研究结论）**：客户端 Agent + 服务端统一 **Rust**（满足"同栈"核心诉求）；管理端因浏览器限制保留 React/TS；MCP 因 TS SDK 最成熟而独立成薄层。Rust 在信创远控有唯一生产实证（RustDesk 麒麟/统信落地）、客户端体积最小、网络加速天花板最高。

| 模块 | 技术 | 理由 |
|------|------|------|
| Agent 客户端 | **Slint**（软渲染，无 GPU 依赖）+ Rust；`xcap`(截屏) `enigo`(输入,锁 X11) `sysinfo`(硬件) + `mac_address`/`local-ip-address` | 二进制几 MB、绕开国产 CPU 最脆弱的图形栈(OpenGL/webkitgtk)；RustDesk 已在麒麟/统信生产实证 |
| 服务端 Relay | **Rust** + `axum` + `tokio` + `tokio-tungstenite`(WS) | 与客户端同栈、共享协议；原生性能；`rustls` 纯 Rust TLS 避免 openssl 交叉编译坑 |
| 共享协议 | Rust crate（`serde`），`ts-rs` 生成管理端 TS 类型 | 单一事实源，Rust↔TS 类型自动一致 |
| 管理端 Web | React + Vite + Tailwind + shadcn/ui + Lucide（浏览器限制保留 TS） | 上帝视角大屏，Claude 最熟，死线最稳 |
| MCP Server | 独立 TS 薄层（`@modelcontextprotocol/sdk`），读 server HTTP（不直连 DB） | TS SDK 最成熟，与主体 Rust 解耦 |
| 数据库 | **MySQL 8**（`sqlx` 异步，`utf8mb4`，`DATABASE_URL`） | 会话/审计历史持久化；实时态在内存不落库；正式产品形态（利于立项/软著） |
| 网络扩展预留 | `quinn`(QUIC)、`webrtc-rs` | 未来传输/画质加速 |
| 信创目标 | `loongarch64-unknown-linux-gnu` / `aarch64` 交叉编译（TLS 用 rustls），参考 RustDesk | 麒麟/统信 + 龙芯/鲲鹏/飞腾 |

**Monorepo（Cargo workspace + 前端子目录）**：
```
OhMyDesk/
├─ Cargo.toml           # workspace 根
├─ crates/protocol/     # 共享协议类型 (serde + ts-rs 导出 TS)
├─ crates/server/       # axum WS Relay + MySQL 审计 (sqlx)
├─ crates/client/       # Slint 桌面 Agent (被控 + 主控)
├─ apps/admin-web/      # React 管理端 (Vite + shadcn，浏览器)
└─ apps/mcp/            # 独立 TS MCP Server (读 server HTTP，不直连 DB)
```

> 技术栈风险标注：① Slint 的 `.slint` DSL 与 sysinfo 最新 GPU API 在 Claude 语料盲区，已抓取最新文档生成项目 skill 缓解；② 信创真机（LoongArch）适配 demo 阶段仅交叉编译验证，不保证国产 GPU 利用率等细节；③ enigo 锁 X11 会话规避 Wayland bug。

## 6.1 代码目录结构（前端 / 后端 / 客户端）

> 三端目录均按「一文件一关注点」组织，后端 Rust、前端 React、客户端 Slint 各自独立清晰。详细文件职责见实现计划「文件结构」节。

**后端 —— 服务端 Relay（`crates/server`）**
```
crates/server/src/
├─ main.rs        # 启动：axum router + WS 升级 + 静态托管
├─ hub.rs         # WS 连接管理 + 信封路由 + 广播
├─ registry.rs    # 内存终端注册表 + 在线超时
├─ session.rs     # 会话建立 / 鉴权(A/B) / 结束
├─ audit.rs       # MySQL 文本审计：落库 + 键鼠聚合计数
├─ db.rs          # MySQL 连接池（sqlx MySqlPool）
└─ http.rs        # 只读 HTTP for MCP（/api/endpoints|sessions|audit）
```

**后端 —— 共享协议（`crates/protocol`，单一事实源）**
```
crates/protocol/src/lib.rs   # 实体 + 信封 + 消息枚举 + 信创标识；ts-rs 导出 TS
```

**客户端 —— Slint Agent（`crates/client`）**
```
crates/client/
├─ build.rs          # slint_build::compile
├─ ui/app.slint      # 被控提示条 + 授权弹窗 + 主控贴帧窗口
└─ src/
   ├─ main.rs        # 启动 + UI 事件循环 + tokio runtime
   ├─ asset.rs       # sysinfo 硬件采集 → EndpointInfo
   ├─ net.rs         # 反连 WS + 注册 + 心跳 + 收发信封
   ├─ capture.rs     # xcap 截屏 → JPEG → base64
   └─ inject.rs      # enigo 键鼠注入（被控端执行）
```

**前端 —— 管理端 Web（`apps/admin-web`，pnpm）**
```
apps/admin-web/src/
├─ main.tsx / App.tsx
├─ lib/
│  ├─ ws.ts          # WS 客户端 + 信封收发
│  └─ types/         # ts-rs 生成的 TS 类型（勿手改）
├─ store.ts          # 终端/会话/审计状态（zustand）
├─ components/ui/    # shadcn 组件（v0 产物落位）
└─ pages/
   ├─ Assets.tsx     # M1 终端资产
   ├─ Grid.tsx       # M3 批量监控 / 截图墙
   ├─ Remote.tsx     # M2 远程控制
   ├─ Audit.tsx      # M4 会话审计
   └─ Assistant.tsx  # M5 AI 助手
```

**MCP Server（`apps/mcp`）与脚本资源**
```
apps/mcp/src/index.ts     # 5 个只读 tool，读 server HTTP
scripts/db/schema.sql     # MySQL 建表：endpoints / sessions / audit_logs
scripts/deploy/           # systemd / 构建发布
assets/                   # logo、信创图标、截图
```

## 7. 数据模型（核心实体）

- **Endpoint**：`id, name(使用人), ip, mac, os{name,type}, cpu{model,arch}, ram, gpu{model,vram}, online, lastSeen, password(临时)`
- **Session**：`id, mode(A|B), fromId, toId, startAt, endAt, status`
- **AuditLog**：`id, sessionId, ts, actorId, type(connect|auth_fail|reject|screenshot|input|disconnect), text`（枚举与 protocol `AuditType` 一致，C-1；DB 列名 `event_type`，B-DB1）

## 8. 消息协议（WS 信封）

统一信封 `{ from, to, ts, payload }`（`payload` 内部 tag `type` 判别消息，前端按 `env.payload.type`），关键 type：
`register / register_ack / heartbeat / endpoint_list / connect_request / auth_result / connect_ack / reject / frame / input / screenshot_req / screenshot_resp / session_end`

## 9. 开发规范（SOP — 规范）

- 复用项目铁律（`.claude/CLAUDE.md`）：简体中文注释/提交、第一性原理、防御性交付、commit 规范
- 语言规范（按需激活 skill）：`typescript-patterns`(strict) / `rust-patterns`(clippy+fmt) / `coding-standards`
- 每模块跑通即 `/commit`，保持可回滚

## 10. 测试策略（SOP — 测试）

> 演示项目务实降标（已与团队认知对齐）：**关键路径优先，不追求 80% 全覆盖**。

- **协议契约测试**（必做）：信封序列化/反序列化 + 消息路由（protocol 包是三端基石）
- **E2E 连接闭环**（必做）：起 server → 模拟两个 client → 注册 → 建会话(A/B 两模式) → 收到帧 → 审计落库
- **MCP 工具测试**（必做）：每个 MCP tool 返回结构正确、能读到注册表/审计数据
- **手动彩排**（明早）：完整演示流程走 2 遍
- **放弃**：UI 组件单测、边界 fuzzing

## 11. 部署（SOP — 部署）

- **服务端**：Rust 编译单二进制，systemd 守护，固定端口，纯内网监听
- **数据库**：MySQL 8（生产内网实例；本地 `docker run mysql:8` 起库），建表脚本 `scripts/db/schema.sql`，连接走 `DATABASE_URL`
- **管理端**：构建静态资源，由服务端同端口托管（一个内网 URL 给评委）
- **客户端**：Slint 编译单二进制（信创软渲染，关 GPU 默认特性）；**兜底**：打包翻车则浏览器全屏运行模拟客户端
- **MCP**：独立 TS 进程，读 server HTTP（`/api/*`），stdio/SSE 暴露给内网 AI 客户端
- **部署目标机**：工作区挂载的 `/data/mxd/mxdserver079/scripts` 可纳入部署脚本（待确认是否为目标机）

## 12. 死线时间盒（今 6/27 下午 → 明 6/28 中午）

| 阶段 | 产物 | 风险 |
|------|------|------|
| P0(~1h) | Monorepo 脚手架 + protocol 协议 + server echo 跑通 | 高(地基) |
| P1(~2h) | Admin 界面 + WS 接入 + 终端列表/态势(假数据起步) | 中 |
| P2(~2h) | Slint 客户端 + 注册上线 + 真实硬件上报 | **最高**(Slint 集成) |
| P3(~2h) | 远程控制闭环：模式 A + 模式 B + 授权 | 中 |
| P4(~2h) | 批量截图 + 文本审计落库 | 中 |
| P5(~1.5h) | MCP Server + AI 自然语言查询 | 中 |
| P6(今晚) | 联调 + 视觉美化(深色/信创标识) | 低 |
| P7(明早) | 彩排 2 遍 + 兜底预案 | 低 |

**关键路径**：P0 → P2 优先（最高风险的 Slint 集成尽早暴露）。

## 13. 风险与缓解

1. **Slint 集成翻车**（最易死线翻车）：客户端 GUI 用 Slint 软渲染（Cargo 关默认特性留 `backend-winit + renderer-software`），API 先查 skill `rust-remote-control-stack`；v0 产物仅用于管理端 Web，**不进 Slint 客户端**（技术栈不兼容）。
2. **批量截图带宽**：base64 膨胀 ~33%，全屏每秒几百 KB；内置降级（720p + JPEG quality 0.6 + 截图按需触发非持续流）。
3. **MCP + AI 现场翻车**：AI 自然语言查询依赖 Claude API；预置降级——断网时切播录好的查询脚本，且 M1-M4 不依赖 AI 仍是完整 demo。
4. **信创真实性存疑**：诚实标注 demo 用模拟，不谎称真机适配；话术强调"架构为信创内网而生"。

## 14. 调研来源（节选）

- FBI IC3 2024 Report（远控诈骗损失）、TeamViewer/AnyDesk 安全事件、BlueKeep CVE-2019-0708
- 等保 2.0 三级（身份鉴别/访问控制/安全审计）、网络安全法第二十一条（日志留存 6 个月）、个人信息保护法
- GitHub：RustDesk / frp / Apache Guacamole / MeshCentral / Teleport
- 信创国产化政策、麒麟/统信信创认证、违规外联/影子资产白皮书

---

*本设计文档由 brainstorming 流程收敛产出，固化 5 轮调研 + 多轮决策结论。下一步：writing-plans 生成实现计划。*
