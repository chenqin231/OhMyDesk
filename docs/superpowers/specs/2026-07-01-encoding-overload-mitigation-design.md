# 被控端编码过载治理 设计文档（Spec ②）

> 日期：2026-07-01 · 状态：待评审 · 类型：性能 · 纪律：先证据后修复（systematic-debugging）
> 范围：测量拆分 + 构建 profile 提速 + 过载自适应闭环。**不含** turbojpeg / tile 区域编码 / 硬件编码（列为后续 spec）。

## 目标

一句话：治理被控端「编码过载」——现网弱 CPU（老 Xeon E5-2697）上 `enc_avg_ms≈360ms`、`effective_fps` 跌到 0.1–1.4，画面近乎卡死。本 spec 用**零新 C 依赖、零协议改动**的三招：先测量定位 360ms 去向，再修构建 profile 提速热路径，最后加过载自适应闭环让系统在弱机上自动降档自愈。

## 背景（现状事实，均带行号）

- **单线程串行是根**：被控推帧线程（`workers.rs:356-424`）在**同一 `std::thread` 内串行**执行 `capture_raw`(RGBA 整屏) → 瓦片哈希 → `encode_frame_q`(Lanczos3 缩放 + 整屏软件 JPEG + base64)。编码 CPU 密集且**阻塞下一帧抓取**，直接压低 `effective_fps`。
- **纯 Rust 软件 JPEG，无 SIMD/硬编/帧间编码**：`capture.rs:158` `image::codecs::jpeg::JpegEncoder::new_with_quality`；缩放 `capture.rs:153` `image::imageops::resize` Lanczos3。
- **`enc_ms` 混算**：`workers.rs:395-398` 只在 `encode_frame_q` 外围测一个 `encode_ms`，**把 Lanczos3 缩放 + JPEG 编码混在一起**。`FrameSample.encode_ms`（`telemetry.rs:13`）单字段，聚合成 `enc_avg_ms/enc_p95_ms`（`telemetry.rs:42-43,95,112`）。**当前无法区分 360ms 花在缩放还是编码**。
- **过载判定已有但不闭环**：`classify` 判 `编码过载 = enc_p95_ms > 200`（`telemetry.rs:135-136`），命中只 `warn!` + 触发式 dump（`telemetry.rs` collector），**不回写降质/降帧**。唯一自适应是与过载无关的空闲降采（`framediff::relaxed_interval`）。
- **构建为体积牺牲速度**：根 `Cargo.toml:14` `opt-level = "z"` + `lto=true`。**纯 Rust 图像处理（DCT/Lanczos3）在 `opt-level="z"` 下会比 `opt-level=3` 慢一个数量级**——这是「360ms 对 720p 反常地慢」的**第一嫌疑**（libjpeg-turbo 同规格约 5–15ms，纯 Rust opt=3 也应在数十 ms）。
- **已有钳制 pattern 可复用**：`render_mode::clamp_params`（`render_mode.rs:64-75`）已示范「在已解析档位之上再钳一层上限」（LowBandwidth：≤1280×720 / q≤80 / 间隔≥150ms）。抓帧线程每帧 `clamp_params(current_params(), mode)`（`workers.rs:271`）。
- **档位事实**：`params_for`（`capture.rs:42-57`）单一事实源——流畅 1280×720/q80/40ms（默认）、高清 1920×1080/q88/100ms。`QualityParams { max_w, max_h, jpeg_q, interval_ms }`。

## 已定决策（brainstorming 确认）

采「先测量 + 构建修 + 自适应闭环」档：**零新 C 依赖、零协议改动、三端协议不动**。turbojpeg / tile 区域编码 / 硬件编码 = 后续 spec，**测了再定**（本 spec 的组件 1 正是为它们提供决策证据）。

## 架构

三组件按 systematic-debugging「证据 → 修复」顺序：

