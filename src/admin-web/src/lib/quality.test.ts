import { describe, it, expect } from "vitest";
import { resolveDisplayParams, type DisplayTiers } from "./quality";

// 三轴显示参数的纯决策：合并「当前档位 + 增量补丁」并按清晰度映射兼容旧被控端的 mode。
// 这是 store.setRemoteDisplayParams 抽出的纯逻辑（store 仅负责读当前值 / set / 发信封）。
const BASE: DisplayTiers = { resolution: "r720p", clarity: "standard", fps: "smooth" };

describe("resolveDisplayParams", () => {
  it("空补丁：三轴全部保留当前值", () => {
    const r = resolveDisplayParams(BASE, {});
    expect(r.resolution).toBe("r720p");
    expect(r.clarity).toBe("standard");
    expect(r.fps).toBe("smooth");
  });

  it("只改分辨率：其余两轴保留当前值", () => {
    const r = resolveDisplayParams(BASE, { resolution: "r1080p" });
    expect(r.resolution).toBe("r1080p");
    expect(r.clarity).toBe("standard");
    expect(r.fps).toBe("smooth");
  });

  it("只改帧率：其余两轴保留当前值", () => {
    const r = resolveDisplayParams(BASE, { fps: "saver" });
    expect(r.resolution).toBe("r720p");
    expect(r.clarity).toBe("standard");
    expect(r.fps).toBe("saver");
  });

  it("同时改多轴：补丁值覆盖当前值", () => {
    const r = resolveDisplayParams(BASE, { resolution: "native", clarity: "high", fps: "standard" });
    expect(r.resolution).toBe("native");
    expect(r.clarity).toBe("high");
    expect(r.fps).toBe("standard");
  });

  it("清晰度=high → mode 映射 high_quality（兼容旧被控端 QualityMode）", () => {
    expect(resolveDisplayParams(BASE, { clarity: "high" }).mode).toBe("high_quality");
  });

  it("清晰度=standard → mode 映射 smooth", () => {
    expect(resolveDisplayParams(BASE, { clarity: "standard" }).mode).toBe("smooth");
  });

  it("mode 由「合并后」的清晰度决定，而非补丁本身", () => {
    // 当前已是 high，本次补丁只改分辨率 → 合并后 clarity 仍为 high → mode=high_quality
    const current: DisplayTiers = { resolution: "r720p", clarity: "high", fps: "smooth" };
    expect(resolveDisplayParams(current, { resolution: "r900p" }).mode).toBe("high_quality");
  });

  it("纯函数：不改动入参对象", () => {
    const current: DisplayTiers = { resolution: "r720p", clarity: "standard", fps: "smooth" };
    const patch = { clarity: "high" as const };
    resolveDisplayParams(current, patch);
    expect(current).toEqual({ resolution: "r720p", clarity: "standard", fps: "smooth" });
    expect(patch).toEqual({ clarity: "high" });
  });
});
