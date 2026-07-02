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
// dy>0 向上、dx>0 向右;步长 40px/格。浏览器 deltaY>0=向下,故取负。
//
// 用**像素累加器**而非"每事件保底 ±1":触摸板/惯性一次手势会连发几十个小 delta,
// 保底 ±1 会滚几十格(飞很远)。累加 px、满一格才发整数格、余量留存 → 发出的总格数
// ≈ 物理滚动距离/步长,与事件个数无关。每个会话建一个累加器(useRef 持有)。
const SCROLL_STEP_PX = 40;
export function makeRemoteScroll(): (
  deltaX: number,
  deltaY: number,
  deltaMode: number,
) => InputEvent | null {
  let accX = 0;
  let accY = 0;
  return (deltaX, deltaY, deltaMode) => {
    // deltaMode: 0=像素,1=行(×16px),2=页(×~视口高,近似用 800px)。
    const unit = deltaMode === 1 ? 16 : deltaMode === 2 ? 800 : 1;
    accX += deltaX * unit;
    accY += deltaY * unit;
    const nx = Math.trunc(accX / SCROLL_STEP_PX); // 满格数(向零取整,保号)
    const ny = Math.trunc(accY / SCROLL_STEP_PX);
    accX -= nx * SCROLL_STEP_PX; // 余量留到下次,不丢距离
    accY -= ny * SCROLL_STEP_PX;
    if (nx === 0 && ny === 0) return null;
    // 浏览器 deltaY>0=内容向下;协议约定 dy>0 向上 → 取负。deltaX>0 向右 → 与协议一致。
    return { kind: "scroll", dx: nx, dy: -ny };
  };
}
