// 光标同步（主控 web 端）纯逻辑：被控端下发的裸 RGBA base64 解码 + 叠加层定位。
// canvas 渲染（RGBA→dataURL）依赖 DOM，放组件里；这里只留可单测的纯函数。

/** 裸 RGBA base64（被控端 CursorShape.rgba）→ Uint8ClampedArray（喂 canvas ImageData）。 */
export function decodeRgbaBase64(b64: string): Uint8ClampedArray {
  const bin = atob(b64);
  const out = new Uint8ClampedArray(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** 裸 RGBA base64 → data URL（canvas 编码 PNG，带 alpha）。
 * DOM 依赖：仅浏览器运行时调用，勿在 node 单测里调用（document 在函数体内引用，import 本模块不受影响）。 */
export function rgbaToDataUrl(b64: string, w: number, h: number): string {
  if (!b64 || w <= 0 || h <= 0) return "";
  const data = decodeRgbaBase64(b64);
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.putImageData(new ImageData(data, w, h), 0, 0);
  return canvas.toDataURL();
}

/** 光标叠加层左上角坐标：指针位置减去热点偏移，令光标热点对齐真实指针位置。 */
export function cursorTopLeft(
  pointerX: number,
  pointerY: number,
  hotspotX: number,
  hotspotY: number,
): { left: number; top: number } {
  return { left: pointerX - hotspotX, top: pointerY - hotspotY };
}
