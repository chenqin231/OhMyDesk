//! 脏区检测纯函数：瓦片哈希 + 变化计数 + 跳过决策。零 X11 依赖，全单测。

use image::RgbaImage;
use std::hash::Hasher;
use twox_hash::XxHash64;

/// 把 RGBA 帧按固定像素边长切网格，每块算一个 64bit 哈希。
/// 返回 (tile_cols, tile_rows, Vec<u64>)；行末/列末不足一整块按实际像素算。
pub fn tile_hashes(img: &RgbaImage, tile_px: u32) -> (u32, u32, Vec<u64>) {
    let (w, h) = (img.width(), img.height());
    let cols = w.div_ceil(tile_px);
    let rows = h.div_ceil(tile_px);
    let raw = img.as_raw(); // &[u8]，长度 w*h*4，行主序 RGBA
    let mut hashes = Vec::with_capacity((cols * rows) as usize);
    for ty in 0..rows {
        let y0 = ty * tile_px;
        let y1 = (y0 + tile_px).min(h);
        for tx in 0..cols {
            let x0 = tx * tile_px;
            let x1 = (x0 + tile_px).min(w);
            let mut hasher = XxHash64::with_seed(0);
            for y in y0..y1 {
                let row_start = ((y * w + x0) * 4) as usize;
                let row_end = ((y * w + x1) * 4) as usize;
                hasher.write(&raw[row_start..row_end]);
            }
            hashes.push(hasher.finish());
        }
    }
    (cols, rows, hashes)
}

/// 与上帧瓦片哈希逐块比较，返回变化块数。维度不一致(分辨率变)时返回 cur.len()(全变)。
pub fn changed_tiles(prev: &[u64], cur: &[u64]) -> usize {
    if prev.len() != cur.len() {
        return cur.len(); // 维度变(分辨率变) = 全变
    }
    prev.iter().zip(cur).filter(|(a, b)| a != b).count()
}

/// 强制全量帧的常量（spec §3.3/§3.5）。
pub const KEYFRAME_INTERVAL_MS: u64 = 3000;
pub const IDLE_SKIPS_THRESHOLD: u32 = 15;
pub const IDLE_INTERVAL_MS: u64 = 200;

/// 跳过决策结果（含遥测所需的脏区比例）。
#[derive(Debug, PartialEq)]
pub struct Decision {
    /// false=跳过(不编码不发送)；true=发送整帧。
    pub send: bool,
    /// 本帧是否由三触发之一强制（keyframe周期/画质切换/首帧），非内容变化驱动。
    pub keyframe_forced: bool,
    /// changed/total 脏区比例（遥测用）。
    pub dirty_ratio: f32,
}

/// 推帧线程私有的跳过决策状态（抽成结构体便于单测）。
#[derive(Default)]
pub struct SkipState {
    last_tiles: Option<Vec<u64>>,
    last_sent_ms: u64,
    last_quality: u8,
    prev_sid: Option<String>,
    pub consecutive_skips: u32,
}

impl SkipState {
    /// 每个 due tick 截帧并算出 cur_tiles 后调用。
    /// frameskip_on=false 时永远发送（full-frame-with-telemetry 模式），但仍更新状态/算 dirty。
    /// changed==0 即 cur==last。
    pub fn decide(
        &mut self,
        now_ms: u64,
        cur_tiles: Vec<u64>,
        quality: u8,
        sid: &str,
        frameskip_on: bool,
    ) -> Decision {
        // 会话切换复位（spec §3.4）：换会话即把基准视为空。
        if self.prev_sid.as_deref() != Some(sid) {
            self.last_tiles = None;
            self.prev_sid = Some(sid.to_string());
        }

        let total = cur_tiles.len();
        let changed = match &self.last_tiles {
            Some(prev) => changed_tiles(prev, &cur_tiles),
            None => total, // 基准为空=全量
        };
        let dirty_ratio = if total > 0 { changed as f32 / total as f32 } else { 0.0 };

        let keyframe_due = now_ms.saturating_sub(self.last_sent_ms) >= KEYFRAME_INTERVAL_MS;
        let quality_changed = quality != self.last_quality;
        let force = keyframe_due || quality_changed || self.last_tiles.is_none();

        // frameskip 关闭 → 永远发送；否则「无变化且未强制」才跳过。
        let send = !frameskip_on || changed != 0 || force;

        if !send {
            self.consecutive_skips += 1;
            self.last_tiles = Some(cur_tiles); // changed==0 即 cur==last，赋值无害
            return Decision { send: false, keyframe_forced: false, dirty_ratio };
        }
        // 发送
        self.last_tiles = Some(cur_tiles);
        self.last_sent_ms = now_ms;
        self.last_quality = quality;
        self.consecutive_skips = 0;
        // keyframe_forced = 由强制触发而发（非内容变化驱动）。
        Decision { send: true, keyframe_forced: force, dirty_ratio }
    }
}

