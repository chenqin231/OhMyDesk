import { ShieldX, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";

type RejectedCardProps = {
  reason: string | null;
  onRetry: () => void;
};

// G-3：模式B密码错→拒连结果态展示
export function RejectedCard({ reason, onRetry }: RejectedCardProps) {
  return (
    <div className="flex h-full w-full items-center justify-center p-6">
      <div className="flex w-full max-w-sm flex-col items-center gap-5 rounded-xl border border-warning/30 bg-card p-8 text-center ring-1 ring-warning/10">
        <span className="flex size-14 items-center justify-center rounded-full border border-warning/30 bg-warning/10 text-warning">
          <ShieldX className="size-6" />
        </span>

        <div className="flex flex-col gap-1.5">
          <h2 className="text-base font-medium text-foreground">连接被拒绝</h2>
          <p className="text-sm text-warning">
            {reason ?? "对方拒绝了远程控制请求"}
          </p>
        </div>

        <div className="flex w-full items-start gap-2 rounded-md border border-border bg-secondary/50 px-3 py-2.5 text-left">
          <p className="text-xs leading-relaxed text-muted-foreground">
            模式 B 密码错误或被控端用户拒绝授权，连接未建立。请确认终端 ID 和 6 位连接密码后重试。
          </p>
        </div>

        <Button variant="outline" onClick={onRetry} className="w-full">
          <RefreshCw data-icon="inline-start" />
          重新发起
        </Button>
      </div>
    </div>
  );
}
