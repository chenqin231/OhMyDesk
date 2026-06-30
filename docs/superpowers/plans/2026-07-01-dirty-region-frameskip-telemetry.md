# Spec D2-① 被控端 frame-skip + 脏区遥测 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 被控端静止画面不编码不发送（纯减法降公网中转带宽），同时落地两数据源遥测 + 触发式诊断包，让上线后「是否需要瓦片增量编码」由数据决定。

**Architecture:** 改动主体局限被控端推帧线程（`workers.rs`），辅以新增纯函数模块 `framediff.rs`（瓦片哈希+跳过决策）、`telemetry.rs`（双源样本按 seq 合并的 collector）、`render_mode.rs`（5 档模式原子组）；越出被控端的只有 server `frame_lane_drop` 与主控端/admin-web 收帧埋点，**全部纯日志、无路由/渲染/协议改动**。每个发出的帧仍是自给自足全量帧，梯队1 三层 drop-stale 与主控整帧渲染原样有效。

**Tech Stack:** Rust（client/server workspace，tokio + slint + xcap + image/jpeg）、twox-hash（XxHash64 瓦片哈希）、tracing-appender（滚动日志）、toml（模式配置）、TypeScript（admin-web，React + zustand）。

**Spec:** `docs/superpowers/specs/2026-07-01-dirty-region-frameskip-telemetry-design.md`（三轮 Codex 评审已落实）。

---

## 文件结构（决策锁定）

| 文件 | 动作 | 单一职责 |
|---|---|---|
| `src/client/src/framediff.rs` | 新建 | 瓦片哈希 `tile_hashes`/变化计数 `changed_tiles` + 跳过状态机 `SkipState`/空闲降采 `relaxed_interval`（纯函数，零 X11 依赖） |
| `src/client/src/telemetry.rs` | 新建 | `FrameSample`/`EgressSample`/`Collector`（按 seq 合并双源）/窗口聚合/异常分类 `classify`/环形缓冲/触发式 dump（纯逻辑 + 一个薄 async 包装） |
| `src/client/src/render_mode.rs` | 新建 | `RenderMode` 5 档枚举 + 原子组（frameskip/telemetry/mode）+ env/arg/config 优先级解析 + low-bandwidth clamp |
| `src/client/src/capture.rs` | 改 | 新增 `capture_raw()`，拆「截」与「编码」 |
| `src/client/src/workers.rs` | 改 | `consume_capture` 推帧线程按 `render_mode` 分支 + 跳过决策 + 投 `FrameSample` |
| `src/client/src/net/conn.rs` | 改 | frame watch 改携 `(seq, json)`；出网产 `EgressSample`（send_stall 实测）经 telemetry 通道回流 |
| `src/client/src/net/mod.rs` | 改 | `ToUi::Frame` 加 `seq` 字段；reconnect 事件埋点 |
| `src/client/src/net/dispatch.rs` | 改 | 主控端保留 seq（`..`→`seq`），透传 `ToUi::Frame` |
| `src/client/src/ui_glue.rs` | 改 | 主控端纯日志埋点（recv_fps/decode_ms/drop_stale/首帧/seq_gap）；UI 隐藏诊断菜单（模式热切 + 导出/复制诊断包） |
| `src/client/src/main.rs` | 改 | 默认滚动日志（tracing-appender）；`mod framediff/telemetry/render_mode;`；telemetry collector + 通道接线；模式初始化 |
| `src/server/src/hub.rs` | 改 | `FrameLaneStat`（enqueued/sent 原子）+ `send_frame_to` 计数 |
| `src/server/src/main.rs` | 改 | 出站泵写帧处累加 sent + 周期 `frame_lane_drop` debug! |
| `src/admin-web/src/lib/diag-ring.ts` | 新建 | 纯函数 ring buffer push（按时间窗，可单测） |
| `src/admin-web/src/store.ts` | 改 | 收帧分支注入 diag ring + 导出 action |
| `src/admin-web/src/components/control/remote-session.tsx` | 改 | 工具栏加「下载诊断 JSON」按钮 |
| `src/client/Cargo.toml` | 改 | 加 `twox-hash`/`tracing-appender`/`toml` |

**依赖顺序**：阶段一（纯函数地基，无 X11，全单测）→ 阶段二（被控端集成）→ 阶段三（跨端取证）→ 阶段四（netem 上线门）。

---

# 阶段一：纯函数地基（被控端，零 X11，全单测）

## Task 1：framediff 瓦片哈希 + 变化计数

**Files:**
- Create: `src/client/src/framediff.rs`
- Modify: `src/client/src/main.rs:15-27`（加 `mod framediff;`）
- Modify: `src/client/Cargo.toml:9`（加 `twox-hash`）

- [ ] **Step 1: 加依赖 twox-hash**

修改 `src/client/Cargo.toml`，在 `[dependencies]` 段（`base64 = "0.22"` 下一行）加：

```toml
twox-hash = "1.6"                                       # 脏区瓦片哈希（XxHash64，整图一次遍历<1ms）
```

- [ ] **Step 2: 注册模块**

修改 `src/client/src/main.rs`，在 `mod geom;`（第 20 行）后加一行：

```rust
mod framediff;
```

- [ ] **Step 3: 写失败测试**

创建 `src/client/src/framediff.rs`，先只写测试（实现函数留空签名让其编译失败）：

```rust
//! 脏区检测纯函数：瓦片哈希 + 变化计数 + 跳过决策。零 X11 依赖，全单测。

use image::RgbaImage;
use std::hash::Hasher;
use twox_hash::XxHash64;

/// 把 RGBA 帧按固定像素边长切网格，每块算一个 64bit 哈希。
/// 返回 (tile_cols, tile_rows, Vec<u64>)；行末/列末不足一整块按实际像素算。
pub fn tile_hashes(img: &RgbaImage, tile_px: u32) -> (u32, u32, Vec<u64>) {
    unimplemented!()
}

/// 与上帧瓦片哈希逐块比较，返回变化块数。维度不一致(分辨率变)时返回 cur.len()(全变)。
pub fn changed_tiles(prev: &[u64], cur: &[u64]) -> usize {
    unimplemented!()
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
```

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test -p client framediff::`
Expected: 编译通过但 panic `not implemented`（或 unimplemented），FAIL。

- [ ] **Step 5: 实现两个函数**

把 `src/client/src/framediff.rs` 顶部两个 `unimplemented!()` 替换为：

```rust
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

pub fn changed_tiles(prev: &[u64], cur: &[u64]) -> usize {
    if prev.len() != cur.len() {
        return cur.len(); // 维度变(分辨率变) = 全变
    }
    prev.iter().zip(cur).filter(|(a, b)| a != b).count()
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p client framediff::`
Expected: PASS（3 个测试全绿）。

- [ ] **Step 7: 提交**

```bash
git add src/client/Cargo.toml src/client/src/main.rs src/client/src/framediff.rs
git commit -m "feat(client): framediff 瓦片哈希+变化计数(脏区检测纯函数地基)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 2：SkipState 跳过决策状态机 + 空闲降采

**Files:**
- Modify: `src/client/src/framediff.rs`（追加 `SkipState`/`SkipDecision`/`relaxed_interval` + 测试）

跳过决策的全部有状态逻辑（keyframe 周期、画质切换、会话复位、首帧、连续跳过计数）抽成纯结构体，使推帧线程只负责「截/比/编/发」。

- [ ] **Step 1: 写失败测试**

在 `src/client/src/framediff.rs` 的 `#[cfg(test)] mod tests` **之前**插入实现占位 + 在 `mod tests` 内追加测试。先插占位（编译可过、逻辑未实现）：

```rust
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
        unimplemented!()
    }
}

/// 空闲降采：连续跳过达阈值且无近期输入时放宽截帧间隔。
pub fn relaxed_interval(consecutive_skips: u32, base_ms: u64, has_recent_input: bool) -> u64 {
    unimplemented!()
}
```

在 `mod tests` 内追加：

```rust
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client framediff::`
Expected: FAIL（`unimplemented`）。

- [ ] **Step 3: 实现 decide + relaxed_interval**

替换两个 `unimplemented!()`：

```rust
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
```

实现 `relaxed_interval`：

```rust
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client framediff::`
Expected: PASS（含 Task 1 共 10 个测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/framediff.rs
git commit -m "feat(client): SkipState 跳过决策状态机+空闲降采(keyframe/画质/会话复位)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 3：telemetry 样本结构 + 窗口聚合 + 异常分类（纯函数）

**Files:**
- Create: `src/client/src/telemetry.rs`
- Modify: `src/client/src/main.rs`（加 `mod telemetry;`）

- [ ] **Step 1: 注册模块**

修改 `src/client/src/main.rs`，在 `mod framediff;`（Task 1 加的）后加：

```rust
mod telemetry;
```

- [ ] **Step 2: 写失败测试**

创建 `src/client/src/telemetry.rs`：

```rust
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

/// 对一窗的合并样本聚合（window_ms = 窗口时长，用于算速率/fps）。
pub fn aggregate(
    frames: &[FrameSample],
    egress: &[EgressSample],
    window_ms: u64,
) -> WindowStats {
    unimplemented!()
}

/// 按阈值分类异常（纯函数）。
pub fn classify(s: &WindowStats) -> Vec<Anomaly> {
    unimplemented!()
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
        for i in 0..8 {
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
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p client telemetry::`
Expected: FAIL（`unimplemented`）。

- [ ] **Step 4: 实现 aggregate + classify + 分位辅助**

替换两个 `unimplemented!()`，并在文件内（`aggregate` 上方）加分位辅助：

```rust
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
    if s.skip_pct < 0.2 && s.dirty_p95 < 0.05 && s.frames > 0 {
        out.push(Anomaly::FrameSkip失效);
    }
    out
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p client telemetry::`
Expected: PASS（7 个测试全绿）。

- [ ] **Step 6: 提交**

```bash
git add src/client/src/main.rs src/client/src/telemetry.rs
git commit -m "feat(client): telemetry 样本结构+窗口聚合+异常分类(纯函数)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 4：telemetry Collector — 按 seq 合并 + 环形缓冲 + 触发式 dump

**Files:**
- Modify: `src/client/src/telemetry.rs`（追加 `Collector` + 测试）

Collector 持有近期 `FrameSample`（按 seq 暂存）与 `EgressSample`，合并、维护 5min 环形缓冲、满 10s 窗聚合出日志、命中异常去抖落盘。纯逻辑结构体，async 包装留到 Task 9 接线。

- [ ] **Step 1: 写失败测试**

在 `src/client/src/telemetry.rs` 的 `mod tests` 之前插入 `Collector` 占位：

```rust
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
    last_dump_ms: u64,
    sid: String,
}

