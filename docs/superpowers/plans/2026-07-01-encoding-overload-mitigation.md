# 被控端编码过载治理 实现计划（Spec ②）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 治理被控端「编码过载」（老 Xeon 上 `enc_avg_ms≈360ms`、`effective_fps` 极低）——先把编码耗时拆成 resize/jpeg 定位主导项，再用 per-package `opt-level=3` 给图像热路径提速，最后加过载自适应闭环让弱机自动降档自愈。零新 C 依赖、零协议改动。

**Architecture:** 三段按「先证据后修复」推进：①`encode_frame_q` 分段计时 → FrameSample/WindowStats/日志暴露 `resize_ms/jpeg_ms`；②根 `Cargo.toml` 对 `image` 及其 JPEG 编码/缩放依赖设 `opt-level=3`，client 主 crate 仍 `"z"` 保体积；③新增纯状态机 `adaptive.rs`，collector 每 10s 窗 `observe` 写 `LEVEL` 原子量，抓帧线程 `clamp` 折入（以手动档为上限，只降不升，带迟滞与秒关开关）。

**Tech Stack:** Rust（client crate）、tokio（collector）、`image` crate（缩放/JPEG）、cargo profile per-package override。

参考 spec：`docs/superpowers/specs/2026-07-01-encoding-overload-mitigation-design.md`

---

## 文件结构

| 文件 | 职责 | 改动 |
|---|---|---|
| `src/client/src/capture.rs` | `encode_frame_q` 返回分段耗时 `EncodeOut{resize_ms,jpeg_ms}` | 改 `:119,125,135-137,140-160` + 测试 `:283,289` |
| `src/client/src/telemetry.rs` | FrameSample 加 `resize_ms/jpeg_ms`；WindowStats/aggregate/format_log 暴露；run_collector 挂 adaptive | 改结构/聚合/日志/collector |
| `src/client/src/workers.rs` | 填 `resize_ms/jpeg_ms`；抓帧线程套 adaptive clamp | 改 `:271,378-389,401-413` |
| `src/client/src/adaptive.rs` | **新建** 过载自适应纯状态机 + clamp 梯度 + 原子量 + 开关解析 | 新文件 |
| `src/client/src/main.rs` | `mod adaptive`；解析 `OHMYDESK_ADAPTIVE`/config → set_enabled | 改 `:22,127` 附近 |
| `Cargo.toml`（根） | image 热路径 per-package `opt-level=3` | 加 `[profile.release.package.*]` |

**任务顺序**：Task1（分段计时）→ Task2（遥测暴露）→ Task3（构建修+测量）→ Task4（adaptive 纯逻辑）→ Task5（接线）。Task3 与其余解耦，可并行，但按此序叙述。

---

### Task 1：`encode_frame_q` 返回分段耗时

**Files:**
- Modify: `src/client/src/capture.rs:140-160`（返回 `EncodeOut`）、`:119,125,135-137`（3 调用点）、`:283,289`（测试解构）

- [ ] **Step 1：写失败测试**

在 `src/client/src/capture.rs` 的 `#[cfg(test)]` 区加：

```rust
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
```

- [ ] **Step 2：运行确认失败**

Run: `cargo test -p client encode_frame_q_返回分段耗时字段 2>&1 | tail -15`
Expected: FAIL（编译错误：`EncodeOut` 不存在 / `o.resize_ms` 无此字段）。

- [ ] **Step 3：定义 `EncodeOut` 并改 `encode_frame_q`**

在 `src/client/src/capture.rs` `encode_frame_q` 定义上方加结构体：

```rust
/// 编码产出 + 分段耗时（resize 与 jpeg 分开计时，用于定位「编码过载」主导项）。
pub struct EncodeOut {
    pub data: String,
    pub w: u32,
    pub h: u32,
    pub resize_ms: u32,
    pub jpeg_ms: u32,
}
```

把 `encode_frame_q`（`:140-160`）整体替换为：

