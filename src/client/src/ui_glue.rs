//! UI 胶水：Slint 回调注册（UI 线程）+ ToUi 流消费（更新 UI）+ 帧解码。
//!
//! UI 更新一律 `invoke_from_event_loop` + `Weak`（AppWindow 强句柄非 Send）。

use crate::{net, AppWindow, SharedSession};
use slint::ComponentHandle;

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
                if target.is_empty() {
                    ui.set_remote_status("请输入目标 ID".into());
                    return;
                }
                ui.set_remote_status("连接中…".into());
                let _ = tx.send(net::FromUi::StartRemote {
                    target_id: target,
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
            let _ = (x, y); // 移动事件先行，按钮事件不重复带坐标
            if let Some(sid) = sess.lock().unwrap().clone() {
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
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_self_id(id.into());
                        ui.set_self_password(password.into());
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
                        ui.set_remote_status("已连接，等待画面…".into());
                        ui.set_remote_active(true);
                    }
                });
            }
            net::ToUi::RemoteRejected { reason } => {
                *cur_session.lock().unwrap() = None;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
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
                        ui.set_being_controlled(false);
                        ui.set_remote_active(false);
                        ui.set_remote_status("会话已结束".into());
                    }
                });
            }
            net::ToUi::Disconnected => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_remote_status("与服务器断开，重连中…".into());
                    }
                });
            }
        }
    }
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
