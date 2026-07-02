//! 屏幕捕获：xcap 截屏 → 等比缩放 → JPEG q85 → base64（P-CLI4）。
//!
//! 坑点（见 references/xcap-enigo.md）：
//! - `Monitor::all()` 启动枚举一次缓存复用，不每帧枚举。
//! - 实时流走 JPEG 非 PNG；等比缩放（长边≤1280/720，不放大），Frame 带真实缩放后 w/h。
//! - 锁 X11；无显示器/Wayland 时 `Capturer::new` 返回 Err，由 net 层降级跳过推帧。

use crate::geom::{scaled_dims, MAX_H, MAX_W};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::RgbaImage;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use xcap::Monitor;

/// 当前画质档位（被控端推帧线程每帧读取）：0=流畅优先(默认)，1=高清优先。
/// 主控端经 SetQuality 协议消息切换，被控端 dispatch 调 [`set_quality`] 更新。
static QUALITY: AtomicU8 = AtomicU8::new(0);

/// 画质档位对应的采集参数：分辨率上限 + JPEG 质量 + 推帧间隔(ms)。
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// 被控端「最近实际发出的一帧」的缩放后尺寸——即 render_mode/adaptive 动态钳制后的**真实**结果。
/// 注入侧据此还原坐标（「发了多大就按多大还原」），避免 `current_frame_dims`（仅档位标称、
/// 不含 adaptive 降档）与实际发出尺寸不一致导致点击错位。0 = 尚无帧发出。
static LAST_FRAME_W: AtomicU32 = AtomicU32::new(0);
static LAST_FRAME_H: AtomicU32 = AtomicU32::new(0);

/// 推帧线程每发出一帧即记录其真实缩放后尺寸（含 clamp/adaptive 结果）。
pub fn set_last_frame_dims(w: u32, h: u32) {
    LAST_FRAME_W.store(w, Ordering::Relaxed);
    LAST_FRAME_H.store(h, Ordering::Relaxed);
}

/// 最近实际发出帧的尺寸；尚无帧发出（任一维为 0）时返回 None，调用方回退到 `current_frame_dims`。
pub fn last_frame_dims() -> Option<(u32, u32)> {
    let w = LAST_FRAME_W.load(Ordering::Relaxed);
    let h = LAST_FRAME_H.load(Ordering::Relaxed);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
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
        let o = encode_frame_q(&img, p.max_w, p.max_h, p.jpeg_q)?;
        Ok((o.data, o.w, o.h))
    }

    /// 截一帧原始 RGBA（不缩放不编码），供变化检测先行（spec §3.1）。
    pub fn capture_raw(&self) -> anyhow::Result<image::RgbaImage> {
        Ok(self.mon.capture_image()?)
    }
}

/// 把一帧 RGBA 等比缩放 + JPEG q85 + base64（默认 1280×720 上限，截图/默认路径用）。
pub fn encode_frame(img: &RgbaImage) -> anyhow::Result<(String, u32, u32)> {
    let o = encode_frame_q(img, MAX_W, MAX_H, 85)?;
    Ok((o.data, o.w, o.h))
}

/// 编码产出 + 分段耗时（resize 与 jpeg 分开计时，用于定位「编码过载」主导项）。
pub struct EncodeOut {
    pub data: String,
    pub w: u32,
    pub h: u32,
    pub resize_ms: u32,
    pub jpeg_ms: u32,
}

