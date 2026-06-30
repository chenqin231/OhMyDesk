//! 被控端遥测：两数据源（FrameSample/EgressSample）按 seq 合并的 collector，
//! 窗口聚合 + 异常分类 + 环形缓冲 + 触发式 dump。纯逻辑优先单测。

/// 来源 A：推帧线程产出（采集/跳过/编码段，无出网字段）。
#[derive(Debug, Clone)]
pub struct FrameSample {
    pub ts_ms: u64,
    pub seq: u64,           // 发送帧 seq；跳过 tick 记 last_sent_seq 且 skipped=true
    pub capture_ms: u32,
    pub skipped: bool,
    pub dirty_ratio: f32,
    pub keyframe_forced: bool,
    pub encode_ms: u32,
    pub encoded_bytes: usize,
    pub w: u32,
    pub h: u32,
}

/// 来源 B：conn.rs 出站泵产出（仅本机出网段）。
#[derive(Debug, Clone)]
pub struct EgressSample {
    pub seq: u64,
    pub send_stall_ms: u32,
    pub sent_ok: bool,
    pub ws_error: bool,
}

/// 窗口聚合结果（10s 滑窗）。
#[derive(Debug, Default, PartialEq)]
pub struct WindowStats {
    pub frames: usize,        // 经决策的 tick 数（含跳过）
    pub sent: usize,          // 实发帧数（非跳过）
    pub egress_writes: usize, // conn.rs 实际写出帧数（EgressSample 数）
    pub effective_fps: f32,
    pub skip_pct: f32,
    pub dirty_p50: f32,
    pub dirty_p95: f32,
    pub enc_bps: u64,         // Σencoded_bytes / 窗秒
    pub bytes_avg: usize,
    pub bytes_p95: usize,
    pub cap_p95_ms: u32,
    pub enc_avg_ms: u32,
    pub enc_p95_ms: u32,
    pub stall_p95_ms: u32,
    pub egress_drop: usize,   // sent(非跳过) − egress_writes，clamp≥0
}

/// 异常类型（spec §4.5）。
#[derive(Debug, PartialEq, Eq)]
pub enum Anomaly {
    Egress阻塞,
    投递饥饿,
    采集异常,
    编码过载,
    FrameSkip失效,
}

