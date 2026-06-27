# Mock 接口契约 + 适配层方案 + 细化并行任务

> **目的（一句话）**：把 protocol 契约做成一套**可执行的 mock 接口**，前端对着它开发，用**适配层**消化「protocol↔原型」漂移；集成时只切数据源、零改组件。三方（需求/原型/设计）由此收敛到同一份可执行契约。
> **配套**：[三方一致性分析](./2026-06-27-tripartite-consistency-analysis.md)（分歧裁决）｜ [实现计划](../plans/2026-06-27-ohmydesk-mvp-implementation.md) ｜ [并行编排](../plans/2026-06-27-parallel-dev-orchestration.md)

---

## 0. 收敛架构：Transport 抽象（mock / real 可切换）

前端不直接碰 WS/fetch，统一走 `Transport` 接口。两种实现切换，组件无感：

```
组件 ──读──> store（zustand） ──消费──> Transport ──┬── mockTransport（本地造数据，Wave1 用）
  │                                                  └── realTransport（WS + fetch，集成用）
  └──渲染时经──> adapters/*（protocol 类型 → 原型视图）
```

```typescript
// src/lib/transport/types.ts —— 唯一对接口的抽象
import type { Envelope } from "@/lib/types/Envelope";
import type { AuditLog } from "@/lib/types/AuditLog";

export interface Transport {
  connect(selfId: string, onEnvelope: (e: Envelope) => void): void;
  send(e: Envelope): void;                       // connect_request / input / screenshot_req / session_end
  fetchAudit(q: AuditQuery): Promise<AuditLog[]>;
  disconnect(): void;
}
export type AuditQuery = { endpoint?: string; from?: number; to?: number; result?: string };

// 切换：import.meta.env.VITE_USE_MOCK === "1" ? mockTransport : realTransport
```

> **关键纪律**：mock 与 real **产出完全相同形状**（protocol 的 ts-rs 类型，snake_case + 嵌套）。前端只对 protocol 形状编程，原型自造类型一律弃用、走适配层。

---

## 1. Mock 接口契约（可执行行为规格）

### 1.1 WS mock 行为（`mockTransport` 实现）

mock 用 `setInterval`/`setTimeout` 模拟 server 推送。**信封一律 `{from,to,ts,payload}`，payload 内部 tag `type`。**

| 触发 | mock 行为 | 推送的 Message |
|------|----------|---------------|
| `connect(admin-*)` | 立即推一次全量列表 | `endpoint_list{ endpoints: EndpointView[6] }` |
| 每 5s | 更新 `last_seen` + 偶尔翻转一台 online | `endpoint_list{...}` |
| 收 `connect_request{mode,target,password}` | 模式B 且 `password!=="123456"` → 拒；否则授权通过并起帧流 | 拒：`reject{session_id,reason:"密码错误"}`；通过：`auth_result{session_id,ok:true}` → `connect_ack{session_id}` |
| 会话激活后每 ~350ms | 推一帧 mock 画面（见 1.3） | `frame{session_id,data,w:1280,h:720,seq++}` |
| 收 `input{...}` | no-op（可 `console.debug`） | — |
| 收 `screenshot_req{req_id}` | 对每台在线终端，延迟随机 200–800ms 各推一张 | `screenshot_resp{req_id,endpoint_id,data,w,h}` |
| 收 `session_end{session_id}` | 停止该会话帧流 | — |

### 1.2 HTTP mock 契约（`fetchAudit` + real 对齐的端点）

> real 端这些由 server `http.rs` 提供（编排必修项 M-SRV3：State 同时持 `Arc<Hub>`+`Db`）。mock 端用本地数据 generator 返回**相同形状**。

| 端点 | 返回（protocol 形状） | 说明 |
|------|---------------------|------|
| `GET /api/endpoints` | `EndpointView[]`（裸数组） | 读注册表实时态；MCP `list_endpoints` 也消费此契约（编排 P-MCP2） |
| `GET /api/sessions` | `Session[]`（status=active 的进行中会话） | M5 `get_active_sessions` |
| `GET /api/audit?endpoint=&from=&to=&result=` | `AuditLog[]`（**事件流**，非聚合） | 审计页前端聚合（见 §2.2） |

### 1.3 Mock 数据 generator 规格（`src/lib/mock/data.ts`）

**6 台终端**覆盖三方一致性要求的信创组合（字段 = protocol `EndpointView`，含 Wave0 拟补的 `department`）：

