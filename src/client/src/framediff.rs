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
}
