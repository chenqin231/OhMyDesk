//! 过载自适应闭环：collector 每 10s 窗 observe → 写 LEVEL 原子量 → 抓帧线程 clamp 折入。
//! 以用户手动档为上限，只降不升；迟滞（2 窗降/3 窗升）；env/config 可秒关。纯逻辑优先单测。

use crate::capture::QualityParams;
use crate::telemetry::WindowStats;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const MAX_LEVEL: u8 = 3;
const OVERLOAD_TO_DEGRADE: u8 = 2; // 连续 2 窗过载 → 降一档
const HEALTHY_TO_RECOVER: u8 = 3;  // 连续 3 窗健康 → 回升一档（比降档慢=更安全）

// 过载/健康判定阈值（口径与 telemetry::classify 一致；若那边改，这里同步）。
const ENC_OVERLOAD_P95_MS: u32 = 200; // 编码 p95 超此判过载
const ENC_HEALTHY_P95_MS: u32 = 120;  // 编码 p95 低于此才算健康（留 80ms 裕度防抖）
const STARVE_FPS: f32 = 1.0;          // 投递饥饿/健康的有效 fps 分界
const STARVE_DIRTY_P95: f32 = 0.1;    // 投递饥饿的脏区分界

// 自适应降档的分辨率地板，避免降到不可用
const FLOOR_W: u32 = 320;
const FLOOR_H: u32 = 180;

static LEVEL: AtomicU8 = AtomicU8::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
/// 手动切档请求重置的标志：dispatch 收到 SetQuality 时置位，collector 每窗取一次并重建控制器。
static PENDING_RESET: AtomicBool = AtomicBool::new(false);

/// 抓帧线程读取的当前档（关闭时恒 0=旁路）。
pub fn level() -> u8 {
    if ENABLED.load(Ordering::Relaxed) { LEVEL.load(Ordering::Relaxed) } else { 0 }
}
pub fn store_level(l: u8) { LEVEL.store(l.min(MAX_LEVEL), Ordering::Relaxed); }
pub fn set_enabled(on: bool) { ENABLED.store(on, Ordering::Relaxed); }
pub fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }

/// 手动切换画质时请求重置自适应：**立即**把 level 归零（抓帧线程下一帧即解除降档钳制，让用户
/// 手动选择先生效），并置标志让 collector 下一窗重建控制器（清空 streak，需重新累计过载才再降）。
/// 解决「弱机上手动切高清被 adaptive 立即拉回」的拉锯。
pub fn request_reset() {
    store_level(0);
    PENDING_RESET.store(true, Ordering::Relaxed);
}
/// collector 每窗取一次：若上次请求过重置则返回 true（并清标志），据此重建 AdaptiveController。
pub fn take_reset() -> bool {
    PENDING_RESET.swap(false, Ordering::Relaxed)
}

/// 过载：与 telemetry::classify 的「编码过载/投递饥饿」口径一致。
pub fn is_overload(s: &WindowStats) -> bool {
    s.enc_p95_ms > ENC_OVERLOAD_P95_MS || (s.effective_fps < STARVE_FPS && s.dirty_p95 > STARVE_DIRTY_P95)
}
/// 健康：留裕度（enc_p95<ENC_HEALTHY_P95_MS 且 fps≥STARVE_FPS）防止阈值边界抖动。
/// 注：idle/frameskip 下静止画面少发帧 → fps 可能 <1 → 既非过载也非健康(中间态)，
/// 恢复被冻结属预期（无画面变化时不需回升；一旦有活动、降档后编码更快→fps 回升即自然恢复）。
pub fn is_healthy(s: &WindowStats) -> bool {
    s.enc_p95_ms < ENC_HEALTHY_P95_MS && s.effective_fps >= STARVE_FPS
}

/// 迟滞状态机（纯逻辑，单测友好）。
#[derive(Default)]
pub struct AdaptiveController {
    pub level: u8,
    overload_streak: u8,
    healthy_streak: u8,
}
impl AdaptiveController {
    /// 喂一窗统计，更新并返回新 level。
    pub fn observe(&mut self, s: &WindowStats) -> u8 {
        if is_overload(s) {
            self.healthy_streak = 0;
            self.overload_streak = self.overload_streak.saturating_add(1);
            if self.overload_streak >= OVERLOAD_TO_DEGRADE && self.level < MAX_LEVEL {
                self.level += 1;
                self.overload_streak = 0;
            }
        } else if is_healthy(s) {
            self.overload_streak = 0;
            self.healthy_streak = self.healthy_streak.saturating_add(1);
            if self.healthy_streak >= HEALTHY_TO_RECOVER && self.level > 0 {
                self.level -= 1;
                self.healthy_streak = 0;
            }
        } else {
            // 中间态：非过载也非健康 → 断两侧连击（要求连续才动作）。
            self.overload_streak = 0;
            self.healthy_streak = 0;
        }
        self.level
    }
}

