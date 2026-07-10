//! 采集回调·远程命令。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // ── 远程命令：执行（本地回显命令行，回执到达后追加结果块）──
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
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
}
