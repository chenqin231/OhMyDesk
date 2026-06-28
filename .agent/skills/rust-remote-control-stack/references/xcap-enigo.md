# 远控采集/注入速查（xcap + enigo）

> 适用：信创内网远程控制，被控端用 xcap 截屏、主控端用 enigo 注入键鼠。环境锁定 **Linux X11**（Wayland 两库均不可靠）。

## 1. xcap 速查（屏幕捕获）

**最新版本**：`0.9.6`（2026-05-24）。

**Cargo 依赖**
```toml
[dependencies]
xcap = "0.9"
image = "0.25"   # capture_image() 返回 image::RgbaImage，保存/转字节需要
```

**Linux 编译期系统依赖**（信创/麒麟需先装）
```
libxcb1-dev libxrandr-dev libdbus-1-dev pkg-config libclang-dev
# Wayland/录屏才需要：libpipewire-0.3-dev libwayland-dev
```

**最小示例：枚举显示器 → 截图 → 拿 RGBA 字节**
```rust
use xcap::Monitor;

fn main() {
    let monitors = Monitor::all().unwrap();       // Vec<Monitor>
    let mon = &monitors[0];                        // 主屏可用 mon.is_primary().unwrap()

    let image = mon.capture_image().unwrap();      // image::RgbaImage (= ImageBuffer<Rgba<u8>, Vec<u8>>)

    let w = image.width();                         // u32
    let h = image.height();                        // u32
    let raw: &[u8] = image.as_raw();               // 紧密排列的 RGBA8 字节，长度 = w*h*4

    image.save("shot.png").unwrap();               // 需 image crate
}
```

**核心 API 签名**
- `Monitor::all() -> XCapResult<Vec<Monitor>>`
- `monitor.capture_image() -> XCapResult<RgbaImage>`
- `monitor.capture_region(x, y, width, height) -> XCapResult<RgbaImage>`（局部截图，省带宽）
- 元数据：`x()`/`y()`/`width()`/`height()`/`is_primary()` 返回 `XCapResult<...>`；`friendly_name() -> XCapResult<String>`
- 也有 `Window::all()` + `window.capture_image()` 抓单窗口

**返回类型**：`RgbaImage`（即 `image::ImageBuffer<Rgba<u8>, Vec<u8>>`）。每像素 4 字节 RGBA，需要 RGB/JPEG 时自行转码或 drop alpha。

**性能注意**
- `capture_image()` 是全屏拷贝，单帧开销不小；高帧率远控用 `capture_region()` 只抓变化区域，或降分辨率/降帧（建议先做帧差再传）。
- `Monitor::all()` 每帧都调会重复枚举，启动时枚举一次缓存 `Monitor` 复用。
- RGBA→PNG 编码慢，实时流改用 JPEG/turbojpeg 或裸 RGBA + 自定义压缩。
- 0.9.x 起官方 README 宣称支持「Linux(X11, Wayland)、macOS、Windows、HarmonyOS」，但 Wayland 仍标注「特定场景不完全支持」。

**X11/Wayland 注意**：X11 全功能稳定；Wayland 截屏在部分桌面/合成器下受限（需 portal/pipewire 授权），信创内网建议强制 X11 会话。

出处：https://docs.rs/xcap/latest/xcap/ ，https://github.com/nashaofu/xcap

---

## 2. enigo 速查（键鼠注入）

**最新版本**：`0.6.1`（2025-08-28）。注意 **MSRV = Rust 1.87+**。

**Cargo 依赖**
```toml
[dependencies]
enigo = "0.6"
```

**Linux 运行/编译依赖**（X11 注入底层走 xdo）
```
Debian/麒麟: libxdo-dev      Fedora/UOS: libX11-devel libxdo-devel      Arch: xdotool
```

**最小示例（当前 0.6.x 写法）**
```rust
use enigo::{
    Enigo, Settings,
    Mouse, Keyboard,                       // trait，必须 use 才能调方法
    Coordinate::{Abs, Rel},
    Direction::{Press, Release, Click},
    Button, Key,
};

fn main() {
    let mut enigo = Enigo::new(&Settings::default()).unwrap();

    enigo.move_mouse(500, 200, Abs).unwrap();        // 绝对坐标，单位像素
    enigo.move_mouse(10, 0, Rel).unwrap();           // 相对移动
    enigo.button(Button::Left, Click).unwrap();      // 左键单击
    enigo.button(Button::Left, Press).unwrap();      // 按下不放（拖拽用）
    enigo.button(Button::Left, Release).unwrap();

    enigo.key(Key::Return, Click).unwrap();          // 回车
    enigo.key(Key::Control, Press).unwrap();         // 组合键：Ctrl 按下…
    enigo.key(Key::Unicode('c'), Click).unwrap();    // …c…
    enigo.key(Key::Control, Release).unwrap();       // …Ctrl 抬起

    enigo.text("Hello 远程控制 ❤️").unwrap();         // 直接输入文本（含 Unicode）
}
```

