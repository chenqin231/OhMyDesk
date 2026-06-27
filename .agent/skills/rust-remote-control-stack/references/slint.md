# Slint 开发速查（信创内网远控桌面客户端）

**最新稳定版：1.17.0**（slint-ui，Rust 优先的声明式 GUI 框架）。本速查面向"被控端把对端 RGBA 屏幕帧贴满窗口、运行在无 GPU 的信创国产 CPU"场景，全程走软件渲染。

---

## 1. 版本 + 项目初始化

`Cargo.toml`——信创无 GPU 必须关闭默认特性、只留软件渲染：

```toml
[dependencies]
# 默认特性含 femtovg(OpenGL)/skia(GPU) + winit + software，无 GPU 环境务必精简：
slint = { version = "1.17", default-features = false, features = [
    "compat-1-2",        # 必带：保证 1.x 兼容
    "std",
    "backend-winit",     # 桌面窗口（X11/Wayland/Win）
    "renderer-software", # 纯 CPU 软件渲染，无 OpenGL/Vulkan 依赖
] }

[build-dependencies]
slint-build = "1.17"
```

`build.rs`——编译 `.slint` 为 Rust 代码：

```rust
fn main() {
    slint_build::compile("ui/app.slint").unwrap();
}
```

也可用 `slint_build::compile_with_config(...)` 指定 style（如 `fluent`/`material`/`native`）。两种取代写法：`build.rs` + `slint::include_modules!()`（推荐，编译期），或内联 `slint::slint!{ ... }` 宏（小 demo）。

---

## 2. `.slint` 标记语言核心语法

```slint
// 顶层窗口组件
export component AppWindow inherits Window {
    title: "远程桌面";
    preferred-width: 1280px;
    preferred-height: 720px;
    background: black;

    // ---- 属性 property ----
    in property <string> peer-name;        // in：外部(Rust)可写，组件内只读
    out property <bool> connected;          // out：组件内可写，外部只读
    in-out property <image> frame;          // in-out：双向，远控贴帧用这个
    private property <int> fps: 0;          // private(默认)：仅组件内

    // ---- 回调 callback ----
    callback disconnect();                  // 无参
    callback key-pressed(string);           // 带参（类型列表）
    callback compute(int) -> int;           // 带返回值
    pure callback hash(string) -> int;      // pure：无副作用，可在绑定中调用

    // ---- 函数（逻辑全在 slint 内）----
    pure function double(x: int) -> int { return x * 2; }

    // ---- 布局 + 元素 ----
    VerticalLayout {
        spacing: 4px;
        padding: 8px;
        alignment: stretch;

        Image {
            source: root.frame;        // 远控帧贴这里
            image-fit: contain;        // contain/cover/fill/preserve；贴满窗口用 fill 或 cover
            width: 100%;
        }
        HorizontalLayout {
            Text { text: "FPS: " + root.fps; color: white; }
            Rectangle { background: connected ? green : red; width: 12px; height: 12px; }
            Button {
                text: "断开";
                clicked => { root.disconnect(); }   // => 设置处理逻辑
            }
        }
    }

    // 捕获键鼠（远控需要回传输入）
    TouchArea {
        clicked => { /* ... */ }
    }
}
```

要点：
- **绑定是响应式的**：`text: "FPS: " + fps;` 中 `fps` 变化自动刷新 UI。
- **双向绑定** `<=>`：`callback clicked <=> area.clicked;` 或属性别名。
- **布局**：`VerticalLayout`/`HorizontalLayout`/`GridLayout`，公共属性 `spacing`、`padding`、`alignment`。
- **常用元素**：`Window`、`Rectangle`、`Text`、`Image`、`TouchArea`；标准控件需 `import { Button, ... } from "std-widgets.slint";`。
- **常用类型**：`int`/`float`/`string`/`bool`/`color`/`image`/`length`(`px`)。

---

## 3. Rust 绑定模式

