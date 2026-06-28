import { useEffect, useRef, useState } from "react";
import { MonitorUp, ShieldAlert } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

type AuthRequestDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDecision?: (approved: boolean) => void;
  seconds?: number;
  targetId?: string;
  targetIp?: string;
  operatorName?: string;
};

// 被控端视角：远程控制授权请求弹窗（15 秒倒计时，超时自动拒绝）
export function AuthRequestDialog({
  open,
  onOpenChange,
  onDecision,
  seconds = 15,
  targetId = "ep-xxx",
  targetIp = "10.0.0.x",
  operatorName = "管理员",
}: AuthRequestDialogProps) {
  const [remaining, setRemaining] = useState(seconds);
  const decidedRef = useRef(false);

  useEffect(() => {
    if (!open) {
      setRemaining(seconds);
      decidedRef.current = false;
      return;
    }
    setRemaining(seconds);
    const timer = setInterval(() => {
      setRemaining((r) => {
        if (r <= 1) {
          clearInterval(timer);
          if (!decidedRef.current) {
            decidedRef.current = true;
            onDecision?.(false);
            onOpenChange(false);
          }
          return 0;
        }
        return r - 1;
      });
    }, 1000);
    return () => clearInterval(timer);
  }, [open, seconds, onDecision, onOpenChange]);

  function decide(approved: boolean) {
    decidedRef.current = true;
    onDecision?.(approved);
    onOpenChange(false);
  }

  const pct = (remaining / seconds) * 100;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton={false} className="max-w-md gap-0 p-0">
        <DialogHeader className="items-center gap-3 border-b border-border p-6 text-center">
          <span
            className="flex size-12 items-center justify-center rounded-full border border-warning/30 bg-warning/10 text-warning"
            aria-hidden
          >
            <ShieldAlert className="size-6" />
          </span>
          <DialogTitle className="text-center">远程控制授权请求</DialogTitle>
          <DialogDescription className="text-center text-pretty">
            <span className="font-medium text-foreground">{operatorName}</span>
            {" 请求远程控制您的终端"}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3 p-6">
          <div className="flex items-center justify-between rounded-md border border-border bg-secondary/50 px-3 py-2 text-sm">
            <span className="text-muted-foreground">本机终端</span>
            <span className="font-mono text-foreground">
              {targetId} · {targetIp}
            </span>
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            同意后，对方将可查看并操作您的桌面，整个会话过程将被全程文本审计。如非本人发起或不认识该管理员，请点击拒绝。
          </p>

          <div className="mt-1 flex flex-col gap-1.5">
            <div className="flex items-center justify-between text-xs">
              <span className="text-muted-foreground">将在倒计时结束后自动拒绝</span>
              <span className="font-mono tabular-nums text-warning">{remaining}s</span>
            </div>
            <div className="h-1 w-full overflow-hidden rounded-full bg-secondary">
              <div
                className="h-full rounded-full bg-warning transition-all duration-1000 ease-linear"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>
        </div>

        <DialogFooter className="grid grid-cols-2 gap-3 border-t border-border p-4">
          <Button variant="outline" onClick={() => decide(false)}>
            拒绝
          </Button>
          <Button onClick={() => decide(true)}>
            <MonitorUp data-icon="inline-start" />
            同意
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
