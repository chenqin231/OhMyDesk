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
