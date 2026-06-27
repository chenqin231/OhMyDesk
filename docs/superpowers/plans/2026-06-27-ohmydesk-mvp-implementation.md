# OhMyDesk 信创内网终端远程安全管控平台 — MVP 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在死线（2026-06-28 中午）前打通 feature spec §7 的 6 条 MVP 验收闭环：终端资产可视、A/B 两模式远控、批量截图墙、文本审计、MCP+AI 查询。

**Architecture:** 星型拓扑 + Agent 反连。服务端 Relay（axum + tokio）是唯一中枢，持内存注册表 + MySQL 文本审计（sqlx），按 WS 统一信封路由帧/输入/指令。Agent（Slint + Rust）反连注册、上报硬件、被控截屏、主控注入。管理端（React + v0 生成 UI）走 WS 接同一信封。MCP Server（TS 薄层）读服务端 HTTP 把管控数据暴露给 AI。

**Tech Stack:** Rust（protocol/server/client）+ React/Vite/shadcn（admin-web）+ TS @modelcontextprotocol/sdk（mcp）。协议单一事实源 = `crates/protocol`，用 `ts-rs` 导出 TS 类型。客户端库 API 一律按 `rust-remote-control-stack` skill 的 reference 写（Slint 1.17 / xcap 0.9 / enigo 0.6 / sysinfo 0.39）。

**测试策略（务实降标，对齐 design §10）：** 纯逻辑（协议序列化、信创标识推断、注册表超时、审计聚合）写真单测；WS/截屏/注入/UI 这类 IO·硬件·界面用集成冒烟 + 手动验证脚本，**不写模拟假测试**（instinct: e2e-test-real-commands-not-simulation）。

**关键路径：** Phase 0 → 1 → 2 → 3 先打通 M1 全链路（最高地基风险尽早暴露），再 4（远控核心）→ 5/6 并行 → 7（MCP）。Slint 客户端集成是最高风险，Phase 2 尽早验证。

---

## 🔒 评审裁决回流（TDD 执行强制约束 — 单一事实源）

> 整合 [三方一致性分析](../specs/2026-06-27-tripartite-consistency-analysis.md) + [并行编排](2026-06-27-parallel-dev-orchestration.md) §0 的全部裁决，**本 plan 现为唯一 TDD 入口**。执行任何 Task 前先对照本清单；协议层裁决（A-1/C-1/W0-3）已落入 Phase 0 代码，server/client 必修项已就地标注。

**协议层（Phase 0，已落代码）**
- **A-1**：`EndpointInfo` 补 `department: Option<String>`（B 端管理 + M5「谁在控财务部电脑」）。
- **C-1**：audit type 统一为 `connect|auth_fail|reject|screenshot|input|disconnect`（删 design 的 `click`、原型的 `transfer`/`error`）；新增 `AuditLog`/`Session` 实体并 ts-rs 导出。
- **W0-3**：`#[serde(tag="type")]` 是内部 tag，type 在 `payload` 内，前端按 `env.payload.type` 判别。

**server（Phase 1/6）**
- **B-DB1**：`audit_logs` 列名用 `event_type`（`type` 是 MySQL 保留字）。
- **M-SRV1**：DB 连接失败降级 `Option<Db>=None`，审计 best-effort，**实时链路 M1/M2/M3 不受 DB 影响**。
- **M-SRV2**：router 挂 `CorsLayer::permissive()`（admin :5173 跨端口 fetch）。
- **M-SRV3**：`http.rs` 的 `State` 同时持 `Arc<Hub>`+`Db`；`/api/endpoints` 读注册表、`/api/audit|sessions` 读 DB。
- **M-SRV4**：hub 转发 `Input` 时对 session aggregator `bump()`（否则审计输入计数恒 0）。
- **P-MCP2**：`/api/endpoints` 返回 `EndpointView[]` 裸数组。

**client（Phase 2/4）**
- **M-CLI1**：`net.rs` 用 mpsc 出站泵（**不**把 `write` move 进心跳 task）。
- **M-CLI2**：断线重连循环（断开 sleep 3s 重连重注册）。
- **M-CLI3**：补 `rand_6()`/`now()`/`cur_ram()` + `rand` 依赖。
- **P-CLI4**：截屏等比缩放 + `Frame` 带真实 `w/h`，注入按 `frame_w` 缩放。

**frontend（Phase 3/4，详见 [mock-api-contract](../specs/2026-06-27-mock-api-contract-and-adapters.md)）**
- **砍 O-1/O-2/O-3**：删 `transfer`、录制标记、临时密码展示。
- **补 G-1~G-5**：帧渲染 canvas、键鼠回传、模式B拒连态、审计时间筛选、AI 真实/降级。
- **适配层 D-1~D-8** + **Transport 抽象**：`adapters/*` 消化漂移，mock/real 同形状切换、集成零改组件。

**mcp（Phase 7）**：**P-MCP1** 锁 SDK 版本，核对 `tool`/`registerTool` 签名。
**收尾**：**P-DOC1** 模式B走 Web 主控；**P-SRV5** ServeDir 托管 admin/dist；**C-2/C-3/C-4** 清理 design 残留（SQLite/Tauri/§8 消息类型）。

---

## 文件结构（Cargo workspace + 前端子目录）

```
OhMyDesk/
├─ Cargo.toml                      # workspace 根，members = crates/*
├─ crates/
│  ├─ protocol/                    # 【单一事实源】信封 + 实体 + 消息枚举 + 信创标识推断
│  │  ├─ Cargo.toml
│  │  └─ src/lib.rs
│  ├─ server/                      # axum WS Relay：注册表/路由/会话/审计/HTTP for MCP
│  │  ├─ Cargo.toml
│  │  └─ src/
│  │     ├─ main.rs                # 启动：axum router + WS 升级 + 静态托管
│  │     ├─ registry.rs            # 内存注册表（DashMap）+ 在线超时
│  │     ├─ hub.rs                 # WS 连接管理 + 信封路由 + 广播
│  │     ├─ session.rs             # 会话建立/鉴权(A/B)/结束
│  │     ├─ audit.rs               # MySQL 文本审计：连接/操作落库 + 查询
│  │     ├─ db.rs                  # MySQL 连接池（sqlx MySqlPool）
│  │     └─ http.rs                # 给 MCP 的只读 HTTP（/api/endpoints 等）
│  └─ client/                      # Slint Agent：被控 + 主控
│     ├─ Cargo.toml
│     ├─ build.rs                  # slint_build::compile
│     ├─ ui/app.slint              # 被控提示条 + 授权弹窗 + 主控贴帧窗口
│     └─ src/
│        ├─ main.rs                # 启动 + UI 事件循环
│        ├─ asset.rs               # sysinfo 硬件采集 → EndpointInfo
│        ├─ net.rs                 # 反连 WS + 注册 + 心跳 + 收发信封
│        ├─ capture.rs             # xcap 截屏 → JPEG → base64
│        └─ inject.rs              # enigo 键鼠注入（被控端执行）
├─ apps/
│  ├─ admin-web/                   # React + Vite + shadcn（v0 生成 UI 落这里）
│  │  └─ src/
│  │     ├─ lib/ws.ts              # WS 客户端 + 信封收发
│  │     ├─ lib/types/             # ts-rs 从 protocol 导出的 TS 类型（生成物）
│  │     ├─ store.ts               # 终端列表/会话/审计状态
│  │     └─ pages/                 # v0 生成的 5 个页面接此 store
│  └─ mcp/                         # TS MCP Server（读 server HTTP）
│     ├─ package.json
│     └─ src/index.ts              # 5 个只读 tool
└─ docs/superpowers/...            # 已有 spec/plan
```

---

## Phase 0 — 地基：Workspace + protocol 协议（design §12 P0，最高风险）

**目标：** workspace 能 `cargo build`；protocol 定义全部实体/信封/消息，序列化往返测试通过，ts-rs 能导出 TS。

### Task 0.1：创建 Cargo workspace 根

**Files:**
- Create: `Cargo.toml`