```rust
pub fn encode_frame_q(
    img: &RgbaImage,
    max_w: u32,
    max_h: u32,
    q: u8,
) -> anyhow::Result<EncodeOut> {
    let (sw, sh) = (img.width(), img.height());
    let (w, h) = scaled_dims(sw, sh, max_w, max_h);

    // 缩放（等比，不放大）。尺寸未变时跳过 resize 省一次拷贝。
    let t_resize = std::time::Instant::now();
    let rgb = if (w, h) == (sw, sh) {
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()
    } else {
        let resized = image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
        image::DynamicImage::ImageRgba8(resized).to_rgb8()
    };
    let resize_ms = t_resize.elapsed().as_millis() as u32;

    let t_jpeg = std::time::Instant::now();
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&rgb)?;
    let data = STANDARD.encode(buf.get_ref());
    let jpeg_ms = t_jpeg.elapsed().as_millis() as u32;

    Ok(EncodeOut { data, w, h, resize_ms, jpeg_ms })
}
```

- [ ] **Step 4：更新 3 个调用点**

- `encode_frame`（`:135-137`）映射回旧三元组：
  ```rust
  pub fn encode_frame(img: &RgbaImage) -> anyhow::Result<(String, u32, u32)> {
      let o = encode_frame_q(img, MAX_W, MAX_H, 85)?;
      Ok((o.data, o.w, o.h))
  }
  ```
- `Capturer::frame_q`（`:123-126`）：
  ```rust
  pub fn frame_q(&self, p: &QualityParams) -> anyhow::Result<(String, u32, u32)> {
      let img = self.mon.capture_image()?;
      let o = encode_frame_q(&img, p.max_w, p.max_h, p.jpeg_q)?;
      Ok((o.data, o.w, o.h))
  }
  ```
- `Capturer::frame`（`:117-120`）不变（它调 `encode_frame`，已在上面映射）。

- [ ] **Step 5：更新测试解构（`:283,289`）**

把两处 `let (_, sw, sh) = encode_frame_q(&img, ...).unwrap();` 改为：
```rust
        let o = encode_frame_q(&img, sp.max_w, sp.max_h, sp.jpeg_q).unwrap();
        let (sw, sh) = (o.w, o.h);
```
（高清档同理，变量名 `hp`/`hw`/`hh` 对应替换。）

- [ ] **Step 6：运行测试确认通过**

Run: `cargo test -p client --lib capture 2>&1 | tail -15`
Expected: PASS（含新测试与既有 capture 测试全过）。

- [ ] **Step 7：Commit**

```bash
git add src/client/src/capture.rs
git commit -m "feat(capture): encode_frame_q 返回 EncodeOut 分段计时(resize_ms/jpeg_ms)"
```

---

### Task 2：遥测暴露 resize/jpeg + format_log 加字段

**Files:**
- Modify: `src/client/src/telemetry.rs`（FrameSample `:6-17`、WindowStats `:28-46`、aggregate `:68-116`、format_log `:250-259`、run_collector `format_log` 调用、既有测试 `fs` helper 与 `format_log含关键字段`）
- Modify: `src/client/src/workers.rs:378-389,401-413`（填充新字段）

- [ ] **Step 1：写失败测试（aggregate 拆分 + 日志字段）**

在 `telemetry.rs` 测试区加：

```rust
    #[test]
    fn aggregate拆分resize_jpeg() {
        let mk = |resize_ms, jpeg_ms| FrameSample {
            ts_ms: 0, seq: 1, capture_ms: 5, skipped: false, dirty_ratio: 0.2,
            keyframe_forced: false, encode_ms: resize_ms + jpeg_ms, resize_ms, jpeg_ms,
            encoded_bytes: 1000, w: 100, h: 100,
        };
        let frames = vec![mk(10, 100), mk(30, 200)];
        let s = aggregate(&frames, &[], 10_000);
        assert_eq!(s.resize_avg_ms, 20, "resize 均值 (10+30)/2");
        assert_eq!(s.jpeg_avg_ms, 150, "jpeg 均值 (100+200)/2");
    }

    #[test]
    fn format_log含resize_jpeg_adapt字段() {
        let s = WindowStats { resize_avg_ms: 12, jpeg_avg_ms: 340, ..Default::default() };
        let out = format_log(&s, "sid1", 2);
        assert!(out.contains("resize_avg_ms=12"), "含 resize_avg_ms");
        assert!(out.contains("jpeg_avg_ms=340"), "含 jpeg_avg_ms");
        assert!(out.contains("adapt_level=2"), "含 adapt_level");
    }
```

