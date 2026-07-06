# 分辨率/清晰度/帧率三轴拆分 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把单一 `QualityMode {HighQuality, Smooth}` 档位拆成三个独立控件（分辨率 720p/900p/1080p/原生、清晰度 标准/高清、帧率 流畅/标准/省流），Slint 客户端与 WEB 管理端两个主控端都提供，协议向后兼容。

**Architecture:** 协议走方案 A——`Message::SetQuality` 兼容扩展三个 `Option` 轴字段（旧端靠 serde 忽略未知字段 + `mode` 兜底）；被控端把单原子档位换成打包三轴的 `AtomicU32`，`params_for_tiers` 纯函数合成 `QualityParams`；服务端原样透传零改动。设计依据：`docs/superpowers/specs/2026-07-06-resolution-clarity-fps-split-design.md`。

**Tech Stack:** Rust（serde + ts-rs 10 协议、Slint UI、原子档位）、React + zustand + Tailwind（admin-web）。

**约定：**
- 所有命令在仓库根目录执行。
- Rust 提交前 `cargo fmt`；每个 Task 结束 commit（简体中文 commit message）。
- 协议改动后 `cargo test -p protocol` 会经 ts-rs 重新导出 TS 到 `src/admin-web/src/lib/types/`，生成物**禁止手改**。

---

### Task 1: 协议层——三个档位枚举 + SetQuality 兼容扩展

**Files:**
- Modify: `src/protocol/src/lib.rs`（`QualityMode` 之后、`Message` 的 `SetQuality` 变体）
- Modify: `src/protocol/src/tests.rs`（新增 2 个契约测试）
- Modify: `src/client/src/net/dispatch.rs:212`、`src/client/src/net/dispatch.rs:685-690`（最小改动保持 workspace 编译绿，逻辑留 Task 4）
- 生成: `src/admin-web/src/lib/types/ResolutionTier.ts` / `ClarityTier.ts` / `FpsTier.ts` / `Message.ts`（ts-rs 自动）

- [ ] **Step 1: 写失败的契约测试**

在 `src/protocol/src/tests.rs` 末尾追加：

```rust
#[test]
fn set_quality_旧json_三轴字段缺省为none() {
    // 旧主控（≤0.5.0）只发 mode：新被控必须能解析且三轴为 None（回退 mode 旧映射）。
    let json = r#"{"from":"admin-1","to":null,"ts":1719500000,"payload":{"type":"set_quality","session_id":"s-1","mode":"high_quality"}}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    match env.payload {
        Message::SetQuality { mode, resolution, clarity, fps, .. } => {
            assert_eq!(mode, QualityMode::HighQuality);
            assert!(resolution.is_none() && clarity.is_none() && fps.is_none());
        }
        _ => panic!("应判别为 SetQuality"),
    }
}

#[test]
fn set_quality_三轴字段_序列化往返() {
    let env = Envelope {
        from: "admin-1".into(),
        to: None,
        ts: 1719500000,
        payload: Message::SetQuality {
            session_id: "s-1".into(),
            mode: QualityMode::Smooth,
            resolution: Some(ResolutionTier::Native),
            clarity: Some(ClarityTier::High),
            fps: Some(FpsTier::Saver),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"resolution\":\"native\""), "snake_case 序列化: {json}");
    assert!(json.contains("\"clarity\":\"high\""));
    assert!(json.contains("\"fps\":\"saver\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back.payload,
        Message::SetQuality { resolution: Some(ResolutionTier::Native), clarity: Some(ClarityTier::High), fps: Some(FpsTier::Saver), .. }
    ));
}
```

- [ ] **Step 2: 跑测试确认失败（编译错：类型不存在）**

Run: `cargo test -p protocol set_quality 2>&1 | tail -20`
Expected: 编译失败，`cannot find type ResolutionTier`。

- [ ] **Step 3: 实现协议扩展**

`src/protocol/src/lib.rs`，紧跟 `QualityMode`（153 行 `}` 之后）插入：

```rust
/// 分辨率档位：采集缩放上限（fit-within 等比，绝不放大）。Native=不缩放，按被控真实屏发送。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTier {
    R720p,
    R900p,
    R1080p,
    Native,
}

/// 清晰度档位：JPEG 编码质量。Standard=q80，High=q88（q≥90 切 4:4:4 体积翻倍，真机已否决）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ClarityTier {
    Standard,
    High,
}

/// 帧率档位：推帧间隔。Smooth=40ms(~25fps)，Standard=66ms(~15fps)，Saver=125ms(~8fps)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FpsTier {
    Smooth,
    Standard,
    Saver,
}
```

`Message::SetQuality` 变体（lib.rs:226-231）替换为：

```rust
    /// 主控→被控：设置显示参数。三轴新字段（v0.5.x 起）独立控制分辨率/清晰度/帧率；
    /// mode 旧字段保留兼容 ≤0.5.0 被控端（新主控按清晰度映射填写：High→HighQuality）。
    /// 旧主控不发三轴（None），新被控按 mode 兜底展开。按 session 对端路由（同 Input）。
    SetQuality {
        session_id: String,
        mode: QualityMode,
        #[serde(default)]
        resolution: Option<ResolutionTier>,
        #[serde(default)]
        clarity: Option<ClarityTier>,
        #[serde(default)]
        fps: Option<FpsTier>,
    },
```

