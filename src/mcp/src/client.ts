/**
 * HTTP fetch 层——对接 OhMyDesk server 只读 API（axum, 默认 127.0.0.1:8765）
 *
 * 设计要点：
 *   - server 把 Rust i64/u64 序列化为 JSON number（非字符串/bigint），
 *     故 await res.json() 拿到的 ts/ram/last_seen/start_at 均为普通 number，
 *     运行时绝不构造 BigInt 与之比较（混合比较抛 TypeError）。
 *   - 任何诊断仅走 console.error（stderr）；stdout 被 MCP stdio 协议帧独占。
 *   - fetch 失败或非 2xx 一律优雅降级返回 []，server 没起也不崩。
 */

import type { AuditLog, EndpointView, Session } from "./types.js";

const BASE_URL = process.env.OHMYDESK_API_BASE ?? "http://127.0.0.1:8765";
const API_TOKEN = process.env.OHMYDESK_API_TOKEN?.trim();

// ---------- JSON 序列化辅助（bigint → string，数据现多为 number，保留以防万一）----------

export function toJson(value: unknown): string {
  return JSON.stringify(
    value,
    (_key, val) => (typeof val === "bigint" ? val.toString() : val),
    2,
  );
}

// ---------- 通用 GET：失败/非 2xx 时 console.error 并返回 fallback ----------

async function getJson<T>(path: string, fallback: T): Promise<T> {
  const url = `${BASE_URL}${path}`;
  try {
    const res = await fetch(url, {
      headers: API_TOKEN ? { Authorization: `Bearer ${API_TOKEN}` } : {},
    });
    if (!res.ok) {
      console.error(`[mcp] GET ${url} 非 2xx：${res.status} ${res.statusText}`);
      return fallback;
    }
    return (await res.json()) as T;
  } catch (err) {
    console.error(`[mcp] GET ${url} 失败：${String(err)}`);
    return fallback;
  }
}

// ---------- 三个只读端点 ----------

export async function fetchEndpoints(): Promise<EndpointView[]> {
  return getJson<EndpointView[]>("/api/endpoints", []);
}

export async function fetchSessions(): Promise<Session[]> {
  return getJson<Session[]>("/api/sessions", []);
}

export async function fetchAudit(p: {
  endpoint?: string;
  fromSec?: number;
  toSec?: number;
}): Promise<AuditLog[]> {
  const params = new URLSearchParams();
  if (p.endpoint) params.set("endpoint", p.endpoint);
  if (p.fromSec !== undefined) params.set("from", String(p.fromSec));
  if (p.toSec !== undefined) params.set("to", String(p.toSec));
  const qs = params.toString();
  return getJson<AuditLog[]>(`/api/audit${qs ? `?${qs}` : ""}`, []);
}