- [ ] **Step 2：运行确认失败**

Run: `cargo test -p client --lib telemetry::tests::aggregate拆分resize_jpeg 2>&1 | tail -15`
Expected: FAIL（`FrameSample` 无 `resize_ms/jpeg_ms`、`WindowStats` 无 `resize_avg_ms`、`format_log` 参数不符）。

- [ ] **Step 3：FrameSample 加字段（保留 encode_ms 并存）**

`telemetry.rs:6-17` 在 `pub encode_ms: u32,` 下加两行：
```rust
    pub resize_ms: u32,
    pub jpeg_ms: u32,
```

- [ ] **Step 4：WindowStats 加字段**

`telemetry.rs:28-46` 在 `pub enc_p95_ms: u32,` 附近加：
```rust
    pub resize_avg_ms: u32,
    pub resize_p95_ms: u32,
    pub jpeg_avg_ms: u32,
    pub jpeg_p95_ms: u32,
```

- [ ] **Step 5：aggregate 计算（`:82-115` 内）**

在 `enc_ms` 处理附近加（`sent_frames` 已存在）：
```rust
    let mut resize_v: Vec<u32> = sent_frames.iter().map(|f| f.resize_ms).collect();
    resize_v.sort_unstable();
    let mut jpeg_v: Vec<u32> = sent_frames.iter().map(|f| f.jpeg_ms).collect();
    jpeg_v.sort_unstable();
    let resize_avg_ms = if sent > 0 { resize_v.iter().sum::<u32>() / sent as u32 } else { 0 };
    let jpeg_avg_ms = if sent > 0 { jpeg_v.iter().sum::<u32>() / sent as u32 } else { 0 };
```
在返回的 `WindowStats { ... }` 字面量里加：
```rust
        resize_avg_ms,
        resize_p95_ms: percentile(&resize_v, 0.95),
        jpeg_avg_ms,
        jpeg_p95_ms: percentile(&jpeg_v, 0.95),
```

- [ ] **Step 6：format_log 加参数与字段（`:250-259`）**

改签名与格式串：
```rust
pub fn format_log(s: &WindowStats, sid: &str, adapt_level: u8) -> String {
    format!(
        "遥测 sid={sid} win=10s effective_fps={:.1} skip_pct={:.2} dirty_p50={:.2} dirty_p95={:.2} \
         cap_p95_ms={} enc_avg_ms={} enc_p95_ms={} resize_avg_ms={} jpeg_avg_ms={} \
         stall_p95_ms={} adapt_level={}",
        s.effective_fps, s.skip_pct, s.dirty_p50, s.dirty_p95,
        s.cap_p95_ms, s.enc_avg_ms, s.enc_p95_ms, s.resize_avg_ms, s.jpeg_avg_ms,
        s.stall_p95_ms, adapt_level
    )
}
```

- [ ] **Step 7：更新既有调用点与旧测试**

- run_collector（`:470` 附近）`format_log(&stats, &collector.sid_str())` → 暂传 0：
  ```rust
  tracing::info!("{}", format_log(&stats, &collector.sid_str(), 0));
  ```
  （Task 5 会改为传实际 level。）
- 旧测试 `format_log含关键字段`（`:393`）：把 `format_log(&s, "sid")` 调用改为 `format_log(&s, "sid", 0)`（断言保持原样即可，字段是超集）。
- telemetry.rs 测试里构造 `FrameSample` 的辅助器 `fs(...)`（`:282` 附近）与任何 `FrameSample { ... }` 字面量：补 `resize_ms: 0, jpeg_ms: 0,`（`encode_ms` 保留）。

