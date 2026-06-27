//! 等比缩放与坐标换算的纯逻辑（P-CLI4 单一事实源）。
//!
//! 截屏侧用 [`fit_scale`] 算等比缩放后的帧尺寸（长边不超过上限、绝不放大）；
//! 注入侧用 [`map_frame_to_real`] 把主控发来的帧内坐标按 real/frame 比例还原成被控真实屏坐标。
//! 两侧共用同一套换算，保证「截多大、点哪里」自洽，非 16:9 屏也不偏。

/// 远控帧长边上限（宽 ≤ MAX_W 且高 ≤ MAX_H，等比，不放大）。
pub const MAX_W: u32 = 1280;
pub const MAX_H: u32 = 720;

/// 等比缩放比例：保证缩放后 w ≤ max_w 且 h ≤ max_h，且不放大（比例 ≤ 1.0）。
///
/// 返回值乘以原尺寸即得目标尺寸。源任一维为 0 时返回 1.0（交由上层处理空图）。
pub fn fit_scale(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> f32 {
    if src_w == 0 || src_h == 0 {
        return 1.0;
    }
    (max_w as f32 / src_w as f32)
        .min(max_h as f32 / src_h as f32)
        .min(1.0)
}

/// 按 [`fit_scale`] 算出等比缩放后的整数尺寸（至少 1×1，避免 0 尺寸图）。
pub fn scaled_dims(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let s = fit_scale(src_w, src_h, max_w, max_h);
    let w = ((src_w as f32 * s) as u32).max(1);
    let h = ((src_h as f32 * s) as u32).max(1);
    (w, h)
}

/// 把帧内坐标 `(fx, fy)`（基于缩放后的帧尺寸 `frame_w×frame_h`）还原为被控真实屏坐标
/// （基于 `real_w×real_h`）。frame 任一维为 0 时按 1 处理，避免除零。
pub fn map_frame_to_real(
    fx: i32,
    fy: i32,
    frame_w: u32,
    frame_h: u32,
    real_w: u32,
    real_h: u32,
) -> (i32, i32) {
    let fw = frame_w.max(1) as f32;
    let fh = frame_h.max(1) as f32;
    let rx = (fx as f32 * real_w as f32 / fw).round() as i32;
    let ry = (fy as f32 * real_h as f32 / fh).round() as i32;
    (rx, ry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_scale_缩小_横屏超宽() {
        // 1920×1080 → 受限于宽 1280/1920 = 0.6667，高 720/1080 同为 0.6667
        let s = fit_scale(1920, 1080, MAX_W, MAX_H);
        assert!((s - 0.6667).abs() < 0.001, "scale={s}");
        assert_eq!(scaled_dims(1920, 1080, MAX_W, MAX_H), (1280, 720));
    }

    #[test]
    fn fit_scale_非16比9_取更小比例() {
        // 1600×1200（4:3）：宽比 1280/1600=0.8，高比 720/1200=0.6 → 取 0.6（高为瓶颈）
        let s = fit_scale(1600, 1200, MAX_W, MAX_H);
        assert!((s - 0.6).abs() < 0.001, "scale={s}");
        let (w, h) = scaled_dims(1600, 1200, MAX_W, MAX_H);
        assert_eq!((w, h), (960, 720)); // 高顶满 720，宽等比
    }

    #[test]
    fn fit_scale_小屏不放大() {
        // 800×600 已小于上限 → 比例钳到 1.0，不拉伸
        assert_eq!(fit_scale(800, 600, MAX_W, MAX_H), 1.0);
        assert_eq!(scaled_dims(800, 600, MAX_W, MAX_H), (800, 600));
    }

    #[test]
    fn fit_scale_空图返回1() {
        assert_eq!(fit_scale(0, 0, MAX_W, MAX_H), 1.0);
        assert_eq!(scaled_dims(0, 0, MAX_W, MAX_H), (1, 1));
    }

    #[test]
    fn 坐标换算_缩放后帧映射回真实屏() {
        // 真实屏 1920×1080，帧 1280×720（比例 1.5）。帧内 (640,360) → 真实 (960,540) 屏中心
        let (rx, ry) = map_frame_to_real(640, 360, 1280, 720, 1920, 1080);
        assert_eq!((rx, ry), (960, 540));
    }

    #[test]
    fn 坐标换算_非16比9_不偏() {
        // 真实 1600×1200，帧 960×720。帧角 (960,720) → 真实 (1600,1200) 右下角
        let (rx, ry) = map_frame_to_real(960, 720, 960, 720, 1600, 1200);
        assert_eq!((rx, ry), (1600, 1200));
        // 帧中心 (480,360) → 真实中心 (800,600)
        assert_eq!(map_frame_to_real(480, 360, 960, 720, 1600, 1200), (800, 600));
    }

    #[test]
    fn 坐标换算_防除零() {
        // frame 维度为 0 时按 1 处理，不 panic
        let (rx, ry) = map_frame_to_real(0, 0, 0, 0, 1920, 1080);
        assert_eq!((rx, ry), (0, 0));
    }
}
