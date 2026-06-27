# OhMyDesk · 信创内网终端远程安全管控平台

> 内网环境下对信创终端（麒麟/统信 × 龙芯/鲲鹏）做**资产管理 + 远程控制 + 批量监控 + 操作审计 + AI 问答**的一体化平台。Rust 全栈，数据不出网。

## 能力总览（5 大模块）

| 模块 | 能力 | 端 |
|------|------|----|
| **M1 终端资产** | Agent 反连注册、硬件采集、心跳在线态、信创标识识别、列表/详情 | Agent + Web |
| **M2 远程控制** | 模式 A（Web→终端）/ 模式 B（终端→终端）授权 → 画面 → 键鼠 → 断开 | Agent + Web |
| **M3 批量监控** | 一键批量截图，在线终端屏幕墙 | Web + Agent |
| **M4 会话审计** | 连接/操作纯文本审计落 MySQL，按终端/时间筛选 | Server |
| **M5 MCP/AI** | 管控数据以 MCP 只读工具暴露给 AI，自然语言问答 | MCP + Web |

## 架构

```
┌────────────┐   WS(反连)   ┌──────────────────────────┐   HTTP/stdio  ┌────────────┐
│ Agent 客户端 │ ───────────▶ │      Server (Relay)       │ ◀───────────  │ MCP Server │
│ Slint+Rust  │ ◀─────────── │  注册表/鉴权/路由/审计落库   │   /api/*       │   TS 薄层   │
│ 被控+主控    │   帧/键鼠    │  axum + MySQL             │               └────────────┘
└────────────┘              │  + 静态托管 admin/dist     │
                            └──────────┬───────────────┘
                            HTTP+WS+UI │ 单一内网 URL :8765
                                  ┌────▼─────┐
                                  │ 管理 Web  │  React + Vite
                                  └──────────┘
```

- **协议单一事实源**：`src/protocol`（Rust）用 `serde` + `ts-rs` 自动导出 TS 类型给 Web，杜绝三端漂移。
- **统一信封**：`Envelope { from, to, ts, payload }`，`payload` 内部 tag `type` 判别消息。
- **单一内网 URL**：Server 同端口（:8765）同时提供 `/`(UI) + `/api/*` + `/ws`。

## 目录

```
src/protocol/   协议契约（三端单一事实源；ts-rs 导出 admin 类型）
src/server/     Relay 服务端（axum WS 中转 + MySQL 审计 + 静态托管）
src/client/     Agent 客户端（Slint UI + 采集/网络/截屏/注入）
src/admin-web/  管理 Web（React + Vite，五页）
src/mcp/        MCP Server（4 只读 tool，真实 HTTP）
scripts/db/     MySQL 建表 DDL
```

## 环境要求

- Rust（stable，工作区构建）
- Node ≥ 20 + pnpm（admin-web / mcp）
- MySQL 8（可选；无则审计自动降级，实时链路不受影响）
- 客户端 GUI 需 **X11 会话**（xcap/enigo 在 Wayland 不可靠，进程会强制锁 X11 + 软渲染）

## 构建

```bash
# 1) Rust 全栈
cargo build --workspace

# 2) 管理 Web（产出 dist 供 server 托管）
pnpm -C src/admin-web install
pnpm -C src/admin-web build

# 3) MCP Server
cd src/mcp && pnpm install && pnpm build && cd -
```

## 运行（单一内网 URL 模式）

```bash
# MySQL（可选；这里用 docker 起一个，库名 ohmydesk）
docker run -d --name ohmydesk-mysql \
  -e MYSQL_ROOT_PASSWORD=ohmydesk -e MYSQL_DATABASE=ohmydesk \
  -p 13307:3306 mysql:8.0

# 启动 Server（务必在仓库根，托管 admin/dist；建表 DDL 自动幂等执行）
DATABASE_URL="mysql://root:ohmydesk@127.0.0.1:13307/ohmydesk" \
OHMYDESK_WEB_DIR="src/admin-web/dist" \
cargo run -p server          # 监听 0.0.0.0:8765
```

- **管理端**：浏览器开 `http://<服务器IP>:8765/`（UI、API、WS 同源）。
- **客户端 Agent**（需 X11 显示）——**推荐 .deb 安装**（终端用户无需懂命令行）：
  ```bash
  # 构建 deb（先 cargo build -p client --release）
  bash packaging/deb/build-deb.sh        # 产出 dist-deb/ohmydesk-client_*.deb
  sudo dpkg -i dist-deb/ohmydesk-client_*.deb   # 缺依赖：sudo apt-get -f install
  # 安装后：编辑 /etc/ohmydesk/client.env 设 OHMYDESK_SERVER=ws://<服务端IP>:8765/ws
  ohmydesk-client-launch                 # 或 应用菜单「OhMyDesk 终端」
  ```
  开发期直接跑：`OHMYDESK_SERVER="ws://<IP>:8765/ws" cargo run -p client -- "张伟-财务部"`。
  起 ≥2 个不同名实例即可在管理端看到多台终端。
- **MCP Server**（stdio，供 Claude Desktop 等 MCP 客户端接入）：
  ```bash
  OHMYDESK_API_BASE="http://<服务器IP>:8765" node src/mcp/dist/index.js
  ```

> 开发期前端单独跑（vite :5173）时，需设 `VITE_WS_URL=ws://127.0.0.1:8765/ws` 指回 server；单一 URL 部署无需此变量（自动同源派生）。

## 验证状态（截至当前提交）

| 层 | 验证方式 | 结果 |
|----|---------|------|
| 协议契约 | `cargo test -p protocol` | 24 通过 |
| 客户端 net/几何/采集 | `cargo test -p client` | 21 通过（含 I3 截图回发契约） |
| 服务端会话/审计/路由 | `cargo test -p server` | 10 通过 |
| 全栈 lint | `cargo clippy --workspace` | 0 警告 |
| Web/MCP 类型 | `tsc --noEmit` | 0 错误 |
| I1 资产链路 | 双真实终端注册 + 信创推断 + 离线检测 | 通过 |
| I2 远控闭环 | node 双端探针（授权→键鼠→帧双向→拒连） | 通过 |
| I3 批量截图 | node 双 agent 探针（截图请求→双端回发→按 endpoint 入墙） | 通过 |
| I4 MCP 真实 HTTP | 真实 server 拉 2 终端/1 会话/4 审计 | 通过 |
| M4 审计落库 | 真实 MySQL 落审计 4 条 + 会话 1 条（含终态 UPDATE） | 通过 |

手工（浏览器/真机）逐条验收见 [docs/06-测试/手工验收清单.md](docs/06-测试/手工验收清单.md)。

## 已知环境限制：截屏

- **WSL/WSLg 无法真实截屏**：WSLg 的 X server 不完整支持 X GetImage，全屏抓图报 `xcb protocol error`；远控画面/批量截图会取不到真实屏幕。**真实信创 X11 物理机无此问题**。
- 在 WSL 上验证「授权→画面→操作→断开」**整条链路**时，设 `OHMYDESK_FAKE_CAPTURE=1` 用合成占位帧（移动竖条+渐变，明确标记为占位、非真实屏幕）。真机部署默认留空走真实截屏。

## 安全约束

- **数据不出网**：仅内网；AI 问答基于本地 MCP 工具数据，不外发到第三方 LLM。
- **TLS**：全栈 `rustls`（不用 openssl，规避 loongarch64 交叉编译坑）。
- **会话锁 X11**：截屏/注入仅在 X11 会话可靠。
