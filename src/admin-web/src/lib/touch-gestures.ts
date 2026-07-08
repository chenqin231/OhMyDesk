// 手机端触控手势引擎（借鉴 UU 远程「触控板模式」）：把触摸事件翻译成发给被控端的 InputEvent
// + 本地虚拟光标位置。纯逻辑、无 DOM、无计时器（长按计时由组件的 setTimeout 驱动，调 longPressFire），
// 便于单测。坐标全程用「帧坐标系」（与 Frame w/h 一致，被控端注入侧按比例还原到真实屏）。
//
// 手势映射：
//   单指移动         → 触控板相对移动虚拟光标（mouse_move）
//   单指轻点         → 左键单击（move→down→up，落在虚拟光标处）
//   长按（按住不动） → 右键单击（组件计时触发 longPressFire）
//   双指拖动         → 滚动（scroll，dx/dy 单位=格）
//   双击后按住拖动   → 左键拖拽（down→move…→up）

export type Pt = { x: number; y: number };

/** 引擎输出动作：move/button 直接映射 InputEvent；scroll 同理。 */
export type GestureAction =
  | { kind: "move"; x: number; y: number }
  | { kind: "button"; button: number; down: boolean }
  | { kind: "scroll"; dx: number; dy: number };

export type TouchGestureOpts = {
  tapMaxMs: number; // 轻点判定：按下→抬起时长上限
  tapMaxMovePx: number; // 轻点判定：位移上限（屏幕 px）
  sensitivity: number; // 触控板相对移动增益（帧坐标/屏幕 px）
  scrollDivisor: number; // 双指位移(px)→滚动格 的除数
  doubleTapMaxMs: number; // 双击判定：两次轻点间隔上限
};

export const DEFAULT_TOUCH_OPTS: TouchGestureOpts = {
  tapMaxMs: 250,
  tapMaxMovePx: 10,
  sensitivity: 1.6,
  scrollDivisor: 40,
  doubleTapMaxMs: 300,
};

const LEFT = 0;
const RIGHT = 2;

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

function dist(a: Pt, b: Pt): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/** 触控手势状态机。虚拟光标位置以帧坐标维护，clamp 在 [0,frameW]×[0,frameH]。 */
export class TouchGestureEngine {
  private cursor: Pt;
  private opts: TouchGestureOpts;

  // 单指追踪
  private startScreen: Pt | null = null;
  private lastScreen: Pt | null = null;
  private startTime = 0;
  private moved = false;
  private longFired = false;
  private dragging = false; // 双击拖拽中：左键按住
  private lastTapEnd = Number.NEGATIVE_INFINITY; // 上次单指轻点抬起时刻（双击判定）；初值 -∞ 使首次手势不被误判为双击
  private pendingDoubleHold = false; // 本次按下是双击的第二次（可能进入拖拽）

  // 双指追踪
  private twoLast: Pt | null = null;

  constructor(
    frameW: number,
    frameH: number,
    cursor: Pt,
    opts: TouchGestureOpts = DEFAULT_TOUCH_OPTS,
  ) {
    this.frameW = frameW;
    this.frameH = frameH;
    this.cursor = { x: clamp(cursor.x, 0, frameW), y: clamp(cursor.y, 0, frameH) };
    this.opts = opts;
  }
  private frameW: number;
  private frameH: number;

  getCursor(): Pt {
    return { x: this.cursor.x, y: this.cursor.y };
  }

  /** 更新帧尺寸（分辨率档切换后帧 w/h 变）。虚拟光标重新 clamp。 */
  setFrameSize(w: number, h: number): void {
    this.frameW = w;
    this.frameH = h;
    this.cursor = { x: clamp(this.cursor.x, 0, w), y: clamp(this.cursor.y, 0, h) };
  }

  /** 触摸开始。touches=当前所有触点（屏幕 px）。 */
  start(touches: Pt[], now: number): GestureAction[] {
    if (touches.length >= 2) {
      // 进入双指：以两指中点为滚动基准。
      this.twoLast = midpoint(touches[0], touches[1]);
      this.startScreen = null; // 取消单指判定
      return [];
    }
    const p = touches[0];
    this.startScreen = p;
    this.lastScreen = p;
    this.startTime = now;
    this.moved = false;
    this.longFired = false;
    // 双击第二次：上次轻点后极短内再次按下 → 标记，可能进入拖拽。
    this.pendingDoubleHold = now - this.lastTapEnd <= this.opts.doubleTapMaxMs;
    return [];
  }

