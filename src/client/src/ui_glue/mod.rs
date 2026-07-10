//! UI 胶水：Slint 回调注册（UI 线程）+ ToUi 流消费（更新 UI）+ 帧解码。
//!
//! UI 更新一律 `invoke_from_event_loop` + `Weak`（AppWindow 强句柄非 Send）。

use crate::{history, net, AppWindow, ChatNoticeWindow, SharedSession};
use slint::ComponentHandle;

mod control;
mod input;
mod util;
pub use util::{
    append_line, build_file_model, build_history_model, group_digits, join_path, parent_of,
    rel_time, resolve_path_arg,
};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering as AtomicOrdering};

/// 用户点「分辨率」档 → 置位；主控 Frame 处理在下一次帧尺寸变化时据此把窗口一次性重贴合到
/// 被控分辨率（用户触发=单次 set_size，不受 adaptive 抖动影响，避开首帧后「不再动窗口」的风暴）。
static REFIT_PENDING: AtomicBool = AtomicBool::new(false);
/// 上次分辨率档位（仅分辨率轴变化才触发窗口重贴合；清晰度/帧率不改帧尺寸，不该动窗口）。
static LAST_RES_TIER: AtomicI32 = AtomicI32::new(-1);

/// 采集侧回调共享句柄打包。全字段皆 Arc/Sender，clone 廉价。
pub(crate) struct UiCtx {
    pub from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    pub cur_session: SharedSession,
    pub ctrl_session: SharedSession,
    pub ended_session: SharedSession,
    pub activity: std::sync::Arc<crate::activity::ClientActivityState>,
    pub telemetry_tx: tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
}