- [ ] **Step 8：workers 填充新字段**

- 跳过帧（`workers.rs:378-389` 的 `FrameSample { ... encode_ms: 0, ... }`）加：`resize_ms: 0, jpeg_ms: 0,`。
- 发送帧（`workers.rs:401-413`）：`encode_ms` 行保留（外层 wall-clock 不变）；从 `o`（`encode_frame_q` 返回，见下）取分段。把 `:396` 的匹配改为绑定 `o`：
  ```rust
  match capture::encode_frame_q(&raw, qp.max_w, qp.max_h, qp.jpeg_q) {
      Ok(o) => {
          let encode_ms = now_ms().saturating_sub(t_enc) as u32;
          let (data, w, h) = (o.data, o.w, o.h);
          let encoded_bytes = data.len();
          seq += 1;
          if tele_on {
              let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Frame(crate::telemetry::FrameSample {
                  ts_ms: tick_now, seq, capture_ms, skipped: false,
                  dirty_ratio: d.dirty_ratio, keyframe_forced: d.keyframe_forced,
                  encode_ms, resize_ms: o.resize_ms, jpeg_ms: o.jpeg_ms,
                  encoded_bytes, w, h,
              }));
          }
          if from_ui_tx.send(net::FromUi::Frame { session_id: sid, data, w, h, seq }).is_err() {
              break;
          }
      }
      Err(e) => tracing::debug!("编码失败：{e}"),
  }
  ```
  > 注意：`o.data` 已被移动进 `data`，故先取 `o.resize_ms/o.jpeg_ms`（Copy）再用 `data`；上面顺序已满足。

- [ ] **Step 9：运行测试确认通过 + 全量**

Run: `cargo test -p client --lib telemetry 2>&1 | tail -15`
Expected: PASS（新旧遥测测试全过）。
Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过。

- [ ] **Step 10：Commit**

```bash
git add src/client/src/telemetry.rs src/client/src/workers.rs
git commit -m "feat(telemetry): 暴露 resize_ms/jpeg_ms 分段 + 日志加 adapt_level 占位"
```

---

### Task 3：构建 profile 修（image 热路径 opt-level=3）+ 基准测量

**Files:**
- Modify: `Cargo.toml`（根，加 `[profile.release.package.*]`）
- Add: `src/client/src/capture.rs` 一个 `#[ignore]` 基准测试（手动跑，记录提速）

- [ ] **Step 1：识别热点 crate**

Run: `cargo tree -p client 2>/dev/null | grep -iE "^\S*(image|jpeg|zune|fdeflate|resize)" | sort -u`
记录承担「JPEG 编码 / 缩放」的 crate 名（已知至少 `image`；`image` 0.25 的 JPEG 编码常经 `jpeg-encoder` crate；解码经 `zune-jpeg`——本路径只编码，但一并提速无害）。

- [ ] **Step 2：加 per-package opt-level 覆盖**

在根 `Cargo.toml` 的 `[profile.release]` 段之后加（crate 名以 Step 1 实测为准；至少含 `image`）：
```toml
# 图像热路径（Lanczos3 缩放 / JPEG 编码）为速度编译，抵消 opt-level="z" 对纯 Rust
# 图像处理的拖累；client 主 crate 与其余依赖仍 opt="z"，分发体积不膨胀。
[profile.release.package.image]
opt-level = 3

[profile.release.package.jpeg-encoder]
opt-level = 3
```
> 若 Step 1 显示编码走的不是 `jpeg-encoder`（比如内建于 `image`），删掉第二段即可——`image` 段已覆盖。不要臆造不存在的 crate 名（cargo 对未知 package 的 profile 覆盖会告警/报错）。

- [ ] **Step 3：加基准测试（手动跑）**

