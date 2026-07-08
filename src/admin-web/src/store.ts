// Zustand store：终端列表 / 会话 / 审计状态
import { create } from "zustand";
import { pushDiagRing, seqGap, type DiagSample } from "@/lib/diag-ring";
import type { EndpointView } from "@/lib/types/EndpointView";
import type { AuditLog } from "@/lib/types/AuditLog";
import type { Session } from "@/lib/types/Session";
import type { Message } from "@/lib/types/Message";
import type { FileEntry } from "@/lib/types/FileEntry";
import type { ResolutionTier } from "@/lib/types/ResolutionTier";
import type { ClarityTier } from "@/lib/types/ClarityTier";
import type { FpsTier } from "@/lib/types/FpsTier";
import { resolveDisplayParams } from "@/lib/quality";
import { rgbaToDataUrl } from "@/lib/cursor-overlay";
import { transport } from "@/lib/transport";
import {
  bytesToB64,
  b64ToBytes,
  downloadBytes,
  genId,
  CHUNK_SIZE,
  EXEC_TIMEOUT_MS,
} from "@/lib/file-transfer";
import { appendChat, type ChatEntry } from "@/lib/chat";
import {
  startProgress,
  advanceProgress,
  completeProgress,
  failProgress,
  type ProgressMap,
} from "@/lib/file-progress";

// 截图缓存：req_id → { endpoint_id: base64 }
export type ScreenshotCache = Record<string, Record<string, string>>;

// 远控帧：含 data(base64) + 分辨率
export type ActiveFrame = { data: string; w: number; h: number; seq: bigint };

// 光标同步：被控端下发的光标形状（RGBA 已转 dataURL）+ 热点 + 尺寸。主控在本地指针位置渲染。
export type RemoteCursorShape = {
  id: bigint;
  dataUrl: string;
  w: number;
  h: number;
  hotspotX: number;
  hotspotY: number;
};

// 一条命令执行记录（pending=等待被控端回执）
export type ExecEntry = {
  exec_id: string;
  command: string;
  pending: boolean;
  exit_code: number | null;
  stdout: string;
  stderr: string;
  truncated: boolean;
  duration_ms: number;
};

// 在途取回（pull）的二进制组装缓冲：transfer_id → 分片（不进 React state，纯瞬态）
const pullBuffers = new Map<string, { name: string; parts: Uint8Array[] }>();

type State = {
  // 终端列表（从 endpoint_list 推送更新）
  endpoints: EndpointView[];
  // 是否已收到过至少一帧 endpoint_list。用于区分「加载中」与「已加载但名下无终端」
  // （按 owner 隔离后普通账号名下可能为空，不能再把空列表当作加载中）。
  endpointsLoaded: boolean;
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

  // 远控会话内的命令执行记录（最近在前）
  execResults: ExecEntry[];
  // 文件传输提示（下发/取回的状态/错误）
  fileNotice: string | null;
  // 被控端会话内提示（如 Wayland 无法截屏）——主控端在等待画面处展示，替代「无限等待第一帧」
  remoteNotice: string | null;

  // 远端文件浏览：当前目录绝对路径 + 条目列表 + 加载/错误态
  remotePath: string;
  remoteEntries: FileEntry[];
  remoteListLoading: boolean;
  remoteListError: string | null;
  // 三轴显示参数（分辨率/清晰度/帧率）——主控选择，发 set_quality 给被控端
  remoteResolution: ResolutionTier;
  remoteClarity: ClarityTier;
  remoteFps: FpsTier;
  // 光标同步：被控端下发的当前光标形状（已转 dataURL）与可见性。主控在本地指针位置渲染此形状。
  // shape 仅在形状变化时下发，位置更新复用缓存 → 此处只在收到新 shape 时替换。
  remoteCursorShape: RemoteCursorShape | null;
  remoteCursorVisible: boolean;
  // 主控侧「控制光标」位置（帧坐标）：桌面由鼠标移动写入、手机由触控引擎写入；叠加层据此渲染真实光标形状。
  remoteCursorPos: { x: number; y: number } | null;

  // 会话内即时消息（时间正序，最新在末尾）
  chatMessages: ChatEntry[];
  // 文件传输进度：transfer_id → 进度条目（push + pull 共用）
  fileProgress: ProgressMap;
  // 远控诊断 ring（最近 5min 收帧标量指标，刷新即丢，脱敏不含像素）
  diagRing: DiagSample[];

  // actions
  initTransport: () => void;
  disconnectTransport: () => void;
  sendEnvelope: (payload: Message) => void;
  fetchAudit: (from?: number, to?: number, endpoint?: string, result?: string) => Promise<void>;
  deleteEndpoints: (ids: string[]) => Promise<void>;
  requestBatchScreenshot: () => void;
  startRemote: (mode: "a" | "b", target: string, password: string | null, name?: string, force?: boolean) => void;
  endRemote: () => void;
  resetRemote: () => void;
  // 远控会话内：执行命令 / 下发文件 / 取回文件 / 浏览远端目录
  execCommand: (command: string) => void;
  pushFile: (file: File, dest?: string) => Promise<void>;
  pullFile: (path: string) => void;
  listRemote: (path: string) => void;
  setRemoteDisplayParams: (p: { resolution?: ResolutionTier; clarity?: ClarityTier; fps?: FpsTier }) => void;
  // 更新主控控制光标位置（帧坐标）→ 驱动光标叠加层渲染。
  setRemoteCursorPos: (p: { x: number; y: number } | null) => void;
  // 远控会话内：发送一条即时消息
  sendChat: (text: string) => void;
};