- [ ] **Step 1: 写 workspace 根清单**

```toml
[workspace]
resolver = "2"
members = ["crates/protocol", "crates/server", "crates/client"]

[workspace.package]
edition = "2021"
version = "0.1.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ts-rs = "10"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
```

- [ ] **Step 2: 提交**

```bash
git add Cargo.toml
git commit -m "chore(workspace): 初始化 Cargo workspace 根清单"
```

### Task 0.2：protocol 实体类型 + 信创标识推断

**Files:**
- Create: `crates/protocol/Cargo.toml`
- Create: `crates/protocol/src/lib.rs`
- Test: `crates/protocol/src/lib.rs`（`#[cfg(test)]` 内联）

- [ ] **Step 1: protocol crate 清单**

```toml
[package]
name = "protocol"
edition.workspace = true
version.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
ts-rs = { workspace = true }
```

- [ ] **Step 2: 写失败测试（先写实体的序列化往返 + 信创标识）**

把以下测试放在 `lib.rs` 末尾，此时类型还没定义，编译应失败：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xinchuang_label_kylin_loongarch() {
        let os = OsInfo { name: "麒麟 V10".into(), kind: OsKind::Kylin };
        let cpu = CpuInfo { model: "Loongson 3A5000".into(), cores: 4, arch: CpuArch::LoongArch };
        assert!(is_xinchuang(&os, &cpu));
        assert_eq!(xinchuang_label(&os, &cpu), "信创·麒麟·龙芯");
    }

    #[test]
    fn xinchuang_label_windows_x86_is_not() {
        let os = OsInfo { name: "Windows 11".into(), kind: OsKind::Windows };
        let cpu = CpuInfo { model: "Intel i7".into(), cores: 8, arch: CpuArch::X86_64 };
        assert!(!is_xinchuang(&os, &cpu));
    }

    #[test]
    fn endpoint_info_roundtrip() {
        let info = EndpointInfo::sample();
        let json = serde_json::to_string(&info).unwrap();
        let back: EndpointInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p protocol`
Expected: FAIL（`OsInfo` 等未定义，编译错误）

- [ ] **Step 4: 实现实体类型 + 信创推断**

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndpointInfo {
    pub id: String,
    pub name: String,                // 使用人
    pub department: Option<String>,  // 部门（裁决 A-1：B 端管理 / 「谁在控财务部电脑」）
    pub ip: String,
    pub mac: String,
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub ram: RamInfo,
    pub gpu: Option<GpuInfo>,
    pub agent_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OsInfo { pub name: String, pub kind: OsKind }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum OsKind { Kylin, Uos, Windows, Linux, Other }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CpuInfo { pub model: String, pub cores: u32, pub arch: CpuArch }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CpuArch { LoongArch, Aarch64, X86_64, Other }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RamInfo { pub total: u64, pub used: u64 }   // 字节

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GpuInfo { pub model: String, pub vram: Option<u64> }

/// OS 或 CPU 任一为国产即判定信创
pub fn is_xinchuang(os: &OsInfo, cpu: &CpuInfo) -> bool {
    matches!(os.kind, OsKind::Kylin | OsKind::Uos)
        || matches!(cpu.arch, CpuArch::LoongArch | CpuArch::Aarch64)
}

pub fn xinchuang_label(os: &OsInfo, cpu: &CpuInfo) -> String {
    if !is_xinchuang(os, cpu) { return "非信创".into(); }
    let os_s = match os.kind { OsKind::Kylin => "麒麟", OsKind::Uos => "统信", _ => "其他" };
    let cpu_s = match cpu.arch { CpuArch::LoongArch => "龙芯", CpuArch::Aarch64 => "鲲鹏", _ => "其他" };
    format!("信创·{os_s}·{cpu_s}")
}

impl EndpointInfo {
    pub fn sample() -> Self {
        EndpointInfo {
            id: "ep-001".into(), name: "张伟".into(), department: Some("财务部".into()),
            ip: "10.0.0.21".into(), mac: "AA:BB:CC:00:00:21".into(),
            os: OsInfo { name: "麒麟 V10".into(), kind: OsKind::Kylin },
            cpu: CpuInfo { model: "Loongson 3A5000".into(), cores: 4, arch: CpuArch::LoongArch },
            ram: RamInfo { total: 16 << 30, used: 6 << 30 },
            gpu: None, agent_version: "0.1.0".into(),
        }
    }
}
```

- [ ] **Step 5: 运行确认通过**

Run: `cargo test -p protocol`
Expected: PASS（3 个测试通过）

- [ ] **Step 6: 提交**

```bash
git add crates/protocol
git commit -m "feat(protocol): 终端实体类型 + 信创标识推断"
```

### Task 0.3：信封 + 消息枚举 + 输入事件

**Files:**
- Modify: `crates/protocol/src/lib.rs`

- [ ] **Step 1: 写失败测试（信封序列化 + tagged 消息判别）**

追加到 `tests` mod：

```rust
#[test]
fn envelope_register_roundtrip() {
    let env = Envelope {
        from: "ep-001".into(), to: None, ts: 1719500000,
        payload: Message::Register { info: EndpointInfo::sample(), password: "123456".into() },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"register\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    matches!(back.payload, Message::Register { .. });
}

#[test]
fn input_event_tagged() {
    let e = InputEvent::MouseMove { x: 100, y: 200 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"kind\":\"mouse_move\""));
}

#[test]
fn audit_type_field_rename_and_snake() {
    let log = AuditLog { id: "a1".into(), session_id: "s1".into(), ts: 0,
        actor_id: "admin".into(), kind: AuditType::AuthFail, text: "密码错误".into() };
    let json = serde_json::to_string(&log).unwrap();
    assert!(json.contains("\"type\":\"auth_fail\""));   // 字段名 type、值 snake_case（裁决 C-1）
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p protocol`
Expected: FAIL（`Envelope`/`Message`/`InputEvent` 未定义）

- [ ] **Step 3: 实现信封 + 消息枚举**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    pub from: String,
    pub to: Option<String>,
    pub ts: i64,
    pub payload: Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Mode { A, B }

/// WS 统一消息体；`#[serde(tag="type")]` 内部 tag——type 在 payload 对象内（非信封顶层），前端按 `env.payload.type` 判别，Rust 按枚举变体匹配（裁决 W0-3）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Register { info: EndpointInfo, password: String },
    RegisterAck { id: String },
    Heartbeat { id: String, ram: RamInfo },
    EndpointList { endpoints: Vec<EndpointView> },
    ConnectRequest { mode: Mode, target: String, password: Option<String> },
    AuthResult { session_id: String, ok: bool, reason: Option<String> },
    ConnectAck { session_id: String },
    Reject { session_id: String, reason: String },
    Frame { session_id: String, data: String, w: u32, h: u32, seq: u64 },
    Input { session_id: String, event: InputEvent },
    ScreenshotReq { req_id: String },
    ScreenshotResp { req_id: String, endpoint_id: String, data: String, w: u32, h: u32 },
    SessionEnd { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: u8, down: bool },
    Key { code: String, down: bool },
    Text { text: String },
}

/// 推给管理端的精简视图（含在线态 + 信创标签，不含密码）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndpointView {
    pub info: EndpointInfo,
    pub online: bool,
    pub last_seen: i64,
    pub xinchuang: String,
}