在 `src/client/src/capture.rs` 测试区加：
```rust
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
```

- [ ] **Step 4：测量前后对比**

改 profile **前**先跑一次记录基线（若已改，可用 git stash 对比）：
Run: `cargo test -p client --release encode_bench_1280x720 -- --ignored --nocapture 2>&1 | grep ms/帧`
改 profile **后**再跑：同命令。记录两次 `ms/帧` 与倍数到 commit message。
体积对比：`cargo build -p client --release 2>/dev/null && ls -l target/release/client | awk '{print $5}'`（前后各一次，记录字节增量）。

> 验收：opt=3 相对 opt=z 有明确提速（记录倍数）；二进制体积增量在可接受范围（记录数值，若异常膨胀则只保留 `image` 段、去掉编码器段再测）。此步不设硬阈值断言（机器差异），以**记录真实数值**为准。

- [ ] **Step 5：Commit**

```bash
git add Cargo.toml src/client/src/capture.rs
git commit -m "perf(build): image 热路径 opt-level=3(体积换编码速度) + 编码基准测试

基线 <前>ms/帧 → <后>ms/帧(<倍数>×)；二进制体积 +<字节>"
```

---

### Task 4：`adaptive.rs` 过载自适应纯状态机

**Files:**
- Create: `src/client/src/adaptive.rs`

- [ ] **Step 1：写失败测试（先建文件仅含测试与签名骨架）**

新建 `src/client/src/adaptive.rs`，先写测试（实现留空签名，让它编译失败/断言失败）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::QualityParams;

    fn overload() -> crate::telemetry::WindowStats {
        crate::telemetry::WindowStats { enc_p95_ms: 250, ..Default::default() }
    }
    fn healthy() -> crate::telemetry::WindowStats {
        crate::telemetry::WindowStats { enc_p95_ms: 50, effective_fps: 10.0, ..Default::default() }
    }

    #[test]
    fn 连续两窗过载_降一档() {
        let mut c = AdaptiveController::default();
        assert_eq!(c.observe(&overload()), 0, "第1窗过载不降");
        assert_eq!(c.observe(&overload()), 1, "第2窗过载降到1");
    }

    #[test]
    fn 连续三窗健康_回升一档() {
        let mut c = AdaptiveController::default();
        c.observe(&overload()); c.observe(&overload()); // → level 1
        assert_eq!(c.observe(&healthy()), 1, "健康1窗不升");
        assert_eq!(c.observe(&healthy()), 1, "健康2窗不升");
        assert_eq!(c.observe(&healthy()), 0, "健康3窗回升到0");
    }

    #[test]
    fn 交替过载健康_不累积降档() {
        let mut c = AdaptiveController::default();
        c.observe(&overload()); c.observe(&healthy());
        c.observe(&overload()); c.observe(&healthy());
        assert_eq!(c.level, 0, "过载不连续 → 不降档");
    }

    #[test]
    fn level有界_0到3() {
        let mut c = AdaptiveController::default();
        for _ in 0..20 { c.observe(&overload()); }
        assert_eq!(c.level, MAX_LEVEL, "封顶 3");
        for _ in 0..20 { c.observe(&healthy()); }
        assert_eq!(c.level, 0, "回落到 0");
    }

    #[test]
    fn clamp梯度_只降不升() {
        let p = QualityParams { max_w: 1280, max_h: 720, jpeg_q: 80, interval_ms: 40 };
        assert_eq!(clamp(p, 0), p, "level0 原样");
        let c1 = clamp(p, 1);
        assert!(c1.max_w < p.max_w && c1.interval_ms > p.interval_ms && c1.jpeg_q <= p.jpeg_q);
        let c3 = clamp(p, 3);
        assert!(c3.max_w < c1.max_w && c3.jpeg_q < p.jpeg_q, "档越高降越多");
    }

    #[test]
    fn resolve_enabled优先级() {
        assert!(!resolve_enabled(Some("0"), Some(true)), "env=0 关(压过 config)");
        assert!(resolve_enabled(Some("1"), Some(false)), "env=1 开");
        assert!(!resolve_enabled(None, Some(false)), "config=false 关");
        assert!(resolve_enabled(None, None), "默认 ON");
    }

    #[test]
    fn level_读受开关门控() {
        set_enabled(false);
        store_level(3);
        assert_eq!(level(), 0, "关闭时 level() 恒 0(旁路)");
        set_enabled(true);
        assert_eq!(level(), 3, "开启时反映实际档");
        store_level(0); // 复位避免污染其它测试
    }
}
```

> `QualityParams` 需 `#[derive(PartialEq, Clone, Copy)]` 才能 `assert_eq!(clamp(p,0), p)`。若其尚未 derive，本 Task Step 3 顺带在 `capture.rs` 的 `QualityParams` 上补 `Copy, PartialEq`（`Clone` 应已有）。

