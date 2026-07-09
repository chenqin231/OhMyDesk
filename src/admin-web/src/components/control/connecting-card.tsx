import { Loader2, ShieldCheck, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { MODE_LABELS } from "@/components/control/launch-panel";
import { useStore } from "@/store";

type ConnectingCardProps = {
  targetName: string;
  mode: "a" | "b";
  onCancel: () => void;
};

// 连接建立中的过场态：居中 loading 卡片，含授权等待说明
export function ConnectingCard({ targetName, mode, onCancel }: ConnectingCardProps) {
  // 强制远程(模式A force)：服务端 AutoAccept，不需对方同意——文案据此区分，避免「正在等待用户确认」误导。
  const forced = useStore((s) => s.remoteForced);
  return (
    <div className="flex h-full w-full items-center justify-center p-6">
      <div className="flex w-full max-w-sm flex-col items-center gap-5 rounded-xl border border-border bg-card p-8 text-center ring-1 ring-foreground/5">
        <span className="relative flex size-14 items-center justify-center">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary/20" />
          <span className="relative flex size-14 items-center justify-center rounded-full border border-primary/30 bg-primary/10 text-primary">
            <Loader2 className="size-6 animate-spin" />
          </span>
        </span>

        <div className="flex flex-col gap-1.5">
          <h2 className="text-balance font-heading text-base font-medium text-foreground">
            正在与 <span className="text-primary">{targetName}</span> 协商连接…
          </h2>
          <p className="font-mono text-xs text-muted-foreground">{MODE_LABELS[mode]}</p>
        </div>

        <div className="flex w-full items-start gap-2 rounded-md border border-border bg-secondary/50 px-3 py-2.5 text-left">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-online" aria-hidden />
          <p className="text-xs leading-relaxed text-muted-foreground">
            {forced
              ? "强制远程：无需对方同意，正在建立会话…（全程文本审计）"
              : "已向对方终端发送授权请求，正在等待用户确认。对方同意后会话将建立并全程文本审计。"}
          </p>
        </div>

        <Button variant="outline" size="sm" onClick={onCancel} className="w-full">
          <X data-icon="inline-start" />
          取消连接
        </Button>
      </div>
    </div>
  );
}
