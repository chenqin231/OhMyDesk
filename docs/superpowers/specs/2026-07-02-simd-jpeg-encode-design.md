# 被控端 JPEG 编码提速 设计（Spec）

> 日期：2026-07-02
> 状态：选型已实测坐实（换 jpeg-encoder 标量），待转实现计划
> 注：文件名含 "simd" 是历史沿用；实测证明**不需要 SIMD**，jpeg-encoder 的纯标量实现即比 image crate 快 3.17×，故选标量、不启用 simd feature（详见方案选型）。
> 关联：[[2026-07-01-resize-pipeline-perf-design]]（下采样 resize 提速，另一 spec，不在本 spec）、[[2026-07-01-encoding-overload-mitigation-design]]（adaptive 过载自适应，本 spec 保留不动）

## 目标

把被控端 JPEG 编码从 `image` crate 的标量编码器换成 `jpeg-encoder` crate（**纯 Rust，默认标量**），把弱机 1920×1080 高清帧 JPEG 编码压到 adaptive 健康阈（`ENC_HEALTHY_P95_MS=120`）以下，使高清档全程停在 adaptive level 0（不降分辨率），**根治弱机高清画面模糊 / 「高清」按钮无效**。**目标机 Ivy Bridge 真机实测**：q88 image 标量 265.6ms → jpeg-encoder 标量 **63.4ms（4.19×）**，远低于 120ms 阈（合成高频图为保守上界，真实桌面更快）。

## 背景与根因（数据驱动）

用户报「远控主控端最大化后画面模糊、点高清无效」。真机日志定案根因：

- 被控是老 CPU（i5-3470 / Xeon E5-2697 v2）。软件 JPEG 编 1920 帧慢：真机遥测 1920 档 `jpeg_avg_ms=137`、`enc_p95>200`（`resize_avg_ms=10`，即 RGBA→RGB 转换很便宜，**JPEG 熵编码是唯一大头**）。
- `enc_p95>ENC_OVERLOAD_P95_MS(200)` → adaptive 连 2 窗判过载 → `adaptive::clamp` 按 level 1/2/3 把分辨率 ×0.85/0.70/0.55 → 主控实收 1632/1344/1056 宽小图（主控日志 `主控收到帧分辨率=` 与 `adapt_level` 逐位吻合）→ 细节丢失 → 糊。
- 点「高清」经 `adaptive::request_reset` 只生效一瞬，被控编 1920 立刻又过载，~20s 内 adapt_level 爬回 1→2→3 → 与用户来回点=拉锯。

结论：模糊的根 = **弱机软件标量 JPEG 编 1920 太慢**。治本 = 提速 JPEG 编码，让 adaptive 不再被触发。

> 注：主控端显示侧（frame_scale / image-fit / HiDPI）经排查**不是**模糊主因（用户主控 100% 缩放）。显示侧唯一确认的 bug 是 `set_size` 风暴（最大化字体割裂 + 拖动窗口随机变大小/位置），已在独立 bugfix 修（`ui_glue.rs` 改为仅首帧贴合窗口），不在本 spec。

## 方案选型

**选定：`jpeg-encoder` crate v0.7，默认 features（纯标量），不启用 `simd` feature。**

### 实测数据（生产 profile：opt-level="z" + lto=true + codegen-units=1；1920×1080 q88，40 次中位）

| 编码器 | 中位耗时 | 产物体积 | 说明 |
|---|---|---|---|
| `image` 标量（现状） | 189.9ms | 1420 KB | 基线（本机 Xeon E5-2686 v4，2.3GHz） |
| **`jpeg-encoder` 标量** | **59.9ms** | 879 KB | **3.17× 提速**，产物小 38% |
| `jpeg-encoder` + `simd` | 71.3ms | 880 KB | **反更慢**：opt-z 洗掉 AVX2 内联优化 |

关键推理：**标量 vs 标量的比值与硬件无关**（同为 x86-64 标量 Rust 码），可外推到目标机。

### 目标机实测坐实（Ivy Bridge 被控机，cross-compile 的 bench.exe 现场跑，同 profile 同合成图）