- [ ] **Step 4: 修 client 两处编译断点（最小改动，逻辑留 Task 4）**

`src/client/src/net/dispatch.rs:212` 模式匹配加 `..`：

```rust
        Message::SetQuality { session_id, mode, .. } => {
```

`src/client/src/net/dispatch.rs:685-690` 构造处补 None：

```rust
        FromUi::SetQuality { session_id, mode } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给被控端
            ts: now(),
            payload: Message::SetQuality {
                session_id,
                mode,
                resolution: None,
                clarity: None,
                fps: None,
            },
        },
```

再 grep 确认无其他构造/解构点遗漏：

Run: `grep -rn "SetQuality" src/ --include="*.rs" | grep -v "protocol/src"`
Expected: 只有 dispatch.rs 上述两处 + net/mod.rs 的 FromUi 定义 + ui_glue.rs 发送处（FromUi 不含新字段，暂不动）+ server/hub.rs（已是 `{ session_id, .. }` 不用改）。有遗漏则同法补齐。

- [ ] **Step 5: 跑协议测试 + 全 workspace 编译**

Run: `cargo test -p protocol 2>&1 | tail -5 && cargo build --workspace 2>&1 | tail -3`
Expected: 协议测试全 PASS（含新增 2 个 + ts-rs 自动导出测试）；workspace 编译通过。

- [ ] **Step 6: 确认 TS 生成物落地**

Run: `ls src/admin-web/src/lib/types/ | grep -E "Resolution|Clarity|Fps" && grep -n "resolution" src/admin-web/src/lib/types/Message.ts`
Expected: 三个新 .ts 文件存在；`Message.ts` 的 set_quality 变体含 `resolution` / `clarity` / `fps` 可空字段。

- [ ] **Step 7: Commit**

```bash
git add src/protocol/ src/client/src/net/dispatch.rs src/admin-web/src/lib/types/
git commit -m "feat(protocol): SetQuality 兼容扩展三轴字段（分辨率/清晰度/帧率）

新增 ResolutionTier/ClarityTier/FpsTier 枚举，SetQuality 加三个
Option 轴字段（serde default），旧端忽略未知字段+mode 兜底不破坏。
ts-rs 重导出 TS 契约。"
```

---

### Task 2: 被控端采集——三轴打包原子 + params_for_tiers

**Files:**
- Modify: `src/client/src/capture.rs`（档位存储区 20-78 行 + 测试区 352-385 行）

- [ ] **Step 1: 写失败的单元测试**

`src/client/src/capture.rs` 测试模块（`占位帧_可解码为_1280x720_rgba` 之后）追加：

```rust
    #[test]
    fn 三轴参数_合成正确() {
        use protocol::{ClarityTier, FpsTier, ResolutionTier};
        let p = params_for_tiers(ResolutionTier::R900p, ClarityTier::High, FpsTier::Saver);
        assert_eq!((p.max_w, p.max_h, p.jpeg_q, p.interval_ms), (1600, 900, 88, 125));
        // 默认组合 = 旧 Smooth 档，升级零感知
        let p = params_for_tiers(ResolutionTier::R720p, ClarityTier::Standard, FpsTier::Smooth);
        assert_eq!((p.max_w, p.max_h, p.jpeg_q, p.interval_ms), (1280, 720, 80, 40));
        // 原生档：不缩放也不放大（fit_scale 恒 ≤1.0）
        let p = params_for_tiers(ResolutionTier::Native, ClarityTier::Standard, FpsTier::Smooth);
        assert_eq!(crate::geom::scaled_dims(800, 600, p.max_w, p.max_h), (800, 600));
        assert_eq!(crate::geom::scaled_dims(3840, 2160, p.max_w, p.max_h), (3840, 2160));
    }

    #[test]
    fn 三轴打包_全组合往返一致() {
        use protocol::{ClarityTier, FpsTier, ResolutionTier};
        let all_r = [ResolutionTier::R720p, ResolutionTier::R900p, ResolutionTier::R1080p, ResolutionTier::Native];
        let all_c = [ClarityTier::Standard, ClarityTier::High];
        let all_f = [FpsTier::Smooth, FpsTier::Standard, FpsTier::Saver];
        for r in all_r {
            for c in all_c {
                for f in all_f {
                    set_tiers(r, c, f);
                    assert_eq!(current_params(), params_for_tiers(r, c, f), "{r:?}/{c:?}/{f:?}");
                }
            }
        }
        set_quality(protocol::QualityMode::Smooth); // 复位，避免污染其它测试
    }

    #[test]
    fn tiers_from_set_quality_缺失轴按mode回退() {
        use protocol::{ClarityTier, FpsTier, QualityMode, ResolutionTier};
        // 全缺失（旧主控）→ mode 兜底组合
        let (r, c, f) = tiers_from_set_quality(QualityMode::HighQuality, None, None, None);
        assert_eq!((r, c, f), (ResolutionTier::R1080p, ClarityTier::High, FpsTier::Standard));
        let (r, c, f) = tiers_from_set_quality(QualityMode::Smooth, None, None, None);
        assert_eq!((r, c, f), (ResolutionTier::R720p, ClarityTier::Standard, FpsTier::Smooth));
        // 部分缺失 → 提供的轴优先，缺失轴回退
        let (r, c, f) = tiers_from_set_quality(QualityMode::Smooth, Some(ResolutionTier::Native), None, Some(FpsTier::Saver));
        assert_eq!((r, c, f), (ResolutionTier::Native, ClarityTier::Standard, FpsTier::Saver));
    }
```

