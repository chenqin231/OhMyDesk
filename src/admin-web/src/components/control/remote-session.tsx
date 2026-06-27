import { useEffect, useRef, useCallback } from "react";
import { Camera, Maximize, PhoneOff, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { frameSrc } from "@/lib/adapters/media";
import { useStore } from "@/store";
import type { InputEvent } from "@/lib/types/InputEvent";
import { MODE_LABELS } from "@/components/control/launch-panel";
import {
  containedFrameRect,
  pointerToFrameCoords,
  remoteMouseButtonEvents,
  shouldBlockRemoteContextMenu,
} from "@/components/control/remote-geometry";
import { RemoteTools } from "@/components/control/remote-tools";

type RemoteSessionProps = {
  targetName: string;
  mode: "a" | "b";
  onDisconnect: () => void;
};

// 顶部工具栏单项
function MetaItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5 whitespace-nowrap">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="font-mono text-xs text-foreground">{value}</span>
    </div>
  );
}

// G-1：canvas/img 消费远控帧；G-2：键鼠监听+坐标映射+发 Input 信封
// O-2 裁决：删除会话录制标记 UI
export function RemoteSession({ targetName, mode, onDisconnect }: RemoteSessionProps) {
  const remoteFrame = useStore((s) => s.remoteFrame);
  const remoteSessionId = useStore((s) => s.remoteSessionId);
  const sendEnvelope = useStore((s) => s.sendEnvelope);
  const containerRef = useRef<HTMLDivElement>(null);

  // G-2：坐标映射：将容器内鼠标坐标映射到帧分辨率
  const toFrameCoords = useCallback(
    (e: MouseEvent): { x: number; y: number } | null => {
      const el = containerRef.current;
      if (!el || !remoteFrame) return null;
      const rect = el.getBoundingClientRect();
      const displayRect = containedFrameRect(rect, remoteFrame);
      return pointerToFrameCoords(e, displayRect, remoteFrame);
    },
    [remoteFrame],
  );

  const sendInput = useCallback(
    (event: InputEvent) => {
      if (!remoteSessionId) return;
      sendEnvelope({ type: "input", session_id: remoteSessionId, event });
    },
    [remoteSessionId, sendEnvelope],
  );

  // G-2：监听键鼠事件并发 Input 信封
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    function onMouseMove(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      sendInput({ kind: "mouse_move", x: coords.x, y: coords.y });
    }

    function onMouseDown(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      remoteMouseButtonEvents(coords, e.button, true).forEach(sendInput);
    }

    function onMouseUp(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      remoteMouseButtonEvents(coords, e.button, false).forEach(sendInput);
    }

    function onContextMenu(e: MouseEvent) {
      if (!shouldBlockRemoteContextMenu()) return;
      e.preventDefault();
    }

    el.addEventListener("mousemove", onMouseMove);
    el.addEventListener("mousedown", onMouseDown);
    el.addEventListener("mouseup", onMouseUp);
    el.addEventListener("contextmenu", onContextMenu);
    return () => {
      el.removeEventListener("mousemove", onMouseMove);
      el.removeEventListener("mousedown", onMouseDown);
      el.removeEventListener("mouseup", onMouseUp);
      el.removeEventListener("contextmenu", onContextMenu);
    };
  }, [toFrameCoords, sendInput]);

  // 键盘事件挂在 window（焦点无关）。
  // 用 e.key（已按 Shift/CapsLock 解析出大小写与上档符，如 "A"/"!"/"/"），而非 e.code（物理键
  // 位 "KeyA"/"Digit1"）——被控端注入侧对单字符直接走 Unicode，能正确还原所输入的字符；
  // 具名功能键（"Enter"/"Backspace"/"ArrowUp"/"Shift"…）由被控端映射为对应 enigo Key。
  // preventDefault 拦掉浏览器自身的按键行为（空格滚动、"/" 快速查找、F 键等），远控期间独占键盘。
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      sendInput({ kind: "key", code: e.key, down: true });
    }
    function onKeyUp(e: KeyboardEvent) {
      e.preventDefault();
      sendInput({ kind: "key", code: e.key, down: false });
    }
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [sendInput]);

  return (
    <div className="flex h-full min-h-[calc(100vh-7rem)] w-full flex-col bg-background">
      {/* 顶部细工具栏 */}
      <header className="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-border bg-card px-4">
        {/* 左：远程控制中 + 目标 */}
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex items-center gap-1.5 rounded-full border border-online/30 bg-online/10 px-2.5 py-1">
            <span className="relative flex size-2">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-online opacity-60" />
              <span className="relative inline-flex size-2 rounded-full bg-online" />
            </span>
            <span className="text-xs font-medium text-online">远程控制中</span>
          </span>
          <span className="truncate text-sm font-medium text-foreground">{targetName}</span>
        </div>

        {/* 中：连接信息 */}
        <div className="hidden items-center gap-3 lg:flex">
          <MetaItem label="模式" value={MODE_LABELS[mode]} />
          {remoteFrame && (
            <>
              <Separator orientation="vertical" className="h-4" />
              <MetaItem label="分辨率" value={`${remoteFrame.w} × ${remoteFrame.h}`} />
              <Separator orientation="vertical" className="h-4" />
              <MetaItem label="帧序" value={String(remoteFrame.seq)} />
            </>
          )}
        </div>

        {/* 右：操作按钮 */}
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" size="sm">
            <Maximize data-icon="inline-start" />
            <span className="hidden sm:inline">全屏</span>
          </Button>
          <Button variant="outline" size="sm">
            <Camera data-icon="inline-start" />
            <span className="hidden sm:inline">截图</span>
          </Button>
          <Button
            size="sm"
            onClick={onDisconnect}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            <PhoneOff data-icon="inline-start" />
            断开连接
          </Button>
        </div>
      </header>

      {/* 主体：远程画面 + 右侧命令/文件工具栏 */}
      <div className="flex min-h-0 flex-1">
      {/* G-1：主体远程画面，<img src=data:image/jpeg;base64,> 消费 frame */}
      <main className="relative flex flex-1 items-center justify-center overflow-hidden bg-black p-3 md:p-6">
        <div
          ref={containerRef}
          className="relative flex h-full w-full max-w-[1920px] items-center justify-center overflow-hidden rounded-lg ring-1 ring-border cursor-pointer"
        >
          {remoteFrame ? (
            <img
              src={frameSrc({ data: remoteFrame.data })}
              alt={`${targetName} 的远程桌面画面`}
              className="max-h-full max-w-full object-contain"
              draggable={false}
            />
          ) : (
            <div className="absolute inset-0 flex items-center justify-center bg-secondary text-sm text-muted-foreground">
              等待第一帧…
            </div>
          )}

          {/* 左上角常驻安全提示条 */}
          <div className="absolute left-3 top-3 flex items-center gap-2 rounded-md bg-warning/90 px-3 py-1.5 text-xs font-medium text-warning-foreground shadow-lg backdrop-blur-sm">
            <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
            此终端正在被 管理员 远程协助
          </div>

          {/* O-2 裁决：删除"会话录制中"标记 */}
        </div>
      </main>
        <RemoteTools />
      </div>
    </div>
  );
}
