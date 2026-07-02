# 鼠标滚轮转发 设计 + 计划（Spec+Plan 合一）

> 日期：2026-07-02 · 分支 spec-simd-jpeg（并入 0.4.9）· 类型：功能（跨协议+客户端+WEB）

## 目标
主控远控被控时可用鼠标滚轮上下/左右滚动。当前五层全缺（见调查）：主控 Slint 无 scroll 捕获、protocol 无 Scroll 变体、被控 enigo 无 scroll 注入、WEB 无 wheel 监听；server 对 Input 透明转发（无需改）。

## 关键设计决策
- **协议单位 = 滚轮"格"(notches, i32)**，非像素。`InputEvent::Scroll { dx: i32, dy: i32 }`。约定 **dy>0 = 向上滚（内容上移）**，dx>0 = 向右。各源把原生 delta 归一到格；被控映射到 enigo。
- **归一**：主控 Slint `delta-y`(逻辑 px, f32) 与 WEB `wheel.deltaY`(按 deltaMode 归 px) 均除以 `SCROLL_STEP_PX=40.0` 四舍五入；非零但舍入为 0 时取 ±1(保证小滚动也动)。
- **方向/步长真机验**：enigo/各平台滚动符号约定不一(enigo `length>0` 与"向上/下"的对应、WEB deltaY 正=下)。**代码用具名常量控制符号**，真机若反向则翻常量，不改结构。
- **enigo Axis**：`Axis::Vertical`(dy) / `Axis::Horizontal`(dx)。仅在对应轴非零时调 scroll。
- server 不改（`Message::Input` 透明转发，InputEvent 不感知）。

## 任务（逐层，依赖顺序：协议先）

### Task S1：protocol 加 Scroll 变体 + 重生成 ts-rs 绑定
- `src/protocol/src/lib.rs` 的 `InputEvent` enum 加 `Scroll { dx: i32, dy: i32 }`（放在 Text 后；`#[serde(rename_all=snake_case)]` 下自动 `"kind":"scroll"`）。
- 重新导出 ts 绑定：跑生成 ts-rs 的测试/命令（项目现有机制，产出 `src/protocol/bindings/InputEvent.ts` 与 `src/admin-web/src/lib/types/InputEvent.ts`），确认新增 `{ kind:"scroll", dx, dy }`。
- 验证 `cargo build -p protocol` + 全 workspace 编译。

### Task S2：被控 enigo 注入 Scroll
- `src/client/src/inject.rs`：`match ev` 加 `InputEvent::Scroll { dx, dy }` arm。用具名常量控制符号（如 `const SCROLL_SIGN: i32 = -1;` 便于真机翻向），非零轴才调：
  `if *dy != 0 { self.enigo.scroll(SCROLL_SIGN * dy, Axis::Vertical)?; }` 同理 Horizontal。
- import 加 `Axis`。
- 单测：inject 纯逻辑难测(需 enigo)，随 workspace 编译验证；真机验方向。

### Task S3：主控 Slint 捕获 + ui_glue 发送
- `src/client/ui/app.slint`：远程画面 `ta := TouchArea` 加 `scroll-event(ev) => { root.on_pointer_scroll(ev.delta-x / 1px, ev.delta-y / 1px); accept }`（delta 除 1px 转 float）。新增 callback `on_pointer_scroll(float, float)`（声明区，紧邻 on_pointer_button）。
- `src/client/src/ui_glue.rs`：`on_on_pointer_scroll` 绑定，把 (dx_px, dy_px) 按 `SCROLL_STEP_PX=40.0` 归一到格（非零保底 ±1），发 `FromUi::Input { session_id, event: Scroll { dx, dy } }`。仅在有会话时发。
- `src/client/src/net/mod.rs` / `dispatch.rs`：`FromUi::Input` 已透传 InputEvent，无需改（Scroll 走同路径）。

### Task S4：WEB 主控 wheel
- `src/admin-web/src/components/control/remote-session.tsx`：加 `wheel` 监听（`{ passive:false }` + `preventDefault` 防页面滚动），按 `deltaMode`(0=px,1=line×16,2=page×视口高) 归 px 再除 `SCROLL_STEP_PX=40` 到格，`sendInput({kind:"scroll", dx, dy})`；卸载时 remove。
- `src/admin-web/src/components/control/remote-geometry.ts`：加 `remoteScrollEvent(deltaX, deltaY, deltaMode): InputEvent` 归一构造器。
- 复用重生成的 `InputEvent.ts`（Task S1 产出）。

## 测试与验收
- workspace 全绿 + Windows 交叉编译。
- 真机：主控滚轮滚被控文件列表/网页，**方向正确**(不反)、速度合理；WEB 端同验。方向反则翻 `SCROLL_SIGN`/WEB 符号常量。

## 非目标
- 平滑/惯性滚动、水平滚动手势、触摸板高精度 delta——先做基础 notch 转发。