| Ivy Bridge（`avx2=false` 确认无 AVX2；avx/sse4.1=true） | image 标量 | jpeg-encoder 标量 | 加速比 |
|---|---|---|---|
| q88 高清 | 265.6ms | **63.4ms** | **4.19×** |
| q80 流畅 | 231.2ms | 57.2ms | 4.04× |

**63.4ms ≪ 健康阈 120ms → adaptive 不再触发降档，决定全实测坐实（非外推）。** 合成图含高频噪声、比真实桌面更狠（真机遥测真实内容 image≈137ms vs 合成 265ms，约 2×），故实战 jpeg-encoder 预计更低（~35–40ms）。目标机加速比 4.19× 高于开发机 3.17×。

### 为何不用 SIMD / libjpeg-turbo

- **不开 `simd` feature**：① 目标被控机是 Ivy Bridge（i5-3470 / Xeon E5-2697 v2，**无 AVX2**），而 jpeg-encoder 的 SIMD 仅 AVX2 路径（无 SSE、无 NEON、无龙芯）→ 对目标机零效；② 即便在有 AVX2 的机器上，生产 `opt-level="z"` 下 SIMD 路径反比标量慢（实测 71.3 > 59.9）；③ 开 simd 会引入 `unsafe`。故纯标量最优。
- **不用 libjpeg-turbo（turbojpeg/mozjpeg）**：其 SSE2 路径虽能覆盖 Ivy Bridge，但需引 C 依赖 + CI 装 nasm/cmake + 六平台（尤其龙芯）交叉编译 C，代价大；而 jpeg-encoder 标量已把 137→43ms 打到阈下，**无需再上 C**。

| 备选 | 结论 |
|---|---|
| **jpeg-encoder 标量（选定）** | 3.17× 已够；纯 Rust 标量在 x86 老机/ARM/龙芯完全一致，交叉编译零特殊处理 |
| jpeg-encoder + simd | 否决：目标机无 AVX2 无效；opt-z 下反更慢；引入 unsafe |
| libjpeg-turbo（C SIMD） | 否决：标量已够，不必付 C 依赖 + 交叉编译代价 |
| 保持 image 标量 | 否决：即根因本身 |

## 架构与改动面（单点，极小）

- `src/client/src/capture.rs::encode_frame_q` —— 全仓**唯一** JPEG 编码点。
- `src/client/Cargo.toml` —— 新增 `jpeg-encoder = "0.7"`（**默认 features，不加 `simd`**）。`image` crate **保留**：主控端解码 `image::load_from_memory` 仍用它，被控 resize 仍用 `image::imageops`。
- adaptive / workers / frameskip / telemetry / 主控解码 —— **全不动**。

## 数据流（encode_frame_q 内）

- 现状：`RGBA →（可选 Triangle resize）→ to_rgb8()（RGBA→RGB, ~10ms）→ image::codecs::jpeg::JpegEncoder（标量, 137ms）→ base64`。
- 目标：`RGBA →（可选 Triangle resize，仍 image::imageops）→ jpeg-encoder 直接吃 RGBA（ColorType::Rgba，内部做 RGB 转换 + 编码）→ base64`。
- **附带收益**：`jpeg-encoder` 直接消费 RGBA 字节，省掉单独 `to_rgb8()` 的一次全帧转换与分配。
- **精确 API（v0.7，已核源码）**：`let mut buf: Vec<u8> = Vec::new(); jpeg_encoder::Encoder::new(&mut buf, q).encode(rgba_bytes, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)?;` —— `std` 下 `&mut Vec<u8>` 满足 `JfifWrite`（`impl<W: Write> JfifWrite for W`）；`encode` 的 **width/height 是 `u16`**（需 `as u16`，1920/1080 安全）；`encode(self,…)` 消费 encoder，借用结束后 `buf` 即含 JPEG 字节。默认采样 `quality<90 → F_2_2`(4:2:0)，q=80/88 均命中 → 与主控 image 解码兼容。
- resize 路径**不碰**（下采样提速是另一 spec）。resize 后得到的 `RgbaImage` 直接喂 `jpeg-encoder`。
- 分段计时保留：`resize_ms`（现在纯 resize；尺寸未变时仍≈0）、`jpeg_ms`（含内部 RGB 转换 + 编码）。telemetry 字段与口径不变。

