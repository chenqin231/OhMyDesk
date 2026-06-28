import { Camera, Maximize2, MonitorUp } from "lucide-react";

import { cn } from "@/lib/utils";
import { Card, CardContent, CardFooter, CardHeader } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { OS_LABEL, OS_DOMESTIC, OS_MONOGRAM } from "@/lib/adapters/endpoint";
import { screenshotSrc } from "@/lib/adapters/media";
import type { OsKind } from "@/lib/types/OsKind";

export type CaptureState = "empty" | "loading" | "done";

type MonitorCardProps = {
  id: string;
  user: string;
  ip: string;
  osKey: OsKind;
  state: CaptureState;
  screenshotData: string | null;  // base64，null 表示未截图
  onZoom: () => void;
  onCapture: () => void;
  onRemoteControl: () => void;
};

export function MonitorCard({ id: _id, user, ip, osKey, state, screenshotData, onZoom, onCapture, onRemoteControl }: MonitorCardProps) {
  const domestic = OS_DOMESTIC[osKey];
  const monogram = OS_MONOGRAM[osKey];
  const osName = OS_LABEL[osKey];

  return (
    <Card className="group gap-0 overflow-hidden py-0 transition-colors hover:border-primary/50">
      {/* 顶部：状态 + 使用人 + IP */}
      <CardHeader className="flex-row items-center gap-2 p-3 [.border-b]:pb-3">
        <span className="relative flex size-2 shrink-0">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-online opacity-60" />
          <span className="relative inline-flex size-2 rounded-full bg-online" />
        </span>
        <span className="truncate text-sm font-medium text-foreground">{user}</span>
        <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground">{ip}</span>
      </CardHeader>

      {/* 中部：16:9 屏幕缩略图 */}
      <CardContent className="p-0">
        <button
          type="button"
          onClick={state === "done" ? onZoom : onCapture}
          className="relative block aspect-video w-full overflow-hidden border-y border-border bg-secondary text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={state === "done" ? `放大查看 ${user} 的屏幕` : `截图 ${user} 的屏幕`}
        >
          {state === "loading" ? (
            <Skeleton className="absolute inset-0 size-full rounded-none" />
          ) : state === "done" && screenshotData ? (
            <>
              {/* G-1：用 <img src=data:image/jpeg;base64,> 渲染真实帧 */}
              <img
                src={screenshotSrc({ data: screenshotData })}
                alt={`${user} 的屏幕缩略图`}
                className="absolute inset-0 size-full object-cover"
              />
              {/* 悬停浮现放大图标 */}
              <span className="absolute inset-0 flex items-center justify-center bg-background/55 opacity-0 backdrop-blur-[1px] transition-opacity group-hover:opacity-100">
                <span className="flex size-10 items-center justify-center rounded-full bg-primary text-primary-foreground">
                  <Maximize2 className="size-4" />
                </span>
              </span>
            </>
          ) : (
            <span className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-muted-foreground transition-colors group-hover:text-foreground">
              <Camera className="size-6" />
              <span className="text-xs">点击批量截图</span>
            </span>
          )}
        </button>
      </CardContent>

      {/* 底部：OS 信创标签 + 操作按钮 */}
      <CardFooter className="flex-row items-center gap-2 p-3">
        <span
          className={cn(
            "flex items-center gap-1.5 rounded-md border px-1.5 py-0.5 text-xs",
            domestic
              ? "border-primary/40 bg-primary/10 text-primary"
              : "border-border bg-secondary text-muted-foreground",
          )}
        >
          <span className="font-semibold" aria-hidden>{monogram}</span>
          {osName}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            onClick={onZoom}
            disabled={state !== "done"}
          >
            <Maximize2 data-icon="inline-start" />
            放大
          </Button>
          <Button size="sm" onClick={onRemoteControl}>
            <MonitorUp data-icon="inline-start" />
            远程控制
          </Button>
        </div>
      </CardFooter>
    </Card>
  );
}