/// 把一帧 RGBA 按指定分辨率上限等比缩放 + JPEG(质量 q) + base64（纯函数，便于单测）。
pub fn encode_frame_q(
    img: &RgbaImage,
    max_w: u32,
    max_h: u32,
    q: u8,
) -> anyhow::Result<EncodeOut> {
    let (sw, sh) = (img.width(), img.height());
    let (w, h) = scaled_dims(sw, sh, max_w, max_h);

    // 缩放（等比，不放大）。尺寸未变时跳过 resize 省一次拷贝。
    // 滤波用 Triangle(双线性) 而非 Lanczos3：真机实测 Lanczos3 降采是编码耗时大头(老 Xeon 上
    // ~200ms/帧，占 ~79%)，且其成本由「读整张输入」决定、降分辨率几乎不省；Triangle 快数倍、
    // 远控 720p 画质损失可接受，直接砍延迟。详见 specs/2026-07-01-resize-pipeline-perf-design.md。
    // 不再单独 to_rgb8()——jpeg-encoder 直接吃 RGBA，内部做 RGB 转换。
    let t_resize = std::time::Instant::now();
    let resized: Option<RgbaImage> = if (w, h) == (sw, sh) {
        None
    } else {
        Some(image::imageops::resize(
            img,
            w,
            h,
            image::imageops::FilterType::Triangle,
        ))
    };
    let rgba: &[u8] = match &resized {
        Some(r) => r.as_raw(),
        None => img.as_raw(),
    };
    let resize_ms = t_resize.elapsed().as_millis() as u32;

    // JPEG：jpeg-encoder 纯 Rust 标量（Ivy Bridge 实测 4.19× 于 image 标量，
    // 见 specs/2026-07-02-simd-jpeg-encode-design.md）。直接消费 RGBA：JPEG 不含 alpha，
    // jpeg-encoder 丢弃第 4 通道。默认采样 q<90 → 4:2:0，与主控 image 解码兼容。
    let t_jpeg = std::time::Instant::now();
    let mut buf: Vec<u8> = Vec::new();
    // 屏幕分辨率远小于 u16::MAX；显式化该假设，未来异形大屏若越界能在 debug 下即时暴露。
    debug_assert!(w <= u16::MAX as u32 && h <= u16::MAX as u32, "分辨率超 u16 上限");
    jpeg_encoder::Encoder::new(&mut buf, q)
        .encode(rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .map_err(|e| anyhow::anyhow!("jpeg 编码失败: {e}"))?;
    let data = STANDARD.encode(&buf);
    let jpeg_ms = t_jpeg.elapsed().as_millis() as u32;

    Ok(EncodeOut { data, w, h, resize_ms, jpeg_ms })
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
    fn last_frame_dims_记录实际发出尺寸与回退() {
        // 复位：无帧发出 → None（调用方回退 current_frame_dims）
        set_last_frame_dims(0, 0);
        assert_eq!(last_frame_dims(), None, "尚无帧发出返回 None");
        // 记录后读回**实际发出**尺寸（如 adaptive 降档后的 896×504，与档位标称无关）
        set_last_frame_dims(896, 504);
        assert_eq!(last_frame_dims(), Some((896, 504)), "返回最近实际发出尺寸");
        // 任一维为 0 视为未设
        set_last_frame_dims(800, 0);
        assert_eq!(last_frame_dims(), None, "任一维为 0 → None");
        set_last_frame_dims(0, 0); // 复位避免污染其它测试
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
        let o = encode_frame_q(&img, sp.max_w, sp.max_h, sp.jpeg_q).unwrap();
        let (sw, sh) = (o.w, o.h);
        assert_eq!(current_frame_dims(1920, 1080), (sw, sh), "流畅档帧尺寸应一致");
        assert_eq!((sw, sh), (1280, 720));

        set_quality(protocol::QualityMode::HighQuality);
        let hp = params_for(protocol::QualityMode::HighQuality);
        let o = encode_frame_q(&img, hp.max_w, hp.max_h, hp.jpeg_q).unwrap();
        let (hw, hh) = (o.w, o.h);
        assert_eq!(current_frame_dims(1920, 1080), (hw, hh), "高清档帧尺寸应一致");
        assert_eq!((hw, hh), (1920, 1080));
        // 关键回归：高清档下注入帧尺寸绝不能再是旧静态 1280×720（那会导致 1.5× 偏移）。
        assert_ne!(current_frame_dims(1920, 1080), (1280, 720));

        set_quality(protocol::QualityMode::Smooth); // 复位，避免污染其它测试
    }

    #[test]
    fn encode_frame_q_返回分段耗时字段() {
        let img = image::RgbaImage::from_pixel(200, 120, image::Rgba([10, 20, 30, 255]));
        // max 100x60 < 源 → 触发缩放路径
        let o = encode_frame_q(&img, 100, 60, 80).unwrap();
        assert_eq!((o.w, o.h), (100, 60), "等比缩放到上限");
        assert!(!o.data.is_empty(), "有 base64 输出");
        // 字段可读、类型为 u32（不断言具体 ms，避免机器差异 flaky）
        let _total: u32 = o.resize_ms + o.jpeg_ms;
    }

    #[test]
    fn encode_质量档位_高清更大() {
        // 同图：高清(更大分辨率上限+更高q)的字节数应 ≥ 流畅
        let img = solid(1920, 1080);
        let hq_o = encode_frame_q(&img, 1920, 1080, 88).unwrap();
        let sm_o = encode_frame_q(&img, 1280, 720, 80).unwrap();
        assert_eq!((hq_o.w, sm_o.w), (1920, 1280), "分辨率上限生效");
        assert!(hq_o.data.len() >= sm_o.data.len(), "高清帧字节应不小于流畅帧");
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

    #[test]
    fn encode_frame_q_产出可解回原尺寸_高清q88() {
        let img = solid(1280, 720);
        let out = encode_frame_q(&img, 1920, 1080, 88).unwrap();
        assert_eq!((out.w, out.h), (1280, 720), "不放大，尺寸原样");
        let bytes = STANDARD
            .decode(out.data.as_bytes())
            .expect("产出应为合法 base64");
        let decoded = image::load_from_memory(&bytes).expect("产出应为可解码 JPEG");
        assert_eq!((decoded.width(), decoded.height()), (1280, 720), "解回同尺寸");
    }

    #[test]
    fn encode_frame_q_大屏缩放后可解回上限尺寸() {
        let img = solid(3840, 2160);
        let out = encode_frame_q(&img, 1920, 1080, 88).unwrap();
        assert_eq!((out.w, out.h), (1920, 1080), "4K 等比缩到 1080p 上限");
        let bytes = STANDARD
            .decode(out.data.as_bytes())
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1920, 1080), "解回同尺寸");
    }

    #[test]
    #[ignore] // 手动：cargo test -p client --release encode_bench_1280x720 -- --ignored --nocapture
    fn encode_bench_1280x720() {
        let img = image::RgbaImage::from_fn(1280, 720, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let n = 30;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = encode_frame_q(&img, 1280, 720, 80).unwrap();
        }
        let per = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("encode_frame_q 1280x720 q80: {per:.1} ms/帧 (n={n})");
    }

    #[test]
    #[ignore] // 手动：cargo test -p client --release encode_bench_split -- --ignored --nocapture
    fn encode_bench_split() {
        // 分段计时：RGBA→RGB 转换 vs JPEG 编码，定位热点
        let img = image::RgbaImage::from_fn(1280, 720, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let n = 30u64;
        let mut tr = 0u64;
        let mut tj = 0u64;
        for _ in 0..n {
            let o = encode_frame_q(&img, 1280, 720, 80).unwrap();
            tr += o.resize_ms as u64;
            tj += o.jpeg_ms as u64;
        }
        println!(
            "resize: {:.1} ms/帧  jpeg: {:.1} ms/帧  total: {:.1} ms/帧 (n={n})",
            tr as f64 / n as f64,
            tj as f64 / n as f64,
            (tr + tj) as f64 / n as f64,
        );
    }
}