```typescript
// 形状即 ts-rs 生成的 EndpointView（snake_case + 嵌套），样例 1 台：
const ep0: EndpointView = {
  info: {
    id: "ep-001", name: "张伟", department: "财务部",        // department = Wave0 协议补充项(A-1)
    ip: "10.0.0.21", mac: "AA:BB:CC:00:00:21",
    os: { name: "银河麒麟 V10 SP3", kind: "kylin" },
    cpu: { model: "Loongson 3A5000", cores: 4, arch: "loongarch" },
    ram: { total: 17179869184, used: 6657331200 },          // 字节(16/6.2 GB)
    gpu: { model: "JJM7200", vram: 4294967296 }, agent_version: "0.1.0",
  },
  online: true, last_seen: /*由 generator 注入秒级 epoch*/ 0, xinchuang: "信创·麒麟·龙芯",
};
// 其余 5 台：统信·鲲鹏(在线)、Win·x86(在线)、麒麟·鲲鹏(在线)、统信·龙芯(离线)、Win·x86(离线)
```

> generator **不得**用 `v0/lib/*.ts` 的原型类型——一律按 ts-rs 形状造，否则 mock 失去「契约镜像」意义。`last_seen` 由 generator 用注入的时间戳生成（脚本里禁用 `Date.now()`，从入参传）。

**审计事件流**（`AuditLog[]`，~30 条，覆盖多会话）：每个会话产生 `connect → screenshot×N → input(聚合一条) → disconnect`；另造 2 条 `auth_fail`/`reject`（模式B 密码错）。type 取值 = spec 集合 `connect|auth_fail|reject|screenshot|input|disconnect`（**统一裁决 C-1，删除原型的 transfer/error**）。

**mock 画面帧**：前端用 canvas 画一张测试图（深色底 + "MOCK FRAME ep-xxx seq:N 时间戳" + 移动方块），`toDataURL("image/jpeg")` 取 base64。避免引外部图，且能直观看到帧在刷新。

---

## 2. 适配层方案（protocol 类型 → 原型视图）

> 落 `src/lib/adapters/`。**消化三方一致性分析 §2.3 的 8 项漂移**。每个实体一个纯函数，组件渲染时调用，不改 ts-rs 生成物。

### 2.1 终端：`EndpointView → TerminalRow`（消化 D-1~D-5）

```typescript
// src/lib/adapters/endpoint.ts
import type { EndpointView } from "@/lib/types/EndpointView";

export type TerminalRow = {           // 原型表格/抽屉期望的扁平视图
  id: string; status: "online" | "offline"; user: string; department: string;
  ip: string; mac: string;
  osKey: "kylin" | "uos" | "windows" | "linux" | "other"; osName: string;
  arch: "loongarch" | "aarch64" | "x86_64" | "other";
  cpuModel: string; cpuCores: number;
  memUsedGb: number; memTotalGb: number;
  gpuModel: string | null; gpuVramGb: number | null;
  lastSeenText: string; xinchuang: string;
  // 注意：不含 connectPassword —— 裁决 O-3，资产视图不暴露密码
};

const B = 1024 ** 3;
export function endpointToRow(e: EndpointView, nowSec: number): TerminalRow {
  return {
    id: e.info.id,
    status: e.online ? "online" : "offline",          // D-2 bool→枚举
    user: e.info.name, department: e.info.department ?? "—",
    ip: e.info.ip, mac: e.info.mac,
    osKey: e.info.os.kind, osName: e.info.os.name,     // D-1 嵌套→扁平
    arch: e.info.cpu.arch, cpuModel: e.info.cpu.model, cpuCores: e.info.cpu.cores,
    memUsedGb: +(e.info.ram.used / B).toFixed(1),      // D-3 字节→GB
    memTotalGb: +(e.info.ram.total / B).toFixed(1),
    gpuModel: e.info.gpu?.model ?? null,
    gpuVramGb: e.info.gpu?.vram ? +(e.info.gpu.vram / B).toFixed(1) : null,
    lastSeenText: relTime(e.last_seen, nowSec),        // D-4 epoch→相对时间
    xinchuang: e.xinchuang,                            // 后端已算好，前端不再自算 domestic
  };
}
// 展示字典保留为前端常量（protocol 只给 kind/arch 字面量，图标/中文名前端映射）：
export const OS_LABEL = { kylin: "银河麒麟", uos: "统信 UOS", windows: "Windows", linux: "Linux", other: "其他" };
export const ARCH_LABEL = { loongarch: "龙芯 LoongArch", aarch64: "鲲鹏 aarch64", x86_64: "x86_64", other: "其他" };
```