/// 在已解析档位之上按 level 再钳（只降不升）。分辨率对 JPEG 耗时影响最大，优先降分辨率。
pub fn clamp(p: QualityParams, level: u8) -> QualityParams {
    let (res_ratio, q_down, iv_mul): (f32, u8, f32) = match level {
        0 => return p,
        1 => (0.85, 0, 1.25),
        2 => (0.70, 8, 1.5),
        _ => (0.55, 12, 2.0),
    };
    QualityParams {
        max_w: (((p.max_w as f32) * res_ratio) as u32).max(FLOOR_W).min(p.max_w),
        max_h: (((p.max_h as f32) * res_ratio) as u32).max(FLOOR_H).min(p.max_h),
        jpeg_q: p.jpeg_q.saturating_sub(q_down),
        interval_ms: ((p.interval_ms as f32) * iv_mul) as u64,
    }
}

/// 开关解析：env OHMYDESK_ADAPTIVE 优先（0/false/off/no=关），否则 config，默认 ON。
pub fn resolve_enabled(env_val: Option<&str>, config_val: Option<bool>) -> bool {
    if let Some(v) = env_val {
        return !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no");
    }
    config_val.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::QualityParams;

    fn overload() -> crate::telemetry::WindowStats {
        crate::telemetry::WindowStats { enc_p95_ms: 250, ..Default::default() }
    }
    fn healthy() -> crate::telemetry::WindowStats {
        crate::telemetry::WindowStats { enc_p95_ms: 50, effective_fps: 10.0, ..Default::default() }
    }

    #[test]
    fn 连续两窗过载_降一档() {
        let mut c = AdaptiveController::default();
        assert_eq!(c.observe(&overload()), 0, "第1窗过载不降");
        assert_eq!(c.observe(&overload()), 1, "第2窗过载降到1");
    }

    #[test]
    fn 连续三窗健康_回升一档() {
        let mut c = AdaptiveController::default();
        c.observe(&overload()); c.observe(&overload()); // → level 1
        assert_eq!(c.observe(&healthy()), 1, "健康1窗不升");
        assert_eq!(c.observe(&healthy()), 1, "健康2窗不升");
        assert_eq!(c.observe(&healthy()), 0, "健康3窗回升到0");
    }

    #[test]
    fn 交替过载健康_不累积降档() {
        let mut c = AdaptiveController::default();
        c.observe(&overload()); c.observe(&healthy());
        c.observe(&overload()); c.observe(&healthy());
        assert_eq!(c.level, 0, "过载不连续 → 不降档");
    }

    #[test]
    fn level有界_0到3() {
        let mut c = AdaptiveController::default();
        for _ in 0..20 { c.observe(&overload()); }
        assert_eq!(c.level, MAX_LEVEL, "封顶 3");
        for _ in 0..20 { c.observe(&healthy()); }
        assert_eq!(c.level, 0, "回落到 0");
    }

    #[test]
    fn clamp梯度_只降不升() {
        let p = QualityParams { max_w: 1280, max_h: 720, jpeg_q: 80, interval_ms: 40 };
        assert_eq!(clamp(p, 0), p, "level0 原样");
        let c1 = clamp(p, 1);
        assert!(c1.max_w < p.max_w && c1.interval_ms > p.interval_ms && c1.jpeg_q <= p.jpeg_q);
        let c3 = clamp(p, 3);
        assert!(c3.max_w < c1.max_w && c3.jpeg_q < p.jpeg_q, "档越高降越多");
    }

    #[test]
    fn resolve_enabled优先级() {
        assert!(!resolve_enabled(Some("0"), Some(true)), "env=0 关(压过 config)");
        assert!(resolve_enabled(Some("1"), Some(false)), "env=1 开");
        assert!(!resolve_enabled(None, Some(false)), "config=false 关");
        assert!(resolve_enabled(None, None), "默认 ON");
    }

    #[test]
    fn level_读受开关门控() {
        set_enabled(false);
        store_level(3);
        assert_eq!(level(), 0, "关闭时 level() 恒 0(旁路)");
        set_enabled(true);
        assert_eq!(level(), 3, "开启时反映实际档");
        // 手动切档重置（并入此唯一触及全局 LEVEL/ENABLED 的测试，避免并发竞争）：
        store_level(3);
        request_reset();
        assert_eq!(level(), 0, "request_reset 后 level() 立即为 0");
        assert!(take_reset(), "take_reset 首次为真");
        assert!(!take_reset(), "再次为假（已清）");
        // 重置=collector 重建控制器后，一窗过载不应立即回到高档
        let mut c = AdaptiveController::default();
        assert_eq!(c.observe(&overload()), 0, "重建后第1窗过载不降");
        store_level(0); // 复位避免污染其它测试
    }
}
