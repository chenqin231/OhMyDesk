import { useCallback, useState } from "react";
import { Camera, Grid2x2, Grid3x3, RotateCw } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useStore } from "@/store";
import { endpointToRow } from "@/lib/adapters/endpoint";
import type { OsKind } from "@/lib/types/OsKind";
import { MonitorCard, type CaptureState } from "@/components/monitor/monitor-card";
import { ScreenDialog } from "@/components/monitor/screen-dialog";

type ThumbSize = "comfortable" | "compact";

type ZoomedItem = {
  id: string;
  user: string;
  ip: string;
  osKey: OsKind;
};

export function MonitorGrid() {
  const endpoints = useStore((s) => s.endpoints);
  const screenshots = useStore((s) => s.screenshots);
  const requestBatchScreenshot = useStore((s) => s.requestBatchScreenshot);

  const nowSec = Math.floor(Date.now() / 1000);
  const onlineRows = endpoints
    .map((ep) => endpointToRow(ep, nowSec))
    .filter((r) => r.status === "online");

  const [capturing, setCapturing] = useState(false);
  const [thumbSize, setThumbSize] = useState<ThumbSize>("comfortable");
  const [zoomed, setZoomed] = useState<ZoomedItem | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const capturedCount = onlineRows.filter((r) => screenshots[r.id]).length;

  const handleBatchCapture = useCallback(() => {
    setCapturing(true);
    requestBatchScreenshot();
    // 给足时间等截图返回（最长 1s 后解锁按钮）
    setTimeout(() => setCapturing(false), 1000);
  }, [requestBatchScreenshot]);

  function openZoom(item: ZoomedItem) {
    setZoomed(item);
    setDialogOpen(true);
  }

  return (
    <div className="flex h-full flex-col gap-4">
      {/* 顶部操作栏 */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-2">
          <h2 className="text-lg font-semibold text-foreground">批量监控</h2>
          <span className="flex items-center gap-1.5 rounded-full border border-border bg-card px-2.5 py-0.5 text-xs text-muted-foreground">
            <span className="size-1.5 rounded-full bg-online" aria-hidden />
            在线 {onlineRows.length}
          </span>
          {capturedCount > 0 && (
            <span className="text-xs text-muted-foreground">
              已截图 {capturedCount}/{onlineRows.length}
            </span>
          )}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <Button onClick={handleBatchCapture} disabled={capturing || onlineRows.length === 0}>
            <Camera data-icon="inline-start" className={cn(capturing && "animate-pulse")} />
            {capturing ? "截图中…" : "一键批量截图"}
          </Button>
          <Button variant="outline" size="icon" aria-label="刷新" disabled={capturing}>
            <RotateCw />
          </Button>
          <ToggleGroup
            value={[thumbSize]}
            onValueChange={(v: string[]) => {
              if (v[0]) setThumbSize(v[0] as ThumbSize);
            }}
            variant="outline"
          >
            <ToggleGroupItem value="comfortable" aria-label="大缩略图">
              <Grid2x2 />
            </ToggleGroupItem>
            <ToggleGroupItem value="compact" aria-label="小缩略图">
              <Grid3x3 />
            </ToggleGroupItem>
          </ToggleGroup>
        </div>
      </div>

      {/* 终端卡片网格 */}
      <div
        className={cn(
          "grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3",
          thumbSize === "comfortable" ? "xl:grid-cols-3" : "xl:grid-cols-4",
        )}
      >
        {onlineRows.map((t) => {
          const data = screenshots[t.id] ?? null;
          const state: CaptureState = capturing && !data ? "loading" : data ? "done" : "empty";
          return (
            <MonitorCard
              key={t.id}
              id={t.id}
              user={t.user}
              ip={t.ip}
              osKey={t.osKey}
              state={state}
              screenshotData={data}
              onZoom={() => openZoom({ id: t.id, user: t.user, ip: t.ip, osKey: t.osKey })}
              onCapture={handleBatchCapture}
            />
          );
        })}
        {onlineRows.length === 0 && (
          <div className="col-span-full flex h-40 items-center justify-center rounded-lg border border-border text-sm text-muted-foreground">
            暂无在线终端
          </div>
        )}
      </div>

      <ScreenDialog
        user={zoomed?.user ?? null}
        id={zoomed?.id ?? null}
        ip={zoomed?.ip ?? null}
        osKey={zoomed?.osKey ?? null}
        screenshotData={zoomed ? (screenshots[zoomed.id] ?? null) : null}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </div>
  );
}
