import { useCallback, useEffect, useRef, useState } from "react";
import { Keyboard, X } from "lucide-react";
import type { InputEvent as WireInput } from "@/lib/types/InputEvent";
import { buildKeyEvents, buildCharEvents, anyMod, NO_MODS, type Mods } from "@/lib/mobile-keys";

// 特殊键（软键盘缺失或不便的键）：具名 code 由被控端 code_to_key 映射。
const SPECIAL_KEYS: { label: string; code: string }[] = [
  { label: "Esc", code: "Escape" },
  { label: "Tab", code: "Tab" },
  { label: "↑", code: "ArrowUp" },
  { label: "↓", code: "ArrowDown" },
  { label: "←", code: "ArrowLeft" },
  { label: "→", code: "ArrowRight" },
  { label: "Del", code: "Delete" },
  { label: "Home", code: "Home" },
  { label: "End", code: "End" },
  { label: "PgUp", code: "PageUp" },
  { label: "PgDn", code: "PageDown" },
];
// 粘滞修饰键（一次性：发一次键后自动清）。Win=Meta。
const MOD_KEYS: { label: string; key: keyof Mods }[] = [
  { label: "Ctrl", key: "Control" },
  { label: "Alt", key: "Alt" },
  { label: "Shift", key: "Shift" },
  { label: "Win", key: "Meta" },
];

// 手机端文本输入栏：隐藏 input 唤起软键盘 + beforeinput/composition 捕获输入 → 发 Text/Key；
// 特殊键工具条 + 一次性粘滞修饰键（Ctrl/Alt/Shift/Win）。渲染在远控画面下方（不覆盖触控手势区）。
export function MobileKeyboardBar({
  sendInput,
  onClose,
}: {
  sendInput: (e: WireInput) => void;
  onClose?: () => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const composingRef = useRef(false);
  const modsRef = useRef<Mods>(NO_MODS);
  const [mods, setMods] = useState<Mods>(NO_MODS);
  const [kbOpen, setKbOpen] = useState(false);

  const setModsBoth = useCallback((next: Mods) => {
    modsRef.current = next;
    setMods(next);
  }, []);

  // 发一批事件后，若有一次性粘滞修饰键则清空（下一个键不再带修饰）。
  const flush = useCallback(
    (evs: WireInput[]) => {
      evs.forEach(sendInput);
      if (anyMod(modsRef.current)) setModsBoth(NO_MODS);
    },
    [sendInput, setModsBoth],
  );

  const sendKey = useCallback((code: string) => flush(buildKeyEvents(code, modsRef.current)), [flush]);
  const sendChars = useCallback((text: string) => flush(buildCharEvents(text, modsRef.current)), [flush]);

  const toggleMod = useCallback(
    (k: keyof Mods) => setModsBoth({ ...modsRef.current, [k]: !modsRef.current[k] }),
    [setModsBoth],
  );

  // 原生 beforeinput / composition 捕获软键盘输入（React 合成事件不可靠）。
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const onComposStart = () => {
      composingRef.current = true;
    };
    const onComposEnd = (e: CompositionEvent) => {
      composingRef.current = false;
      if (e.data) sendChars(e.data);
      el.value = "";
    };
    const onBeforeInput = (e: globalThis.InputEvent) => {
      if (composingRef.current) return; // IME 组合中：等 compositionend 一次性发
      const t = e.inputType;
      const data = e.data;
      if (t === "insertText" && data) {
        e.preventDefault();
        sendChars(data);
      } else if (t === "deleteContentBackward") {
        e.preventDefault();
        sendKey("Backspace");
      } else if (t === "deleteContentForward") {
        e.preventDefault();
        sendKey("Delete");
      } else if (t === "insertLineBreak" || t === "insertParagraph") {
        e.preventDefault();
        sendKey("Enter");
      }
      el.value = "";
    };
    el.addEventListener("compositionstart", onComposStart);
    el.addEventListener("compositionend", onComposEnd);
    el.addEventListener("beforeinput", onBeforeInput);
    return () => {
      el.removeEventListener("compositionstart", onComposStart);
      el.removeEventListener("compositionend", onComposEnd);
      el.removeEventListener("beforeinput", onBeforeInput);
    };
  }, [sendChars, sendKey]);

  const openKeyboard = useCallback(() => {
    inputRef.current?.focus();
    setKbOpen(true);
  }, []);

  // 键盘栏出现即自动聚焦隐藏 input 唤起软键盘（Android 有效；iOS 若不弹，点栏内「键盘」按钮补触发）。
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 特殊键/修饰键按下时阻止默认，避免抢走隐藏 input 焦点（软键盘不收起）。
  const keepFocus = (e: React.PointerEvent) => e.preventDefault();

  const btn =
    "shrink-0 rounded-md border border-border px-2.5 py-1.5 text-xs font-medium text-foreground active:bg-secondary";

  return (
    <div className="flex shrink-0 items-center gap-1.5 overflow-x-auto border-t border-border bg-card px-2 py-1.5">
      {/* 隐藏 input：置视口内(opacity 0)以确保移动端 focus 能唤起软键盘 */}
      <input
        ref={inputRef}
        aria-label="远程文本输入"
        autoCapitalize="off"
        autoCorrect="off"
        autoComplete="off"
        spellCheck={false}
        className="pointer-events-none absolute bottom-0 left-0 h-px w-px opacity-0"
        onBlur={() => setKbOpen(false)}
      />
      {onClose && (
        <button type="button" onPointerDown={keepFocus} onClick={onClose} className={btn} aria-label="收起键盘">
          <X className="size-3.5" />
        </button>
      )}
      <button
        type="button"
        onClick={openKeyboard}
        className={`${btn} flex items-center gap-1 ${kbOpen ? "bg-primary/15 text-primary" : ""}`}
      >
        <Keyboard className="size-3.5" />
        键盘
      </button>
      {MOD_KEYS.map((m) => (
        <button
          key={m.key}
          type="button"
          onPointerDown={keepFocus}
          onClick={() => toggleMod(m.key)}
          className={`${btn} ${mods[m.key] ? "bg-primary/20 text-primary" : ""}`}
        >
          {m.label}
        </button>
      ))}
      {SPECIAL_KEYS.map((k) => (
        <button
          key={k.code}
          type="button"
          onPointerDown={keepFocus}
          onClick={() => sendKey(k.code)}
          className={btn}
        >
          {k.label}
        </button>
      ))}
    </div>
  );
}
