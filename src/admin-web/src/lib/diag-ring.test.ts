import { describe, it, expect } from "vitest";
import { pushDiagRing, seqGap, type DiagSample } from "./diag-ring";

const s = (ts: number, seq: number): DiagSample => ({ ts, seq, seq_gap: 0, w: 1280, h: 720 });

describe("diag-ring", () => {
  it("超窗样本被裁剪", () => {
    // 两个旧样本 ts=1000/2000，新帧 ts=302001，cutoff=302001-300000=2001
    // ts=1000 和 ts=2000 均 < 2001，应全部裁掉，只剩新帧 seq=3
    const ring = [s(1000, 1), s(2000, 2)];
    const next = pushDiagRing(ring, s(1000 + 300_000 + 1001, 3), 300_000);
    // 5min 前的样本应裁掉，只剩新帧 seq=3
    expect(next.map((x) => x.seq)).toEqual([3]);
  });
  it("窗内样本保留", () => {
    const next = pushDiagRing([s(1000, 1)], s(2000, 2), 300_000);
    expect(next.length).toBe(2);
  });
  it("seqGap 计算", () => {
    expect(seqGap(null, 1)).toBe(0);
    expect(seqGap(1, 2)).toBe(0);
    expect(seqGap(2, 5)).toBe(2); // 缺 3、4
    expect(seqGap(5, 2)).toBe(0); // 回退不计
  });
});