```rust
slint::include_modules!();  // 引入 build.rs 生成的代码（产生 AppWindow 类型）

use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;

    // 读/写属性：get_<name> / set_<name>，名字里的 '-' 转成 '_'
    ui.set_peer_name("192.168.1.20".into());
    let connected = ui.get_connected();

    // 注册回调：on_<name>
    let ui_weak = ui.as_weak();              // 弱引用，避免闭包里循环引用
    ui.on_disconnect(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_connected(false);
        }
    });
    ui.on_key_pressed(|key| { /* 把按键回传对端 */ });

    ui.run()                                  // show + 进入事件循环（阻塞）
}
```

ComponentHandle 关键方法：`new()` / `run()`（show+事件循环）/ `show()` / `hide()` / `window()` / `as_weak()` / `clone_strong()`（未实现 `Clone`，跨持有用这个）。

**跨线程更新 UI（远控网络线程→UI 线程，必须）**——UI 只能在事件循环线程操作，网络线程通过 `invoke_from_event_loop` + `Weak` 投递：

```rust
let ui_weak = ui.as_weak();   // Weak 可跨线程，强引用 AppWindow 不可 Send

std::thread::spawn(move || {
    loop {
        let rgba_frame: Vec<u8> = recv_frame_from_peer();  // 收对端帧
        let w = ui_weak.clone();
        // 把闭包投递到 UI 事件循环执行
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                let img = build_image(&rgba_frame, FRAME_W, FRAME_H);
                ui.set_frame(img);          // 触发响应式重绘
            }
        }).unwrap();
    }
});
```

也可用 `Weak::upgrade_in_event_loop(move |ui| { ... })` 一步完成"投递+upgrade"。

---

## 4. 远控关键：显示动态帧（RGBA → Image）

把对端屏幕的原始 RGBA 字节填进 `SharedPixelBuffer`，转成 `Image`，set 到 `in-out property<image>`：

```rust
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// 把一帧 RGBA8 原始字节（len = w*h*4）转为 Slint Image
fn build_image(rgba: &[u8], w: u32, h: u32) -> Image {
    // SharedPixelBuffer 引用计数、clone 廉价
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    // make_mut_bytes() 返回 &mut [u8]（共享时会 copy-on-write）
    buffer.make_mut_bytes().copy_from_slice(rgba);
    Image::from_rgba8(buffer)   // 非预乘 alpha
}
```

要点：
- 帧带预乘 alpha 用 `Image::from_rgba8_premultiplied`；无 alpha 通道用 `Image::from_rgb8(SharedPixelBuffer<Rgb8Pixel>)`（远控不透明帧用 RGB8 更省 1/4 内存+带宽）。
- **贴满窗口**：`.slint` 里 Image 设 `image-fit: fill;`（拉伸铺满）或 `cover`（保持比例裁切铺满），`width/height` 给 `100%`。
- **定时刷新**：每收到一帧就在事件循环线程 `set_frame(build_image(...))`；Slint 响应式系统自动只重绘脏区。可配合 `slint::Timer` 控制刷新节奏，或纯事件驱动（来帧即 set）。
- **性能**：`copy_from_slice` 整帧拷贝一次即可；若帧尺寸固定，可复用缓冲思路减少分配。Image 元素本身缓存纹理，软件渲染下脏区重绘可显著降 CPU。

---

## 5. 软件渲染器配置（无 GPU / 信创国产 CPU 关键）

桌面应用最简路径——**不需要实现 `Platform` trait**，开了 `renderer-software` 特性后 Slint 会自动选用；想强制指定用环境变量：

```bash
# winit 窗口 + 纯软件渲染（无 OpenGL/Vulkan），信创无独显机器用这个
export SLINT_BACKEND=winit-software
# 或仅 software（让 Slint 自选窗口后端）
export SLINT_BACKEND=software
```

落地建议（双保险）：
1. 编译期：`Cargo.toml` 关默认特性、只留 `backend-winit` + `renderer-software`（见第 1 节），从根上排除 femtovg/skia 的 GPU 依赖，避免链接 libGL。
2. 运行期：进程内设 `SLINT_BACKEND=winit-software` 兜底，防止环境里残留的 GPU 后端被选中。