```
组件1 测量拆分  ── 把 enc_ms 拆成 resize_ms + jpeg_ms，跑一次真机拿证据
      │  （证据门：360ms 到底在缩放还是编码？）
      ▼
组件2 构建 profile 修 ── image 及其编码/缩放依赖 crate 提到 opt-level=3，
      │                    client 主 crate 保留 "z"（绿色分发体积）
      ▼
组件3 自适应闭环 ── collector 每 10s 窗判过载 → 写 adaptive_level 原子量
                     → 抓帧线程 clamp 折入 → 过载自动降档、健康回升（迟滞）
```

## 组件分解（MECE）

### 组件 1：遥测拆分（先定位，证据门）
把 `encode_ms` 一个数拆成两个可观测量，回答「360ms 在缩放还是编码」。

- **`capture.rs::encode_frame_q`**：在 `:153` 缩放段与 `:158` 编码段各取 `Instant`，返回值增加 `(resize_us, jpeg_us)`（或返回一个结构体）。为不破坏纯函数可测性，签名改为返回 `EncodeTiming { data, w, h, resize_us: u32, jpeg_us: u32 }`。
- **`FrameSample`（`telemetry.rs:6-17`）**：`encode_ms` 拆为 `resize_ms: u32` + `jpeg_ms: u32`（或并存 `encode_ms` 兼容旧字段）。`workers.rs:395-413` 填充两值。
- **`WindowStats` + `aggregate`（`telemetry.rs:28-116`）**：新增 `resize_avg_ms/resize_p95_ms`、`jpeg_avg_ms/jpeg_p95_ms`。
- **`format_log`（`telemetry.rs:250-259`）**：日志行增 `resize_avg_ms= jpeg_avg_ms=`，可 grep。
- **产出**：用户在 Xeon 上跑一次远控发日志 → 定位主导项。**这是后续是否需要 turbojpeg（编码主导）还是换缩放算法（缩放主导）的决策依据**。

### 组件 2：构建 profile 修（体积换速度，仅热路径）
验证并消除 `opt-level="z"` 对图像热路径的拖累，同时守住绿色分发体积。

- **根 `Cargo.toml` 加 per-package 覆盖**：
  ```toml
  [profile.release.package.image]
  opt-level = 3
  # 及 image 的 JPEG 编码 / 缩放实际依赖 crate（实现时以 `cargo tree -p client` 确认 crate 名，
  # 逐个补 [profile.release.package.<crate>] opt-level = 3）
  ```
  **client 主 crate 与其余依赖保持 `opt-level="z"`**（分发体积不膨胀）。
- **验收（本地基准）**：写一个基准（criterion 或简单计时循环）测「1280×720 RGBA → Lanczos3 → JPEG q80」在 z vs 3 下的耗时倍数；记录客户端二进制**体积增量**（预算：可接受的 MB 级增量，具体在计划里定阈值）。
- **风险**：per-package opt 覆盖仅作用于该 crate 代码，若热点在 client 自身的胶水代码则收益有限——组件 1 的拆分会先告诉我们热点位置；若 360ms 主要在 `image` crate 内部（极可能），此招收益最大。

### 组件 3：自适应闭环（过载自愈）
把「过载判定」从纯观测变成闭环控制：弱机上自动降档（降分辨率/降帧/降质），恢复后回升；以用户手动档位为上限，只降不越。

- **纯状态机 `adaptive.rs`（可单测）**：
  ```
  AdaptiveController { level: u8 (0..=3), overload_streak: u8, healthy_streak: u8 }
  fn observe(&mut self, stats: &WindowStats) -> u8   // 返回新 level
  ```
  - **降档触发**：连续 **2** 窗命中「过载」（`enc_p95_ms > 200` 或 `effective_fps < 1.0 && dirty_p95 > 0.1`）→ `level += 1`（上限 3）；`overload_streak` 计数。
  - **回升触发**：连续 **3** 窗「健康」（无过载 且 `enc_p95_ms < 120` 留裕度防抖）→ `level -= 1`（下限 0）。**回升比降档慢**（3 vs 2 窗）= 更安全。
  - 迟滞：降档清零 healthy_streak，回升清零 overload_streak。
