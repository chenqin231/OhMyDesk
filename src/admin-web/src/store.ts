// Zustand store：终端列表 / 会话 / 审计状态
import { create } from "zustand";
import type { EndpointView } from "@/lib/types/EndpointView";
import type { AuditLog } from "@/lib/types/AuditLog";
import type { Session } from "@/lib/types/Session";
import type { Message } from "@/lib/types/Message";
import { transport } from "@/lib/transport";
import { makeSessions } from "@/lib/mock/data";

// 截图缓存：req_id → { endpoint_id: base64 }
export type ScreenshotCache = Record<string, Record<string, string>>;

// 远控帧：含 data(base64) + 分辨率
export type ActiveFrame = { data: string; w: number; h: number; seq: bigint };

type State = {
  // 终端列表（从 endpoint_list 推送更新）
  endpoints: EndpointView[];
  // 审计日志（fetchAudit 返回）
  auditLogs: AuditLog[];
  // 会话列表（mock 预生成）
  sessions: Session[];
  // 当前远控会话状态
  remoteSessionId: string | null;
  remoteTarget: string;           // 当前远控目标展示名（连接中/控制中卡片显示）
  remotePhase: "launch" | "connecting" | "connected" | "rejected";
  remoteFrame: ActiveFrame | null;
  remoteRejectReason: string | null;
  // Grid 截图缓存 { endpointId: base64 }
  screenshots: Record<string, string>;
  // 当前截图请求 id（用于匹配 screenshot_resp）
  activeReqId: string | null;

  // actions
  initTransport: () => void;
  disconnectTransport: () => void;
  sendEnvelope: (payload: Message) => void;
  fetchAudit: (from?: number, to?: number, endpoint?: string, result?: string) => Promise<void>;
  deleteEndpoints: (ids: string[]) => Promise<void>;
  requestBatchScreenshot: () => void;
  startRemote: (mode: "a" | "b", target: string, password: string | null, name?: string) => void;
  endRemote: () => void;
  resetRemote: () => void;
};

const selfId = "admin-" + Math.random().toString(36).slice(2, 8);

export const useStore = create<State>((set, get) => ({
  endpoints: [],
  auditLogs: [],
  sessions: makeSessions(Math.floor(Date.now() / 1000)),
  remoteSessionId: null,
  remoteTarget: "",
  remotePhase: "launch",
  remoteFrame: null,
  remoteRejectReason: null,
  screenshots: {},
  activeReqId: null,

  initTransport() {
    transport.connect(selfId, (env) => {
      const p = env.payload;

      if (p.type === "endpoint_list") {
        set({ endpoints: p.endpoints });
        return;
      }

      if (p.type === "screenshot_resp") {
        const { endpoint_id, data } = p;
        set((s) => ({
          screenshots: { ...s.screenshots, [endpoint_id]: data },
        }));
        return;
      }

      if (p.type === "frame") {
        set({ remoteFrame: { data: p.data, w: p.w, h: p.h, seq: p.seq } });
        return;
      }

      if (p.type === "auth_result") {
        // auth_result ok=true → 等 connect_ack
        if (!p.ok) {
          set({ remotePhase: "rejected", remoteRejectReason: p.reason ?? "鉴权失败" });
        }
        return;
      }

      if (p.type === "connect_ack") {
        set({ remoteSessionId: p.session_id, remotePhase: "connected" });
        return;
      }

      if (p.type === "reject") {
        set({ remotePhase: "rejected", remoteRejectReason: p.reason });
        return;
      }
    });

    // 预加载审计数据
    get().fetchAudit();
  },

  disconnectTransport() {
    transport.disconnect();
  },

  sendEnvelope(payload) {
    transport.send({
      from: selfId,
      to: null,
      ts: BigInt(Math.floor(Date.now() / 1000)),
      payload,
    });
  },

  async fetchAudit(from, to, endpoint, result) {
    const logs = await transport.fetchAudit({ from, to, endpoint, result });
    set({ auditLogs: logs });
  },

  async deleteEndpoints(ids) {
    await transport.deleteEndpoints(ids);
    // 乐观移除（server 删完也会 push 最新 endpoint_list 兜底）
    set((s) => ({ endpoints: s.endpoints.filter((e) => !ids.includes(e.info.id)) }));
  },

  requestBatchScreenshot() {
    const reqId = "req-" + Date.now();
    set({ activeReqId: reqId });
    get().sendEnvelope({ type: "screenshot_req", req_id: reqId });
  },

  startRemote(mode, target, password, name) {
    set({
      remotePhase: "connecting",
      remoteTarget: name ?? target,
      remoteRejectReason: null,
      remoteFrame: null,
    });
    get().sendEnvelope({
      type: "connect_request",
      mode,
      target,
      password,
    });
  },

  endRemote() {
    const sessionId = get().remoteSessionId;
    if (sessionId) {
      get().sendEnvelope({ type: "session_end", session_id: sessionId });
    }
    set({ remotePhase: "launch", remoteSessionId: null, remoteFrame: null });
  },

  resetRemote() {
    set({ remotePhase: "launch", remoteSessionId: null, remoteFrame: null, remoteRejectReason: null });
  },
}));
