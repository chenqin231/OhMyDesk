import { describe, it, expect } from "vitest";
import { buildKeyEvents, buildCharEvents, anyMod, NO_MODS, type Mods } from "./mobile-keys";

const CTRL: Mods = { ...NO_MODS, Control: true };
const CTRL_SHIFT: Mods = { ...NO_MODS, Control: true, Shift: true };

describe("buildKeyEvents", () => {
  it("无修饰的具名键 → 仅 down+up", () => {
    expect(buildKeyEvents("Enter", NO_MODS)).toEqual([
      { kind: "key", code: "Enter", down: true },
      { kind: "key", code: "Enter", down: false },
    ]);
  });

  it("单修饰包裹：Ctrl+c = Ctrl↓ c↓ c↑ Ctrl↑", () => {
    expect(buildKeyEvents("c", CTRL)).toEqual([
      { kind: "key", code: "Control", down: true },
      { kind: "key", code: "c", down: true },
      { kind: "key", code: "c", down: false },
      { kind: "key", code: "Control", down: false },
    ]);
  });

  it("多修饰：按下正序、释放逆序包住目标键", () => {
    const evs = buildKeyEvents("t", CTRL_SHIFT);
    expect(evs.map((e) => `${(e as any).code}:${(e as any).down}`)).toEqual([
      "Control:true",
      "Shift:true",
      "t:true",
      "t:false",
      "Shift:false",
      "Control:false",
    ]);
  });
});

describe("buildCharEvents", () => {
  it("无修饰 → 走 Text 直输(可靠含 IME)", () => {
    expect(buildCharEvents("你好", NO_MODS)).toEqual([{ kind: "text", text: "你好" }]);
  });

  it("有修饰 → 逐字符走 Key 组合", () => {
    expect(buildCharEvents("a", CTRL)).toEqual([
      { kind: "key", code: "Control", down: true },
      { kind: "key", code: "a", down: true },
      { kind: "key", code: "a", down: false },
      { kind: "key", code: "Control", down: false },
    ]);
  });

  it("空串 → 空事件", () => {
    expect(buildCharEvents("", CTRL)).toEqual([]);
  });
});

describe("anyMod", () => {
  it("无修饰 false / 有修饰 true", () => {
    expect(anyMod(NO_MODS)).toBe(false);
    expect(anyMod(CTRL)).toBe(true);
  });
});