同时更新既有测试 `画质档位_参数符合预期`（capture.rs:352-360）——旧高清档 100ms 兜底后变 66ms（spec §3.3）：

```rust
    #[test]
    fn 画质档位_参数符合预期() {
        let hq = params_for(protocol::QualityMode::HighQuality);
        assert_eq!((hq.max_w, hq.max_h, hq.jpeg_q), (1920, 1080, 88));
        // 旧 100ms 间隔在新三档(40/66/125)中不存在，兜底取最近档 66ms（spec §3.3，帧率 10→15fps 属改善向）
        assert_eq!(hq.interval_ms, 66);
        let sm = params_for(protocol::QualityMode::Smooth);
        assert_eq!((sm.max_w, sm.max_h, sm.jpeg_q), (1280, 720, 80));
        assert!(sm.interval_ms < hq.interval_ms, "流畅档帧率应高于高清档");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client 三轴 2>&1 | tail -10`
Expected: 编译失败，`cannot find function params_for_tiers`。

- [ ] **Step 3: 实现三轴档位存储与合成**

`src/client/src/capture.rs` 把 20-78 行（`QUALITY` 静态到 `current_params`）整体替换为：

```rust
/// 三轴显示档位打包存储（低→高字节：分辨率/清晰度/帧率），被控端推帧线程每帧读取。
/// 单原子保证三轴一次性生效（无撕裂），quality_changed 判断只比较一个 u32。
/// 0 = (R720p, Standard, Smooth) 默认组合 = 旧 Smooth 档，升级零感知。
static TIERS: AtomicU32 = AtomicU32::new(0);

/// 画质档位对应的采集参数：分辨率上限 + JPEG 质量 + 推帧间隔(ms)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityParams {
    pub max_w: u32,
    pub max_h: u32,
    pub jpeg_q: u8,
    pub interval_ms: u64,
}

fn res_u8(t: protocol::ResolutionTier) -> u8 {
    match t {
        protocol::ResolutionTier::R720p => 0,
        protocol::ResolutionTier::R900p => 1,
        protocol::ResolutionTier::R1080p => 2,
        protocol::ResolutionTier::Native => 3,
    }
}

fn clarity_u8(t: protocol::ClarityTier) -> u8 {
    match t {
        protocol::ClarityTier::Standard => 0,
        protocol::ClarityTier::High => 1,
    }
}

fn fps_u8(t: protocol::FpsTier) -> u8 {
    match t {
        protocol::FpsTier::Smooth => 0,
        protocol::FpsTier::Standard => 1,
        protocol::FpsTier::Saver => 2,
    }
}

fn unpack_tiers(v: u32) -> (protocol::ResolutionTier, protocol::ClarityTier, protocol::FpsTier) {
    let res = match v & 0xff {
        1 => protocol::ResolutionTier::R900p,
        2 => protocol::ResolutionTier::R1080p,
        3 => protocol::ResolutionTier::Native,
        _ => protocol::ResolutionTier::R720p,
    };
    let clarity = match (v >> 8) & 0xff {
        1 => protocol::ClarityTier::High,
        _ => protocol::ClarityTier::Standard,
    };
    let fps = match (v >> 16) & 0xff {
        1 => protocol::FpsTier::Standard,
        2 => protocol::FpsTier::Saver,
        _ => protocol::FpsTier::Smooth,
    };
    (res, clarity, fps)
}

/// 设置三轴档位（被控端收到主控 SetQuality 时调用）。
pub fn set_tiers(res: protocol::ResolutionTier, clarity: protocol::ClarityTier, fps: protocol::FpsTier) {
    let v = res_u8(res) as u32 | (clarity_u8(clarity) as u32) << 8 | (fps_u8(fps) as u32) << 16;
    TIERS.store(v, Ordering::Relaxed);
}

/// 旧入口（mode 两档）：保留供旧主控兜底路径与既有测试；内部展开为三轴。
pub fn set_quality(mode: protocol::QualityMode) {
    let (r, c, f) = tiers_for_mode(mode);
    set_tiers(r, c, f);
}

/// 三轴打包原子值，供推帧线程 quality_changed 判断（替代旧 quality_u8）。
pub fn tiers_u32() -> u32 {
    TIERS.load(Ordering::Relaxed)
}

/// mode 兜底映射：旧主控只发 mode 时按此展开三轴。
/// HighQuality→(1080p, 高清, 帧率标准/66ms)——旧 100ms 在新三档中不存在，取最近档（spec §3.3）；
/// Smooth→(720p, 标准, 流畅/40ms) 与旧值完全一致。
pub fn tiers_for_mode(mode: protocol::QualityMode) -> (protocol::ResolutionTier, protocol::ClarityTier, protocol::FpsTier) {
    match mode {
        protocol::QualityMode::HighQuality => (
            protocol::ResolutionTier::R1080p,
            protocol::ClarityTier::High,
            protocol::FpsTier::Standard,
        ),
        protocol::QualityMode::Smooth => (
            protocol::ResolutionTier::R720p,
            protocol::ClarityTier::Standard,
            protocol::FpsTier::Smooth,
        ),
    }
}

/// SetQuality 消息 → 最终三轴：新字段优先，缺失轴按 mode 兜底映射逐轴回退。
pub fn tiers_from_set_quality(
    mode: protocol::QualityMode,
    resolution: Option<protocol::ResolutionTier>,
    clarity: Option<protocol::ClarityTier>,
    fps: Option<protocol::FpsTier>,
) -> (protocol::ResolutionTier, protocol::ClarityTier, protocol::FpsTier) {
    let (dr, dc, df) = tiers_for_mode(mode);
    (resolution.unwrap_or(dr), clarity.unwrap_or(dc), fps.unwrap_or(df))
}

/// 三轴 → 采集参数（纯函数，便于单测）。
/// 高清 q 保持 88（4:2:0）：真机试过 q92（jpeg-encoder 以 q≥90 切 4:4:4）文字略清晰，但 4:4:4
/// 帧体积大 ~1.5–2×，拥塞公网上「切高清后首帧」传输慢 2–3s（切换延迟回归），得不偿失。
pub fn params_for_tiers(
    res: protocol::ResolutionTier,
    clarity: protocol::ClarityTier,
    fps: protocol::FpsTier,
) -> QualityParams {
    let (max_w, max_h) = match res {
        protocol::ResolutionTier::R720p => (1280, 720),
        protocol::ResolutionTier::R900p => (1600, 900),
        protocol::ResolutionTier::R1080p => (1920, 1080),
        // 不缩放：fit_scale 比例恒 ≤1.0 → scaled_dims 原样返回，encode_frame_q 跳过 resize
        protocol::ResolutionTier::Native => (u32::MAX, u32::MAX),
    };
    let jpeg_q = match clarity {
        protocol::ClarityTier::Standard => 80,
        protocol::ClarityTier::High => 88,
    };
    let interval_ms = match fps {
        protocol::FpsTier::Smooth => 40,
        protocol::FpsTier::Standard => 66,
        protocol::FpsTier::Saver => 125,
    };
    QualityParams { max_w, max_h, jpeg_q, interval_ms }
}

/// 档位 → 采集参数（旧签名保留：mode 兜底展开三轴后合成）。
pub fn params_for(mode: protocol::QualityMode) -> QualityParams {
    let (r, c, f) = tiers_for_mode(mode);
    params_for_tiers(r, c, f)
}

/// 取当前三轴档位的采集参数（推帧线程每帧调用）。
pub fn current_params() -> QualityParams {
    let (r, c, f) = unpack_tiers(TIERS.load(Ordering::Relaxed));
    params_for_tiers(r, c, f)
}
```

