import { ArrowRight, ShieldX, UserCog } from "lucide-react";

import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { MODE_LABEL, RESULT_LABEL, type AuditRecord } from "@/lib/adapters/audit";

// 模式：A 管理端→终端 / B 终端→终端
export function ModeBadge({ mode }: { mode: "A" | "B" }) {
  return (
    <Badge
      variant="outline"
      className={cn(
        "rounded-md font-normal",
        mode === "A"
          ? "border-primary/40 bg-primary/10 text-primary"
          : "border-border bg-secondary text-muted-foreground",
      )}
    >
      {MODE_LABEL[mode]}
    </Badge>
  );
}

// 结果：成功绿 / 拒绝灰 / 鉴权失败红 / 进行中蓝
export function ResultBadge({ result }: { result: AuditRecord["result"] }) {
  const map: Record<AuditRecord["result"], string> = {
    active: "border-primary/30 bg-primary/10 text-primary",
    success: "border-online/30 bg-online/10 text-online",
    rejected: "border-border bg-secondary text-muted-foreground",
    auth_failed: "border-warning/30 bg-warning/10 text-warning",
  };
  const dotMap: Record<AuditRecord["result"], string> = {
    active: "bg-primary",
    success: "bg-online",
    rejected: "bg-offline",
    auth_failed: "bg-warning",
  };
  return (
    <Badge variant="outline" className={cn("gap-1.5 rounded-full font-normal", map[result])}>
      <span className={cn("size-1.5 rounded-full", dotMap[result])} />
      {RESULT_LABEL[result]}
    </Badge>
  );
}

// 发起方：管理员高亮 / 终端用户普通
export function InitiatorCell({ name }: { name: string }) {
  const isAdmin = name.startsWith("admin-");
  return (
    <span className="inline-flex items-center gap-1.5 text-sm text-foreground">
      <span
        className={cn(
          "flex size-6 shrink-0 items-center justify-center rounded-md border",
          isAdmin
            ? "border-primary/40 bg-primary/10 text-primary"
            : "border-border bg-secondary text-muted-foreground",
        )}
        aria-hidden
      >
        <UserCog className="size-3.5" />
      </span>
      <span>{isAdmin ? "管理员" : name}</span>
    </span>
  );
}

// 目标终端
export function TargetCell({ id, user }: { id: string; user: string }) {
  return (
    <div className="flex items-center gap-2">
      <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm text-foreground">{user}</span>
        <span className="font-mono text-[11px] text-muted-foreground">{id}</span>
      </div>
    </div>
  );
}

// 鉴权失败提示图标（用于时间线）
export function AuthFailIcon() {
  return <ShieldX className="size-4 text-warning" />;
}
