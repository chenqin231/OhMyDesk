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
    ended_session: &SharedSession,
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
        let ended = ended_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_connect_b(move || {
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
                ui.set_history(build_history_model(&list, net::now()));
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
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ended = ended_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_disconnect_remote(move || {
            if let Some(sid) = sess.lock().unwrap().take() {
                // 标记该会话已断开：迟到的在途帧据此被丢弃，不再「复活」远程态（一次点击即真断开）。
                *ended.lock().unwrap() = Some(sid.clone());
                let _ = tx.send(net::FromUi::Disconnect { session_id: sid });
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_remote_active(false);
                ui.set_connecting(false);
                ui.set_remote_status("已断开".into());
                ui.window().set_size(slint::LogicalSize::new(460.0, 620.0));
            }
        });
    }
    // 主控切换画质档位（高清优先 / 流畅优先）→ 发 SetQuality 给被控端
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_set_quality(move |high| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let mode = if high {
                    protocol::QualityMode::HighQuality
                } else {
                    protocol::QualityMode::Smooth
                };
                let _ = tx.send(net::FromUi::SetQuality {
                    session_id: sid,
                    mode,
                });
            }
        });
    }
    // 键鼠捕获 → Input（坐标已是帧内像素，Slint 侧换算）
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_on_pointer_move(move |x, y| {
            let sid = sess.lock().unwrap().clone();
            if let Some(sid) = sid {
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
            let sid = sess.lock().unwrap().clone();
            tracing::info!(
                "主控采集·鼠标键 btn={btn} down={down} pos=({x},{y}) session={}",
                sid.as_deref().unwrap_or("<无>")
            );
            if let Some(sid) = sid {
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
            let sid = sess.lock().unwrap().clone();
            tracing::info!(
                "主控采集·键盘 code={code:?} down={down} session={}",
                sid.as_deref().unwrap_or("<无>")
            );
            if let Some(sid) = sid {
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

/// 该帧是否属于「已断开」会话——是则丢弃，不渲染、不复活远程态（修复需点两次断开的 Bug）。
fn frame_belongs_to_ended(ended: &Option<String>, session_id: &str) -> bool {
    ended.as_deref() == Some(session_id)
}

/// 拉 ToUi 流，逐条应用到 UI（invoke_from_event_loop），并维护主控/被控会话 id。
pub async fn consume_to_ui(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<net::ToUi>,
    ui_weak: slint::Weak<AppWindow>,
    cur_session: SharedSession,
    ctrl_session: SharedSession,
    ended_session: SharedSession,
) {
    // 诊断画面发虚：记录主控实际收到的帧分辨率，变化时打印（流畅=1280×720 / 高清=1920×1080 上限）。
    // 据此判断高清是否真生效、被控源分辨率多大。
    let mut last_frame_dims: Option<(u32, u32)> = None;
    while let Some(mut ev) = rx.recv().await {
        // 丢过期帧：收到 Frame 时若通道里还有积压，丢弃当前帧取下一条——只解码/渲染最新帧，
        // 消除「操作后看到一串旧画面」的滞后感（主控渲染慢于被控推帧时积压会堆积）。
        while matches!(ev, net::ToUi::Frame { .. }) {
            match rx.try_recv() {
                Ok(next) => ev = next,
                Err(_) => break,
            }
        }
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
                *ended_session.lock().unwrap() = None; // 新会话建立：解除丢帧标记
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
            net::ToUi::Frame { session_id, data, w, h } => {
                // 丢弃已断开会话的迟到帧：否则在途帧会把已断开的远程态「复活」（需点两次断开）。
                if frame_belongs_to_ended(&ended_session.lock().unwrap(), &session_id) {
                    continue;
                }
                let dims_changed = last_frame_dims != Some((w, h));
                if dims_changed {
                    tracing::info!("主控收到帧分辨率={w}x{h}（流畅档≤1280×720 / 高清档≤1920×1080）");
                    last_frame_dims = Some((w, h));
                }
                // 统一会话态：收到帧即把 cur_session 设为该会话——保证「有画面时输入一定有目标」，
                // 即便 RemoteAck 因时序/路由未设上 cur_session，输入也不会被静默丢弃。
                {
                    let mut s = cur_session.lock().unwrap();
                    if s.as_deref() != Some(session_id.as_str()) {
                        *s = Some(session_id.clone());
                    }
                }
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
                            // 把主控窗口调到接近被控分辨率，让远程桌面尽量 1:1 显示，避免被压进小窗
                            // 强制下采样导致发虚。仅尺寸变化时调整。
                            // DPI 感知：set_size 用逻辑像素，除以主控缩放系数，使窗口的「物理」尺寸≈帧尺寸
                            // （高 DPI 主控上才不会把窗口放大到溢出屏幕）。上限取常见屏物理 1920×1080。
                            if dims_changed {
                                let sf = ui.window().scale_factor().max(1.0);
                                let win_w = (w.min(1920) as f32) / sf;
                                let win_h = (h.min(1080) as f32) / sf;
                                ui.window()
                                    .set_size(slint::LogicalSize::new(win_w, win_h));
                            }
                        }
                    });
                }
            }
            net::ToUi::SessionEnded => {
                // 记下结束的会话 id，丢弃其迟到帧（与本地断开同样防「复活」）。
                let prev = cur_session.lock().unwrap().take();
                if prev.is_some() {
                    *ended_session.lock().unwrap() = prev;
                }
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

    #[test]
    fn 已断开会话的迟到帧应被丢弃() {
        // 已断开 sess-1：其迟到帧必须丢弃（否则复活远程态，需点两次断开）。
        let ended = Some("sess-1".to_string());
        assert!(frame_belongs_to_ended(&ended, "sess-1"), "已断开会话的帧应丢弃");
        // 新会话 sess-2 的帧不受影响，正常渲染。
        assert!(!frame_belongs_to_ended(&ended, "sess-2"), "其它会话的帧不应丢弃");
        // 无断开标记时一律不丢。
        assert!(!frame_belongs_to_ended(&None, "sess-1"));
    }
}