/// 取已排序切片的 p 分位（p∈[0,1]），空切片返回 0。
fn percentile_u32(sorted: &[u32], p: f32) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f32 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn percentile_f32(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f32 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn percentile_usize(sorted: &[usize], p: f32) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f32 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 对一窗的合并样本聚合（window_ms = 窗口时长，用于算速率/fps）。
pub fn aggregate(
    frames: &[FrameSample],
    egress: &[EgressSample],
    window_ms: u64,
) -> WindowStats {
    let total = frames.len();
    let sent_frames: Vec<&FrameSample> = frames.iter().filter(|f| !f.skipped).collect();
    let sent = sent_frames.len();
    let skipped = total - sent;
    let window_s = (window_ms as f32 / 1000.0).max(0.001);

    // dirty 分位（全 tick，跳过的 dirty=0）
    let mut dirty: Vec<f32> = frames.iter().map(|f| f.dirty_ratio).collect();
    dirty.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // 编码耗时/字节（仅发送帧）
    let mut enc_ms: Vec<u32> = sent_frames.iter().map(|f| f.encode_ms).collect();
    enc_ms.sort_unstable();
    let mut bytes: Vec<usize> = sent_frames.iter().map(|f| f.encoded_bytes).collect();
    bytes.sort_unstable();
    let mut cap_ms: Vec<u32> = frames.iter().map(|f| f.capture_ms).collect();
    cap_ms.sort_unstable();
    let mut stall: Vec<u32> = egress.iter().map(|e| e.send_stall_ms).collect();
    stall.sort_unstable();

    let total_bytes: usize = bytes.iter().sum();
    let bytes_avg = if sent > 0 { total_bytes / sent } else { 0 };
    let enc_sum: u32 = enc_ms.iter().sum();
    let enc_avg_ms = if sent > 0 { enc_sum / sent as u32 } else { 0 };
    let egress_writes = egress.len();
    let egress_drop = sent.saturating_sub(egress_writes);

    WindowStats {
        frames: total,
        sent,
        egress_writes,
        effective_fps: sent as f32 / window_s,
        skip_pct: if total > 0 { skipped as f32 / total as f32 } else { 0.0 },
        dirty_p50: percentile_f32(&dirty, 0.5),
        dirty_p95: percentile_f32(&dirty, 0.95),
        enc_bps: (total_bytes as f32 / window_s) as u64,
        bytes_avg,
        bytes_p95: percentile_usize(&bytes, 0.95),
        cap_p95_ms: percentile_u32(&cap_ms, 0.95),
        enc_avg_ms,
        enc_p95_ms: percentile_u32(&enc_ms, 0.95),
        stall_p95_ms: percentile_u32(&stall, 0.95),
        egress_drop,
    }
}

/// 按阈值分类异常（纯函数）。
pub fn classify(s: &WindowStats) -> Vec<Anomaly> {
    let mut out = vec![];
    // 出网阻塞：stall 高 或 单窗 egress_drop 占已发比 > 50%
    let drop_ratio = if s.sent > 0 { s.egress_drop as f32 / s.sent as f32 } else { 0.0 };
    if s.stall_p95_ms > 1000 || drop_ratio > 0.5 {
        out.push(Anomaly::Egress阻塞);
    }
    // 投递饥饿：在产帧（dirty 高）却发不出去（fps 极低）
    if s.effective_fps < 1.0 && s.dirty_p95 > 0.1 {
        out.push(Anomaly::投递饥饿);
    }
    // 采集异常：采集 p95 > 200ms
    if s.cap_p95_ms > 200 {
        out.push(Anomaly::采集异常);
    }
    // 编码过载
    if s.enc_p95_ms > 200 {
        out.push(Anomaly::编码过载);
    }
    // frame-skip 失效：几乎不跳但画面也几乎不动（疑 bug）
    if s.skip_pct < 0.2 && s.dirty_p95 < 0.05 {
        out.push(Anomaly::FrameSkip失效);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(seq: u64, skipped: bool, dirty: f32, enc_ms: u32, bytes: usize, cap_ms: u32) -> FrameSample {
        FrameSample {
            ts_ms: seq * 50,
            seq,
            capture_ms: cap_ms,
            skipped,
            dirty_ratio: dirty,
            keyframe_forced: false,
            encode_ms: enc_ms,
            encoded_bytes: bytes,
            w: 1280,
            h: 720,
        }
    }

    #[test]
    fn 聚合_skip占比与fps与字节率() {
        // 10 帧：8 跳过 + 2 发送（各 60KB，编码 30ms），窗 10s
        let mut frames = vec![];
        for _i in 0..8 {
            frames.push(fs(0, true, 0.0, 0, 0, 10));
        }
        frames.push(fs(1, false, 0.2, 30, 60_000, 12));
        frames.push(fs(2, false, 0.1, 30, 60_000, 12));
        let egress = vec![
            EgressSample { seq: 1, send_stall_ms: 100, sent_ok: true, ws_error: false },
            EgressSample { seq: 2, send_stall_ms: 180, sent_ok: true, ws_error: false },
        ];
        let s = aggregate(&frames, &egress, 10_000);
        assert_eq!(s.frames, 10);
        assert_eq!(s.sent, 2);
        assert_eq!(s.egress_writes, 2);
        assert_eq!(s.egress_drop, 0, "sent==egress_writes → 无丢帧");
        assert!((s.skip_pct - 0.8).abs() < 1e-6);
        assert!((s.effective_fps - 0.2).abs() < 1e-6, "2 帧/10s=0.2fps");
        assert_eq!(s.enc_bps, 12_000, "120000 字节/10s");
        assert_eq!(s.bytes_avg, 60_000);
        assert_eq!(s.stall_p95_ms, 180);
    }

    #[test]
    fn 聚合_egress丢帧() {
        // 发送 3 帧但 conn 只写出 1（watch 覆盖）→ drop=2
        let frames = vec![fs(1, false, 0.3, 30, 50_000, 12), fs(2, false, 0.3, 30, 50_000, 12), fs(3, false, 0.3, 30, 50_000, 12)];
        let egress = vec![EgressSample { seq: 3, send_stall_ms: 1200, sent_ok: true, ws_error: false }];
        let s = aggregate(&frames, &egress, 10_000);
        assert_eq!(s.sent, 3);
        assert_eq!(s.egress_writes, 1);
        assert_eq!(s.egress_drop, 2, "Δenqueued−Δsent=3−1=2");
    }

    #[test]
    fn 分类_出网阻塞() {
        let s = WindowStats { sent: 10, egress_writes: 3, egress_drop: 7, stall_p95_ms: 1200, ..Default::default() };
        assert!(classify(&s).contains(&Anomaly::Egress阻塞));
    }

    #[test]
    fn 分类_投递饥饿() {
        let s = WindowStats { effective_fps: 0.5, dirty_p95: 0.3, ..Default::default() };
        assert!(classify(&s).contains(&Anomaly::投递饥饿));
    }

    #[test]
    fn 分类_编码过载() {
        let s = WindowStats { enc_p95_ms: 250, ..Default::default() };
        assert!(classify(&s).contains(&Anomaly::编码过载));
    }

    #[test]
    fn 分类_frameskip失效() {
        let s = WindowStats { skip_pct: 0.1, dirty_p95: 0.02, ..Default::default() };
        assert!(classify(&s).contains(&Anomaly::FrameSkip失效));
    }

    #[test]
    fn 分类_正常窗不误报() {
        let s = WindowStats { frames: 100, sent: 20, egress_writes: 20, effective_fps: 2.0, skip_pct: 0.8, dirty_p95: 0.2, enc_p95_ms: 60, stall_p95_ms: 150, ..Default::default() };
        assert!(classify(&s).is_empty(), "健康窗口不应报异常");
    }
}
