//! 被控端遥测：两数据源（FrameSample/EgressSample）按 seq 合并的 collector，
//! 窗口聚合 + 异常分类 + 环形缓冲 + 触发式 dump。纯逻辑优先单测。

/// 来源 A：推帧线程产出（采集/跳过/编码段，无出网字段）。
#[derive(Debug, Clone)]
pub struct FrameSample {
    pub ts_ms: u64,
    pub seq: u64, // 发送帧 seq；跳过 tick 记 last_sent_seq 且 skipped=true
    pub capture_ms: u32,
    pub skipped: bool,
    pub dirty_ratio: f32,
    pub keyframe_forced: bool,
    pub encode_ms: u32,
    pub resize_ms: u32,
    pub jpeg_ms: u32,
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
    pub enc_bps: u64, // Σencoded_bytes / 窗秒
    pub bytes_avg: usize,
    pub bytes_p95: usize,
    pub cap_p95_ms: u32,
    pub enc_avg_ms: u32,
    pub enc_p95_ms: u32,
    pub resize_avg_ms: u32,
    pub resize_p95_ms: u32,
    pub jpeg_avg_ms: u32,
    pub jpeg_p95_ms: u32,
    pub stall_p95_ms: u32,
    pub egress_drop: usize, // sent(非跳过) − egress_writes，clamp≥0
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

/// 取已排序切片的 p 分位（p∈[0,1]），空切片返回默认值（0/0.0）。泛型统一 u32/f32/usize。
fn percentile<T: Copy + Default>(sorted: &[T], p: f32) -> T {
    if sorted.is_empty() {
        return T::default();
    }
    let idx = ((sorted.len() as f32 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 对一窗的合并样本聚合（window_ms = 窗口时长，用于算速率/fps）。
pub fn aggregate(frames: &[FrameSample], egress: &[EgressSample], window_ms: u64) -> WindowStats {
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

    let mut resize_v: Vec<u32> = sent_frames.iter().map(|f| f.resize_ms).collect();
    resize_v.sort_unstable();
    let mut jpeg_v: Vec<u32> = sent_frames.iter().map(|f| f.jpeg_ms).collect();
    jpeg_v.sort_unstable();
    let resize_avg_ms = if sent > 0 {
        resize_v.iter().sum::<u32>() / sent as u32
    } else {
        0
    };
    let jpeg_avg_ms = if sent > 0 {
        jpeg_v.iter().sum::<u32>() / sent as u32
    } else {
        0
    };

    WindowStats {
        frames: total,
        sent,
        egress_writes,
        effective_fps: sent as f32 / window_s,
        skip_pct: if total > 0 {
            skipped as f32 / total as f32
        } else {
            0.0
        },
        dirty_p50: percentile(&dirty, 0.5),
        dirty_p95: percentile(&dirty, 0.95),
        enc_bps: (total_bytes as f32 / window_s) as u64,
        bytes_avg,
        bytes_p95: percentile(&bytes, 0.95),
        cap_p95_ms: percentile(&cap_ms, 0.95),
        enc_avg_ms,
        enc_p95_ms: percentile(&enc_ms, 0.95),
        resize_avg_ms,
        resize_p95_ms: percentile(&resize_v, 0.95),
        jpeg_avg_ms,
        jpeg_p95_ms: percentile(&jpeg_v, 0.95),
        stall_p95_ms: percentile(&stall, 0.95),
        egress_drop,
    }
}

/// 按阈值分类异常（纯函数）。
pub fn classify(s: &WindowStats) -> Vec<Anomaly> {
    let mut out = vec![];
    // 出网阻塞：stall 高 或 单窗 egress_drop 占已发比 > 50%
    let drop_ratio = if s.sent > 0 {
        s.egress_drop as f32 / s.sent as f32
    } else {
        0.0
    };
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

use std::collections::VecDeque;

/// 合并后的单帧记录（FrameSample + 贴回的出网字段）。
#[derive(Debug, Clone)]
pub struct MergedSample {
    pub frame: FrameSample,
    pub send_stall_ms: Option<u32>, // 跳过帧/未发出帧为 None
}

/// 触发式 dump 的去抖窗（同类异常 N 秒内只 dump 一次）。
pub const DUMP_DEBOUNCE_MS: u64 = 30_000;
/// 环形缓冲保留时长（5 分钟）。
pub const RING_RETAIN_MS: u64 = 300_000;

pub struct Collector {
    ring: VecDeque<MergedSample>,
    pending_egress: std::collections::HashMap<u64, u32>, // seq→stall，等待对应 frame
    events: VecDeque<String>,
    last_dump_ms: Option<u64>, // None = 从未 dump
    pub sid: String,
}

impl Collector {
    pub fn new(sid: String) -> Self {
        Collector {
            ring: VecDeque::new(),
            pending_egress: std::collections::HashMap::new(),
            events: VecDeque::new(),
            last_dump_ms: None,
            sid,
        }
    }

    /// 收到一条帧样本：合并已到的 egress（乱序容忍），入环并按时长裁剪。
    pub fn on_frame(&mut self, f: FrameSample) {
        let now = f.ts_ms;
        let stall = if f.skipped {
            None
        } else {
            self.pending_egress.remove(&f.seq)
        };
        self.ring.push_back(MergedSample {
            frame: f,
            send_stall_ms: stall,
        });
        // 按时长裁剪环（保留最近 RING_RETAIN_MS）
        while let Some(front) = self.ring.front() {
            if now.saturating_sub(front.frame.ts_ms) > RING_RETAIN_MS {
                self.ring.pop_front();
            } else {
                break;
            }
        }
    }

    /// 收到一条出网样本：若对应帧已在环则贴回，否则暂存等待。
    pub fn on_egress(&mut self, e: EgressSample) {
        // 若对应帧已在环（多数情况 egress 紧随 frame），就地贴回；否则暂存等待。
        if let Some(m) = self
            .ring
            .iter_mut()
            .rev()
            .find(|m| m.frame.seq == e.seq && !m.frame.skipped)
        {
            m.send_stall_ms = Some(e.send_stall_ms);
        } else {
            self.pending_egress.insert(e.seq, e.send_stall_ms);
        }
    }

    /// 是否应触发 dump：命中异常且过了去抖窗。调用即更新 last_dump_ms。
    pub fn should_dump(&mut self, now_ms: u64, anomalies: &[Anomaly]) -> bool {
        if anomalies.is_empty() {
            return false;
        }
        // 从未 dump（None）直接允许；有记录则检查去抖窗。
        if let Some(last) = self.last_dump_ms {
            if now_ms.saturating_sub(last) < DUMP_DEBOUNCE_MS {
                return false;
            }
        }
        self.last_dump_ms = Some(now_ms);
        true
    }

    /// 当前环内帧数（测试/诊断用）。
    pub fn ring_len(&self) -> usize {
        self.ring.len()
    }

    /// 取最近一条合并样本的 stall（测试用）。
    pub fn last_stall(&self) -> Option<u32> {
        self.ring.back().and_then(|m| m.send_stall_ms)
    }

    /// 遍历环内所有样本（用于 dump）。
    pub fn ring_iter(&self) -> impl Iterator<Item = &MergedSample> {
        self.ring.iter()
    }

    /// 获取 sid 字符串（用于日志格式化）。
    pub fn sid_str(&self) -> String {
        self.sid.clone()
    }
}

use std::path::{Path, PathBuf};

/// 遥测通道消息（worker 与 conn.rs 各投一种；Event 为离散事件）。
pub enum TelemetryMsg {
    Frame(FrameSample),
    Egress(EgressSample),
    Event(String),
    ExportNow, // UI 手动导出：立即 dump 环形缓冲（忽略去抖）
}

/// 把窗口聚合格式化成一行可 grep 的日志（spec §4.2）。
pub fn format_log(s: &WindowStats, sid: &str, adapt_level: u8) -> String {
    format!(
        "遥测 sid={sid} win=10s effective_fps={:.1} skip_pct={:.2} dirty_p50={:.2} dirty_p95={:.2} \
         sent_frames={} egress_writes={} egress_drop={} enc_Bps={} bytes_avg={} bytes_p95={} \
         cap_p95_ms={} enc_avg_ms={} enc_p95_ms={} resize_avg_ms={} jpeg_avg_ms={} stall_p95_ms={} adapt_level={}",
        s.effective_fps, s.skip_pct, s.dirty_p50, s.dirty_p95,
        s.sent, s.egress_writes, s.egress_drop, s.enc_bps, s.bytes_avg, s.bytes_p95,
        s.cap_p95_ms, s.enc_avg_ms, s.enc_p95_ms, s.resize_avg_ms, s.jpeg_avg_ms,
        s.stall_p95_ms, adapt_level
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(
        seq: u64,
        skipped: bool,
        dirty: f32,
        enc_ms: u32,
        bytes: usize,
        cap_ms: u32,
    ) -> FrameSample {
        FrameSample {
            ts_ms: seq * 50,
            seq,
            capture_ms: cap_ms,
            skipped,
            dirty_ratio: dirty,
            keyframe_forced: false,
            encode_ms: enc_ms,
            resize_ms: 0,
            jpeg_ms: 0,
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
            EgressSample {
                seq: 1,
                send_stall_ms: 100,
                sent_ok: true,
                ws_error: false,
            },
            EgressSample {
                seq: 2,
                send_stall_ms: 180,
                sent_ok: true,
                ws_error: false,
            },
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
        let frames = vec![
            fs(1, false, 0.3, 30, 50_000, 12),
            fs(2, false, 0.3, 30, 50_000, 12),
            fs(3, false, 0.3, 30, 50_000, 12),
        ];
        let egress = vec![EgressSample {
            seq: 3,
            send_stall_ms: 1200,
            sent_ok: true,
            ws_error: false,
        }];
        let s = aggregate(&frames, &egress, 10_000);
        assert_eq!(s.sent, 3);
        assert_eq!(s.egress_writes, 1);
        assert_eq!(s.egress_drop, 2, "Δenqueued−Δsent=3−1=2");
    }

    #[test]
    fn 分类_出网阻塞() {
        let s = WindowStats {
            sent: 10,
            egress_writes: 3,
            egress_drop: 7,
            stall_p95_ms: 1200,
            ..Default::default()
        };
        assert!(classify(&s).contains(&Anomaly::Egress阻塞));
    }

    #[test]
    fn 分类_投递饥饿() {
        let s = WindowStats {
            effective_fps: 0.5,
            dirty_p95: 0.3,
            ..Default::default()
        };
        assert!(classify(&s).contains(&Anomaly::投递饥饿));
    }

    #[test]
    fn 分类_编码过载() {
        let s = WindowStats {
            enc_p95_ms: 250,
            ..Default::default()
        };
        assert!(classify(&s).contains(&Anomaly::编码过载));
    }

    #[test]
    fn 分类_frameskip失效() {
        let s = WindowStats {
            skip_pct: 0.1,
            dirty_p95: 0.02,
            ..Default::default()
        };
        assert!(classify(&s).contains(&Anomaly::FrameSkip失效));
    }

    #[test]
    fn 分类_正常窗不误报() {
        let s = WindowStats {
            frames: 100,
            sent: 20,
            egress_writes: 20,
            effective_fps: 2.0,
            skip_pct: 0.8,
            dirty_p95: 0.2,
            enc_p95_ms: 60,
            stall_p95_ms: 150,
            ..Default::default()
        };
        assert!(classify(&s).is_empty(), "健康窗口不应报异常");
    }

    fn frame_at(seq: u64, ts: u64) -> FrameSample {
        FrameSample {
            ts_ms: ts,
            seq,
            capture_ms: 10,
            skipped: false,
            dirty_ratio: 0.2,
            keyframe_forced: false,
            encode_ms: 30,
            resize_ms: 0,
            jpeg_ms: 0,
            encoded_bytes: 50_000,
            w: 1280,
            h: 720,
        }
    }

    #[test]
    fn 合并_egress先到后到都正确() {
        let mut c = Collector::new("s1".into());
        // 帧先到，egress 后到
        c.on_frame(frame_at(1, 1000));
        c.on_egress(EgressSample {
            seq: 1,
            send_stall_ms: 120,
            sent_ok: true,
            ws_error: false,
        });
        assert_eq!(c.last_stall(), Some(120), "帧先到→egress 贴回");
        // egress 先到，帧后到
        c.on_egress(EgressSample {
            seq: 2,
            send_stall_ms: 200,
            sent_ok: true,
            ws_error: false,
        });
        c.on_frame(frame_at(2, 1050));
        assert_eq!(c.last_stall(), Some(200), "egress 先到→暂存待帧到贴回");
    }

    #[test]
    fn 跳过帧无egress不报错() {
        let mut c = Collector::new("s1".into());
        let mut skipped = frame_at(1, 1000);
        skipped.skipped = true;
        c.on_frame(skipped);
        assert_eq!(c.last_stall(), None, "跳过帧无 egress 样本");
        assert_eq!(c.ring_len(), 1);
    }

    #[test]
    fn 环形缓冲_超时裁剪() {
        let mut c = Collector::new("s1".into());
        c.on_frame(frame_at(1, 1000));
        // 5 分钟后的新帧 → 老帧应被裁掉
        c.on_frame(frame_at(2, 1000 + RING_RETAIN_MS + 1));
        assert_eq!(c.ring_len(), 1, "超 5min 的老帧裁剪");
    }

    #[test]
    fn dump去抖() {
        let mut c = Collector::new("s1".into());
        let anomalies = vec![Anomaly::Egress阻塞];
        assert!(c.should_dump(10_000, &anomalies), "首次命中→dump");
        assert!(!c.should_dump(10_000 + 1000, &anomalies), "去抖窗内不重复");
        assert!(
            c.should_dump(10_000 + DUMP_DEBOUNCE_MS + 1, &anomalies),
            "过去抖窗→再 dump"
        );
        assert!(!c.should_dump(99_999_999, &[]), "无异常→不 dump");
    }

    #[test]
    fn format_log含关键字段() {
        let s = WindowStats {
            sent: 2,
            egress_writes: 2,
            egress_drop: 0,
            skip_pct: 0.8,
            effective_fps: 0.2,
            enc_bps: 12_000,
            stall_p95_ms: 180,
            ..Default::default()
        };
        let line = format_log(&s, "ab12", 0);
        assert!(line.contains("sid=ab12"));
        assert!(line.contains("skip_pct=0.80"));
        assert!(line.contains("egress_drop=0"));
        assert!(line.contains("stall_p95_ms=180"));
    }

    #[test]
    fn main_recv_seq_gap累计() {
        let mut s = MainRecvStats::default();
        assert_eq!(s.on_frame(1, 20, 1000), None);
        assert_eq!(s.on_frame(2, 20, 1100), None); // 连续，无 gap
        assert_eq!(s.on_frame(5, 20, 1200), None); // 跳 3,4 → gap+2
                                                   // 窗满触发日志
        let line = s.on_frame(6, 20, 12_000).expect("窗满应出日志");
        assert!(
            line.contains("seq_gap=2"),
            "缺 3、4 两帧 → seq_gap=2: {line}"
        );
        assert!(line.contains("recv_fps=0.4"), "4 帧/10s");
    }

    #[test]
    fn main_recv_drop_stale累计() {
        let mut s = MainRecvStats::default();
        s.on_drop_stale(3);
        s.on_frame(1, 10, 1000);
        let line = s.on_frame(2, 10, 12_000).unwrap();
        assert!(line.contains("drop_stale=3"));
    }

    #[test]
    fn aggregate拆分resize_jpeg() {
        let mk = |resize_ms, jpeg_ms| FrameSample {
            ts_ms: 0,
            seq: 1,
            capture_ms: 5,
            skipped: false,
            dirty_ratio: 0.2,
            keyframe_forced: false,
            encode_ms: resize_ms + jpeg_ms,
            resize_ms,
            jpeg_ms,
            encoded_bytes: 1000,
            w: 100,
            h: 100,
        };
        let frames = vec![mk(10, 100), mk(30, 200)];
        let s = aggregate(&frames, &[], 10_000);
        assert_eq!(s.resize_avg_ms, 20, "resize 均值 (10+30)/2");
        assert_eq!(s.jpeg_avg_ms, 150, "jpeg 均值 (100+200)/2");
    }

    #[test]
    fn format_log含resize_jpeg_adapt字段() {
        let s = WindowStats {
            resize_avg_ms: 12,
            jpeg_avg_ms: 340,
            ..Default::default()
        };
        let out = format_log(&s, "sid1", 2);
        assert!(out.contains("resize_avg_ms=12"), "含 resize_avg_ms");
        assert!(out.contains("jpeg_avg_ms=340"), "含 jpeg_avg_ms");
        assert!(out.contains("adapt_level=2"), "含 adapt_level");
    }
}

/// 把环形缓冲 dump 成 JSONL 诊断包（脱敏：只含指标，绝不含像素/剪贴板/文件内容）。
/// 单包封顶 2MB，超出截断旧样本。
pub fn dump_ring(c: &Collector, diag_dir: &Path, ts_ms: u64) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(diag_dir)?;
    let path = diag_dir.join(format!("diag-{ts_ms}-{}.jsonl", c.sid));
    let mut f = std::fs::File::create(&path)?;
    let mut written = 0usize;
    const CAP: usize = 2 * 1024 * 1024;
    for m in c.ring_iter() {
        let line = format!(
            "{{\"ts_ms\":{},\"seq\":{},\"skipped\":{},\"dirty\":{:.3},\"keyframe\":{},\"cap_ms\":{},\"enc_ms\":{},\"bytes\":{},\"stall_ms\":{},\"w\":{},\"h\":{}}}\n",
            m.frame.ts_ms, m.frame.seq, m.frame.skipped, m.frame.dirty_ratio, m.frame.keyframe_forced,
            m.frame.capture_ms, m.frame.encode_ms, m.frame.encoded_bytes,
            m.send_stall_ms.map(|v| v as i64).unwrap_or(-1), m.frame.w, m.frame.h
        );
        if written + line.len() > CAP {
            break;
        }
        f.write_all(line.as_bytes())?;
        written += line.len();
    }
    Ok(path)
}

/// 异步 collector 任务：收两源消息 → 10s 窗聚合日志 + 异常分类 + 命中即落盘。
pub async fn run_collector(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<TelemetryMsg>,
    diag_dir: PathBuf,
) {
    let mut collector = Collector::new(String::new());
    let mut win_frames: Vec<FrameSample> = vec![];
    let mut win_egress: Vec<EgressSample> = vec![];
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut adaptive = crate::adaptive::AdaptiveController::default();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(TelemetryMsg::Frame(f)) => { win_frames.push(f.clone()); collector.on_frame(f); }
                Some(TelemetryMsg::Egress(e)) => { win_egress.push(e.clone()); collector.on_egress(e); }
                Some(TelemetryMsg::Event(ev)) => tracing::info!("遥测事件 {ev}"),
                Some(TelemetryMsg::ExportNow) => {
                    let now = win_frames.last().map(|f| f.ts_ms)
                        .or_else(|| collector.ring_iter().last().map(|m| m.frame.ts_ms))
                        .unwrap_or(0);
                    match dump_ring(&collector, &diag_dir, now) {
                        Ok(p) => tracing::warn!("手动导出诊断包 {}", p.display()),
                        Err(e) => tracing::warn!("手动导出失败 {e}"),
                    }
                }
                None => break,
            },
            _ = ticker.tick() => {
                // 手动切档请求重置：重建控制器（清 streak+归零），让手动选择先生效再重新评估。
                if crate::adaptive::take_reset() {
                    adaptive = crate::adaptive::AdaptiveController::default();
                }
                if win_frames.is_empty() { continue; }
                let stats = aggregate(&win_frames, &win_egress, 10_000);
                let lvl = adaptive.observe(&stats);
                crate::adaptive::store_level(lvl);
                tracing::info!("{}", format_log(&stats, &collector.sid_str(), lvl));
                let anomalies = classify(&stats);
                if !anomalies.is_empty() {
                    tracing::warn!("遥测异常 {anomalies:?}");
                    let now = win_frames.last().map(|f| f.ts_ms).unwrap_or(0);
                    if collector.should_dump(now, &anomalies) {
                        match dump_ring(&collector, &diag_dir, now) {
                            Ok(p) => tracing::warn!("诊断包已落盘 {}", p.display()),
                            Err(e) => tracing::warn!("诊断包落盘失败 {e}"),
                        }
                    }
                }
                win_frames.clear();
                win_egress.clear();
            }
        }
    }
}

/// 主控端收帧统计（纯日志，10s 窗）。seq_gap=相邻渲染帧 seq 差>1 的累计缺失数。
pub struct MainRecvStats {
    window_start_ms: u64,
    frames: u32,
    decode_ms_sum: u64,
    drop_stale: u32,
    last_seq: Option<u64>,
    seq_gap: u64,
}

impl Default for MainRecvStats {
    fn default() -> Self {
        MainRecvStats {
            window_start_ms: 0,
            frames: 0,
            decode_ms_sum: 0,
            drop_stale: 0,
            last_seq: None,
            seq_gap: 0,
        }
    }
}

impl MainRecvStats {
    pub fn on_drop_stale(&mut self, n: u32) {
        self.drop_stale += n;
    }

    /// 喂一帧；窗满(≥10s)返回一行日志并复位窗口(保留 last_seq 跨窗连续)。
    pub fn on_frame(&mut self, seq: u64, decode_ms: u32, now_ms: u64) -> Option<String> {
        if self.window_start_ms == 0 {
            self.window_start_ms = now_ms;
        }
        if let Some(last) = self.last_seq {
            if seq > last + 1 {
                self.seq_gap += seq - last - 1;
            }
        }
        self.last_seq = Some(seq);
        self.frames += 1;
        self.decode_ms_sum += decode_ms as u64;
        if now_ms.saturating_sub(self.window_start_ms) >= 10_000 {
            let decode_avg = if self.frames > 0 {
                self.decode_ms_sum / self.frames as u64
            } else {
                0
            };
            let line = format!(
                "主控遥测 recv_fps={:.1} decode_avg_ms={} drop_stale={} seq_gap={}",
                self.frames as f32 / 10.0,
                decode_avg,
                self.drop_stale,
                self.seq_gap
            );
            let keep_seq = self.last_seq;
            *self = MainRecvStats::default();
            self.last_seq = keep_seq;
            return Some(line);
        }
        None
    }
}
