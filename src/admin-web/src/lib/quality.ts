// 三轴显示参数（分辨率/清晰度/帧率）的纯决策逻辑。
// 从 store.setRemoteDisplayParams 抽出：store 只负责读当前档位、写回 state、发 set_quality 信封；
// 「合并当前值 + 补丁」与「按清晰度映射兼容旧被控端的 QualityMode」这两件纯逻辑放这里，便于单测。
import type { ResolutionTier } from "@/lib/types/ResolutionTier";
import type { ClarityTier } from "@/lib/types/ClarityTier";
import type { FpsTier } from "@/lib/types/FpsTier";
import type { QualityMode } from "@/lib/types/QualityMode";

// 主控当前选定的三轴档位。
export type DisplayTiers = {
  resolution: ResolutionTier;
  clarity: ClarityTier;
  fps: FpsTier;
};

// 合并后的三轴 + 兼容旧被控端的 mode。
export type ResolvedQuality = DisplayTiers & { mode: QualityMode };

/**
 * 合并「当前三轴档位」与「本次增量补丁」，得到发给被控端的完整档位。
 * - 补丁未给的轴保留当前值（增量语义）；
 * - mode 由「合并后」的清晰度决定（high→high_quality，其余→smooth），兜底不认三轴的旧被控端。
 * 纯函数：不改动入参。
 */
export function resolveDisplayParams(
  current: DisplayTiers,
  patch: { resolution?: ResolutionTier; clarity?: ClarityTier; fps?: FpsTier },
): ResolvedQuality {
  const resolution = patch.resolution ?? current.resolution;
  const clarity = patch.clarity ?? current.clarity;
  const fps = patch.fps ?? current.fps;
  const mode: QualityMode = clarity === "high" ? "high_quality" : "smooth";
  return { resolution, clarity, fps, mode };
}