- [ ] **Step 2：运行确认失败**

Run: `cargo test -p client --lib adaptive 2>&1 | tail -15`
Expected: FAIL（`AdaptiveController`/`clamp`/`level` 等未定义）。

- [ ] **Step 3：实现 adaptive.rs**

在测试模块**上方**写实现：

```rust
//! 过载自适应闭环：collector 每 10s 窗 observe → 写 LEVEL 原子量 → 抓帧线程 clamp 折入。
//! 以用户手动档为上限，只降不升；迟滞（2 窗降/3 窗升）；env/config 可秒关。纯逻辑优先单测。

use crate::capture::QualityParams;
use crate::telemetry::WindowStats;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const MAX_LEVEL: u8 = 3;
const OVERLOAD_TO_DEGRADE: u8 = 2; // 连续 2 窗过载 → 降一档
const HEALTHY_TO_RECOVER: u8 = 3;  // 连续 3 窗健康 → 回升一档（比降档慢=更安全）

static LEVEL: AtomicU8 = AtomicU8::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);

/// 抓帧线程读取的当前档（关闭时恒 0=旁路）。
pub fn level() -> u8 {
    if ENABLED.load(Ordering::Relaxed) { LEVEL.load(Ordering::Relaxed) } else { 0 }
}
pub fn store_level(l: u8) { LEVEL.store(l.min(MAX_LEVEL), Ordering::Relaxed); }
pub fn set_enabled(on: bool) { ENABLED.store(on, Ordering::Relaxed); }
pub fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }

/// 过载：与 telemetry::classify 的「编码过载/投递饥饿」口径一致。
pub fn is_overload(s: &WindowStats) -> bool {
    s.enc_p95_ms > 200 || (s.effective_fps < 1.0 && s.dirty_p95 > 0.1)
}
/// 健康：留裕度（enc_p95<120 且 fps≥1）防止阈值边界抖动。
pub fn is_healthy(s: &WindowStats) -> bool {
    s.enc_p95_ms < 120 && s.effective_fps >= 1.0
}

/// 迟滞状态机（纯逻辑，单测友好）。
#[derive(Default)]
pub struct AdaptiveController {
    pub level: u8,
    overload_streak: u8,
    healthy_streak: u8,
}
impl AdaptiveController {
    /// 喂一窗统计，更新并返回新 level。
    pub fn observe(&mut self, s: &WindowStats) -> u8 {
        if is_overload(s) {
            self.healthy_streak = 0;
            self.overload_streak = self.overload_streak.saturating_add(1);
            if self.overload_streak >= OVERLOAD_TO_DEGRADE && self.level < MAX_LEVEL {
                self.level += 1;
                self.overload_streak = 0;
            }
        } else if is_healthy(s) {
            self.overload_streak = 0;
            self.healthy_streak = self.healthy_streak.saturating_add(1);
            if self.healthy_streak >= HEALTHY_TO_RECOVER && self.level > 0 {
                self.level -= 1;
                self.healthy_streak = 0;
            }
        } else {
            // 中间态：非过载也非健康 → 断两侧连击（要求连续才动作）。
            self.overload_streak = 0;
            self.healthy_streak = 0;
        }
        self.level
    }
}

/// 在已解析档位之上按 level 再钳（只降不升）。分辨率对 JPEG 耗时影响最大，优先降分辨率。
pub fn clamp(p: QualityParams, level: u8) -> QualityParams {
    let (res_ratio, q_down, iv_mul): (f32, u8, f32) = match level {
        0 => return p,
        1 => (0.85, 0, 1.25),
        2 => (0.70, 8, 1.5),
        _ => (0.55, 12, 2.0),
    };
    QualityParams {
        max_w: ((p.max_w as f32) * res_ratio) as u32,
        max_h: ((p.max_h as f32) * res_ratio) as u32,
        jpeg_q: p.jpeg_q.saturating_sub(q_down),
        interval_ms: ((p.interval_ms as f32) * iv_mul) as u64,
    }
}

/// 开关解析：env OHMYDESK_ADAPTIVE 优先（0/false/off/no=关），否则 config，默认 ON。
pub fn resolve_enabled(env_val: Option<&str>, config_val: Option<bool>) -> bool {
    if let Some(v) = env_val {
        return !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no");
    }
    config_val.unwrap_or(true)
}
```

