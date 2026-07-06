# 分辨率 / 清晰度 / 帧率 三轴拆分设计

> 日期：2026-07-06
> 状态：设计已确认（方案 A：扩展 SetQuality），待实现
> 前置调研：本文件基于对 `src/protocol`、`src/client`、`src/server`、`src/admin-web` 的实码定位，行号以 0.5.0 master 为准。

## 1. 背景与问题

当前远控画质由单一枚举 `QualityMode { HighQuality, Smooth }` 控制（`src/protocol/src/lib.rs:150-153`），被控端 `params_for(mode)`（`src/client/src/capture.rs:53-68`）把一个档位炸成四个参数：

| 档位 | 分辨率上限 | JPEG 质量 | 推帧间隔 |
|------|-----------|----------|---------|
| HighQuality（高清） | 1920×1080 | q88 | 100ms（~10fps） |
| Smooth（流畅，默认） | 1280×720 | q80 | 40ms（~25fps） |

**问题**：分辨率（显示大小）、清晰度（压缩质量）、帧率（顺滑度）三种正交职责被捆进一个开关。用户想要"高分辨率但省带宽"或"标准分辨率但高帧率"都做不到。

## 2. 目标

把一个档位拆成三个**独立**用户控件，两个主控端（Slint 客户端 + WEB 管理端）都提供：

| 轴 | 档位 | 落地参数 |
|----|------|---------|
| **分辨率** | 1280×720 / 1600×900 / 1920×1080 / 原始 | 采集缩放上限 `max_w × max_h`（fit-within 等比，绝不放大；"原始"= 不缩放） |
| **清晰度** | 标准 / 高清 | JPEG 质量 q80 / q88 |
| **帧率** | 流畅 / 标准 / 省流 | 推帧间隔 40ms(~25fps) / 66ms(~15fps) / 125ms(~8fps) |

**默认组合** = 1280×720 + 标准 + 流畅，与现状 Smooth 档行为完全一致（升级零感知）。

非目标：
- 不做 q≥90（jpeg-encoder 切 4:4:4 帧体积 ~1.5-2×，已被真机否决，见 `capture.rs:50-52` 注释）。
- 不改 adaptive 自适应算法本身，只适配组合顺序。
- 不改帧传输通道、frameskip、懒推流（SetCapture）机制。

## 3. 协议设计（方案 A：兼容扩展 SetQuality）

### 3.1 新增三个档位枚举（`src/protocol/src/lib.rs`，ts-rs 导出）

```rust
/// 分辨率档位：采集缩放上限。Native = 不缩放（按被控真实屏发送）。
#[derive(..., Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTier { R720p, R900p, R1080p, Native }

/// 清晰度档位：JPEG 编码质量。Standard=q80，High=q88。
#[serde(rename_all = "snake_case")]
pub enum ClarityTier { Standard, High }

/// 帧率档位：推帧间隔。Smooth=40ms，Standard=66ms，Saver=125ms。
#[serde(rename_all = "snake_case")]
pub enum FpsTier { Smooth, Standard, Saver }
```

### 3.2 扩展 `Message::SetQuality`（`lib.rs:228-231`）

```rust
SetQuality {
    session_id: String,
    /// 旧档位字段，保留供旧被控端兜底；新主控端按「清晰度」映射填写：
    /// ClarityTier::High → HighQuality，Standard → Smooth。
    mode: QualityMode,
    /// 三轴新字段。旧主控端不发（None），新被控端回退到 mode 的旧映射。
    #[serde(default)]
    resolution: Option<ResolutionTier>,
    #[serde(default)]
    clarity: Option<ClarityTier>,
    #[serde(default)]
    fps: Option<FpsTier>,
}
```

### 3.3 兼容矩阵（依赖已有 `Unknown` 兜底 + serde 忽略未知字段）

| 主控 → 被控 | 行为 |
|------------|------|
| 新 → 新 | 三轴字段全生效 |
| 新 → 旧 | 旧被控 serde 忽略未知字段，只读 `mode`（按清晰度映射）→ 退化为现状两档，不崩 |
| 旧 → 新 | 只带 `mode`，新被控把 `mode` 映射为三轴组合：HighQuality → (1080p, 高清, 帧率标准/66ms)，Smooth → (720p, 标准, 流畅/40ms)。注：旧高清档间隔 100ms 在新三档中不存在，取最近档 66ms（同分辨率同质量下帧率 10→15fps，属改善方向；过载由 adaptive 兜底） |
| 服务端 | `hub.rs:403-409` 对 SetQuality 原样透传 raw 文本、不解析 payload → **零改动** |

