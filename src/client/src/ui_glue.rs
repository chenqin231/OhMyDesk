//! UI 胶水：Slint 回调注册（UI 线程）+ ToUi 流消费（更新 UI）+ 帧解码。
//!
//! UI 更新一律 `invoke_from_event_loop` + `Weak`（AppWindow 强句柄非 Send）。

use crate::{history, net, AppWindow, HistoryItem, SharedSession};
use slint::{ComponentHandle, ModelRc, VecModel};

/// 把 9 位 id 按 3-3-3 分组展示（"617343065" → "617 343 065"）。复制时 Rust 侧再去空白。
pub fn group_digits(id: &str) -> String {
    let digits: String = id.chars().filter(|c| !c.is_whitespace()).collect();
    digits
        .as_bytes()
        .chunks(3)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 相对时间（毫秒）：刚刚 / N 分钟前 / N 小时前 / N 天前。
pub fn rel_time(ts_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - ts_ms).max(0) / 1000;
    if secs < 60 {
        "刚刚".into()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86_400)
    }
}

/// 把历史记录构造为 Slint 列表模型（必须在 UI 线程调用：VecModel 非 Send）。
pub fn build_history_model(items: &[history::RecentConn], now_ms: i64) -> ModelRc<HistoryItem> {
    let rows: Vec<HistoryItem> = items
        .iter()
        .map(|c| HistoryItem {
            raw_id: c.id.clone().into(),
            label: group_digits(&c.id).into(),
            sub: rel_time(c.ts, now_ms).into(),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// 注册全部 UI 回调（运行在 UI 线程，把动作经 from_ui_tx 投给 net）。
pub fn wire_ui_callbacks(
    ui: &AppWindow,
    from_ui_tx: &tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
) {
    // 授权：同意 / 拒绝（用被控会话 id 回传）
    for accept in [true, false] {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
        let ui_weak = ui.as_weak();
        let cb = move || {
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
        let tx = from_ui_tx.clone();
        let ui_weak = ui.as_weak();
        ui.on_connect_b(move || {
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
                ui.set_history(build_history_model(&list, net::now()));
                let _ = tx.send(net::FromUi::StartRemote {
                    target_id: history::normalize_id(&target),
                    password,
                });
            }
        });
    }
    // 主控断开
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_disconnect_remote(move || {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::Disconnect { session_id: sid });
            }
        });
    }
    // 键鼠捕获 → Input（坐标已是帧内像素，Slint 侧换算）
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_on_pointer_move(move |x, y| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::MouseMove { x, y },
                });
            }
        });
    }
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_on_pointer_button(move |x, y, btn, down| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid.clone(),
                    event: protocol::InputEvent::MouseMove { x, y },
                });
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::MouseButton {
                        button: btn as u8,
                        down,
                    },
                });
            }
        });
    }
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_on_key(move |code, down| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::Key {
                        code: code.to_string(),
                        down,
                    },
                });
            }
        });
    }
    // 复制 ID/密码到剪贴板（ID 分组带空格，先去白）
    {
        ui.on_copy_text(move |s| {
            let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(cleaned);
            }
        });
    }
    // 刷新临时密码（重发 Register，server upsert 覆盖；新密码经 Registered 回推展示）
    {
        let tx = from_ui_tx.clone();
        ui.on_refresh_password(move || {
            let _ = tx.send(net::FromUi::RefreshPassword);
        });
    }
}

/// 拉 ToUi 流，逐条应用到 UI（invoke_from_event_loop），并维护主控/被控会话 id。
pub async fn consume_to_ui(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<net::ToUi>,
    ui_weak: slint::Weak<AppWindow>,
    cur_session: SharedSession,
    ctrl_session: SharedSession,
) {
    while let Some(ev) = rx.recv().await {
        let ui_weak = ui_weak.clone();
        match ev {
            net::ToUi::Registered { id, password } => {
                let id_disp = group_digits(&id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_self_id(id_disp.into());
                        ui.set_self_password(password.into());
                        ui.set_connected(true);
                    }
                });
            }
            net::ToUi::ControlRequest {
                requester,
                session_id,
            } => {
                // 记下被控会话 id，授权回调据此回传 AuthResult
                *ctrl_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_requester(requester.into());
                        ui.set_auth_pending(true);
                    }
                });
            }
            net::ToUi::BeingControlled { peer_name } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_pending(false);
                        ui.set_peer_name(peer_name.into());
                        ui.set_being_controlled(true);
                    }
                });
            }
            net::ToUi::RemoteAck { session_id } => {
                *cur_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_connecting(false);
                        ui.set_remote_status("".into());
                        ui.set_remote_active(true);
                        // 进入主控画面态：放大窗口给远程桌面腾空间
                        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
                    }
                });
            }
            net::ToUi::RemoteRejected { reason } => {
                *cur_session.lock().unwrap() = None;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_connecting(false);
                        ui.set_remote_status(format!("连接失败：{reason}").into());
                        ui.set_remote_active(false);
                    }
                });
            }
            net::ToUi::Frame { data, w, h } => {
                // 在本（tokio）线程解码 JPEG→RGBA（产出 Vec<u8> 是 Send）；Image 非 Send，
                // 故只把裸 RGBA + 尺寸传进闭包，在 UI 线程内构造 Image（slint.md §3 坑 2）。
                if let Ok((rgba, iw, ih)) = decode_frame_rgba(&data) {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let mut buffer =
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(iw, ih);
                            buffer.make_mut_bytes().copy_from_slice(&rgba);
                            ui.set_frame_w(w as i32);
                            ui.set_frame_h(h as i32);
                            ui.set_frame(slint::Image::from_rgba8(buffer));
                            ui.set_remote_active(true);
                        }
                    });
                }
            }
            net::ToUi::SessionEnded => {
                *cur_session.lock().unwrap() = None;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let was_controlling = ui.get_remote_active();
                        ui.set_being_controlled(false);
                        ui.set_connecting(false);
                        ui.set_remote_active(false);
                        ui.set_remote_status("会话已结束".into());
                        // 退出主控画面态：缩回紧凑小窗
                        if was_controlling {
                            ui.window().set_size(slint::LogicalSize::new(460.0, 620.0));
                        }
                    }
                });
            }
            net::ToUi::Disconnected => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_connected(false);
                        ui.set_remote_status("与服务器断开，重连中…".into());
                    }
                });
            }
        }
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

/// JPEG base64 → 裸 RGBA 字节 + 尺寸（在非 UI 线程解码；Image 在 UI 线程构造，避免跨线程 Send）。
fn decode_frame_rgba(data: &str) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let bytes = STANDARD.decode(data)?;
    let dyn_img = image::load_from_memory(&bytes)?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok((rgba.into_raw(), w, h))
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
