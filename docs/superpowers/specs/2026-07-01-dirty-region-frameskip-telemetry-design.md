# Spec D2-①：被控端 frame-skip + 脏区遥测 设计

> **状态**：设计已确认 + 三轮 Codex 评审已落实，待最终审 → writing-plans
> **归属**：Spec D 梯队2 第①项（远程延迟/带宽根治，公网中转场景）
> **关联**：`remote-latency-analysis`（根因与部署现实订正）、`five-feature-roadmap`、梯队1（drop-stale 双 lane）

---

## §0.0 评审修订记录（Codex，2026-07-01）

三轮评审共 6 项 HIGH + 6 项 MEDIUM/LOW，均已落实到本 spec。

**第一、二轮（5 HIGH + 4 MEDIUM/LOW）：**

| 评审条目 | 落实位置 |
|---|---|
| HIGH 默认日志落盘不是必做 → 改硬性前置 | §1 纳入、§4.2 Sink、§10 |
| HIGH kill-switch 仅环境变量+重启 → 多通道快速切换 + 4 档模式 | §3.6、§5 点4、§7 |
| HIGH send_stall 只反映本机背压 / 覆盖不到 relay→主控 → 补 server frame-lane drop + 主控 seq_gap，端到端切两段 | §4.1、§4.6、§4.7 |
| HIGH「无需 netem 前置门」过强 → 改最小 netem 冒烟门（必过） | §5、§5.1 |
| MEDIUM 报告问题按钮未落地 → Slint 导出/复制诊断包 + admin-web 下载 JSON（限大小/脱敏） | §4.3、§4.7 |
| MEDIUM `wire 触顶`/`rtt` 未采集 → 只用实采 `sent_Bps`/`enc_Bps`/`effective_fps`/`seq_gap` | §4.2、§4.6 |
| LOW 开关耦合 → 拆 `OHMYDESK_FRAMESKIP` 与 `OHMYDESK_DIRTY_TELEMETRY` | §3.6 |

**第三轮（3 HIGH + 2 MEDIUM，均已核验代码属实）：**

| 评审条目 | 代码实证 | 落实位置 |
|---|---|---|
| HIGH `legacy-full-frame` 仍走新 capture_raw/哈希/遥测路径，非真旧路径，无法回退 | `workers.rs:324` 旧路径=`frame_q()` 一次完成 | §3.1、§3.6（模式重定义为「精确旧路径」+ 增 `full-frame-with-telemetry` 对照档） |
| HIGH `egress_drop`/`frame_lane_drop` 数 `send_replace` 覆盖会过报（槽内旧值≠未消费旧帧） | `conn.rs:60` `borrow_and_update`、`hub.rs:53` `send_replace` | §4.1（改为 Δenqueued−Δsent 计数）、§4.7、§8 测试10 |
| HIGH `send_stall_ms` 无数据回流：worker 线程看不到 conn.rs 异步泵的 flush 耗时 | worker `workers.rs:332` 只 `from_ui_tx.send`；WS 写在 `conn.rs:62` | §4.1（拆 `EgressSample`，conn.rs 经 telemetry 通道回流，collector 按 seq 合并）、§4.2 |
| MEDIUM 热切模式缺运行期传播机制 | worker `workers.rs:352` 只认 Start/Stop；`capture.rs:16` QUALITY 原子是现成范式 | §3.6（`RenderModeState` 原子组，worker 每 tick 读，复用 QUALITY 范式；定义 low-bandwidth 与 SetQuality 优先级） |
| MEDIUM admin-web 诊断包未定义 ring/刷新丢失/上限 | §4.7 原只写 console+按钮 | §4.7（按 session 内存 ring 5min、刷新即丢可接受、字段上限、按钮导出 ring） |

---

## §0 背景与前提订正

**部署现实**：首批用户是自家客服团队，远程支持公网客户，**全程经公网中转**（rc.guoziweb.com relay，非 P2P）。公网下网络 RTT 几十~上百 ms、**客户被控端上行带宽成为真实瓶颈**（家用/移动上行常 1-5Mbps）。

**前提订正（关键）**：当前「整帧 JPEG + drop-stale」实现**只过了编译 + 单测，从未经客服/公网检验**——它与任何新方案一样是**未验证的系统**，不能当成稳定基线。因此本轮目标不是「在稳定基线上做优化」，而是：

> 在上线前，给生产路径加一层**只会变好不会变坏的减法**（静止不发帧、不编码），同时**埋点采集真实数据**，让上线后「是否需要瓦片增量编码」这个高复杂度决策由证据而非猜测做出。

**本 spec 明确不做**有状态增量/瓦片编码——那是 Layer 3，单独并行立项、feature flag 默认关、经 `tc netem` 劣化台验证后才考虑灰度（见 §6）。

---

## §1 范围

