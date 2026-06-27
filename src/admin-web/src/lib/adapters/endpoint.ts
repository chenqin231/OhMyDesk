// D-1~D-5 适配：EndpointView（protocol/ts-rs 形状）→ TerminalRow（UI 视图）
import type { EndpointView } from "@/lib/types/EndpointView";
import type { OsKind } from "@/lib/types/OsKind";
import type { CpuArch } from "@/lib/types/CpuArch";

export type { OsKind, CpuArch };

// UI 层期望的扁平视图——不含 connectPassword（O-3 裁决）
export type TerminalRow = {
  id: string;
  status: "online" | "offline";      // D-2 bool → 枚举
  user: string;
  department: string;
  ip: string;
  mac: string;
  osKey: OsKind;                       // D-1 嵌套 → 扁平
  osName: string;
  arch: CpuArch;                       // D-1 嵌套 → 扁平
  cpuModel: string;
  cpuCores: number;
  memUsedGb: number;                   // D-3 bigint 字节 ÷ 1024³
  memTotalGb: number;
  gpuModel: string | null;
  gpuVramGb: number | null;            // D-3 bigint 字节 ÷ 1024³
  lastSeenText: string;                // D-4 bigint epoch → 相对时间
  xinchuang: string;
  agentVersion: string;
};

const B = 1024 ** 3;

// D-4 epoch → 相对时间文本
function relTime(epochSec: bigint, nowSec: number): string {
  const diffSec = nowSec - Number(epochSec);
  if (diffSec < 60) return "刚刚";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} 分钟前`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} 小时前`;
  return `${Math.floor(diffSec / 86400)} 天前`;
}

// D-1~D-5 适配主函数
export function endpointToRow(e: EndpointView, nowSec: number): TerminalRow {
  return {
    id: e.info.id,
    status: e.online ? "online" : "offline",   // D-2
    user: e.info.name,
    department: e.info.department ?? "—",
    ip: e.info.ip,
    mac: e.info.mac,
    osKey: e.info.os.kind,                      // D-1
    osName: e.info.os.name,
    arch: e.info.cpu.arch,                      // D-1
    cpuModel: e.info.cpu.model,
    cpuCores: e.info.cpu.cores,
    memUsedGb: +(Number(e.info.ram.used) / B).toFixed(1),   // D-3 bigint
    memTotalGb: +(Number(e.info.ram.total) / B).toFixed(1),
    gpuModel: e.info.gpu?.model ?? null,
    gpuVramGb: e.info.gpu?.vram != null
      ? +(Number(e.info.gpu.vram) / B).toFixed(1)           // D-3 bigint
      : null,
    lastSeenText: relTime(e.last_seen, nowSec), // D-4
    xinchuang: e.xinchuang,
    agentVersion: e.info.agent_version,
  };
}

// 展示字典：protocol 只给 kind/arch 字面量，图标/中文名前端映射
export const OS_LABEL: Record<OsKind, string> = {
  kylin: "银河麒麟",
  uos: "统信 UOS",
  windows: "Windows",
  linux: "Linux",
  other: "其他",
};

export const OS_MONOGRAM: Record<OsKind, string> = {
  kylin: "麒",
  uos: "统",
  windows: "W",
  linux: "Lin",
  other: "?",
};

export const OS_DOMESTIC: Record<OsKind, boolean> = {
  kylin: true,
  uos: true,
  windows: false,
  linux: false,
  other: false,
};

export const ARCH_LABEL: Record<CpuArch, string> = {
  loong_arch: "龙芯 LoongArch",
  aarch64: "鲲鹏 aarch64",
  x86_64: "x86_64",
  other: "其他",
};

export const ARCH_DOMESTIC: Record<CpuArch, boolean> = {
  loong_arch: true,
  aarch64: true,
  x86_64: false,
  other: false,
};
