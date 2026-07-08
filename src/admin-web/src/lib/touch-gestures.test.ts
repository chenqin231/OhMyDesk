import { describe, it, expect } from "vitest";
import { TouchGestureEngine, DEFAULT_TOUCH_OPTS, type GestureAction } from "./touch-gestures";

function eng(cursor = { x: 100, y: 100 }) {
  return new TouchGestureEngine(1280, 720, cursor, DEFAULT_TOUCH_OPTS);
}

function kinds(a: GestureAction[]) {
  return a.map((x) => x.kind);
}

describe("单指轻点 → 左键单击（落在虚拟光标处）", () => {
  it("按下→短时间内原地抬起 = move + 左键 down + up", () => {
    const e = eng({ x: 100, y: 100 });
    e.start([{ x: 50, y: 50 }], 1000);
    const out = e.end([], 1100); // 100ms < tapMaxMs，无移动
    expect(out).toEqual([
      { kind: "move", x: 100, y: 100 },
      { kind: "button", button: 0, down: true },
      { kind: "button", button: 0, down: false },
    ]);
  });

  it("按下过久（超 tapMaxMs）不算轻点", () => {
    const e = eng();
    e.start([{ x: 50, y: 50 }], 0);
    expect(e.end([], 9999)).toEqual([]);
  });
});

describe("单指移动 → 触控板相对移动虚拟光标", () => {
  it("位移 × 增益累加到虚拟光标（不发左键）", () => {
    const e = eng({ x: 100, y: 100 });
    e.start([{ x: 50, y: 50 }], 0);
    const out = e.move([{ x: 70, y: 50 }], 16); // 位移 20px>10 → move；dx=20*1.6=32
    expect(out).toEqual([{ kind: "move", x: 132, y: 100 }]);
    expect(e.getCursor().x).toBe(132);
  });

  it("移动后抬起不产生点击", () => {
    const e = eng();
    e.start([{ x: 50, y: 50 }], 0);
    e.move([{ x: 90, y: 90 }], 16);
    expect(e.end([], 40)).toEqual([]);
  });
});

describe("长按 → 右键单击", () => {
  it("longPressFire 原地未移动 = move + 右键 down + up", () => {
    const e = eng({ x: 100, y: 100 });
    e.start([{ x: 50, y: 50 }], 0);
    const out = e.longPressFire();
    expect(out).toEqual([
      { kind: "move", x: 100, y: 100 },
      { kind: "button", button: 2, down: true },
      { kind: "button", button: 2, down: false },
    ]);
    // 长按已触发 → 抬手不再左键点击
    expect(e.end([], 600)).toEqual([]);
  });

  it("已移动则 longPressFire 不触发右键", () => {
    const e = eng();
    e.start([{ x: 50, y: 50 }], 0);
    e.move([{ x: 90, y: 50 }], 16);
    expect(e.longPressFire()).toEqual([]);
  });
});

describe("双指拖动 → 滚动", () => {
  it("两指中点竖直位移 → scroll（自然方向）", () => {
    const e = eng();
    e.start([{ x: 0, y: 100 }, { x: 100, y: 100 }], 0); // 中点 (50,100)
    const out = e.move([{ x: 0, y: 20 }, { x: 100, y: 20 }], 16); // 中点上移 80 → dy=-80/40=-2 → scroll dy=2
    expect(out).toEqual([{ kind: "scroll", dx: 0, dy: 2 }]);
  });
});

describe("双击后按住拖动 → 左键拖拽", () => {
  it("首轻点 → 第二次按下+移动补左键 down，抬起补 up", () => {
    const e = eng({ x: 100, y: 100 });
    // 第一次轻点
    e.start([{ x: 50, y: 50 }], 0);
    e.end([], 100); // lastTapEnd=100
    // 第二次：300ms 内再次按下（双击候选）
    e.start([{ x: 50, y: 50 }], 150);
    const moveOut = e.move([{ x: 80, y: 50 }], 200); // dx=30*1.6=48 → cursor.x=148
    expect(kinds(moveOut)).toEqual(["move", "button", "move"]); // 补 down 后再 move
    expect(moveOut).toContainEqual({ kind: "button", button: 0, down: true });
    // 抬起 → 松左键
    expect(e.end([], 300)).toEqual([{ kind: "button", button: 0, down: false }]);
  });
});
