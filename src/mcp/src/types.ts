/**
 * 领域类型定义——对齐 ts-rs 生成物（src/admin-web/src/lib/types/）
 * u64/i64 字段保持 bigint
 */

export type OsKind = "kylin" | "uos" | "windows" | "linux" | "other";
export type CpuArch = "loong_arch" | "aarch64" | "x86_64" | "other";

export interface OsInfo {
  name: string;
  kind: OsKind;
}

export interface CpuInfo {
  model: string;
  cores: number;
  arch: CpuArch;
}

export interface RamInfo {
  total: bigint;
  used: bigint;
}

export interface GpuInfo {
  model: string;
  vram: bigint | null;
}

export interface EndpointInfo {
  id: string;
  name: string;
  department: string | null;
  ip: string;
  mac: string;
  os: OsInfo;
  cpu: CpuInfo;
  ram: RamInfo;
  gpu: GpuInfo | null;
  agent_version: string;
}

export interface EndpointView {
  info: EndpointInfo;
  online: boolean;
  last_seen: bigint;
  xinchuang: string;
}

export type SessionStatus = "active" | "ended" | "rejected";
export type Mode = "a" | "b";

export interface Session {
  id: string;
  mode: Mode;
  from_id: string;
  to_id: string;
  start_at: bigint;
  end_at: bigint | null;
  status: SessionStatus;
}

export type AuditType =
  | "connect"
  | "auth_fail"
  | "reject"
  | "screenshot"
  | "input"
  | "disconnect";

export interface AuditLog {
  id: string;
  session_id: string;
  ts: bigint;
  actor_id: string;
  type: AuditType;
  text: string;
}
