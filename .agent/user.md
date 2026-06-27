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
| MCP Server | 独立 TS 薄层（`@modelcontextprotocol/sdk`），读 server 的 SQLite/HTTP |
| 审计存储 | SQLite（`rusqlite` / `sqlx`） |
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
- 客户端：发布二进制；MCP 独立 TS 进程读 server SQLite。

## 📎 项目自定义约束

- **统一栈红线**：客户端 + 服务端只用 Rust，不引入第二种系统语言；管理端 Web/MCP 用 TS 是浏览器与 SDK 成熟度的例外，不扩大。
- **信创优先**：运行环境锁 **X11 会话**（xcap/enigo 在 Wayland 不可靠）；TLS 一律 `rustls` 纯 Rust（避免 openssl 交叉编译坑）；GUI 用 Slint 软渲染（绕开国产 CPU 的 OpenGL/webkitgtk）。
- **数据不出网**：内网部署，MCP Server 与 AI 均在内网；任何"数据外发"仅指经 MCP 协议供内网 AI 消费。
- **Claude 语料盲区**：Slint `.slint` DSL、enigo 0.6 新 API、sysinfo 最新 GPU API 易写成过时版本——开发前先查项目 skill `rust-remote-control-stack`。
- **设计依据**：`docs/superpowers/specs/2026-06-27-xinchuang-remote-control-design.md`。
