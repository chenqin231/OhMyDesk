//! 采集回调·控制域：授权同意/拒绝、远程发起/断开/取消、被控断开。
use super::UiCtx;
use crate::{history, net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 授权：同意 / 拒绝（用被控会话 id 回传）
    for accept in [true, false] {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.ctrl_session.clone();
        let ui_weak = ui.as_weak();
        let activity = cx.activity.clone();
        let cb = move || {
            if accept && activity.is_updating() {
                return;
            } // 替换窗口内拒绝被控接入
            let sid = sess.lock().unwrap().clone().unwrap_or_default();
            let _ = tx.send(net::FromUi::AuthDecision {
                session_id: sid,
                accept,
            });
            // 本地即时切 UI（回调在 UI 线程，可直接 set）：被控端收不到授权回执，
            // 不本地切则授权框永不消失。同意 → 关框 + 进入"被控中"；拒绝 → 仅关框。
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_auth_pending(false);
                if accept {
                    ui.set_being_controlled(true);
                    ui.set_controlled_forced(false);
                }
            }
        };
        if accept {
            ui.on_auth_accept(cb);
        } else {
            ui.on_auth_reject(cb);
        }
    }
    // 模式 B 发起远控
    {
        let tx = cx.from_ui_tx.clone();
        let ended = cx.ended_session.clone();
        let ui_weak = ui.as_weak();
        let activity = cx.activity.clone();
        ui.on_connect_b(move || {
            // 更新中门控
            if activity.is_updating() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_remote_status("正在更新，请稍后".into());
                }
                return;
            }
            // 新连接意图：清掉「已断开会话」标记，否则若复用同一 id 会话，帧会被误丢。
            *ended.lock().unwrap() = None;
            if let Some(ui) = ui_weak.upgrade() {
                let target = ui.get_target_id().to_string();
                let password = ui.get_target_password().to_string();
                let self_id = ui.get_self_id().to_string();
                if let Err(msg) = validate_remote_target(&target, &self_id) {
                    ui.set_connecting(false); // 校验失败要撤掉 Slint 预置的连接中遮罩
                    ui.set_remote_status(msg.into());
                    return;
                }
                // 清旧错误，连接态走遮罩。
                ui.set_remote_status("".into());
                // 记录最近连接（本地持久化）并刷新列表。
                let list = history::record(&target);
                ui.set_history(super::build_history_model(&list, net::now()));
                // 无密码申请：进等待态（有密码则预期免同意，不显示等待态）
                if password.is_empty() {
                    ui.set_consent_countdown(60);
                    ui.set_awaiting_consent(true);
                }
                activity.begin_pending_connect(crate::update::now_ms());
                let _ = tx.send(net::FromUi::StartRemote {
                    target_id: history::normalize_id(&target),
                    password,
                });
            }
        });
    }
    // 主控断开：发 SessionEnd 给被控 + **本地即时退出远程态**。
    // 关键修复：server 的 SessionEnd 只路由给对端（被控），不回发主控，主控自身收不到
    // SessionEnded；故必须在此本地重置 UI（与授权回调对称），否则点「断开」后主控画面/大窗卡住。
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        let ended = cx.ended_session.clone();
        let ui_weak = ui.as_weak();
        let activity = cx.activity.clone();
        ui.on_disconnect_remote(move || {
            activity.end_pending_connect();
            if let Some(sid) = sess.lock().unwrap().take() {
                // 标记该会话已断开：迟到的在途帧据此被丢弃，不再「复活」远程态（一次点击即真断开）。
                *ended.lock().unwrap() = Some(sid.clone());
                let _ = tx.send(net::FromUi::Disconnect { session_id: sid });
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_remote_active(false);
                ui.set_cursor_ready(false); // 隐藏光标同步叠加层
                ui.set_connecting(false);
                ui.set_remote_status("已断开".into());
                ui.window().set_size(slint::LogicalSize::new(460.0, 620.0));
            }
        });
    }
    // 被控端主动断开：发 SessionEnd 给控制方 + **本地即时撤下被控横幅**。
    // 关键修复（issue#1a）：server 的 SessionEnd 只路由给对端（主控），不回发被控自身，
    // 被控收不到自己发出的结束回执；故必须本地复位，否则点「我要断开」后横幅常驻。
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.ctrl_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_stop_being_controlled(move || {
            if let Some(sid) = sess.lock().unwrap().take() {
                let _ = tx.send(net::FromUi::StopControlled { session_id: sid });
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_being_controlled(false);
                ui.set_controlled_forced(false);
            }
        });
    }
    // 主控取消申请（无密码等待态下取消/超时）：带 target 让 server 撤销被控端弹窗（issue#4）。
    {
        let tx = cx.from_ui_tx.clone();
        let ui_weak = ui.as_weak();
        let activity = cx.activity.clone();
        ui.on_cancel_remote(move || {
            activity.end_pending_connect();
            let target = ui_weak
                .upgrade()
                .map(|ui| history::normalize_id(&ui.get_target_id()))
                .unwrap_or_default();
            let _ = tx.send(net::FromUi::CancelRemote { target });
        });
    }
}

fn validate_remote_target(target: &str, self_id: &str) -> Result<(), &'static str> {
    // self_id 展示为分组形式（带空格），故两侧都去白再比，保证自连守卫不被空格绕过。
    let target = history::normalize_id(target);
    if target.is_empty() {
        return Err("请输入目标 ID");
    }
    let me = history::normalize_id(self_id);
    if !me.is_empty() && target == me {
        return Err("您不能远程自己！");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 远控目标校验_拒绝自连并保留空目标提示() {
        assert_eq!(
            validate_remote_target("", "123456789"),
            Err("请输入目标 ID")
        );
        assert_eq!(
            validate_remote_target("123456789", "123456789"),
            Err("您不能远程自己！")
        );
        assert_eq!(validate_remote_target("987654321", "123456789"), Ok(()));
    }
}
