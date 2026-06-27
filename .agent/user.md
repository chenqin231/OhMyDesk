# 项目坐标

> 描述本项目的技术栈 / 目的 / 工作流。Claude Code 会自动读取本文件作为项目上下文。
>
> - **AI 生成**：`ai-rules init-context`（需安装 Claude CLI）
> - **强制重建**：`ai-rules init-context --refresh`（仅此命令会覆盖）
> - **手动维护**：直接编辑本文件；`ai-rules update` 不会覆盖你的修改。

## 🔧 技术栈（WHAT）

**统一栈决策**：客户端 Agent + 服务端统一 **Rust**；管理端因浏览器限制用 React；MCP 因 SDK 成熟度独立成 TS 薄层。

| 模块 | 技术 |
|------|------|
| 客户端 Agent（桌面，被控+主控） | **Slint**（软渲染，无 GPU 依赖）+ Rust；`xcap`(截屏) `enigo`(键鼠注入,锁 X11) `sysinfo`(硬件) `mac_address`/`local-ip-address` |
| 服务端 Relay | **Rust** + `axum` + `tokio` + `tokio-tungstenite`(WS) + `rustls`(纯 Rust TLS) |
| 共享协议 | Rust crate（`serde`），`ts-rs` 生成管理端 TS 类型 |
| 管理端 Web | React + Vite + Tailwind + shadcn/ui |
| MCP Server | 独立 TS 薄层（`@modelcontextprotocol/sdk`），读 server HTTP（不直连 DB） |
| 数据库 | **MySQL**（`sqlx` 异步驱动，`utf8mb4`，`DATABASE_URL` 配置）；存会话/审计历史，实时态在内存 |
| 网络扩展预留 | `quinn`(QUIC)、`webrtc-rs` |
| 信创目标 | `loongarch64-unknown-linux-gnu` / `aarch64` 交叉编译，参考 RustDesk |

**仓库结构（Cargo workspace + 前端子目录）**：`crates/protocol`(协议) `crates/server`(服务端) `crates/client`(Slint 客户端) `apps/admin-web`(React 管理端) `apps/mcp`(TS MCP)。

## 🎯 项目目的（WHY）

**信创内网终端远程安全管控平台**（AI 编程大赛参赛作品）。

把"远程控制"重新定义为"内网终端安全管控"，填补市场空白：消费级远控（RustDesk/向日葵）依赖公网中转、内网物理隔离网用不了、无 B 端审计；堡垒机/PAM 只管服务器、够不着员工终端 PC。本平台为信创内网（麒麟/统信 + 龙芯/鲲鹏）提供数据不出网的终端远程协助 + 安全管控 + 文本审计 + MCP 数据外发（供 AI 自然语言查全网态势）。

**demo 范围**：① 终端资产上报（含信创标识）② 远程控制（Web→客户端 / 客户端→客户端）③ 批量查看 + 一键批量截图 ④ 文本会话审计 ⑤ 平台做 MCP Server 供 AI 查询。

## ⚙️ 工作流（HOW）

> 工程尚未初始化，以下为规划中的命令，建脚手架后校正。

### 构建与运行
- 服务端：`cargo run -p server`
- 客户端：`cargo run -p client`（Slint 桌面）
- 管理端：`cd apps/admin-web && pnpm dev`
- MCP：`cd apps/mcp && pnpm start`
- 信创交叉编译：`cargo build --target loongarch64-unknown-linux-gnu`（TLS 用 rustls，避免 openssl）

### 测试
- 协议契约测试 + E2E 连接闭环（A/B 两模式）为关键路径，**务实降标：不追求 80% 全覆盖**。
- enigo 测试需串行：`cargo test -- --test-threads=1`。

### 部署
- 服务端：Rust 单二进制 + systemd 守护，纯内网监听。
- 管理端：构建静态资源由服务端同端口托管。
- 数据库：MySQL（生产内网实例；本地用 `docker run mysql:8` 起库），建表脚本 `scripts/db/schema.sql`。
- 客户端：发布二进制；MCP 独立 TS 进程**读 server HTTP**（不直连 MySQL）。

## 📎 项目自定义约束

- **统一栈红线**：客户端 + 服务端只用 Rust，不引入第二种系统语言；管理端 Web/MCP 用 TS 是浏览器与 SDK 成熟度的例外，不扩大。
- **信创优先**：运行环境锁 **X11 会话**（xcap/enigo 在 Wayland 不可靠）；TLS 一律 `rustls` 纯 Rust（避免 openssl 交叉编译坑）；GUI 用 Slint 软渲染（绕开国产 CPU 的 OpenGL/webkitgtk）。
- **数据不出网**：内网部署，MCP Server 与 AI 均在内网；任何"数据外发"仅指经 MCP 协议供内网 AI 消费。
- **Claude 语料盲区**：Slint `.slint` DSL、enigo 0.6 新 API、sysinfo 最新 GPU API 易写成过时版本——开发前先查项目 skill `rust-remote-control-stack`。
- **设计依据**：`docs/superpowers/specs/2026-06-27-xinchuang-remote-control-design.md`。

## 📐 代码规范（前端 / 后端 / 数据库 / 跨端契约）

> 用 v0 生成前端代码后**尤须**遵守 A 组接入规范，防止外来代码风格失控。详见 skill `v0-to-project`。

### A. 前端（admin-web）
- TypeScript strict、禁 `any`；跨端数据类型**只用 ts-rs 生成的**（`src/lib/types/`），不手写重复实体。
- **v0 代码接入三步**：① 剥离 v0 自带 mock，改从 store 读真实数据；② 删除 Next.js 专属写法（`next/*`、app router、`"use client"`），纯化为 Vite + React；③ 页面落 `src/pages/`、shadcn 组件落 `src/components/ui/`。
- 状态单一来源：一个 store（zustand），WS 数据单向流入，组件不自持服务端状态。
- 样式 Tailwind + shadcn/ui，深色主题 token 统一；包管理器统一 **pnpm**。

### B. 后端（Rust：protocol / server / client）
- 提交前 `cargo fmt` + `cargo clippy -- -D warnings` 必过。
- 错误用 `anyhow::Result` 传播；WS 协议坏包忽略不 `panic`；不对外部输入 `unwrap()`。
- 实体单一定义：所有跨端数据结构只在 `crates/protocol` 定义一次（serde + ts-rs）。
- 模块单一职责：一文件一关注点（hub/registry/session/audit/capture/inject 分离）。
- 注释与 commit 用简体中文。

### C. 数据库（MySQL）
- 选型 **MySQL 8 + `sqlx`**（异步，与 tokio/axum 同生态）；字符集 `utf8mb4`，连接走 `DATABASE_URL`，连接池 `MySqlPool`。
- 职责边界：**实时态（终端在线/注册表）在内存**不落库；**历史态（会话、审计）落 MySQL**。
- 命名：表名/字段 snake_case；时间戳统一 `BIGINT`（秒级 epoch，对齐协议 `i64`）；业务主键用 `VARCHAR`（如 `ep-xxxx`/uuid）。
- 建表脚本集中 `scripts/db/schema.sql`，用 `CREATE TABLE IF NOT EXISTS`，不引迁移框架（demo）。
- 核心表：`endpoints`(资产台账) `sessions`(会话历史) `audit_logs`(文本审计)。

### D. 跨端契约
- 改 `crates/protocol` 协议后**必须重跑** `cargo test -p protocol` 重新导出 TS；**禁止前端手改 `src/lib/types/` 生成物**。
- 字段命名 Rust snake_case，serde 透传，前端用生成类型对接，三端零漂移。
