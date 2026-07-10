//! UI 胶水：Slint 回调注册（UI 线程）+ ToUi 流消费（更新 UI）+ 帧解码。
//!
//! UI 更新一律 `invoke_from_event_loop` + `Weak`（AppWindow 强句柄非 Send）。

use crate::{net, AppWindow, SharedSession};

mod chat;
mod chat_notice;
mod control;
mod exec;
mod files;
mod input;
mod login;
mod misc;
mod restore;
mod ui_update;
mod util;
pub use chat_notice::wire_chat_notice_callbacks;
pub use login::wire_login_callbacks;
pub use restore::wire_repaint_on_restore;
pub use ui_update::consume_to_ui;
use ui_update::{LAST_RES_TIER, REFIT_PENDING};
pub use util::{build_history_model, group_digits};

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
    files::wire(ui, &cx);
    exec::wire(ui, &cx);
    chat::wire(ui, &cx);
    misc::wire(ui, &cx);
}
