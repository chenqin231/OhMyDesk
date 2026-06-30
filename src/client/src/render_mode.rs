//! 渲染/推帧运行模式（spec §3.6）。5 档 + 原子组，运行期热切复用 capture.rs QUALITY 范式。

use crate::capture::QualityParams;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// 5 档预设模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// frameskip off + telemetry off + 精确旧路径(frame_q)。最可靠回退。
    LegacyFullFrame,
    /// frameskip on + telemetry on（默认）。
    Frameskip,
    /// frameskip off + telemetry on：旧整帧 vs frame-skip 带宽对照。
    FullFrameWithTelemetry,
    /// frameskip on + 强制 smooth + 更低 fps（弱网兜底）。
    LowBandwidth,
    /// frameskip on + 强制日志/诊断 + 较高 keyframe 频率（取证）。
    Diagnostic,
}

impl RenderMode {
    pub fn as_u8(self) -> u8 {
        match self {
            RenderMode::LegacyFullFrame => 0,
            RenderMode::Frameskip => 1,
            RenderMode::FullFrameWithTelemetry => 2,
            RenderMode::LowBandwidth => 3,
            RenderMode::Diagnostic => 4,
        }
    }
    pub fn from_u8(v: u8) -> RenderMode {
        match v {
            0 => RenderMode::LegacyFullFrame,
            2 => RenderMode::FullFrameWithTelemetry,
            3 => RenderMode::LowBandwidth,
            4 => RenderMode::Diagnostic,
            _ => RenderMode::Frameskip,
        }
    }
    /// 模式 → (frameskip_on, telemetry_on)。
    pub fn switches(self) -> (bool, bool) {
        match self {
            RenderMode::LegacyFullFrame => (false, false),
            RenderMode::FullFrameWithTelemetry => (false, true),
            _ => (true, true),
        }
    }
}

/// 字符串 → 模式（配置/启动参数解析）。
pub fn parse_mode(s: &str) -> Option<RenderMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "legacy-full-frame" | "legacy" => Some(RenderMode::LegacyFullFrame),
        "frameskip" => Some(RenderMode::Frameskip),
        "full-frame-with-telemetry" => Some(RenderMode::FullFrameWithTelemetry),
        "low-bandwidth" => Some(RenderMode::LowBandwidth),
        "diagnostic" => Some(RenderMode::Diagnostic),
        _ => None,
    }
}

/// 低带宽档对 QualityParams 设地板（不写 QUALITY，与 SetQuality 正交）。
pub const LOW_BW_INTERVAL_MS: u64 = 150;
pub fn clamp_params(p: QualityParams, mode: RenderMode) -> QualityParams {
    if mode == RenderMode::LowBandwidth {
        QualityParams {
            max_w: p.max_w.min(1280),
            max_h: p.max_h.min(720),
            jpeg_q: p.jpeg_q.min(80),
            interval_ms: p.interval_ms.max(LOW_BW_INTERVAL_MS),
        }
    } else {
        p
    }
}

static RENDER_MODE: AtomicU8 = AtomicU8::new(1); // 默认 Frameskip
static FRAMESKIP_ON: AtomicBool = AtomicBool::new(true);
static TELEMETRY_ON: AtomicBool = AtomicBool::new(true);

/// 应用一个模式：原子写入 mode + 两开关（运行期热切的唯一入口）。
pub fn apply(mode: RenderMode) {
    let (fs, tele) = mode.switches();
    RENDER_MODE.store(mode.as_u8(), Ordering::Relaxed);
    FRAMESKIP_ON.store(fs, Ordering::Relaxed);
    TELEMETRY_ON.store(tele, Ordering::Relaxed);
}

pub fn current_mode() -> RenderMode {
    RenderMode::from_u8(RENDER_MODE.load(Ordering::Relaxed))
}
pub fn frameskip_on() -> bool {
    FRAMESKIP_ON.load(Ordering::Relaxed)
}
pub fn telemetry_on() -> bool {
    TELEMETRY_ON.load(Ordering::Relaxed)
}

/// 优先级解析：env mode > 启动参数 > 配置文件 > 默认 Frameskip。
/// 再用 OHMYDESK_FRAMESKIP/OHMYDESK_DIRTY_TELEMETRY 单独覆盖两开关（最高）。
pub fn resolve(env_mode: Option<&str>, arg_mode: Option<&str>, config_mode: Option<&str>) -> RenderMode {
    env_mode
        .and_then(parse_mode)
        .or_else(|| arg_mode.and_then(parse_mode))
        .or_else(|| config_mode.and_then(parse_mode))
        .unwrap_or(RenderMode::Frameskip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse往返与未知() {
        assert_eq!(parse_mode("legacy-full-frame"), Some(RenderMode::LegacyFullFrame));
        assert_eq!(parse_mode("Frameskip"), Some(RenderMode::Frameskip));
        assert_eq!(parse_mode("xxx"), None);
    }

    #[test]
    fn u8往返() {
        for m in [RenderMode::LegacyFullFrame, RenderMode::Frameskip, RenderMode::FullFrameWithTelemetry, RenderMode::LowBandwidth, RenderMode::Diagnostic] {
            assert_eq!(RenderMode::from_u8(m.as_u8()), m);
        }
    }

    #[test]
    fn 开关映射() {
        assert_eq!(RenderMode::LegacyFullFrame.switches(), (false, false));
        assert_eq!(RenderMode::FullFrameWithTelemetry.switches(), (false, true));
        assert_eq!(RenderMode::Frameskip.switches(), (true, true));
    }

    #[test]
    fn resolve优先级() {
        // env 最高
        assert_eq!(resolve(Some("legacy-full-frame"), Some("frameskip"), Some("diagnostic")), RenderMode::LegacyFullFrame);
        // env 缺→arg
        assert_eq!(resolve(None, Some("low-bandwidth"), Some("diagnostic")), RenderMode::LowBandwidth);
        // 都缺→config
        assert_eq!(resolve(None, None, Some("diagnostic")), RenderMode::Diagnostic);
        // 全缺→默认
        assert_eq!(resolve(None, None, None), RenderMode::Frameskip);
    }

    #[test]
    fn apply后原子可读() {
        apply(RenderMode::LegacyFullFrame);
        assert_eq!(current_mode(), RenderMode::LegacyFullFrame);
        assert!(!frameskip_on() && !telemetry_on());
        apply(RenderMode::Frameskip); // 复位避免污染其它测试
        assert!(frameskip_on() && telemetry_on());
    }

    #[test]
    fn low_bandwidth_clamp设地板() {
        let hq = QualityParams { max_w: 1920, max_h: 1080, jpeg_q: 88, interval_ms: 100 };
        let c = clamp_params(hq, RenderMode::LowBandwidth);
        assert_eq!((c.max_w, c.max_h, c.jpeg_q), (1280, 720, 80), "强制 smooth 上限");
        assert_eq!(c.interval_ms, LOW_BW_INTERVAL_MS, "间隔地板 150ms");
        // 非 low-bandwidth 不改
        let hq2 = QualityParams { max_w: 1920, max_h: 1080, jpeg_q: 88, interval_ms: 100 };
        let p = clamp_params(hq2, RenderMode::Frameskip);
        assert_eq!((p.max_w, p.max_h, p.jpeg_q, p.interval_ms), (1920, 1080, 88, 100));
    }
}