### 纳入（本 spec）
- **Layer 1 — frame-skip**：被控端推帧线程在「截帧」与「编码」之间插入变化检测；屏幕未变化则**既不编码也不发送**，周期性强制全量帧兜底。
- **Layer 2 — 可观测性**（双目的）：复用同一次变化检测 + 流水线分段埋点，既产出**优化决策指标**（是否需要瓦片编码），又提供**故障取证能力**（客服出现异常时能从数据/日志证明根因、定位改进方向）。
  - 被控端：分段指标（采集/跳过/编码/**出网 egress**）、结构化事件日志、环形缓冲触发式落盘、异常阈值告警、诊断快照触发。
  - **默认滚动日志文件 + 诊断包目录**（§4.2/§4.3，无需环境变量即落盘）——上线前硬性前置。
  - **运行模式与多通道快速切换**（§3.6，5 档模式含「精确旧路径」回退 + 运行期原子热切 + 配置/启动参数/UI/环境变量）——现场可一步回退。
  - server：**纯日志** frame-lane drop 计数（§4.7，relay→主控 段证据；不改路由/转发）。
  - 主控端 Slint + admin-web：**纯日志**收帧/解码/渲染/丢帧/seq_gap/重连指标（无行为、无协议、无渲染改动）。

### 排除（非本 spec，单独立项）
- 协议 `Message::Frame` 变更（保持 `lib.rs:213-219` 原样）。
- server/主控端**行为/渲染/路由**改动（server 转发逻辑、Slint `set_frame` 整帧渲染、admin-web `<img>` 渲染均不动；§4.7 只加日志）。
- 瓦片/增量编码、keyframe 重同步、admin-web canvas 改造（Layer 3）。
- 遥测中心化上报（v1 仅本地日志；中心化是后续一个附加协议消息）。

---

## §2 架构与数据流

**核心改动（frame-skip + 被控端取证）完全局限在被控端**：主体在 `src/client/src/workers.rs` 的 `consume_capture` 推帧线程（现 `workers.rs:256-349`），辅以 `capture.rs`（拆「截/编码」）、`net/conn.rs`（出网段 egress 指标）、`main.rs`（默认滚动日志）、配置/模式小模块（§3.6）。**越出被控端的只有「纯日志」：server frame-lane drop 计数 + 主控端/admin-web 收帧埋点（§4.7）——全部无路由、无渲染、无协议改动，与 frame-skip 安全性同一等级。**

```
推帧线程每 tick（事件驱动，现有逻辑不变）:
  capture_raw()                    # xcap 截原始 RGBA（real_w×real_h）
    → 逐 64px 瓦片哈希(xxhash)      # 一次遍历，同时产出两样东西:
        ├─ 跳过决策(变化块数==0 且 未到 keyframe → continue，不编码不发)
        └─ dirty_ratio = 变化块/总块（遥测）
    → 若需发送: encode_frame_q()    # 现有纯函数，整帧 JPEG（不变）
    → FromUi::Frame{...}            # 现有出站，主控端原样消费（仍是自给自足全量帧）
    → 投 FrameSample 入 telemetry 通道  # 遥测来源A（conn.rs 另投 EgressSample=来源B；
                                      #   collector 按 seq 合并 → 10s 聚合日志 + 环形缓冲）
```

**核心不变量得以保持**：每个发出的帧仍是**自给自足的完整帧**，因此梯队1 的三层 drop-stale（`conn.rs:154`、server `hub.rs:215`、`ui_glue.rs:546`）和主控端「整帧替换」渲染**全部原样有效，零冲突**。frame-skip 只是「该发的时候发完整帧，不该发的时候什么都不发」。

---

## §3 Layer 1 — frame-skip 设计

### §3.1 capture.rs：拆分截屏与编码

新增方法，把「截」独立出来，编码沿用现有纯函数：

```rust
impl Capturer {
    /// 截一帧原始 RGBA（不缩放不编码），供变化检测先行。
    pub fn capture_raw(&self) -> anyhow::Result<image::RgbaImage> {
        Ok(self.mon.capture_image()?)
    }
}
```

现有 `frame_q`（`capture.rs:118-121`）保留不动；编码继续走现有纯函数 `encode_frame_q`（`capture.rs:130-150`）。

> **`frame_q()` = `legacy-full-frame` 模式的「精确旧路径」（Codex 三轮 HIGH①）**：`legacy-full-frame` 模式下推帧线程**直接调 `frame_q(&qp)`**（即 `workers.rs:324` 现状一字不改），**完全不经过** `capture_raw`、`tile_hashes`、跳过决策、遥测——这样若新截屏拆分/哈希/遥测/诊断写盘任一环出 bug，这个模式仍是**已编译验证过的原路径**，是可靠回退底座。其余模式（`frameskip`/`low-bandwidth`/`diagnostic`）走新 `capture_raw → tile_hashes → 决策 → encode_frame_q` 路径；另设 `full-frame-with-telemetry`（frameskip off + telemetry on）走新路径但不跳过，仅用于「旧整帧 vs frame-skip」带宽对照采样。模式与路径映射见 §3.6。

### §3.2 瓦片哈希（统一支撑 skip 决策 + 脏区比例）

新增独立可测模块 `src/client/src/framediff.rs`，纯函数、不依赖 X11：

```rust
/// 把 RGBA 帧按固定像素边长切网格，每块算一个 64bit 哈希。
/// 返回 (tile_cols, tile_rows, Vec<u64>)；行末/列末不足一整块按实际像素算。
pub fn tile_hashes(img: &image::RgbaImage, tile_px: u32) -> (u32, u32, Vec<u64>);

/// 与上帧瓦片哈希逐块比较，返回变化块数。维度不一致(分辨率变)时返回 total(全变)。
pub fn changed_tiles(prev: &[u64], cur: &[u64]) -> usize;
```

哈希用 **twox-hash（XxHash64）**：对 1920×1080×4≈8MB 全图一次遍历 < 1ms，相对整屏 JPEG 编码 20-80ms 可忽略。新增依赖写入 `src/client/Cargo.toml`。

> 设计取舍：在**原始截屏分辨率**上切瓦片（而非缩放后帧），因为这是已持有的缓冲、`dirty_ratio` 作为比例与分辨率无关、且跳过时无需做缩放。`tile_px` 默认 **64**，写入常量并标注可调。

### §3.3 跳过决策 + keyframe 兜底

推帧线程内新增线程私有状态：`last_tiles: Option<Vec<u64>>`、`last_sent_ms: u64`、`last_quality: u8`、`prev_sid: Option<String>`、`consecutive_skips: u32`。

每个 due tick 截帧后：

```text
cur_tiles = tile_hashes(img, 64)
changed   = match last_tiles { Some(p) => changed_tiles(p, cur_tiles), None => total }
keyframe_due = now - last_sent_ms >= KEYFRAME_INTERVAL_MS(=3000)
quality_changed = current_quality != last_quality

force = keyframe_due || quality_changed || last_tiles.is_none()
if changed == 0 && !force {
    consecutive_skips += 1
    last_tiles = Some(cur_tiles)      # changed==0 即 cur==last，赋值是无害 no-op
    continue                          # 不缩放、不编码、不发送
}
# 需要发送：
(data, w, h) = encode_frame_q(img, qp...)
send FromUi::Frame{ session_id, data, w, h, seq }
last_tiles = Some(cur_tiles); last_sent_ms = now; last_quality = current_quality
consecutive_skips = 0
```

**强制全量帧的三个触发**（缺一不可，均已论证必要）：
1. **keyframe 周期**（3s）：新主控接入/重连/丢帧后必有一帧兜底（主控端无「请求 keyframe」通道，靠周期保证）。
2. **画质切换**：`SetQuality` 改的是编码分辨率/质量；静止画面下若不强制发，切高清不生效。
3. **首帧 / 基准为空**：`last_tiles.is_none()`。

### §3.4 会话切换复位（边界）

推帧线程已读 `active`（`workers.rs:278`）。仿照剪贴板 worker 的 `prev_active` 模式（`workers.rs:402-406`）：检测到 `sid != prev_sid` 时 `last_tiles = None`（强制新会话首帧必发全量），`prev_sid = Some(sid)`。这覆盖「主控换人/断开重连」——新渲染器一接入就拿到完整帧，不会停在残帧。

### §3.5 静止空闲降采（信创弱 CPU 优化，边界明确）

即使跳过编码，每 tick 仍要 `capture_raw`（5-15ms）才能比对。为进一步降弱 CPU 占用：

```text
当 consecutive_skips >= IDLE_SKIPS_THRESHOLD(=15) 且 无近期输入(last_input_after 为假):
    本 tick 截帧间隔放宽到 IDLE_INTERVAL_MS(=200)
任何输入(LAST_INPUT_MS) 或 检测到变化 → 立即恢复正常档间隔
```

输入驱动路径（`workers.rs:273` `last_input_after`）不受影响，点击/打字仍即时唤醒抓帧。空闲降采只在「连续静止且无输入」时生效，对体验无损、对弱 CPU 显著省电。常量可调。

### §3.6 运行模式与快速切换（Codex 评审强化）

**两个独立开关，不耦合**（评审 LOW：避免「关跳过」连带「丢遥测」，否则无法做 on/off 对照）：

| 开关 | 作用 | 默认 |
|---|---|---|
| frameskip on/off | 只控「静止跳过」逻辑；off=逐帧编码发送（旧行为） | on |
| dirty_telemetry on/off | 只控脏区/遥测统计；与 frameskip 独立 | on |

**五档预设模式 + 路径映射**（评审 HIGH：现场要能一键切到**已知良好态**；三轮 HIGH① 要求 legacy 是「精确旧路径」而非「新路径关跳过」）：

| 模式 | frameskip | telemetry | 截屏/编码路径 | 用途 |
|---|---|---|---|---|
| `legacy-full-frame` | off | **off** | **`frame_q()` 精确旧路径**（不经 capture_raw/哈希/遥测） | 最可靠回退底座：新路径任一环出 bug 都能退回 |
| `frameskip`（默认） | on | on | 新路径 `capture_raw→tile_hashes→决策→encode_frame_q` | 生产默认，周期 keyframe |
| `full-frame-with-telemetry` | off | on | 新路径但不跳过（逐帧编码发送 + 脏区遥测） | 「旧整帧 vs frame-skip」带宽对照采样 |
| `low-bandwidth` | on | on | 新路径 + 强制 smooth 画质 + 更低 fps | 弱网兜底 |
| `diagnostic` | on | on | 新路径 + 强制日志/诊断包 + 较高 keyframe 频率 | 复现取证 |

> 关键区分：`legacy-full-frame`（HIGH① 的真回退）与 `full-frame-with-telemetry`（仅关跳过、仍走新路径）**是两档不同模式**——前者证「整个新代码路径是否引入问题」，后者做带宽对照。两者不可合并。

**运行期热切传播机制（评审 三轮 MEDIUM④，复用 `capture.rs:16` QUALITY 原子范式）**：

现状 `QUALITY: AtomicU8`（`capture.rs:16`）已证明「主控切画质 → 推帧线程每 tick 读 `current_params()` 即时生效，无需重启、无需新 CaptureCtrl 变体」这一范式可行。模式热切照搬：

```rust
// 新模式小模块内（被 capture.rs 与 workers.rs 共享读取）
static FRAMESKIP_ON:  AtomicBool = AtomicBool::new(true);
static TELEMETRY_ON:  AtomicBool = AtomicBool::new(true);
static RENDER_MODE:   AtomicU8   = AtomicU8::new(MODE_FRAMESKIP); // 5 档枚举编码
```

推帧线程**每 tick 读** `render_mode()`/`frameskip_on()`（与现 `current_params()` 同位置、同开销），分支选「精确旧路径 `frame_q()`」或「新路径」。UI 隐藏菜单/启动解析/配置加载都只是 `store` 这几个原子——**不新增 CaptureCtrl 变体、不动 worker 控制循环**（`workers.rs:352` Start/Stop 保持原样）。环境变量在启动时一次性 `store` 进原子。

**`low-bandwidth` 与 `SetQuality` 的优先级（评审 MEDIUM④，避免互相踩）**：`low-bandwidth` 不改 `QUALITY` 原子，而是在推帧线程取 `current_params()` 后**叠加一层 clamp**（强制 `max(interval_ms, LOW_BW_INTERVAL)` + 锁 smooth 上限），即「主控 SetQuality 仍可调，但 low-bandwidth 设地板」。退出 low-bandwidth 即恢复纯 `current_params()`。二者正交，不写同一状态。

**切换入口必须多通道**（评审 HIGH：客户双击 exe、远控画面已不可用时，靠环境变量+重启不现实）：
- **配置文件**：`%APPDATA%/OhMyDesk/config.toml`（Linux `~/.config/ohmydesk/`）的 `[render] mode=`，启动读取；优先级低于环境变量。
- **启动参数 / 安全模式快捷方式**：`ohmydesk-client.exe --render-mode=legacy-full-frame`；随安装包附一个 `安全模式(旧画面).bat`/快捷方式，客户被指导时双击即回退。
- **UI 隐藏诊断菜单**：被控端隐藏入口（如关于页连点）切模式 + 导出诊断包，**无需重启**即时生效——经上面的原子组传播，五档**全部可热切**（含 `legacy-full-frame` 一键退回精确旧路径）。
- **环境变量**（保留，最高优先级）：`OHMYDESK_FRAMESKIP=0`、`OHMYDESK_DIRTY_TELEMETRY=0`。

优先级：环境变量 > 启动参数 > 配置文件 > 默认。运行期 UI 热切覆盖当前进程值。**目标：远控已不可用时，电话指导客户一步切回旧画面，不依赖命令行。**

---

## §4 Layer 2 — 可观测性（优化决策指标 + 故障取证）

本层有**两个目的**，同等重要：（A）**优化决策**——回答「活动态带宽/编码是否核心痛点」，决定 Layer 3 瓦片编码是否转正；（B）**故障取证**——客服上线后出现「卡顿/黑屏/花屏/断连/不跟手」时，能从数据/日志**定位发生时刻与会话、区分根因属于哪一段（采集/跳过/编码/出网/网络/重连/主控渲染）、给出改进方向**。对一次性信任窗口，(B) 比 (A) 更要命。

### §4.1 单帧记录（两数据源 + collector 按 seq 合并）

> **三轮 HIGH③ 的结构性纠正**：采集/跳过/编码三段发生在**推帧 std::thread**（`workers.rs`），出网 flush 发生在**另一个 tokio 任务**（`conn.rs:40` 出站泵）——worker 物理上拿不到 flush 耗时与覆盖丢弃数。把单帧记录**拆成两个来源**，各自经 telemetry 通道汇入一个独立 collector，**按 seq 合并**，从根上消除「worker 填 send_stall」这个无法实现的假设。

**来源 A — 推帧线程产出 `FrameSample`**（采集/跳过/编码，无出网字段）：

```rust
struct FrameSample {
    ts_ms: u64,            // 墙钟毫秒（关联键）
    seq: u64,              // 发送帧 seq；跳过 tick 记 last_sent_seq 且 skipped=true（不参与 egress 合并）
    capture_ms: u32,       // capture_raw 耗时；采集失败另记事件
    skipped: bool,
    dirty_ratio: f32,      // changed / total；跳过时 0.0
    keyframe_forced: bool, // §3.3 三触发之一强制
    encode_ms: u32,        // 跳过时 0；发送时实测 encode_frame_q 耗时
    encoded_bytes: usize,  // 跳过时 0（= 编码产出字节，算 enc_Bps）
    w: u32, h: u32,
}
```

**来源 B — conn.rs 出站泵产出 `EgressSample`**（仅本机出网段，唯一知道 flush 的地方）：

```rust
struct EgressSample {
    seq: u64,            // 与被发送帧对应（frame watch 改携 (seq, json)）
    send_stall_ms: u32,  // write.send(..).await 实测耗时；本机 WS sink→OS 缓冲背压
    sent_ok: bool,
    ws_error: bool,
}
```

**`egress_drop` 的正确口径（评审 HIGH②，不再数 `send_replace`）**：watch 单槽里的旧值**=已被泵取走发过的值**，数「`send_replace` 返回 Some」会把正常覆盖误计为丢帧、严重过报。改为**两个单调计数器之差**：
- conn.rs 在「帧入 frame watch」处累加 `enqueued`；在「泵实际 `borrow_and_update` 并成功 `write.send`」处累加 `sent`。
- 窗内 `egress_drop = Δenqueued − Δsent`（=被新帧覆盖、从未上网的帧数），是「相邻两次泵唤醒间被替换的真实丢帧」，无过报。

**汇聚**：worker `FrameSample` 与 conn.rs `EgressSample` 各经 `telemetry_tx`（`mpsc`）送入 telemetry collector（`telemetry.rs` 内一个轻量任务）。collector 用一张小 map 按 seq 暂存近期 `FrameSample`，`EgressSample{seq}` 到达即把 `send_stall_ms` 贴回该帧记录（egress 样本可能晚于帧样本，collector 容短窗等待）；跳过帧无 egress 样本（从未发送），正确。collector 负责 §4.2 聚合、§4.3 环形缓冲+dump、§4.5 异常分类——全部**移出推帧线程**，线程只管「截/比/编/发 + 投 sample」。

> **`send_stall_ms` 的边界（评审 HIGH，务必写清）**：它只反映**本机 WS sink → OS 发送缓冲的背压**，**不等于**数据真正离开公网链路、**更覆盖不到** server→主控这一段（relay 转发、主控解码/渲染）。所以「上行带宽饱和」**不能只凭 send_stall 单点断言**，要结合 §4.2 聚合速率（`sent_Bps` 接近链路上限 + `effective_fps` 跌）+ §4.7 主控端 `seq_gap` 共同佐证。它是被控端能拿到的**最强本地信号**，但**间接**，不是 egress 完成的直接证据。`egress_drop`（Δenqueued−Δsent）同理是本机侧信号。

### §4.2 聚合日志（v1 sink = 本地结构化 tracing）

collector（§4.1）维护 10s 滑窗的合并后样本，满窗输出一条**可 grep/可解析**的 info 日志（固定字段名）：

```
遥测 sid=ab12 win=10s effective_fps=3.2 skip_pct=0.81 dirty_p50=0.04 dirty_p95=0.22 \
     sent_Bps=218K enc_Bps=205K bytes_avg=68KB bytes_p95=142KB cap_p95_ms=12 \
     enc_avg_ms=31 enc_p95_ms=72 stall_p95_ms=180 egress_drop=4 quality=smooth res=1920x1080
```

- 优化判据：`skip_pct`（静止占比）、`effective_fps`、`dirty_p50/p95`、`bytes_*`、`enc_*_ms`——回答「活动态带宽/编码是否核心痛点」（Layer 4 输入）。
- **带宽判据（Codex 评审 MEDIUM：只写实采字段，不写没采集的 `wire`）**：`sent_Bps`（**conn.rs 实写字节率**=窗内泵实际 `write.send` 字节/窗时长，来源 B）、`enc_Bps`（**worker 编码产出字节率**=Σ`encoded_bytes`/窗时长，来源 A）、`effective_fps`（实际送达帧率）。`sent_Bps` 在出网段实测、`enc_Bps` 在编码段实测，两者背离即「编出来但发不走」=出网瓶颈。「上行饱和」= `sent_Bps` 逼近已知链路上限 + `effective_fps` 跌 + `stall_p95_ms` 升，三者合证，不靠单一臆测的「wire 触顶」。
- 取证判据：`cap_p95_ms`（采集段）、`stall_p95_ms`/`egress_drop`（出网段本机信号，边界见 §4.1）。
- 仅在有活跃会话时输出，避免空转刷屏。
- **Sink（Codex 评审 HIGH，改为本 spec 必做项）**：Windows GUI 子系统下日志默认不可见（现仅设 `OHMYDESK_LOG_FILE` 才落盘），**客服现场出问题极可能没有可取证数据**。故本 spec **必须**实现：**默认滚动日志文件**——Windows `%APPDATA%/OhMyDesk/logs/`、Linux `~/.local/state/ohmydesk/logs/`，按天滚动、保留最近 N 天（默认 7），无需任何环境变量即生效；**诊断包目录** `…/OhMyDesk/diag/` 存 §4.3 触发式 dump。中心化上报留后续，但**本地默认落盘 + 诊断包是上线前的硬性前置**。

### §4.3 结构化事件日志 + 环形缓冲 + 触发式落盘

聚合指标会平滑掉瞬时事件，故另设**离散事件流**与**取证快照**机制：

- **事件**（带 ts+sid+endpoint_id，info/warn）：session_start/end、reconnect、quality_switch、wayland_fallback、capture_unavailable、capture_fail、encode_error、ws_send_error、disconnect、keyframe_forced、frameskip_disabled(kill-switch 命中)。
- **环形缓冲**（collector 持有）：内存中保留最近 ~5 分钟的合并样本（`FrameSample`+贴回的 egress 字段）+ 事件（定长 `VecDeque`，满则弹头，开销恒定、平时不落盘）。
- **触发式落盘**——把环形缓冲 dump 成 `…/OhMyDesk/diag/diag-<ts>-<sid>.jsonl`（JSON 行）：
  - **自动**：命中 §4.5 异常阈值时，WARN + dump（去抖：同类异常 N 秒内只 dump 一次，防刷盘）。
  - **手动（Codex 评审 MEDIUM：必须落到具体入口，不能停留概念）**：
    - **Slint 被控/控制端**：§3.6 隐藏诊断菜单内加「**导出诊断包**」+「**复制诊断包路径**」两个动作（外加可选热键 `Ctrl+Alt+D`）；点击即 dump 当前环形缓冲并把路径复制到剪贴板，客服直接粘进工单。
    - **admin-web**：远控页加「下载诊断 JSON」按钮，导出当前会话的主控侧指标（§4.7）。
  - **大小与脱敏（硬约束）**：单个诊断包封顶（如 ≤2MB，超出截断并标记）；**只含指标/事件/seq/时间，绝不含**屏幕像素、剪贴板文本、文件内容、密码/token——dump 前按字段白名单过滤。

### §4.4 关联键（让多端/多日志对得上）

每条指标/事件均带：`墙钟 ts(ISO8601 或 epoch_ms) + session_id + endpoint_id + seq`（帧相关时）。

- **seq 不再在主控端丢弃**：现 Slint 侧 `dispatch.rs:170` 用 `..` 丢 seq——改为在主控端**日志里保留 seq**（不改协议、不改渲染，只是日志多带一个已有字段）。
- 用途：端到端延迟（被控发 seq=N@T1 vs 主控渲 seq=N@T2）、丢帧定位（看 seq 跳变区分「中转/链路丢」与「主控 drop-stale 丢」）。

### §4.5 异常阈值 → 自动取证（把「感觉」变成「证据」）

定义可量化的异常，命中即 WARN + 触发 §4.3 落盘（阈值写常量、可调）：

| 异常 | 判据（示例阈值） |
|---|---|
| 出网阻塞 | `send_stall_ms > 1000` 或 单窗 `egress_drop` 占比 > 50% |
| 投递饥饿 | `effective_fps < 1` 且 `dirty_p95 > 0.1`（在产帧却发不出去） |
| 采集异常 | `capture_fail` 事件 或 `cap_ms > 200` |
| 编码过载 | `encode_p95_ms > 200` |
| 链路抖动 | 单窗 reconnect ≥ 2 |
| frame-skip 失效 | `skip_pct < 0.2` 且 `dirty_p95 < 0.05`（静止却几乎不跳，疑 bug） |

### §4.6 症状 → 根因段 → 改进方向（取证直接导向决策）

客服可能报的每类故障，都能由指标特征反查到段与下一步：

所有特征列只用**本 spec 实际采集**的字段（Codex 评审 MEDIUM：不引用 `wire`/`rtt` 等未采集量；RTT 为 fast-follow，本轮用 seq_gap 代偿）：

| 观测特征（均为实采字段） | 根因段 | 改进方向 |
|---|---|---|
| `send_stall` 高 + `sent_Bps` 逼近链路上限 + `effective_fps` 跌 + `dirty` 高 | 被控端上行带宽饱和（被控→relay） | 瓦片增量(Layer 3)/降分辨率/降质 |
| `encode_p95_ms`>100 + CPU 高 | 弱 CPU 软编 | 降分辨率/降帧/远期硬编 |
| `cap_p95_ms` 高 / `capture_fail` | 采集层(X11/DPI/虚拟化) | 采集兜底/会话类型提示 |
| 被控 `send_stall` 低 + **主控 `seq_gap` 大 / server `frame_lane_drop` 高** | **relay→主控 段**（中转/主控跟不上，非被控上行） | 主控解码优化/限速 / 中转排查（RTT 精测见 fast-follow） |
| `reconnect` 频繁 | 链路不稳/服务端 | 重连退避/服务端排查 |
| `skip_pct` 低但 `dirty_p95` 低 | **frame-skip 自身 bug** | 查哈希/keyframe 逻辑 |

> 第 4 行靠 §4.7 的「server frame-lane drop + 主控 seq_gap」把卡顿**明确切成「被控→relay」与「relay→主控」两段**（Codex 评审 HIGH/MEDIUM：否则只能定位被控局部）。末行让本轮引入的 frame-skip **可证伪**：要么数据自证清白，要么自曝 bug。

### §4.7 端到端取证：server + 主控端轻量埋点（纯日志，无行为/协议/渲染改动）

被控端只能看到「被控→relay」半段。要区分卡顿在「被控→relay」还是「relay→主控」，**必须** server 与主控端也吐数据（Codex 评审 HIGH/MEDIUM）。**全部只加日志**，不改路由/渲染/协议（与已做的 `route_to_peer` 丢弃告警同一性质、同一风险等级）：

- **server（纳入本 spec，纯日志）**：在帧 lane 记 **`frame_lane_drop` 计数**，按 session 周期 `debug!`——这是「relay→主控带宽/主控消费跟不上」的直接证据，被控端看不到。**计数口径同 §4.1 HIGH②**：不数 `hub.rs:53 send_frame_to` 的 `send_replace` 覆盖（会过报），而是 `send_frame_to` 入队累加 `enqueued` − 主控向 watch 泵实际写出累加 `sent`，`frame_lane_drop = Δenqueued − Δsent`。（`route_to_peer` 路由失败告警已在 chat bug 修复中落地，复用同一「server 不静默」原则。）
- **Slint 控制端（纯日志）**：`recv_fps`、`decode_ms`、`render_set_ms`、`drop_stale_drops`（`ui_glue.rs:546` 排空丢弃计数）、`waiting_first_frame_ms`、reconnect，且**保留 seq 算 `seq_gap`**（相邻渲染帧 seq 差>1 即中间帧在 relay→主控 段被丢/积压）。10s 聚合一条日志。
- **admin-web（最小化，纯日志，评审 三轮 MEDIUM⑤ 落地）**：不依赖 console，按 session 维护**浏览器内存 ring**（zustand store 内一个定长数组，保留最近 5 分钟收帧指标：`{ts, seq, recv_fps, seq_gap, first_frame_wait_ms, reconnect}`，满则弹头）。明确边界：① **刷新/关页即丢**（纯内存，可接受——诊断针对「当前这次远控异常」，客服会在断开前点导出）；② **字段上限**：只存上述标量指标，**绝不含**帧像素/base64/剪贴板/文件内容（脱敏白名单同 §4.3），单次导出封顶（如 ≤1MB，超出按时间截断旧样本）；③ **导出触发**：远控页「下载诊断 JSON」按钮 = 把当前 session 的 ring 序列化下载（文件名带 sid+ts），console 仅作开发期辅助。
- **段定位逻辑**：被控 `send_stall/egress_drop` 高 → 被控→relay 段；被控正常但 server `frame_lane_drop` 高 / 主控 `seq_gap` 大 → relay→主控 段。两段分明，不再「只能定位被控局部」。
- **RTT 精测**：需 ping/pong 计时钩子，标 fast-follow；本轮用 `seq_gap` 代偿 relay→主控 的延迟/丢帧信号。

---

## §5 安全性论证 + 最小 netem 冒烟门（上线前置）

**Codex 评审 HIGH（撤回过强结论）**：原写「无需 netem 前置门即可上客服」过强——现有 full-frame 与 frame-skip **都没经过公网/客服验证**，不能默认现状稳定，也不能只因为是减法就免劣化验证。**修正为：本层无需像瓦片那样做完整状态机验证，但必须过一道「最小 netem 冒烟门」（§5.1），通过后方可 Go。**

frame-skip 的低风险论据（说明为何只需「最小冒烟」而非「完整验证」，均已勘察确认）：

1. **静止不发帧安全**：主控端心跳独立于帧（Slint `conn.rs:91-112` 每 5s 走可靠 lane；admin-web `real.ts` onopen 注册），**无任何「N 秒无帧 → 黑屏/判断连」逻辑**，无帧时两端均保留末帧。停发帧不会被误判为卡死/断开。
2. **纯减法**：本层只「少发 + 少编码 + 记数」，在带宽受限链路上只会减轻负载。最坏情况（哈希误判该发不发）由 3s keyframe 周期兜底，画面最多滞后 3s 自愈。
3. **与梯队1 零冲突**：每帧仍自给自足全量，三层 drop-stale 与主控端整帧渲染原样有效。
4. **多通道快速回退**：§3.6 配置文件 / 启动参数 / 安全模式快捷方式 / UI 隐藏菜单 / 环境变量任一即可切回 `legacy-full-frame`，现场不依赖命令行。
5. **server/主控端改动仅日志**：§4.7 的 server frame-lane drop 与主控埋点不改路由/渲染/drop-stale/协议——故障模式与现状一致，安全性等同 frame-skip 本体。
6. **取证能力本身就是安全网**：Layer 2 让你在客服之前就看到「现有整帧在公网是不是幻灯片」，出事时能 §4.6 反查根因——独立于优化之争就值得做。

### §5.1 最小 netem 冒烟门（必过，Codex 评审 HIGH）

上线前用 `tc netem` 在一台机器上跑完以下矩阵，**frameskip on / off 各一遍**，作为 Go/No-Go 硬门（不是瓦片那种完整状态机验证，是冒烟）：

| 维度 | 档位 |
|---|---|
| 上行带宽 | 1 Mbps / 2 Mbps / 5 Mbps |
| RTT | 加一档高延迟（如 +150ms） |
| 抖动/中断 | 一次断续重连（链路掉 5s 再恢复） |

**通过判据**：① 各档下画面不黑屏/不卡死、重连后自愈；② frameskip on 相对 off 的 `sent_Bps`/`effective_fps` 改善可量化（静态场景带宽显著下降）；③ §4 遥测与日志在劣化网络下字段齐全、§4.6 段定位可用；④ 无 §4.5 异常被漏报。

**额外三项验收（评审 三轮 HIGH/MEDIUM 强制，否则上线闭环不成立）**：
- ⑤ **`legacy-full-frame` 真回退**：切到该模式时，断言推帧走 `frame_q()` 精确旧路径（不调 `capture_raw`/不算哈希/不投 sample），画面与改造前一致——证「新路径若有 bug 能干净退回」。
- ⑥ **运行期热切**：劣化网络下经 UI 隐藏菜单从 `frameskip`↔`legacy-full-frame` 热切，**不重启**即生效、画面不中断——证 §3.6 原子传播机制真的通。
- ⑦ **诊断包导出**：被控 Slint「导出诊断包」与 admin-web「下载诊断 JSON」在该网络条件下都能产出**含 egress 段（send_stall/egress_drop）与主控 seq_gap** 的有效文件，且脱敏白名单生效（无像素/明文）。

任一不过 → 不 Go，先修。该 netem 台同时复用为 Layer 3 的验证基座。

---

## §6 Layer 3 边界（非本 spec，并行立项）

为防范围蔓延，明确钉死：

- **触发**：仅 `OHMYDESK_TILE=1`，默认关，只给内测/自测机。
- **复用**：Layer 2 的 `framediff::tile_hashes`/`changed_tiles` 即脏区编码地基（一份代码，先量后编）。
- **必须先过**：`tc netem` 劣化台（高 RTT / 限上行 / 丢包多档）+ Codex 列的六项测试矩阵——弱网补丁丢失/积压、admin-web 刷新等 keyframe、Slint 与 canvas 合成一致性、画质切换 framebuffer 重置、断连重连状态恢复、客服真机性能差异。
- **转正路径**：作为「Frame 协议 v2」（含二进制帧、admin-web 上 canvas）灰度，由 Layer 4 的遥测数据触发，**不在上线前**。

---

## §7 回滚

| 层级 | 手段 | 粒度 |
|---|---|---|
| 运行期热切 | §3.6 UI 隐藏菜单切 `legacy-full-frame`（无需重启） | 即时 |
| 现场免命令行 | 安全模式快捷方式/批处理 或 配置文件 `mode=legacy-full-frame` 重启 | 秒级，电话可指导客户 |
| 环境变量 | `OHMYDESK_FRAMESKIP=0`（仅关跳过，遥测仍在）/ `OHMYDESK_DIRTY_TELEMETRY=0` | 秒级 |
| 源码 | revert 被控端 `workers.rs`/`capture.rs`/`framediff.rs`/`telemetry.rs`/`conn.rs`/`Cargo.toml` | 干净，纯附加改动 |

无数据迁移、无协议兼容性问题（协议未变，新旧被控端对主控端表现一致）。server `frame_lane_drop`、主控/admin-web 埋点均为纯日志，单独 revert 互不影响。

---

## §8 测试策略（TDD，纯函数优先）

`framediff.rs` 与决策逻辑均可脱离 X11 单测，沿用 `capture.rs`/`coalesce_inputs` 既有测试风格：

1. **哈希稳定性**：同一 RgbaImage 两次 `tile_hashes` 全等；单像素改动 → 对应那块哈希变、其余不变。
2. **changed_tiles**：全同→0；全异→total；改 1/4 区域→≈total/4；维度不一致→total（分辨率变=全量）。
3. **跳过决策**（抽成纯函数 `decide(changed, force_flags) -> bool`）：变化=0 且 无强制 → 跳过；变化>0 → 发送；keyframe_due/quality_changed/首帧 任一为真 → 即使变化=0 也发送。
4. **keyframe 周期**：构造「连续静止超 3s」时间序列，断言至少发 1 帧。
5. **会话切换复位**：`sid` 改变 → 下一帧 `last_tiles` 视为 None → 必发。
6. **空闲降采**：连续跳过达阈值且无输入 → 截帧间隔放宽；注入 `mark_input_now` → 立即恢复。
7. **遥测聚合**：构造混合 skip/send 的合并样本序列，断言 `skip_pct`/`effective_fps`/`enc_Bps`/`stall_p95`/分位数计算正确。
8. **异常阈值判定**（纯函数 `classify(window) -> Vec<Anomaly>`）：构造各类窗口断言命中对应异常（出网阻塞/投递饥饿/采集异常/编码过载/frame-skip 失效），正常窗口不误报。
9. **环形缓冲 + 去抖**：满缓冲弹头不增长；命中异常触发一次 dump，去抖窗内同类异常不重复 dump；dump 内容含触发前的历史帧。
10. **egress_drop = Δenqueued−Δsent（评审 HIGH②）**：构造「enqueue 5、pump 实发 3」序列断言 drop=2；关键反例——「enqueue 3、pump 1:1 全发 3」断言 drop=0（证明不再把正常覆盖过报，旧的「数 send_replace 返 Some」口径会误报）。
11. **collector 按 seq 合并（评审 HIGH③）**：投 `FrameSample{seq=N}` 后投 `EgressSample{seq=N, send_stall_ms=120}`，断言合并记录的 stall=120；乱序到达（egress 先于 frame）仍正确合并；跳过帧（skipped=true）无对应 egress 样本不报错。
12. **legacy-full-frame 走精确旧路径（评审 HIGH①）**：`render_mode=legacy-full-frame` 时，断言推帧分支选 `frame_q()`、**不调** `capture_raw`/不算 `tile_hashes`/不投 sample（用注入计数或 trait 桩验证零调用）。
13. **模式热切原子（评审 MEDIUM④）**：`store` `RENDER_MODE`/`FRAMESKIP_ON` 后，读取端即时反映新值（同 QUALITY 原子语义）；`low-bandwidth` clamp 在 `current_params()` 之上设地板而不写 `QUALITY`（SetQuality 仍可调、但不破地板）。

集成冒烟（`--ignored`，依赖真实 X11）：静态桌面跑 10s，断言发送帧数 ≈ keyframe 周期次数（≈3-4 帧）而非满帧率，且聚合日志字段齐全。

---

## §9 非目标 / YAGNI

- 不做任何「近似相等」容差跳过（只做**精确**瓦片哈希一致才跳）——避免漏掉细微更新导致冻屏。
- 不做自适应码率/分辨率（画质仍由现有 `SetQuality` 手动档控制）。
- 不做遥测中心化上报、不做远程配置下发。
- 不碰协议；**server/主控端仅加日志**（§4.7 frame-lane drop + seq_gap，无路由/渲染/协议改动），不做主控端瓦片补丁渲染（那是 Layer 3）。

---

## §10 文件清单

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/client/src/framediff.rs` | 新建 | 瓦片哈希 + 变化计数 + 跳过决策纯函数（含单测） |
| `src/client/src/telemetry.rs` | 新建 | `FrameSample`/`EgressSample`/事件 + **collector 任务（按 seq 合并双源）**/环形缓冲/分位聚合/异常分类(`classify`)/触发式 dump（纯逻辑，含单测） |
| `src/client/src/capture.rs` | 改 | 新增 `capture_raw()` 拆「截」与「编码」；模式原子组读取辅助（复用 QUALITY 范式） |
| `src/client/src/workers.rs` | 改 | `consume_capture` 推帧线程：按 `render_mode` 分支（`legacy-full-frame`→`frame_q()` 精确旧路径；其余→新路径跳过决策/keyframe/复位/空闲降采）+ 投 `FrameSample` 入 telemetry 通道 |
| `src/client/src/net/conn.rs` | 改 | frame watch 改携 `(seq, json)`；出网段埋点产 `EgressSample`（`send_stall_ms` 实测 + `egress_drop = Δenqueued−Δsent`，**非数 send_replace**）经 `telemetry_tx` 回流；disconnect/reconnect 事件 |
| `src/client/src/net/dispatch.rs` | 改 | 主控端保留 seq（去掉 `..` 丢弃），透传给主控遥测 |
| `src/client/src/ui_glue.rs` | 改 | 主控端纯日志埋点：recv_fps/decode_ms/drop_stale_drops/首帧等待/seq_gap；§3.6 UI 隐藏诊断菜单（模式热切 + 导出/复制诊断包） |
| `src/client/src/main.rs`（或日志初始化处） | 改 | **默认滚动日志文件**（`%APPDATA%/OhMyDesk/logs/`，按天滚动保留 N 天，不依赖 `OHMYDESK_LOG_FILE`）；`mod framediff; mod telemetry;` |
| 客户端配置/模式（新增小模块） | 新建 | `RenderModeState` 原子组（`FRAMESKIP_ON`/`TELEMETRY_ON`/`RENDER_MODE`，复用 QUALITY 范式）+ `config.toml` 读取 + 启动参数 `--render-mode` + **5 档**模式解析与优先级（§3.6）+ `low-bandwidth` clamp |
| `src/server/src/hub.rs` | 改 | **纯日志**：帧 lane `frame_lane_drop = Δenqueued−Δsent`（**非数 send_replace**，§4.1 HIGH②）+ 周期 `debug!`（§4.7）。（`route_to_peer` 丢弃告警已随 chat bug 修复落地） |
| `src/admin-web/src/store.ts`、`.../transport/real.ts`、远控页组件 | 改（最小） | 纯日志：按 session **内存 ring（5min，刷新即丢）** 存 recv_fps/seq_gap/首帧等待/reconnect + 「下载诊断 JSON」按钮导出 ring（限大小/脱敏，§4.7） |
| `src/client/Cargo.toml` | 改 | 加 `twox-hash`（+ 滚动日志/配置所需 crate，如 `tracing-appender`/`toml`） |