**当前 API 签名（trait 方法，均返回 `Result<(), InputError>`）**
- `Enigo::new(settings: &Settings) -> Result<Enigo, NewConError>`
- `Mouse::move_mouse(&mut self, x: i32, y: i32, coordinate: Coordinate)`
- `Mouse::button(&mut self, button: Button, direction: Direction)`
- `Mouse::scroll(&mut self, length: i32, axis: Axis)`
- `Keyboard::key(&mut self, key: Key, direction: Direction)`
- `Keyboard::text(&mut self, text: &str)`
- `Keyboard::raw(&mut self, keycode: u16, direction: Direction)`（平台原始扫描码）
- 枚举：`Coordinate::{Abs, Rel}`、`Direction::{Press, Release, Click}`、`Button::{Left, Middle, Right, Back, Forward, ScrollUp/Down/Left/Right}`、`Key`（含 `Key::Unicode(char)` 任意字符、`Key::Other(u32)` 原始键码）

**新旧 API 差异（近年大改，旧教程基本作废）**
- 旧（≤0.1.x）：`Enigo::new()` 无参；`enigo.mouse_move_to(x,y)`、`enigo.mouse_down(MouseButton::Left)`、`enigo.key_click(Key::Layout('a'))`、`enigo.key_sequence("...")`，方法不返回 Result。
- 新（0.2+~0.6）：`Enigo::new(&Settings)` 返回 `Result`；统一为 `move_mouse(x,y,Coordinate)` / `button(Button,Direction)` / `key(Key,Direction)` / `text(&str)`，全部返回 `Result`；`Key::Layout(c)` → `Key::Unicode(c)`；方法挂在 `Mouse`/`Keyboard` trait 上，**必须 `use enigo::{Mouse, Keyboard}`** 否则方法不可见。

**Settings 关键字段（X11 相关）**
- `x11_display: Option<String>`：显式指定 X11 display（如服务/无登录会话下手动设 `Some(":0")`）
- `linux_delay: u32`：X11 事件间延迟（默认 0；某些环境丢事件时调大）
- `release_keys_when_dropped: bool`（默认 `true`）：Enigo drop 时自动抬起按住的键，防卡键
- `wayland_display`、`event_source_user_data`、macOS/Windows 专属字段等

**X11 会话注意事项**
- 必须有可用的 X11 `DISPLAY`。无头/服务进程要正确传 `DISPLAY` 和 `XAUTHORITY` 环境变量，或在 `Settings.x11_display` 指定。
- 测试需串行：`cargo test -- --test-threads=1`（注入是全局状态）。
- Linux 需安装 `libxdo`，否则编译/链接失败。

**坑**
- 不 `use` trait 方法（`Mouse`/`Keyboard`）会报「方法不存在」。
- Wayland 标注 **experimental，有已知 bug**（feature flag 后），不要用于生产；本项目锁 X11。
- 输入瞬间生效，按住型操作（拖拽、组合键）要自己配对 `Press`/`Release`，异常退出靠 `release_keys_when_dropped` 兜底。

出处：https://docs.rs/enigo/latest/enigo/ ，https://github.com/enigo-rs/enigo

---

## 3. 两者结合做远控的注意点

- **统一锁 X11**：被控端 xcap 截屏、主控端（或被控端执行注入时）enigo 注入，在 Wayland 下都不稳；信创内网强制 X11 会话（`echo $XDG_SESSION_TYPE` 应为 `x11`），登录管理器关闭 Wayland。
- **坐标映射**：enigo `move_mouse` 用的是像素坐标，需与 xcap 截图的尺寸/原点对齐。多屏时 `Monitor::x()/y()` 是该屏在虚拟桌面中的偏移——主控点击的屏内坐标要加上目标 `Monitor` 的 `(x, y)` 再用 `Coordinate::Abs` 注入，否则多屏点偏。先按单屏跑通再处理多屏偏移。
- **截屏帧率**：`capture_image()` 全屏单帧不便宜，目标 10–15fps 远控用 `capture_region()` + 帧差，只传变化块；RGBA 实时流走 JPEG/turbojpeg 而非 PNG。`Monitor` 启动枚举一次复用，别每帧 `Monitor::all()`。
- **服务化运行**：作为守护进程跑时，xcap 和 enigo 都依赖 X11 会话，确保进程能拿到 `DISPLAY`/`XAUTHORITY`；enigo 可用 `Settings.x11_display` 兜底。
- **防卡键**：注入侧保留 `release_keys_when_dropped = true`（默认），主控断连/崩溃时被控端不残留按住的键鼠。
- **系统依赖一次装齐**：`libxcb1-dev libxrandr-dev libdbus-1-dev libxdo-dev pkg-config libclang-dev`，信创发行版（麒麟/UOS）按对应包名安装。
