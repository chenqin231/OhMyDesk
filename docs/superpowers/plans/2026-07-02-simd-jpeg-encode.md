# 被控端 JPEG 编码提速（换 jpeg-encoder）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把被控端唯一 JPEG 编码点从 `image` crate 标量编码器换成 `jpeg-encoder`（纯 Rust 标量），弱机 1920 高清帧编码 265→63ms（Ivy Bridge 实测 4.19×），压到 adaptive 健康阈以下，根治高清模糊/无效。

**Architecture:** 单点替换 `capture.rs::encode_frame_q` 里的 JPEG 编码调用；`jpeg-encoder` 直接消费 RGBA（省掉单独 `to_rgb8()`）；resize 仍用 `image::imageops`（不在本计划）；adaptive/workers/主控解码全不动。

**Tech Stack:** Rust、`jpeg-encoder` v0.7（默认 features，**不启用 simd**）、`image` 0.25（保留：主控解码 + resize）、base64。

**关联 Spec:** `docs/superpowers/specs/2026-07-02-simd-jpeg-encode-design.md`

**分支:** `spec-simd-jpeg`（与已完成的 set_size 割裂/拖动跳 bugfix 一起进 0.4.9）。

---

## 前置：确认当前编码点

`src/client/src/capture.rs::encode_frame_q`（约 174–204 行）现状：
```rust
let t_resize = std::time::Instant::now();
let rgb = if (w, h) == (sw, sh) {
    image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()
} else {
    let resized = image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle);
    image::DynamicImage::ImageRgba8(resized).to_rgb8()
};
let resize_ms = t_resize.elapsed().as_millis() as u32;

let t_jpeg = std::time::Instant::now();
let mut buf = std::io::Cursor::new(Vec::new());
image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&rgb)?;
let data = STANDARD.encode(buf.get_ref());
let jpeg_ms = t_jpeg.elapsed().as_millis() as u32;

Ok(EncodeOut { data, w, h, resize_ms, jpeg_ms })
```
`EncodeOut { data, w, h, resize_ms, jpeg_ms }` 结构与 `MAX_W/MAX_H`、`scaled_dims`、`STANDARD`（base64）均已存在，不改。

---

### Task 1: 加 jpeg-encoder 依赖 + 表征测试（锁定"换器后仍产合法 JPEG"）

**Files:**
- Modify: `src/client/Cargo.toml`
- Test: `src/client/src/capture.rs`（`#[cfg(test)] mod tests`）

- [ ] **Step 1: 加依赖**

在 `src/client/Cargo.toml` 的 `[dependencies]` 加一行（默认 features，**不加 simd**——目标机 Ivy Bridge 无 AVX2，且 opt-z 下 simd 反更慢，见 spec）：
```toml
jpeg-encoder = "0.7"
```

- [ ] **Step 2: 写表征测试（换器前先立，锁定行为）**

在 `src/client/src/capture.rs` 的 `mod tests` 内加（沿用现有测试的 base64 解码 + `image::load_from_memory` 模式）：
```rust
#[test]
fn encode_frame_q_产出可解回原尺寸_高清q88() {
    use base64::Engine;
    let img = RgbaImage::from_pixel(1280, 720, image::Rgba([90, 140, 200, 255]));
    let out = encode_frame_q(&img, 1920, 1080, 88).unwrap();
    assert_eq!((out.w, out.h), (1280, 720), "不放大，尺寸原样");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(out.data.as_bytes())
        .expect("产出应为合法 base64");
    let decoded = image::load_from_memory(&bytes).expect("产出应为可解码 JPEG");
    assert_eq!((decoded.width(), decoded.height()), (1280, 720), "解回同尺寸");
}

#[test]
fn encode_frame_q_大屏缩放后可解回上限尺寸() {
    use base64::Engine;
    let img = RgbaImage::from_pixel(3840, 2160, image::Rgba([10, 20, 30, 255]));
    let out = encode_frame_q(&img, 1920, 1080, 88).unwrap();
    assert_eq!((out.w, out.h), (1920, 1080), "4K 等比缩到 1080p 上限");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(out.data.as_bytes())
        .unwrap();
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (1920, 1080));
}
```
> 注：`RgbaImage`、`encode_frame_q` 已在 `use super::*;` 可见；`base64::Engine` trait 需在测试内 `use`（`.decode` 依赖它）。

- [ ] **Step 3: 跑测试确认当前(image 编码器)已通过**

Run: `cargo test -p client encode_frame_q_产出可解回原尺寸_高清q88 encode_frame_q_大屏缩放后可解回上限尺寸`
Expected: PASS（当前 image 编码器即满足；这两条是换器安全网）。

- [ ] **Step 4: Commit**

```bash
git add src/client/Cargo.toml src/client/src/capture.rs Cargo.lock
git commit -m "test(client): 为 JPEG 编码器替换加表征测试 + 引入 jpeg-encoder 依赖"
```

---

### Task 2: 换编码器（image 标量 → jpeg-encoder 标量，直吃 RGBA）

**Files:**
- Modify: `src/client/src/capture.rs::encode_frame_q`（前置节所示的 resize+jpeg 块）

- [ ] **Step 1: 替换 encode_frame_q 的 resize+jpeg 实现**