### 3.4 契约同步

- 改协议后必须 `cargo test -p protocol` 重导出 TS 到 `src/admin-web/src/lib/types/`（项目 D 组规范）。
- 新增生成物：`ResolutionTier.ts` / `ClarityTier.ts` / `FpsTier.ts`；`Message.ts` 的 `set_quality` 变体新增三个可选字段。

## 4. 被控端改造（src/client）

### 4.1 capture.rs — 档位存储与参数合成

- 现状：单原子 `QUALITY: AtomicU8`（`capture.rs:23`）+ `params_for(mode)`。
- 改为：**单个 `AtomicU32` 打包三轴档位**（各占 1 字节），保持推帧线程 `quality_changed` 判断只读一个原子、无撕裂：

```rust
static TIERS: AtomicU32 = AtomicU32::new(DEFAULT_TIERS); // res | clarity<<8 | fps<<16

pub fn set_tiers(res: ResolutionTier, clarity: ClarityTier, fps: FpsTier);
pub fn tiers_u32() -> u32;  // 替代 quality_u8()，供 quality_changed 判断

/// 三轴 → 采集参数（纯函数，单测覆盖全组合）
pub fn params_for_tiers(res, clarity, fps) -> QualityParams {
    QualityParams {
        max_w/max_h: 匹配 res（Native ⇒ u32::MAX，scaled_dims 自然不缩放不放大）,
        jpeg_q: Standard=80 / High=88,
        interval_ms: Smooth=40 / Standard=66 / Saver=125,
    }
}
```

- `QualityParams` 结构体不变 → 下游 `workers.rs` 组合链（档位 → render_mode clamp → adaptive clamp，`workers.rs:270-278`）**接口不变**。
- 坐标反算单一事实源 `current_frame_dims`（`capture.rs:84-87`）与 `last_frame_dims` 机制不变，自动跟随新参数。
- 旧 `params_for(mode)` 保留为 `mode → 三轴兜底组合 → params_for_tiers` 的薄封装（兼容旧主控路径）。注意：HighQuality 兜底后 interval 100ms→66ms（见 §3.3），现有断言 100ms 的单测需同步改为 66ms 并注明原因。

### 4.2 net/dispatch.rs — SetQuality handler（`dispatch.rs:212-231`）

- 读新字段：`resolution/clarity/fps` 任一为 `Some` 则按三轴设置；全 `None`（旧主控）则按 `mode` 旧映射。
- 继续调 `adaptive::request_reset()`（`adaptive.rs:38-41`）——手动切档立即清自适应降级，沿用已修的"弱机切高清被拉回"防护。

### 4.3 adaptive.rs / render_mode.rs — 不改算法

- adaptive 夹的是组合后的 `QualityParams`，与档位来源无关 → 算法零改。
- 注意点：`render_mode.rs:64-71` LowBandwidth 强制 ≤1280×720、q≤80——用户选"原始分辨率+高清"且处于 LowBandwidth 时会被夹紧。**行为保留**（保护弱网），但主控端 UI 无从感知。V1 接受此限制；帧 w/h 顶栏回显（已有）可让用户看到实际生效分辨率。

## 5. Slint 主控端改造（src/client/ui + ui_glue.rs）

- 现状：`app.slint:1088-1137` 顶部标签条「流畅/高清」二选一按钮组；`ui_glue.rs:215-232` `on_set_quality(bool)`。
- 改为三个控件（顶栏空间有限，用紧凑分段按钮或下拉）：
  - 分辨率：下拉/循环按钮 4 档（720p / 900p / 1080p / 原始）
  - 清晰度：二段按钮（标准 / 高清）
  - 帧率：三段按钮（流畅 / 标准 / 省流）