注意：
- 文件头 `use` 区把 `AtomicU8` 换成 `AtomicU32`（若 `AtomicU32` 已因 `LAST_FRAME_W` 引入则只删 `AtomicU8`）。
- `quality_u8()` 函数删除——调用点 `workers.rs:380` 在 Task 3 改为 `tiers_u32()`（本 Task 结束时 client 暂不编译，Task 3 立即修复；两 Task 同一次提交前完成）。

- [ ] **Step 4: 跑 capture 测试（预期 workers.rs 编译错，先行 Task 3 Step 1-2 后一起绿）**

Run: `cargo build -p client 2>&1 | grep -E "error|quality_u8"`
Expected: 仅 `workers.rs:380` 一处 `quality_u8` 未找到——确认无其他遗漏调用点后进入 Task 3（**本 Task 不单独 commit**，与 Task 3 合并提交）。

---

### Task 3: 推帧线程——tiers_u32 接线 + framediff u32 + Native 档 adaptive 生效

**Files:**
- Modify: `src/client/src/workers.rs:266-278`（组合链）、`src/client/src/workers.rs:380`（quality 读取）
- Modify: `src/client/src/framediff.rs:62`、`framediff.rs:71-107`（decide 签名 u8→u32）

- [ ] **Step 1: framediff quality 参数 u8→u32**

`src/client/src/framediff.rs:62` 字段类型改：

```rust
    last_quality: u32,
```

