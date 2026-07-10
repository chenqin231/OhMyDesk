//! 连接/认证类 ToUi 事件处理：Registered、ControlRequest、BeingControlled、RemoteAck、
//! RemoteRejected、Disconnected、AuthExpired。
use crate::{AppWindow, SharedSession};
use slint::ComponentHandle;

pub(super) fn handle_registered(ui_weak: &slint::Weak<AppWindow>, id: String, password: String) {
    let id_disp = super::super::util::group_digits(&id);
    let ui_weak = ui_weak.clone();
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

pub(super) fn handle_control_request(
    ui_weak: &slint::Weak<AppWindow>,
    ctrl_session: &SharedSession,
    requester: String,
    session_id: String,
    source: String,
) {
    // 记下被控会话 id，授权回调据此回传 AuthResult
    *ctrl_session.lock().unwrap() = Some(session_id);
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_auth_requester(requester.into());
            ui.set_auth_source(source.into());
            ui.set_auth_countdown(60);
            ui.set_auth_pending(true);
        }
    });
}

pub(super) fn handle_being_controlled(
    ui_weak: &slint::Weak<AppWindow>,
    ctrl_session: &SharedSession,
    peer_name: String,
    forced: bool,
    session_id: String,
) {
    *ctrl_session.lock().unwrap() = Some(session_id);
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_auth_pending(false);
            ui.set_peer_name(peer_name.into());
            ui.set_controlled_forced(forced);
            ui.set_being_controlled(true);
        }
    });
}

pub(super) fn handle_remote_ack(
    ui_weak: &slint::Weak<AppWindow>,
    cur_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    session_id: String,
) {
    *ended_session.lock().unwrap() = None; // 新会话建立：解除丢帧标记
    *cur_session.lock().unwrap() = Some(session_id);
    activity.end_pending_connect();
    let ui_weak = ui_weak.clone();
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

pub(super) fn handle_remote_rejected(
    ui_weak: &slint::Weak<AppWindow>,
    cur_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    reason: String,
) {
    *cur_session.lock().unwrap() = None;
    activity.end_pending_connect();
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_awaiting_consent(false);
            ui.set_connecting(false);
            ui.set_remote_status(format!("连接失败：{reason}").into());
            ui.set_remote_active(false);
        }
    });
}

pub(super) fn handle_disconnected(ui_weak: &slint::Weak<AppWindow>) {
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_connected(false);
            ui.set_remote_status("与服务器断开，重连中…".into());
        }
    });
}

pub(super) fn handle_auth_expired(
    ui_weak: &slint::Weak<AppWindow>,
    token_tx: &std::sync::Arc<tokio::sync::watch::Sender<Option<String>>>,
) {
    // token 失效/过期（服务端 close 1008）：清凭据 + token 置 None（停重连循环，
    // 否则会拿着过期 token 反复重连被拒），回登录页提示重新登录。
    crate::credential::clear();
    let _ = token_tx.send(None);
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_connected(false);
            ui.set_logged_in(false);
            ui.set_login_pass("".into());
            ui.set_login_error("登录已过期，请重新登录".into());
        }
    });
}
