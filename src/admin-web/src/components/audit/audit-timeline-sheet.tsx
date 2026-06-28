import {
  Camera,
  FileUp,
  KeyboardIcon,
  LogOut,
  PlugZap,
  ShieldX,
  Terminal,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { AuditRecord } from "@/lib/adapters/audit";
import type { AuditType } from "@/lib/types/AuditType";
import { ModeBadge, ResultBadge } from "@/components/audit/audit-cells";

type Props = {
  record: AuditRecord | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

// O-1 裁决：砍掉 transfer 分支，只含 spec 集合事件类型
const KIND_ICON: Record<AuditType, LucideIcon> = {
  connect: PlugZap,
  screenshot: Camera,
  input: KeyboardIcon,
  auth_fail: ShieldX,
  reject: ShieldX,
  disconnect: LogOut,
  command: Terminal,
  file_transfer: FileUp,
};

export function AuditTimelineSheet({ record, open, onOpenChange }: Props) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full gap-0 overflow-y-auto sm:max-w-md">
        {record && (
          <>
            <SheetHeader className="gap-3 border-b border-border">
              <div className="flex items-center justify-between gap-2">
                <SheetTitle className="flex items-center gap-2 text-base">
                  <span className="font-mono text-xs text-muted-foreground">{record.sessionId}</span>
                </SheetTitle>
                <ResultBadge result={record.result} />
              </div>
              <SheetDescription className="flex flex-wrap items-center gap-2 text-xs">
                <span>{record.actor}</span>
                <span className="text-muted-foreground">控制</span>
                <span className="text-foreground">{record.target}</span>
                <ModeBadge mode={record.mode} />
              </SheetDescription>
              <div className="flex items-center justify-between font-mono text-[11px] text-muted-foreground">
                <span>{record.startText}</span>
                <span>时长 {record.durationText}</span>
              </div>
            </SheetHeader>

            <div className="px-5 py-5">
              <div className="mb-3 text-xs font-medium tracking-wide text-muted-foreground">
                会话操作时间线
              </div>
              {/* 垂直 timeline */}
              <ol className="relative flex flex-col">
                {record.timeline.map((ev, i) => {
                  const Icon = KIND_ICON[ev.kind];
                  const isError = ev.kind === "auth_fail" || ev.kind === "reject";
                  const isLast = i === record.timeline.length - 1;
                  const timeStr = new Date(ev.ts * 1000).toLocaleTimeString("zh-CN", {
                    hour: "2-digit",
                    minute: "2-digit",
                    second: "2-digit",
                  });
                  return (
                    <li key={i} className="relative flex gap-3 pb-5 last:pb-0">
                      {/* 连接线 */}
                      {!isLast && (
                        <span
                          className="absolute left-[15px] top-8 h-[calc(100%-1rem)] w-px bg-border"
                          aria-hidden
                        />
                      )}
                      {/* 节点图标 */}
                      <span
                        className={cn(
                          "relative z-10 flex size-8 shrink-0 items-center justify-center rounded-full border",
                          isError
                            ? "border-warning/40 bg-warning/10 text-warning"
                            : "border-border bg-card text-primary",
                        )}
                      >
                        <Icon className="size-4" />
                      </span>
                      {/* 事件内容 */}
                      <div
                        className={cn(
                          "flex min-w-0 flex-1 flex-col gap-1 rounded-lg border px-3 py-2",
                          isError ? "border-warning/30 bg-warning/5" : "border-border bg-card",
                        )}
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span
                            className={cn(
                              "text-sm font-medium",
                              isError ? "text-warning" : "text-foreground",
                            )}
                          >
                            {ev.kind}
                          </span>
                          <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                            {timeStr}
                          </span>
                        </div>
                        <span className="text-xs leading-relaxed text-muted-foreground">
                          {ev.text}
                        </span>
                      </div>
                    </li>
                  );
                })}
              </ol>
            </div>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}
