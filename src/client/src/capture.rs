//! 屏幕捕获：xcap 截屏 → 等比缩放 → JPEG q85 → base64（P-CLI4）。
//!
//! 坑点（见 references/xcap-enigo.md）：
//! - `Monitor::all()` 启动枚举一次缓存复用，不每帧枚举。
//! - 实时流走 JPEG 非 PNG；等比缩放（长边≤1280/720，不放大），Frame 带真实缩放后 w/h。
//! - 锁 X11；无显示器/Wayland 时 `Capturer::new` 返回 Err，由 net 层降级跳过推帧。

use crate::geom::{scaled_dims, MAX_H, MAX_W};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use std::sync::atomic::{AtomicU8, Ordering};
use xcap::Monitor;

/// 当前画质档位（被控端推帧线程每帧读取）：0=流畅优先(默认)，1=高清优先。
/// 主控端经 SetQuality 协议消息切换，被控端 dispatch 调 [`set_quality`] 更新。
static QUALITY: AtomicU8 = AtomicU8::new(0);

/// 画质档位对应的采集参数：分辨率上限 + JPEG 质量 + 推帧间隔(ms)。
pub struct QualityParams {
    pub max_w: u32,
    pub max_h: u32,
    pub jpeg_q: u8,
    pub interval_ms: u64,
}

/// 设置画质档位（被控端收到主控 SetQuality 时调用）。
pub fn set_quality(mode: protocol::QualityMode) {
    let v = match mode {
        protocol::QualityMode::Smooth => 0,
        protocol::QualityMode::HighQuality => 1,
    };
    QUALITY.store(v, Ordering::Relaxed);
}

/// 当前画质档位原子值（0=流畅,1=高清），供推帧线程做 quality_changed 判断。
pub fn quality_u8() -> u8 {
    QUALITY.load(Ordering::Relaxed)
}

/// 档位 → 采集参数（纯函数，便于单测）。
/// 流畅优先：1280×720 / q80 / ~16fps；高清优先：1920×1080 / q88 / ~10fps。
pub fn params_for(mode: protocol::QualityMode) -> QualityParams {
    match mode {
        protocol::QualityMode::HighQuality => QualityParams {
            max_w: 1920,
            max_h: 1080,
            jpeg_q: 88,
            interval_ms: 100,
        },
        protocol::QualityMode::Smooth => QualityParams {
            max_w: 1280,
            max_h: 720,
            jpeg_q: 80,
            interval_ms: 40,
        },
    }
}

/// 取当前档位的采集参数（推帧线程每帧调用）。
pub fn current_params() -> QualityParams {
    let mode = if QUALITY.load(Ordering::Relaxed) == 1 {
        protocol::QualityMode::HighQuality
    } else {
        protocol::QualityMode::Smooth
    };
    params_for(mode)
}

/// 当前档位下、被控真实屏 `real_w×real_h` 对应的**推帧分辨率**。
///
/// 与采集线程 `frame_q` 内部 `scaled_dims(屏, 档位上限)` 同源同算法，是「截多大」的单一事实源；
/// 注入侧据此把主控帧内坐标还原为真实屏坐标，保证切换高清/流畅后点击不偏（见 P-CLI4）。
pub fn current_frame_dims(real_w: u32, real_h: u32) -> (u32, u32) {
    let p = current_params();
    crate::geom::scaled_dims(real_w, real_h, p.max_w, p.max_h)
}

/// 持有主显示器句柄，复用于每帧截屏。
pub struct Capturer {
    mon: Monitor,
    /// 被控真实屏尺寸（注入坐标换算的 real_w/real_h）。
    real_w: u32,
    real_h: u32,
}

impl Capturer {
    /// 枚举显示器取主屏（或第一块）。无显示器/X11 不可用时返回 Err。
    ///
    /// `Monitor::all()` 在部分环境（如 WSL2 走 wayland 后端）会**panic 而非返回 Err**
    /// （libwayshot-xcap UnsupportedVersion），故用 `catch_unwind` 兜住转成 Err，
    /// 保证调用线程不被炸（降级原则，见 references）。
    pub fn new() -> anyhow::Result<Self> {
        let monitors = std::panic::catch_unwind(Monitor::all)
            .map_err(|_| anyhow::anyhow!("xcap 枚举显示器 panic（环境不支持，已降级）"))??;
        // 优先主屏，回退第一块
        let mon = monitors
            .iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .cloned()
            .or_else(|| monitors.into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("未发现可用显示器"))?;
        let real_w = mon.width()?;
        let real_h = mon.height()?;
        Ok(Capturer {
            mon,
            real_w,
            real_h,
        })
    }

    /// 被控真实屏尺寸（供注入器构造 real_w/real_h）。
    pub fn real_size(&self) -> (u32, u32) {
        (self.real_w, self.real_h)
    }

    /// 截一帧 → 等比缩放 → JPEG q85 → base64（默认档，截图等用）。返回 (base64, 缩放后 w, 缩放后 h)。
    pub fn frame(&self) -> anyhow::Result<(String, u32, u32)> {
        let img = self.mon.capture_image()?; // RgbaImage
        encode_frame(&img)
    }