`framediff.rs:71-75` `decide` 签名中 `quality: u8` 改为 `quality: u32`（第 93/107 行的比较与赋值逻辑不变）。既有测试 `framediff.rs:217` 传字面量 `1`，u32 下无需改动。

- [ ] **Step 2: workers.rs 接线 tiers_u32**

`src/client/src/workers.rs:380` 改：

```rust
                let quality = capture::tiers_u32();
```

- [ ] **Step 3: Native 档让 adaptive 分辨率降级生效（真实屏尺寸先钳 ceiling）**

问题：Native 档 ceiling=u32::MAX，adaptive 的 `res_ratio × u32::MAX` 仍远大于真实屏 → 分辨率降级失效，弱机 4K 原生档过载只能靠降 q/帧率兜底，不够。
修法：推帧线程缓存上一帧真实屏尺寸，组合链在 adaptive **之前**用它钳一次 ceiling，让 ratio 作用在真实基数上。

`workers.rs:266` 附近（`let mut last_cap_ms: u64 = 0;` 之后）加循环外状态：

```rust
            let mut last_real: Option<(u32, u32)> = None; // 上一帧真实屏尺寸：Native 档钳 ceiling 用
```

`workers.rs:270-278` 组合链改为：

```rust
                let mode = crate::render_mode::current_mode();
                let mut qp = crate::render_mode::clamp_params(capture::current_params(), mode);
                // Native 档 ceiling=u32::MAX：先用真实屏尺寸钳到实际基数，adaptive 的 res_ratio
                // 才能在原生档继续降分辨率（否则 ratio×u32::MAX 恒大于屏，降级失效）。
                // 非 Native 档 min() 后不大于原 ceiling，fit_scale 不放大语义不变。
                if let Some((rw0, rh0)) = last_real {
                    qp.max_w = qp.max_w.min(rw0);
                    qp.max_h = qp.max_h.min(rh0);
                }
                // adaptive 仅作用于 frameskip 新路径：LegacyFullFrame/FullFrameWithTelemetry 是整帧基准，
                // 且 telemetry 关时 collector 不 observe→level 无法恢复，故不叠加，防状态泄漏。
                let qp = if crate::render_mode::frameskip_on() {
                    crate::adaptive::clamp(qp, crate::adaptive::level())
                } else {
                    qp
                };
```

`workers.rs:378`（`let (rw, rh) = (raw.width(), raw.height());` 之后）记录：

```rust
                last_real = Some((rw, rh));
```

- [ ] **Step 4: 跑 client 全部测试**

Run: `cargo test -p client 2>&1 | tail -8`
Expected: 全 PASS（含 Task 2 新增 3 测试、更新后的 `画质档位_参数符合预期`、framediff 既有测试、`注入帧尺寸_随档位变化` 回归测试）。

- [ ] **Step 5: Commit（Task 2+3 合并）**

```bash
cargo fmt
git add src/client/src/capture.rs src/client/src/workers.rs src/client/src/framediff.rs
git commit -m "feat(client): 被控端三轴档位存储与合成（分辨率/清晰度/帧率）

单原子 AtomicU8 档位 → AtomicU32 打包三轴；params_for_tiers 纯函数
合成采集参数；mode 兜底映射保旧主控兼容（高清间隔 100→66ms 取最近档）。
Native 档用真实屏尺寸先钳 ceiling，adaptive 分辨率降级在原生档继续生效。"
```

---

### Task 4: 被控端分发 + 主控端网络出站三轴透传

**Files:**
- Modify: `src/client/src/net/mod.rs:153-157`（FromUi::SetQuality 加三字段）
- Modify: `src/client/src/net/dispatch.rs:212-231`（被控 handler 读三轴）、`dispatch.rs:685-690`（出站带三轴）
- Modify: `src/client/src/ui_glue.rs:219-231`（临时补 None，Task 5 换新回调）

- [ ] **Step 1: FromUi::SetQuality 扩展**

`src/client/src/net/mod.rs:153-157` 替换为：

```rust
    /// 主控端切换三轴显示参数（分辨率/清晰度/帧率）→ 发 SetQuality 给被控端。
    /// mode 为旧被控端兜底字段（按清晰度映射）；三轴 None 时对端回退 mode 旧映射。
    SetQuality {
        session_id: String,
        mode: protocol::QualityMode,
        resolution: Option<protocol::ResolutionTier>,
        clarity: Option<protocol::ClarityTier>,
        fps: Option<protocol::FpsTier>,
    },
```

- [ ] **Step 2: 出站转换带三轴**

`src/client/src/net/dispatch.rs:685-690`（Task 1 已补 None 的那处）替换为：

```rust
        FromUi::SetQuality { session_id, mode, resolution, clarity, fps } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给被控端
            ts: now(),
            payload: Message::SetQuality { session_id, mode, resolution, clarity, fps },
        },
```

- [ ] **Step 3: 被控 handler 按三轴落地**

`src/client/src/net/dispatch.rs:212-231` 替换为：

