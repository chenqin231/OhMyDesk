---
name: rust-remote-control-stack
description: 开发 OhMyDesk「信创内网终端远程安全管控平台」的 Rust 客户端/服务端代码时使用。提供 Slint(GUI)、xcap(截屏)、enigo(键鼠注入)、sysinfo(硬件+GPU)、quinn(QUIC) 的最新 API 速查与避坑指南，专治这些库在 Claude 语料里版本过时、API 已大改导致的写错。触发场景：写远控客户端 UI/采集/注入、服务端 WS 中转、硬件资产上报、未来网络加速。
---

# Rust 远控技术栈速查（OhMyDesk 信创内网远控平台）

> **为什么有这个 skill**：本项目客户端+服务端统一 Rust。其中 Slint 的 `.slint` DSL、enigo 0.6 新 API、sysinfo 2026 新增的 GPU API 在 Claude 训练语料里要么稀少、要么是已废弃的旧写法，凭记忆直接写极易出错。**动手写这些库相关代码前，先读对应 reference 文件**，按里面验证过的最新 API（截至 2026-06）来写。

## 技术栈定位

- 项目规范：`.agent/user.md`
- 完整设计：`docs/superpowers/specs/2026-06-27-xinchuang-remote-control-design.md`
- 统一栈：客户端 Agent(Slint+Rust) + 服务端 Relay(axum+Rust)；管理端 React、MCP TS 薄层是浏览器/SDK 例外。

## Reference 索引（按需深读）

| 文件 | 覆盖库 | 何时读 |
|------|--------|--------|
| `references/slint.md` | Slint 1.17 GUI | 写客户端窗口、远控贴帧、软渲染配置时 |
| `references/xcap-enigo.md` | xcap 0.9 截屏 + enigo 0.6 键鼠注入 | 写被控端截屏、主控端注入、坐标映射时 |
| `references/sysinfo-quinn.md` | sysinfo 0.39 硬件 + quinn 0.11 QUIC | 写硬件资产上报、未来 QUIC 加速时 |

## 高频踩坑铁律（必记，详见各 reference）

**Slint**
- `Cargo.toml` 必须 `default-features = false` 只留 `compat-1-2 + std + backend-winit + renderer-software`——否则默认带 GPU(femtovg/skia)，信创无独显机启动黑屏。
- 网络/解码线程**禁止**直接 `set_xxx` 操作 UI（句柄非 Send），一律 `slint::invoke_from_event_loop` + `Weak`。
- 远控贴帧：`SharedPixelBuffer::<Rgba8Pixel>` → `Image::from_rgba8` → `set_frame`；`.slint` 里 `image-fit: fill/cover`。
- 新语法 `component X inherits Y {}`，旧 `X := Y {}` 已废；属性名 `peer-name` 在 Rust 侧是 `set_peer_name`。

**enigo（0.6，近年 API 大改，旧教程作废）**
- **必须 `use enigo::{Mouse, Keyboard}`** 否则方法不可见。
- `Enigo::new(&Settings::default())` 返回 `Result`；统一 `move_mouse(x,y,Coordinate)` / `button(Button,Direction)` / `key(Key,Direction)` / `text(&str)`，全返回 Result。
- 字符键是 `Key::Unicode('c')`（旧 `Key::Layout` 已废）。

**xcap**
- `Monitor::all()` 启动枚举一次复用，别每帧调；高帧率用 `capture_region()` + 帧差省带宽；`capture_image()` 返回 `RgbaImage`，`as_raw()` 拿 RGBA 字节。

**sysinfo**
- CPU usage 必须刷新 ≥2 次、中间 sleep `MINIMUM_CPU_UPDATE_INTERVAL`，否则为 0。
- MAC/IP 用 `Networks` 的 `mac_address()`/`ip_networks()` 自带，无需第三方 crate。
- **GPU 是 unreleased**：要用得 `sysinfo = { git = "...", features = ["gpu"] }`；国产 GPU 走 Vulkan 路径 `usage()` 返回 `None`，必须做降级。

**quinn**
- `0.11` + `rustls 0.23`；`ServerConfig::with_single_cert` / `ClientConfig::try_with_platform_verifier`；`open_bi`/`accept_bi` 双向流，单连接多路复用。

## 服务端栈速记（Claude 较熟，未单独抓取，以 docs.rs 为准）

- **WS 中转**：`axum`(0.7+) 的 `axum::extract::ws::WebSocketUpgrade`，或 `tokio-tungstenite`。消息用 `serde_json` 序列化统一信封 `{type,from,to,payload,ts}`。
- **协议单一事实源**：协议类型定义在 `crates/protocol`，用 `serde` + `ts-rs`（`#[derive(TS)] #[ts(export)]`）自动生成管理端 TS 类型，避免 Rust↔TS 漂移。
- **审计存储**：`rusqlite`（同步、轻）或 `sqlx`（异步）写文本审计（连接记录 + 操作记录）。
- **TLS**：全栈 `rustls` 纯 Rust，**不用 openssl**，规避 loongarch64 交叉编译坑。

## 全局约束（与 user.md 一致）

- 运行环境**锁 X11 会话**（xcap/enigo 在 Wayland 不可靠）。
- TLS 一律 `rustls`；GUI 用 Slint 软渲染（绕开国产 CPU 的 OpenGL/webkitgtk）。
- 信创交叉编译目标 `loongarch64-unknown-linux-gnu` / `aarch64`，参考 RustDesk。
