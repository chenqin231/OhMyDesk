//! 聊天通知窗口回调注册。
use crate::{AppWindow, ChatNoticeWindow};
use slint::ComponentHandle;

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