impl Collector {
    pub fn new(sid: String) -> Self {
        Collector {
            ring: VecDeque::new(),
            pending_egress: std::collections::HashMap::new(),
            events: VecDeque::new(),
            last_dump_ms: 0,
            sid,
        }
    }

    /// 收到一条帧样本：合并已到的 egress（乱序容忍），入环并按时长裁剪。
    pub fn on_frame(&mut self, f: FrameSample) {
        unimplemented!()
    }

    /// 收到一条出网样本：若对应帧已在环则贴回，否则暂存等待。
    pub fn on_egress(&mut self, e: EgressSample) {
        unimplemented!()
    }

    /// 是否应触发 dump：命中异常且过了去抖窗。调用即更新 last_dump_ms。
    pub fn should_dump(&mut self, now_ms: u64, anomalies: &[Anomaly]) -> bool {
        unimplemented!()
    }

    /// 当前环内帧数（测试/诊断用）。
    pub fn ring_len(&self) -> usize {
        self.ring.len()
    }

    /// 取最近一条合并样本的 stall（测试用）。
    pub fn last_stall(&self) -> Option<u32> {
        self.ring.back().and_then(|m| m.send_stall_ms)
    }
}
```

在 `mod tests` 内追加：

```rust
    fn frame_at(seq: u64, ts: u64) -> FrameSample {
        FrameSample { ts_ms: ts, seq, capture_ms: 10, skipped: false, dirty_ratio: 0.2, keyframe_forced: false, encode_ms: 30, encoded_bytes: 50_000, w: 1280, h: 720 }
    }

    #[test]
    fn 合并_egress先到后到都正确() {
        let mut c = Collector::new("s1".into());
        // 帧先到，egress 后到
        c.on_frame(frame_at(1, 1000));
        c.on_egress(EgressSample { seq: 1, send_stall_ms: 120, sent_ok: true, ws_error: false });
        assert_eq!(c.last_stall(), Some(120), "帧先到→egress 贴回");
        // egress 先到，帧后到
        c.on_egress(EgressSample { seq: 2, send_stall_ms: 200, sent_ok: true, ws_error: false });
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
        assert!(c.should_dump(10_000 + DUMP_DEBOUNCE_MS + 1, &anomalies), "过去抖窗→再 dump");
        assert!(!c.should_dump(99_999_999, &[]), "无异常→不 dump");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client telemetry::`
Expected: FAIL（`unimplemented`）。

- [ ] **Step 3: 实现 Collector 方法**

替换三个 `unimplemented!()`：

```rust
    pub fn on_frame(&mut self, f: FrameSample) {
        let now = f.ts_ms;
        let stall = if f.skipped { None } else { self.pending_egress.remove(&f.seq) };
        self.ring.push_back(MergedSample { frame: f, send_stall_ms: stall });
        // 按时长裁剪环（保留最近 RING_RETAIN_MS）
        while let Some(front) = self.ring.front() {
            if now.saturating_sub(front.frame.ts_ms) > RING_RETAIN_MS {
                self.ring.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn on_egress(&mut self, e: EgressSample) {
        // 若对应帧已在环（多数情况 egress 紧随 frame），就地贴回；否则暂存等待。
        if let Some(m) = self.ring.iter_mut().rev().find(|m| m.frame.seq == e.seq && !m.frame.skipped) {
            m.send_stall_ms = Some(e.send_stall_ms);
        } else {
            self.pending_egress.insert(e.seq, e.send_stall_ms);
        }
    }

    pub fn should_dump(&mut self, now_ms: u64, anomalies: &[Anomaly]) -> bool {
        if anomalies.is_empty() {
            return false;
        }
        if now_ms.saturating_sub(self.last_dump_ms) < DUMP_DEBOUNCE_MS {
            return false;
        }
        self.last_dump_ms = now_ms;
        true
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client telemetry::`
Expected: PASS（含前序共 11 个测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/telemetry.rs
git commit -m "feat(client): telemetry Collector 按 seq 合并双源+环形缓冲+dump去抖

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

# 阶段二：被控端集成

## Task 5：render_mode 模式模块（5 档枚举 + 原子组 + 解析 + clamp）

**Files:**
- Create: `src/client/src/render_mode.rs`
- Modify: `src/client/src/main.rs`（加 `mod render_mode;`）

模式纯逻辑 + 原子组（复用 `capture.rs:16` QUALITY 范式），运行期热切靠 store/load 原子，无需重启、不动 CaptureCtrl。

- [ ] **Step 1: 注册模块**

修改 `src/client/src/main.rs`，在 `mod telemetry;` 后加：

```rust
mod render_mode;
```

- [ ] **Step 2: 写失败测试**

创建 `src/client/src/render_mode.rs`：

```rust
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
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test -p client render_mode::`
Expected: PASS（6 个测试全绿）。无需写空实现——本文件直接给完整实现，测试即验证。

- [ ] **Step 4: 提交**

```bash
git add src/client/src/main.rs src/client/src/render_mode.rs
git commit -m "feat(client): render_mode 5档模式+原子组+优先级解析+low-bandwidth clamp

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 6：telemetry 异步包装（TelemetryMsg + format_log + dump_ring + run_collector）

**Files:**
- Modify: `src/client/src/telemetry.rs`（追加通道枚举、日志格式化、落盘、collector 任务）

- [ ] **Step 1: 写 format_log 失败测试**

在 `src/client/src/telemetry.rs` 的 `mod tests` 之前插入：

```rust
use std::path::{Path, PathBuf};

/// 遥测通道消息（worker 与 conn.rs 各投一种；Event 为离散事件）。
pub enum TelemetryMsg {
    Frame(FrameSample),
    Egress(EgressSample),
    Event(String),
}

/// 把窗口聚合格式化成一行可 grep 的日志（spec §4.2）。
pub fn format_log(s: &WindowStats, sid: &str) -> String {
    format!(
        "遥测 sid={sid} win=10s effective_fps={:.1} skip_pct={:.2} dirty_p50={:.2} dirty_p95={:.2} \
         sent_frames={} egress_writes={} egress_drop={} enc_Bps={} bytes_avg={} bytes_p95={} \
         cap_p95_ms={} enc_avg_ms={} enc_p95_ms={} stall_p95_ms={}",
        s.effective_fps, s.skip_pct, s.dirty_p50, s.dirty_p95,
        s.sent, s.egress_writes, s.egress_drop, s.enc_bps, s.bytes_avg, s.bytes_p95,
        s.cap_p95_ms, s.enc_avg_ms, s.enc_p95_ms, s.stall_p95_ms
    )
}
```

在 `mod tests` 内追加：

```rust
    #[test]
    fn format_log含关键字段() {
        let s = WindowStats { sent: 2, egress_writes: 2, egress_drop: 0, skip_pct: 0.8, effective_fps: 0.2, enc_bps: 12_000, stall_p95_ms: 180, ..Default::default() };
        let line = format_log(&s, "ab12");
        assert!(line.contains("sid=ab12"));
        assert!(line.contains("skip_pct=0.80"));
        assert!(line.contains("egress_drop=0"));
        assert!(line.contains("stall_p95_ms=180"));
    }
```

- [ ] **Step 2: 跑测试确认通过**

Run: `cargo test -p client telemetry::format_log`
Expected: PASS（format_log 已给完整实现）。

- [ ] **Step 3: 加 dump_ring + run_collector（异步包装，build 验证）**

在 `src/client/src/telemetry.rs` 末尾（`mod tests` 之后）追加：

```rust
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
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(TelemetryMsg::Frame(f)) => { win_frames.push(f.clone()); collector.on_frame(f); }
                Some(TelemetryMsg::Egress(e)) => { win_egress.push(e.clone()); collector.on_egress(e); }
                Some(TelemetryMsg::Event(ev)) => tracing::info!("遥测事件 {ev}"),
                None => break,
            },
            _ = ticker.tick() => {
                if win_frames.is_empty() { continue; }
                let stats = aggregate(&win_frames, &win_egress, 10_000);
                tracing::info!("{}", format_log(&stats, &collector.sid_str()));
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
```

`run_collector` 用到 `Collector::ring_iter` 与 `Collector::sid_str`，在 `impl Collector`（Task 4）内追加两个方法：

```rust
    pub fn ring_iter(&self) -> impl Iterator<Item = &MergedSample> {
        self.ring.iter()
    }
    pub fn sid_str(&self) -> String {
        // 用最近一帧无从得知 sid，sid 由首帧 ts 关联键体现；此处返回构造时 sid。
        self.sid.clone()
    }
```

> 说明：`Collector::sid` 当前在 `new` 时传入空串。sid 关联主要靠每条 FrameSample 的 seq + 日志 ts；若需把当前会话 sid 写进聚合行，Task 8 可在 worker 投 `TelemetryMsg::Event(format!("session {sid}"))`，collector 暂不强依赖。本轮 `sid_str` 返回构造串即可，不阻塞。

- [ ] **Step 4: 编译确认通过**

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过（无 error；`run_collector`/`dump_ring` 暂未被调用会有 dead_code warning，Task 10 接线后消除）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/telemetry.rs
git commit -m "feat(client): telemetry 异步包装(TelemetryMsg/format_log/dump_ring/run_collector)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 7：capture.rs 拆截屏 + 暴露画质档位

**Files:**
- Modify: `src/client/src/capture.rs:122`（`Capturer` impl 内加 `capture_raw`）
- Modify: `src/client/src/capture.rs:33`（加 `quality_u8`）

- [ ] **Step 1: 加 capture_raw + quality_u8**

修改 `src/client/src/capture.rs`，在 `frame_q`（`capture.rs:118-121`）之后、`impl Capturer` 闭合 `}`（第 122 行）之前插入：

```rust
    /// 截一帧原始 RGBA（不缩放不编码），供变化检测先行（spec §3.1）。
    pub fn capture_raw(&self) -> anyhow::Result<image::RgbaImage> {
        Ok(self.mon.capture_image()?)
    }
```

在 `set_quality`（`capture.rs:27-33`）之后插入读取当前档位 u8 的辅助：

```rust
/// 当前画质档位原子值（0=流畅,1=高清），供推帧线程做 quality_changed 判断。
pub fn quality_u8() -> u8 {
    QUALITY.load(Ordering::Relaxed)
}
```

- [ ] **Step 2: 编译确认通过**

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过（capture_raw/quality_u8 暂未被调用会 dead_code warning，Task 8 消除）。

- [ ] **Step 3: 提交**

```bash
git add src/client/src/capture.rs
git commit -m "feat(client): capture_raw 拆截屏与编码 + quality_u8 暴露画质档位

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 8：workers 推帧线程集成（模式分支 + 跳过决策 + 投 FrameSample）

**Files:**
- Modify: `src/client/src/workers.rs:249-362`（`consume_capture` 签名 + 推帧线程闭包）

- [ ] **Step 1: 改 consume_capture 签名加 telemetry_tx**

把 `src/client/src/workers.rs:249-252` 的函数签名：

```rust
pub async fn consume_capture(
    mut ctrl_rx: tokio::sync::mpsc::UnboundedReceiver<net::CaptureCtrl>,
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
) {
```

改为（加 `telemetry_tx` 参数）：

```rust
pub async fn consume_capture(
    mut ctrl_rx: tokio::sync::mpsc::UnboundedReceiver<net::CaptureCtrl>,
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    telemetry_tx: tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
```

- [ ] **Step 2: 替换推帧线程闭包**

把 `src/client/src/workers.rs:256-349`（从 `{` 起的 `let active = active.clone(); std::thread::spawn(move || { ... });` 整块）替换为下面版本（保留 fake/wayland/懒构造原逻辑，新增模式分支 + 跳过决策 + 遥测）。`telemetry_tx` 需在 spawn 前 clone 进闭包：

```rust
    {
        let active = active.clone();
        let telemetry_tx = telemetry_tx.clone();
        std::thread::spawn(move || {
            let fake = capture::fake_capture_enabled();
            let mut capturer: Option<capture::Capturer> = None;
            let mut seq: u64 = 0;
            let mut last_sent_seq: u64 = 0;
            let mut skip = crate::framediff::SkipState::default();
            let mut notified_for: Option<String> = None;
            let mut last_cap_ms: u64 = 0;
            const TICK_MS: u64 = 16;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                let mode = crate::render_mode::current_mode();
                let qp = crate::render_mode::clamp_params(capture::current_params(), mode);
                let now = now_ms();
                let input_driven = last_input_after(last_cap_ms);
                // 空闲降采（spec §3.5）：连续静止且无近期输入时放宽截帧间隔。
                let eff_interval =
                    crate::framediff::relaxed_interval(skip.consecutive_skips, qp.interval_ms, input_driven);
                let due = now.saturating_sub(last_cap_ms) >= eff_interval;
                if !due && !input_driven {
                    continue;
                }
                last_cap_ms = now;
                let sid = match active.lock().unwrap().clone() {
                    Some(s) => s,
                    None => continue,
                };

                // fake 模式：占位帧走旧路径（dev 验链路，不做 skip/telemetry）。
                if fake {
                    seq += 1;
                    if let Ok((data, w, h)) = capture::placeholder_frame(seq) {
                        if from_ui_tx
                            .send(net::FromUi::Frame { session_id: sid, data, w, h, seq })
                            .is_err()
                        {
                            break;
                        }
                    }
                    continue;
                }

                // Wayland 无法截屏：回执并停推（原逻辑）。
                if capture::is_wayland_session() {
                    if notified_for.as_deref() != Some(sid.as_str()) {
                        tracing::warn!("Wayland 会话无法截屏，已通知主控端；请切换 X11（UKUI 兼容）会话");
                        let _ = from_ui_tx.send(net::FromUi::Notice {
                            session_id: sid.clone(),
                            text: "被控端为 Wayland 会话，无法截屏。请在登录界面切换到 X11（UKUI 兼容）会话后重新连接。".into(),
                        });
                        notified_for = Some(sid.clone());
                    }
                    *active.lock().unwrap() = None;
                    continue;
                }

                // 懒构造截屏器（原逻辑）。
                if capturer.is_none() {
                    match capture::Capturer::new() {
                        Ok(c) => {
                            let (cw, ch) = c.real_size();
                            tracing::info!("被控截屏器就绪 抓屏分辨率={cw}x{ch}");
                            capturer = Some(c);
                        }
                        Err(e) => {
                            if notified_for.as_deref() != Some(sid.as_str()) {
                                let _ = from_ui_tx.send(net::FromUi::Notice {
                                    session_id: sid.clone(),
                                    text: format!("被控端截屏不可用：{e}。请确认在 X11 桌面会话下运行。"),
                                });
                                notified_for = Some(sid.clone());
                            }
                            tracing::warn!("截屏器构造失败（无显示器/X11？）：{e}，推帧禁用；WSLg 可设 OHMYDESK_FAKE_CAPTURE=1 验链路");
                            *active.lock().unwrap() = None;
                            continue;
                        }
                    }
                }
                let cap = capturer.as_ref().unwrap();

                // ── legacy-full-frame：精确旧路径，直接 frame_q，不经 capture_raw/哈希/遥测 ──
                if mode == crate::render_mode::RenderMode::LegacyFullFrame {
                    match cap.frame_q(&qp) {
                        Ok((data, w, h)) => {
                            seq += 1;
                            if from_ui_tx
                                .send(net::FromUi::Frame { session_id: sid, data, w, h, seq })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => tracing::debug!("截帧失败：{e}"),
                    }
                    continue;
                }

                // ── 新路径：capture_raw → 瓦片哈希 → 决策 → (跳过 | 编码发送) + 遥测 ──
                let t_cap = now_ms();
                let raw = match cap.capture_raw() {
                    Ok(img) => img,
                    Err(e) => {
                        tracing::debug!("capture_raw 失败：{e}");
                        let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Event(format!("capture_fail {e}")));
                        continue;
                    }
                };
                let capture_ms = now_ms().saturating_sub(t_cap) as u32;
                let (rw, rh) = (raw.width(), raw.height());
                let (_c, _r, cur_tiles) = crate::framediff::tile_hashes(&raw, 64);
                let quality = capture::quality_u8();
                let frameskip = crate::render_mode::frameskip_on();
                let tele_on = crate::render_mode::telemetry_on();
                let d = skip.decide(now_ms(), cur_tiles, quality, &sid, frameskip);

                if !d.send {
                    if tele_on {
                        let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Frame(crate::telemetry::FrameSample {
                            ts_ms: now_ms(),
                            seq: last_sent_seq,
                            capture_ms,
                            skipped: true,
                            dirty_ratio: d.dirty_ratio,
                            keyframe_forced: false,
                            encode_ms: 0,
                            encoded_bytes: 0,
                            w: rw,
                            h: rh,
                        }));
                    }
                    continue;
                }

                // 发送：整帧编码（与旧路径同款 encode_frame_q）。
                let t_enc = now_ms();
                match capture::encode_frame_q(&raw, qp.max_w, qp.max_h, qp.jpeg_q) {
                    Ok((data, w, h)) => {
                        let encode_ms = now_ms().saturating_sub(t_enc) as u32;
                        let encoded_bytes = data.len(); // base64 长度=上网字节(JSON 内即此串)
                        seq += 1;
                        last_sent_seq = seq;
                        if tele_on {
                            let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Frame(crate::telemetry::FrameSample {
                                ts_ms: now_ms(),
                                seq,
                                capture_ms,
                                skipped: false,
                                dirty_ratio: d.dirty_ratio,
                                keyframe_forced: d.keyframe_forced,
                                encode_ms,
                                encoded_bytes,
                                w,
                                h,
                            }));
                        }
                        if from_ui_tx
                            .send(net::FromUi::Frame { session_id: sid, data, w, h, seq })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => tracing::debug!("编码失败：{e}"),
                }
            }
        });
    }
```

- [ ] **Step 3: 编译确认通过**

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译报错——`consume_capture` 调用方（`main.rs:129`）参数不匹配。这是预期的，Task 10 接线修复。**先只确认 workers.rs 自身无语法/类型错**：`cargo build -p client 2>&1 | grep -A3 "workers.rs"` 应无 workers.rs 内错误（仅 main.rs 调用处报参数数量）。

- [ ] **Step 4: 提交**

```bash
git add src/client/src/workers.rs
git commit -m "feat(client): workers 推帧线程集成模式分支+跳过决策+投 FrameSample

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 9：conn.rs 出网埋点（frame watch 携 seq + EgressSample）

**Files:**
- Modify: `src/client/src/net/conn.rs:25-67`（connect_once 签名 + 出站泵）
- Modify: `src/client/src/net/conn.rs:155-164`（帧入 watch 携 seq）
- Modify: `src/client/src/net/mod.rs:201-217`（run 签名 + reconnect 事件）

- [ ] **Step 1: connect_once 与 run 透传 telemetry_tx**

修改 `src/client/src/net/mod.rs` 的 `run`（`mod.rs:201-217`）签名，加 `telemetry_tx`：

```rust
pub async fn run(
    server_url: String,
    info: EndpointInfo,
    to_ui: mpsc::UnboundedSender<ToUi>,
    mut from_ui: mpsc::UnboundedReceiver<FromUi>,
    telemetry_tx: mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
```

并把循环体改为透传 + 重连事件：

```rust
    let password = std::sync::Arc::new(std::sync::Mutex::new(format!("{:06}", rand_6())));
    loop {
        match conn::connect_once(&server_url, &info, &password, &to_ui, &mut from_ui, &telemetry_tx).await {
            Ok(()) => tracing::warn!("连接正常关闭，3s 后重连"),
            Err(e) => tracing::warn!("连接异常：{e}，3s 后重连"),
        }
        let _ = to_ui.send(ToUi::Disconnected);
        let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Event("reconnect".into()));
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
```

修改 `src/client/src/net/conn.rs` 的 `connect_once`（`conn.rs:25-31`）签名，加末参：

```rust
pub(super) async fn connect_once(
    server_url: &str,
    info: &EndpointInfo,
    password: &Arc<std::sync::Mutex<String>>,
    to_ui: &mpsc::UnboundedSender<ToUi>,
    from_ui: &mut mpsc::UnboundedReceiver<FromUi>,
    telemetry_tx: &mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) -> anyhow::Result<()> {
```

- [ ] **Step 2: frame watch 改携 (seq, String) + 泵测 send_stall**

把 `conn.rs:39` 的 watch 类型：

```rust
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<String>>(None);
```

改为携带 seq：

```rust
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<(u64, String)>>(None);
```

把出站泵的帧分支（`conn.rs:52-64`）替换为（测 flush 耗时 + 投 EgressSample）：

```rust
    let tele_pump = telemetry_tx.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                ctrl = out_rx.recv() => {
                    match ctrl {
                        Some(text) => {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        None => break,
                    }
                }
                changed = frame_rx.changed() => {
                    if changed.is_err() {
                        while let Some(text) = out_rx.recv().await {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        break;
                    }
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some((seq, text)) = latest {
                        let t0 = std::time::Instant::now();
                        let res = write.send(WsMsg::Text(text)).await;
                        let stall = t0.elapsed().as_millis() as u32;
                        let ws_error = res.is_err();
                        let _ = tele_pump.send(crate::telemetry::TelemetryMsg::Egress(crate::telemetry::EgressSample {
                            seq, send_stall_ms: stall, sent_ok: !ws_error, ws_error,
                        }));
                        if ws_error { break; }
                    }
                }
            }
        }
    });
```

- [ ] **Step 3: 帧入 watch 携带 seq**

把 `conn.rs:155-164` 的帧上行分支：

```rust
                    Some(FromUi::Frame { session_id, data, w, h, seq }) => {
                        let env = Envelope {
                            from: id.clone(),
                            to: None,
                            ts: now(),
                            payload: Message::Frame { session_id, data, w, h, seq },
                        };
                        if let Ok(s) = serde_json::to_string(&env) {
                            let _ = frame_tx.send_replace(Some(s));
                        }
                    }
```

改为把 seq 一并塞进 watch：

```rust
                    Some(FromUi::Frame { session_id, data, w, h, seq }) => {
                        let env = Envelope {
                            from: id.clone(),
                            to: None,
                            ts: now(),
                            payload: Message::Frame { session_id, data, w, h, seq },
                        };
                        if let Ok(s) = serde_json::to_string(&env) {
                            let _ = frame_tx.send_replace(Some((seq, s)));
                        }
                    }
```

- [ ] **Step 4: 编译确认通过（仅 net 层，main 调用待 Task 10）**

Run: `cargo build -p client 2>&1 | grep -E "conn.rs|mod.rs" | head`
Expected: net 层（conn.rs/mod.rs）无错误；仅 main.rs 处 `net::run` 调用参数不足报错（Task 10 修）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/net/conn.rs src/client/src/net/mod.rs
git commit -m "feat(client): conn 出网埋点(frame watch 携 seq + EgressSample send_stall) + reconnect 事件

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 10：main.rs 接线 + 默认滚动日志 + 模式初始化

**Files:**
- Modify: `src/client/Cargo.toml`（加 `tracing-appender`/`toml`/`directories` 已有）
- Modify: `src/client/src/main.rs:41-55`（默认滚动日志）
- Modify: `src/client/src/main.rs:83-131`（telemetry 通道 + collector spawn + 模式初始化 + 传参）

- [ ] **Step 1: 加依赖**

修改 `src/client/Cargo.toml`，在 `tracing-subscriber = { ... }`（第 31 行）下加：

```toml
tracing-appender = "0.2"                                # 默认滚动日志（按天滚动，无需环境变量）
toml = "0.8"                                            # 读 config.toml 的 [render] mode
```

- [ ] **Step 2: 默认滚动日志（保留 OHMYDESK_LOG_FILE 兼容）**

把 `src/client/src/main.rs:41-55` 整段日志初始化替换为：

```rust
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "client=info".into());
    // 默认滚动日志（spec §4.2 硬性前置）：无需任何环境变量即落盘，按天滚动。
    // 目录：%APPDATA%/OhMyDesk/logs（Win）、~/.local/state/ohmydesk/logs（Linux）。
    // OHMYDESK_LOG_FILE 仍兼容（显式指定单文件时优先）。
    let _log_guard = match std::env::var("OHMYDESK_LOG_FILE") {
        Ok(p) if !p.is_empty() => {
            match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                Ok(file) => {
                    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false)
                        .with_writer(std::sync::Mutex::new(file)).init();
                    None
                }
                Err(_) => { tracing_subscriber::fmt().with_env_filter(filter).init(); None }
            }
        }
        _ => {
            let log_dir = ohmydesk_state_dir().join("logs");
            let _ = std::fs::create_dir_all(&log_dir);
            let appender = tracing_appender::rolling::daily(&log_dir, "client.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false)
                .with_writer(nb).init();
            Some(guard)
        }
    };
```

在 `main.rs` 文件内（`main` 函数外，靠近文件末尾的辅助函数区）加状态目录解析（复用已有 `directories` crate）：

```rust
/// 跨平台状态目录：Win=%APPDATA%/OhMyDesk，Linux=~/.local/state/ohmydesk。
fn ohmydesk_state_dir() -> std::path::PathBuf {
    if let Some(pd) = directories::ProjectDirs::from("", "", "OhMyDesk") {
        #[cfg(windows)]
        { return pd.data_dir().to_path_buf(); }
        #[cfg(not(windows))]
        { return pd.state_dir().map(|p| p.to_path_buf()).unwrap_or_else(|| pd.data_local_dir().to_path_buf()); }
    }
    std::env::temp_dir().join("ohmydesk")
}
```

> 注：`_log_guard` 必须在 `main` 全程存活（非阻塞 appender 的 worker 靠它），故绑定到 `main` 作用域变量、勿 `_` 丢弃后立即 drop。已用 `let _log_guard = ...` 持有。

- [ ] **Step 3: 模式初始化 + telemetry 通道 + collector spawn + 传参**

在 `main.rs:92-93`（`cap_tx/cap_rx` 创建附近）之后、worker spawn 之前，加 telemetry 通道与模式初始化：

```rust
    // 遥测通道（worker 投 FrameSample、conn 投 EgressSample）
    let (tele_tx, tele_rx) = tokio::sync::mpsc::unbounded_channel::<telemetry::TelemetryMsg>();
    // 运行模式初始化（env > 启动参数 > config.toml > 默认 Frameskip）
    let arg_mode = std::env::args()
        .find_map(|a| a.strip_prefix("--render-mode=").map(|s| s.to_string()));
    let cfg_mode = std::fs::read_to_string(ohmydesk_state_dir().join("config.toml"))
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("render").and_then(|r| r.get("mode")).and_then(|m| m.as_str().map(|s| s.to_string())));
    let env_mode = std::env::var("OHMYDESK_RENDER_MODE").ok();
    let mode = render_mode::resolve(env_mode.as_deref(), arg_mode.as_deref(), cfg_mode.as_deref());
    render_mode::apply(mode);
    // 单开关环境变量覆盖（最高优先级）
    if std::env::var("OHMYDESK_FRAMESKIP").as_deref() == Ok("0") {
        render_mode::set_frameskip(false);
    }
    if std::env::var("OHMYDESK_DIRTY_TELEMETRY").as_deref() == Ok("0") {
        render_mode::set_telemetry(false);
    }
    tracing::info!("渲染模式 mode={:?} frameskip={} telemetry={}", render_mode::current_mode(), render_mode::frameskip_on(), render_mode::telemetry_on());
```

`render_mode` 加两个单开关覆盖函数（在 `render_mode.rs` 的 `apply` 之后）：

```rust
pub fn set_frameskip(on: bool) {
    FRAMESKIP_ON.store(on, Ordering::Relaxed);
}
pub fn set_telemetry(on: bool) {
    TELEMETRY_ON.store(on, Ordering::Relaxed);
}
```

把 worker spawn（`main.rs:118` 与 `129`）改为传 `tele_tx`，并 spawn collector。具体：`net::run` 调用（`main.rs:118`）改为：

```rust
    rt.spawn(net::run(server_url, info, to_ui_tx, from_ui_rx, tele_tx.clone()));
```

`consume_capture`（`main.rs:129`）改为：

```rust
    rt.spawn(workers::consume_capture(cap_rx, from_ui_tx.clone(), tele_tx.clone()));
```

在四个 worker spawn 之后加 collector spawn：

```rust
    rt.spawn(telemetry::run_collector(tele_rx, ohmydesk_state_dir().join("diag")));
```

- [ ] **Step 4: 全量编译 + 单测**

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过（0 error）。

Run: `cargo test -p client 2>&1 | tail -15`
Expected: 全部单测通过（framediff 10 + telemetry 12 + render_mode 6 + 既有 capture 测试）。

- [ ] **Step 5: 运行冒烟（验证默认日志落盘）**

Run: `timeout 3 cargo run -p client 2>&1 | head -5 || true`，然后检查日志目录：
Run: `ls -la ~/.local/state/ohmydesk/logs/ 2>/dev/null || ls -la ~/.local/share/OhMyDesk/logs/ 2>/dev/null`
Expected: 出现 `client.log.YYYY-MM-DD` 滚动文件（无需设任何环境变量）。

> 若 GUI 在无显示器 CI 环境起不来，本步在有桌面的开发机执行；CI 上以 `cargo build` + 单测为门。

- [ ] **Step 6: 提交**

```bash
git add src/client/Cargo.toml src/client/src/main.rs src/client/src/render_mode.rs
git commit -m "feat(client): main 接线遥测collector+默认滚动日志+模式初始化(env/arg/config)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

# 阶段三：跨端取证（server + 主控端纯日志）

## Task 11：server frame_lane_drop（Δenqueued−Δsent，纯日志）

**Files:**
- Modify: `src/server/src/hub.rs:17-60`（frame_clients 值改 `FrameClient` 含 enqueued 计数 + `frame_lane_drop` 纯函数）
- Modify: `src/server/src/main.rs:200-265`（pump 累加 sent + 周期 debug；登记点传 enqueued）

- [ ] **Step 1: 写 frame_lane_drop 纯函数失败测试**

在 `src/server/src/hub.rs` 的 `#[cfg(test)] mod tests` 内追加测试（与现有 7 个测试同模块）：

```rust
    #[test]
    fn frame_lane_drop_计算() {
        assert_eq!(super::frame_lane_drop(5, 3), 2, "入5发3→丢2");
        assert_eq!(super::frame_lane_drop(3, 3), 0, "1:1→不丢(不过报)");
        assert_eq!(super::frame_lane_drop(2, 5), 0, "sent>enqueued(并发瞬态)→clamp 0");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server hub::frame_lane_drop`
Expected: FAIL（`frame_lane_drop` 未定义）。

- [ ] **Step 3: 改 hub.rs：FrameClient 结构 + 计数 + 纯函数**

把 `src/server/src/hub.rs:23-24` 的字段：

```rust
    /// 帧专用 lane（drop-stale）：endpoint_id/admin_id → 单槽最新帧 watch。与 clients 并存（附加式）。
    frame_clients: DashMap<String, tokio::sync::watch::Sender<Option<String>>>,
```

改为携 enqueued 计数：

```rust
    /// 帧专用 lane（drop-stale）：endpoint_id/admin_id → 帧 watch + enqueued 计数（与 clients 并存）。
    frame_clients: DashMap<String, FrameClient>,
```

在 `pub struct Hub { ... }` 之前（`hub.rs:16` 附近）加 `FrameClient` 定义与纯函数：

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// 帧 lane 客户端：watch 发送端 + 入队计数（enqueued）。sent 由对应连接出站泵持有。
pub struct FrameClient {
    pub tx: tokio::sync::watch::Sender<Option<String>>,
    pub enqueued: std::sync::Arc<AtomicU64>,
}

/// frame_lane_drop = 入队 − 实发（clamp≥0），不数 send_replace 覆盖（防过报，spec §4.1 HIGH②）。
pub fn frame_lane_drop(enqueued: u64, sent: u64) -> u64 {
    enqueued.saturating_sub(sent)
}
```

改 `add_frame_client`（`hub.rs:42-48`）签名收 enqueued：

```rust
    pub fn add_frame_client(
        &self,
        id: String,
        frame_tx: tokio::sync::watch::Sender<Option<String>>,
        enqueued: std::sync::Arc<AtomicU64>,
    ) {
        self.frame_clients.insert(id, FrameClient { tx: frame_tx, enqueued });
    }
```

改 `send_frame_to`（`hub.rs:51-55`）累加 enqueued：

```rust
    /// 帧定向推送（drop-stale）：覆盖目标的单槽最新帧；累加 enqueued（入队计数）。
    pub fn send_frame_to(&self, id: &str, json: &str) {
        if let Some(fc) = self.frame_clients.get(id) {
            fc.enqueued.fetch_add(1, Ordering::Relaxed);
            let _ = fc.tx.send_replace(Some(json.to_string()));
        }
    }
```

- [ ] **Step 4: 改 server main.rs：pump 累加 sent + 周期日志；登记点传 enqueued**

在 `src/server/src/main.rs:203`（frame watch 创建）之后加两个计数器：

```rust
    let frame_enqueued = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let frame_sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
```

把 pump 帧写出分支（`src/server/src/main.rs:225-228`）：

```rust
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some(s) = latest {
                        if sink.send(WsMsg::Text(s)).await.is_err() { break; }
                    }
```

替换为（累加 sent + 每 100 帧 debug 一次 drop）：

```rust
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some(s) = latest {
                        if sink.send(WsMsg::Text(s)).await.is_err() { break; }
                        let n = frame_sent_pump.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if n % 100 == 0 {
                            let enq = frame_enqueued_pump.load(std::sync::atomic::Ordering::Relaxed);
                            tracing::debug!("frame_lane_drop enqueued={enq} sent={n} drop={}", hub::frame_lane_drop(enq, n));
                        }
                    }
```

在 pump spawn 之前 clone 两计数器进闭包（`main.rs:206` `let pump = tokio::spawn(async move {` 之前）：

```rust
    let frame_sent_pump = frame_sent.clone();
    let frame_enqueued_pump = frame_enqueued.clone();
```

把 `add_frame_client` 登记点（`main.rs:263-265`）改为传 enqueued：

```rust
            if let Some(ftx) = frame_tx.take() {
                hub.add_frame_client(id.clone(), ftx, frame_enqueued.clone());
            }
```

> 注：`hub::frame_lane_drop` 与 `hub` 模块需在 main.rs 可见（既有 `use` 或 `hub::` 路径）。若 main.rs 已 `mod hub;`/`use crate::hub;`，按现状路径引用即可。

- [ ] **Step 5: 跑测试 + 编译确认通过**

Run: `cargo test -p server hub:: 2>&1 | tail -5`
Expected: PASS（既有 7 + 新增 frame_lane_drop 测试全绿）。

Run: `cargo build -p server 2>&1 | tail -5`
Expected: 编译通过（0 error）。

- [ ] **Step 6: 提交**

```bash
git add src/server/src/hub.rs src/server/src/main.rs
git commit -m "feat(server): frame_lane_drop 计数(Δenqueued−Δsent,纯日志,relay→主控段证据)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 12：主控端保留 seq（ToUi::Frame 加字段，类型贯通）

**Files:**
- Modify: `src/client/src/net/mod.rs:53-58`（`ToUi::Frame` 加 `seq`）
- Modify: `src/client/src/net/dispatch.rs:165-178`（`..`→`seq`，透传）
- Modify: `src/client/src/ui_glue.rs:635-640`（Frame 臂头加 `seq`）

- [ ] **Step 1: ToUi::Frame 加 seq 字段**

把 `src/client/src/net/mod.rs:53-58`：

```rust
    Frame {
        session_id: String,
        data: String,
        w: u32,
        h: u32,
    },
```

改为：

```rust
    Frame {
        session_id: String,
        data: String,
        w: u32,
        h: u32,
        seq: u64,
    },
```

- [ ] **Step 2: dispatch.rs 保留 seq 并透传**

把 `src/client/src/net/dispatch.rs:165-178`：

```rust
        Message::Frame {
            session_id,
            data,
            w,
            h,
            ..
        } => {
            let _ = to_ui.send(ToUi::Frame {
                session_id,
                data,
                w,
                h,
            });
        }
```

改为（不再 `..` 丢 seq）：

```rust
        Message::Frame {
            session_id,
            data,
            w,
            h,
            seq,
        } => {
            let _ = to_ui.send(ToUi::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            });
        }
```

- [ ] **Step 3: ui_glue.rs Frame 臂头解构 seq**

把 `src/client/src/ui_glue.rs:635-640`：

```rust
            net::ToUi::Frame {
                session_id,
                data,
                w,
                h,
            } => {
```

改为：

```rust
            net::ToUi::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            } => {
```

- [ ] **Step 4: 编译确认贯通**

Run: `cargo build -p client 2>&1 | tail -8`
Expected: 编译通过（`seq` 在 ui_glue Frame 臂内暂未使用会有 unused warning，Task 13 用上）。其它 `ToUi::Frame` 构造处若有遗漏会报「缺字段 seq」——全仓仅 dispatch.rs 一处构造，已改。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/net/mod.rs src/client/src/net/dispatch.rs src/client/src/ui_glue.rs
git commit -m "feat(client): ToUi::Frame 保留 seq(主控端 seq_gap 取证地基,去掉 dispatch .. 丢弃)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 13：主控端收帧埋点（recv_fps/decode_ms/drop_stale/seq_gap，纯日志）

**Files:**
- Modify: `src/client/src/telemetry.rs`（加 `MainRecvStats` + 测试）
- Modify: `src/client/src/ui_glue.rs:543-551`（drop-stale 计数）、`632-662`（decode 计时 + 喂 stats）

- [ ] **Step 1: 写 MainRecvStats 失败测试**

在 `src/client/src/telemetry.rs` 的 `run_collector` 之后追加：

```rust
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
        MainRecvStats { window_start_ms: 0, frames: 0, decode_ms_sum: 0, drop_stale: 0, last_seq: None, seq_gap: 0 }
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
            let decode_avg = if self.frames > 0 { self.decode_ms_sum / self.frames as u64 } else { 0 };
            let line = format!(
                "主控遥测 recv_fps={:.1} decode_avg_ms={} drop_stale={} seq_gap={}",
                self.frames as f32 / 10.0, decode_avg, self.drop_stale, self.seq_gap
            );
            let keep_seq = self.last_seq;
            *self = MainRecvStats::default();
            self.last_seq = keep_seq;
            return Some(line);
        }
        None
    }
}
```

在 `mod tests` 内追加：

```rust
    #[test]
    fn main_recv_seq_gap累计() {
        let mut s = MainRecvStats::default();
        assert_eq!(s.on_frame(1, 20, 1000), None);
        assert_eq!(s.on_frame(2, 20, 1100), None); // 连续，无 gap
        assert_eq!(s.on_frame(5, 20, 1200), None); // 跳 3,4 → gap+2
        // 窗满触发日志
        let line = s.on_frame(6, 20, 12_000).expect("窗满应出日志");
        assert!(line.contains("seq_gap=2"), "缺 3、4 两帧 → seq_gap=2: {line}");
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
```

- [ ] **Step 2: 跑测试确认通过**

Run: `cargo test -p client telemetry::main_recv`
Expected: PASS（MainRecvStats 已给完整实现）。

- [ ] **Step 3: 在 consume_to_ui 接入**

在 `src/client/src/ui_glue.rs:543` `while let Some(mut ev)` **之前**声明 stats（约 542 行）：

```rust
    let mut recv_stats = crate::telemetry::MainRecvStats::default();
```

把 drop-stale drain（`ui_glue.rs:546-551`）改为计数被丢弃帧：

```rust
        let mut dropped = 0u32;
        while matches!(ev, net::ToUi::Frame { .. }) {
            match rx.try_recv() {
                Ok(next) => { ev = next; dropped += 1; }
                Err(_) => break,
            }
        }
        if dropped > 0 {
            recv_stats.on_drop_stale(dropped);
        }
```

在 Frame 臂内 decode 处（`ui_glue.rs:662` `if let Ok((rgba, iw, ih)) = decode_frame_rgba(&data)`）外层加计时并喂 stats。把该行附近改为：

```rust
                let t_dec = std::time::Instant::now();
                let decoded = decode_frame_rgba(&data);
                let decode_ms = t_dec.elapsed().as_millis() as u32;
                if let Some(line) = recv_stats.on_frame(seq, decode_ms, now_ms_ui()) {
                    tracing::info!("{line}");
                }
                if let Ok((rgba, iw, ih)) = decoded {
```

> 把原 `if let Ok((rgba, iw, ih)) = decode_frame_rgba(&data) {` 一行替换为上面 5 行（decode 提前到 `decoded`，原 `{` 闭合不变）。

在 `ui_glue.rs` 文件内加一个毫秒时钟辅助（若文件内已有等价函数则复用，勿重复定义）：

```rust
fn now_ms_ui() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
```

- [ ] **Step 4: 编译 + 单测**

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过（0 error；`seq` 现已被 `recv_stats.on_frame(seq, ...)` 使用，Task 12 的 unused warning 消除）。

Run: `cargo test -p client telemetry:: 2>&1 | tail -5`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/telemetry.rs src/client/src/ui_glue.rs
git commit -m "feat(client): 主控端收帧埋点(recv_fps/decode_ms/drop_stale/seq_gap,纯日志)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 14：UI 隐藏诊断菜单（模式热切 + 导出诊断包）

**Files:**
- Modify: `src/client/src/telemetry.rs`（`TelemetryMsg::ExportNow`；run_collector 处理）
- Modify: `src/client/ui/app.slint`（隐藏入口 + 诊断模态 + 3 callback/property）
- Modify: `src/client/src/ui_glue.rs:112-119`（wire_ui_callbacks 加 tele_tx 参 + 注册 3 回调）
- Modify: `src/client/src/main.rs`（wire_ui_callbacks 调用处传 tele_tx + set_diag_dir）

- [ ] **Step 1: telemetry 加 ExportNow**

在 `src/client/src/telemetry.rs` 的 `TelemetryMsg` 枚举（Task 6）加一个变体：

```rust
pub enum TelemetryMsg {
    Frame(FrameSample),
    Egress(EgressSample),
    Event(String),
    ExportNow, // UI 手动导出：立即 dump 环形缓冲（忽略去抖）
}
```

在 `run_collector` 的 `match msg` 内加分支（与 Frame/Egress 并列）：

```rust
                Some(TelemetryMsg::ExportNow) => {
                    let now = win_frames.last().map(|f| f.ts_ms)
                        .or_else(|| collector.ring_iter().last().map(|m| m.frame.ts_ms))
                        .unwrap_or(0);
                    match dump_ring(&collector, &diag_dir, now) {
                        Ok(p) => tracing::warn!("手动导出诊断包 {}", p.display()),
                        Err(e) => tracing::warn!("手动导出失败 {e}"),
                    }
                }
```

- [ ] **Step 2: app.slint 加隐藏入口 + 诊断模态 + 声明**

在 `app.slint` 的 AppWindow property 区（仿 `488-491` 样式，约 533 行附近）加：

```slint
    in property <string> diag_dir;                   // 诊断包目录（Rust 启动时 set）
    in-out property <bool> diag_visible: false;      // 诊断面板可见
    in-out property <int> diag_taps: 0;              // Header logo 连点计数（≥5 开面板）
    callback set_render_mode(string);                // 切运行模式（legacy-full-frame/frameskip…）
    callback export_diag();                          // 导出诊断包（发 ExportNow）
    callback copy_diag_path();                       // 复制诊断目录路径到剪贴板
```

给 Header logo 块（`746-779`，现无 TouchArea）加连点入口——在该 `Rectangle`/布局内追加：

```slint
        TouchArea {
            clicked => {
                root.diag_taps += 1;
                if (root.diag_taps >= 5) {
                    root.diag_visible = true;
                    root.diag_taps = 0;
                }
            }
        }
```

在文件顶层结构区（仿授权弹窗 `1435-1544` 模态范式，放在其后）加诊断模态：

```slint
    if root.diag_visible: Rectangle {
        background: #000000bb;
        TouchArea { }  // 吃背景点击
        VerticalLayout {
            alignment: center;
            HorizontalLayout {
                alignment: center;
                Card {
                    width: 420px;
                    background: #18181c;
                    VerticalLayout {
                        padding: 20px;
                        spacing: 12px;
                        SectionHeader { text: "诊断 / 渲染模式（仅排障用）"; }
                        Text { text: "当前画面异常时，可切回旧整帧画面或导出诊断包。"; color: Theme.fg-muted; wrap: word-wrap; }
                        HorizontalLayout {
                            spacing: 8px;
                            GhostButton { text: "切回旧画面"; clicked => { root.set_render_mode("legacy-full-frame"); } }
                            GhostButton { text: "默认(跳帧)"; clicked => { root.set_render_mode("frameskip"); } }
                        }
                        HorizontalLayout {
                            spacing: 8px;
                            PrimaryButton { text: "导出诊断包"; enabled: true; clicked => { root.export_diag(); } }
                            GhostButton { text: "复制路径"; clicked => { root.copy_diag_path(); } }
                        }
                        ValueRow { label: "诊断目录"; value: root.diag_dir; }
                        DangerButton { text: "关闭"; clicked => { root.diag_visible = false; } }
                    }
                }
            }
        }
    }
```

> 若 `SectionHeader`/`ValueRow`/`GhostButton`/`PrimaryButton`/`DangerButton`/`Card` 的具体属性名与上略有出入，以 `app.slint` 顶部组件定义（`Card`@67、`GhostButton`@102、`PrimaryButton`@129、`DangerButton`@156、`SectionHeader`@75、`ValueRow`@223）为准微调（如 `clicked` 回调名、`text`/`label`/`value` 属性名）。

- [ ] **Step 3: ui_glue wire 三回调（含 tele_tx 参）**

把 `wire_ui_callbacks` 签名（`ui_glue.rs:112-119`）加末参：

```rust
pub fn wire_ui_callbacks(
    ui: &AppWindow,
    from_ui_tx: &tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    telemetry_tx: &tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
```

在 `wire_ui_callbacks` 体内（函数末闭合 `}` 前，仿 `502-523` 块作用域惯用法）追加三回调注册：

```rust
    // ── 诊断菜单：模式热切（render_mode 原子，运行期即时生效，无需重启） ──
    ui.on_set_render_mode(move |m| {
        if let Some(mode) = crate::render_mode::parse_mode(&m.to_string()) {
            crate::render_mode::apply(mode);
            tracing::warn!("UI 热切渲染模式 → {:?}", crate::render_mode::current_mode());
        }
    });
    // ── 诊断菜单：导出诊断包（发 ExportNow 给 collector 落盘） ──
    {
        let tele = telemetry_tx.clone();
        ui.on_export_diag(move || {
            let _ = tele.send(crate::telemetry::TelemetryMsg::ExportNow);
        });
    }
    // ── 诊断菜单：复制诊断目录路径（复用剪贴板回调由 UI 内联，已 set diag_dir）──
    {
        let ui_weak = ui.as_weak();
        ui.on_copy_diag_path(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.invoke_copy_text(ui.get_diag_dir());
            }
        });
    }
```

> `invoke_copy_text` 调用既有 `copy_text(string)` callback（app.slint:559，已接剪贴板）。若该 callback 在 Rust 侧由其它处注册（非 UI 内联），`on_copy_diag_path` 可改为直接 `arboard` 写剪贴板；优先复用 `copy_text`。

- [ ] **Step 4: main.rs 传 tele_tx + set_diag_dir**

把 `main.rs:99`（`wire_ui_callbacks(...)` 调用）改为加 `&tele_tx`：

```rust
    ui_glue::wire_ui_callbacks(&ui, &from_ui_tx, &cur_session, &ctrl_session, &ended_session, &activity, &tele_tx);
```

在该调用之前（`ui` 已构造、`tele_tx` 已创建之后）设诊断目录到 UI：

```rust
    ui.set_diag_dir(ohmydesk_state_dir().join("diag").to_string_lossy().to_string().into());
```

> `tele_tx` 在 Task 10 已于 `cap_tx` 附近创建；确保 `wire_ui_callbacks` 调用在 `tele_tx` 创建之后（必要时把 `let (tele_tx, tele_rx) = ...` 上移到 `wire_ui_callbacks` 之前）。

- [ ] **Step 5: 编译确认通过**

Run: `cargo build -p client 2>&1 | tail -8`
Expected: 编译通过（0 error）。Slint 宏会校验 `on_set_render_mode`/`on_export_diag`/`on_copy_diag_path`/`set_diag_dir`/`get_diag_dir` 与 app.slint 声明一致——若报「no method」说明 callback/property 名不匹配，核对第 2 步声明。

- [ ] **Step 6: 手动验证热切（有桌面开发机）**

启动客户端 → Header logo 连点 5 次 → 诊断面板弹出 → 点「切回旧画面」→ 日志出现 `UI 热切渲染模式 → LegacyFullFrame`（无需重启）→ 点「导出诊断包」→ 日志出现 `手动导出诊断包 <path>` 且 diag 目录出现 jsonl。

- [ ] **Step 7: 提交**

```bash
git add src/client/src/telemetry.rs src/client/ui/app.slint src/client/src/ui_glue.rs src/client/src/main.rs
git commit -m "feat(client): UI 隐藏诊断菜单(模式热切+导出诊断包,连点入口+模态)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

## Task 15：admin-web 诊断 ring + 下载按钮（纯日志，脱敏）

**Files:**
- Create: `src/admin-web/src/lib/diag-ring.ts`（纯函数 + 测试）
- Modify: `src/admin-web/src/store.ts:48-104`（State 加 `diagRing`）、`108`（初值）、`147-150`（收帧注入）
- Modify: `src/admin-web/src/components/control/remote-session.tsx`（工具栏加「诊断」下载按钮）

- [ ] **Step 1: 写 diag-ring 纯函数 + 测试**

创建 `src/admin-web/src/lib/diag-ring.ts`：

```typescript
// 主控(admin)侧诊断样本：只含标量指标，绝不含帧像素/base64（脱敏白名单）。
export type DiagSample = {
  ts: number;        // epoch ms
  seq: number;       // 帧序（< 2^53 安全）
  seq_gap: number;   // 相邻帧 seq 差>1 的缺失数
  w: number;
  h: number;
};

// 入环 + 按时间窗裁剪（保留最近 maxAgeMs；满即弹头）。纯函数。
export function pushDiagRing(ring: DiagSample[], s: DiagSample, maxAgeMs: number): DiagSample[] {
  const cutoff = s.ts - maxAgeMs;
  return [...ring, s].filter((x) => x.ts >= cutoff);
}

// 由上一帧 seq 与当前 seq 算 gap（无上一帧或回退时为 0）。
export function seqGap(lastSeq: number | null, seq: number): number {
  return lastSeq != null && seq > lastSeq + 1 ? seq - lastSeq - 1 : 0;
}
```

创建测试 `src/admin-web/src/lib/diag-ring.test.ts`：

```typescript
import { describe, it, expect } from "vitest";
import { pushDiagRing, seqGap, type DiagSample } from "./diag-ring";

const s = (ts: number, seq: number): DiagSample => ({ ts, seq, seq_gap: 0, w: 1280, h: 720 });

describe("diag-ring", () => {
  it("超窗样本被裁剪", () => {
    const ring = [s(1000, 1), s(2000, 2)];
    const next = pushDiagRing(ring, s(1000 + 300_000 + 1, 3), 300_000);
    expect(next.map((x) => x.seq)).toEqual([3], "5min 前的样本应裁掉");
  });
  it("窗内样本保留", () => {
    const next = pushDiagRing([s(1000, 1)], s(2000, 2), 300_000);
    expect(next.length).toBe(2);
  });
  it("seqGap 计算", () => {
    expect(seqGap(null, 1)).toBe(0);
    expect(seqGap(1, 2)).toBe(0);
    expect(seqGap(2, 5)).toBe(2); // 缺 3、4
    expect(seqGap(5, 2)).toBe(0); // 回退不计
  });
});
```

- [ ] **Step 2: 跑测试确认通过**

Run: `pnpm --filter admin-web exec vitest run src/lib/diag-ring.test.ts`
Expected: PASS（3 用例）。

> 若 admin-web 未配置 vitest：以 `pnpm --filter admin-web build`（tsc 类型通过）为门，并在浏览器手动验证（Step 5）。diag-ring.ts 是纯函数，类型通过即逻辑正确度高。

- [ ] **Step 3: store.ts 接入 diagRing**

在 `store.ts` 顶部 import 区加：

```typescript
import { pushDiagRing, seqGap, type DiagSample } from "@/lib/diag-ring";
```

在 `type State = {` 内（`chatMessages` 字段附近，`store.ts:84` 区）加：

```typescript
  // 远控诊断 ring（最近 5min 收帧标量指标，刷新即丢，脱敏不含像素）
  diagRing: DiagSample[];
```

在 store 初值区（`store.ts:115` `remoteFrame: null` 附近）加初值：

```typescript
  diagRing: [],
```

把收帧分支（`store.ts:147-150`）：

```typescript
      if (p.type === "frame") {
        set({ remoteFrame: { data: p.data, w: p.w, h: p.h, seq: p.seq } });
        return;
      }
```

改为（注入诊断 ring）：

```typescript
      if (p.type === "frame") {
        set((s) => {
          const seqNum = Number(p.seq);
          const lastSeq = s.remoteFrame ? Number(s.remoteFrame.seq) : null;
          const sample: DiagSample = { ts: Date.now(), seq: seqNum, seq_gap: seqGap(lastSeq, seqNum), w: p.w, h: p.h };
          return {
            remoteFrame: { data: p.data, w: p.w, h: p.h, seq: p.seq },
            diagRing: pushDiagRing(s.diagRing, sample, 300_000),
          };
        });
        return;
      }
```

- [ ] **Step 4: remote-session.tsx 加下载按钮**

在 `remote-session.tsx` 组件内（`remoteFrame` 选择处附近，约 39 行）加 diagRing 选择 + 下载回调（仿 `saveScreenshot` 62-71）：

```tsx
  const diagRing = useStore((s) => s.diagRing);
  // 下载诊断 JSON：仅标量指标，绝不含帧像素（脱敏）。
  const downloadDiag = useCallback(() => {
    const blob = new Blob([JSON.stringify({ target: targetName, exported_at: Date.now(), samples: diagRing }, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${targetName}-diag-${Date.now()}.json`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(a.href);
  }, [diagRing, targetName]);
```

在工具栏 `tab === "remote"` 片段（`remote-session.tsx:230-241`，与「截图」并列）加按钮（`Download` 图标需在文件顶部 lucide-react import 补上）：

```tsx
              <Button variant="outline" size="sm" onClick={downloadDiag} disabled={diagRing.length === 0}>
                <Download data-icon="inline-start" />
                <span className="hidden sm:inline">诊断</span>
              </Button>
```

在文件顶部图标 import（现有 `Camera`/`Maximize`/`PhoneOff` 等所在行）补 `Download`。

- [ ] **Step 5: 类型检查 + 手动验证**

Run: `pnpm --filter admin-web build 2>&1 | tail -8`
Expected: tsc 类型通过、Vite 构建成功。

手动（开发机有被控端）：远控一台 → 工具栏点「诊断」→ 下载 JSON，打开确认含 `samples`（seq/seq_gap/w/h，**无 data 字段**）。

- [ ] **Step 6: 提交**

```bash
git add src/admin-web/src/lib/diag-ring.ts src/admin-web/src/lib/diag-ring.test.ts src/admin-web/src/store.ts src/admin-web/src/components/control/remote-session.tsx
git commit -m "feat(admin-web): 诊断 ring(5min/脱敏)+下载诊断 JSON 按钮(主控侧 seq_gap 取证)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

# 阶段四：上线门

## Task 16：最小 netem 冒烟门（Go/No-Go，spec §5.1）

**Files:**
- Create: `docs/superpowers/netem-smoke-2026-07.md`（验收记录，跑完填写）

这是上线前**人工验收**，非代码。在一台 Linux 机器上用 `tc netem` 劣化本机出网，跑被控端，主控端（admin-web 或另一台 Slint）连入观察。**frameskip on / off 各跑一遍**。

- [ ] **Step 1: 准备 netem 脚本**

创建 `docs/superpowers/netem-smoke-2026-07.md`，记录以下命令与结果。施加劣化（被控机出网网卡设 `$IF`，如 `eth0`）：

```bash
# 限上行带宽 + 高 RTT（示例 2Mbps + 150ms）
sudo tc qdisc add dev $IF root handle 1: tbf rate 2mbit burst 32kbit latency 400ms
sudo tc qdisc add dev $IF parent 1:1 handle 10: netem delay 150ms
# 清除
sudo tc qdisc del dev $IF root
```

- [ ] **Step 2: 跑矩阵（每格 frameskip on/off 各一遍）**

| 上行带宽 | RTT | 抖动/中断 | frameskip on | frameskip off |
|---|---|---|---|---|
| 1 Mbps | +150ms | — | ☐ | ☐ |
| 2 Mbps | +150ms | — | ☐ | ☐ |
| 5 Mbps | +150ms | 断 5s 再恢复 | ☐ | ☐ |

切换 frameskip：环境变量 `OHMYDESK_FRAMESKIP=0` 起被控端，或 UI 诊断面板热切。

- [ ] **Step 3: 逐项判通过（任一不过 → 不 Go，先修）**

- ☐ ① 各档画面不黑屏/不卡死，断 5s 重连后自愈（≤3s keyframe 周期内恢复）。
- ☐ ② frameskip on 相对 off：静态桌面场景 `sent_Bps` 显著下降、`skip_pct` 高（看被控滚动日志「遥测」行）。
- ☐ ③ 劣化网络下 §4 遥测字段齐全（被控「遥测」行 + 主控「主控遥测」行 + server `frame_lane_drop`），§4.6 段定位可用（能区分被控上行饱和 vs relay→主控）。
- ☐ ④ 无 §4.5 异常被漏报（人为制造卡顿时，被控日志出现 WARN「遥测异常」+ diag 目录落 jsonl）。
- ☐ ⑤ **legacy-full-frame 真回退**：切到该模式，确认画面与改造前一致，且日志**无** capture_raw/遥测「遥测」行（证走精确旧路径）。
- ☐ ⑥ **运行期热切**：劣化网络下经 UI 隐藏菜单 frameskip↔legacy 热切，不重启即生效、画面不中断。
- ☐ ⑦ **诊断包导出**：被控「导出诊断包」与 admin-web「下载诊断 JSON」都产出含 egress(send_stall/egress_drop) + 主控 seq_gap 的有效文件，脱敏白名单生效（无像素/明文）。

- [ ] **Step 4: 记录结论并提交验收文档**

把矩阵勾选 + 关键日志样本 + 结论（Go / No-Go + 待修项）写入 `docs/superpowers/netem-smoke-2026-07.md`，提交：

```bash
git add docs/superpowers/netem-smoke-2026-07.md
git commit -m "test(release): frame-skip 最小 netem 冒烟门验收记录(Go/No-Go)

Claude-Session: https://claude.ai/code/session_01Lj2bGw99rv47nDycWzqQG1"
```

---

# 自检（计划 vs spec）

**Spec 覆盖**：§3.1→T7、§3.2→T1、§3.3/§3.4→T2、§3.5→T2+T8、§3.6→T5+T14、§4.1→T3/T4/T8/T9、§4.2→T3/T6/T10、§4.3→T4/T6/T14/T15、§4.4→T12、§4.5→T3、§4.7→T11/T13/T15、§5.1→T16。§4.6 症状表为参考（字段均已采集，无独立代码）。**全覆盖,无遗漏。**

**类型贯通**：`Decision{send,keyframe_forced,dirty_ratio}`（T2↔T8）、`FrameSample`/`EgressSample`（T3↔T8/T9）、`TelemetryMsg{Frame,Egress,Event,ExportNow}`（T6+T14）、`consume_capture`/`net::run`/`wire_ui_callbacks` 三处签名加 `telemetry_tx`（T8/T9/T14 ↔ T10 调用处）、`ToUi::Frame.seq`（T12 构造↔解构↔T13 使用）—— 均一致。

**已知软依赖（实现时注意,非占位）**：① `Collector.sid` 当前为空串,聚合行 `sid=` 为空（不阻塞,关联靠 seq+ts,见 T6 注）；② T14 Slint 组件属性名以 `app.slint` 顶部组件定义为准微调；③ admin-web 若无 vitest,以 `pnpm build` 类型门 + 手动验证代偿（T15 注）。

**依赖顺序**（执行必须遵守）：T1→T2（同文件）；T3→T4→T6（同文件）；T5、T7 独立；T8 依赖 T1/T2/T5/T6/T7;T9 依赖 T6;T10 依赖 T5/T6/T8/T9;T12→T13;T14 依赖 T6/T10;**T11(server)、T15(admin-web) 跨 crate 独立,可与 client 链并行**;T16 最后,人工。

---

# 执行交接

REQUIRED SUB-SKILL：`superpowers:subagent-driven-development`（每任务派子代理 TDD + 两段审查）。

**并行策略（受依赖约束）**：client crate 内 13 个任务共享 `main.rs`/`telemetry.rs`/`ui_glue.rs`,**必须串行**（并行会文件冲突）;仅 **T11(server)** 与 **T15(admin-web)** 跨 crate 真独立,用 worktree 隔离与 client 链并行。

**上线硬门（不可绕过）**：T16 netem 冒烟门是 spec §5.1 定的 Go/No-Go,需 Linux+真桌面+主控 实机,**人工执行**;且本特性服务一次性信任窗口的客服+公网中转,**「全量发布」必须在 T16 全绿 + 用户显式 Go 之后**,不得在 dev+/simplify 后自动发版。