/// 注册全部 UI 回调（运行在 UI 线程，把动作经 from_ui_tx 投给 net）。
pub fn wire_ui_callbacks(
    ui: &AppWindow,
    from_ui_tx: &tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    telemetry_tx: &tokio::sync::mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
    let cx = UiCtx {
        from_ui_tx: from_ui_tx.clone(),
        cur_session: cur_session.clone(),
        ctrl_session: ctrl_session.clone(),
        ended_session: ended_session.clone(),
        activity: activity.clone(),
        telemetry_tx: telemetry_tx.clone(),
    };
    control::wire(ui, &cx);
    input::wire(ui, &cx);
    // 复制 ID/密码到剪贴板（ID 分组带空格，先去白）
    {
        ui.on_copy_text(move |s| {
            let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(cleaned);
            }
        });
    }
    // 复制更新下载链接（URL 原样复制，不过滤空白，避免吃掉 URL 字符）
    {
        ui.on_copy_url(move |s| {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(s.to_string()); // URL 原样复制，不做白空格过滤
            }
        });
    }
    // 手动检查更新：先置「检查中」状态再唤醒守护立即做一次检查（nudge）
    {
        let ui_weak = ui.as_weak();
        ui.on_check_update(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_update_status("正在检查更新…".into());
                ui.set_update_phase(1);
            }
            crate::update::nudge();
        });
    }
    // AI助手:检测/静默安装/拉起素问。仅 Windows 显示按钮(其余平台 supported=false 隐藏)。
    {
        ui.set_suwen_supported(cfg!(windows));
        let ui_weak = ui.as_weak();
        ui.on_launch_suwen(move || {
            crate::suwen::ensure_and_launch(ui_weak.clone());
        });
    }
    // 刷新临时密码（重发 Register，server upsert 覆盖；新密码经 Registered 回推展示）
    {
        let tx = from_ui_tx.clone();
        ui.on_refresh_password(move || {
            let _ = tx.send(net::FromUi::RefreshPassword);
        });
    }
    // ── tab 切换 → 懒推流：tab 0(远程桌面)发 SetCapture{active:true}，其余 false ──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_tab_changed(move |tab| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SetCapture {
                    session_id: sid,
                    active: tab == 0,
                });
            }
        });
    }
    // ── 远程命令：执行（本地回显命令行，回执到达后追加结果块）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_run_command(move |command| {
            let command = command.to_string();
            if command.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::ExecCommand {
                    session_id: sid,
                    command: command.clone(),
                });
                // 本地回显命令行（下行 ExecResult 不带 command 原文，此处回显补齐，解决 Minor #1）。
                if let Some(ui) = ui_weak.upgrade() {
                    let prev = ui.get_cmd_output().to_string();
                    let echo = format!("$ {command}");
                    let next = if prev.is_empty() {
                        echo
                    } else {
                        format!("{prev}\n\n{echo}")
                    };
                    ui.set_cmd_output(next.into());
                }
            }
        });
    }
    // ── 远程文件：浏览本机目录（左栏，复用 transfer::list_dir 列本机任意路径）──
    {
        let ui_weak = ui.as_weak();
        ui.on_list_local(move |arg| {
            let arg = arg.to_string();
            let ui_weak = ui_weak.clone();
            // 解析 Slint 传来的指令串（<up>:/<cd>: 标记）→ 目标绝对路径
            let cur = ui_weak
                .upgrade()
                .map(|u| u.get_local_path().to_string())
                .unwrap_or_default();
            let target = resolve_path_arg(&arg, &cur);
            // 列目录是阻塞 IO，放后台线程，完成后投回 UI 线程 set。
            std::thread::spawn(move || {
                let listed = crate::transfer::list_dir(&target);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match listed {
                            Ok((dir, entries)) => {
                                ui.set_local_path(dir.into());
                                ui.set_local_entries(build_file_model(&entries));
                            }
                            Err(reason) => {
                                ui.set_file_notice(format!("本机目录读取失败：{reason}").into());
                            }
                        }
                    }
                });
            });
        });
    }
    // ── 远程文件：浏览远端目录（右栏）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_list_remote(move |arg| {
            let arg = arg.to_string();
            let cur = ui_weak
                .upgrade()
                .map(|u| u.get_remote_path().to_string())
                .unwrap_or_default();
            let target = resolve_path_arg(&arg, &cur);
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::ListRemote {
                    session_id: sid,
                    path: target,
                });
            }
        });
    }
    // ── 远程文件：下发（左栏选中文件 → 右栏当前目录）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_push_file(move |name| {
            let name = name.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let local_dir = ui.get_local_path().to_string();
                let dest_dir = ui.get_remote_path().to_string();
                let local_path = join_path(&local_dir, &name);
                if let Some(sid) = sess.lock().unwrap().clone() {
                    let _ = tx.send(net::FromUi::PushFile {
                        session_id: sid,
                        local_path,
                        dest_dir,
                    });
                }
            }
        });
    }
    // ── 远程文件：取回（右栏选中文件 → 左栏当前目录）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_pull_file(move |name| {
            let name = name.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let remote_dir = ui.get_remote_path().to_string();
                let local_dir = ui.get_local_path().to_string();
                let remote_path = join_path(&remote_dir, &name);
                if let Some(sid) = sess.lock().unwrap().clone() {
                    let _ = tx.send(net::FromUi::PullFile {
                        session_id: sid,
                        remote_path,
                        local_dir,
                    });
                }
            }
        });
    }
    // ── 即时消息：主控发送（本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_chat_log().to_string();
                    ui.set_chat_log(append_line(&log, "我", &text).into());
                }
            }
        });
    }
    // ── 即时消息：被控发送（用被控会话 ctrl_session，本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_controlled_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_controlled_chat_log().to_string();
                    ui.set_controlled_chat_log(append_line(&log, "我", &text).into());
                }
            }
        });
    }
    // ── 诊断菜单：模式热切（render_mode 原子，运行期即时生效，无需重启） ──
    ui.on_set_render_mode(move |m| {
        if let Some(mode) = crate::render_mode::parse_mode(&m.to_string()) {
            crate::render_mode::apply(mode);
            tracing::warn!("UI 热切渲染模式 → {:?}", crate::render_mode::current_mode());
        }
    });
    // ── 诊断菜单：导出诊断包（发 ExportNow 给 collector 落盘） ──
    {
        let tele = telemetry_tx.clone();
        ui.on_export_diag(move || {
            let _ = tele.send(crate::telemetry::TelemetryMsg::ExportNow);
        });
    }
    // ── 诊断菜单：复制诊断目录路径（复用已有 copy_text callback） ──
    {
        let ui_weak = ui.as_weak();
        ui.on_copy_diag_path(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.invoke_copy_text(ui.get_diag_dir());
            }
        });
    }
}

/// 该帧是否属于「已断开」会话——是则丢弃，不渲染、不复活远程态（修复需点两次断开的 Bug）。
fn frame_belongs_to_ended(ended: &Option<String>, session_id: &str) -> bool {
    ended.as_deref() == Some(session_id)
}

/// SessionEnd 到达时，UI 侧被控会话副本 `ctrl_session` 的门控清理。
///
/// 与权威源 `SessionCtx.controlled` 的清理条件对齐（见 `net/dispatch.rs` SessionEnd：
/// 仅当结束的 session_id 等于当前被控会话时才清）。这样在「重控 / 多会话 / 迟到
/// SessionEnd」序列下，`ctrl_session` 不会被无关会话的结束错误清空或指向失效 id，
/// 避免被控发聊天带失效 session_id 上行被服务端静默丢弃。
///
/// - `current == Some(ending)`：结束的正是当前被控会话 → 清空。
/// - `current == Some(其它)`：结束的是旧/别的会话（如迟到 SessionEnd{S1}，而当前已重控 S2）→ 保留。
/// - `current == None`：本无被控会话 → 保持 None。
fn next_ctrl_session_after_end(current: Option<&str>, ending_session_id: &str) -> Option<String> {
    if current == Some(ending_session_id) {
        None
    } else {
        current.map(str::to_owned)
    }
}