const selfId = "admin-" + Math.random().toString(36).slice(2, 8);

export const useStore = create<State>((set, get) => ({
  endpoints: [],
  endpointsLoaded: false,
  auditLogs: [],
  sessions: [],
  remoteSessionId: null,
  remoteTarget: "",
  remotePhase: "launch",
  remoteFrame: null,
  remoteRejectReason: null,
  screenshots: {},
  activeReqId: null,
  execResults: [],
  fileNotice: null,
  remoteNotice: null,
  remotePath: "",
  remoteEntries: [],
  remoteListLoading: false,
  remoteListError: null,
  remoteResolution: "r720p",
  remoteClarity: "standard",
  remoteFps: "smooth",
  remoteCursorShape: null,
  remoteCursorVisible: true,
  remoteCursorPos: null,
  chatMessages: [],
  fileProgress: {},
  diagRing: [],

  initTransport() {
    transport.connect(selfId, (env) => {
      const p = env.payload;

      if (p.type === "endpoint_list") {
        set({ endpoints: p.endpoints, endpointsLoaded: true });
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
        set((s) => {
          const seqNum = Number(p.seq);
          const lastSeq = s.remoteFrame ? Number(s.remoteFrame.seq) : null;
          const sample: DiagSample = { ts: Date.now(), seq: seqNum, seq_gap: seqGap(lastSeq, seqNum), w: p.w, h: p.h };
          return {
            remoteFrame: { data: p.data, w: p.w, h: p.h, seq: p.seq },
            diagRing: pushDiagRing(s.diagRing, sample, 300_000),
          };
        });
        return;
      }

      // 光标同步：被控端下发当前光标形状+可见性。shape 仅形状变化时带（主控换算为 dataURL 缓存），
      // 位置更新（shape=null）只刷新可见性、复用已缓存形状。渲染在本地指针位置由组件负责。
      if (p.type === "cursor_update") {
        set((s) => {
          let shape = s.remoteCursorShape;
          if (p.shape) {
            shape = {
              id: p.shape.id,
              w: p.shape.w,
              h: p.shape.h,
              hotspotX: p.shape.hotspot_x,
              hotspotY: p.shape.hotspot_y,
              dataUrl: rgbaToDataUrl(p.shape.rgba, p.shape.w, p.shape.h),
            };
          }
          return { remoteCursorShape: shape, remoteCursorVisible: p.visible };
        });
        return;
      }

      // 被控端会话内提示（如 Wayland 无法截屏）→ 在等待画面处展示
      if (p.type === "remote_notice") {
        set({ remoteNotice: p.text });
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

      // 被控端主动断开（点「我要断开」）→ server 转发 session_end 给本控制端：
      // 退出远控查看态并明确告知，避免「画面卡住、以为出问题」（issue#1b）。
      if (p.type === "session_end") {
        set({
          remotePhase: "rejected",
          remoteRejectReason: "对方已结束远程会话",
          remoteFrame: null,
          remoteCursorShape: null,
          remoteCursorVisible: true,
          remoteCursorPos: null,
          chatMessages: [],
        });
        return;
      }

      // 命令执行回执：按 exec_id 回填对应记录
      if (p.type === "exec_result") {
        set((s) => ({
          execResults: s.execResults.map((e) =>
            e.exec_id === p.exec_id
              ? {
                  ...e,
                  pending: false,
                  exit_code: p.exit_code,
                  stdout: p.stdout,
                  stderr: p.stderr,
                  truncated: p.truncated,
                  duration_ms: p.duration_ms,
                }
              : e,
          ),
        }));
        return;
      }

      // 取回（pull）回流首包：开缓冲 + 开进度条目
      if (p.type === "file_open") {
        pullBuffers.set(p.transfer_id, { name: p.name, parts: [] });
        set((s) => ({
          fileProgress: startProgress(s.fileProgress, {
            transfer_id: p.transfer_id,
            name: p.name,
            total: Number(p.size),
            dir: "pull",
          }),
        }));
        return;
      }

      // 取回数据块：累积 + 进度推进，末块触发浏览器下载并标完成
      if (p.type === "file_chunk") {
        const buf = pullBuffers.get(p.transfer_id);
        if (buf) {
          const bytes = p.data ? b64ToBytes(p.data) : new Uint8Array();
          if (p.data) buf.parts.push(bytes);
          set((s) => ({
            fileProgress: advanceProgress(s.fileProgress, p.transfer_id, bytes.length),
          }));
          if (p.last) {
            pullBuffers.delete(p.transfer_id);
            downloadBytes(buf.name, buf.parts);
            set((s) => ({ fileProgress: completeProgress(s.fileProgress, p.transfer_id) }));
          }
        }
        return;
      }

      // 传输失败：标进度失败 + 文字兜底
      if (p.type === "file_error") {
        pullBuffers.delete(p.transfer_id);
        set((s) => ({
          fileProgress: failProgress(s.fileProgress, p.transfer_id),
          fileNotice: `传输失败：${p.reason}`,
        }));
        return;
      }

      // push 下发落盘回执：标完成 + 文字告知最终绝对路径
      if (p.type === "file_done") {
        set((s) => ({
          fileProgress: completeProgress(s.fileProgress, p.transfer_id),
          fileNotice: `已下发到被控端：${p.path}`,
        }));
        return;
      }

      // 远端目录列表回流：刷新右侧文件浏览器
      if (p.type === "file_list_resp") {
        if (p.error) {
          set({ remoteListLoading: false, remoteListError: p.error });
        } else {
          set({
            remotePath: p.path,
            remoteEntries: p.entries,
            remoteListLoading: false,
            remoteListError: null,
          });
        }
        return;
      }

      // 会话内即时消息下行：对端（被控方）发来的消息 → 追加（mine=false）
      if (p.type === "chat_message") {
        set((s) => ({
          chatMessages: appendChat(s.chatMessages, {
            msg_id: p.msg_id,
            text: p.text,
            mine: false,
            ts: Date.now(),
          }),
        }));
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
    const [logs, sessions] = await Promise.all([
      transport.fetchAudit({ from, to, endpoint, result }),
      transport.fetchSessions(),
    ]);
    set({ auditLogs: logs, sessions });
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

  startRemote(mode, target, password, name, force = false) {
    set({
      remotePhase: "connecting",
      remoteTarget: name ?? target,
      remoteRejectReason: null,
      remoteFrame: null,
      remoteNotice: null,
    });
    get().sendEnvelope({
      type: "connect_request",
      mode,
      target,
      password,
      force,
    });
  },

  endRemote() {
    const sessionId = get().remoteSessionId;
    if (sessionId) {
      get().sendEnvelope({ type: "session_end", session_id: sessionId });
    }
    set({
      remotePhase: "launch",
      remoteSessionId: null,
      remoteFrame: null,
      chatMessages: [],
      fileProgress: {},
    });
  },

  resetRemote() {
    set({
      remotePhase: "launch",
      remoteSessionId: null,
      remoteFrame: null,
      remoteRejectReason: null,
      execResults: [],
      fileNotice: null,
      remoteNotice: null,
      remotePath: "",
      remoteEntries: [],
      remoteResolution: "r720p",
      remoteClarity: "standard",
      remoteFps: "smooth",
      remoteCursorShape: null,
      remoteCursorVisible: true,
      remoteCursorPos: null,
      remoteListLoading: false,
      remoteListError: null,
      chatMessages: [],
      fileProgress: {},
    });
  },

  // 在已授权远控会话内下发一次性命令
  execCommand(command) {
    const sessionId = get().remoteSessionId;
    if (!sessionId || !command.trim()) return;
    const exec_id = genId("e");
    set((s) => ({
      execResults: [
        {
          exec_id,
          command,
          pending: true,
          exit_code: null,
          stdout: "",
          stderr: "",
          truncated: false,
          duration_ms: 0,
        },
        ...s.execResults,
      ].slice(0, 20),
    }));
    get().sendEnvelope({
      type: "exec_request",
      session_id: sessionId,
      exec_id,
      command,
      timeout_ms: EXEC_TIMEOUT_MS,
    });
  },

  // 下发本地文件到被控端（分块 push）；dest 为远端文件浏览器当前目录（留空落被控端 recv 目录）。
  // 不在发完分片即宣称成功——等被控端 file_done 回执显示最终落盘路径。
  async pushFile(file, dest) {
    const sessionId = get().remoteSessionId;
    if (!sessionId) return;
    const transfer_id = genId("t");
    const buf = new Uint8Array(await file.arrayBuffer());
    set((s) => ({
      fileProgress: startProgress(s.fileProgress, {
        transfer_id,
        name: file.name,
        total: buf.length,
        dir: "push",
      }),
    }));
    get().sendEnvelope({
      type: "file_open",
      session_id: sessionId,
      transfer_id,
      name: file.name,
      size: BigInt(buf.length),
      dir: "push",
      dest: dest && dest.trim() ? dest : null,
    });
    if (buf.length === 0) {
      get().sendEnvelope({
        type: "file_chunk",
        session_id: sessionId,
        transfer_id,
        seq: 0n,
        data: "",
        last: true,
      });
    } else {
      let seq = 0;
      for (let off = 0; off < buf.length; off += CHUNK_SIZE) {
        const slice = buf.subarray(off, Math.min(off + CHUNK_SIZE, buf.length));
        const last = off + CHUNK_SIZE >= buf.length;
        get().sendEnvelope({
          type: "file_chunk",
          session_id: sessionId,
          transfer_id,
          seq: BigInt(seq),
          data: bytesToB64(slice),
          last,
        });
        set((s) => ({ fileProgress: advanceProgress(s.fileProgress, transfer_id, slice.length) }));
        seq++;
      }
    }
  },

  // 从被控端取回指定路径文件（pull）
  pullFile(path) {
    const sessionId = get().remoteSessionId;
    if (!sessionId || !path.trim()) return;
    const transfer_id = genId("t");
    set({ fileNotice: `请求取回 ${path}…` });
    get().sendEnvelope({
      type: "file_pull_request",
      session_id: sessionId,
      transfer_id,
      path,
    });
  },

  // 列出被控端某目录（path 空 = 被控端默认目录 home）
  listRemote(path) {
    const sessionId = get().remoteSessionId;
    if (!sessionId) return;
    set({ remoteListLoading: true, remoteListError: null });
    get().sendEnvelope({
      type: "file_list_request",
      session_id: sessionId,
      transfer_id: genId("l"),
      path,
    });
  },

  setRemoteCursorPos(p) {
    set({ remoteCursorPos: p });
  },

  // 切换三轴显示参数 → 合并当前值后发 set_quality 给被控端（mode 按清晰度映射兜底旧被控端）。
  // 纯决策（合并 + mode 映射）在 lib/quality.resolveDisplayParams，本处只做 state 读写与发信封。
  setRemoteDisplayParams(p) {
    const sessionId = get().remoteSessionId;
    const { resolution, clarity, fps, mode } = resolveDisplayParams(
      { resolution: get().remoteResolution, clarity: get().remoteClarity, fps: get().remoteFps },
      p,
    );
    set({ remoteResolution: resolution, remoteClarity: clarity, remoteFps: fps });
    if (!sessionId) return;
    get().sendEnvelope({ type: "set_quality", session_id: sessionId, mode, resolution, clarity, fps });
  },

  // 在已授权远控会话内发送一条即时消息：乐观本地追加（mine=true）+ 发 chat_message。
  // server 不回显自己的消息（route_to_peer 只发对端），故本端消息靠乐观追加显示。
  sendChat(text) {
    const sessionId = get().remoteSessionId;
    const trimmed = text.trim();
    if (!sessionId || !trimmed) return;
    const msg_id = genId("c");
    set((s) => ({
      chatMessages: appendChat(s.chatMessages, {
        msg_id,
        text: trimmed,
        mine: true,
        ts: Date.now(),
      }),
    }));
    get().sendEnvelope({
      type: "chat_message",
      session_id: sessionId,
      msg_id,
      text: trimmed,
    });
  },
}));