  /** 触摸移动。（now 预留：move 目前不依赖时间，签名与 start/end 对齐。） */
  move(touches: Pt[], _now: number): GestureAction[] {
    if (touches.length >= 2) {
      // 双指滚动：中点位移 → 滚动格。
      const mid = midpoint(touches[0], touches[1]);
      if (!this.twoLast) {
        this.twoLast = mid;
        return [];
      }
      const dx = Math.trunc((mid.x - this.twoLast.x) / this.opts.scrollDivisor);
      const dy = Math.trunc((mid.y - this.twoLast.y) / this.opts.scrollDivisor);
      if (dx === 0 && dy === 0) return [];
      this.twoLast = mid;
      // 自然滚动：手指下移=内容下移=滚轮向下（dy<0）。屏幕 y 向下为正 → dy 取负。
      return [{ kind: "scroll", dx, dy: -dy }];
    }

    const p = touches[0];
    if (!this.startScreen || !this.lastScreen) return [];
    if (dist(p, this.startScreen) > this.opts.tapMaxMovePx) this.moved = true;

    // 触控板相对移动：屏幕位移 × 增益 → 虚拟光标增量。
    const dx = (p.x - this.lastScreen.x) * this.opts.sensitivity;
    const dy = (p.y - this.lastScreen.y) * this.opts.sensitivity;
    this.lastScreen = p;
    if (this.moved && !this.longFired) {
      this.cursor = {
        x: clamp(this.cursor.x + dx, 0, this.frameW),
        y: clamp(this.cursor.y + dy, 0, this.frameH),
      };
      const out: GestureAction[] = [];
      // 双击后按住拖动：首次移动时补发左键 down，进入拖拽。
      if (this.pendingDoubleHold && !this.dragging) {
        this.dragging = true;
        out.push({ kind: "move", x: Math.round(this.cursor.x), y: Math.round(this.cursor.y) });
        out.push({ kind: "button", button: LEFT, down: true });
      }
      out.push({ kind: "move", x: Math.round(this.cursor.x), y: Math.round(this.cursor.y) });
      return out;
    }
    return [];
  }

  /** 触摸结束。touches=抬手后仍在的触点。 */
  end(touches: Pt[], now: number): GestureAction[] {
    // 仍有触点（多指里松了一根）：重置双指基准，不产生动作。
    if (touches.length >= 1) {
      this.twoLast = touches.length >= 2 ? midpoint(touches[0], touches[1]) : null;
      this.startScreen = null;
      return [];
    }
    this.twoLast = null;

    const out: GestureAction[] = [];
    // 拖拽结束：松开左键。
    if (this.dragging) {
      this.dragging = false;
      this.pendingDoubleHold = false;
      out.push({ kind: "button", button: LEFT, down: false });
      this.startScreen = null;
      return out;
    }
    // 长按已触发过右键：本次抬手不再产生点击。
    if (this.longFired) {
      this.longFired = false;
      this.startScreen = null;
      this.pendingDoubleHold = false;
      return out;
    }
    // 轻点判定：时长短 + 未移动 → 左键单击（落在虚拟光标处）。
    if (this.startScreen && !this.moved && now - this.startTime <= this.opts.tapMaxMs) {
      const x = Math.round(this.cursor.x);
      const y = Math.round(this.cursor.y);
      out.push({ kind: "move", x, y });
      out.push({ kind: "button", button: LEFT, down: true });
      out.push({ kind: "button", button: LEFT, down: false });
      this.lastTapEnd = now;
    }
    this.startScreen = null;
    this.pendingDoubleHold = false;
    return out;
  }

  /** 长按触发（组件 setTimeout 到点调用）：按住未移动则右键单击。 */
  longPressFire(): GestureAction[] {
    if (!this.startScreen || this.moved || this.longFired || this.dragging) return [];
    this.longFired = true;
    const x = Math.round(this.cursor.x);
    const y = Math.round(this.cursor.y);
    return [
      { kind: "move", x, y },
      { kind: "button", button: RIGHT, down: true },
      { kind: "button", button: RIGHT, down: false },
    ];
  }
}

function midpoint(a: Pt, b: Pt): Pt {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}
