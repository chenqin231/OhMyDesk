/**
 * Wave 1 mock 数据——替代真实 HTTP
 *
 * Wave 2 迁移指引：整块替换为真实 fetch client：
 *   const BASE_URL = process.env.OHMYDESK_API_BASE ?? "http://127.0.0.1:8765";
 *   export async function fetchEndpoints() { ... fetch(`${BASE_URL}/api/endpoints`) }
 *   export async function fetchSessions()  { ... fetch(`${BASE_URL}/api/sessions`)  }
 *   export async function fetchAudit(...)  { ... fetch(`${BASE_URL}/api/audit?...`) }
 * 保留本文件中的 toJson 工具函数（bigint 序列化需求不变）。
 */

import type { AuditLog, EndpointView, Session } from "./types.js";

// ---------- JSON 序列化辅助（bigint → string）----------

export function toJson(value: unknown): string {
  return JSON.stringify(
    value,
    (_key, val) => (typeof val === "bigint" ? val.toString() : val),
    2,
  );
}

// ---------- mock 终端列表（GET /api/endpoints → EndpointView[]）----------

export const MOCK_ENDPOINTS: EndpointView[] = [
  {
    info: {
      id: "ep-001",
      name: "kylin-workstation-01",
      department: "研发部",
      ip: "192.168.1.101",
      mac: "aa:bb:cc:dd:ee:01",
      os: { name: "银河麒麟 V10 SP1", kind: "kylin" },
      cpu: { model: "飞腾 FT-2000+", cores: 64, arch: "aarch64" },
      ram: { total: BigInt(34359738368), used: BigInt(17179869184) },
      gpu: null,
      agent_version: "0.1.0",
    },
    online: true,
    last_seen: BigInt(Date.now()),
    xinchuang: "芯片:飞腾/OS:麒麟",
  },
  {
    info: {
      id: "ep-002",
      name: "uos-desktop-02",
      department: "运营部",
      ip: "192.168.1.102",
      mac: "aa:bb:cc:dd:ee:02",
      os: { name: "统信 UOS 专业版 1060", kind: "uos" },
      cpu: { model: "龙芯 3A5000", cores: 4, arch: "loong_arch" },
      ram: { total: BigInt(17179869184), used: BigInt(8589934592) },
      gpu: { model: "AMD Radeon RX 580", vram: BigInt(8589934592) },
      agent_version: "0.1.0",
    },
    online: true,
    last_seen: BigInt(Date.now() - 5000),
    xinchuang: "芯片:龙芯/OS:统信",
  },
  {
    info: {
      id: "ep-003",
      name: "win-legacy-03",
      department: "财务部",
      ip: "192.168.1.103",
      mac: "aa:bb:cc:dd:ee:03",
      os: { name: "Windows 10 LTSC", kind: "windows" },
      cpu: { model: "Intel Core i7-8700", cores: 6, arch: "x86_64" },
      ram: { total: BigInt(17179869184), used: BigInt(6442450944) },
      gpu: null,
      agent_version: "0.1.0",
    },
    online: false,
    last_seen: BigInt(Date.now() - 3600000),
    xinchuang: "非信创",
  },
];

// ---------- mock 会话列表（GET /api/sessions → 进行中会话）----------

export const MOCK_SESSIONS: Session[] = [
  {
    id: "sess-001",
    mode: "a",
    from_id: "admin-user-001",
    to_id: "ep-001",
    start_at: BigInt(Date.now() - 600000),
    end_at: null,
    status: "active",
  },
];

// ---------- mock 审计日志（GET /api/audit?... → AuditLog[]）----------

export const MOCK_AUDIT_LOGS: AuditLog[] = [
  {
    id: "log-001",
    session_id: "sess-001",
    ts: BigInt(Date.now() - 600000),
    actor_id: "admin-user-001",
    type: "connect",
    text: "管理员建立远控连接至 kylin-workstation-01",
  },
  {
    id: "log-002",
    session_id: "sess-002",
    ts: BigInt(Date.now() - 7200000),
    actor_id: "admin-user-002",
    type: "auth_fail",
    text: "密码错误，连接 uos-desktop-02 失败",
  },
  {
    id: "log-003",
    session_id: "sess-003",
    ts: BigInt(Date.now() - 86400000),
    actor_id: "admin-user-001",
    type: "disconnect",
    text: "管理员正常断开与 win-legacy-03 的连接",
  },
];
