// Real Transport：连接真实 WS server + fetch HTTP，集成期使用
// 接口签名与 mockTransport 完全相同，切换零改组件
import type { Transport, AuditQuery } from "./types";
import type { Envelope } from "@/lib/types/Envelope";
import type { AuditLog } from "@/lib/types/AuditLog";
import { getToken, useAuthStore } from "@/store/auth";

let ws: WebSocket | null = null;

// 拼接同源 API 地址：默认相对路径（生产由 server 托管）；
// 开发期可用 VITE_API_BASE 指向 http://127.0.0.1:8765 跨端口联调。
function apiUrl(path: string): string {
  const base = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";
  return `${base}${path}`;
}

// 401 统一处理：清 token + 跳登录页。WS 与所有 /api/* fetch 共用。
function onUnauthorized() {
  useAuthStore.getState().logout();
  if (window.location.pathname !== "/login") {
    window.location.href = "/login";
  }
}

// WS 地址同源派生（P-SRV5：admin 由 server 静态托管，单一内网 URL）。
// 优先 VITE_WS_URL（开发期跨端口连 :8765 用）；否则按当前页面 origin 推导 ws(s)://host/ws。
// 浏览器 WS 不能设 header，token 以 query 传递（?token=<jwt>）。
function wsUrl(): string {
  const override = import.meta.env.VITE_WS_URL as string | undefined;
  const base =
    override ??
    `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}/ws`;
  const token = getToken();
  if (!token) return base;
  return `${base}${base.includes("?") ? "&" : "?"}token=${encodeURIComponent(token)}`;
}

export const realTransport: Transport = {
  connect(selfId, onEnvelope) {
    ws = new WebSocket(wsUrl());
    ws.onopen = () => {
      // admin 首条消息登记 admin-* conn id，触发 server 推送 endpoint_list
      const heartbeat: Envelope = {
        from: selfId,
        to: null,
        ts: BigInt(Math.floor(Date.now() / 1000)),
        payload: {
          type: "heartbeat",
          id: selfId,
          ram: { total: BigInt(0), used: BigInt(0) },
        },
      };
      // bigint→Number（非 toString）：server 的 Envelope.ts/ram 是 i64/u64，serde 拒绝字符串数字
      ws?.send(JSON.stringify(heartbeat, (_k, v) =>
        typeof v === "bigint" ? Number(v) : v
      ));
    };
    ws.onmessage = (ev) => {
      try {
        const raw = JSON.parse(ev.data as string);
        // bigint 字段由 server 以字符串或数字发送，需还原
        onEnvelope(raw as Envelope);
      } catch {
        // 忽略解析错误
      }
    };
    // server 对无/失效 token 的 WS 用 1008（Policy Violation）关闭：清 token 跳登录
    ws.onclose = (ev) => {
      if (ev.code === 1008) onUnauthorized();
    };
  },

  send(e) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    // bigint→Number（非 toString）：对齐 server serde i64/u64（ts/ram/seq 值 < 2^53 安全）
    ws.send(JSON.stringify(e, (_k, v) =>
      typeof v === "bigint" ? Number(v) : v
    ));
  },

  async fetchAudit(q: AuditQuery): Promise<AuditLog[]> {
    const params = new URLSearchParams();
    if (q.endpoint) params.set("endpoint", q.endpoint);
    if (q.from !== undefined) params.set("from", String(q.from));
    if (q.to !== undefined) params.set("to", String(q.to));
    if (q.result) params.set("result", q.result);
    const token = getToken();
    const res = await fetch(apiUrl(`/api/audit?${params.toString()}`), {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    });
    if (res.status === 401) {
      onUnauthorized();
      return [];
    }
    return res.json() as Promise<AuditLog[]>;
  },

  disconnect() {
    ws?.close();
    ws = null;
  },
};