把前置节所示代码块整体替换为：
```rust
    // 缩放（等比，不放大）。尺寸未变时跳过 resize。resize 仍用 image::imageops(Triangle)，
    // 提速见另一 spec。不再单独 to_rgb8()——jpeg-encoder 直接吃 RGBA，内部做 RGB 转换。
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

    // JPEG：jpeg-encoder 纯 Rust 标量（Ivy Bridge 实测 4.19× 于 image 标量，见 spec）。
    // 直接消费 RGBA（alpha 编码时忽略）。width/height 为 u16——屏幕分辨率不越界。
    // 默认采样：q<90 → 4:2:0，与主控 image 解码兼容。
    let t_jpeg = std::time::Instant::now();
    let mut buf: Vec<u8> = Vec::new();
    jpeg_encoder::Encoder::new(&mut buf, q)
        .encode(rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .map_err(|e| anyhow::anyhow!("jpeg 编码失败: {e}"))?;
    let data = STANDARD.encode(&buf);
    let jpeg_ms = t_jpeg.elapsed().as_millis() as u32;

    Ok(EncodeOut { data, w, h, resize_ms, jpeg_ms })
```
> `RgbaImage` 已在文件顶部 `use`。若编译报 `RgbaImage` 未导入，确认顶部 `use image::RgbaImage;` 存在（现有 `encode_frame` 签名已用它，应在）。

- [ ] **Step 2: 跑 capture 全部编码测试**

Run: `cargo test -p client --lib capture`
Expected: PASS —— 现有 6 条（`encode_帧体积可控`/`encode_质量档位_高清更大`/`encode_非16比9_等比`/`占位帧_可解码为_1280x720_rgba`/`encode_大屏_缩放到上限`/`encode_小屏_不放大`）+ Task 1 两条全绿。
> 若 `encode_质量档位_高清更大`（断言 q88 产物 > q80 产物）失败：jpeg-encoder 在极小纯色测试图上不同 q 的体积差可能很小甚至反转。若失败，改该测试用 Task 1 的内容丰富图（`RgbaImage::from_fn` 加渐变+噪声）而非纯色，再断言 q88 ≥ q80 体积。

- [ ] **Step 3: 跑 client 全量测试**

Run: `cargo test -p client`
Expected: PASS（162+ 条全绿，无回归）。

- [ ] **Step 4: 确认 image crate 的 jpeg feature 仍需保留**

Run: `cargo build -p client 2>&1 | grep -i "unused\|jpeg" || echo "no jpeg-unused warning"`
说明：主控端 `ui_glue.rs::decode_frame_rgba` 用 `image::load_from_memory` 解 JPEG，故 `image` 的 `jpeg` feature **不能删**。此步仅确认没有"因不再用 image 编码器就误删 feature"的冲动——不改 `image` 依赖。

- [ ] **Step 5: Commit**

```bash
git add src/client/src/capture.rs
git commit -m "perf(client): JPEG 编码换 jpeg-encoder 标量，弱机高清 265→63ms(4.19×) 治本编码过载"
```

---

### Task 3: 交叉编译 + 收尾验证

**Files:** 无（仅验证）

- [ ] **Step 1: Windows 交叉编译（被控主平台）**

Run:
```bash
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release -p client --target x86_64-pc-windows-gnu 2>&1 | tail -3
```
Expected: `Finished release`。jpeg-encoder 纯 Rust，无 C/nasm，windows-gnu 无痛。

- [ ] **Step 2: 龙芯交叉编译（若本地有 loongarch64 target；无则由 CI/自托管覆盖）**

Run: `rustup target list --installed | grep -q loongarch64 && cargo build --release -p client --target loongarch64-unknown-linux-gnu 2>&1 | tail -3 || echo "本地无 loongarch target，交由 CI/自托管 runner 验证（纯 Rust 标量预期无痛）"`
Expected: 编译通过，或明确交由 CI。

- [ ] **Step 3: 真机基准与观感（人工，交付后）**

在 Xeon E5-2697 v2 / i5-3470 被控机装新版，远控高清全屏动态内容，观察被控日志：
- `jpeg_avg_ms` 应从 ~137 降到 <120（预计 ~40–63），`adapt_level` 全程 0，不再 `[编码过载]`。
- 主控端肉眼比对高清清晰度。**若 q88 观感偏软**（spec 记：同 q 产物小 38%）：把 `src/client/src/capture.rs` 里 HighQuality 档的 `jpeg_q`（`capture.rs:43-58` 参数表，现 88）上调到 90~92，重测；即便上调耗时仍远低于旧值。此调参为独立小改，视真机观感决定，不阻塞合入。

- [ ] **Step 4: 最终提交（若 Step 3 需调 q 才做，否则跳过）**

```bash
git add src/client/src/capture.rs
git commit -m "tune(client): 高清档 q 上调对齐 jpeg-encoder 观感（真机比对后）"
```

---

## Self-Review

- **Spec 覆盖**：换编码器（Task 2）✓；依赖+不启用 simd（Task 1 Step 1）✓；直吃 RGBA 省 to_rgb8（Task 2 Step 1）✓；分段计时保留（Task 2 Step 1 保留 resize_ms/jpeg_ms）✓；主控解码不动/image jpeg feature 保留（Task 2 Step 4）✓；交叉编译含龙芯（Task 3）✓；真机基准+观感 q 调优（Task 3 Step 3）✓；4:2:0 兼容（Task 2 Step 1 注释）✓。
- **占位符扫描**：无 TBD/TODO；每个代码步给了完整代码与确切命令。
- **类型/签名一致**：`EncodeOut { data, w, h, resize_ms, jpeg_ms }` 全程一致；`encode(rgba, w as u16, h as u16, ColorType::Rgba)` 与 spec 核过的 v0.7 API 一致；`resized: Option<RgbaImage>` 与 `.as_raw()` 用法自洽。
- **风险已在步骤内兜底**：`encode_质量档位_高清更大` 可能因 jpeg-encoder 体积特性失败 → Task 2 Step 2 给了改测试内容的兜底；观感偏软 → Task 3 Step 3 给了 q 上调路径。
