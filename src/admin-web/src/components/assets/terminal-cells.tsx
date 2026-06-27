import { ShieldCheck } from "lucide-react";

import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import {
  OS_MONOGRAM,
  OS_DOMESTIC,
  ARCH_LABEL,
  ARCH_DOMESTIC,
  type OsKind,
  type CpuArch,
} from "@/lib/adapters/endpoint";

// 状态：在线绿点 / 离线灰点
export function StatusBadge({ status }: { status: "online" | "offline" }) {
  const online = status === "online";
  return (
    <Badge
      variant="outline"
      className={cn(
        "gap-1.5 rounded-full border-border bg-card font-normal",
        online ? "text-online" : "text-muted-foreground",
      )}
    >
      <span className="relative flex size-2">
        {online && (
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-online opacity-60" />
        )}
        <span
          className={cn("relative inline-flex size-2 rounded-full", online ? "bg-online" : "bg-offline")}
        />
      </span>
      {online ? "在线" : "离线"}
    </Badge>
  );
}

// 操作系统：信创图标 + 名称
export function OsCell({ osKey, osName }: { osKey: OsKind; osName: string }) {
  const domestic = OS_DOMESTIC[osKey];
  const monogram = OS_MONOGRAM[osKey];
  return (
    <div className="flex items-center gap-2.5">
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-md border text-xs font-semibold",
          domestic
            ? "border-primary/40 bg-primary/10 text-primary"
            : "border-border bg-secondary text-muted-foreground",
        )}
        aria-hidden
      >
        {monogram}
      </span>
      <div className="flex min-w-0 flex-col">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm text-foreground">{osName}</span>
          {domestic && (
            <ShieldCheck className="size-3.5 shrink-0 text-online" aria-label="国产信创" />
          )}
        </div>
      </div>
    </div>
  );
}

// 国产/非国产 标签
export function DomesticTag({ domestic }: { domestic: boolean }) {
  return domestic ? (
    <Badge className="border-online/30 bg-online/10 font-normal text-online">国产信创</Badge>
  ) : (
    <Badge variant="outline" className="border-border font-normal text-muted-foreground">
      非国产
    </Badge>
  );
}

// CPU 架构：国产架构高亮
export function ArchBadge({ arch }: { arch: CpuArch }) {
  const label = ARCH_LABEL[arch];
  const domestic = ARCH_DOMESTIC[arch];
  return (
    <Badge
      variant="outline"
      className={cn(
        "rounded-md font-mono font-normal",
        domestic
          ? "border-primary/40 bg-primary/10 text-primary"
          : "border-border bg-secondary text-muted-foreground",
      )}
    >
      {label}
    </Badge>
  );
}

// 内存占用：数值 + 细进度条
export function MemoryBar({ used, total }: { used: number; total: number }) {
  const pct = total > 0 ? Math.round((used / total) * 100) : 0;
  const high = pct >= 80;
  return (
    <div className="flex w-32 flex-col gap-1">
      <span className="font-mono text-xs text-foreground">
        {used.toFixed(1)}/{total} GB
      </span>
      <div className="h-1 w-full overflow-hidden rounded-full bg-secondary">
        <div
          className={cn("h-full rounded-full", high ? "bg-warning" : "bg-primary")}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}