// ── 会话与审计实体（ts-rs 导出给前端审计页 + mock；裁决 C-1 audit type 统一）──
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Session {
    pub id: String,
    pub mode: Mode,
    pub from_id: String,
    pub to_id: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus { Active, Ended, Rejected }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditLog {
    pub id: String,
    pub session_id: String,
    pub ts: i64,
    pub actor_id: String,
    #[serde(rename = "type")]
    pub kind: AuditType,   // Rust 关键字 type → 用 kind + serde rename；DB 列名 event_type(B-DB1)
    pub text: String,
}

/// 裁决 C-1：统一为 feature-spec 集合（删 design 的 click、原型的 transfer/error）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AuditType { Connect, AuthFail, Reject, Screenshot, Input, Disconnect }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p protocol`
Expected: PASS（全部测试通过）

- [ ] **Step 5: 提交**

```bash
git add crates/protocol
git commit -m "feat(protocol): WS 信封 + 消息枚举 + 输入事件 + 终端视图"
```

### Task 0.4：ts-rs 导出 TS 类型到 admin-web

**Files:**
- Modify: `crates/protocol/src/lib.rs`（加导出测试）
- Create（生成物）: `apps/admin-web/src/lib/types/*.ts`

- [ ] **Step 1: 加 ts-rs 导出测试**

ts-rs 约定：带 `#[ts(export)]` 的类型在 `cargo test` 时导出到 `bindings/`。追加一个显式导出测试，指定输出目录：

```rust
#[cfg(test)]
mod ts_export {
    use super::*;
    use ts_rs::TS;
    #[test]
    fn export_all() {
        let dir = "../../apps/admin-web/src/lib/types";
        EndpointInfo::export_all_to(dir).unwrap();   // 带出 OsInfo/CpuInfo/RamInfo/GpuInfo/枚举
        Envelope::export_all_to(dir).unwrap();       // 带出 Message/InputEvent/EndpointView/Mode
        AuditLog::export_all_to(dir).unwrap();       // 审计页/mock 需要（不在 Envelope 链上，须显式）
        Session::export_all_to(dir).unwrap();        // 同上（带出 SessionStatus）
    }
}
```

- [ ] **Step 2: 运行导出**

Run: `cargo test -p protocol export_all`
Expected: PASS，且生成 `EndpointInfo.ts`/`Envelope.ts`/`Message.ts`/`InputEvent.ts`/`EndpointView.ts`/`AuditLog.ts`/`AuditType.ts`/`Session.ts`/`SessionStatus.ts` 等全部依赖类型

- [ ] **Step 3: 验证生成文件存在**

Run: `ls apps/admin-web/src/lib/types/`
Expected: 看到导出的 `.ts` 文件

- [ ] **Step 4: 提交**

```bash
git add crates/protocol apps/admin-web/src/lib/types
git commit -m "feat(protocol): ts-rs 导出 TS 类型到 admin-web"
```

**Phase 0 验收：** `cargo test` 全绿；`apps/admin-web/src/lib/types/` 有生成的 TS 类型；协议契约 = 三端单一事实源。

---

## Phase 1 — Server Relay 骨架（design §12 P0）

**目标：** server 起 axum，WS 端点能升级；客户端连上 → 注册 → 进内存注册表 → 管理端连上即收到 `endpoint_list` 广播；心跳超时标记离线。

### Task 1.1：server crate + axum 启动 + WS 升级 echo

**Files:**
- Create: `crates/server/Cargo.toml`
- Create: `crates/server/src/main.rs`

- [ ] **Step 1: server 清单**

```toml
[package]
name = "server"
edition.workspace = true
version.workspace = true

[dependencies]
protocol = { path = "../protocol" }
axum = { version = "0.7", features = ["ws"] }
tokio = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
dashmap = "6"
futures-util = "0.3"
tower-http = { version = "0.6", features = ["fs", "cors"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "mysql"] }
```

- [ ] **Step 2: 写最小 axum + WS echo**

```rust
use axum::{routing::get, Router, extract::ws::{WebSocketUpgrade, WebSocket, Message as WsMsg}, response::Response};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let app = Router::new().route("/ws", get(ws_handler));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8765").await?;
    tracing::info!("server on ws://0.0.0.0:8765/ws");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    while let Some(Ok(msg)) = socket.recv().await {
        if let WsMsg::Text(t) = msg {
            let _ = socket.send(WsMsg::Text(t)).await;   // echo
        }
    }
}
```

- [ ] **Step 3: 冒烟验证**

Run: `cargo run -p server`（另开终端用 `websocat ws://127.0.0.1:8765/ws` 或浏览器 console 发一条文本，确认原样返回）
Expected: 控制台打印 `server on ...`，echo 回原文

- [ ] **Step 4: 提交**

```bash
git add crates/server
git commit -m "feat(server): axum WS 端点 echo 骨架"
```

### Task 1.2：内存注册表 + 在线超时（纯逻辑，TDD）

**Files:**
- Create: `crates/server/src/registry.rs`
- Modify: `crates/server/src/main.rs`（`mod registry;`）

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use protocol::EndpointInfo;

    #[test]
    fn upsert_and_view() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        let views = reg.views(1000);
        assert_eq!(views.len(), 1);
        assert!(views[0].online);
        assert_eq!(views[0].xinchuang, "信创·麒麟·龙芯");
    }

    #[test]
    fn offline_after_timeout() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        // now 比 last_seen 晚 16s，超过 15s 阈值
        let views = reg.views(1016);
        assert!(!views[0].online);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p server`
Expected: FAIL（`Registry` 未定义）

- [ ] **Step 3: 实现注册表**

```rust
use dashmap::DashMap;
use protocol::{EndpointInfo, EndpointView, xinchuang_label};

const ONLINE_TIMEOUT_SEC: i64 = 15;

struct Entry { info: EndpointInfo, password: String, last_seen: i64 }

pub struct Registry { map: DashMap<String, Entry> }

impl Registry {
    pub fn new() -> Self { Registry { map: DashMap::new() } }

    pub fn upsert(&self, info: EndpointInfo, password: String, now: i64) {
        self.map.insert(info.id.clone(), Entry { info, password, last_seen: now });
    }

    pub fn touch(&self, id: &str, now: i64) {
        if let Some(mut e) = self.map.get_mut(id) { e.last_seen = now; }
    }

    pub fn check_password(&self, id: &str, pw: &str) -> bool {
        self.map.get(id).map(|e| e.password == pw).unwrap_or(false)
    }