```rust
        // 被控端收主控切换的显示参数 → 更新采集三轴（仅本会话被控态时生效）
        Message::SetQuality { session_id, mode, resolution, clarity, fps } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            tracing::info!(
                "被控收到画质切换 mode={mode:?} res={resolution:?} clarity={clarity:?} fps={fps:?} controlled={controlled} session={session_id}"
            );
            if controlled {
                let (r, c, f) = crate::capture::tiers_from_set_quality(mode, resolution, clarity, fps);
                crate::capture::set_tiers(r, c, f);
                // 手动切档立即重置自适应降档，让用户选择先生效（弱机上避免被 adaptive 立即拉回）。
                crate::adaptive::request_reset();
                let p = crate::capture::current_params();
                tracing::info!(
                    "被控已应用画质 上限={}x{} q={} 间隔={}ms",
                    p.max_w,
                    p.max_h,
                    p.jpeg_q,
                    p.interval_ms
                );
            }
        }
```

- [ ] **Step 4: ui_glue 临时补 None 保编译（Task 5 换新回调）**

`src/client/src/ui_glue.rs:226-229` 的 `FromUi::SetQuality` 构造加三个 `None` 字段：

```rust
                let _ = tx.send(net::FromUi::SetQuality {
                    session_id: sid,
                    mode,
                    resolution: None,
                    clarity: None,
                    fps: None,
                });
```

- [ ] **Step 5: 编译 + 测试 + Commit**

Run: `cargo test -p client 2>&1 | tail -5`
Expected: 全 PASS。

```bash
cargo fmt
git add src/client/src/net/ src/client/src/ui_glue.rs
git commit -m "feat(client): SetQuality 三轴字段全链路透传（FromUi→协议→被控落地）"
```

---

### Task 5: Slint 主控端——三组分段控件

**Files:**
- Modify: `src/client/ui/app.slint:610-611`（属性/回调）、`app.slint:1088-1137`（控件区）
- Modify: `src/client/src/ui_glue.rs:215-232`（回调实现）

⚠️ Slint DSL 是语料盲区。本 Task 只复用文件内已有写法（`Rectangle`+`TouchArea` 分段按钮、`for x[i] in [...]` 循环、`Theme.emerald`），不引入新 widget。若 `for label[idx] in [...]` 语法编译报错，先查项目 skill `rust-remote-control-stack` 再修正。

- [ ] **Step 1: 属性与回调替换**

`src/client/ui/app.slint:610-611` 替换为：

```slint
    // ── 三轴显示参数（主控选择，发 SetQuality 给被控端）──
    in-out property <int> res_tier: 0;      // 分辨率 0=720p 1=900p 2=1080p 3=原生
    in-out property <int> clarity_tier: 0;  // 清晰度 0=标准 1=高清
    in-out property <int> fps_tier: 0;      // 帧率 0=流畅 1=标准 2=省流
    callback set_display_params(int /*res*/, int /*clarity*/, int /*fps*/);
```

grep 确认旧属性无其他引用点：

Run: `grep -n "high_quality\|set_quality" src/client/ui/*.slint src/client/src/ui_glue.rs`
Expected: 仅 app.slint 1088-1137 控件区（Step 2 一起换）与 ui_glue.rs 215-232（Step 3 换）。有其他引用则同步改名。

- [ ] **Step 2: 控件区替换（1088-1137 的画质切换块）**

替换为三组分段按钮（同款 `Rectangle`+`TouchArea` 写法）：

```slint
                    // 三轴显示参数（分辨率/清晰度/帧率），常驻标签条右侧随时可切
                    VerticalLayout {
                        alignment: center;
                        Rectangle {
                            width: 172px;
                            height: 28px;
                            border-radius: 8px;
                            background: #1a1a1edd;
                            HorizontalLayout {
                                padding: 2px;
                                spacing: 2px;
                                for r-label[r-idx] in ["720", "900", "1080", "原生"]: Rectangle {
                                    border-radius: 6px;
                                    background: root.res_tier == r-idx ? Theme.emerald : transparent;
                                    Text {
                                        text: r-label;
                                        color: white;
                                        font-size: 11px;
                                        horizontal-alignment: center;
                                        vertical-alignment: center;
                                    }
                                    TouchArea {
                                        mouse-cursor: pointer;
                                        clicked => {
                                            root.res_tier = r-idx;
                                            root.set_display_params(root.res_tier, root.clarity_tier, root.fps_tier);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    VerticalLayout {
                        alignment: center;
                        Rectangle {
                            width: 92px;
                            height: 28px;
                            border-radius: 8px;
                            background: #1a1a1edd;
                            HorizontalLayout {
                                padding: 2px;
                                spacing: 2px;
                                for c-label[c-idx] in ["标准", "高清"]: Rectangle {
                                    border-radius: 6px;
                                    background: root.clarity_tier == c-idx ? Theme.emerald : transparent;
                                    Text {
                                        text: c-label;
                                        color: white;
                                        font-size: 11px;
                                        horizontal-alignment: center;
                                        vertical-alignment: center;
                                    }
                                    TouchArea {
                                        mouse-cursor: pointer;
                                        clicked => {
                                            root.clarity_tier = c-idx;
                                            root.set_display_params(root.res_tier, root.clarity_tier, root.fps_tier);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    VerticalLayout {
                        alignment: center;
                        Rectangle {
                            width: 134px;
                            height: 28px;
                            border-radius: 8px;
                            background: #1a1a1edd;
                            HorizontalLayout {
                                padding: 2px;
                                spacing: 2px;
                                for f-label[f-idx] in ["流畅", "标准", "省流"]: Rectangle {
                                    border-radius: 6px;
                                    background: root.fps_tier == f-idx ? Theme.emerald : transparent;
                                    Text {
                                        text: f-label;
                                        color: white;
                                        font-size: 11px;
                                        horizontal-alignment: center;
                                        vertical-alignment: center;
                                    }
                                    TouchArea {
                                        mouse-cursor: pointer;
                                        clicked => {
                                            root.fps_tier = f-idx;
                                            root.set_display_params(root.res_tier, root.clarity_tier, root.fps_tier);
                                        }
                                    }
                                }
                            }
                        }
                    }
```

