// 文件传输进度的纯逻辑：进度条目类型 + 百分比计算 + 累计/完成/失败 reducer。
// 抽成纯函数便于单测；store 持有 fileProgress 映射并调用这里的 reducer。

import type { FileDir } from "@/lib/types/FileDir";

// 一笔在途/完成的传输。done/total 为字节数；failed 与 done>=total(完成) 互斥语义由 UI 解读。
export type FileProgress = {
  transfer_id: string;
  name: string;
  done: number;
  total: number; // 0 表示未知大小（pull 首包可能未带 total）
  dir: FileDir;
  failed: boolean;
};

export type ProgressMap = Record<string, FileProgress>;

// 百分比（0–100 整数）。total<=0（未知）时返回 null（UI 显示不确定态）。
export function percent(p: FileProgress): number | null {
  if (p.total <= 0) return null;
  return Math.min(100, Math.round((p.done / p.total) * 100));
}

// 开始一笔传输（push file_open / pull file_open）：建条目。
export function startProgress(
  map: ProgressMap,
  args: { transfer_id: string; name: string; total: number; dir: FileDir },
): ProgressMap {
  return {
    ...map,
    [args.transfer_id]: {
      transfer_id: args.transfer_id,
      name: args.name,
      done: 0,
      total: args.total,
      dir: args.dir,
      failed: false,
    },
  };
}

// 累加已传字节（push 每发一片 / pull 每收一片）。未知条目忽略。
export function advanceProgress(map: ProgressMap, transfer_id: string, bytes: number): ProgressMap {
  const cur = map[transfer_id];
  if (!cur) return map;
  return { ...map, [transfer_id]: { ...cur, done: cur.done + bytes } };
}

// 标记完成（done 拉满到 total，total 未知时用当前 done）。
export function completeProgress(map: ProgressMap, transfer_id: string): ProgressMap {
  const cur = map[transfer_id];
  if (!cur) return map;
  return {
    ...map,
    [transfer_id]: { ...cur, done: cur.total > 0 ? cur.total : cur.done },
  };
}

// 标记失败。
export function failProgress(map: ProgressMap, transfer_id: string): ProgressMap {
  const cur = map[transfer_id];
  if (!cur) return map;
  return { ...map, [transfer_id]: { ...cur, failed: true } };
}
