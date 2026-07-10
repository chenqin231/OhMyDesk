//! 会话生命周期类 ToUi 事件处理：SessionEnded。
//! 包含帧丢弃检测 / ctrl_session 门控清理两个私有辅助函数。
use crate::{AppWindow, ChatNoticeWindow, SharedSession};
use slint::ComponentHandle;

use super::super::util::build_file_model;

/// 该帧是否属于「已断开」会话——是则丢弃，不渲染、不复活远程态（修复需点两次断开的 Bug）。
pub(super) fn frame_belongs_to_ended(ended: &Option<String>, session_id: &str) -> bool {
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
pub(super) fn next_ctrl_session_after_end(
    current: Option<&str>,
    ending_session_id: &str,
) -> Option<String> {
    if current == Some(ending_session_id) {
        None
    } else {
        current.map(str::to_owned)
    }
}

pub(super) fn handle_session_ended(
    ui_weak: &slint::Weak<AppWindow>,
    chat_notice_weak: &slint::Weak<ChatNoticeWindow>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
    last_frame_dims: &mut Option<(u32, u32)>,
    session_id: String,
) {
    // 重置首帧标志：窗口贴合是「每会话一次」而非「每进程一次」。否则重连新会话时
    // last_frame_dims 仍是上次的 Some(..)，is_first_frame 恒 false → 新会话首帧不再
    // 贴合窗口，卡在上次尺寸。置 None 让下次连接的首帧重新贴合（不重新引入 set_size 风暴：
    // 会话内首帧后 last_frame_dims 即非 None，其余帧仍不触发）。
    *last_frame_dims = None;
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
    let ui_weak = ui_weak.clone();
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
}
