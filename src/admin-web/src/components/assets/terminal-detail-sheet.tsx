import type { ReactNode } from "react";
import { Cpu, HardDrive, MonitorCog, Network, Tag, Terminal as TerminalIcon } from "lucide-react";

import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import type { TerminalRow } from "@/lib/adapters/endpoint";
import { OS_DOMESTIC } from "@/lib/adapters/endpoint";
import { ArchBadge, DomesticTag, MemoryBar, OsCell, StatusBadge } from "@/components/assets/terminal-cells";

type Props = {
  terminal: TerminalRow | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRemoteControl?: (id: string) => void;
};

// 单条信息行
function Field({ label, children, mono }: { label: string; children: ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-4 py-2">
      <span className="shrink-0 text-xs text-muted-foreground">{label}</span>
      <span className={mono ? "font-mono text-sm text-foreground" : "text-sm text-foreground"}>
        {children}
      </span>
    </div>
  );
}

// 分组标题
function GroupTitle({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 pb-1 pt-1 text-xs font-medium tracking-wide text-muted-foreground">
      <span className="text-primary">{icon}</span>
      {children}
    </div>
  );
}

export function TerminalDetailSheet({ terminal, open, onOpenChange, onRemoteControl }: Props) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full gap-0 overflow-y-auto sm:max-w-md">
        {terminal && (
          <>
            <SheetHeader className="gap-3 border-b border-border">
              <div className="flex items-center justify-between gap-2">
                <SheetTitle className="flex items-center gap-2 text-base">
                  <span className="font-mono text-muted-foreground">{terminal.id}</span>
                  <span>{terminal.user}</span>
                </SheetTitle>
                <StatusBadge status={terminal.status} />
              </div>
              <SheetDescription className="text-xs">
                {terminal.department} · 终端硬件画像
              </SheetDescription>
            </SheetHeader>

            <div className="flex flex-col gap-4 px-4 py-4">
              {/* 身份与网络 */}
              <section>
                <GroupTitle icon={<Network className="size-3.5" />}>身份与网络</GroupTitle>
                <div className="rounded-lg border border-border bg-card px-3">
                  <Field label="使用人">{terminal.user}</Field>
                  <Separator />
                  <Field label="IP 地址" mono>{terminal.ip}</Field>
                  <Separator />
                  <Field label="MAC 地址" mono>{terminal.mac}</Field>
                </div>
              </section>

              {/* 操作系统 */}
              <section>
                <GroupTitle icon={<MonitorCog className="size-3.5" />}>操作系统</GroupTitle>
                <div className="flex items-center justify-between rounded-lg border border-border bg-card px-3 py-3">
                  <OsCell osKey={terminal.osKey} osName={terminal.osName} />
                  <DomesticTag domestic={OS_DOMESTIC[terminal.osKey]} />
                </div>
              </section>

              {/* 处理器 */}
              <section>
                <GroupTitle icon={<Cpu className="size-3.5" />}>处理器</GroupTitle>
                <div className="rounded-lg border border-border bg-card px-3">
                  <Field label="型号">{terminal.cpuModel}</Field>
                  <Separator />
                  <Field label="核数" mono>{terminal.cpuCores} 核</Field>
                  <Separator />
                  <div className="flex items-center justify-between gap-4 py-2">
                    <span className="shrink-0 text-xs text-muted-foreground">架构</span>
                    <ArchBadge arch={terminal.arch} />
                  </div>
                </div>
              </section>

              {/* 内存与显卡 */}
              <section>
                <GroupTitle icon={<HardDrive className="size-3.5" />}>内存与显卡</GroupTitle>
                <div className="rounded-lg border border-border bg-card px-3">
                  <div className="flex items-center justify-between gap-4 py-2">
                    <span className="shrink-0 text-xs text-muted-foreground">内存占用</span>
                    <MemoryBar used={terminal.memUsedGb} total={terminal.memTotalGb} />
                  </div>
                  <Separator />
                  <Field label="GPU 型号">{terminal.gpuModel ?? "无独立显卡"}</Field>
                  {terminal.gpuVramGb !== null && (
                    <>
                      <Separator />
                      <Field label="显存" mono>{terminal.gpuVramGb} GB</Field>
                    </>
                  )}
                </div>
              </section>

              {/* Agent 信息（O-3 裁决：不展示连接密码） */}
              <section>
                <GroupTitle icon={<Tag className="size-3.5" />}>Agent 信息</GroupTitle>
                <div className="rounded-lg border border-border bg-card px-3">
                  <div className="flex items-center justify-between gap-4 py-2">
                    <span className="shrink-0 text-xs text-muted-foreground">Agent 版本</span>
                    <span className="inline-flex items-center gap-1.5 font-mono text-sm text-foreground">
                      <Tag className="size-3.5 text-muted-foreground" />
                      {terminal.agentVersion}
                    </span>
                  </div>
                  <Separator />
                  <Field label="最后在线" mono>{terminal.lastSeenText}</Field>
                </div>
              </section>
            </div>

            <SheetFooter className="border-t border-border">
              <Button
                className="w-full gap-2"
                disabled={terminal.status === "offline"}
                onClick={() => onRemoteControl?.(terminal.id)}
              >
                <TerminalIcon className="size-4" />
                申请远程
              </Button>
            </SheetFooter>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}
