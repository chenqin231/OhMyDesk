// 命令历史回溯的纯逻辑：给定历史数组（最新在末尾）+ 当前游标，计算 ↑/↓ 后的新游标与回填文本。
// 游标语义：null = 不在浏览历史（停在用户当前输入）；0..len-1 = 指向 history[idx]。
// ↑ 越往旧走（idx 减小，最旧为 0）；↓ 越往新走，越过最新回到 null（清空到当前输入）。

export type HistoryNav = { cursor: number | null; text: string | null };
// text=null 表示「保持输入框现有文本不变」；string 表示「回填为该文本」。

export function navPrev(history: string[], cursor: number | null): HistoryNav {
  if (history.length === 0) return { cursor: null, text: null };
  // 初次按 ↑：从最新一条（末尾）开始。
  const next = cursor === null ? history.length - 1 : Math.max(0, cursor - 1);
  return { cursor: next, text: history[next] };
}

export function navNext(history: string[], cursor: number | null): HistoryNav {
  if (cursor === null) return { cursor: null, text: null };
  const next = cursor + 1;
  if (next >= history.length) return { cursor: null, text: "" }; // 越过最新 → 回到空白当前输入
  return { cursor: next, text: history[next] };
}

// 提交一条命令后追加进历史（去掉与上一条完全相同的连续重复），返回新历史。
export function pushHistory(history: string[], cmd: string): string[] {
  const t = cmd.trim();
  if (!t) return history;
  if (history.length > 0 && history[history.length - 1] === t) return history;
  return [...history, t];
}
