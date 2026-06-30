import { describe, it, expect } from "vitest";
import {
  percent,
  startProgress,
  advanceProgress,
  completeProgress,
  failProgress,
  type ProgressMap,
} from "./file-progress";

describe("file-progress", () => {
  it("start 建立条目，初始 done=0、未失败", () => {
    const m = startProgress({}, { transfer_id: "t1", name: "a.txt", total: 200, dir: "push" });
    expect(m.t1).toMatchObject({ name: "a.txt", done: 0, total: 200, dir: "push", failed: false });
  });

  it("advance 累加字节，percent 取整 0–100", () => {
    let m: ProgressMap = startProgress({}, { transfer_id: "t1", name: "a", total: 200, dir: "push" });
    m = advanceProgress(m, "t1", 50);
    expect(percent(m.t1)).toBe(25);
    m = advanceProgress(m, "t1", 50);
    expect(percent(m.t1)).toBe(50);
  });

  it("percent 封顶 100（done 溢出 total）", () => {
    let m = startProgress({}, { transfer_id: "t1", name: "a", total: 100, dir: "push" });
    m = advanceProgress(m, "t1", 150);
    expect(percent(m.t1)).toBe(100);
  });

  it("total 未知（<=0）时 percent 返回 null", () => {
    const m = startProgress({}, { transfer_id: "t1", name: "a", total: 0, dir: "pull" });
    expect(percent(m.t1)).toBeNull();
  });

  it("complete 把 done 拉满到 total", () => {
    let m = startProgress({}, { transfer_id: "t1", name: "a", total: 200, dir: "push" });
    m = advanceProgress(m, "t1", 30);
    m = completeProgress(m, "t1");
    expect(percent(m.t1)).toBe(100);
  });

  it("fail 标记失败位", () => {
    let m = startProgress({}, { transfer_id: "t1", name: "a", total: 200, dir: "push" });
    m = failProgress(m, "t1");
    expect(m.t1.failed).toBe(true);
  });

  it("advance/complete/fail 对未知 transfer_id 是无操作", () => {
    const m: ProgressMap = {};
    expect(advanceProgress(m, "x", 10)).toBe(m);
    expect(completeProgress(m, "x")).toBe(m);
    expect(failProgress(m, "x")).toBe(m);
  });
});
