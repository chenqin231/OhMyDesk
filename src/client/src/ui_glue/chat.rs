//! 采集回调·聊天。
use super::util::append_line;
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // ── 即时消息：主控发送（本地即时回显「我」）──
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
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
        let tx = cx.from_ui_tx.clone();
        let sess = cx.ctrl_session.clone();
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
}
