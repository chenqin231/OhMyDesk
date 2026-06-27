//! OhMyDesk 客户端入口：Slint UI（主线程）+ tokio 网络/截屏/注入（后台线程）。
//!
//! 线程模型：
//! - 主线程：Slint 事件循环 `ui.run()`（阻塞）。UI 更新一律 `invoke_from_event_loop` + `Weak`。
//! - 后台线程：tokio runtime 跑 [`net::run`]（反连/注册/心跳/重连）+ ToUi 消费 + 注入/截图/推帧消费。
//! - net↔UI：`ToUi`/`FromUi` 两条 mpsc 跨线程通信。
//! - 注入/截图/截屏依赖 X11（阻塞、`!Send`），各自独立线程消费（见 [`workers`]），不混进 async select。
//!
//! 模块分工：[`ui_glue`] UI 回调 + ToUi 消费；[`workers`] X11 后台 worker；[`net`] 网络。

mod asset;
mod capture;
mod geom;
mod inject;
mod net;
mod ui_glue;
mod workers;

use slint::ComponentHandle;
use std::sync::Arc;

slint::include_modules!();

/// UI 线程与后台共享的会话 id（主控/被控各一份）。
pub(crate) type SharedSession = Arc<std::sync::Mutex<Option<String>>>;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "client=info".into()),
        )
        .init();
    lock_x11_session();

    let user = std::env::args().nth(1).unwrap_or_else(|| "演示终端".into());
    let server_url =
        std::env::var("OHMYDESK_SERVER").unwrap_or_else(|_| "ws://127.0.0.1:8765/ws".into());

    let info = asset::collect(&user);
    let self_id = info.id.clone();
    tracing::info!(
        "采集完成 id={} cpu={} ip={}",
        info.id,
        info.cpu.model,
        info.ip
    );

    let ui = AppWindow::new()?;
    ui.set_self_id(self_id.into());

    // net ↔ UI 双向通道
    let (to_ui_tx, to_ui_rx) = tokio::sync::mpsc::unbounded_channel::<net::ToUi>();
    let (from_ui_tx, from_ui_rx) = tokio::sync::mpsc::unbounded_channel::<net::FromUi>();
    // 旁路：net 收下行 → 交 main 的 X11 执行侧
    let (inject_tx, inject_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, protocol::InputEvent)>();
    net::INJECT_TX.init(inject_tx);
    let (shot_tx, shot_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    net::SCREENSHOT_TX.init(shot_tx);
    let (cap_tx, cap_rx) = tokio::sync::mpsc::unbounded_channel::<net::CaptureCtrl>();
    net::CAPTURE_CTRL.init(cap_tx);

    // 共享：当前主控会话 id（键鼠回传需要）+ 当前被控会话 id（授权回传需要）
    let cur_session: SharedSession = Arc::new(std::sync::Mutex::new(None));
    let ctrl_session: SharedSession = Arc::new(std::sync::Mutex::new(None));

    // UI 回调注册（UI 线程）
    ui_glue::wire_ui_callbacks(&ui, &from_ui_tx, &cur_session, &ctrl_session);

    // 后台 tokio runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.spawn(net::run(server_url, info, to_ui_tx, from_ui_rx));
    rt.spawn(ui_glue::consume_to_ui(
        to_ui_rx,
        ui.as_weak(),
        cur_session,
        ctrl_session,
    ));
    rt.spawn(workers::consume_inject(inject_rx));
    rt.spawn(workers::consume_screenshot(shot_rx));
    rt.spawn(workers::consume_capture(cap_rx, from_ui_tx));

    // 主线程进入 Slint 事件循环（阻塞）
    ui.run()?;
    Ok(())
}

/// 锁 X11 会话（项目硬约束：xcap/enigo 在 Wayland 不可靠）。WSL2 等环境残留的 WAYLAND_DISPLAY
/// 会让 xcap 选 wayland 后端 panic（UnsupportedVersion），故进程级强制 X11：清 WAYLAND_DISPLAY +
/// 标记 session 为 x11 + 软渲染兜底，让 xcap/winit 统一走 X11。真实信创 X11 机本无此变量。
fn lock_x11_session() {
    std::env::remove_var("WAYLAND_DISPLAY");
    if std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        std::env::set_var("XDG_SESSION_TYPE", "x11");
    }
    // 软渲染兜底（防环境残留 GPU 后端被选中）。
    if std::env::var("SLINT_BACKEND").is_err() {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }
}
