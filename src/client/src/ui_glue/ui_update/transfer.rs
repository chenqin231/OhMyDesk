//! 文件传输/命令类 ToUi 事件处理：ExecResult、RemoteEntries、FileProgress、FileNotice、PaneRefresh。
use crate::AppWindow;

use super::super::util::build_file_model;

pub(super) fn handle_exec_result(
    ui_weak: &slint::Weak<AppWindow>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
    duration_ms: u32,
) {
    let code = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "无(超时/未启动)".into());
    let mut block = format!("退出码 {code} · 耗时 {duration_ms}ms");
    if !stdout.is_empty() {
        block.push_str(&format!("\n{stdout}"));
    }
    if !stderr.is_empty() {
        block.push_str(&format!("\n[stderr] {stderr}"));
    }
    if truncated {
        block.push_str("\n[输出已截断]");
    }
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            // 结果块紧跟其上方刚回显的「$ 命令」行（单换行），不同命令间已由回显侧空行分隔。
            let prev = ui.get_cmd_output().to_string();
            let next = if prev.is_empty() {
                block
            } else {
                format!("{prev}\n{block}")
            };
            ui.set_cmd_output(next.into());
        }
    });
}

pub(super) fn handle_remote_entries(
    ui_weak: &slint::Weak<AppWindow>,
    path: String,
    entries: Vec<protocol::FileEntry>,
    error: Option<String>,
) {
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            match error {
                Some(reason) => ui.set_file_notice(format!("远端目录读取失败：{reason}").into()),
                None => {
                    ui.set_remote_path(path.into());
                    ui.set_remote_entries(build_file_model(&entries));
                }
            }
        }
    });
}

pub(super) fn handle_file_progress(
    ui_weak: &slint::Weak<AppWindow>,
    name: String,
    done: u64,
    total: u64,
) {
    let pct = (done * 100).checked_div(total).unwrap_or(0);
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_notice(format!("传输中 {name} {pct}%").into());
        }
    });
}

pub(super) fn handle_file_notice(ui_weak: &slint::Weak<AppWindow>, text: String) {
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_notice(text.into());
        }
    });
}

pub(super) fn handle_pane_refresh(ui_weak: &slint::Weak<AppWindow>, local: bool) {
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            // 直填当前路径（resolve_path_arg 对非 <up>/<cd> 串原样列出），重列当前目录。
            if local {
                ui.invoke_list_local(ui.get_local_path());
            } else {
                ui.invoke_list_remote(ui.get_remote_path());
            }
        }
    });
}