    /// 按画质档位截一帧（推流用）：分辨率上限/质量由 `QualityParams` 决定。
    pub fn frame_q(&self, p: &QualityParams) -> anyhow::Result<(String, u32, u32)> {
        let img = self.mon.capture_image()?;
        encode_frame_q(&img, p.max_w, p.max_h, p.jpeg_q)
    }

    /// 截一帧原始 RGBA（不缩放不编码），供变化检测先行（spec §3.1）。
    pub fn capture_raw(&self) -> anyhow::Result<image::RgbaImage> {
        Ok(self.mon.capture_image()?)
    }
}

/// 把一帧 RGBA 等比缩放 + JPEG q85 + base64（默认 1280×720 上限，截图/默认路径用）。
pub fn encode_frame(img: &RgbaImage) -> anyhow::Result<(String, u32, u32)> {
    encode_frame_q(img, MAX_W, MAX_H, 85)
}

/// 把一帧 RGBA 按指定分辨率上限等比缩放 + JPEG(质量 q) + base64（纯函数，便于单测）。
pub fn encode_frame_q(
    img: &RgbaImage,
    max_w: u32,
    max_h: u32,
    q: u8,
) -> anyhow::Result<(String, u32, u32)> {
    let (sw, sh) = (img.width(), img.height());
    let (w, h) = scaled_dims(sw, sh, max_w, max_h);

    // 缩放（等比，不放大）。尺寸未变时跳过 resize 省一次拷贝。
    let rgb = if (w, h) == (sw, sh) {
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()
    } else {
        let resized = image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
        image::DynamicImage::ImageRgba8(resized).to_rgb8()
    };

    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&rgb)?;
    Ok((STANDARD.encode(buf.get_ref()), w, h))
}

/// 降级占位帧（仅 `OHMYDESK_FAKE_CAPTURE=1` 时启用）：真实截屏不可用的环境（如 WSLg 的
/// X GetImage 限制）下，生成**可辨识的合成测试图案**（随 seq 移动的亮竖条 + 渐变 + 网格），
/// 用于在开发机验证「授权 → 画面 → 操作 → 断开」整条链路。**非真实屏幕**，真机 X11 默认不启用。
pub fn placeholder_frame(seq: u64) -> anyhow::Result<(String, u32, u32)> {
    let (w, h) = (1280u32, 720u32);
    let bar = (seq.wrapping_mul(16) % w as u64) as u32; // 竖条位置随 seq 移动，证明帧在刷新
    let img = RgbaImage::from_fn(w, h, |x, y| {
        let base_r = (x * 90 / w) as u8;
        let base_b = (y * 160 / h) as u8 + 60;
        let grid = if x % 64 == 0 || y % 64 == 0 { 40 } else { 0 };
        let moving = if x.abs_diff(bar) < 6 { 200 } else { 0 };
        image::Rgba([
            base_r.saturating_add(moving).saturating_add(grid),
            40u8.saturating_add(moving).saturating_add(grid),
            base_b.saturating_add(grid),
            255,
        ])
    });
    encode_frame(&img)
}

/// 本机是否为 Wayland 会话（由 `lock_x11_session` 在抹掉 WAYLAND_DISPLAY 前打的标记决定）。
/// Wayland 下 xcap 抓不到桌面（Xwayland 隔离），截屏线程据此直接回执主控端而非空等。
pub fn is_wayland_session() -> bool {
    std::env::var("OHMYDESK_WAYLAND").map(|v| v == "1").unwrap_or(false)
}