- [ ] **Step 3: ui_glue 回调实现**

`src/client/src/ui_glue.rs:215-232`（`on_set_quality` 块）替换为：

```rust
    // 主控切换三轴显示参数（分辨率/清晰度/帧率）→ 发 SetQuality 给被控端
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_set_display_params(move |res, clarity, fps| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let resolution = match res {
                    1 => protocol::ResolutionTier::R900p,
                    2 => protocol::ResolutionTier::R1080p,
                    3 => protocol::ResolutionTier::Native,
                    _ => protocol::ResolutionTier::R720p,
                };
                let clarity_t = match clarity {
                    1 => protocol::ClarityTier::High,
                    _ => protocol::ClarityTier::Standard,
                };
                let fps_t = match fps {
                    1 => protocol::FpsTier::Standard,
                    2 => protocol::FpsTier::Saver,
                    _ => protocol::FpsTier::Smooth,
                };
                // 旧被控端（≤0.5.0）兜底：mode 按清晰度映射
                let mode = if matches!(clarity_t, protocol::ClarityTier::High) {
                    protocol::QualityMode::HighQuality
                } else {
                    protocol::QualityMode::Smooth
                };
                let _ = tx.send(net::FromUi::SetQuality {
                    session_id: sid,
                    mode,
                    resolution: Some(resolution),
                    clarity: Some(clarity_t),
                    fps: Some(fps_t),
                });
            }
        });
    }
```

- [ ] **Step 4: 编译 + 手动冒烟**

Run: `cargo build -p client 2>&1 | tail -3 && cargo test -p client 2>&1 | tail -3`
Expected: 编译通过、测试全 PASS。若 slint 宏编译报 `for x[i]` 语法错，查 `rust-remote-control-stack` skill 修正语法后重试。

可选冒烟（有 X11 环境时）：`cargo run -p client` 起两实例本机连本机，切三组控件观察被控日志 `被控已应用画质 上限=...`。

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/client/ui/app.slint src/client/src/ui_glue.rs
git commit -m "feat(client): Slint 主控端三组分段控件（分辨率/清晰度/帧率）"
```

---

### Task 6: WEB 管理端——store 三字段 + 三组控件

**Files:**
- Modify: `src/admin-web/src/store.ts:9,82-83,107,133,383,499-505`
- Modify: `src/admin-web/src/components/control/remote-session.tsx:41 附近选择器,235-253 控件区`

- [ ] **Step 1: store 类型与字段**

`src/admin-web/src/store.ts:9` 的类型导入改为：

```ts
import type { QualityMode } from "@/lib/types/QualityMode";
import type { ResolutionTier } from "@/lib/types/ResolutionTier";
import type { ClarityTier } from "@/lib/types/ClarityTier";
import type { FpsTier } from "@/lib/types/FpsTier";
```

`store.ts:82-83` 字段声明替换为：

```ts
  // 三轴显示参数（分辨率/清晰度/帧率）——主控选择，发 set_quality 给被控端
  remoteResolution: ResolutionTier;
  remoteClarity: ClarityTier;
  remoteFps: FpsTier;
```

`store.ts:107` action 类型替换为：

```ts
  setRemoteDisplayParams: (p: { resolution?: ResolutionTier; clarity?: ClarityTier; fps?: FpsTier }) => void;
```

`store.ts:133` 与 `store.ts:383` 两处默认值 `remoteQuality: "smooth",` 均替换为：

```ts
      remoteResolution: "r720p",
      remoteClarity: "standard",
      remoteFps: "smooth",
```

- [ ] **Step 2: action 实现**

`store.ts:499-505`（`setRemoteQuality`）替换为：

```ts
  // 切换三轴显示参数 → 合并当前值后发 set_quality 给被控端（mode 按清晰度映射兜底旧被控端）
  setRemoteDisplayParams(p) {
    const sessionId = get().remoteSessionId;
    const resolution = p.resolution ?? get().remoteResolution;
    const clarity = p.clarity ?? get().remoteClarity;
    const fps = p.fps ?? get().remoteFps;
    set({ remoteResolution: resolution, remoteClarity: clarity, remoteFps: fps });
    if (!sessionId) return;
    const mode: QualityMode = clarity === "high" ? "high_quality" : "smooth";
    get().sendEnvelope({ type: "set_quality", session_id: sessionId, mode, resolution, clarity, fps });
  },
