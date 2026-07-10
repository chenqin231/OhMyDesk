//! ToUi 流消费：拉网络事件逐条应用到 UI（invoke_from_event_loop），维护会话 id、帧解码。
use crate::{net, AppWindow, ChatNoticeWindow, SharedSession};
use std::sync::atomic::{AtomicBool, AtomicI32};

mod chat_update;
mod conn;
mod frame;
mod session;
mod transfer;

/// 用户点「分辨率」档 → 置位；主控 Frame 处理在下一次帧尺寸变化时据此把窗口一次性重贴合到
/// 被控分辨率（用户触发=单次 set_size，不受 adaptive 抖动影响，避开首帧后「不再动窗口」的风暴）。
pub(super) static REFIT_PENDING: AtomicBool = AtomicBool::new(false);
/// 上次分辨率档位（仅分辨率轴变化才触发窗口重贴合；清晰度/帧率不改帧尺寸，不该动窗口）。
pub(super) static LAST_RES_TIER: AtomicI32 = AtomicI32::new(-1);

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
                conn::handle_registered(&ui_weak, id, password);
            }
            net::ToUi::ControlRequest {
                requester,
                session_id,
                source,
            } => {
                conn::handle_control_request(
                    &ui_weak,
                    &ctrl_session,
                    requester,
                    session_id,
                    source,
                );
            }
            net::ToUi::BeingControlled {
                peer_name,
                forced,
                session_id,
            } => {
                conn::handle_being_controlled(
                    &ui_weak,
                    &ctrl_session,
                    peer_name,
                    forced,
                    session_id,
                );
            }
            net::ToUi::RemoteAck { session_id } => {
                conn::handle_remote_ack(
                    &ui_weak,
                    &cur_session,
                    &ended_session,
                    &activity,
                    session_id,
                );
            }
            net::ToUi::RemoteRejected { reason } => {
                conn::handle_remote_rejected(&ui_weak, &cur_session, &activity, reason);
            }
            net::ToUi::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            } => {
                frame::handle_frame(
                    &ui_weak,
                    &cur_session,
                    &ended_session,
                    &mut last_frame_dims,
                    &mut recv_stats,
                    session_id,
                    data,
                    w,
                    h,
                    seq,
                );
            }
            net::ToUi::Cursor { visible, shape } => {
                frame::handle_cursor(&ui_weak, visible, shape);
            }
            net::ToUi::SessionEnded { session_id } => {
                session::handle_session_ended(
                    &ui_weak,
                    &chat_notice_weak,
                    &cur_session,
                    &ctrl_session,
                    &ended_session,
                    &activity,
                    &mut last_frame_dims,
                    session_id,
                );
            }
            net::ToUi::Disconnected => {
                conn::handle_disconnected(&ui_weak);
            }
            net::ToUi::AuthExpired => {
                conn::handle_auth_expired(&ui_weak, &token_tx);
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
                transfer::handle_exec_result(
                    &ui_weak,
                    exit_code,
                    stdout,
                    stderr,
                    truncated,
                    duration_ms,
                );
            }
            // ── 远程文件：远端目录列表 → 右栏渲染 ──
            net::ToUi::RemoteEntries {
                path,
                entries,
                error,
            } => {
                transfer::handle_remote_entries(&ui_weak, path, entries, error);
            }
            // ── 文件传输进度 → 状态行 ──
            net::ToUi::FileProgress {
                name, done, total, ..
            } => {
                transfer::handle_file_progress(&ui_weak, name, done, total);
            }
            // ── 文件传输一次性通知 → 状态行 ──
            net::ToUi::FileNotice { text } => {
                transfer::handle_file_notice(&ui_weak, text);
            }
            // ── 传输完成 → 重列对应栏，使取回/下发的文件立即可见 ──
            net::ToUi::PaneRefresh { local } => {
                transfer::handle_pane_refresh(&ui_weak, local);
            }
            // ── 即时消息：据当前会话角色渲染到主控聊天页或被控聊天面板 ──
            net::ToUi::ChatIncoming {
                session_id, text, ..
            } => {
                chat_update::handle_chat_incoming(
                    &ui_weak,
                    &chat_notice_weak,
                    &cur_session,
                    session_id,
                    text,
                );
            }
            net::ToUi::UpdateAvailable {
                version,
                url,
                notes,
            } => {
                chat_update::handle_update_available(&ui_weak, version, url, notes);
            }
            // 更新状态文本：始终可见的设备卡状态行（检查中/已是最新/下载中/失败）
            net::ToUi::UpdateStatus { text, phase } => {
                chat_update::handle_update_status(&ui_weak, text, phase);
            }
        }
    }
}
