//! 采集回调·文件。
use super::util::{build_file_model, join_path, resolve_path_arg};
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
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
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
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
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
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
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
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
}