/// 是否启用降级占位帧（环境变量开关，默认关）。
pub fn fake_capture_enabled() -> bool {
    std::env::var("OHMYDESK_FAKE_CAPTURE")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 合成纯色 RgbaImage，避免依赖真实显示器测编码/缩放。
    fn solid(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([10, 120, 200, 255]))
    }

    #[test]
    fn encode_大屏_缩放到上限() {
        let (b64, w, h) = encode_frame(&solid(1920, 1080)).unwrap();
        assert_eq!((w, h), (1280, 720), "1920×1080 等比缩到 1280×720");
        assert!(!b64.is_empty());
        // base64 可解码回字节
        let raw = STANDARD.decode(&b64).unwrap();
        // JPEG 魔数 FF D8
        assert_eq!(&raw[..2], &[0xFF, 0xD8], "应为 JPEG 字节流");
    }

    #[test]
    fn encode_小屏_不放大() {
        let (_b64, w, h) = encode_frame(&solid(800, 600)).unwrap();
        assert_eq!((w, h), (800, 600), "小于上限不放大");
    }

    #[test]
    fn encode_非16比9_等比() {
        let (_b64, w, h) = encode_frame(&solid(1600, 1200)).unwrap();
        assert_eq!((w, h), (960, 720), "4:3 高顶满 720 宽等比");
    }

    /// 真实截屏冒烟（依赖 X11 显示器）：默认 ignore，手动 `cargo test -p client -- --ignored` 跑。
    /// 注：WSLg 的 X server 不完整支持 X GetImage（全屏抓图报 xcb Match error），抓屏一步会失败；
    /// 真实信创 X11 物理机无此限制。本测试只硬断言「能枚举显示器 + 拿到真实屏尺寸」，抓屏失败仅打印。
    #[test]
    #[ignore]
    fn 真实截屏冒烟() {
        std::env::remove_var("WAYLAND_DISPLAY"); // 锁 X11
        let cap = Capturer::new().expect("应能枚举到显示器");
        let (rw, rh) = cap.real_size();
        assert!(rw > 0 && rh > 0, "真实屏尺寸应非零：{rw}x{rh}");
        match cap.frame() {
            Ok((b64, w, h)) => {
                assert!(!b64.is_empty());
                assert!(w <= MAX_W && h <= MAX_H, "缩放后不超上限：{w}x{h}");
                println!(
                    "真实截屏：屏 {rw}x{rh} → 帧 {w}x{h}，base64 {} 字节",
                    b64.len()
                );
            }
            Err(e) => {
                println!("枚举屏 {rw}x{rh} OK；抓屏失败（WSLg GetImage 限制，真机无此问题）：{e}")
            }
        }
    }

    #[test]
    fn 占位帧_可解码为_1280x720_rgba() {
        // 主控贴帧前会 JPEG→RgbaImage 解码（ui_glue::decode_frame_rgba）；占位帧须走通同款路径。
        let (b64, w, h) = placeholder_frame(7).unwrap();
        assert_eq!((w, h), (1280, 720));
        let raw = STANDARD.decode(&b64).unwrap();
        assert_eq!(&raw[..2], &[0xFF, 0xD8], "应为 JPEG");
        let img = image::load_from_memory(&raw).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (1280, 720), "解码后尺寸应一致");
    }

    #[test]
    fn 画质档位_参数符合预期() {
        let hq = params_for(protocol::QualityMode::HighQuality);
        assert_eq!((hq.max_w, hq.max_h, hq.jpeg_q), (1920, 1080, 88));
        assert!(hq.interval_ms >= 80, "高清档帧率不应过高(信创CPU)：{}ms", hq.interval_ms);
        let sm = params_for(protocol::QualityMode::Smooth);
        assert_eq!((sm.max_w, sm.max_h, sm.jpeg_q), (1280, 720, 80));
        assert!(sm.interval_ms < hq.interval_ms, "流畅档帧率应高于高清档");
    }

    #[test]
    fn 注入帧尺寸_随档位变化_与采集编码一致() {
        // 被控真实屏 1920×1080。注入侧 current_frame_dims 必须与采集 encode_frame_q 的输出尺寸一致，
        // 且随档位变化——否则切高清后注入用陈旧尺寸还原坐标，点击错位（回归 Bug）。
        let img = solid(1920, 1080);

        set_quality(protocol::QualityMode::Smooth);
        let sp = params_for(protocol::QualityMode::Smooth);
        let (_, sw, sh) = encode_frame_q(&img, sp.max_w, sp.max_h, sp.jpeg_q).unwrap();
        assert_eq!(current_frame_dims(1920, 1080), (sw, sh), "流畅档帧尺寸应一致");
        assert_eq!((sw, sh), (1280, 720));

        set_quality(protocol::QualityMode::HighQuality);
        let hp = params_for(protocol::QualityMode::HighQuality);
        let (_, hw, hh) = encode_frame_q(&img, hp.max_w, hp.max_h, hp.jpeg_q).unwrap();
        assert_eq!(current_frame_dims(1920, 1080), (hw, hh), "高清档帧尺寸应一致");
        assert_eq!((hw, hh), (1920, 1080));
        // 关键回归：高清档下注入帧尺寸绝不能再是旧静态 1280×720（那会导致 1.5× 偏移）。
        assert_ne!(current_frame_dims(1920, 1080), (1280, 720));

        set_quality(protocol::QualityMode::Smooth); // 复位，避免污染其它测试
    }

    #[test]
    fn encode_质量档位_高清更大() {
        // 同图：高清(更大分辨率上限+更高q)的字节数应 ≥ 流畅
        let img = solid(1920, 1080);
        let (hq, hw, _) = encode_frame_q(&img, 1920, 1080, 88).unwrap();
        let (sm, sw, _) = encode_frame_q(&img, 1280, 720, 80).unwrap();
        assert_eq!((hw, sw), (1920, 1280), "分辨率上限生效");
        assert!(hq.len() >= sm.len(), "高清帧字节应不小于流畅帧");
    }

    #[test]
    fn encode_帧体积可控() {
        // q85 纯色帧 base64 后应在合理量级（远小于裸 RGBA 1280*720*4 ≈ 3.5MB）
        let (b64, _, _) = encode_frame(&solid(1920, 1080)).unwrap();
        assert!(
            b64.len() < 1_000_000,
            "单帧 base64 应远小于 1MB，实际 {}",
            b64.len()
        );
    }
}
