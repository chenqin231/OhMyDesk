// Real Transport：连接真实 WS server + fetch HTTP，集成期使用
// 接口签名与 mockTransport 完全相同，切换零改组件
import type { Transport, AuditQuery } from "./types";
import type { Envelope } from "@/lib/types/Envelope";
import type { AuditLog } from "@/lib/types/AuditLog";

let ws: WebSocket | null = null;

export const realTransport: Transport = {
  connect(selfId, onEnvelope) {
    ws = new WebSocket("ws://127.0.0.1:8765/ws");
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
      ws?.send(JSON.stringify(heartbeat, (_k, v) =>
        typeof v === "bigint" ? v.toString() : v
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
  },

  send(e) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify(e, (_k, v) =>
      typeof v === "bigint" ? v.toString() : v
    ));
  },

  async fetchAudit(q: AuditQuery): Promise<AuditLog[]> {
    const params = new URLSearchParams();
    if (q.endpoint) params.set("endpoint", q.endpoint);
    if (q.from !== undefined) params.set("from", String(q.from));
    if (q.to !== undefined) params.set("to", String(q.to));
    if (q.result) params.set("result", q.result);
    const res = await fetch(`/api/audit?${params.toString()}`);
    return res.json() as Promise<AuditLog[]>;
  },

  disconnect() {
    ws?.close();
    ws = null;
  },
};
