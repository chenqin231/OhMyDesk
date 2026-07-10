//! 聊天/更新类 ToUi 事件处理：ChatIncoming、UpdateAvailable、UpdateStatus。
//! 包含被控端消息通知辅助函数。
use crate::{AppWindow, ChatNoticeWindow, SharedSession};
use slint::ComponentHandle;

use super::super::util::append_line;

fn should_show_controlled_chat_notice(chat_panel_open: bool) -> bool {
    !chat_panel_open
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

pub(super) fn handle_chat_incoming(
    ui_weak: &slint::Weak<AppWindow>,
    chat_notice_weak: &slint::Weak<ChatNoticeWindow>,
    cur_session: &SharedSession,
    session_id: String,
    text: String,
) {
    let is_controlling =
        cur_session.lock().unwrap().as_deref() == Some(session_id.as_str());
    let chat_notice_weak = chat_notice_weak.clone();
    let ui_weak = ui_weak.clone();
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

pub(super) fn handle_update_available(
    ui_weak: &slint::Weak<AppWindow>,
    version: String,
    url: String,
    notes: Option<String>,
) {
    let ui_weak = ui_weak.clone();
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

pub(super) fn handle_update_status(
    ui_weak: &slint::Weak<AppWindow>,
    text: String,
    phase: u8,
) {
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_update_status(text.into());
            ui.set_update_phase(phase as i32);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