/// 登录 / 注销回调（客户端账号登录 + 归属绑定）。独立于 net 上行通道：
/// - `on_do_login`：后台线程阻塞 ureq 调 `/api/login` → `invoke_from_event_loop` 回填 UI；
///   成功则存盘凭据 + 经 `token_tx`(watch) 通知 `net::run` 携 token 上线（放行登录门，服务端派生 owner）。
/// - `on_do_logout`：清凭据 + token 置 None（令 net 主动断开 WS + 回登录门）+ 回登录页。
pub fn wire_login_callbacks(
    ui: &AppWindow,
    token_tx: std::sync::Arc<tokio::sync::watch::Sender<Option<String>>>,
    server_url: String,
    active_server_url: crate::SharedServerUrl,
) {
    // on_do_login(user, pass, server_override)
    {
        let ui_weak = ui.as_weak();
        let token_tx = token_tx.clone();
        let default_server = server_url;
        let active_server_url = active_server_url.clone();
        ui.on_do_login(move |user, pass, server_override| {
            let user = user.to_string();
            let pass = pass.to_string();
            // 服务器地址：高级项非空则用它，否则用默认（env/OHMYDESK_SERVER）。
            let server = selected_login_server(&default_server, &server_override);
            let ui_weak = ui_weak.clone();
            let token_tx = token_tx.clone();
            let active_server_url = active_server_url.clone();
            // 进入 loading 态（回调在 UI 线程，直接 set）。
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_login_loading(true);
                ui.set_login_error("".into());
            }
            // 阻塞式 ureq 放后台线程，完成后投回 UI 线程回填。
            std::thread::spawn(move || {
                let result = crate::login::login(&server, &user, &pass);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else {
                        return;
                    };
                    ui.set_login_loading(false);
                    match result {
                        Ok(creds) => {
                            crate::credential::save(&creds);
                            *active_server_url.lock().unwrap() = server;
                            ui.set_logged_user(creds.user.clone().into());
                            ui.set_login_pass("".into()); // 清密码框
                            ui.set_login_error("".into());
                            ui.set_logged_in(true);
                            // 放行 net 登录门：携 token 上线（服务端据此派生 owner）。
                            let _ = token_tx.send(Some(creds.token));
                        }
                        Err(e) => {
                            // 错密码：清空密码框、账号保留（AC-001-E1）；网络错则保留密码便于直接重试。
                            if e == crate::login::LoginErr::BadCredential {
                                ui.set_login_pass("".into());
                            }
                            ui.set_login_error(e.message().into());
                        }
                    }
                });
            });
        });
    }
    // on_do_logout（S3 确定）
    {
        let ui_weak = ui.as_weak();
        let token_tx = token_tx.clone();
        ui.on_do_logout(move || {
            crate::credential::clear();
            let _ = token_tx.send(None); // 令 net 主动断开 WS + 回登录门（终端离线，归属服务端保留）
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_logged_in(false);
                ui.set_connected(false);
                ui.set_login_pass("".into());
                ui.set_login_error("".into());
            }
        });
    }
}

pub fn wire_chat_notice_callbacks(ui: &AppWindow, notice: &ChatNoticeWindow) {
    {
        let ui_weak = ui.as_weak();
        let notice_weak = notice.as_weak();
        notice.on_open_chat(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_chat_panel_open(true);
                ui.set_controlled_chat_unread(false);
                ui.window().set_minimized(false);
                let _ = ui.show();
            }
            if let Some(notice) = notice_weak.upgrade() {
                let _ = notice.hide();
            }
        });
    }

    {
        let notice_weak = notice.as_weak();
        notice.on_dismiss(move || {
            if let Some(notice) = notice_weak.upgrade() {
                let _ = notice.hide();
            }
        });
    }

    {
        let notice_weak = notice.as_weak();
        ui.on_controlled_chat_panel_opened(move || {
            if let Some(notice) = notice_weak.upgrade() {
                let _ = notice.hide();
            }
        });
    }
}

