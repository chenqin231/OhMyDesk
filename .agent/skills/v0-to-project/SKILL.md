---
name: v0-to-project
description: 把 v0.dev 生成的前端代码接入 OhMyDesk 管理端（apps/admin-web）时使用。提供去 Next.js 化、剥离 mock 接 store、对齐 ts-rs 生成类型、shadcn 组件落位、深色 token 对齐的逐步 checklist 与常见坑速查。触发场景：从 v0 复制/拉取页面或组件代码、把静态界面接上真实 WS/HTTP 数据。
---

# v0 产物接入 OhMyDesk 管理端

> **为什么有这个 skill**：v0 生成的是 Next.js 风格的 React 代码，自带 mock 数据与 `next/*` 专属写法，直接粘进本项目（Vite + React）会编译失败或风格失控。本 skill 给出确定性的「转换 checklist」，把 v0 静态界面规范化为本项目可用、且接上真实数据的代码。

## 前提（红线）

- **v0 只用于 `apps/admin-web`（React Web）**。**绝不用于 Slint 客户端**（`crates/client`）——技术栈不兼容，v0 的 HTML/React 代码一行都进不了 `.slint`。客户端 UI 查 skill `rust-remote-control-stack`。
- 数据契约单一事实源是 `crates/protocol`（ts-rs 生成物在 `src/lib/types/`）。**但 v0 自造类型几乎一定与契约结构性漂移**（拍平 vs 嵌套、字符串枚举 vs bool、GB vs 字节、camelCase vs snake_case）——**不能直接把 v0 类型换成 ts-rs 类型，要写「适配层」把 `EndpointView`/`AuditLog` 映射成组件期望的视图**（见 §字段适配层）。

## 本项目现状：已有整套 v0 原型（`v0/`）

本项目的 v0 产物是**一整个 Next.js 工程**（`/data/code/OhMyDesk/v0/`，5 页 + 业务组件 + `lib/` mock + `components/ui/`），不是零散组件。**接入方式 = 整套迁移**（建 Vite 工程 → 搬 `app/*`→`pages/`、`components/`、`globals.css`、`components/ui/` 连依赖一起搬），**不是**逐个 `shadcn add` 链接。原型完整度：UI 层 90%+（深色 token/信创视觉零返工），但**全静态 mock、零真实数据**，M2 远控帧渲染/键鼠回传几乎要从零写。

## 接入 checklist（每粘一个 v0 页面/组件都走一遍）

- [ ] **1. 迁移源码**：整套搬 `v0/` → `apps/admin-web/`（`components/ui/` **连 `@base-ui/react` 依赖一起搬**，见 §底座漂移）；本项目**不用** `shadcn add` 重拉组件。
- [ ] **2. 去 Next.js 化**（见下表逐项替换），删到 `pnpm build` 不再报 `next` 相关错误。
- [ ] **3. 剥离 mock**：删除 v0 内置的假数据数组/对象，改为从 `store`（zustand）读真实数据；列表/详情组件通过 props 或 store 选择器拿数据，**组件不自持服务端状态**。
- [ ] **4. 类型适配（非直接替换）**：v0 自造类型与 ts-rs 生成类型结构漂移，写**适配函数**把 ts-rs 类型映射成组件视图（如 `EndpointView → 行视图`：online bool→badge、`info.cpu/ram` 拍平、字节→GB、`last_seen` epoch→相对时间）。展示字典（OS 图标/中文名映射）保留为前端常量。详见 §字段适配层。
- [ ] **5. 落位**：页面 → `src/pages/`，shadcn 基础组件 → `src/components/ui/`，业务组合组件 → `src/components/`。
- [ ] **6. 深色 token 对齐**：确认沿用 v0 提示词 0 的色板（深空灰背景 `#0E1117`、卡片 `#161B22`、主色 `#2F81F7`、在线绿 `#3FB950`、离线灰 `#8B949E`、告警红 `#F85149`）；统一在 `index.css`/tailwind config，不在组件内散落硬编码色值。
- [ ] **7. 接数据并验证**：`pnpm dev` 起前端，连本地 server（`ws://127.0.0.1:8765/ws` + `http://127.0.0.1:8765/api/*`），确认真实数据渲染、无 console 报错。

## v0 → 本项目 替换速查（去 Next.js 化）

| v0 写法（Next.js） | 本项目替换（Vite + React） |
|---|---|
| `"use client"` 顶部指令 | 直接删除 |
| `import Image from "next/image"` + `<Image>` | `<img>`（或自封装），手动给 `width/height` |
| `import Link from "next/link"` | `react-router-dom` 的 `<Link>`，或普通 `<a>` |
| `next/font`（如 Geist） | **字体本地化**：信创内网拉不到 Google 字体 CDN，必须 `@fontsource` 或本地 `@font-face`，不能依赖 `next/font` 远程拉取 |
| `next/head`、`next/navigation` | 删除；路由用 react-router（`useLocation`/`<Link to>`） |
| `app/page.tsx`/`layout.tsx` 文件约定 | 改成普通导出组件，挂到 `src/pages/` + 路由表 |
| `async function Page()`（RSC 服务端组件） | 改成普通函数组件，数据用 store/`useEffect` 拉 |
| `process.env.NEXT_PUBLIC_*` | `import.meta.env.VITE_*` |
| 内置 `const mockData = [...]` | 删除，改 store/props |
| `@/components/ui/...` 路径别名 | 确认 `tsconfig.json` + `vite.config.ts` 配了 `@` → `src` |