### 2.2 审计：`AuditLog[] + Session[] → AuditRecord[]`（消化 D-7/D-8，**最重**）

```typescript
// src/lib/adapters/audit.ts —— 事件流 → 会话聚合视图
import type { AuditLog } from "@/lib/types/AuditLog";
import type { Session } from "@/lib/types/Session";

export type TimelineItem = { ts: number; kind: AuditLog["type"]; text: string };
export type AuditRecord = {
  sessionId: string; actor: string; target: string;
  mode: "A" | "B";                                   // D-6 展示用大写
  result: "active" | "success" | "rejected" | "auth_failed";
  startText: string; durationText: string; summary: string;
  timeline: TimelineItem[];
};

export function aggregate(logs: AuditLog[], sessions: Session[]): AuditRecord[] {
  const bySession = groupBy(logs, l => l.session_id);
  return sessions.map(s => {
    const items = (bySession[s.id] ?? []).sort((a, b) => a.ts - b.ts)
      .map(l => ({ ts: l.ts, kind: l.type, text: l.text }));
    return {
      sessionId: s.id, actor: s.from_id, target: s.to_id,
      mode: s.mode.toUpperCase() as "A" | "B",        // D-6 小写→大写
      result: deriveResult(s, items),                 // D-8: status + log 推 success/rejected/auth_failed/active
      startText: fmtTime(s.start_at),
      durationText: s.end_at ? fmtDur(s.end_at - s.start_at) : "进行中",
      summary: summarize(items),                      // "截图 2 次，输入操作 47 次"
      timeline: items,
    };
  });
}
// deriveResult: status==="rejected" → 看是否有 auth_fail 日志 → "auth_failed" 否则 "rejected"；
//               status==="active" → "active"；status==="ended" → "success"
// summarize: 统计 screenshot 条数 + input 聚合 text（input 已是 server 聚合的一条"输入操作 N 次"）
```

### 2.3 截图/帧（消化数据源差异）

```typescript
// src/lib/adapters/media.ts
export const screenshotSrc = (r: { data: string }) => `data:image/jpeg;base64,${r.data}`;
export const frameSrc      = (f: { data: string }) => `data:image/jpeg;base64,${f.data}`;
```

### 2.4 砍原型项（裁决 O-1/O-2/O-3，适配层无对应即删 UI）
- **O-1** 审计 timeline 删 `transfer` 分支（adapter 的 `kind` 联合类型不含 transfer，原型相关渲染删）。
- **O-2** 远控页删「录制标记」组件（spec 不录像）。
- **O-3** 终端抽屉删「临时连接密码」字段（`TerminalRow` 无此字段）。

---

## 3. 三方收敛检查表（这套契约如何保证一致）

| 三方分歧（来自一致性分析） | 本契约如何消化 | 落点 |
|---|---|---|
| 契约漂移 D-1~D-8 | 适配层 §2 全部映射 | `adapters/*` |
| 原型超范围 O-1~O-3 | 适配层不暴露 + 删 UI | §2.4 |
| 原型缺 G-1~G-5 | mock 接口提供帧/截图/拒连数据，组件补交互 | §1.1 + §4 任务 |
| audit type 三方枚举不一 (C-1) | mock/适配层**统一用 spec 集合** | §1.3 + §2.2 |
| department (A-1) | 契约/mock **纳入** `info.department` | §1.3 |
| 原型 mock 类型自造 | **弃用**，全部走 protocol 形状 + 适配层 | §0 纪律 |

> **唯一事实源**：本文件的 mock 接口形状 == protocol ts-rs 类型。protocol 改（如 Wave0 补 department / 统一 audit type），mock generator 与 adapters 跟着改，**组件不动**。

---

## 4. 细化 Agents Team 并行开发任务

> 在编排（Wave0→1→2→3）基础上细化到可认领 task。**mock 契约（本文件）让 frontend / mcp 在 Wave0 后即可全速并行，不等 server。**

### Wave 0（protocol-owner，串行，barrier）
| Task | 产出 | 验收 |
|------|------|------|
| W0-T1 | Phase 0 协议 crate + **补 `info.department`(A-1)** + **audit type 枚举统一含 input、删 click/transfer(C-1)** | `cargo test -p protocol` 绿 |
| W0-T2 | ts-rs 导出 TS 类型到 `apps/admin-web/src/lib/types/` | `.ts` 生成、无遗漏依赖类型 |
| W0-T3 | 冻结公告 + 根 `Cargo.toml` 配齐四线依赖（含 rand/sqlx features） | 四线可开工 |

