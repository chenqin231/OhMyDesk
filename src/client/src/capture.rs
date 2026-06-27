//! 屏幕捕获：xcap 截屏 → 等比缩放 → JPEG q60 → base64（P-CLI4）。
//!
//! 坑点（见 references/xcap-enigo.md）：
//! - `Monitor::all()` 启动枚举一次缓存复用，不每帧枚举。
//! - 实时流走 JPEG 非 PNG；等比缩放（长边≤1280/720，不放大），Frame 带真实缩放后 w/h。
//! - 锁 X11；无显示器/Wayland 时 `Capturer::new` 返回 Err，由 net 层降级跳过推帧。

use crate::geom::{scaled_dims, MAX_H, MAX_W};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use xcap::Monitor;

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

    /// 截一帧 → 等比缩放 → JPEG q60 → base64。返回 (base64, 缩放后 w, 缩放后 h)。
    pub fn frame(&self) -> anyhow::Result<(String, u32, u32)> {
        let img = self.mon.capture_image()?; // RgbaImage
        encode_frame(&img)
    }
}

/// 把一帧 RGBA 等比缩放 + JPEG + base64（纯函数，便于单测缩放/编码逻辑，无需真实屏）。
pub fn encode_frame(img: &RgbaImage) -> anyhow::Result<(String, u32, u32)> {
    let (sw, sh) = (img.width(), img.height());
    let (w, h) = scaled_dims(sw, sh, MAX_W, MAX_H);

    // 缩放（等比，不放大）。尺寸未变时跳过 resize 省一次拷贝。
    let rgb = if (w, h) == (sw, sh) {
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()
    } else {
        let resized = image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle);
        image::DynamicImage::ImageRgba8(resized).to_rgb8()
    };

    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60).encode_image(&rgb)?;
    Ok((STANDARD.encode(buf.get_ref()), w, h))
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
                println!("真实截屏：屏 {rw}x{rh} → 帧 {w}x{h}，base64 {} 字节", b64.len());
            }
            Err(e) => println!("枚举屏 {rw}x{rh} OK；抓屏失败（WSLg GetImage 限制，真机无此问题）：{e}"),
        }
    }

    #[test]
    fn encode_帧体积可控() {
        // q60 纯色帧 base64 后应在合理量级（远小于裸 RGBA 1280*720*4 ≈ 3.5MB）
        let (b64, _, _) = encode_frame(&solid(1920, 1080)).unwrap();
        assert!(b64.len() < 1_000_000, "单帧 base64 应远小于 1MB，实际 {}", b64.len());
    }
}