> 保留即可：`lucide-react` 图标、`tailwindcss`、`class-variance-authority`/`clsx`/`tailwind-merge`、**UI 底座库（本项目是 `@base-ui/react`，不是 Radix）**——连依赖一起带过去。
> **Tailwind 版本**：本项目 v0 原型用 **Tailwind v4**（`@import 'tailwindcss'` + `@theme inline`，配置内联进 CSS，无 `tailwind.config.js`）。**admin-web 直接建成 v4**，`globals.css` 几乎原样搬（深色 token 已符合色板，零返工），不要降级回 v3。

## ⚠️ 底座漂移：UI 可能是 Base UI 而非 Radix

本项目原型的 `components/ui/*` 基于 **`@base-ui/react`**（shadcn `base-nova` 风格，`components.json` 的 `style: "base-nova"`），用 `render={...}` prop + `useRender`/`mergeProps`，**与标准 shadcn（Radix `asChild`）API 不兼容**。

- **整套照搬** `components/ui/` + `@base-ui/react` 依赖，**绝不用 `pnpm dlx shadcn add` 重拉 Radix 版**——业务组件大量用 `render={}`、函数式 `SelectValue` children，换 Radix 会让五页交互全部重写。
- `lucide-react` 注意版本（原型较新，个别图标名需确认存在）。
- 保留 base-nova 的按钮图标约定（`data-icon` 等）对应 CSS。

## 字段适配层（v0 类型 → protocol 契约）

v0 类型把 protocol 的嵌套结构拍平、改了命名/单位/大小写。**每个实体写一个适配函数**，不改 ts-rs 生成物：

| 实体 | 主要漂移 | 适配策略 |
|------|---------|---------|
| 终端 ↔ `EndpointView` | 扁平 vs `info.os.kind`/`info.cpu.arch`；`status:string` vs `online:bool`；`memGb` vs `ram`字节；`user` vs `info.name`；camel vs snake | `viewFromEndpoint(EndpointView)→行视图`；字节÷1024³；epoch→相对时间；**抽屉删「临时密码」展示**（`EndpointView` 不下发密码） |
| 监控 MonitorTerminal | Terminal 子集 + `desktop` 静态路径 | 删独立类型，复用 `EndpointView` 过 online；`desktop` 换 `ScreenshotResp.data`(base64) |
| 审计 ↔ `AuditLog` | **最重**：protocol 是 `AuditLog[]` **事件流**，原型要**会话级聚合视图**（一条=一会话+内嵌 timeline） | 写「`AuditLog[]` 按 `session_id` 分组→拼 timeline→算 summary/duration」聚合适配；或 server 加会话聚合接口。注意 `mode` 大写 vs serde 小写、原型有 spec 不做的 `transfer` 类型 |
| 聊天 ChatMessage | 无 protocol 对应（AI 链路独立） | 纯展示类型，**原样保留**，不对齐 ts-rs |

## 页面 ↔ 数据映射（接哪份数据、消费哪些消息）

| 页面（`src/pages/`） | 模块 | 数据来源 | 关键消息/接口 |
|---|---|---|---|
| `Assets.tsx` | M1 终端资产 | `store.endpoints: EndpointView[]` | WS `endpoint_list` |
| `Grid.tsx` | M3 批量/截图墙 | `store.endpoints` + `store.screenshots` | WS `screenshot_req`/`screenshot_resp` |
| `Remote.tsx` | M2 远程控制 | `store.session` + `store.frame` | WS `connect_request`/`frame`/`input`/`session_end` |
| `Audit.tsx` | M4 会话审计 | `fetch /api/audit` → `AuditLog[]`，**前端聚合成会话视图**（见 §字段适配层）；补时间范围筛选（原型有 UI 无逻辑） | HTTP `/api/audit?endpoint=&from=&to=&result=` |
| `Assistant.tsx` | M5 AI 助手 | AI/MCP（独立链路） | MCP tools（经 server `/api/*`） |

## 自检（接入完成判定）

- [ ] `pnpm build` 通过，无 `next` 残留依赖（`package.json` 不含 `next`）。
- [ ] 页面渲染的是 store/接口的真实数据，搜不到遗留 mock。
- [ ] 实体字段引用的是 `src/lib/types/` 生成类型，无手写重复实体。
- [ ] 深色 token 统一，无散落硬编码色值。
- [ ] 相关规范见 `.agent/user.md` §代码规范 A 组。
