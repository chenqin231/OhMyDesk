// 主控(admin)侧诊断样本：只含标量指标，绝不含帧像素/base64（脱敏白名单）。
export type DiagSample = {
  ts: number;        // epoch ms
  seq: number;       // 帧序（< 2^53 安全）
  seq_gap: number;   // 相邻帧 seq 差>1 的缺失数
  w: number;
  h: number;
};

// 入环 + 按时间窗裁剪（保留最近 maxAgeMs；满即弹头）。纯函数。
export function pushDiagRing(ring: DiagSample[], s: DiagSample, maxAgeMs: number): DiagSample[] {
  const cutoff = s.ts - maxAgeMs;
  return [...ring, s].filter((x) => x.ts >= cutoff);
}

// 由上一帧 seq 与当前 seq 算 gap（无上一帧或回退时为 0）。
export function seqGap(lastSeq: number | null, seq: number): number {
  return lastSeq != null && seq > lastSeq + 1 ? seq - lastSeq - 1 : 0;
}