### Wave 1（四线并行，protocol 冻结后）

**① server-dev（瓶颈，优先）**
| Task | 产出 | 必修项 |
|------|------|-------|
| S-T1 | Phase1 registry/hub/列表广播 + `register_ack`(W0-1) | M-SRV2 CORS |
| S-T2 | Phase4 server：session A/B 鉴权 + Frame/Input 路由 | M-SRV4 input bump |
| S-T3 | Phase5 server：screenshot 广播+聚合 | — |
| S-T4 | Phase6：db.rs(降级 None) + audit(event_type) + http.rs(State 双源) | B-DB1/M-SRV1/M-SRV3/P-MCP2 |
| S-T5 | 验收：websocat 脚本驱动跑通注册→列表→会话→截图→审计落库 | 不依赖前端 |

**② client-dev**
| Task | 产出 | 必修项 |
|------|------|-------|
| C-T1 | Phase2：asset 采集 + net(mpsc 泵/重连/helper) + 最小 Slint UI | M-CLI1/2/3 |
| C-T2 | Phase4 client：capture(等比缩放+真实 w/h) + inject(按 frame_w 缩放) + Slint 贴帧 + 授权弹窗 | P-CLI4 |
| C-T3 | 验收：连本地 server 跑通注册→被控截屏→收 input 注入 | — |

**③ frontend-dev（mock 契约就绪即可全速，不等 server）**
| Task | 产出 | 依赖 |
|------|------|------|
| F-T1 | 建 Vite 工程 + 整套迁移 `v0/`（去 Next/Base UI 照搬/Tailwind v4/字体本地化） | W0-T2 类型 |
| F-T2 | `Transport` 抽象 + `mockTransport` + mock data generator（§0/§1） | F-T1 |
| F-T3 | 适配层 `adapters/*`（§2）+ store 接 Transport | F-T2 |
| F-T4 | 五页接 mock：Assets/Grid 先通（§2.1）；Audit 聚合（§2.2）；**砍 O-1/O-2/O-3** | F-T3 |
| F-T5 | **补缺口**：G-1 帧渲染 canvas、G-2 键鼠监听+发 Input、G-3 模式B拒连态、G-4 审计时间筛选 | F-T4 |
| F-T6 | Assistant：保留 UI，接 mock 应答或降级脚本(G-5) | F-T4 |
| F-T7 | 验收：`VITE_USE_MOCK=1` 五页全可视、交互闭环、无 console 报错 | — |

**④ mcp-dev（mock HTTP 契约就绪即可并行）**
| Task | 产出 | 依赖 |
|------|------|------|
| M-T1 | 锁 SDK 版本 + 4 个只读 tool 打 mock `/api/*`（§1.2 契约，P-MCP1） | §1.2 |
| M-T2 | inspector 验证 tool 结构 | — |

### Wave 2（滚动集成，integrator）
| 里程碑 | 集成 = 把各线 `VITE_USE_MOCK→0` / 连真实 server | 验收 |
|--------|------|------|
| I1 = M1 | client 注册 + admin `realTransport` 收 endpoint_list | §7-1 |
| I2 = M2 | 远控闭环（帧/注入/AB/拒连）；模式B = **client→client（P0，P-DOC1 修正）**，Web 主控兜底 | §7-2/3 |
| I3 = M3+M4 | 批量截图真实 + 审计真实（input 计数 M-SRV4） | §7-4/5 |
| I4 = M5 | mcp 切真实 server HTTP + AI 问答 | §7-6 |

> **集成零改组件**：因 mock/real 同形状（protocol），切换只改 `VITE_USE_MOCK` + adapter 已消化漂移。集成期只调 transport 与 server，不动 UI。

### Wave 3（收尾，integrator）
ServeDir 托管 admin/dist(P-SRV5) ｜ 清理 design 残留(C-2 SQLite/C-3 Tauri/C-4 §8) ｜ 彩排 + 兜底。

---

## 5. 启动顺序

```
Wave0(协议+department+audit type 定死) → ┬ server-dev(S-T1..5)
                                          ├ client-dev(C-T1..3)
                                          ├ frontend-dev(F-T1..7)  ← 靠 mock 契约全速，不等 server
                                          └ mcp-dev(M-T1..2)        ← 靠 mock HTTP，不等 server
                                          → Wave2 滚动集成(切 real) → Wave3 收尾
```

*下一步：执行 Wave0（含 department / audit type 裁决并入协议），随后四线按本细化任务并行。*
