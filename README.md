# OhMyDesk · 信创内网终端远程安全管控平台

[![CI](https://github.com/chenqin231/OhMyDesk/actions/workflows/ci.yml/badge.svg)](https://github.com/chenqin231/OhMyDesk/actions/workflows/ci.yml)
[![Release](https://github.com/chenqin231/OhMyDesk/actions/workflows/release.yml/badge.svg)](https://github.com/chenqin231/OhMyDesk/actions/workflows/release.yml)

> 内网环境下对信创终端（麒麟/统信 × 龙芯/鲲鹏）做**资产管理 + 远程控制 + 批量监控 + 操作审计 + AI 问答**的一体化平台。Rust 全栈，数据不出网。

## 能力总览（5 大模块）

| 模块 | 能力 | 端 |
|------|------|----|
| **M1 终端资产** | Agent 反连注册、硬件采集、心跳在线态、信创标识识别、列表/详情 | Agent + Web |
| **M2 远程控制** | 模式 A（Web→终端）/ 模式 B（终端→终端）授权 → 画面 → 键鼠 → 断开 | Agent + Web |
| **M3 批量监控** | 一键批量截图，在线终端屏幕墙 | Web + Agent |
| **M4 会话审计** | 连接/操作纯文本审计落 SQLite，按终端/时间筛选 | Server |
| **M5 MCP/AI** | 管控数据以 MCP 只读工具暴露给 AI，自然语言问答 | MCP + Web |

## 架构

```
┌────────────┐   WS(反连)   ┌──────────────────────────┐   HTTP/stdio  ┌────────────┐
│ Agent 客户端 │ ───────────▶ │      Server (Relay)       │ ◀───────────  │ MCP Server │
│ Slint+Rust  │ ◀─────────── │  注册表/鉴权/路由/审计落库   │   /api/*       │   TS 薄层   │
│ 被控+主控    │   帧/键鼠    │  axum + SQLite             │               └────────────┘
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

## 目录结构

```
OhMyDesk/
├── Cargo.toml          # Workspace 根：声明成员、公共依赖、release 优化参数
├── Cargo.lock          # 依赖版本锁（可重现构建，二进制项目必须提交）
├── Dockerfile          # 服务端容器化部署
│
├── src/                # 全部源代码
│   ├── protocol/       # 协议契约（三端单一事实源，ts-rs 导出 TS 类型）
│   │
│   ├── server/         # Relay 服务端（Rust + axum + tokio）
│   │   ├── hub.rs      #   WS 连接池 + 消息路由 + 广播
│   │   ├── registry.rs #   内存终端注册表（在线/离线检测）
│   │   ├── session.rs  #   会话鉴权（A/B 模式）+ 生命周期
│   │   ├── audit.rs    #   SQLite 文本审计落库
│   │   ├── auth.rs     #   JWT HS256 签发验证
│   │   └── http.rs     #   /api/* 只读 HTTP（供 MCP + 管理端）
│   │
│   ├── client/         # Agent 客户端（Rust + Slint 软渲染桌面）
│   │   ├── asset.rs    #   sysinfo 硬件采集 → 信创标识推断
│   │   ├── net.rs      #   WS 反连 + 注册 + 心跳
│   │   ├── capture.rs  #   xcap 截屏 → JPEG → base64
│   │   └── inject.rs   #   enigo 键鼠注入（X11 锁定）
│   │
│   ├── admin-web/      # 管理端 Web（React + Vite + Tailwind + shadcn/ui）
│   │   └── src/pages/
│   │       ├── Assets.tsx     # M1 终端资产列表 + 信创标识
│   │       ├── Grid.tsx       # M3 批量截图墙
│   │       ├── Remote.tsx     # M2 远程控制画面
│   │       ├── Audit.tsx      # M4 会话审计列表
│   │       └── Assistant.tsx  # M5 AI 自然语言问答
│   │
│   └── mcp/            # MCP Server（TypeScript + @modelcontextprotocol/sdk）
│       └── index.ts    #   5 只读工具：endpoints/sessions/audit/screenshots
│
├── scripts/
│   ├── db/             # SQLite 建表脚本（schema.sqlite.sql）
│   ├── deploy/         # systemd 守护配置
│   ├── probes/         # 集成测试探针（Node.js，验证 WS 闭环）
│   └── packaging/
│       ├── deb/        # Linux/信创 .deb 打包脚本
│       ├── windows/    # Windows exe 交叉编译脚本
│       └── download/   # 下载页 + Linux 便携启动包
│
├── dist/               # 发布产物（脚本生成，不入库）
│   ├── linux/          #   .deb 安装包（amd64 / arm64 / loong64）
│   ├── windows/        #   ohmydesk-client.exe + 连接服务器.bat
│   └── macos/          #   ohmydesk-client tar.gz（arm64 / x86_64）
│
├── docs/               # 产品文档（立项/需求/设计/演示/用户手册）
├── assets/             # 品牌素材：logo、信创图标
└── proto/              # UI 设计原型（v0 生成，已提炼到 src/admin-web，不开源）
```

## 环境要求

- Rust（stable，工作区构建）
- Node ≥ 20 + pnpm（admin-web / mcp）
- 无需外部数据库：审计用内置 SQLite（单文件 ohmydesk.db，自动创建；连接失败则审计降级，实时链路不受影响）
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
# 启动 Server（务必在仓库根，托管 admin/dist；内置 SQLite 自动建库建表，零外部依赖）
OHMYDESK_WEB_DIR="src/admin-web/dist" \
cargo run -p server          # 监听 0.0.0.0:8765，审计落 ./ohmydesk.db
# 容器部署挂数据卷持久化：DATABASE_URL=sqlite:/app/data/ohmydesk.db
```

- **管理端**：浏览器开 `http://<服务器IP>:8765/`（UI、API、WS 同源）。
- **客户端 Agent**（需 X11 显示）——**推荐 .deb 安装**（终端用户无需懂命令行）：
  ```bash
  # 构建 deb（先 cargo build -p client --release）
  bash scripts/packaging/deb/build-deb.sh        # 产出 dist/linux/ohmydesk-client_*.deb
  sudo dpkg -i dist/linux/ohmydesk-client_*.deb   # 缺依赖：sudo apt-get -f install
  # 默认连接 wss://rc.guoziweb.com/ws；内网部署可编辑 /etc/ohmydesk/client.env
  ohmydesk-client-launch                 # 或 应用菜单「OhMyDesk 终端」
  ```
  开发期默认连接 `wss://rc.guoziweb.com/ws`；如需本地服务端：`OHMYDESK_SERVER="ws://127.0.0.1:8765/ws" cargo run -p client -- "张伟-财务部"`。
  起 ≥2 个不同名实例即可在管理端看到多台终端。
- **Windows 被控端**（从 Linux/WSL 交叉编译出 exe，远控 Windows 终端）：
  ```bash
  bash scripts/packaging/windows/build-windows.sh     # 产出 dist/windows/（独立 exe，无需装运行时）
  ```
  把整个 `dist/windows/` 拷到 Windows 机器 → 双击 `连接服务器.bat`（已内嵌 `wss://rc.guoziweb.com/ws`，
  可记事本改地址）→ 即注册为被控端，在管理后台远控该 Windows。Windows 真实截屏/键鼠注入开箱可用
  （xcap Windows Graphics Capture + enigo SendInput，无 WSLg 限制）。
  自定义地址：`OHMYDESK_SERVER="wss://<域名>/ws" bash scripts/packaging/windows/build-windows.sh`。
- **MCP Server**（stdio，供 Claude Desktop 等 MCP 客户端接入）：
  ```bash
  OHMYDESK_API_BASE="http://<服务器IP>:8765" \
  OHMYDESK_API_TOKEN="<管理端登录JWT>" \
  node src/mcp/dist/index.js
  ```

> 开发期前端单独跑（vite :5173）时，需设 `VITE_WS_URL=ws://127.0.0.1:8765/ws` 指回 server；单一 URL 部署无需此变量（自动同源派生）。

### 管理平台登录（公网暴露必备）

浏览器打开后需登录才能进入。**默认账号 `admin` / 默认密码 `OhMyDesk@2026`**，登录后在「系统设置」页可改账号密码（存 SQLite `settings` 表持久化）。

- 鉴权：JWT(HS256)。`/api/*` 需 `Authorization: Bearer <token>`；`/ws` 的 admin 连接需 `?token=<jwt>`（无/失效 token 以 close 1008 拒绝）。终端 Agent 注册无需登录。
- 模式 A 远控、批量截图、终端列表**只对已登录 admin 开放**——攻击者即使连上 WS 也拿不到列表、发不动远控。
- 部署 env（codex 容器注入）：`OHMYDESK_JWT_SECRET`（签名密钥，**生产务必设固定值**，否则重启踢登录）；凭据改动落 DB，无需 env。

## 验证状态（截至当前提交）

| 层 | 验证方式 | 结果 |
|----|---------|------|
| 协议契约 | `cargo test -p protocol` | 24 通过 |
| 客户端 net/几何/采集 | `cargo test -p client` | 21 通过（含 I3 截图回发契约） |
| 服务端会话/审计/路由/鉴权 | `cargo test -p server` | 15 通过（含 5 鉴权单测） |
| 全栈 lint | `cargo clippy --workspace` | 0 警告 |
| Web/MCP 类型 | `tsc --noEmit` | 0 错误 |
| I1 资产链路 | 双真实终端注册 + 信创推断 + 离线检测 | 通过 |
| I2 远控闭环 | node 双端探针（授权→键鼠→帧双向→拒连） | 通过 |
| I3 批量截图 | node 双 agent 探针（截图请求→双端回发→按 endpoint 入墙） | 通过 |
| I4 MCP 真实 HTTP | 真实 server 拉 2 终端/1 会话/4 审计 | 通过 |
| M4 审计落库 | SQLite 落审计 4 条 + 会话 1 条（含终态 UPDATE）+ 改密跨重启持久化 | 通过 |

手工（浏览器/真机）逐条验收见 [docs/06-测试/手工验收清单.md](docs/06-测试/手工验收清单.md)。

## 已知环境限制：截屏

- **WSL/WSLg 无法真实截屏**：WSLg 的 X server 不完整支持 X GetImage，全屏抓图报 `xcb protocol error`；远控画面/批量截图会取不到真实屏幕。**真实信创 X11 物理机无此问题**。
- 在 WSL 上验证「授权→画面→操作→断开」**整条链路**时，设 `OHMYDESK_FAKE_CAPTURE=1` 用合成占位帧（移动竖条+渐变，明确标记为占位、非真实屏幕）。真机部署默认留空走真实截屏。

## 安全约束

- **数据不出网**：仅内网；AI 问答基于本地 MCP 工具数据，不外发到第三方 LLM。
- **TLS**：服务端 + Linux/信创客户端用 `rustls`（ring provider，不引 openssl，规避 loongarch64 交叉编译坑）；
  Windows 客户端用系统 `SChannel`（native-tls，同样不引 openssl）。客户端连 `wss://` 时按目标自动选后端。
- **会话锁 X11**：截屏/注入仅在 X11 会话可靠。
