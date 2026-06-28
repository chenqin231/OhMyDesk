import type { Envelope } from "@/lib/types/Envelope";
import type { AuditLog } from "@/lib/types/AuditLog";
import type { Session } from "@/lib/types/Session";

// 审计查询参数（对应 GET /api/audit?endpoint=&from=&to=&result=）
export type AuditQuery = {
  endpoint?: string;
  from?: number;  // 秒级 epoch
  to?: number;    // 秒级 epoch
  result?: string;
};

// Transport 抽象接口：mock 与 real 实现同一接口，组件无感切换
export interface Transport {
  // 建立连接并注册信封消费者
  connect(selfId: string, onEnvelope: (e: Envelope) => void): void;
  // 发送信封（connect_request / input / screenshot_req / session_end）
  send(e: Envelope): void;
  // 获取审计日志
  fetchAudit(q: AuditQuery): Promise<AuditLog[]>;
  // 获取会话历史
  fetchSessions(): Promise<Session[]>;
  // 删除终端记录（单个或批量；清理离线/冗余）
  deleteEndpoints(ids: string[]): Promise<void>;
  // 断开连接并清理资源
  disconnect(): void;
}
