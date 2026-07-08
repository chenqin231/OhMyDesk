// 手机端文本输入的纯逻辑：把软键盘字符/特殊键 + 粘滞修饰键，转成发给被控端的 InputEvent 序列。
// 被控端 code_to_key 支持具名修饰键(Control/Alt/Shift/Meta)+具名功能键+单字符，组合键靠事件流
// 的修饰掩码判定（见 src/client/src/inject.rs）。这里只产事件、不碰 DOM，便于单测。
import type { InputEvent } from "@/lib/types/InputEvent";

export type Mods = { Control: boolean; Alt: boolean; Shift: boolean; Meta: boolean };
export const NO_MODS: Mods = { Control: false, Alt: false, Shift: false, Meta: false };

// 注入顺序：修饰键先按下(正序)、后释放(逆序)，包住目标键。
const MOD_ORDER: (keyof Mods)[] = ["Control", "Alt", "Shift", "Meta"];

function activeMods(mods: Mods): (keyof Mods)[] {
  return MOD_ORDER.filter((m) => mods[m]);
}

export function anyMod(mods: Mods): boolean {
  return activeMods(mods).length > 0;
}

/** 具名键(如 "Enter"/"ArrowUp")或单字符，按当前修饰键包裹成 down/up 事件序列。 */
export function buildKeyEvents(code: string, mods: Mods): InputEvent[] {
  const active = activeMods(mods);
  return [
    ...active.map((m) => ({ kind: "key", code: m, down: true }) as InputEvent),
    { kind: "key", code, down: true },
    { kind: "key", code, down: false },
    ...active
      .slice()
      .reverse()
      .map((m) => ({ kind: "key", code: m, down: false }) as InputEvent),
  ];
}

/** 软键盘输入的字符串：无修饰 → Text 直输(可靠、含 IME/emoji)；有修饰 → 逐字符走 Key 组合触发快捷键。 */
export function buildCharEvents(text: string, mods: Mods): InputEvent[] {
  if (!text) return [];
  if (!anyMod(mods)) return [{ kind: "text", text }];
  return Array.from(text).flatMap((ch) => buildKeyEvents(ch, mods));
}