- 回调换成 `set_display_params(res: int, clarity: int, fps: int)` 单回调（避免三回调竞态半套参数），Rust 侧组包发 `FromUi::SetQuality`（`net/mod.rs:153-157` 同步扩三字段）→ `dispatch.rs:685-690` 序列化处带上三轴 + 映射后的 `mode`。
- ⚠️ Slint DSL 是语料盲区：编码前先查项目 skill `rust-remote-control-stack` 确认 ComboBox/按钮组当前写法。

## 6. WEB 主控端改造（src/admin-web)

- `remote-session.tsx:235-253`：现「流畅/高清」分段按钮 → 三组控件（shadcn 风格与现有一致：分辨率下拉 + 清晰度二段 + 帧率三段）。
- `store.ts`：
  - `remoteQuality` 字段（`store.ts:82-83,133,383`）→ 拆为 `remoteResolution / remoteClarity / remoteFps` 三字段（默认 `"r720p" / "standard" / "smooth"`）。
  - `setRemoteQuality`（`store.ts:499-505`）→ `setRemoteDisplayParams(partial)`，合并三字段后发一条 `set_quality` 信封（带 mode 映射 + 三轴字段）。
- 类型只用 ts-rs 生成物，禁手写（D 组规范）。

## 7. 数据流（改后）

```
[主控 UI 三控件] → set_quality{mode, resolution, clarity, fps}
  → server hub.rs 原样透传（零改）
  → 被控 dispatch：三轴 Some ⇒ set_tiers；全 None ⇒ mode 旧映射
  → capture 推帧线程每帧读 TIERS → params_for_tiers
  → render_mode clamp → adaptive clamp（不变）
  → scaled_dims 缩放 + jpeg_q 编码 + interval 节流
  → Frame{w,h} 回传，主控顶栏回显实际分辨率（已有）
```

## 8. 测试计划

1. **单元**（cargo test -p client）：
   - `params_for_tiers` 参数合成测试；Native 不放大（小屏 800×600 原样发）。实现注：因三轴映射相互独立，参数值改为按轴抽样断言，24 全组合覆盖由 pack/unpack 往返测试承担。
   - 旧 `params_for(mode)` 兜底映射：分辨率/jpeg_q 与旧值一致；interval 按 §3.3 调整（Smooth=40ms 不变，HighQuality 100→66ms）。
   - TIERS 打包/解包往返。
2. **协议契约**（cargo test -p protocol）：
   - 旧 JSON（只有 mode）反序列化 → 三字段 None。
   - 新 JSON 带三字段 → 正确解析；TS 重导出 diff 检查。
3. **组合链**：三轴 + adaptive clamp + LowBandwidth clamp 叠加顺序不变（沿用 workers 现测）。
4. **真机必测**（历史坑）：
   - 切分辨率后**点击坐标不偏**（`last_frame_dims` 路径，P-CLI4 回归）。
   - 弱机切"原始+高清"不被 adaptive 立即拉回（request_reset 生效）。
   - 新主控 ↔ 旧被控（0.5.0）互通：切档退化为两档但不崩、不断流。
5. **WEB E2E**：三控件切换 → 顶栏分辨率回显变化。

## 9. 风险与对策

| 风险 | 等级 | 对策 |
|------|------|------|
| 切分辨率后点击偏移 | 高（历史踩过） | 单一事实源机制未动；真机回归必测 |
| Slint DSL 写成过时 API | 中 | 编码前查 `rust-remote-control-stack` skill |
| "原始"分辨率在 4K 屏上帧体积暴涨 → 卡顿 | 中 | adaptive 闭环兜底会降档；UI 顶栏回显实际尺寸；文档标注"原始档吃带宽" |
| 三控件顶栏放不下（Slint 窗口窄） | 低 | 紧凑分段/下拉；必要时收进设置弹层 |
| 旧被控收新消息行为退化被误认为 bug | 低 | 退化映射与现状一致；发版说明标注 |

## 10. 工作量估算

| 层 | 估时 |
|----|------|
| 协议 + ts-rs 重导出 | 0.5d |
| 被控端 capture/dispatch/组合链 | 1.3d |
| Slint 主控 UI | 1d |
| WEB 主控 UI + store | 0.7d |
| 服务端 | 0 |
| 测试（单测+契约+真机） | 1d |
| **合计** | **~4.5d** |
