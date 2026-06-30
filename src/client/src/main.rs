//! OhMyDesk 客户端入口：Slint UI（主线程）+ tokio 网络/截屏/注入（后台线程）。
//!
//! 线程模型：
//! - 主线程：Slint 事件循环 `ui.run()`（阻塞）。UI 更新一律 `invoke_from_event_loop` + `Weak`。
//! - 后台线程：tokio runtime 跑 [`net::run`]（反连/注册/心跳/重连）+ ToUi 消费 + 注入/截图/推帧消费。
//! - net↔UI：`ToUi`/`FromUi` 两条 mpsc 跨线程通信。
//! - 注入/截图/截屏依赖 X11（阻塞、`!Send`），各自独立线程消费（见 [`workers`]），不混进 async select。
//!
//! 模块分工：[`ui_glue`] UI 回调 + ToUi 消费；[`workers`] X11 后台 worker；[`net`] 网络。

// Windows release 走 GUI 子系统：否则默认控制台子系统会先弹一个 cmd 黑窗再开 GUI。
// 仅 release 生效，debug 保留控制台以便看 tracing 日志；非 Windows 平台此属性为 no-op。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod asset;
mod capture;
mod elevate;
mod exec;
mod framediff;
mod telemetry;
mod render_mode;
mod geom;
mod history;
mod inject;
mod net;
mod transfer;
mod ui_glue;
mod update;
mod workers;

use slint::ComponentHandle;
use std::sync::Arc;

slint::include_modules!();

/// UI 线程与后台共享的会话 id（主控/被控各一份）。
pub(crate) type SharedSession = Arc<std::sync::Mutex<Option<String>>>;

fn main() -> anyhow::Result<()> {
    // 最先声明 DPI 感知：必须早于任何窗口/GDI/截屏初始化，否则缩放屏上 xcap 抓到模糊的虚拟化画面。
    elevate::set_dpi_aware();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "client=info".into());
    // 默认滚动日志（spec §4.2 硬性前置）：无需任何环境变量即落盘，按天滚动。
    // 目录：%APPDATA%/OhMyDesk/logs（Win）、~/.local/state/ohmydesk/logs（Linux）。
    // OHMYDESK_LOG_FILE 仍兼容（显式指定单文件时优先）。
    let _log_guard = match std::env::var("OHMYDESK_LOG_FILE") {
        Ok(p) if !p.is_empty() => {
            match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                Ok(file) => {
                    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false)
                        .with_writer(std::sync::Mutex::new(file)).init();
                    None
                }
                Err(_) => { tracing_subscriber::fmt().with_env_filter(filter).init(); None }
            }
        }
        _ => {
            let log_dir = ohmydesk_state_dir().join("logs");
            let _ = std::fs::create_dir_all(&log_dir);
            let appender = tracing_appender::rolling::daily(&log_dir, "client.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false)
                .with_writer(nb).init();
            Some(guard)
        }
    };
    // Windows：若未提权则触发 UAC 自重启（成功则本体退出），保证能向受保护/提权窗口注入。
    // 须在反连/起线程之前，避免提权副本与本体同时上线。非 Windows 为 no-op。
    elevate::ensure_elevated();
    lock_x11_session();

    // 使用人：优先命令行参数（启动脚本传 用户@主机）；缺省自动探测系统登录用户，
    // 避免直接双击 exe（未经启动脚本）时回退成「演示终端」（issue#5）。
    let user = std::env::args()
        .nth(1)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(detect_user);
    let server_url = std::env::var("OHMYDESK_SERVER").unwrap_or_else(|_| default_server_url());

    let info = asset::collect(&user);
    let self_id = info.id.clone();
    tracing::info!(
        "采集完成 id={} cpu={} ip={}",
        info.id,
        info.cpu.model,
        info.ip
    );

    let ui = AppWindow::new()?;
    ui.set_self_id(ui_glue::group_digits(&self_id).into());
    // 最近连接历史（本地持久化）初始填充
    ui.set_history(ui_glue::build_history_model(&history::load(), net::now()));

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
    let (clip_tx, clip_rx) = tokio::sync::mpsc::unbounded_channel::<net::ClipboardMsg>();
    net::CLIPBOARD_TX.init(clip_tx);
    // 遥测通道（worker 投 FrameSample、conn 投 EgressSample）
    let (tele_tx, tele_rx) = tokio::sync::mpsc::unbounded_channel::<telemetry::TelemetryMsg>();
    // 运行模式初始化（env > 启动参数 > config.toml > 默认 Frameskip）
    let arg_mode = std::env::args()
        .find_map(|a| a.strip_prefix("--render-mode=").map(|s| s.to_string()));
    let cfg_mode = std::fs::read_to_string(ohmydesk_state_dir().join("config.toml"))
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("render").and_then(|r| r.get("mode")).and_then(|m| m.as_str().map(|s| s.to_string())));
    let env_mode = std::env::var("OHMYDESK_RENDER_MODE").ok();
    let mode = render_mode::resolve(env_mode.as_deref(), arg_mode.as_deref(), cfg_mode.as_deref());
    render_mode::apply(mode);
    // 单开关环境变量覆盖（最高优先级）
    if std::env::var("OHMYDESK_FRAMESKIP").as_deref() == Ok("0") {
        render_mode::set_frameskip(false);
    }
    if std::env::var("OHMYDESK_DIRTY_TELEMETRY").as_deref() == Ok("0") {
        render_mode::set_telemetry(false);
    }
    tracing::info!("渲染模式 mode={:?} frameskip={} telemetry={}", render_mode::current_mode(), render_mode::frameskip_on(), render_mode::telemetry_on());

    // 共享：当前主控会话 id（键鼠回传需要）+ 当前被控会话 id（授权回传需要）
    let cur_session: SharedSession = Arc::new(std::sync::Mutex::new(None));
    let ctrl_session: SharedSession = Arc::new(std::sync::Mutex::new(None));
    // 已断开的主控会话 id：断开后到下次连接前，丢弃该会话迟到的帧，避免在途帧把已断开的
    // 远程态「复活」（Bug：点断开后窗口先缩小、迟到帧又重开远程视图，需点两次才真断开）。
    let ended_session: SharedSession = Arc::new(std::sync::Mutex::new(None));

    // 活动门控 + nudge 通道
    let activity = std::sync::Arc::new(activity::ClientActivityState::new(cur_session.clone(), ctrl_session.clone()));
    let (nudge_tx, nudge_rx) = std::sync::mpsc::sync_channel::<()>(4);
    update::set_nudge_sender(nudge_tx);

    // UI 回调注册（UI 线程）
    ui_glue::wire_ui_callbacks(&ui, &from_ui_tx, &cur_session, &ctrl_session, &ended_session, &activity);

    // 后台 tokio runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // 更新守护（独立 std 线程，在 to_ui_tx move 进 net::run 之前 clone）
    update::spawn_update_daemon(server_url.clone(), self_id.clone(), activity.clone(), to_ui_tx.clone(), nudge_rx);
    rt.spawn(net::run(server_url, info, to_ui_tx, from_ui_rx, tele_tx.clone()));
    rt.spawn(ui_glue::consume_to_ui(
        to_ui_rx,
        ui.as_weak(),
        cur_session,
        ctrl_session,
        ended_session,
        activity.clone(),
    ));
    rt.spawn(workers::consume_inject(inject_rx));
    rt.spawn(workers::consume_screenshot(shot_rx, from_ui_tx.clone()));
    rt.spawn(workers::consume_capture(cap_rx, from_ui_tx.clone(), tele_tx.clone()));
    rt.spawn(workers::consume_clipboard(clip_rx, from_ui_tx));
    rt.spawn(telemetry::run_collector(tele_rx, ohmydesk_state_dir().join("diag")));

    // 主线程进入 Slint 事件循环（阻塞）
    ui.run()?;
    Ok(())
}

