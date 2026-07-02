import type { InputEvent } from "@/lib/types/InputEvent";

type Box = {
  left: number;
  top: number;
  width: number;
  height: number;
};

type FrameSize = {
  w: number;
  h: number;
};

type PointerPoint = {
  clientX: number;
  clientY: number;
};

export function containedFrameRect(container: Box, frame: FrameSize): Box {
  const frameW = Math.max(1, frame.w);
  const frameH = Math.max(1, frame.h);
  const scale = Math.min(container.width / frameW, container.height / frameH, 1);
  const width = frameW * scale;
  const height = frameH * scale;

  return {
    left: container.left + (container.width - width) / 2,
    top: container.top + (container.height - height) / 2,
    width,
    height,
  };
}

export function pointerToFrameCoords(
  point: PointerPoint,
  displayRect: Box,
  frame: FrameSize,
): { x: number; y: number } | null {
  const relX = point.clientX - displayRect.left;
  const relY = point.clientY - displayRect.top;
  if (relX < 0 || relY < 0 || relX > displayRect.width || relY > displayRect.height) {
    return null;
  }

  return {
    x: Math.round((relX / displayRect.width) * frame.w),
    y: Math.round((relY / displayRect.height) * frame.h),
  };
}

export function remoteMouseButtonEvents(
  coords: { x: number; y: number },
  button: number,
  down: boolean,
): InputEvent[] {
  return [
    { kind: "mouse_move", x: coords.x, y: coords.y },
    { kind: "mouse_button", button, down },
  ];
}

export function shouldBlockRemoteContextMenu(): boolean {
  return true;
}

// 滚轮:把浏览器 wheel delta(受 deltaMode 影响)归一到"格"。与桌面端一致:
// dy>0 向上、dx>0 向右;步长 40px/格,非零保底 ±1。浏览器 deltaY>0=向下,故取负。
const SCROLL_STEP_PX = 40;
function toNotch(d: number): number {
  if (d === 0) return 0;
  const n = Math.round(d / SCROLL_STEP_PX);
  return n !== 0 ? n : d > 0 ? 1 : -1;
}
export function remoteScrollEvent(deltaX: number, deltaY: number, deltaMode: number): InputEvent {
  // deltaMode: 0=像素,1=行(×16px),2=页(×~视口高,近似用 800px)。
  const unit = deltaMode === 1 ? 16 : deltaMode === 2 ? 800 : 1;
  const dxPx = deltaX * unit;
  const dyPx = deltaY * unit;
  // 浏览器 deltaY>0 表示内容向下滚;协议约定 dy>0 向上 → 取负。deltaX>0 向右 → 与协议一致。
  return { kind: "scroll", dx: toNotch(dxPx), dy: -toNotch(dyPx) };
}