- **降档梯度 `clamp(params, level)`（纯函数，可单测）**——分辨率对 JPEG 耗时影响最大（≈像素数线性），优先降分辨率：

  | level | 分辨率上限 | jpeg_q | interval 倍数 | 意图 |
  |---|---|---|---|---|
  | 0 | 不钳 | 不钳 | ×1.0 | 用当前手动档 |
  | 1 | ×0.85 | −0 | ×1.25 | 轻降：小缩分辨率+略降帧 |
  | 2 | ×0.70 | −8 | ×1.5 | 中降 |
  | 3 | ×0.55 | −12 | ×2.0 | 重降：保「能动」为先 |

  以 `clamp_params` 已解析出的档位为**输入上限**，自适应只在其下降（永不超过用户手动 SetQuality/高清档）。与 LowBandwidth 钳制正交叠加（先手动/render_mode 钳，再自适应钳）。
- **跨线程桥接**：新增 `AtomicU8 ADAPTIVE_LEVEL`。collector（async，持 `WindowStats`）每窗 `observe` 后写原子量；抓帧线程 `workers.rs:271` 在 `clamp_params(...)` 后再 `adaptive::clamp(_, level)` 读原子量折入。
- **开关 + 可观测**：env `OHMYDESK_ADAPTIVE=0` / `config.toml [render] adaptive=false` 可秒关，**默认 ON**（呼应项目「随时可回滚」文化）。每次 level 变化 `info!` 记录；`format_log` 增 `adapt_level=` 字段，让日志能解释「为何 fps/分辨率降了」。

## 数据流

`抓帧线程`每帧产 `FrameSample{resize_ms,jpeg_ms,...}` → `collector` 10s 窗 `aggregate` → `WindowStats{resize_*,jpeg_*,enc_*}` → `classify` 过载 + `AdaptiveController.observe` → 写 `ADAPTIVE_LEVEL` 原子量 → 抓帧线程下一帧 `adaptive::clamp` 折入 → 降档生效。日志行同时输出 `resize_avg_ms/jpeg_avg_ms/adapt_level` 供真机取证。

## 测试策略

- **组件 1**：`aggregate` 单测断言含 `resize_*/jpeg_*` 字段并计算正确；`format_log` 单测断言日志含 `resize_avg_ms=`、`jpeg_avg_ms=`。
- **组件 2**：基准测试证明 encode 提速倍数（记录 z vs 3）；二进制体积断言在预算内。
- **组件 3**：`AdaptiveController.observe` 纯逻辑单测——合成窗口序列断言迟滞（2 窗降 / 3 窗升）、level 边界 [0,3]、`clamp` 不越手动上限、`OHMYDESK_ADAPTIVE=0` 时 observe 恒返回 0（旁路）。
- **回归**：既有 `分类_编码过载`/`format_log含关键字段` 等单测（`telemetry.rs:329,393`）随字段扩展更新，三端测试保持全绿。

## 非目标（YAGNI / 后续 spec）

- **turbojpeg（libjpeg-turbo SIMD）**：单帧编码可快 3–5×，但引入 C 依赖 + Windows(gnu)/龙芯交叉编译适配。**待组件 1 证明编码主导再上**。
- **tile 区域编码**：只编码/发送变化瓦片，需协议加 `x/y/w/h` 局部帧 + 主控端画布合成 + 三端联改。改动最大，另立 spec。
- **硬件/帧间编码（H.264/VP8）**：梯队 3，本 spec 不碰。
- **多线程分条并行编码**：暂不做（先看构建修 + 自适应是否已够）。

## 验收标准

1. 真机日志出现 `resize_avg_ms/jpeg_avg_ms/adapt_level` 三字段，能定位 360ms 主导项。
2. 本地基准显示 image 热路径在 opt=3 下相对 opt=z 有明确提速倍数（记录数值）；二进制体积增量在预算内。
3. 合成过载场景下自适应状态机按迟滞降档（2 窗）/回升（3 窗），不越手动档上限，开关可关。
4. 三端测试全绿；`OHMYDESK_ADAPTIVE=0` 时行为回到改造前（纯观测）。