/// 修「最小化→托盘图标恢复后白板」：Slint 软渲染器按 softbuffer buffer age 做脏区复用——
/// 假设上一帧像素仍在缓冲里。Windows 最小化→恢复不重建表面(age 仍有效),但窗口内容已被 OS 清空,
/// 于是渲染器只重绘「新脏区」(如点中的控件)、其余留白 = 白板+局部。仅 request_redraw 无效(脏区为空)。
/// 根治：恢复瞬间把窗口逻辑高度 +1px 再复原,强制 winit 重建 softbuffer 表面(buffer age=0 → 整窗重绘)。
/// 触发门：最小化态→非最小化态(Windows)或 Occluded(false)(X11/Wayland/macOS);最大化/全屏不 nudge
/// (winit 忽略尺寸变更,且此类恢复通常自带 Resized 会整窗重绘)。
pub fn wire_repaint_on_restore(ui: &AppWindow) {
    use i_slint_backend_winit::winit::event::WindowEvent;
    use i_slint_backend_winit::{EventResult, WinitWindowAccessor};
    use std::cell::Cell;

    let ui_weak = ui.as_weak();
    // 最小化状态跟踪：true→false = 恢复。回调对多数 winit 事件触发,故正常使用(未最小化)恒 false,不误触。
    let was_minimized = Cell::new(false);
    ui.window().on_winit_window_event(move |win, ev| {
        let now_min = win.is_minimized();
        let restored = was_minimized.get() && !now_min;
        was_minimized.set(now_min);

        if restored || matches!(ev, WindowEvent::Occluded(false)) {
            win.request_redraw();
            if let Some(ui) = ui_weak.upgrade() {
                let w = ui.window();
                if !w.is_maximized() && !w.is_fullscreen() {
                    let sf = w.scale_factor();
                    let sz = w.size(); // 物理像素 → 逻辑像素(set_size 要逻辑)
                    let lw = sz.width as f32 / sf;
                    let lh = sz.height as f32 / sf;
                    w.set_size(slint::LogicalSize::new(lw, lh + 1.0));
                    let back = ui.as_weak();
                    slint::Timer::single_shot(std::time::Duration::from_millis(32), move || {
                        if let Some(ui) = back.upgrade() {
                            ui.window().set_size(slint::LogicalSize::new(lw, lh));
                        }
                    });
                }
            }
        }
        EventResult::Propagate
    });
}

fn selected_login_server(default_server: &str, server_override: &str) -> String {
    let server = server_override.trim();
    if server.is_empty() {
        default_server.to_string()
    } else {
        server.to_string()
    }
}

fn should_show_controlled_chat_notice(chat_panel_open: bool) -> bool {
    !chat_panel_open
}

