# 被控端缩放/像素转换管线提速 设计文档（Spec ②-后续）

> 日期：2026-07-01 · 状态：待评审 · 类型：性能 · 纪律：先修基准再治理（前一个基准是废的）
> 取代原「turbojpeg」后续项——真机证据表明 JPEG 不是大头。

## 目标

一句话：把被控端 `encode_frame_q` 的**缩放 + RGBA→RGB 转换**耗时（老 Xeon 上 ≈292ms，占编码 79%）砍下来，让弱机远控真正可用；JPEG 编码只占 21%（78ms），非本 spec 重点。

## 背景（真机实证，推翻旧结论）

老 Xeon E5-2697 真机 `client.log`（0.4.4，遥测分段）：
- 流畅档（1904×957 → 1280×720 真降采）：`enc_avg_ms=371 · resize_avg_ms=292 · jpeg_avg_ms=78` → **缩放段 79%**。
- 高清档（1904×957，**无降采**）：`resize_avg_ms≈315` → 即便不缩放，`encode_frame_q` 里 `image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()` 这步（clone + RGBA→RGB 标量转换）本身就 ≈315ms。

**根因两条**（`src/client/src/capture.rs:158-177` encode_frame_q）：
1. **Lanczos3 缩放**（`imageops::resize`, 6-tap 分离核）在降采路径上巨贵。
2. **RGBA→RGB 转换**经 `DynamicImage` 中转 + `to_rgb8()` 逐像素标量循环，1904×957≈1.82M 像素在 `opt-level="z"` 下极慢；还多一次 `img.clone()`（7.3MB memcpy）。

**为何之前误判为「JPEG 97%」**：dev 基准 `encode_bench_1280x720` 造 1280×720 图"缩放"到 1280×720——`scaled_dims` 判等尺寸走**跳过 resize** 分支，根本没测真实降采，resize 记成 1.3ms 是假象。**教训：基准必须复现真实输入尺寸→输出尺寸；分段遥测是抓出此事的关键。**

## 已定方向（brainstorming 确认）

按「先修基准 → 便宜招 → SIMD → 重测构建」推进；turbojpeg 降级（只 21%，不值 C 依赖）。

## 架构（measurement-first，分档）

```
Phase0 修基准 ── encode_bench 改为真实降采(1904×957→1280×720,真实图案)，dev 相对 + Xeon 绝对(遥测)双测
      ▼
Lever A 便宜招(无依赖) ── Lanczos3→Triangle/CatmullRom 滤波 + 避开 DynamicImage clone/中转
      ▼
Lever B SIMD(Rust 依赖) ── fast_image_resize 直接对 RGBA SIMD 降采(~5-10×)，验交叉编译
      ▼
Lever C 重测构建 profile ── 用修好的基准重测 整crate opt=3 / lto="thin"，权衡体积
```

## 组件分解（MECE）

### 组件 0：修基准（证据门，先做）
`src/client/src/capture.rs` 的 `encode_bench_1280x720` 改为真实降采：源用 **1904×957**（贴近现网 Xeon 屏）随机/网格图，`encode_frame_q(&img, 1280, 720, 80)` 走真实 Lanczos3 降采 + 转换。另加一个高清档基准（源 1904×957，max 1920×1080，走无降采路径）单量 RGBA→RGB 转换成本。产出 dev 机基线 `resize_ms/jpeg_ms`，供各 Lever 对比。**真机绝对值仍以老 Xeon 遥测 `resize_avg_ms/jpeg_avg_ms` 为准。**

### 组件 A：便宜招（无新依赖，先试）
- **换滤波**：`FilterType::Lanczos3` → `Triangle`（双线性 2-tap）或 `CatmullRom`（4-tap）。远控 720p 下画质损失可接受，缩放显著提速。做成可切换/可测。
- **避开 DynamicImage 中转**：现 `DynamicImage::ImageRgba8(img.clone()).to_rgb8()` 有 clone + 枚举包装开销。改为：直接 `imageops::resize`（RgbaImage→RgbaImage，无 clone）再对**已变小**的结果做一次紧凑 RGBA→RGB（`chunks_exact(4)` 手写或 `to_rgb8`）。转换在小图上做，成本随分辨率下降。
- 度量组件 0 基准的 resize_ms 降幅。

### 组件 B：SIMD 缩放（引入 Rust 依赖 `fast_image_resize`）
- `fast_image_resize`（纯 Rust + SIMD，无 C 依赖）直接对 RGBA 做 SIMD 降采，常见 5-10× 于 `imageops`。
- 集成点：`encode_frame_q` 的缩放段替换为 fast_image_resize（RGBA→RGBA），再 RGBA→RGB 交 JpegEncoder。
- **必验交叉编译**：`cargo check -p client --target x86_64-pc-windows-gnu` + 龙芯（若可）；SIMD 特性对 target 的可用性。
- 度量：相对组件 A 的进一步提速；二进制体积增量。

### 组件 C：重测构建 profile（用修好的基准）
- 之前「opt-level 对速度无用」是**废基准**得出的。用组件 0 的真实基准重测：
  - 整 crate `[profile.release] opt-level = 3`（或 `"s"`）在真实缩放/转换路径的提速；
  - 或 `lto = "thin"` 让 per-package opt 生效（fat-LTO 洗 per-package 的结论仍成立）。
- **权衡二进制体积**（绿色分发底线）：记录各方案 提速倍数 vs 体积增量，取性价比最高。

### 组件 D（非目标/降级）
- **turbojpeg**：JPEG 仅 21%，换它顶多省 ~50ms，不解 292ms 缩放。除非组件 A/B/C 后 jpeg 反成新瓶颈才考虑。
- 硬件/帧间编码、tile 区域编码：仍另立。

## 数据流

抓帧线程 `capture_raw`(RGBA 1904×957) → `encode_frame_q`（缩放[组件A/B] + RGBA→RGB[组件A] + JPEG）→ 遥测 `resize_ms/jpeg_ms` → 真机日志验证降幅。自适应闭环不变（它是负载兜底，与本 spec 正交且互补：缩放变快后同样 CPU 能承载更高分辨率/帧率）。

## 测试策略

- **组件 0**：修好的基准 dev 可跑，输出 resize_ms/jpeg_ms（不设硬阈值断言，记录真实值）。
- **组件 A/B**：基准对比提速倍数；画质人工抽检（720p 远控可接受）；组件 B 加交叉编译门。
- **组件 C**：基准 opt 对比 + 体积断言在预算内。
- **真机验收**：老 Xeon 远控，遥测 `resize_avg_ms` 从 ≈292ms 明显下降（目标量级 <100ms），`effective_fps` 上升，画面可用。

## 非目标（YAGNI）

turbojpeg（降级）、硬件/帧间编码、tile 区域编码、采集端换库。

## 验收标准

1. 基准复现真实降采（1904×957→1280×720），dev + Xeon 遥测双路径可测。
2. 组件 A（无依赖）先给出可观降幅；不足再上组件 B（SIMD）。
3. 老 Xeon 真机 `resize_avg_ms` 显著下降、`effective_fps` 上升、画质可接受。
4. Windows 交叉编译不回归；二进制体积增量在预算内；三端测试全绿。
