// 主控(admin)侧诊断样本：只含标量指标，绝不含帧像素/base64（脱敏白名单）。
export type DiagSample = {
  ts: number;        // epoch ms
  seq: number;       // 帧序（< 2^53 安全）
  seq_gap: number;   // 相邻帧 seq 差>1 的缺失数
  w: number;
  h: number;
};

// 入环 + 按时间窗裁剪（保留最近 maxAgeMs；满即弹头）。纯函数。
// ring 按 ts 单调递增（按接收顺序入环）：二分找首个未过期位置，slice 截前缀 + push 仅 1 次分配，
// 避免每帧 [...ring] 展开 + filter 的 O(2n) 临时数组。
export function pushDiagRing(ring: DiagSample[], s: DiagSample, maxAgeMs: number): DiagSample[] {
  const cutoff = s.ts - maxAgeMs;
  let lo = 0;
  let hi = ring.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (ring[mid].ts < cutoff) lo = mid + 1;
    else hi = mid;
  }
  const kept = ring.slice(lo);
  kept.push(s);
  return kept;
}

// 由上一帧 seq 与当前 seq 算 gap（无上一帧或回退时为 0）。
export function seqGap(lastSeq: number | null, seq: number): number {
  return lastSeq != null && seq > lastSeq + 1 ? seq - lastSeq - 1 : 0;
}