- [ ] **Step 4：QualityParams 补 derive（若缺）**

确认 `src/client/src/capture.rs` 的 `pub struct QualityParams` 有 `#[derive(Clone, Copy, PartialEq, ...)]`。若缺 `Copy`/`PartialEq`，补上（结构体全为 `u32/u8/u64`，可安全 Copy）。

- [ ] **Step 5：运行测试确认通过**

Run: `cargo test -p client --lib adaptive 2>&1 | tail -15`
Expected: PASS（7 测试全过）。

- [ ] **Step 6：Commit**

```bash
git add src/client/src/adaptive.rs src/client/src/capture.rs
git commit -m "feat(adaptive): 过载自适应纯状态机(迟滞2降/3升+clamp梯度+开关解析)"
```

---

### Task 5：接线 adaptive（collector observe + 抓帧 clamp + 日志 level + 开关）

**Files:**
- Modify: `src/client/src/main.rs`（`mod adaptive`；解析开关 → set_enabled）
- Modify: `src/client/src/workers.rs:271`（抓帧线程套 clamp）
- Modify: `src/client/src/telemetry.rs` run_collector（observe + store_level + 传实 level 给 format_log）

- [ ] **Step 1：main.rs 声明模块**

`src/client/src/main.rs:22` 附近（`mod render_mode;` 旁）加：
```rust
mod adaptive;
```

- [ ] **Step 2：main.rs 解析开关并应用**

在渲染模式初始化块之后（`tracing::info!("渲染模式 ...")` 那行之后，约 `:127`）加：
```rust
    // 过载自适应开关：env OHMYDESK_ADAPTIVE > config.toml [render].adaptive > 默认 ON
    let cfg_adaptive = std::fs::read_to_string(ohmydesk_state_dir().join("config.toml"))
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("render").and_then(|r| r.get("adaptive")).and_then(|a| a.as_bool()));
    let env_adaptive = std::env::var("OHMYDESK_ADAPTIVE").ok();
    adaptive::set_enabled(adaptive::resolve_enabled(env_adaptive.as_deref(), cfg_adaptive));
    tracing::info!("过载自适应 adaptive_enabled={}", adaptive::enabled());
```

- [ ] **Step 3：抓帧线程套 adaptive clamp（workers.rs:271）**

把 `:271`：
```rust
                let qp = crate::render_mode::clamp_params(capture::current_params(), mode);
```
改为在其后再折入一层：
```rust
                let qp = crate::render_mode::clamp_params(capture::current_params(), mode);
                let qp = crate::adaptive::clamp(qp, crate::adaptive::level());
```

- [ ] **Step 4：run_collector 挂 observe + 传 level**