fn default_server_url() -> String {
    "wss://rc.guoziweb.com/ws".into()
}

/// 由登录用户名 + 主机名组合「使用人」展示标签。纯函数，便于单测（issue#5）。
fn compose_user_label(user: Option<String>, host: Option<String>) -> String {
    let user = user.filter(|s| !s.is_empty());
    let host = host.filter(|s| !s.is_empty());
    match (user, host) {
        (Some(u), Some(h)) => format!("{u}@{h}"),
        (Some(u), None) => u,
        (None, Some(h)) => h,
        (None, None) => "未知终端".into(),
    }
}

/// 探测系统当前登录用户（直接运行 exe、未经启动脚本传名时的兜底）。
/// 登录用户环境变量各平台不同：Unix=USER/LOGNAME，Windows=USERNAME。
fn detect_user() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .ok();
    compose_user_label(user, sysinfo::System::host_name())
}

/// 锁 X11 会话（项目硬约束：xcap/enigo 在 Wayland 不可靠）。WSL2 等环境残留的 WAYLAND_DISPLAY
/// 会让 xcap 选 wayland 后端 panic（UnsupportedVersion），故进程级强制 X11：清 WAYLAND_DISPLAY +
/// 标记 session 为 x11 + 软渲染兜底，让 xcap/winit 统一走 X11。真实信创 X11 机本无此变量。
fn lock_x11_session() {
    // 先记录“原本是 Wayland 会话”——下面会抹掉 WAYLAND_DISPLAY 强制 X11 后端，
    // 但真实 Wayland 会话即便连到 Xwayland 也抓不到桌面，截屏线程据此明确回执主控端。
    let is_wayland = std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
    if is_wayland {
        std::env::set_var("OHMYDESK_WAYLAND", "1");
    }
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

/// 跨平台状态目录：Win=%APPDATA%/OhMyDesk，Linux=~/.local/state/ohmydesk。
fn ohmydesk_state_dir() -> std::path::PathBuf {
    if let Some(pd) = directories::ProjectDirs::from("", "", "OhMyDesk") {
        #[cfg(windows)]
        { return pd.data_dir().to_path_buf(); }
        #[cfg(not(windows))]
        { return pd.state_dir().map(|p| p.to_path_buf()).unwrap_or_else(|| pd.data_local_dir().to_path_buf()); }
    }
    std::env::temp_dir().join("ohmydesk")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 客户端默认服务器地址_所有平台指向公网中转() {
        assert_eq!(default_server_url(), "wss://rc.guoziweb.com/ws");
    }

    #[test]
    fn 使用人标签_用户加主机组合() {
        assert_eq!(
            compose_user_label(Some("chin".into()), Some("guozi".into())),
            "chin@guozi"
        );
        // 缺主机名时仅用户名
        assert_eq!(compose_user_label(Some("chin".into()), None), "chin");
        // 缺用户名时仅主机名
        assert_eq!(compose_user_label(None, Some("guozi".into())), "guozi");
        // 空串视为缺失
        assert_eq!(
            compose_user_label(Some("".into()), Some("guozi".into())),
            "guozi"
        );
        // 全缺回退（绝不再用「演示终端」）
        assert_eq!(compose_user_label(None, None), "未知终端");
    }
}
