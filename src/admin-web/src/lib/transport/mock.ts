// Mock Transport：用 setInterval/setTimeout 模拟 server 推送，形状与 realTransport 完全相同
import type { Transport, AuditQuery } from "./types";
import type { Envelope } from "@/lib/types/Envelope";
import type { AuditLog } from "@/lib/types/AuditLog";
import {
  makeEndpoints,
  makeAuditLogs,
  makeMockFrameBase64,
  makeMockScreenshotBase64,
  makeSessions,
} from "@/lib/mock/data";

let activeSessionId: string | null = null;
let frameSeq = 0;
let frameTimer: ReturnType<typeof setInterval> | null = null;
const pendingScreenshots = new Map<string, string[]>(); // req_id → [endpoint_id]

export const mockTransport: Transport = {
  connect(selfId, onEnvelope) {
    const nowSec = () => Math.floor(Date.now() / 1000);

    // 立即推送全量终端列表
    setTimeout(() => {
      onEnvelope({
        from: "server",
        to: selfId,
        ts: BigInt(nowSec()),
        payload: {
          type: "endpoint_list",
          endpoints: makeEndpoints(nowSec()),
        },
      });
    }, 200);

    // 每 5s 更新 last_seen，偶尔翻转在线态
    const listTimer = setInterval(() => {
      const eps = makeEndpoints(nowSec());
      // 偶尔翻转第一台在线态（展示动态效果）
      if (Math.random() < 0.15) {
        eps[0] = { ...eps[0], online: !eps[0].online };
      }
      onEnvelope({
        from: "server",
        to: selfId,
        ts: BigInt(nowSec()),
        payload: { type: "endpoint_list", endpoints: eps },
      });
    }, 5000);

    // 保存 timer 供 disconnect 清理
    const internal2 = mockTransport as _MockInternal;
    internal2._timers = internal2._timers ?? [];
    internal2._timers.push(listTimer);
    (mockTransport as _MockInternal)._onEnvelope = onEnvelope;
    (mockTransport as _MockInternal)._selfId = selfId;
  },

  send(e) {
    const internal = mockTransport as _MockInternal;
    const onEnvelope = internal._onEnvelope;
    if (!onEnvelope) return;

    const nowSec = Math.floor(Date.now() / 1000);
    const payload = e.payload;

    if (payload.type === "connect_request") {
      const { mode, password, target } = payload;
      const sessionId = `ses-mock-${Date.now()}`;

      if (mode === "b" && password !== "123456") {
        // G-3：模式B密码错 → 拒连
        setTimeout(() => {
          onEnvelope({
            from: "server",
            to: internal._selfId ?? "admin",
            ts: BigInt(nowSec),
            payload: { type: "reject", session_id: sessionId, reason: "密码错误，连接被拒绝" },
          });
        }, 600);
        return;
      }

      // 授权通过：先 auth_result 再 connect_ack
      setTimeout(() => {
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(nowSec),
          payload: { type: "auth_result", session_id: sessionId, ok: true, reason: null },
        });
      }, 800);
      setTimeout(() => {
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(nowSec + 1),
          payload: { type: "connect_ack", session_id: sessionId },
        });
        // 开启帧流
        activeSessionId = sessionId;
        frameSeq = 0;
        frameTimer = setInterval(() => {
          if (!activeSessionId) return;
          const seq = frameSeq++;
          const data = makeMockFrameBase64(target, seq, Date.now());
          onEnvelope({
            from: "server",
            to: internal._selfId ?? "admin",
            ts: BigInt(Math.floor(Date.now() / 1000)),
            payload: { type: "frame", session_id: activeSessionId, data, w: 1280, h: 720, seq: BigInt(seq) },
          });
        }, 350);
        if (frameTimer) { internal._timers = internal._timers ?? []; internal._timers.push(frameTimer); }
      }, 1200);
      return;
    }

    if (payload.type === "session_end") {
      activeSessionId = null;
      if (frameTimer) { clearInterval(frameTimer); frameTimer = null; }
      return;
    }

    // 远程命令执行（mock 回执：echo 命令 + 退出码 0）
    if (payload.type === "exec_request") {
      const { session_id, exec_id, command } = payload;
      setTimeout(() => {
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(Math.floor(Date.now() / 1000)),
          payload: {
            type: "exec_result",
            session_id,
            exec_id,
            exit_code: 0,
            stdout: `[mock] $ ${command}\nOhMyDesk mock shell\n`,
            stderr: "",
            truncated: false,
            duration_ms: 42,
          },
        });
      }, 400);
      return;
    }

    // 文件取回（mock 回流一个小文本文件）
    if (payload.type === "file_pull_request") {
      const { session_id, transfer_id, path } = payload;
      const content = `mock file for ${path}\n`;
      const name = path.split(/[\\/]/).pop() || "file.txt";
      setTimeout(() => {
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(Math.floor(Date.now() / 1000)),
          payload: { type: "file_open", session_id, transfer_id, name, size: BigInt(content.length), dir: "pull", dest: null },
        });
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(Math.floor(Date.now() / 1000)),
          payload: { type: "file_chunk", session_id, transfer_id, seq: 0n, data: btoa(content), last: true },
        });
      }, 400);
      return;
    }

    // 文件下发首包：mock 模拟被控端落盘后回 file_done（最终路径 = dest 当前目录或 recv 目录）
    if (payload.type === "file_open") {
      if (payload.dir === "push") {
        const { session_id, transfer_id, name, dest } = payload;
        const baseDir = dest && dest.trim() ? dest : "/home/mock/.config/OhMyDesk/recv";
        const sep = baseDir.includes("\\") ? "\\" : "/";
        const finalPath = `${baseDir.replace(/[\\/]$/, "")}${sep}${name}`;
        setTimeout(() => {
          onEnvelope({
            from: "server",
            to: internal._selfId ?? "admin",
            ts: BigInt(Math.floor(Date.now() / 1000)),
            payload: { type: "file_done", session_id, transfer_id, path: finalPath },
          });
        }, 500);
      }
      return;
    }

    // 数据块 / 传输错误：mock 直接消费
    if (payload.type === "file_chunk" || payload.type === "file_error") {
      return;
    }

    // 远端目录浏览：mock 回一份固定的目录树
    if (payload.type === "file_list_request") {
      const { session_id, transfer_id, path } = payload;
      const dir = path && path.trim() ? path : "/home/mock";
      setTimeout(() => {
        onEnvelope({
          from: "server",
          to: internal._selfId ?? "admin",
          ts: BigInt(Math.floor(Date.now() / 1000)),
          payload: {
            type: "file_list_resp",
            session_id,
            transfer_id,
            path: dir,
            entries: [
              { name: "Documents", is_dir: true, size: 0n },
              { name: "Downloads", is_dir: true, size: 0n },
              { name: "Desktop", is_dir: true, size: 0n },
              { name: "report.xlsx", is_dir: false, size: 20480n },
              { name: "notes.txt", is_dir: false, size: 512n },
            ],
            error: null,
          },
        });
      }, 350);
      return;
    }

    if (payload.type === "screenshot_req") {
      const { req_id } = payload;
      const nowSec2 = Math.floor(Date.now() / 1000);
      const eps = makeEndpoints(nowSec2).filter((ep) => ep.online);
      // 每台在线终端随机延迟 200-800ms 各推一张截图
      eps.forEach((ep, i) => {
        const delay = 200 + Math.floor(Math.random() * 600) + i * 50;
        setTimeout(() => {
          const data = makeMockScreenshotBase64(ep.info.id, Date.now());
          onEnvelope({
            from: "server",
            to: internal._selfId ?? "admin",
            ts: BigInt(Math.floor(Date.now() / 1000)),
            payload: {
              type: "screenshot_resp",
              req_id,
              endpoint_id: ep.info.id,
              data,
              w: 1920,
              h: 1080,
            },
          });
        }, delay);
      });
      pendingScreenshots.set(req_id, eps.map((ep) => ep.info.id));
      return;
    }

    if (payload.type === "input") {
      // no-op，仅记录调试信息
      console.debug("[mockTransport] received input event:", payload.event);
    }
  },

  async fetchAudit(q: AuditQuery): Promise<AuditLog[]> {
    const nowSec = Math.floor(Date.now() / 1000);
    let logs = makeAuditLogs(nowSec);

    // G-4：时间范围筛选 today/3d/7d → from/to
    const from = q.from;
    const to = q.to;
    if (from !== undefined) logs = logs.filter((l) => Number(l.ts) >= from);
    if (to !== undefined) logs = logs.filter((l) => Number(l.ts) <= to);
    if (q.endpoint) logs = logs.filter((l) => l.session_id.includes(q.endpoint!));
    if (q.result) logs = logs.filter((l) => l.type === q.result);

    return logs;
  },

  async fetchSessions() {
    return makeSessions(Math.floor(Date.now() / 1000));
  },

  async deleteEndpoints(_ids: string[]): Promise<void> {
    // mock：无持久注册表，no-op（接口占位）
  },

  disconnect() {
    const internal = mockTransport as _MockInternal;
    (internal._timers ?? []).forEach(clearInterval);
    internal._timers = [];
    internal._onEnvelope = undefined;
    if (frameTimer) { clearInterval(frameTimer); frameTimer = null; }
    activeSessionId = null;
  },
};

// 内部状态（不暴露在 Transport 接口）
type _MockInternal = Transport & {
  _timers?: ReturnType<typeof setInterval>[];
  _onEnvelope?: (e: Envelope) => void;
  _selfId?: string;
};