## 兼容与质量

- quality：`jpeg-encoder` 的 quality 亦为 1–100，起步沿用现档位 `q=80`（Smooth）/ `q=88`（HighQuality）。**注意**：实测同 q88 下 jpeg-encoder 产物 879KB vs image 1420KB（小 38%），说明**同 q 数值下量化更激进 / 观感质量偏低**。故需真机比对观感，很可能要把档位 q 调高（如 HighQuality q88→90~92）对齐清晰度；即便调高，耗时仍远低于旧值。产物偏小对拥塞公网是额外收益。
- 色度采样：用 `jpeg-encoder` 默认（`q<90 → F_2_2` 即 4:2:0），与主控 `image` 解码及浏览器兼容。若为观感把 q 提到 ≥90 会切到 F_1_1(4:4:4)、体积上升——需在 q 调优时一并权衡。
- 产出为标准 baseline JPEG，主控端 `image::load_from_memory` 解码路径无需改动。

## 错误处理

`jpeg-encoder` 编码返回 `Result`，失败经现有 `anyhow::Result` 冒泡；workers 推帧循环已有 catch 兜底，行为不变。

## 测试（TDD）

- **复用** capture.rs 现有 6 个编码测试（`encode_帧体积可控` / `encode_质量档位_高清更大` / `encode_非16比9_等比` / `占位帧_可解码为_1280x720_rgba` / `encode_大屏_缩放到上限` / `encode_小屏_不放大`）——它们断言产出能被 `image` 解回正确尺寸，天然覆盖「换编码器后仍产出合法、可解码、尺寸正确的 JPEG」。
- **新增**：
  - round-trip：`encode_frame_q` 产出经 `image::load_from_memory` 解回与输入一致的 w×h。
  - RGBA 直喂与旧 `to_rgb8` 路径在尺寸/可解码性上一致（防回归）。
- **真机基准**（非 CI，人工）：Xeon E5-2697 / i5-3470 上 1920 全屏动态内容，对比新旧 `jpeg_ms`，确认稳定 <120ms 且 `adapt_level` 全程 0。
- **交叉编译**：六平台编译通过。纯标量 Rust、无 C、无 arch 特化 → 龙芯 loongarch64 与其它平台代码路径完全一致，预期无痛（无需 nasm/cmake/C 工具链）。
- **观感验收**：真机主控端肉眼比对新旧高清清晰度，据此定档位 q（见「兼容与质量」）。

## 成功标准

真机（Xeon E5-2697）1920 全屏动态内容下 `enc_p95` 稳定 <120ms（健康阈）→ adaptive 全程停在 level 0（不降分辨率）；主控端高清主观清晰、不再模糊、「高清」按钮切换后不被拉回。

## 不在范围

- resize（下采样）提速 —— 另 spec `2026-07-01-resize-pipeline-perf-design`。
- adaptive 策略调整 —— 保留现闭环当安全网（换编码器后基本不触发；真扛不住时降分辨率仍是正确响应）。
- 主控端显示侧 set_size 风暴（割裂 / 拖动窗口乱跳）—— 已在独立 bugfix 修复（`ui_glue.rs` 仅首帧贴合窗口）。

## 风险

- `jpeg-encoder` 真机提速不达标（<2×）→ 仍可能触发 adaptive。缓解：真机基准（本 spec 已在开发机实测 3.17×/opt-z，风险低）；若不达标，回退评估 libjpeg-turbo（SSE2 覆盖 Ivy Bridge）。
- 同 q 数值下 jpeg-encoder 产物更小=观感质量偏低 → 真机比对后上调档位 q（见「兼容与质量」）。
- 若未来目标机普遍为 AVX2+（新 x86），可评估是否值得为那批机启用 simd feature + opt-level 调整——当前不做（YAGNI，目标机是 Ivy Bridge）。