/// 拉 ToUi 流，逐条应用到 UI（invoke_from_event_loop），并维护主控/被控会话 id。
pub async fn consume_to_ui(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<net::ToUi>,
    ui_weak: slint::Weak<AppWindow>,
    chat_notice_weak: slint::Weak<ChatNoticeWindow>,
    cur_session: SharedSession,
    ctrl_session: SharedSession,
    ended_session: SharedSession,
    activity: std::sync::Arc<crate::activity::ClientActivityState>,
    token_tx: std::sync::Arc<tokio::sync::watch::Sender<Option<String>>>,
) {
    // 诊断画面发虚：记录主控实际收到的帧分辨率，变化时打印（流畅=1280×720 / 高清=1920×1080 上限）。
    // 据此判断高清是否真生效、被控源分辨率多大。
    let mut last_frame_dims: Option<(u32, u32)> = None;
    let mut recv_stats = crate::telemetry::MainRecvStats::default();
    while let Some(mut ev) = rx.recv().await {
        // 丢过期帧：收到 Frame 时若通道里还有积压，丢弃当前帧取下一条——只解码/渲染最新帧，
        // 消除「操作后看到一串旧画面」的滞后感（主控渲染慢于被控推帧时积压会堆积）。
        let mut dropped = 0u32;
        while matches!(ev, net::ToUi::Frame { .. }) {
            match rx.try_recv() {
                Ok(next) => {
                    ev = next;
                    dropped += 1;
                }
                Err(_) => break,
            }
        }
        if dropped > 0 {
            recv_stats.on_drop_stale(dropped);
        }
        let ui_weak = ui_weak.clone();
        match ev {
            net::ToUi::Registered { id, password } => {
                let id_disp = group_digits(&id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_self_id(id_disp.into());
                        ui.set_self_password(password.into());
                        ui.set_connected(true);
                        // 重连/注册成功即清除「与服务器断开，重连中…」残留横幅。
                        // Disconnected 会置该串，但此前无人清它 → 抖一次后永久钉死（在线却显红串）。
                        ui.set_remote_status("".into());
                    }
                });
            }
            net::ToUi::ControlRequest {
                requester,
                session_id,
                source,
            } => {
                // 记下被控会话 id，授权回调据此回传 AuthResult
                *ctrl_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_requester(requester.into());
                        ui.set_auth_source(source.into());
                        ui.set_auth_countdown(60);
                        ui.set_auth_pending(true);
                    }
                });
            }
            net::ToUi::BeingControlled {
                peer_name,
                forced,
                session_id,
            } => {
                *ctrl_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_pending(false);
                        ui.set_peer_name(peer_name.into());
                        ui.set_controlled_forced(forced);
                        ui.set_being_controlled(true);
                    }
                });
            }
            net::ToUi::RemoteAck { session_id } => {
                *ended_session.lock().unwrap() = None; // 新会话建立：解除丢帧标记
                *cur_session.lock().unwrap() = Some(session_id);
                activity.end_pending_connect();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_awaiting_consent(false);
                        ui.set_connecting(false);
                        ui.set_remote_status("".into());
                        ui.set_remote_active(true);
                        // 新会话进入工作台：回到远程桌面标签，同时消除懒推流接缝
                        // （主控落桌面标签、被控进入态默认推流，二者一致）。
                        ui.set_active_tab(0);
                        // 清空各标签会话内残留状态，避免上一会话（目标 X）的数据带入本会话（目标 Y）。
                        // 命令输出 / 聊天记录 / 未读红点 / 文件状态行清空；目录列表由下方 invoke 重新列出。
                        ui.set_cmd_output("".into());
                        ui.set_chat_log("".into());
                        ui.set_chat_unread(false);
                        ui.set_file_notice("".into());
                        // 进入主控画面态：放大窗口给远程桌面腾空间
                        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
                        // 进入工作台：左栏列本机 home、右栏列远端默认目录（空路径=被控 home）。
                        // invoke_<callback> 主动触发已接线的列目录逻辑，不重复写。
                        ui.invoke_list_local("".into());
                        ui.invoke_list_remote("".into());
                    }
                });
            }
            net::ToUi::RemoteRejected { reason } => {
                *cur_session.lock().unwrap() = None;
                activity.end_pending_connect();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_awaiting_consent(false);
                        ui.set_connecting(false);
                        ui.set_remote_status(format!("连接失败：{reason}").into());
                        ui.set_remote_active(false);
                    }
                });
            }
            net::ToUi::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            } => {
                // 丢弃已断开会话的迟到帧：否则在途帧会把已断开的远程态「复活」（需点两次断开）。
                if frame_belongs_to_ended(&ended_session.lock().unwrap(), &session_id) {
                    continue;
                }
                // 首帧标志:仅连上远程收到的第一帧才自动贴合窗口尺寸(见下方 set_size)。
                // 之后 adaptive 过载降档会让分辨率不停变，若每次都 set_size，窗口会在用户
                // 拖动/操作时被强行改尺寸+重定位 → 表现为「窗口随机变大小和位置」「最大化下字体割裂」。
                let is_first_frame = last_frame_dims.is_none();
                let dims_changed = last_frame_dims != Some((w, h));
                if dims_changed {
                    tracing::info!(
                        "主控收到帧分辨率={w}x{h}（流畅档≤1280×720 / 高清档≤1920×1080）"
                    );
                    last_frame_dims = Some((w, h));
                }
                // 统一会话态：收到帧即把 cur_session 设为该会话——保证「有画面时输入一定有目标」，
                // 即便 RemoteAck 因时序/路由未设上 cur_session，输入也不会被静默丢弃。
                {
                    let mut s = cur_session.lock().unwrap();
                    if s.as_deref() != Some(session_id.as_str()) {
                        *s = Some(session_id.clone());
                    }
                }
                // 在本（tokio）线程解码 JPEG→RGBA（产出 Vec<u8> 是 Send）；Image 非 Send，
                // 故只把裸 RGBA + 尺寸传进闭包，在 UI 线程内构造 Image（slint.md §3 坑 2）。
                let t_dec = std::time::Instant::now();
                let decoded = decode_frame_rgba(&data);
                let decode_ms = t_dec.elapsed().as_millis() as u32;
                if let Some(line) = recv_stats.on_frame(seq, decode_ms, crate::update::now_ms()) {
                    tracing::info!("{line}");
                }
                if let Ok((rgba, iw, ih)) = decoded {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let mut buffer =
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(iw, ih);
                            buffer.make_mut_bytes().copy_from_slice(&rgba);
                            ui.set_frame_w(w as i32);
                            ui.set_frame_h(h as i32);
                            ui.set_frame(slint::Image::from_rgba8(buffer));
                            ui.set_remote_active(true);
                            // 把主控窗口调到接近被控分辨率，让远程桌面尽量 1:1 显示，避免被压进小窗
                            // 强制下采样导致发虚。仅尺寸变化时调整。
                            // DPI 感知：set_size 用逻辑像素，除以主控缩放系数，使窗口的「物理」尺寸≈帧尺寸
                            // （高 DPI 主控上才不会把窗口放大到溢出屏幕）。上限取常见屏物理 1920×1080。
                            //
                            // 【仅首帧贴合，且非最大化/全屏】只在连上远程的第一帧把窗口调到接近被控
                            // 分辨率。之后 adaptive 过载降档让分辨率频繁跳变（1920↔1632↔1344↔1056…），
                            // 若每次都 set_size，会与窗口管理器/用户拖动抢状态 → 窗口随机变大小和位置、
                            // 最大化下渲染表面与布局 desync 致字体割裂。首帧后一律不再动窗口，画面靠
                            // frame_scale 在窗口内自适应缩放。
                            let win = ui.window();
                            // 首帧贴合 或 用户切分辨率档触发的一次性重贴合（REFIT_PENDING）。
                            // 均要求「尺寸真变 + 非最大化/全屏」，且重贴合后清位——单次 set_size，
                            // 不受 adaptive 降档抖动影响，避免窗口风暴/最大化字体割裂（见 900-946 注释）。
                            let want_refit =
                                is_first_frame || REFIT_PENDING.load(AtomicOrdering::Relaxed);
                            if want_refit
                                && dims_changed
                                && !win.is_maximized()
                                && !win.is_fullscreen()
                            {
                                let sf = win.scale_factor().max(1.0);
                                let win_w = (w.min(1920) as f32) / sf;
                                let win_h = (h.min(1080) as f32) / sf;
                                win.set_size(slint::LogicalSize::new(win_w, win_h));
                                REFIT_PENDING.store(false, AtomicOrdering::Relaxed);
                            }
                        }
                    });
                }
            }
            net::ToUi::Cursor { visible, shape } => {
                // 解码裸 RGBA(base64)→ UI 线程构造 Image 并设光标属性;shape=None 仅改可见性。
                let decoded = shape.as_ref().and_then(|s| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD
                        .decode(&s.rgba)
                        .ok()
                        .filter(|b| b.len() == (s.w * s.h * 4) as usize)
                        .map(|b| (b, s.w, s.h, s.hotspot_x, s.hotspot_y))
                });
                let ui_weak = ui_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_cursor_visible(visible);
                        if let Some((bytes, cw, ch, hx, hy)) = decoded {
                            let mut buffer =
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(cw, ch);
                            buffer.make_mut_bytes().copy_from_slice(&bytes);
                            ui.set_cursor_img(slint::Image::from_rgba8(buffer));
                            ui.set_cursor_w(cw as i32);
                            ui.set_cursor_h(ch as i32);
                            ui.set_cursor_hotspot_x(hx as i32);
                            ui.set_cursor_hotspot_y(hy as i32);
                            ui.set_cursor_ready(true);
                        }
                    }
                });
            }
            net::ToUi::SessionEnded { session_id } => {
                // 重置首帧标志：窗口贴合是「每会话一次」而非「每进程一次」。否则重连新会话时
                // last_frame_dims 仍是上次的 Some(..)，is_first_frame 恒 false → 新会话首帧不再
                // 贴合窗口，卡在上次尺寸。置 None 让下次连接的首帧重新贴合（不重新引入 set_size 风暴：
                // 会话内首帧后 last_frame_dims 即非 None，其余帧仍不触发）。
                last_frame_dims = None;
                // 记下结束的会话 id，丢弃其迟到帧（与本地断开同样防「复活」）。
                activity.end_pending_connect();
                let prev = cur_session.lock().unwrap().take();
                if prev.is_some() {
                    *ended_session.lock().unwrap() = prev;
                }
                // 被控会话结束：门控清理被控会话副本——只有结束的正是当前被控会话才清，
                // 否则保留（对齐权威源 dispatch.rs 的按 session_id 清理）。避免重控/多会话/
                // 迟到 SessionEnd 下 `ctrl_session` 被无关会话错误清空 → 被控发聊天带失效 id 被丢弃。
                {
                    let mut g = ctrl_session.lock().unwrap();
                    *g = next_ctrl_session_after_end(g.as_deref(), &session_id);
                }
                let chat_notice_weak = chat_notice_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let was_controlling = ui.get_remote_active();
                        // 撤销可能仍开着的授权弹窗（主控在被控同意前取消了申请，issue#4）。
                        ui.set_auth_pending(false);
                        ui.set_being_controlled(false);
                        ui.set_connecting(false);
                        ui.set_remote_active(false);
                        ui.set_cursor_ready(false); // 隐藏光标同步叠加层
                        ui.set_remote_status("会话已结束".into());
                        // 会话结束清空各标签状态（spec §12）：回到远程桌面标签，下次新会话
                        // active_tab 不残留非 0（与被控进入态默认推流一致，消除懒推流接缝）。
                        ui.set_active_tab(0);
                        // 命令输出 / 主控聊天记录 / 未读红点 清空。
                        ui.set_cmd_output("".into());
                        ui.set_chat_log("".into());
                        ui.set_chat_unread(false);
                        // 被控聊天记录 / 入口红点 / 面板开合复位，避免残留。
                        ui.set_controlled_chat_log("".into());
                        ui.set_controlled_chat_unread(false);
                        ui.set_chat_panel_open(false);
                        // 远端 / 本机目录条目与路径、文件状态行清空（下次会话由 RemoteAck 重列）。
                        ui.set_remote_entries(build_file_model(&[]));
                        ui.set_remote_path("".into());
                        ui.set_local_entries(build_file_model(&[]));
                        ui.set_local_path("".into());
                        ui.set_file_notice("".into());
                        // 退出主控画面态：缩回紧凑小窗
                        if was_controlling {
                            ui.window().set_size(slint::LogicalSize::new(460.0, 620.0));
                        }
                    }
                    if let Some(notice) = chat_notice_weak.upgrade() {
                        let _ = notice.hide();
                    }
                });
            }
            net::ToUi::Disconnected => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_connected(false);
                        ui.set_remote_status("与服务器断开，重连中…".into());
                    }
                });
            }
            net::ToUi::AuthExpired => {
                // token 失效/过期（服务端 close 1008）：清凭据 + token 置 None（停重连循环，
                // 否则会拿着过期 token 反复重连被拒），回登录页提示重新登录。
                crate::credential::clear();
                let _ = token_tx.send(None);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_connected(false);
                        ui.set_logged_in(false);
                        ui.set_login_pass("".into());
                        ui.set_login_error("登录已过期，请重新登录".into());
                    }
                });
            }
            // ── 远程命令：被控回执 → 累积到命令输出区 ──
            net::ToUi::ExecResult {
                exit_code,
                stdout,
                stderr,
                truncated,
                duration_ms,
                ..
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "无(超时/未启动)".into());
                let mut block = format!("退出码 {code} · 耗时 {duration_ms}ms");
                if !stdout.is_empty() {
                    block.push_str(&format!("\n{stdout}"));
                }
                if !stderr.is_empty() {
                    block.push_str(&format!("\n[stderr] {stderr}"));
                }
                if truncated {
                    block.push_str("\n[输出已截断]");
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        // 结果块紧跟其上方刚回显的「$ 命令」行（单换行），不同命令间已由回显侧空行分隔。
                        let prev = ui.get_cmd_output().to_string();
                        let next = if prev.is_empty() {
                            block
                        } else {
                            format!("{prev}\n{block}")
                        };
                        ui.set_cmd_output(next.into());
                    }
                });
            }
            // ── 远程文件：远端目录列表 → 右栏渲染 ──
            net::ToUi::RemoteEntries {
                path,
                entries,
                error,
            } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match error {
                            Some(reason) => {
                                ui.set_file_notice(format!("远端目录读取失败：{reason}").into())
                            }
                            None => {
                                ui.set_remote_path(path.into());
                                ui.set_remote_entries(build_file_model(&entries));
                            }
                        }
                    }
                });
            }
            // ── 文件传输进度 → 状态行 ──
            net::ToUi::FileProgress {
                name, done, total, ..
            } => {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(format!("传输中 {name} {pct}%").into());
                    }
                });
            }
            // ── 文件传输一次性通知 → 状态行 ──
            net::ToUi::FileNotice { text } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(text.into());
                    }
                });
            }
            // ── 传输完成 → 重列对应栏，使取回/下发的文件立即可见 ──
            net::ToUi::PaneRefresh { local } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        // 直填当前路径（resolve_path_arg 对非 <up>/<cd> 串原样列出），重列当前目录。
                        if local {
                            ui.invoke_list_local(ui.get_local_path());
                        } else {
                            ui.invoke_list_remote(ui.get_remote_path());
                        }
                    }
                });
            }
            // ── 即时消息：据当前会话角色渲染到主控聊天页或被控聊天面板 ──
            net::ToUi::ChatIncoming {
                session_id, text, ..
            } => {
                let is_controlling =
                    cur_session.lock().unwrap().as_deref() == Some(session_id.as_str());
                let chat_notice_weak = chat_notice_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        if is_controlling {
                            let log = ui.get_chat_log().to_string();
                            ui.set_chat_log(append_line(&log, "对方", &text).into());
                            if ui.get_active_tab() != 3 {
                                ui.set_chat_unread(true);
                            }
                        } else {
                            let log = ui.get_controlled_chat_log().to_string();
                            ui.set_controlled_chat_log(append_line(&log, "对方", &text).into());
                            if should_show_controlled_chat_notice(ui.get_chat_panel_open()) {
                                ui.set_controlled_chat_unread(true);
                                show_controlled_chat_notice(
                                    &chat_notice_weak,
                                    &ui.get_peer_name().to_string(),
                                    &text,
                                );
                            }
                        }
                    }
                });
            }
            net::ToUi::UpdateAvailable {
                version,
                url,
                notes,
            } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_available(true);
                        ui.set_update_version(version.into());
                        ui.set_update_url(url.into());
                        ui.set_update_notes(notes.unwrap_or_default().into());
                        let _ = ui.show(); // best-effort 置前，避免最小化看不见
                    }
                });
            }
            // 更新状态文本：始终可见的设备卡状态行（检查中/已是最新/下载中/失败）
            net::ToUi::UpdateStatus { text, phase } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_status(text.into());
                        ui.set_update_phase(phase as i32);
                    }
                });
            }
        }
    }
}