    pub fn views(&self, now: i64) -> Vec<EndpointView> {
        self.map.iter().map(|e| {
            let online = now - e.last_seen <= ONLINE_TIMEOUT_SEC;
            EndpointView {
                info: e.info.clone(), online, last_seen: e.last_seen,
                xinchuang: xinchuang_label(&e.info.os, &e.info.cpu),
            }
        }).collect()
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p server`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/server/src/registry.rs crates/server/src/main.rs
git commit -m "feat(server): 内存注册表 + 在线超时判定"
```

### Task 1.3：WS hub —— 信封路由 + 注册 + 列表广播

**Files:**
- Create: `crates/server/src/hub.rs`
- Modify: `crates/server/src/main.rs`（接 hub，替换 echo）

- [ ] **Step 1: 实现 hub（连接管理 + 路由）**

> 设计：每个 WS 连接分配一个出站 `mpsc::Sender<String>`，存进 `Clients: DashMap<conn_id, Sender>`。收到 `Register` → 写注册表 + 记 conn_id↔endpoint_id 映射 + 广播最新 `endpoint_list`。管理端用特殊 id（如 `admin-*`）连入，连上即推一次列表。`Frame`/`Input`/`ConnectRequest` 按 `to` 字段定向转发。

```rust
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::mpsc;
use protocol::{Envelope, Message};
use crate::registry::Registry;

pub struct Hub {
    pub reg: Arc<Registry>,
    clients: DashMap<String, mpsc::UnboundedSender<String>>, // endpoint_id/admin_id -> 出站
}

impl Hub {
    pub fn new(reg: Arc<Registry>) -> Self { Hub { reg, clients: DashMap::new() } }

    pub fn add_client(&self, id: String, tx: mpsc::UnboundedSender<String>) {
        self.clients.insert(id, tx);
    }
    pub fn remove_client(&self, id: &str) { self.clients.remove(id); }

    pub fn send_to(&self, id: &str, json: &str) {
        if let Some(tx) = self.clients.get(id) { let _ = tx.send(json.to_string()); }
    }
    pub fn broadcast_admins(&self, json: &str) {
        for kv in self.clients.iter() {
            if kv.key().starts_with("admin-") { let _ = kv.value().send(json.to_string()); }
        }
    }

    /// 处理一条入站信封；now 由调用方传当前秒级时间戳
    pub fn handle(&self, env: Envelope, now: i64) {
        match &env.payload {
            Message::Register { info, password } => {
                self.reg.upsert(info.clone(), password.clone(), now);
                self.push_list(now);
            }
            Message::Heartbeat { id, .. } => {
                self.reg.touch(id, now);
            }
            // 定向转发：建连/帧/输入/截图/结束都按 to 路由
            Message::ConnectRequest { .. } | Message::Frame { .. } | Message::Input { .. }
            | Message::AuthResult { .. } | Message::ConnectAck { .. } | Message::Reject { .. }
            | Message::ScreenshotResp { .. } | Message::SessionEnd { .. } => {
                if let Some(to) = &env.to {
                    if let Ok(json) = serde_json::to_string(&env) { self.send_to(to, &json); }
                }
            }
            Message::ScreenshotReq { .. } => { /* Phase 5 广播 */ }
            _ => {}
        }
    }

    pub fn push_list(&self, now: i64) {
        let env = Envelope {
            from: "server".into(), to: None, ts: now,
            payload: Message::EndpointList { endpoints: self.reg.views(now) },
        };
        if let Ok(json) = serde_json::to_string(&env) { self.broadcast_admins(&json); }
    }
}

pub fn now_sec() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}
```

- [ ] **Step 2: main.rs 接 hub（替换 echo，按首条消息登记 conn id）**

```rust
mod registry; mod hub;
use std::sync::Arc;
use axum::{routing::get, Router, extract::{ws::{WebSocketUpgrade, WebSocket, Message as WsMsg}, State}, response::Response};
use futures_util::{StreamExt, SinkExt};
use protocol::Envelope;
use hub::{Hub, now_sec};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let reg = Arc::new(registry::Registry::new());
    let hub = Arc::new(Hub::new(reg));
    let app = Router::new().route("/ws", get(ws_handler)).with_state(hub.clone());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8765").await?;
    tracing::info!("server on ws://0.0.0.0:8765/ws");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>) -> Response {
    ws.on_upgrade(move |sock| handle_socket(sock, hub))
}

async fn handle_socket(socket: WebSocket, hub: Arc<Hub>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // 出站泵
    let pump = tokio::spawn(async move {
        while let Some(s) = rx.recv().await { if sink.send(WsMsg::Text(s)).await.is_err() { break; } }
    });
    let mut my_id: Option<String> = None;
    while let Some(Ok(WsMsg::Text(t))) = stream.next().await {
        if let Ok(env) = serde_json::from_str::<Envelope>(&t) {
            if my_id.is_none() {
                my_id = Some(env.from.clone());
                hub.add_client(env.from.clone(), tx.clone());
                if env.from.starts_with("admin-") { hub.push_list(now_sec()); }
            }
            hub.handle(env, now_sec());
        }
    }
    if let Some(id) = my_id { hub.remove_client(&id); }
    pump.abort();
}
```

- [ ] **Step 3: 冒烟验证（脚本模拟一个 client + 一个 admin）**

Run: `cargo run -p server`，另用临时脚本（node/websocat）：一个连接发 `{"from":"ep-001","to":null,"ts":0,"payload":{"type":"register","info":{...sample...},"password":"123456"}}`，另一个 `admin-1` 连上，确认 admin 收到 `endpoint_list` 含 ep-001。
Expected: admin 端收到含 ep-001 的列表

- [ ] **Step 4: 提交**

```bash
git add crates/server/src/hub.rs crates/server/src/main.rs
git commit -m "feat(server): WS hub 信封路由 + 注册 + 列表广播"
```

**Phase 1 验收：** server 起得来；client 注册即进注册表；admin 连上收到 `endpoint_list`；心跳超时逻辑单测通过。

---

## Phase 2 — Agent 客户端：注册 + 硬件采集 + 心跳（design §12 最高风险，M1 闭环）

**目标：** Slint Agent 启动 → 采真实硬件 → 反连 server 注册 → 5s 心跳；管理端能看到这台真实终端。**Slint 集成是全项目最高风险，本 Phase 先跑通无界面的注册链路，再加最小 UI。**

### Task 2.1：client crate + sysinfo 硬件采集（纯逻辑，TDD 可测字段映射）

**Files:**
- Create: `crates/client/Cargo.toml`
- Create: `crates/client/src/asset.rs`

> **写代码前读 skill：** `rust-remote-control-stack` 的 `references/sysinfo-quinn.md`（CPU 必须刷新 2 次、MAC/IP 走 Networks、GPU 是 unreleased 走降级）。

- [ ] **Step 1: client 清单（先不加 Slint，降低首次编译风险）**

```toml
[package]
name = "client"
edition.workspace = true
version.workspace = true

[dependencies]
protocol = { path = "../protocol" }
tokio = { workspace = true }
tokio-tungstenite = "0.24"
futures-util = "0.3"
serde_json = { workspace = true }
anyhow = { workspace = true }
sysinfo = "0.39"
xcap = "0.9"
image = "0.25"
enigo = "0.6"
base64 = "0.22"
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: 实现硬件采集 → EndpointInfo**

按 `sysinfo-quinn.md`：`System::new_with_specifics` + 刷新 2 次取 CPU；`Networks` 取首个有效物理网卡 MAC/IP；OS 名称用 `System::name()` + `os_version()` 推断 `OsKind`；arch 用 `std::env::consts::ARCH` 映射 `CpuArch`。GPU 走可选降级（拿不到则 `None`）。

```rust
use protocol::{EndpointInfo, OsInfo, OsKind, CpuInfo, CpuArch, RamInfo};
use sysinfo::{System, Networks, RefreshKind, CpuRefreshKind, MemoryRefreshKind};

pub fn collect(user_name: &str) -> EndpointInfo {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()));
    sys.refresh_cpu_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_all();

