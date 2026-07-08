import { describe, it, expect } from "vitest";
import { decodeRgbaBase64, cursorTopLeft } from "./cursor-overlay";

describe("decodeRgbaBase64", () => {
  it("裸 RGBA base64 → 字节数组", () => {
    const b64 = btoa(String.fromCharCode(255, 0, 0, 255, 0, 128, 64, 32));
    const arr = decodeRgbaBase64(b64);
    expect(Array.from(arr)).toEqual([255, 0, 0, 255, 0, 128, 64, 32]);
  });

  it("空串 → 空数组", () => {
    expect(decodeRgbaBase64("").length).toBe(0);
  });
});

describe("cursorTopLeft", () => {
  it("指针位置减热点：热点对齐指针", () => {
    expect(cursorTopLeft(100, 80, 3, 4)).toEqual({ left: 97, top: 76 });
  });

  it("热点为 0：左上角即指针", () => {
    expect(cursorTopLeft(10, 20, 0, 0)).toEqual({ left: 10, top: 20 });
  });
});