fn show_controlled_chat_notice(
    notice_weak: &slint::Weak<ChatNoticeWindow>,
    peer: &str,
    text: &str,
) {
    if crate::chat_notice::auto_dismiss_ms().is_some() {
        return;
    }
    if let Some(notice) = notice_weak.upgrade() {
        notice.set_peer_name(peer.into());
        notice.set_message_text(text.into());
        notice.window().set_size(slint::LogicalSize::new(
            crate::chat_notice::NOTICE_SIZE.width as f32,
            crate::chat_notice::NOTICE_SIZE.height as f32,
        ));
        if let Some(pos) = crate::chat_notice::desktop_bottom_right_position() {
            notice
                .window()
                .set_position(slint::LogicalPosition::new(pos.x as f32, pos.y as f32));
        }
        let _ = notice.show();
    }
}

/// JPEG base64 → 裸 RGBA 字节 + 尺寸（在非 UI 线程解码；Image 在 UI 线程构造，避免跨线程 Send）。
fn decode_frame_rgba(data: &str) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let bytes = STANDARD.decode(data)?;
    let dyn_img = image::load_from_memory(&bytes)?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok((rgba.into_raw(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 已断开会话的迟到帧应被丢弃() {
        // 已断开 sess-1：其迟到帧必须丢弃（否则复活远程态，需点两次断开）。
        let ended = Some("sess-1".to_string());
        assert!(
            frame_belongs_to_ended(&ended, "sess-1"),
            "已断开会话的帧应丢弃"
        );
        // 新会话 sess-2 的帧不受影响，正常渲染。
        assert!(
            !frame_belongs_to_ended(&ended, "sess-2"),
            "其它会话的帧不应丢弃"
        );
        // 无断开标记时一律不丢。
        assert!(!frame_belongs_to_ended(&None, "sess-1"));
    }

    #[test]
    fn next_ctrl_session_门控清理() {
        // 匹配才清：结束的正是当前被控会话 → 清空（与权威 controlled 对齐）。
        assert_eq!(next_ctrl_session_after_end(Some("S1"), "S1"), None);
        // 漂移序列核心：控制 S1 → 重控 S2 → 迟到 SessionEnd{S1} 不该清掉 S2。
        assert_eq!(
            next_ctrl_session_after_end(Some("S2"), "S1"),
            Some("S2".to_string())
        );
        // 本无被控会话：保持 None。
        assert_eq!(next_ctrl_session_after_end(None, "S1"), None);
    }

    #[test]
    fn 登录服务器地址_高级项覆盖默认地址() {
        assert_eq!(
            selected_login_server("wss://rc.guoziweb.com/ws", " ws://172.16.76.1:8765/ws "),
            "ws://172.16.76.1:8765/ws"
        );
        assert_eq!(
            selected_login_server("wss://rc.guoziweb.com/ws", "   "),
            "wss://rc.guoziweb.com/ws"
        );
    }

    #[test]
    fn 被控端新消息_仅面板未打开时触发自绘通知() {
        assert!(should_show_controlled_chat_notice(false));
        assert!(!should_show_controlled_chat_notice(true));
    }

    #[test]
    fn 被控消息通知_自绘常驻并贴工作区右下角() {
        assert_eq!(crate::chat_notice::auto_dismiss_ms(), None);

        let pos = crate::chat_notice::bottom_right_position(
            crate::chat_notice::WorkArea {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            crate::chat_notice::NoticeSize {
                width: 340,
                height: 148,
            },
            18,
        );

        assert_eq!(pos, crate::chat_notice::NoticePosition { x: 1562, y: 874 });
    }
}