```

注意：以实际 `setRemoteQuality` 原实现的 session 取值写法为准（如原用局部变量/守卫顺序不同则保持原风格）。

grep 清残留：

Run: `grep -rn "remoteQuality\|setRemoteQuality" src/admin-web/src/`
Expected: 仅剩 remote-session.tsx 引用（Step 3 一起换）。

- [ ] **Step 3: remote-session.tsx 三组控件**

组件顶部（`MetaItem` 等局部组件旁）加分段按钮局部组件：

```tsx
/** 分段按钮组：三轴显示参数共用（样式与原「流畅/高清」一致） */
function SegGroup<T extends string>({
  options,
  value,
  onChange,
}: {
  options: { v: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex items-center overflow-hidden rounded-md border border-border">
      {options.map((o) => (
        <button
          key={o.v}
          type="button"
          onClick={() => onChange(o.v)}
          className={`px-2.5 py-1 text-xs ${o.v === value ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-secondary"}`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
```

`remote-session.tsx:41` 附近的 store 选择器把 `remoteQuality`/`setRemoteQuality` 换为：

```tsx
  const remoteResolution = useStore((s) => s.remoteResolution);
  const remoteClarity = useStore((s) => s.remoteClarity);
  const remoteFps = useStore((s) => s.remoteFps);
  const setRemoteDisplayParams = useStore((s) => s.setRemoteDisplayParams);
```

（以文件内现有选择器写法为准，可能是解构式——保持原风格。）

`remote-session.tsx:235-253`（原「流畅/高清」块）替换为：

```tsx
          {/* 三轴显示参数：分辨率 / 清晰度 / 帧率（仅远程控制标签） */}
          {tab === "remote" && (
            <div className="flex items-center gap-1.5">
              <SegGroup
                options={[
                  { v: "r720p", label: "720" },
                  { v: "r900p", label: "900" },
                  { v: "r1080p", label: "1080" },
                  { v: "native", label: "原生" },
                ]}
                value={remoteResolution}
                onChange={(v) => setRemoteDisplayParams({ resolution: v })}
              />
              <SegGroup
                options={[
                  { v: "standard", label: "标准" },
                  { v: "high", label: "高清" },
                ]}
                value={remoteClarity}
                onChange={(v) => setRemoteDisplayParams({ clarity: v })}
              />
              <SegGroup
                options={[
                  { v: "smooth", label: "流畅" },
                  { v: "standard", label: "标准" },
                  { v: "saver", label: "省流" },
                ]}
                value={remoteFps}
                onChange={(v) => setRemoteDisplayParams({ fps: v })}
              />
            </div>
          )}
```

- [ ] **Step 4: 构建验证**

Run: `pnpm -C src/admin-web build 2>&1 | tail -5`
Expected: tsc + vite 构建通过，无类型错误。

- [ ] **Step 5: Commit**

```bash
git add src/admin-web/src/
git commit -m "feat(admin-web): WEB 主控端三组分段控件（分辨率/清晰度/帧率）"
```

---

### Task 7: 全量验证

**Files:** 无新改动（只验证）

- [ ] **Step 1: Rust 质量门**

Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings 2>&1 | tail -3`
Expected: 均通过。有 warning 修完再过。

- [ ] **Step 2: 全 workspace 测试**

Run: `cargo test --workspace 2>&1 | tail -10`
Expected: protocol / client / server 全 PASS。

- [ ] **Step 3: TS 生成物一致性**

Run: `git status --short src/admin-web/src/lib/types/`
Expected: 干净（无未提交生成物漂移）。

- [ ] **Step 4: admin-web 构建**

Run: `pnpm -C src/admin-web build 2>&1 | tail -3`
Expected: 通过。

- [ ] **Step 5: 若有修正则 Commit**

```bash
git add -A && git commit -m "chore: 三轴拆分全量验证收尾（clippy/fmt 修正）"
```

---

## 真机验收清单（代码完成后，用户执行）

计划内自动化测试不覆盖以下真机项（历史坑，spec §8.4）：

1. **切分辨率后点击坐标不偏**——4 档分辨率各切一遍，点被控端固定目标（P-CLI4 回归）。
2. **弱机切「原生+高清」不被 adaptive 立即拉回**——request_reset 生效；随后过载时 adaptive 应能在原生档继续降分辨率（本次新增能力）。
3. **新主控 ↔ 旧被控（0.5.0）互通**——切三轴退化为 mode 两档，不崩、不断流。
4. **WEB 端三控件切换**→ 顶栏「分辨率」回显跟随变化。
5. **原生档 4K 屏带宽观察**——帧体积与卡顿是否可接受（spec §9 已标注吃带宽）。
