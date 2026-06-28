import { MonitorUp } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { OsCell } from "@/components/assets/terminal-cells";
import { screenshotSrc } from "@/lib/adapters/media";
import type { OsKind } from "@/lib/types/OsKind";

type ScreenDialogProps = {
  user: string | null;
  id: string | null;
  ip: string | null;
  osKey: OsKind | null;
  screenshotData: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

// 缩略图放大查看大图，含发起远程控制入口
export function ScreenDialog({
  user,
  id,
  ip,
  osKey,
  screenshotData,
  open,
  onOpenChange,
}: ScreenDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl gap-0 p-0 sm:max-w-3xl">
        <DialogHeader className="flex-row items-center justify-between gap-4 border-b border-border p-4">
          <div className="flex min-w-0 flex-col gap-1">
            <DialogTitle className="flex items-center gap-2">
              <span className="size-2 shrink-0 rounded-full bg-online" aria-hidden />
              {user ?? "终端屏幕"}
            </DialogTitle>
            <DialogDescription className="font-mono text-xs">
              {id && ip ? `${id} · ${ip}` : ""}
            </DialogDescription>
          </div>
          {osKey && (
            <div className="pr-8">
              <OsCell osKey={osKey} osName={osKey} />
            </div>
          )}
        </DialogHeader>

        {/* G-1：<img src=data:image/jpeg;base64,> 渲染截图 */}
        <div className="relative aspect-video w-full overflow-hidden bg-secondary">
          {screenshotData ? (
            <img
              src={screenshotSrc({ data: screenshotData })}
              alt={`${user ?? "终端"} 的屏幕画面`}
              className="absolute inset-0 size-full object-cover"
            />
          ) : (
            <span className="absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
              暂无截图
            </span>
          )}
          <div className="absolute left-3 top-3 flex items-center gap-1.5 rounded-md bg-black/55 px-2 py-1 text-xs text-white backdrop-blur-sm">
            <span className="size-1.5 animate-pulse rounded-full bg-warning" aria-hidden />
            实时画面 · 截图时间 {new Date().toLocaleTimeString("zh-CN")}
          </div>
        </div>

        <DialogFooter className="border-t border-border">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
          <Button>
            <MonitorUp data-icon="inline-start" />
            发起远程控制
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