在 `telemetry.rs` `run_collector`（`:450` 起）循环**之前**加控制器：
```rust
    let mut adaptive = crate::adaptive::AdaptiveController::default();
```
在 `ticker.tick()` 分支里，把
```rust
                let stats = aggregate(&win_frames, &win_egress, 10_000);
                tracing::info!("{}", format_log(&stats, &collector.sid_str(), 0));
                let anomalies = classify(&stats);
```
改为：
```rust
                let stats = aggregate(&win_frames, &win_egress, 10_000);
                let lvl = adaptive.observe(&stats);
                crate::adaptive::store_level(lvl);
                tracing::info!("{}", format_log(&stats, &collector.sid_str(), lvl));
                let anomalies = classify(&stats);
```
> 注：`store_level` 只在 collector 单点写；`level()` 内部按 ENABLED 门控——关闭时抓帧线程读到 0（旁路），但 observe 仍持续更新内部 streak（重新开启即热）。

- [ ] **Step 5：编译 + 全量测试 + Windows 交叉编译门**

Run: `cargo test -p client 2>&1 | tail -15`
Expected: PASS（含 adaptive/telemetry/capture 全部）。
Run: `cargo build -p client 2>&1 | tail -5`
Expected: 通过。
Run: `cargo check -p client --target x86_64-pc-windows-gnu 2>&1 | tail -5`
Expected: 通过（Windows 路径不回归）。

- [ ] **Step 6：Commit**

```bash
git add src/client/src/main.rs src/client/src/workers.rs src/client/src/telemetry.rs
git commit -m "feat(adaptive): collector observe→level 原子量→抓帧 clamp 接线,日志出 adapt_level,env/config 开关"
```

---

## 人工验收清单（真机取证，弱 Xeon 上跑一次远控）

- [ ] **组件 1 证据**：`%APPDATA%\OhMyDesk\data\logs\client.log` 的遥测行含 `resize_avg_ms= jpeg_avg_ms= adapt_level=`；据此判断 360ms 主导项（jpeg 主导→后续 spec 上 turbojpeg；resize 主导→换缩放滤波/降分辨率）。
- [ ] **组件 2 效果**：opt=3 版相对 opt=z 版 `enc_avg_ms/jpeg_avg_ms` 明显下降（对比同机同档日志）；二进制体积增量已记录。
- [ ] **组件 3 自愈**：持续大面积变化（拖窗/滚动）触发过载后，`adapt_level` 从 0 逐步升到 1/2/3、分辨率/帧率随之下降、画面「能动」；停止操作恢复健康后 `adapt_level` 缓慢回落到 0。`OHMYDESK_ADAPTIVE=0` 启动后 `adapt_level` 恒 0（旁路）。

---

## Self-Review 记录

- **Spec 覆盖**：组件1(遥测拆分)=Task1+Task2；组件2(构建修)=Task3；组件3(自适应闭环)=Task4(纯逻辑)+Task5(接线)。非目标(turbojpeg/tile/硬编)未进任务。✔
- **占位符扫描**：无 TBD/TODO；Task3 Step1「cargo tree 识别 crate 名」是必要的真实发现步骤（附已知 crate 与兜底规则），非省略。基准不设硬阈值断言是刻意规避机器 flaky，改为记录真实数值——已注明。
- **类型一致**：`EncodeOut{data,w,h,resize_ms,jpeg_ms}`（Task1）↔ workers 取 `o.resize_ms/o.jpeg_ms`（Task2）↔ FrameSample 同名字段（Task2）↔ aggregate `resize_avg_ms/jpeg_avg_ms`（Task2）↔ format_log 同名（Task2）。`AdaptiveController::observe/level/store_level/clamp/resolve_enabled/enabled`（Task4）↔ Task5 调用签名一致。`QualityParams` 补 `Copy/PartialEq`（Task4 Step4）供 clamp 测试断言。✔
- **编译连续性**：Task2 format_log 改签名同步改 run_collector 调用(传0)与旧测试；Task5 再把 0 换成 lvl——每步均可编译。✔
