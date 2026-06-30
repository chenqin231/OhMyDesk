//! UI 胶水：Slint 回调注册（UI 线程）+ ToUi 流消费（更新 UI）+ 帧解码。
//!
//! UI 更新一律 `invoke_from_event_loop` + `Weak`（AppWindow 强句柄非 Send）。

use crate::{history, net, AppWindow, FileEntry, HistoryItem, SharedSession};
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

/// 把 protocol::FileEntry 列表构造为 Slint 列表模型（必须在 UI 线程调用）。
pub fn build_file_model(items: &[protocol::FileEntry]) -> ModelRc<FileEntry> {
    let rows: Vec<FileEntry> = items
        .iter()
        .map(|e| FileEntry {
            name: e.name.clone().into(),
            is_dir: e.is_dir,
            // u64→i32：仅展示用，超 i32 的文件大小极少见，饱和截断不影响功能。
            size: e.size.min(i32::MAX as u64) as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// 解析 Slint 传来的路径指令串 → 目标绝对路径。
/// "<up>:当前路径" → 父目录；"<cd>:当前路径|子名" → 子目录；其余原样（首次/直填路径）。
pub fn resolve_path_arg(arg: &str, cur: &str) -> String {
    if let Some(rest) = arg.strip_prefix("<up>:") {
        parent_of(rest)
    } else if let Some(rest) = arg.strip_prefix("<cd>:") {
        match rest.split_once('|') {
            Some((base, name)) => join_path(base, name),
            None => cur.to_string(),
        }
    } else {
        arg.to_string()
    }
}

/// 父目录：去掉最后一段；到顶（无分隔或仅根）返回空串（被控端空路径=home/盘符列表）。
pub fn parent_of(path: &str) -> String {
    let win = path.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let trimmed = path.trim_end_matches(sep);
    match trimmed.rsplit_once(sep) {
        // head 为空（如 "/home" 的父）→ 空（回根列表）
        Some(("", _)) => String::new(),
        // Windows 盘根 "C:" → 保留 "C:\"（回盘根而非此电脑）
        Some((head, _)) if win && head.ends_with(':') => format!("{head}{sep}"),
        Some((head, _)) => head.to_string(),
        // 无分隔符（如 "C:" 去尾后无 '\'）→ 空（回此电脑）
        None => String::new(),
    }
}

/// 拼接目录 + 子名（按 base 是否含 '\' 选分隔符）。base 为空时返回 name 本身。
pub fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }
    let win = base.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let base = base.trim_end_matches(sep);
    format!("{base}{sep}{name}")
}

/// 聊天记录追加一行（"发送者: 文本"），保持纯文本累积（Slint Text 渲染）。
pub fn append_line(log: &str, who: &str, text: &str) -> String {
    if log.is_empty() {
        format!("{who}: {text}")
    } else {
        format!("{log}\n{who}: {text}")
    }
}

/// 注册全部 UI 回调（运行在 UI 线程，把动作经 from_ui_tx 投给 net）。
pub fn wire_ui_callbacks(
    ui: &AppWindow,
    from_ui_tx: &tokio::sync::mpsc::UnboundedSender<net::FromUi>,
    cur_session: &SharedSession,
    ctrl_session: &SharedSession,
    ended_session: &SharedSession,
    activity: &std::sync::Arc<crate::activity::ClientActivityState>,
) {
    // 授权：同意 / 拒绝（用被控会话 id 回传）
    for accept in [true, false] {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
        let ui_weak = ui.as_weak();
        let activity = activity.clone();
        let cb = move || {
            if accept && activity.is_updating() { return; } // 替换窗口内拒绝被控接入
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
        let tx = from_ui_tx.clone();
        let ended = ended_session.clone();
        let ui_weak = ui.as_weak();
        let activity = activity.clone();
        ui.on_connect_b(move || {
            // 更新中门控
            if activity.is_updating() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_remote_status("正在更新，请稍后".into()); }
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
                ui.set_history(build_history_model(&list, net::now()));
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
    // 复制更新下载链接（URL 原样复制，不过滤空白，避免吃掉 URL 字符）
    {
        ui.on_copy_url(move |s| {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(s.to_string()); // URL 原样复制，不做白空格过滤
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
    // 被控端主动断开：发 SessionEnd 给控制方 + **本地即时撤下被控横幅**。
    // 关键修复（issue#1a）：server 的 SessionEnd 只路由给对端（主控），不回发被控自身，
    // 被控收不到自己发出的结束回执；故必须本地复位，否则点「我要断开」后横幅常驻。
    {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
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
        let tx = from_ui_tx.clone();
        let ui_weak = ui.as_weak();
        ui.on_cancel_remote(move || {
            let target = ui_weak
                .upgrade()
                .map(|ui| history::normalize_id(&ui.get_target_id()))
                .unwrap_or_default();
            let _ = tx.send(net::FromUi::CancelRemote { target });
        });
    }
    // ── tab 切换 → 懒推流：tab 0(远程桌面)发 SetCapture{active:true}，其余 false ──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        ui.on_tab_changed(move |tab| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SetCapture {
                    session_id: sid,
                    active: tab == 0,
                });
            }
        });
    }
    // ── 远程命令：执行（本地回显命令行，回执到达后追加结果块）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
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
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
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
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
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
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
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
    // ── 即时消息：主控发送（本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_chat_log().to_string();
                    ui.set_chat_log(append_line(&log, "我", &text).into());
                }
            }
        });
    }
    // ── 即时消息：被控发送（用被控会话 ctrl_session，本地即时回显「我」）──
    {
        let tx = from_ui_tx.clone();
        let sess = ctrl_session.clone();
        let ui_weak = ui.as_weak();
        ui.on_send_controlled_chat(move |text| {
            let text = text.to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SendChat {
                    session_id: sid,
                    text: text.clone(),
                });
                if let Some(ui) = ui_weak.upgrade() {
                    let log = ui.get_controlled_chat_log().to_string();
                    ui.set_controlled_chat_log(append_line(&log, "我", &text).into());
                }
            }
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
    activity: std::sync::Arc<crate::activity::ClientActivityState>,
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
                source,
            } => {
                // 记下被控会话 id，授权回调据此回传 AuthResult
                *ctrl_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_requester(requester.into());
                        ui.set_auth_source(source.into());
                        ui.set_auth_countdown(60);
                        ui.set_auth_pending(true);
                    }
                });
            }
            net::ToUi::BeingControlled {
                peer_name,
                forced,
                session_id,
            } => {
                *ctrl_session.lock().unwrap() = Some(session_id);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_auth_pending(false);
                        ui.set_peer_name(peer_name.into());
                        ui.set_controlled_forced(forced);
                        ui.set_being_controlled(true);
                    }
                });
            }
            net::ToUi::RemoteAck { session_id } => {
                *ended_session.lock().unwrap() = None; // 新会话建立：解除丢帧标记
                *cur_session.lock().unwrap() = Some(session_id);
                activity.end_pending_connect();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_awaiting_consent(false);
                        ui.set_connecting(false);
                        ui.set_remote_status("".into());
                        ui.set_remote_active(true);
                        // 新会话进入工作台：回到远程桌面标签，同时消除懒推流接缝
                        // （主控落桌面标签、被控进入态默认推流，二者一致）。
                        ui.set_active_tab(0);
                        // 清空各标签会话内残留状态，避免上一会话（目标 X）的数据带入本会话（目标 Y）。
                        // 命令输出 / 聊天记录 / 未读红点 / 文件状态行清空；目录列表由下方 invoke 重新列出。
                        ui.set_cmd_output("".into());
                        ui.set_chat_log("".into());
                        ui.set_chat_unread(false);
                        ui.set_file_notice("".into());
                        // 进入主控画面态：放大窗口给远程桌面腾空间
                        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
                        // 进入工作台：左栏列本机 home、右栏列远端默认目录（空路径=被控 home）。
                        // invoke_<callback> 主动触发已接线的列目录逻辑，不重复写。
                        ui.invoke_list_local("".into());
                        ui.invoke_list_remote("".into());
                    }
                });
            }
            net::ToUi::RemoteRejected { reason } => {
                *cur_session.lock().unwrap() = None;
                activity.end_pending_connect();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_awaiting_consent(false);
                        ui.set_connecting(false);
                        ui.set_remote_status(format!("连接失败：{reason}").into());
                        ui.set_remote_active(false);
                    }
                });
            }
            net::ToUi::Frame {
                session_id,
                data,
                w,
                h,
            } => {
                // 丢弃已断开会话的迟到帧：否则在途帧会把已断开的远程态「复活」（需点两次断开）。
                if frame_belongs_to_ended(&ended_session.lock().unwrap(), &session_id) {
                    continue;
                }
                let dims_changed = last_frame_dims != Some((w, h));
                if dims_changed {
                    tracing::info!(
                        "主控收到帧分辨率={w}x{h}（流畅档≤1280×720 / 高清档≤1920×1080）"
                    );
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
                                ui.window().set_size(slint::LogicalSize::new(win_w, win_h));
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
                // 被控会话结束：清被控会话 id（含主控取消挂起申请的场景）。
                *ctrl_session.lock().unwrap() = None;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let was_controlling = ui.get_remote_active();
                        // 撤销可能仍开着的授权弹窗（主控在被控同意前取消了申请，issue#4）。
                        ui.set_auth_pending(false);
                        ui.set_being_controlled(false);
                        ui.set_connecting(false);
                        ui.set_remote_active(false);
                        ui.set_remote_status("会话已结束".into());
                        // 会话结束清空各标签状态（spec §12）：回到远程桌面标签，下次新会话
                        // active_tab 不残留非 0（与被控进入态默认推流一致，消除懒推流接缝）。
                        ui.set_active_tab(0);
                        // 命令输出 / 主控聊天记录 / 未读红点 清空。
                        ui.set_cmd_output("".into());
                        ui.set_chat_log("".into());
                        ui.set_chat_unread(false);
                        // 被控聊天记录 / 入口红点 / 面板开合 / 右下角弹卡 复位，避免残留。
                        ui.set_controlled_chat_log("".into());
                        ui.set_controlled_chat_unread(false);
                        ui.set_chat_panel_open(false);
                        ui.set_controlled_toast_visible(false);
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
            // ── 远程命令：被控回执 → 累积到命令输出区 ──
            net::ToUi::ExecResult {
                exit_code,
                stdout,
                stderr,
                truncated,
                duration_ms,
                ..
            } => {
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
            // ── 远程文件：远端目录列表 → 右栏渲染 ──
            net::ToUi::RemoteEntries {
                path,
                entries,
                error,
            } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match error {
                            Some(reason) => {
                                ui.set_file_notice(format!("远端目录读取失败：{reason}").into())
                            }
                            None => {
                                ui.set_remote_path(path.into());
                                ui.set_remote_entries(build_file_model(&entries));
                            }
                        }
                    }
                });
            }
            // ── 文件传输进度 → 状态行 ──
            net::ToUi::FileProgress {
                name, done, total, ..
            } => {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(format!("传输中 {name} {pct}%").into());
                    }
                });
            }
            // ── 文件传输一次性通知 → 状态行 ──
            net::ToUi::FileNotice { text } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_file_notice(text.into());
                    }
                });
            }
            // ── 传输完成 → 重列对应栏，使取回/下发的文件立即可见 ──
            net::ToUi::PaneRefresh { local } => {
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
            // ── 即时消息：据当前会话角色渲染到主控聊天页或被控聊天面板 ──
            net::ToUi::ChatIncoming {
                session_id, text, ..
            } => {
                let is_controlling =
                    cur_session.lock().unwrap().as_deref() == Some(session_id.as_str());
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        if is_controlling {
                            let log = ui.get_chat_log().to_string();
                            ui.set_chat_log(append_line(&log, "对方", &text).into());
                            if ui.get_active_tab() != 3 {
                                ui.set_chat_unread(true);
                            }
                        } else {
                            let log = ui.get_controlled_chat_log().to_string();
                            ui.set_controlled_chat_log(append_line(&log, "对方", &text).into());
                            if !ui.get_chat_panel_open() {
                                ui.set_controlled_chat_unread(true);
                                // 右下角弹卡醒目提醒（面板未打开时）：被控用户此前看不到顶部小入口。
                                ui.set_controlled_toast_text(text.into());
                                ui.set_controlled_toast_visible(true);
                                // 被控窗口可能最小化/在后台：尽力取消最小化，让弹卡可见（best-effort）。
                                ui.window().set_minimized(false);
                            }
                        }
                    }
                });
            }
            net::ToUi::UpdateAvailable { version, url, notes } => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_available(true);
                        ui.set_update_version(version.into());
                        ui.set_update_url(url.into());
                        ui.set_update_notes(notes.unwrap_or_default().into());
                        let _ = ui.show(); // best-effort 置前，避免最小化看不见
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
    fn 路径父级_unix与windows() {
        assert_eq!(parent_of("/home/me/docs"), "/home/me");
        assert_eq!(parent_of("/home"), "");
        assert_eq!(parent_of(r"C:\Users\me"), r"C:\Users");
        assert_eq!(parent_of(r"C:\"), ""); // 盘根回此电脑
    }

    #[test]
    fn 路径拼接_按分隔符() {
        assert_eq!(join_path("/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join_path(r"C:\Users", "a.txt"), r"C:\Users\a.txt");
        assert_eq!(join_path("", "a.txt"), "a.txt");
    }

    #[test]
    fn 指令串解析_up与cd() {
        assert_eq!(resolve_path_arg("<up>:/home/me/docs", ""), "/home/me");
        assert_eq!(resolve_path_arg("<cd>:/home/me|docs", ""), "/home/me/docs");
        assert_eq!(resolve_path_arg("/etc", "/home"), "/etc"); // 直填原样
    }

    #[test]
    fn 聊天行追加() {
        assert_eq!(append_line("", "我", "hi"), "我: hi");
        assert_eq!(append_line("我: hi", "对方", "yo"), "我: hi\n对方: yo");
    }
}