    let cpus = sys.cpus();
    let cpu = CpuInfo {
        model: cpus.get(0).map(|c| c.brand().to_string()).unwrap_or_default(),
        cores: cpus.len() as u32,
        arch: map_arch(std::env::consts::ARCH),
    };
    let os = OsInfo {
        name: format!("{} {}", System::name().unwrap_or_default(), System::os_version().unwrap_or_default()),
        kind: map_os(&System::name().unwrap_or_default()),
    };
    let (ip, mac) = first_nic();
    EndpointInfo {
        id: format!("ep-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        name: user_name.to_string(), ip, mac, os, cpu,
        ram: RamInfo { total: sys.total_memory(), used: sys.used_memory() },
        gpu: None, agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn map_arch(a: &str) -> CpuArch {
    match a { "loongarch64" => CpuArch::LoongArch, "aarch64" => CpuArch::Aarch64,
              "x86_64" => CpuArch::X86_64, _ => CpuArch::Other }
}
fn map_os(name: &str) -> OsKind {
    let n = name.to_lowercase();
    if n.contains("kylin") || n.contains("麒麟") { OsKind::Kylin }
    else if n.contains("uos") || n.contains("统信") || n.contains("deepin") { OsKind::Uos }
    else if n.contains("windows") { OsKind::Windows }
    else if n.contains("linux") { OsKind::Linux } else { OsKind::Other }
}
fn first_nic() -> (String, String) {
    let nets = Networks::new_with_refreshed_list();
    for (_, d) in &nets {
        let mac = d.mac_address().to_string();
        if mac == "00:00:00:00:00:00" { continue; }
        if let Some(ipn) = d.ip_networks().iter().find(|n| !n.addr.is_loopback() && n.addr.is_ipv4()) {
            return (ipn.addr.to_string(), mac);
        }
    }
    ("0.0.0.0".into(), "00:00:00:00:00:00".into())
}
```

- [ ] **Step 3: 冒烟验证采集**

加临时 `fn main()` 打印 `collect("测试机")`，Run: `cargo run -p client`
Expected: 打印出真实本机 CPU 型号/核数/arch、内存、本机 IP/MAC

- [ ] **Step 4: 提交**

```bash
git add crates/client
git commit -m "feat(client): sysinfo 硬件采集 → EndpointInfo"
```

### Task 2.2：反连注册 + 心跳

**Files:**
- Create: `crates/client/src/net.rs`
- Create: `crates/client/src/main.rs`

- [ ] **Step 1: 实现 WS 反连 + 注册 + 心跳**

```rust
use protocol::{Envelope, Message, EndpointInfo, RamInfo};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};
use futures_util::{SinkExt, StreamExt};

pub async fn run(server_url: &str, info: EndpointInfo) -> anyhow::Result<()> {
    let (ws, _) = connect_async(server_url).await?;
    let (mut write, mut read) = ws.split();
    let id = info.id.clone();
    let password = format!("{:06}", rand_6());

    // 注册
    let reg = Envelope { from: id.clone(), to: None, ts: now(),
        payload: Message::Register { info, password: password.clone() } };
    write.send(WsMsg::Text(serde_json::to_string(&reg)?)).await?;
    tracing::info!("registered id={id} password={password}");

    // 心跳任务
    let hb_id = id.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let hb = Envelope { from: hb_id.clone(), to: None, ts: now(),
                payload: Message::Heartbeat { id: hb_id.clone(), ram: cur_ram() } };
            if write.send(WsMsg::Text(serde_json::to_string(&hb).unwrap())).await.is_err() { break; }
        }
    });

    // 收下行（Phase 4 在此分发 connect_request/input/screenshot_req）
    while let Some(Ok(WsMsg::Text(t))) = read.next().await {
        if let Ok(env) = serde_json::from_str::<Envelope>(&t) {
            tracing::debug!("recv {:?}", env.payload);
            // TODO Phase 4/5: 处理被控/截图
        }
    }
    Ok(())
}
```

> 注：`write` 被心跳任务 move 走与主循环收发冲突——实现时用 `Arc<Mutex<>>` 包 sink 或拆成 `mpsc` 出站泵（同 server 模式）。心跳与下行处理共用一个出站通道。

- [ ] **Step 2: main.rs 串起来**

```rust
mod asset; mod net;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let user = std::env::args().nth(1).unwrap_or_else(|| "演示终端".into());
    let info = asset::collect(&user);
    net::run("ws://127.0.0.1:8765/ws", info).await
}
```

- [ ] **Step 3: 端到端冒烟（server + 2 个 client + 临时 admin 脚本）**

Run: 先 `cargo run -p server`；再开两个终端 `cargo run -p client 财务-张伟` / `cargo run -p client 行政-李娜`；临时 `admin-1` WS 脚本确认收到 2 台在线。杀掉一个 client，15s 后再请求列表确认变离线。
Expected: 注册表出现 2 台真实终端，杀进程后超时离线

- [ ] **Step 4: 提交**

```bash
git add crates/client/src/net.rs crates/client/src/main.rs
git commit -m "feat(client): 反连注册 + 5s 心跳"
```

### Task 2.3：最小 Slint UI（被控提示条 + 授权弹窗占位）

**Files:**
- Create: `crates/client/build.rs`
- Create: `crates/client/ui/app.slint`
- Modify: `crates/client/Cargo.toml`（加 slint）、`crates/client/src/main.rs`

> **写代码前读 skill：** `references/slint.md`（关默认特性只留 `backend-winit + renderer-software`；网络线程更新 UI 必须 `invoke_from_event_loop` + `Weak`）。

- [ ] **Step 1: Cargo.toml 加 Slint（信创软渲染配置）**

```toml
slint = { version = "1.17", default-features = false, features = ["compat-1-2","std","backend-winit","renderer-software"] }
[build-dependencies]
slint-build = "1.17"
```

- [ ] **Step 2: build.rs + app.slint（被控提示条 + 授权弹窗 + 主控贴帧位）**

`build.rs`：`fn main(){ slint_build::compile("ui/app.slint").unwrap(); }`
`ui/app.slint`：按 `slint.md` §2 语法做窗口，含 `in property <string> peer_name`、`in property <bool> being_controlled`、`callback auth_accept()` / `auth_reject()`、`in-out property <image> frame`（主控贴帧用）。提示条文案"⚠ 此终端正在被 {peer_name} 远程"。

- [ ] **Step 3: main.rs 起 UI 事件循环 + 后台 tokio**

UI 在主线程 `ui.run()`，网络在 tokio runtime 后台线程；二者用 channel 通信。被控授权弹窗的"同意/拒绝"回调通过 channel 通知 net 层回 `AuthResult`。

- [ ] **Step 4: 冒烟验证窗口起得来**

Run: `cargo run -p client 演示终端`
Expected: Slint 窗口弹出（软渲染，无 GPU 报错），注册链路仍正常

- [ ] **Step 5: 提交**

```bash
git add crates/client/build.rs crates/client/ui crates/client/src/main.rs crates/client/Cargo.toml
git commit -m "feat(client): 最小 Slint UI（被控提示条 + 授权弹窗）"
```

**Phase 2 验收：** 真实硬件采集正确；2 台 Agent 注册在线；杀进程 15s 离线；Slint 窗口软渲染起得来。

---

## Phase 3 — Admin Web 接数据：终端列表 + 网格（M1 可视闭环）

**目标：** v0 生成的 UI（提示词 0/1/2）落进 `apps/admin-web`，接 WS 收 `endpoint_list` 渲染真实终端；点行看硬件抽屉。

### Task 3.1：admin-web 脚手架 + WS 客户端

**Files:**
- Create: `apps/admin-web/`（Vite React+TS，**基于 `v0/` 原型整套迁移**，非空脚手架）
- Create: `apps/admin-web/src/lib/ws.ts`
- Create: `apps/admin-web/src/store.ts`

> **接入策略见 skill `v0-to-project`**：`v0/` 是完整 Next.js 原型，UI 层 90%+ 已就绪（深色 token 零返工）。本 Task = 把它迁成 Vite 工程并接数据，**不是从零搭 UI**。三个暗坑：① UI 底座是 **Base UI（`@base-ui/react`）非 Radix**，`components/ui/` 连依赖整套搬，**不要 `shadcn add` 重拉**；② **Tailwind v4**（配置内联 CSS，无 `tailwind.config.js`），admin-web 直接建 v4；③ `next/font`(Geist) **字体本地化**（内网无 Google CDN）。

- [ ] **Step 1: 建 Vite 工程并迁移原型**

Run: `pnpm create vite apps/admin-web --template react-ts && cd apps/admin-web && pnpm i`，装 Tailwind v4（`pnpm add tailwindcss @tailwindcss/vite`）+ `@base-ui/react`。
迁移：`v0/app/*/page.tsx` → `src/pages/{Assets,Grid,Remote,Audit,Assistant}.tsx`（去 `"use client"`、app router → react-router 路由表）；`v0/components/` → `src/components/`（`ui/` 整套含 Base UI）；`v0/app/globals.css` → `src/index.css`（几乎原样，含深色 token）；`v0/lib/*` 的展示字典保留、mock 删除。

- [ ] **Step 2: WS 客户端（用 ts-rs 生成的类型）**

```typescript
import type { Envelope } from "./lib/types/Envelope";
export function connectWs(onEnvelope: (e: Envelope) => void) {
  const id = "admin-" + Math.random().toString(36).slice(2, 8);
  const ws = new WebSocket("ws://127.0.0.1:8765/ws");
  ws.onopen = () => ws.send(JSON.stringify({ from: id, to: null, ts: Date.now()/1000|0, payload: { type: "heartbeat", id, ram: { total:0, used:0 } } }));
  ws.onmessage = (ev) => { try { onEnvelope(JSON.parse(ev.data)); } catch {} };
  return ws;
}
```

> 注：admin 首条消息只为登记 `admin-*` conn id 触发列表推送；server 端 `admin-*` 的 heartbeat 不影响注册表。

- [ ] **Step 3: store 收 endpoint_list**

zustand/简单 state：收到 `payload.type === "endpoint_list"` → setEndpoints。

- [ ] **Step 4: 冒烟**

Run: server + 2 client 起着，`pnpm dev`，浏览器看 console 收到列表。
Expected: 浏览器收到 2 台真实终端

- [ ] **Step 5: 提交**

```bash
git add apps/admin-web
git commit -m "feat(admin): Vite 脚手架 + WS 客户端接 endpoint_list"
```

### Task 3.2：终端列表/网格页接真实数据（v0 UI）

**Files:**
- Create: `apps/admin-web/src/pages/Assets.tsx`（v0 提示词 1 产物）
- Create: `apps/admin-web/src/pages/Grid.tsx`（v0 提示词 2 产物）

- [ ] **Step 1: 落 v0 生成的页面，替换 mock 为 store 数据**

把 v0 生成的 Assets/Grid 组件 props 从内置 mock 改为读 `store.endpoints`（`EndpointView[]`）；字段已对齐（online/info.os/info.cpu/xinchuang）。

- [ ] **Step 2: 点行抽屉显示硬件画像**

Sheet 内容绑定 `EndpointView.info`（CPU/内存/GPU/MAC/Agent 版本）。

- [ ] **Step 3: 手动验证 M1 闭环**

Run: server + 2 client + admin dev。浏览器列表显示 2 台真实终端、信创标识正确；点行看硬件；杀 client 15s 内变离线。
Expected: 满足 feature spec §7 第 1 条

- [ ] **Step 4: 提交**

```bash
git add apps/admin-web/src/pages
git commit -m "feat(admin): 终端列表/网格接真实数据，M1 可视闭环"
```

**Phase 3 验收：** feature spec §7 第 1 条达成（2+ 台真实硬件 + 信创标识 + 离线）。

---

## Phase 4 — 远程控制 M2（demo 核心）

**目标：** 模式 A/B 各跑通"授权→画面→操作→断开"；密码错拒连。

### Task 4.1：会话建立 + 鉴权（A/B，服务端逻辑 TDD）

**Files:**
- Create: `crates/server/src/session.rs`
- Modify: `crates/server/src/hub.rs`（ConnectRequest 处理）

- [ ] **Step 1: 写失败测试（模式 B 密码校验）**

```rust
#[test]
fn mode_b_wrong_password_rejected() {
    let reg = Registry::new();
    reg.upsert(EndpointInfo::sample(), "123456".into(), 0); // 目标 ep-001
    assert!(reg.check_password("ep-001", "123456"));
    assert!(!reg.check_password("ep-001", "000000"));
}
```

- [ ] **Step 2: 运行确认通过**（check_password 已在 Phase 1 实现）

Run: `cargo test -p server`
Expected: PASS

- [ ] **Step 3: hub 实现 ConnectRequest 流程**

模式 A：admin 发 `connect_request{mode:A,target}` → server 转发给 target Agent 弹授权 → Agent 回 `auth_result{ok}` → server 建 Session、回 `connect_ack` 给发起方。
模式 B：发起 Agent 带 password → server `check_password` → 错则回 `reject`，对则转目标弹授权。会话 id 用 uuid。建会话即写审计（Phase 6）。

- [ ] **Step 4: 提交**

```bash
git add crates/server/src/session.rs crates/server/src/hub.rs
git commit -m "feat(server): 会话建立 + A/B 鉴权路由"
```

### Task 4.2：被控端截屏推帧（xcap → JPEG → base64）

**Files:**
- Create: `crates/client/src/capture.rs`
- Modify: `crates/client/src/net.rs`（会话激活后起推帧）

> **写代码前读 skill：** `references/xcap-enigo.md`（`Monitor::all()` 启动枚举一次复用；实时流走 JPEG 非 PNG；锁 X11）。

- [ ] **Step 1: 实现截屏 + 编码**

```rust
use xcap::Monitor;
use base64::{Engine, engine::general_purpose::STANDARD};

pub struct Capturer { mon: Monitor }
impl Capturer {
    pub fn new() -> anyhow::Result<Self> {
        let mon = Monitor::all()?.into_iter().next().ok_or_else(|| anyhow::anyhow!("no monitor"))?;
        Ok(Capturer { mon })
    }
    /// 截一帧 → 缩到 720p → JPEG q60 → base64
    pub fn frame(&self) -> anyhow::Result<(String, u32, u32)> {
        let img = self.mon.capture_image()?;          // RgbaImage
        let resized = image::imageops::resize(&img, 1280, 720, image::imageops::FilterType::Triangle);
        let mut buf = std::io::Cursor::new(Vec::new());
        let rgb = image::DynamicImage::ImageRgba8(resized).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60).encode_image(&rgb)?;
        Ok((STANDARD.encode(buf.get_ref()), 1280, 720))
    }
}
```

- [ ] **Step 2: 会话激活后 2-3fps 推帧**

net 层：收到 `connect_ack`/被控授权通过后，起循环：每 ~350ms `frame()` → 发 `Frame{session_id,data,w,h,seq}` 到对端；`session_end` 停。

- [ ] **Step 3: 冒烟验证帧大小**

临时打印每帧 base64 长度，确认 720p JPEG q60 单帧在几十~一百多 KB 量级（base64 后）。
Expected: 帧可控，不爆带宽

- [ ] **Step 4: 提交**

```bash
git add crates/client/src/capture.rs crates/client/src/net.rs
git commit -m "feat(client): xcap 截屏 → 720p JPEG → base64 推帧"
```

### Task 4.3：主控端渲染（Web canvas + Slint 贴帧）

**Files:**
- Create: `apps/admin-web/src/pages/Remote.tsx`（v0 提示词 3 产物）
- Modify: `crates/client/src/main.rs`（Slint 主控贴帧，按 slint.md §4）

- [ ] **Step 1: Web 端渲染帧**

Remote.tsx：WS 收 `frame` → `<img src={"data:image/jpeg;base64,"+data}>` 或 canvas drawImage；顶部工具栏显示目标名/状态/断开。

- [ ] **Step 2: Slint 端贴帧（客户端→客户端模式 B 主控）**

按 `slint.md` §4：base64 解码 → JPEG 解码为 RGBA → `SharedPixelBuffer::<Rgba8Pixel>` → `Image::from_rgba8` → `invoke_from_event_loop` set 到 `frame` 属性。

- [ ] **Step 3: 手动验证画面**

模式 A：admin 远控某 client，看到该 client 实时桌面。
Expected: Web 端出现对端实时画面（2-3fps）

- [ ] **Step 4: 提交**

```bash
git add apps/admin-web/src/pages/Remote.tsx crates/client/src/main.rs
git commit -m "feat(remote): 主控端画面渲染（Web canvas + Slint 贴帧）"
```

### Task 4.4：键鼠回传 + 注入（enigo）

**Files:**
- Create: `crates/client/src/inject.rs`
- Modify: `apps/admin-web/src/pages/Remote.tsx`（捕获鼠标键盘回传）、`crates/client/src/net.rs`（收 input 调注入）

> **写代码前读 skill：** `references/xcap-enigo.md`（必须 `use enigo::{Mouse,Keyboard}`；坐标按被控屏尺寸映射；`release_keys_when_dropped`）。

- [ ] **Step 1: 实现注入器**

```rust
use enigo::{Enigo, Settings, Mouse, Keyboard, Coordinate::Abs, Direction::{Press, Release, Click}, Button, Key};
use protocol::InputEvent;

pub struct Injector { enigo: Enigo, scale_x: f32, scale_y: f32 }
impl Injector {
    pub fn new(real_w: u32, real_h: u32) -> anyhow::Result<Self> {
        // 帧是 1280x720，真实屏是 real_w×real_h；注入要还原到真实坐标
        Ok(Injector { enigo: Enigo::new(&Settings::default())?,
            scale_x: real_w as f32 / 1280.0, scale_y: real_h as f32 / 720.0 })
    }
    pub fn apply(&mut self, ev: &InputEvent) -> anyhow::Result<()> {
        match ev {
            InputEvent::MouseMove { x, y } =>
                self.enigo.move_mouse((*x as f32*self.scale_x) as i32, (*y as f32*self.scale_y) as i32, Abs)?,
            InputEvent::MouseButton { button, down } => {
                let b = match button { 0 => Button::Left, 1 => Button::Middle, _ => Button::Right };
                self.enigo.button(b, if *down { Press } else { Release })?;
            }
            InputEvent::Text { text } => self.enigo.text(text)?,
            InputEvent::Key { code, down } => {
                if let Some(c) = code.chars().next() {
                    self.enigo.key(Key::Unicode(c), if *down { Press } else { Release })?;
                }
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Web 端捕获并回传**

Remote.tsx：画面区监听 mousemove/click/keydown，坐标换算成帧内 1280×720 坐标，发 `Input{session_id,event}`。

- [ ] **Step 3: 被控端收 input 调注入**

net 层收 `Input` → `Injector::apply`（注入器在会话开始时按真实屏尺寸构造）。

- [ ] **Step 4: 手动验证操作生效**

模式 A：admin 鼠标在画面点击 → 被控端对应位置响应。
Expected: 点击/移动在对端生效，坐标不偏

- [ ] **Step 5: 提交**

```bash
git add crates/client/src/inject.rs apps/admin-web/src/pages/Remote.tsx crates/client/src/net.rs
git commit -m "feat(remote): 键鼠回传 + enigo 注入（坐标映射）"
```

### Task 4.5：会话结束 + 授权弹窗联动

**Files:**
- Modify: `crates/client/src/main.rs`、`crates/server/src/session.rs`、`apps/admin-web/src/pages/Remote.tsx`

- [ ] **Step 1: 任一端断开 → SessionEnd**

断开按钮/掉线 → 发 `session_end` → server 关会话停转发 → 被控端停推帧、提示条消失。

- [ ] **Step 2: 被控端授权弹窗真实联动**

Slint 授权弹窗"同意/拒绝"→ 回 `auth_result` → 决定是否建会话；提示条在被控期间常驻。

- [ ] **Step 3: 手动验证 A/B 完整闭环 + 密码错拒连**

Run: 模式 A 全流程；模式 B 用 ID+正确密码成功、错误密码被拒。
Expected: feature spec §7 第 2、3 条达成

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -m "feat(remote): 会话结束 + 授权弹窗联动，A/B 闭环达成"
```

**Phase 4 验收：** feature spec §7 第 2、3 条（A/B 远控闭环 + 密码错拒连）。

---

## Phase 5 — 批量截图 M3

**目标：** 一键批量截图 → 所有在线 Agent 各截一帧 → 截图墙呈现。

### Task 5.1：截图指令广播 + 回传聚合

**Files:**
- Modify: `crates/server/src/hub.rs`（ScreenshotReq 广播给所有在线 Agent）
- Modify: `crates/client/src/net.rs`（收 ScreenshotReq → 截一帧回 ScreenshotResp）

- [ ] **Step 1: server 广播截图指令**

`ScreenshotReq{req_id}` 来自 admin → 遍历注册表在线 Agent，逐个 `send_to(req)`；Agent 的 `ScreenshotResp` 按 to=admin 转回。

- [ ] **Step 2: Agent 响应截图**

net 收 `ScreenshotReq` → `Capturer::frame()` → 回 `ScreenshotResp{req_id,endpoint_id,data,w,h}`。

- [ ] **Step 3: 提交**

```bash
git add crates/server/src/hub.rs crates/client/src/net.rs
git commit -m "feat(batch): 截图指令广播 + 回传聚合"
```

### Task 5.2：截图墙 UI 接数据

**Files:**
- Modify: `apps/admin-web/src/pages/Grid.tsx`（v0 提示词 2 的批量截图态）

- [ ] **Step 1: 一键批量截图按钮 + 收图填墙**

点按钮发 `screenshot_req{req_id}`；收到 `screenshot_resp` 按 endpoint_id 填对应卡片缩略图；点缩略图放大 Dialog。

- [ ] **Step 2: 手动验证**

Run: server + 多 client + admin。点"一键批量截图"，数秒内截图墙填满所有在线终端屏幕。
Expected: feature spec §7 第 4 条达成

- [ ] **Step 3: 提交**

```bash
git add apps/admin-web/src/pages/Grid.tsx
git commit -m "feat(batch): 截图墙 UI 接数据，M3 闭环"
```

**Phase 5 验收：** feature spec §7 第 4 条（一键批量截图墙）。

---

## Phase 6 — 会话审计 M4（纯文本）

**目标：** 远控产生连接记录 + 操作记录（键鼠聚合计数）；管理端可查询筛选。

### Task 6.1：MySQL 审计存储 + 聚合计数（纯逻辑 TDD）

**Files:**
- Create: `scripts/db/schema.sql`（MySQL 建表）
- Create: `crates/server/src/db.rs`（sqlx 连接池）
- Create: `crates/server/src/audit.rs`
- Modify: `crates/server/src/hub.rs`/`session.rs`（事件落库）

> 数据库规范见 `.agent/user.md` §C：MySQL 8 + sqlx，`utf8mb4`，`DATABASE_URL`，时间戳 `BIGINT`，表名/字段 snake_case。

- [ ] **Step 1: 建表脚本 `scripts/db/schema.sql`**

```sql
CREATE TABLE IF NOT EXISTS endpoints (
  id VARCHAR(64) PRIMARY KEY, name VARCHAR(128), ip VARCHAR(64), mac VARCHAR(32),
  os_name VARCHAR(128), os_kind VARCHAR(16), cpu_model VARCHAR(128), cpu_arch VARCHAR(16),
  last_seen BIGINT
) DEFAULT CHARSET=utf8mb4;
CREATE TABLE IF NOT EXISTS sessions (
  id VARCHAR(64) PRIMARY KEY, mode CHAR(1), from_id VARCHAR(64), to_id VARCHAR(64),
  start_at BIGINT, end_at BIGINT, status VARCHAR(16)
) DEFAULT CHARSET=utf8mb4;
CREATE TABLE IF NOT EXISTS audit_logs (
  id VARCHAR(64) PRIMARY KEY, session_id VARCHAR(64), ts BIGINT,
  actor_id VARCHAR(64), event_type VARCHAR(16), text TEXT,   -- event_type：type 是 MySQL 保留字（裁决 B-DB1）
  INDEX idx_session (session_id), INDEX idx_ts (ts)
) DEFAULT CHARSET=utf8mb4;
```

- [ ] **Step 2: `db.rs` 连接池 + 启动建表**

```rust
use sqlx::mysql::MySqlPoolOptions;
pub type Db = sqlx::MySqlPool;

pub async fn connect() -> anyhow::Result<Db> {
    let url = std::env::var("DATABASE_URL")?;     // mysql://user:pass@127.0.0.1/ohmydesk
    let pool = MySqlPoolOptions::new().max_connections(5).connect(&url).await?;
    sqlx::raw_sql(include_str!("../../../scripts/db/schema.sql")).execute(&pool).await?;
    Ok(pool)
}
```

> **M-SRV1 降级**：`main.rs` 用 `db::connect().await.ok()` 得 `Option<Db>`——连不上则 `None`；`AuditStore` 持 `Option<Db>`、`None` 时写操作 no-op + 告警 log，**实时链路 M1/M2/M3 不依赖 DB**。

- [ ] **Step 3: 写失败测试（输入聚合计数，纯逻辑不依赖 DB）**

```rust
#[test]
fn input_events_aggregate_count() {
    let mut agg = InputAggregator::new();
    for _ in 0..47 { agg.bump(); }
    assert_eq!(agg.summary(), "输入操作 47 次");
}
```

- [ ] **Step 4: 运行确认失败 → 实现 `audit.rs`**

`InputAggregator`（会话内累加，断开时落一条聚合 text，纯逻辑）；`AuditStore { db: Option<Db> }`（M-SRV1）用 `sqlx::query("INSERT INTO audit_logs (...,event_type,text) ...").bind(..).execute(&db)` 写审计；会话起止写 `sessions`；写入 `connect/auth_fail/reject/screenshot/input/disconnect`（列名 `event_type`，对齐 `AuditType` 枚举 C-1）。`endpoints` 资产台账落库为 P1。

- [ ] **Step 5: 运行确认通过**

Run: `cargo test -p server`
Expected: PASS（聚合计数纯逻辑测试通过；DB 写入靠下方端到端冒烟验证）

- [ ] **Step 6: 事件接入落库**

会话建立/拒绝/结束、发起截图落审计；**M-SRV4：server hub 转发 `Message::Input` 时对该 session 的 `InputAggregator.bump()`，`session_end` 落一条聚合 text**（否则审计输入计数恒 0）。

- [ ] **Step 7: 提交**

```bash
git add scripts/db/schema.sql crates/server/src/db.rs crates/server/src/audit.rs crates/server/src/hub.rs crates/server/src/session.rs
git commit -m "feat(audit): MySQL 文本审计 + 输入聚合计数"
```

### Task 6.2：审计查询 HTTP + 审计页 UI

**Files:**
- Create: `crates/server/src/http.rs`（`/api/audit` 查询）
- Create: `apps/admin-web/src/pages/Audit.tsx`（v0 提示词 4 产物）

- [ ] **Step 1: server 加只读 HTTP 查询**

axum 加 `/api/audit?endpoint=&from=&to=&result=` 返回 `AuditLog[]`；同时加 `/api/endpoints`、`/api/sessions`（供 Phase 7 MCP 复用）。
- **M-SRV3**：http router 的 `State` **同时持 `Arc<Hub>`+`Option<Db>`**——`/api/endpoints` 读注册表 `reg.views()`、`/api/audit|sessions` 读 DB（否则 MCP `list_endpoints` 拿不到实时终端）。
- **P-MCP2**：`/api/endpoints` 返回 **`EndpointView[]` 裸数组**（与 MCP `all.filter` 对齐）。
- **M-SRV2**：router 挂 `CorsLayer::permissive()`（admin dev :5173 跨端口 fetch `/api/*`，否则浏览器审计页 CORS 报错）。

- [ ] **Step 2: 审计页接数据（v0 提示词 4 产物）**

Audit.tsx：列表 + 筛选器 fetch `/api/audit` → `AuditLog[]`，**前端用 `adapters/audit.ts` 聚合成会话视图 + timeline**（D-7）；补时间范围筛选逻辑（原型有 UI 无逻辑，G-4）；点记录看时间线。

- [ ] **Step 3: 手动验证**

Run: 完成 1 次远控后，审计页出现连接记录（起止/时长）+ 操作文本，可按终端筛选。
Expected: feature spec §7 第 5 条达成

- [ ] **Step 4: 提交**

```bash
git add crates/server/src/http.rs apps/admin-web/src/pages/Audit.tsx
git commit -m "feat(audit): 审计查询 HTTP + 审计页，M4 闭环"
```

**Phase 6 验收：** feature spec §7 第 5 条（审计记录 + 操作文本 + 筛选）。

---

## Phase 7 — MCP Server M5（AI 亮点）

**目标：** TS MCP Server 暴露 5 个只读 tool（P0 四个），AI 自然语言查实时数据。

### Task 7.1：MCP Server 脚手架 + 4 个只读 tool

**Files:**
- Create: `apps/mcp/package.json`、`apps/mcp/src/index.ts`

- [ ] **Step 1: 建 MCP 工程**

```bash
mkdir -p apps/mcp/src && cd apps/mcp && npm init -y && npm i @modelcontextprotocol/sdk zod
```

- [ ] **Step 2: 实现 server + tool（读 Phase 6 的 HTTP）**

`index.ts`：用 `@modelcontextprotocol/sdk` 注册 `list_endpoints`/`get_endpoint_detail`/`get_active_sessions`/`query_audit_log`，每个 tool `fetch("http://127.0.0.1:8765/api/...")` 取数据返回；stdio transport。

```typescript
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const API = "http://127.0.0.1:8765/api";
const server = new McpServer({ name: "ohmydesk", version: "0.1.0" });

server.tool("list_endpoints", { online: z.boolean().optional(), os: z.string().optional() },
  async (args) => {
    const r = await fetch(`${API}/endpoints`); const all = await r.json();
    const filtered = all.filter((e: any) =>
      (args.online === undefined || e.online === args.online) &&
      (!args.os || e.info.os.name.includes(args.os)));
    return { content: [{ type: "text", text: JSON.stringify(filtered, null, 2) }] };
  });
// get_endpoint_detail / get_active_sessions / query_audit_log 同构

await server.connect(new StdioServerTransport());
```

- [ ] **Step 3: 用 MCP inspector 验证**

Run: `npx @modelcontextprotocol/inspector node apps/mcp/src/index.ts`（或编译后），调每个 tool 确认返回实时数据。
Expected: 4 个 tool 都返回正确结构

- [ ] **Step 4: 提交**

```bash
git add apps/mcp
git commit -m "feat(mcp): MCP Server + 4 个只读 tool"
```

### Task 7.2：AI 自然语言问答（管理端助手 / Claude 连接）

**Files:**
- Create: `apps/admin-web/src/pages/Assistant.tsx`（v0 提示词 5 产物）

- [ ] **Step 1: 接入 AI 问答**

最稳路径：把 MCP Server 配进 Claude Desktop/Code，现场用 AI 提问演示（M5 验收 + 降级预案）。可选：admin 内嵌助手框调后端代理 + Claude API。

- [ ] **Step 2: 手动验证 ≥2 个自然语言问题**

问"列出在线的麒麟终端""今天有哪些远程连接"，AI 调 MCP tool 返回基于实时数据的正确回答。
Expected: feature spec §7 第 6 条达成

- [ ] **Step 3: 录制降级视频**（design §13 风险 3）

录一段 AI 问答正常流程，现场断网兜底播放。

- [ ] **Step 4: 提交**

```bash
git add apps/admin-web/src/pages/Assistant.tsx
git commit -m "feat(mcp): AI 自然语言问答，M5 闭环"
```

**Phase 7 验收：** feature spec §7 第 6 条（AI 查实时数据）。

---

## 收尾（design §12 P6/P7）

- [ ] **联调全链路**：6 条 §7 验收逐条跑通一遍
- [ ] **视觉美化**：深色主题 + 信创标识统一
- [ ] **彩排 2 遍 + 兜底预案**：客户端打包翻车→浏览器模拟；AI 断网→播录像
- [ ] **修文档残留**：design §11/§13 的 "tauri" 字样改为 Slint
- [ ] **最终 commit + push**

---

## Self-Review（spec 覆盖核对）

| feature spec §7 验收 | 对应 Phase/Task |
|---|---|
| 1. 2+ 台真实硬件 + 信创标识 + 离线 | Phase 2（采集/注册/心跳）+ Phase 3（列表） |
| 2. 模式 A 授权→画面→操作→断开 | Phase 4（4.1 鉴权 / 4.2 推帧 / 4.3 渲染 / 4.4 注入 / 4.5 结束） |
| 3. 模式 B + 密码错拒连 | Phase 4.1（check_password）+ 4.5 |
| 4. 一键批量截图墙 | Phase 5 |
| 5. 审计记录 + 操作文本 + 筛选 | Phase 6 |
| 6. AI 查实时数据 | Phase 7 |

**模块映射：** M1=Phase2+3，M2=Phase4，M3=Phase5，M4=Phase6，M5=Phase7。全部 P0 功能点有对应 task。

**已知取舍（务实降标，非 placeholder）：**
- IO/硬件/UI（WS、xcap、enigo、Slint、React 页）用集成冒烟 + 手动验证，不写模拟单测——这是 design §10 明确的降标策略。
- v0 生成的 UI 组件代码不在本计划逐行展开（来自提示词包 + skill 模板），计划只规定"接哪份数据、达成哪条验收"。
- Slint 主控贴帧（Task 4.3 Step 2）是模式 B 客户端→客户端才需要；若时间紧，模式 B 可先只跑通鉴权+审计，主控画面用 Web 端演示。

*下一步：选择执行方式（subagent-driven / inline），按 Phase 顺序推进，每 Task 跑通即 commit。*
