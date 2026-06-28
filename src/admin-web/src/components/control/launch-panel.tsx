import { useState } from "react";
import { ArrowRight, KeyRound, MonitorUp, ServerCog } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useStore } from "@/store";
import { endpointToRow } from "@/lib/adapters/endpoint";

type LaunchPanelProps = {
  onLaunch: (mode: "a" | "b", target: string, password: string | null) => void;
  onPreviewAuth: () => void;
};

export const MODE_LABELS: Record<"a" | "b", string> = {
  a: "管理端 → 终端",
  b: "终端 → 终端",
};

// 远程控制发起入口：模式 A（管理端→终端）与模式 B（终端→终端）
export function LaunchPanel({ onLaunch, onPreviewAuth }: LaunchPanelProps) {
  const endpoints = useStore((s) => s.endpoints);
  const nowSec = Math.floor(Date.now() / 1000);
  const onlineTargets = endpoints
    .map((ep) => endpointToRow(ep, nowSec))
    .filter((r) => r.status === "online");

  const [targetId, setTargetId] = useState<string>(onlineTargets[0]?.id ?? "");
  const [peerId, setPeerId] = useState("");
  const [peerPwd, setPeerPwd] = useState("");

  const targetName = onlineTargets.find((t) => t.id === targetId)?.user ?? "目标终端";
  const modeBReady = peerId.trim().length > 0 && peerPwd.trim().length === 6;

  return (
    <div className="mx-auto flex min-h-full w-full max-w-4xl flex-col justify-center gap-6 p-6">
      <div className="flex flex-col gap-1.5 text-center">
        <h1 className="text-balance font-heading text-xl font-semibold text-foreground">
          发起远程控制
        </h1>
        <p className="text-sm text-muted-foreground">
          选择发起方式，向目标终端请求授权后建立受控会话
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        {/* 模式 A：管理端 → 终端 */}
        <div className="flex flex-col gap-4 rounded-xl border border-primary/30 bg-card p-5 ring-1 ring-primary/10">
          <div className="flex items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-md border border-primary/40 bg-primary/10 text-primary">
              <ServerCog className="size-5" />
            </span>
            <div className="flex flex-col">
              <span className="font-heading text-sm font-medium text-foreground">
                模式 A · 管理端 → 终端
              </span>
              <span className="text-xs text-muted-foreground">
                由管理端直接选择在线终端发起协助
              </span>
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs text-muted-foreground">目标终端</label>
            <Select value={targetId} onValueChange={(v) => { if (v !== null) setTargetId(v); }}>
              <SelectTrigger className="w-full">
                <SelectValue>
                  {onlineTargets.find((t) => t.id === targetId)
                    ? `${onlineTargets.find((t) => t.id === targetId)!.user} · ${onlineTargets.find((t) => t.id === targetId)!.ip}`
                    : "选择终端"}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {onlineTargets.map((t) => (
                    <SelectItem key={t.id} value={t.id}>
                      {t.user} · {t.ip}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>

          <Button
            className="mt-auto w-full"
            disabled={!targetId}
            onClick={() => onLaunch("a", targetName, null)}
          >
            <MonitorUp data-icon="inline-start" />
            发起远程协助
          </Button>
        </div>

        {/* 模式 B：终端 → 终端（G-3：密码错→拒连结果态） */}
        <div className="flex flex-col gap-4 rounded-xl border border-border bg-card p-5">
          <div className="flex items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border bg-secondary text-muted-foreground">
              <KeyRound className="size-5" />
            </span>
            <div className="flex flex-col">
              <span className="font-heading text-sm font-medium text-foreground">
                模式 B · 终端 → 终端
              </span>
              <span className="text-xs text-muted-foreground">
                输入对方终端 ID 与 6 位连接密码发起（mock 密码：123456）
              </span>
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs text-muted-foreground">终端 ID</label>
            <Input
              value={peerId}
              onChange={(e) => setPeerId(e.target.value)}
              placeholder="如 ep-001"
              className="font-mono"
            />
          </div>
          <div className="flex flex-col gap-2">
            <label className="text-xs text-muted-foreground">6 位连接密码</label>
            <Input
              value={peerPwd}
              onChange={(e) => setPeerPwd(e.target.value.replace(/\D/g, "").slice(0, 6))}
              inputMode="numeric"
              placeholder="······"
              className={cn("font-mono tracking-[0.4em]")}
            />
          </div>

          <Button
            variant="outline"
            className="mt-auto w-full"
            disabled={!modeBReady}
            onClick={() => onLaunch("b", peerId.trim(), peerPwd)}
          >
            <ArrowRight data-icon="inline-start" />
            连接
          </Button>
        </div>
      </div>

      <div className="flex justify-center">
        <Button variant="ghost" size="sm" onClick={onPreviewAuth}>
          预览被控端授权请求弹窗
        </Button>
      </div>
    </div>
  );
}
