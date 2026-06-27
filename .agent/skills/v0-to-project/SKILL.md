---
name: v0-to-project
description: 把 v0.dev 生成的前端代码接入 OhMyDesk 管理端（apps/admin-web）时使用。提供去 Next.js 化、剥离 mock 接 store、对齐 ts-rs 生成类型、shadcn 组件落位、深色 token 对齐的逐步 checklist 与常见坑速查。触发场景：从 v0 复制/拉取页面或组件代码、把静态界面接上真实 WS/HTTP 数据。
---

# v0 产物接入 OhMyDesk 管理端

> **为什么有这个 skill**：v0 生成的是 Next.js 风格的 React 代码，自带 mock 数据与 `next/*` 专属写法，直接粘进本项目（Vite + React）会编译失败或风格失控。本 skill 给出确定性的「转换 checklist」，把 v0 静态界面规范化为本项目可用、且接上真实数据的代码。

## 前提（红线）

- **v0 只用于 `apps/admin-web`（React Web）**。**绝不用于 Slint 客户端**（`crates/client`）——技术栈不兼容，v0 的 HTML/React 代码一行都进不了 `.slint`。客户端 UI 查 skill `rust-remote-control-stack`。
- 数据契约单一事实源是 `crates/protocol`，前端类型用 ts-rs 生成物（`src/lib/types/`），**不照搬 v0 自造的类型**。

## 接入 checklist（每粘一个 v0 页面/组件都走一遍）

- [ ] **1. 拉取**：优先 `pnpm dlx shadcn@latest add "<v0 生成链接>"`；或手动复制组件源码到对应位置。
- [ ] **2. 去 Next.js 化**（见下表逐项替换），删到 `pnpm build` 不再报 `next` 相关错误。
- [ ] **3. 剥离 mock**：删除 v0 内置的假数据数组/对象，改为从 `store`（zustand）读真实数据；列表/详情组件通过 props 或 store 选择器拿数据，**组件不自持服务端状态**。
- [ ] **4. 类型对齐**：把 v0 自造的 `interface`/`type` 替换为 `src/lib/types/` 下 ts-rs 生成的类型（`EndpointView`/`Envelope`/`Message`/`AuditLog` 等）；字段名以生成类型为准。
- [ ] **5. 落位**：页面 → `src/pages/`，shadcn 基础组件 → `src/components/ui/`，业务组合组件 → `src/components/`。
- [ ] **6. 深色 token 对齐**：确认沿用 v0 提示词 0 的色板（深空灰背景 `#0E1117`、卡片 `#161B22`、主色 `#2F81F7`、在线绿 `#3FB950`、离线灰 `#8B949E`、告警红 `#F85149`）；统一在 `index.css`/tailwind config，不在组件内散落硬编码色值。
- [ ] **7. 接数据并验证**：`pnpm dev` 起前端，连本地 server（`ws://127.0.0.1:8765/ws` + `http://127.0.0.1:8765/api/*`），确认真实数据渲染、无 console 报错。

## v0 → 本项目 替换速查（去 Next.js 化）

| v0 写法（Next.js） | 本项目替换（Vite + React） |
|---|---|
| `"use client"` 顶部指令 | 直接删除 |
| `import Image from "next/image"` + `<Image>` | `<img>`（或自封装），手动给 `width/height` |
| `import Link from "next/link"` | `react-router-dom` 的 `<Link>`，或普通 `<a>` |
| `next/font`、`next/head`、`next/navigation` | 删除；字体进 `index.css`，路由用 react-router |
| `app/page.tsx`/`layout.tsx` 文件约定 | 改成普通导出组件，挂到 `src/pages/` + 路由表 |
| `async function Page()`（RSC 服务端组件） | 改成普通函数组件，数据用 store/`useEffect` 拉 |
| `process.env.NEXT_PUBLIC_*` | `import.meta.env.VITE_*` |
| 内置 `const mockData = [...]` | 删除，改 store/props |
| `@/components/ui/...` 路径别名 | 确认 `tsconfig.json` + `vite.config.ts` 配了 `@` → `src` |

> 保留即可：`lucide-react` 图标、`tailwindcss`、`class-variance-authority`/`clsx`/`tailwind-merge`、Radix 底层（shadcn 依赖）——这些本项目同样使用。
> 注意 Tailwind 大版本：若 v0 产物按 Tailwind v4 写、项目用 v3（或反之），需对齐 `tailwind.config` 与 `@import`/`@tailwind` 指令，否则样式不生效。

## 页面 ↔ 数据映射（接哪份数据、消费哪些消息）

| 页面（`src/pages/`） | 模块 | 数据来源 | 关键消息/接口 |
|---|---|---|---|
| `Assets.tsx` | M1 终端资产 | `store.endpoints: EndpointView[]` | WS `endpoint_list` |
| `Grid.tsx` | M3 批量/截图墙 | `store.endpoints` + `store.screenshots` | WS `screenshot_req`/`screenshot_resp` |
| `Remote.tsx` | M2 远程控制 | `store.session` + `store.frame` | WS `connect_request`/`frame`/`input`/`session_end` |
| `Audit.tsx` | M4 会话审计 | `fetch /api/audit` → `AuditLog[]` | HTTP `/api/audit?endpoint=&from=&to=&result=` |
| `Assistant.tsx` | M5 AI 助手 | AI/MCP（独立链路） | MCP tools（经 server `/api/*`） |

## 自检（接入完成判定）

- [ ] `pnpm build` 通过，无 `next` 残留依赖（`package.json` 不含 `next`）。
- [ ] 页面渲染的是 store/接口的真实数据，搜不到遗留 mock。
- [ ] 实体字段引用的是 `src/lib/types/` 生成类型，无手写重复实体。
- [ ] 深色 token 统一，无散落硬编码色值。
- [ ] 相关规范见 `.agent/user.md` §代码规范 A 组。