进阶（裸机/无窗口系统/直接写 framebuffer，如 LinuxKMS 或自定义显示）才需要 `slint::platform::software_renderer::SoftwareRenderer` + 实现 `Platform` trait：
- 整帧渲染 `render(buffer, stride)`，或逐行 `render_by_line(LineBufferProvider)`（内存受限设备/直推显示硬件）。
- `RepaintBufferType` 控制脏区复用策略（`ReusedBuffer`/`NewBuffer`/`SwappedBuffers`）加速重绘。
- 一般信创桌面（有 X11/Wayland）用上面的 `winit-software` 即可，**无需**碰这层。

---

## 6. 常见坑

1. **默认特性带 GPU**：`slint` 默认启用 `renderer-femtovg`(OpenGL ES 2.0) 和 `renderer-skia`，信创无独显机器若不 `default-features = false`，会链接/加载 GL 库导致启动黑屏或崩溃。必须显式精简特性。
2. **跨线程直接操作 UI 会 panic**：`AppWindow` 强句柄不是 `Send`，网络/解码线程绝不能直接 `set_xxx`。一律走 `invoke_from_event_loop` 或 `Weak::upgrade_in_event_loop`。`Weak` 是可跨线程的。
3. **属性名连字符 → 下划线**：`.slint` 里 `peer-name`，Rust 侧是 `set_peer_name`/`get_peer_name`。容易对不上。
4. **`compat-1-2` 必带**：关默认特性后若漏掉它，跨 1.x 版本会有兼容性/编译问题。
5. **RGBA 字节序与对齐**：`SharedPixelBuffer::<Rgba8Pixel>` 期望 `R,G,B,A` 顺序、`len == w*h*4`；对端若是 BGRA（Windows 抓屏常见）需先转换，否则颜色错乱。
6. **预乘 vs 非预乘 alpha**：选错 `from_rgba8` / `from_rgba8_premultiplied` 会导致半透明区域发暗/发亮。远控不透明帧建议直接 RGB8 规避。
7. **旧版 API 差异**：网上大量教程基于 0.x，组件声明从旧的 `Foo := Rectangle { }` 语法已改为 `component Foo inherits Rectangle { }`（新语法），`export` 才能在 Rust 侧可见；老的 `:=` 现仅用于元素实例命名（如 `area := TouchArea {}`）。文档务必认准 `latest`/1.17，别用 `releases.slint.dev/1.x` 老快照。
8. **`run()` 阻塞**：`ui.run()` 进入事件循环且阻塞主线程，初始化逻辑要在 `run()` 之前完成，运行期变更只能从回调或投递闭包里做。

---

## 出处（关键文档 URL）

- 官网与文档入口：https://slint.dev/ ｜ https://docs.slint.dev/latest/docs/slint/
- Rust API：https://docs.rs/slint/latest/slint/
- GitHub（版本/示例/README）：https://github.com/slint-ui/slint
- Cargo 特性（含 renderer-software）：https://docs.rs/slint/latest/slint/docs/cargo_features/index.html
- SharedPixelBuffer API：https://docs.rs/slint/latest/slint/struct.SharedPixelBuffer.html
- 软件渲染器模块：https://docs.rs/slint/latest/slint/platform/software_renderer/index.html

---

**核心三件套**：①`Cargo.toml` 关默认特性只留 `backend-winit + renderer-software`（无 GPU 信创适配根本）；②远控贴帧走 `SharedPixelBuffer::<Rgba8Pixel> → Image::from_rgba8 → set_frame`，`.slint` 里 `image-fit: fill/cover` 铺满窗口；③网络线程更新 UI 必须经 `invoke_from_event_loop` + `Weak`，禁止直接操作句柄。版本以 1.17.0 为准，API 认准 `latest` 文档避免旧 `:=` 语法断裂。