/// 空闲降采：连续跳过达阈值且无近期输入时放宽截帧间隔。
pub fn relaxed_interval(consecutive_skips: u32, base_ms: u64, has_recent_input: bool) -> u64 {
    if has_recent_input {
        return base_ms; // 任何近期输入立即恢复正常档（spec §3.5）
    }
    if consecutive_skips >= IDLE_SKIPS_THRESHOLD {
        IDLE_INTERVAL_MS.max(base_ms) // 放宽但不小于基准
    } else {
        base_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn 哈希稳定_同图全等() {
        let img = solid(200, 150, [10, 20, 30, 255]);
        let (c1, r1, h1) = tile_hashes(&img, 64);
        let (c2, r2, h2) = tile_hashes(&img, 64);
        assert_eq!((c1, r1), (c2, r2));
        assert_eq!(h1, h2, "同一图两次哈希必须全等");
        // 200x150 / 64 → cols=4(0,64,128,192) rows=3(0,64,128)
        assert_eq!((c1, r1), (4, 3));
        assert_eq!(h1.len(), 12);
    }

    #[test]
    fn 单像素改动_只动对应块() {
        let img = solid(200, 150, [10, 20, 30, 255]);
        let (cols, _rows, base) = tile_hashes(&img, 64);
        // 改 (100, 70) 像素 → 落在 tile (col=1, row=1)
        let mut img2 = img.clone();
        img2.put_pixel(100, 70, Rgba([99, 99, 99, 255]));
        let (_c, _r, after) = tile_hashes(&img2, 64);
        let changed_idx = (1 * cols + 1) as usize; // row*cols+col
        for (i, (a, b)) in base.iter().zip(&after).enumerate() {
            if i == changed_idx {
                assert_ne!(a, b, "被改像素所在块哈希必须变");
            } else {
                assert_eq!(a, b, "其余块哈希必须不变 (块 {i})");
            }
        }
    }

    #[test]
    fn changed_tiles_各情况() {
        assert_eq!(changed_tiles(&[1, 2, 3, 4], &[1, 2, 3, 4]), 0, "全同=0");
        assert_eq!(changed_tiles(&[1, 2, 3, 4], &[9, 9, 9, 9]), 4, "全异=total");
        assert_eq!(changed_tiles(&[1, 2, 3, 4], &[1, 9, 3, 4]), 1, "改1块=1");
        assert_eq!(changed_tiles(&[1, 2, 3, 4], &[1, 2, 3]), 3, "维度不一致=cur.len(全变)");
    }

    #[test]
    fn 首帧必发() {
        let mut st = SkipState::default();
        let d = st.decide(1000, vec![1, 2, 3], 0, "s1", true);
        assert!(d.send && d.keyframe_forced, "首帧强制发");
    }

    #[test]
    fn 静止跳过_变化发送() {
        let mut st = SkipState::default();
        st.decide(1000, vec![1, 2, 3], 0, "s1", true); // 首帧发
        let d = st.decide(1050, vec![1, 2, 3], 0, "s1", true);
        assert!(!d.send, "无变化→跳过");
        assert_eq!(d.dirty_ratio, 0.0);
        assert_eq!(st.consecutive_skips, 1);
        let d2 = st.decide(1100, vec![1, 9, 3], 0, "s1", true);
        assert!(d2.send && !d2.keyframe_forced, "有变化→发(非强制)");
        assert!((d2.dirty_ratio - 1.0 / 3.0).abs() < 1e-6, "1/3 块变");
        assert_eq!(st.consecutive_skips, 0, "发送后清零");
    }

    #[test]
    fn frameskip关闭_永远发送() {
        let mut st = SkipState::default();
        st.decide(1000, vec![1, 2, 3], 0, "s1", false); // 首帧
        let d = st.decide(1050, vec![1, 2, 3], 0, "s1", false); // 无变化但 frameskip off
        assert!(d.send, "frameskip off → 即使无变化也发");
        assert_eq!(d.dirty_ratio, 0.0, "dirty 仍如实计算");
    }

    #[test]
    fn keyframe周期_静止也发() {
        let mut st = SkipState::default();
        st.decide(1000, vec![1, 2, 3], 0, "s1", true);
        assert!(!st.decide(1050, vec![1, 2, 3], 0, "s1", true).send);
        // 距上次发送超 3000ms → 强制
        let d = st.decide(1000 + KEYFRAME_INTERVAL_MS, vec![1, 2, 3], 0, "s1", true);
        assert!(d.send && d.keyframe_forced);
    }

    #[test]
    fn 画质切换_静止也发() {
        let mut st = SkipState::default();
        st.decide(1000, vec![1, 2, 3], 0, "s1", true);
        let d = st.decide(1050, vec![1, 2, 3], 1, "s1", true); // quality 0→1
        assert!(d.send && d.keyframe_forced);
    }

    #[test]
    fn 会话切换_复位必发() {
        let mut st = SkipState::default();
        st.decide(1000, vec![1, 2, 3], 0, "s1", true);
        // 同内容但换会话 → last_tiles 视为 None → 必发
        let d = st.decide(1050, vec![1, 2, 3], 0, "s2", true);
        assert!(d.send && d.keyframe_forced);
    }

    #[test]
    fn 空闲降采_阈值与输入() {
        // 未达阈值：用基准间隔
        assert_eq!(relaxed_interval(5, 40, false), 40);
        // 达阈值且无输入：放宽
        assert_eq!(relaxed_interval(IDLE_SKIPS_THRESHOLD, 40, false), IDLE_INTERVAL_MS);
        // 达阈值但有近期输入：立即恢复基准
        assert_eq!(relaxed_interval(IDLE_SKIPS_THRESHOLD, 40, true), 40);
    }
}
