//! 下行/上行消息分发：下行解析 Envelope → 通知 UI / 旁路执行；上行动作 → 出站。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use protocol::{Envelope, Message};
use tokio::sync::mpsc;

use super::conn::SessionCtx;
use super::{
    now, CaptureCtrl, ClipboardMsg, FromUi, ToUi, CAPTURE_CTRL, CLIPBOARD_TX, INJECT_TX,
    SCREENSHOT_TX,
};

/// 进程内自增 id 计数器（exec_id/transfer_id/msg_id 用，禁随机/时间以保证可复现）。
static SEQ: AtomicU64 = AtomicU64::new(1);

/// 生成下一个带前缀的进程内唯一 id（如 "exec-12"）。
fn next_id(prefix: &str) -> String {
    format!("{prefix}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// 给控制方回一条 `FileError`（被控端拒收/写盘失败时）。
fn send_file_error(
    out_tx: &mpsc::UnboundedSender<String>,
    self_id: &str,
    session_id: String,
    transfer_id: String,
    reason: String,
) {
    let env = Envelope {
        from: self_id.to_string(),
        to: None,
        ts: now(),
        payload: Message::FileError {
            session_id,
            transfer_id,
            reason,
        },
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
    }
}

/// 给控制方回一条 `FileDone`（被控端收齐 push 文件并落盘后，告知最终绝对路径）。
fn send_file_done(
    out_tx: &mpsc::UnboundedSender<String>,
    self_id: &str,
    session_id: String,
    transfer_id: String,
    path: String,
) {
    let env = Envelope {
        from: self_id.to_string(),
        to: None,
        ts: now(),
        payload: Message::FileDone {
            session_id,
            transfer_id,
            path,
        },
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
    }
}

/// 处理一条下行消息。
pub(super) async fn handle_downlink(
    text: &str,
    self_id: &str,
    out_tx: &mpsc::UnboundedSender<String>,
    to_ui: &mpsc::UnboundedSender<ToUi>,
    session: &Arc<tokio::sync::Mutex<SessionCtx>>,
) -> anyhow::Result<()> {
    let env: Envelope = serde_json::from_str(text)?;
    match env.payload {
        // 被控端收 server 转发的来控通知。
        // auto_accept=true:免同意(密码对/强制)→ 不弹框,直接进被控态(复用同意副作用);
        // auto_accept=false:弹授权框,等用户同意。
        Message::IncomingControl {
            session_id,
            from,
            operator_username,
            mode,
            auto_accept,
        } => {
            let peer_name = control_peer_name(operator_username.as_deref(), &from);
            if auto_accept {
                {
                    let mut ctx = session.lock().await;
                    ctx.controlled = Some(session_id.clone());
                    ctx.controlled_peer_name = Some(peer_name.clone());
                }
                CAPTURE_CTRL.send(CaptureCtrl::Start {
                    session_id: session_id.clone(),
                });
                CLIPBOARD_TX.send(ClipboardMsg::Start {
                    session_id: session_id.clone(),
                });
                let _ = to_ui.send(ToUi::BeingControlled {
                    peer_name,
                    forced: mode == protocol::Mode::A,
                    session_id: session_id.clone(),
                });
            } else {
                session.lock().await.controlled_peer_name = Some(peer_name.clone());
                let _ = to_ui.send(ToUi::ControlRequest {
                    requester: peer_name,
                    session_id,
                    source: control_source(mode).to_string(),
                });
            }
        }
        // 鉴权结果（server 下发）：被控端据此进入被控态并回 ConnectAck 由 server 处理
        Message::AuthResult {
            session_id,
            ok,
            reason,
        } => {
            if ok {
                let peer_name = {
                    let mut ctx = session.lock().await;
                    ctx.controlled = Some(session_id.clone());
                    ctx.controlled_peer_name
                        .clone()
                        .unwrap_or_else(|| "远程方".into())
                };
                // 进入被控态：启动 2-3fps 截屏推帧（main 截屏线程消费此信号）
                CAPTURE_CTRL.send(CaptureCtrl::Start {
                    session_id: session_id.clone(),
                });
                CLIPBOARD_TX.send(ClipboardMsg::Start {
                    session_id: session_id.clone(),
                });
                let _ = to_ui.send(ToUi::BeingControlled {
                    peer_name,
                    forced: false,
                    session_id: session_id.clone(),
                });
            } else {
                let _ = to_ui.send(ToUi::RemoteRejected {
                    reason: reason.unwrap_or_else(|| "鉴权失败".into()),
                });
            }
        }
        // 主控端收到 ack：进入主控态（若已取消则收尾 server 会话，不进主控态）
        Message::ConnectAck { session_id } => {
            let mut ctx = session.lock().await;
            if ctx.initiate_cancelled {
                // 申请已被主控取消/超时 → 收尾 server 会话，不进主控态。
                ctx.initiate_cancelled = false;
                drop(ctx);
                let env = Envelope {
                    from: self_id.to_string(),
                    to: None,
                    ts: now(),
                    payload: Message::SessionEnd { session_id },
                };
                if let Ok(json) = serde_json::to_string(&env) {
                    let _ = out_tx.send(json);
                }
                return Ok(());
            }
            ctx.controlling = Some(session_id.clone());
            drop(ctx);
            CLIPBOARD_TX.send(ClipboardMsg::Start {
                session_id: session_id.clone(),
            });
            let _ = to_ui.send(ToUi::RemoteAck { session_id });
        }
        // 主控端收到拒绝
        Message::Reject { reason, .. } => {
            let _ = to_ui.send(ToUi::RemoteRejected { reason });
        }
        // 主控端收到画面帧 → 通知 UI 贴帧（带 session_id 供 UI 统一会话态）
        Message::Frame {
            session_id,
            data,
            w,
            h,
            seq,
        } => {
            let _ = to_ui.send(ToUi::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            });
        }
        // 主控端收到被控端会话内提示（如 Wayland 无法截屏）→ 复用拒绝态 UI 展示原因
        Message::RemoteNotice { text, .. } => {
            let _ = to_ui.send(ToUi::RemoteRejected { reason: text });
        }
        // 被控端收到键鼠 → 经旁路交 main 注入侧（注入依赖 X11，不在 net 任务里执行）
        Message::Input { session_id, event } => {
            let ctx = session.lock().await;
            let matched = ctx.controlled.as_deref() == Some(session_id.as_str());
            if matched {
                drop(ctx);
                let _ = out_tx; // Input 不回发，交注入侧
                                // 诊断键盘问题（组合键/上档符）：记录被控实际收到的键事件原文。
                if let protocol::InputEvent::Key { code, down } = &event {
                    tracing::info!("被控收到按键 code={code:?} down={down}");
                }
                INJECT_TX.with_send(session_id, event);
            }
        }
        // 被控端收主控切换的画质档位 → 更新采集参数（仅本会话被控态时生效）
        Message::SetQuality {
            session_id, mode, ..
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            tracing::info!(
                "被控收到画质切换 mode={mode:?} controlled={controlled} session={session_id}"
            );
            if controlled {
                crate::capture::set_quality(mode);
                // 手动切档立即重置自适应降档，让用户选择先生效（弱机上避免被 adaptive 立即拉回）。
                crate::adaptive::request_reset();
                let p = crate::capture::current_params();
                tracing::info!(
                    "被控已应用画质 上限={}x{} q={} 间隔={}ms",
                    p.max_w,
                    p.max_h,
                    p.jpeg_q,
                    p.interval_ms
                );
            }
        }
        // 被控端收主控懒推流开关 → 据 active 启停推帧（仅启停采集，不动 controlled 态）
        Message::SetCapture { session_id, active } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
                if active {
                    CAPTURE_CTRL.send(CaptureCtrl::Start {
                        session_id: session_id.clone(),
                    });
                } else {
                    CAPTURE_CTRL.send(CaptureCtrl::Stop);
                }
                tracing::info!("被控应用懒推流开关 active={active} session={session_id}");
            }
        }
        // 截图请求：被控端截一帧回 ScreenshotResp（Phase 5，主控/被控共用截屏能力）
        Message::ScreenshotReq { req_id } => {
            SCREENSHOT_TX.with_send(req_id, env.from);
        }
        Message::SessionEnd { session_id } => {
            let mut ctx = session.lock().await;
            if ctx.controlling.as_deref() == Some(session_id.as_str()) {
                ctx.controlling = None;
            }
            if ctx.controlled.as_deref() == Some(session_id.as_str()) {
                ctx.controlled = None;
                ctx.controlled_peer_name = None;
                CAPTURE_CTRL.send(CaptureCtrl::Stop); // 停被控端推帧
            }
            CLIPBOARD_TX.send(ClipboardMsg::Stop);
            crate::transfer::clear_pull_targets(); // 清理本会话残留的取回目标登记
                                                   // 携结束的 session_id 上抛，UI 侧据此门控清理被控会话副本（对齐上方 controlled 的按 id 清理）。
            let _ = to_ui.send(ToUi::SessionEnded { session_id });
        }

        // ── 主控端：收被控回执的命令执行结果 → 投 UI 渲染 ─────────────────────
        Message::ExecResult {
            session_id,
            exec_id,
            exit_code,
            stdout,
            stderr,
            truncated,
            duration_ms,
        } => {
            // 门控：仅当本端确为该会话的主控方才投 UI（与文件分支防御风格一致）。
            let controlling =
                session.lock().await.controlling.as_deref() == Some(session_id.as_str());
            if controlling {
                let _ = to_ui.send(ToUi::ExecResult {
                    exec_id,
                    exit_code,
                    stdout,
                    stderr,
                    truncated,
                    duration_ms,
                });
            }
        }
        // ── 主控端：收被控回执的远端目录列表 → 投 UI（右栏渲染）──────────────────
        Message::FileListResp {
            session_id,
            path,
            entries,
            error,
            ..
        } => {
            // 门控：仅主控方投 UI（与文件分支防御风格一致）。
            let controlling =
                session.lock().await.controlling.as_deref() == Some(session_id.as_str());
            if controlling {
                let _ = to_ui.send(ToUi::RemoteEntries {
                    path,
                    entries,
                    error,
                });
            }
        }
        // ── 主控端：收对端即时消息 → 投 UI（即时消息标签 / 被控聊天面板）──────────
        Message::ChatMessage {
            session_id,
            msg_id,
            text,
        } => {
            let _ = to_ui.send(ToUi::ChatIncoming {
                session_id,
                msg_id,
                text,
            });
        }

        // ── 被控端：收控制方下发的一次性命令 → 执行 → 回 ExecResult ───────────
        Message::ExecRequest {
            session_id,
            exec_id,
            command,
            timeout_ms,
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
                let out = out_tx.clone();
                let from = self_id.to_string();
                tokio::spawn(async move {
                    let r = crate::exec::run_command(&command, timeout_ms).await;
                    let env = Envelope {
                        from,
                        to: None,
                        ts: now(),
                        payload: Message::ExecResult {
                            session_id,
                            exec_id,
                            exit_code: r.exit_code,
                            stdout: r.stdout,
                            stderr: r.stderr,
                            truncated: r.truncated,
                            duration_ms: r.duration_ms,
                        },
                    };
                    if let Ok(s) = serde_json::to_string(&env) {
                        let _ = out.send(s);
                    }
                });
            }
        }

        // ── FileOpen：被控态收 push 首包(落盘准备) / 主控态收 pull 回流首包(本地落盘准备)──
        Message::FileOpen {
            session_id,
            transfer_id,
            name,
            size,
            dir,
            dest,
        } => {
            let ctx = session.lock().await;
            let controlled = ctx.controlled.as_deref() == Some(session_id.as_str());
            let controlling = ctx.controlling.as_deref() == Some(session_id.as_str());
            drop(ctx);
            if controlled && dir == protocol::FileDir::Push {
                // 被控端：push 下发首包 → 打开接收文件（失败回 FileError）
                if let Err(reason) =
                    crate::transfer::open_recv(&transfer_id, &name, size, dest.as_deref())
                {
                    send_file_error(out_tx, self_id, session_id, transfer_id, reason);
                }
            } else if controlling && dir == protocol::FileDir::Pull {
                // 主控端：pull 回流首包 → peek 本地保存目录（不消费，文件夹多文件复用同一登记），
                // 打开本地接收文件。登记在会话结束时由 clear_pull_targets 统一清理。
                let local_dir = crate::transfer::peek_pull_target(&transfer_id)
                    .map(|p| p.to_string_lossy().to_string());
                match crate::transfer::open_recv(&transfer_id, &name, size, local_dir.as_deref()) {
                    Ok(_) => {
                        let _ = to_ui.send(ToUi::FileProgress {
                            transfer_id: transfer_id.clone(),
                            name: name.clone(),
                            done: 0,
                            total: size,
                        });
                    }
                    Err(reason) => {
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("取回 {name} 失败：{reason}"),
                        });
                    }
                }
            }
        }

        // ── FileChunk：被控态收 push 块(落盘) / 主控态收 pull 回流块(本地落盘)──────
        Message::FileChunk {
            session_id,
            transfer_id,
            data,
            last,
            ..
        } => {
            let ctx = session.lock().await;
            let controlled = ctx.controlled.as_deref() == Some(session_id.as_str());
            let controlling = ctx.controlling.as_deref() == Some(session_id.as_str());
            drop(ctx);
            if controlled {
                // 被控端：push 块落盘，末块回 FileDone(带最终路径)，失败回 FileError
                match crate::transfer::write_chunk(&transfer_id, &data, last) {
                    Ok(Some(path)) => send_file_done(
                        out_tx,
                        self_id,
                        session_id,
                        transfer_id,
                        path.to_string_lossy().to_string(),
                    ),
                    Ok(None) => {}
                    Err(reason) => {
                        send_file_error(out_tx, self_id, session_id, transfer_id, reason)
                    }
                }
            } else if controlling {
                // 主控端：pull 回流块落盘到本机；末块完成 → 投 FileNotice 告知本机最终路径
                match crate::transfer::write_chunk(&transfer_id, &data, last) {
                    Ok(Some(path)) => {
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("已取回到本机：{}", path.to_string_lossy()),
                        });
                        // 取回完成 → 刷新左栏，让取回的文件/文件夹立即可见。
                        let _ = to_ui.send(ToUi::PaneRefresh { local: true });
                    }
                    Ok(None) => {}
                    Err(reason) => {
                        crate::transfer::abort(&transfer_id);
                        let _ = to_ui.send(ToUi::FileNotice {
                            text: format!("取回失败：{reason}"),
                        });
                    }
                }
            }
        }

        // ── 被控端：收取回请求 → 读文件分块回流（独立任务）─────────────────────
        Message::FilePullRequest {
            session_id,
            transfer_id,
            path,
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
                tokio::spawn(crate::transfer::send_file(
                    out_tx.clone(),
                    self_id.to_string(),
                    session_id,
                    transfer_id,
                    path,
                ));
            }
        }

        // ── 被控端：收远端目录浏览请求 → 列目录回 FileListResp（独立任务，IO 不阻塞分发）──
        Message::FileListRequest {
            session_id,
            transfer_id,
            path,
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
                let out = out_tx.clone();
                let from = self_id.to_string();
                tokio::spawn(async move {
                    // 列目录是阻塞文件 IO（read_dir/metadata/canonicalize），放 spawn_blocking 执行，
                    // 不占用 async 工作线程——否则大目录会卡住同线程的出站泵/心跳，拖慢整体响应。
                    let listed = {
                        let path = path.clone();
                        tokio::task::spawn_blocking(move || crate::transfer::list_dir(&path)).await
                    };
                    let payload = match listed {
                        Ok(Ok((dir, entries))) => Message::FileListResp {
                            session_id,
                            transfer_id,
                            path: dir,
                            entries,
                            error: None,
                        },
                        Ok(Err(reason)) => Message::FileListResp {
                            session_id,
                            transfer_id,
                            path,
                            entries: Vec::new(),
                            error: Some(reason),
                        },
                        Err(join_err) => Message::FileListResp {
                            session_id,
                            transfer_id,
                            path,
                            entries: Vec::new(),
                            error: Some(format!("列目录任务失败: {join_err}")),
                        },
                    };
                    let env = Envelope {
                        from,
                        to: None,
                        ts: now(),
                        payload,
                    };
                    if let Ok(s) = serde_json::to_string(&env) {
                        let _ = out.send(s);
                    }
                });
            }
        }

        // ── 主控端：收被控回执的下发完成（push）→ 投 UI 告知远端最终路径 ──────────
        Message::FileDone { path, .. } => {
            let _ = to_ui.send(ToUi::FileNotice {
                text: format!("已下发到远端：{path}"),
            });
            // 下发完成 → 刷新右栏，让下发到远端的文件/文件夹立即可见。
            let _ = to_ui.send(ToUi::PaneRefresh { local: false });
        }
        // ── 传输失败：清理在途接收 + 通知 UI ───────────────────────────────────
        Message::FileError {
            transfer_id,
            reason,
            ..
        } => {
            crate::transfer::abort(&transfer_id);
            let _ = to_ui.send(ToUi::FileNotice {
                text: format!("传输失败：{reason}"),
            });
        }
        // 被控/主控收对端剪贴板 → 校验方向后交 worker 写本地(防回环由 worker 的 last_synced 处理)。
        Message::ClipboardSync { session_id, text } => {
            let ctx = session.lock().await;
            let in_session = ctx.controlling.as_deref() == Some(session_id.as_str())
                || ctx.controlled.as_deref() == Some(session_id.as_str());
            drop(ctx);
            if in_session {
                CLIPBOARD_TX.send(ClipboardMsg::Incoming { text });
            }
        }
        _ => {}
    }
    Ok(())
}

fn control_peer_name(operator_username: Option<&str>, fallback_conn_id: &str) -> String {
    operator_username
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_conn_id)
        .to_string()
}

/// 来控来源中文标签(管理端 mode A / 终端伙伴 mode B),用于被控端授权弹窗展示。
pub(super) fn control_source(mode: protocol::Mode) -> &'static str {
    match mode {
        protocol::Mode::A => "管理员",
        protocol::Mode::B => "终端伙伴",
    }
}

/// 伙伴密码归一:空串视为「未填」→ None(选填语义,server 据此走同意流程)。
pub(super) fn opt_password(p: String) -> Option<String> {
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

/// 处理一条 UI 上行动作 → 出站。
pub(super) async fn handle_uplink(
    act: FromUi,
    self_id: &str,
    out_tx: &mpsc::UnboundedSender<String>,
    session: &Arc<tokio::sync::Mutex<SessionCtx>>,
) {
    let env = match act {
        // RefreshPassword 在 connect_once 的 select 处已拦截重注册，不进入本分发；
        // 此臂仅为穷尽匹配，理论不可达。
        FromUi::RefreshPassword => return,
        FromUi::CancelRemote { target } => {
            {
                let mut ctx = session.lock().await;
                if ctx.controlling.is_none() {
                    ctx.initiate_cancelled = true;
                }
            }
            // 通知 server 据 (from, target) 定位挂起会话并撤销被控端弹窗。
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::CancelRequest { target },
            }
        }
        FromUi::StartRemote {
            target_id,
            password,
        } => {
            session.lock().await.initiate_cancelled = false;
            Envelope {
                from: self_id.to_string(),
                to: Some(target_id.clone()),
                ts: now(),
                payload: Message::ConnectRequest {
                    mode: protocol::Mode::B,
                    target: target_id,
                    password: opt_password(password),
                    force: false,
                },
            }
        }
        FromUi::AuthDecision { session_id, accept } => {
            if accept {
                // 被控端授权通过 → 进入被控态 + 启动截屏推帧（主控才有画面）。
                // 关键：Start 必须挂在此「上行授权」处——被控端不会收到 AuthResult 下行回执
                //（server 消费 AuthResult 后只把 ConnectAck 回给主控），挂下行分支等于永不触发。
                session.lock().await.controlled = Some(session_id.clone());
                CAPTURE_CTRL.send(CaptureCtrl::Start {
                    session_id: session_id.clone(),
                });
                CLIPBOARD_TX.send(ClipboardMsg::Start {
                    session_id: session_id.clone(),
                });
            }
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::AuthResult {
                    session_id,
                    ok: accept,
                    reason: if accept {
                        None
                    } else {
                        Some("用户拒绝".into())
                    },
                },
            }
        }
        FromUi::Input { session_id, event } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::Input { session_id, event },
        },
        // 注意：正常路径下 FromUi::Frame 已在 connect_once 出站泵处被拦截、改走 frame_tx
        // 单槽 watch（drop-stale），不会到达这里。此臂仅作穷尽匹配的安全兜底——若被命中
        // 仍能正确出帧（退化为走可靠 out_tx，不丢但不 drop-stale）。切勿据此误判帧走 control lane。
        FromUi::Frame {
            session_id,
            data,
            w,
            h,
            seq,
        } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给控制方
            ts: now(),
            payload: Message::Frame {
                session_id,
                data,
                w,
                h,
                seq,
            },
        },
        FromUi::Notice { session_id, text } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给主控
            ts: now(),
            payload: Message::RemoteNotice { session_id, text },
        },
        FromUi::SetQuality { session_id, mode } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给被控端
            ts: now(),
            payload: Message::SetQuality {
                session_id,
                mode,
                resolution: None,
                clarity: None,
                fps: None,
            },
        },
        FromUi::ClipboardSync { session_id, text } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session_id 路由给对端
            ts: now(),
            payload: Message::ClipboardSync { session_id, text },
        },
        FromUi::Disconnect { session_id } => {
            session.lock().await.controlling = None;
            CLIPBOARD_TX.send(ClipboardMsg::Stop);
            crate::transfer::clear_pull_targets(); // 主控自断开：清理取回目标登记
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::SessionEnd { session_id },
            }
        }
        FromUi::StopControlled { session_id } => {
            {
                let mut ctx = session.lock().await;
                if ctx.controlled.as_deref() == Some(session_id.as_str()) {
                    ctx.controlled = None;
                    ctx.controlled_peer_name = None;
                }
            }
            CAPTURE_CTRL.send(CaptureCtrl::Stop);
            CLIPBOARD_TX.send(ClipboardMsg::Stop);
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::SessionEnd { session_id },
            }
        }
        // 被控端截图回发：to=请求方(admin)，endpoint_id=本机 id，由 server forward_by_to 路由
        FromUi::ScreenshotResp {
            req_id,
            requester,
            data,
            w,
            h,
        } => Envelope {
            from: self_id.to_string(),
            to: Some(requester),
            ts: now(),
            payload: Message::ScreenshotResp {
                req_id,
                endpoint_id: self_id.to_string(),
                data,
                w,
                h,
            },
        },
        FromUi::ExecCommand {
            session_id,
            command,
        } => Envelope {
            from: self_id.to_string(),
            to: None, // server 按 session 路由给被控端
            ts: now(),
            payload: Message::ExecRequest {
                session_id,
                exec_id: next_id("exec"),
                command,
                timeout_ms: crate::exec::MAX_TIMEOUT_MS,
            },
        },
        FromUi::ListRemote { session_id, path } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::FileListRequest {
                session_id,
                transfer_id: next_id("ls"),
                path,
            },
        },
        FromUi::SendChat { session_id, text } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::ChatMessage {
                session_id,
                msg_id: next_id("msg"),
                text,
            },
        },
        FromUi::SetCapture { session_id, active } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::SetCapture { session_id, active },
        },
        // 下发（push）：直接委托 transfer::send_file_push 在独立任务里读本机文件分块出站，
        // 本分支不构造 Envelope（已在子任务里发首包+块），故提前 return。
        FromUi::PushFile {
            session_id,
            local_path,
            dest_dir,
        } => {
            tokio::spawn(crate::transfer::send_file_push(
                out_tx.clone(),
                self_id.to_string(),
                session_id,
                next_id("tx"),
                local_path,
                dest_dir,
            ));
            return;
        }
        // 取回（pull）：先登记 transfer_id→本地保存目录（被控回流首包据此落盘），再发请求。
        FromUi::PullFile {
            session_id,
            remote_path,
            local_dir,
        } => {
            let transfer_id = next_id("tx");
            crate::transfer::set_pull_target(&transfer_id, std::path::PathBuf::from(local_dir));
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::FilePullRequest {
                    session_id,
                    transfer_id,
                    path: remote_path,
                },
            }
        }
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
    }
}

#[cfg(test)]
mod uplink_tests {
    use std::sync::Arc;

    use protocol::{Envelope, Message};
    use tokio::sync::mpsc;

    use super::super::conn::SessionCtx;
    use super::super::FromUi;
    use super::{handle_uplink, opt_password};

    #[test]
    fn 空密码映射为_none() {
        assert_eq!(opt_password(String::new()), None);
    }

    /// Bug 回归（issue#4）：主控取消申请 → 上行须发 CancelRequest（带 target），
    /// 且 controlling 为 None 时置 initiate_cancelled（迟到 ConnectAck 收尾）。
    #[tokio::test]
    async fn cancel_remote_uplink_sends_cancel_request_with_target() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::CancelRemote {
                target: "638924533".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 CancelRequest");
        assert!(
            s.contains("\"type\":\"cancel_request\""),
            "缺 cancel_request: {s}"
        );
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::CancelRequest { target } => assert_eq!(target, "638924533"),
            other => panic!("应为 CancelRequest，实际 {other:?}"),
        }
        assert!(
            session.lock().await.initiate_cancelled,
            "controlling 为 None 时应置 initiate_cancelled"
        );
    }
    #[test]
    fn 非空密码映射为_some() {
        assert_eq!(opt_password("123456".into()), Some("123456".into()));
    }
    #[test]
    fn 来源标签_按模式() {
        assert_eq!(super::control_source(protocol::Mode::A), "管理员");
        assert_eq!(super::control_source(protocol::Mode::B), "终端伙伴");
    }

    /// 远程命令上行：ExecCommand → ExecRequest（带 session_id/command，timeout_ms 用封顶值）。
    #[tokio::test]
    async fn exec_command_uplink_sends_exec_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::ExecCommand {
                session_id: "s-1".into(),
                command: "whoami".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 ExecRequest");
        assert!(
            s.contains("\"type\":\"exec_request\""),
            "缺 exec_request: {s}"
        );
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::ExecRequest {
                session_id,
                command,
                ..
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(command, "whoami");
            }
            other => panic!("应为 ExecRequest，实际 {other:?}"),
        }
    }

    /// 远端目录浏览上行：ListRemote → FileListRequest（path 透传）。
    #[tokio::test]
    async fn list_remote_uplink_sends_file_list_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::ListRemote {
                session_id: "s-1".into(),
                path: "/home".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 FileListRequest");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileListRequest {
                session_id, path, ..
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(path, "/home");
            }
            other => panic!("应为 FileListRequest，实际 {other:?}"),
        }
    }

    /// 即时消息上行：SendChat → ChatMessage（带 msg_id，text 透传）。
    #[tokio::test]
    async fn send_chat_uplink_sends_chat_message() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::SendChat {
                session_id: "s-1".into(),
                text: "你好".into(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 ChatMessage");
        assert!(
            s.contains("\"type\":\"chat_message\""),
            "缺 chat_message: {s}"
        );
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::ChatMessage {
                session_id,
                msg_id,
                text,
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(text, "你好");
                assert!(!msg_id.is_empty(), "msg_id 必须非空（AtomicU64 计数器）");
            }
            other => panic!("应为 ChatMessage，实际 {other:?}"),
        }
    }

    /// 懒推流上行：SetCapture → SetCapture（active 透传）。
    #[tokio::test]
    async fn set_capture_uplink_sends_set_capture() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::SetCapture {
                session_id: "s-1".into(),
                active: true,
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 SetCapture");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(
            env.payload,
            Message::SetCapture { active: true, .. }
        ));
    }

    /// 取回上行：PullFile 先把 transfer_id→local_dir 记入 PULL_TARGETS，再发 FilePullRequest。
    #[tokio::test]
    async fn pull_file_uplink_records_target_and_sends_request() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let tmp = std::env::temp_dir().join("ohmydesk-pull-dst");
        std::fs::create_dir_all(&tmp).unwrap();
        handle_uplink(
            FromUi::PullFile {
                session_id: "s-1".into(),
                remote_path: "/etc/hostname".into(),
                local_dir: tmp.to_string_lossy().to_string(),
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应发出 FilePullRequest");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FilePullRequest {
                session_id,
                transfer_id,
                path,
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(path, "/etc/hostname");
                // 取回目标已登记：peek（不消费）应等于本地目录
                let got = crate::transfer::peek_pull_target(&transfer_id);
                assert_eq!(
                    got.as_deref(),
                    Some(tmp.as_path()),
                    "transfer_id 应登记 local_dir"
                );
            }
            other => panic!("应为 FilePullRequest，实际 {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::conn::SessionCtx;
    use super::*;

    /// 截图回发上行映射契约：to=请求方、endpoint_id=本机、type=screenshot_resp。
    #[tokio::test]
    async fn screenshot_resp_uplink_envelope_contract() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        handle_uplink(
            FromUi::ScreenshotResp {
                req_id: "req-1".into(),
                requester: "admin-x".into(),
                data: "<b64>".into(),
                w: 1280,
                h: 720,
            },
            "ep-self",
            &tx,
            &session,
        )
        .await;
        let s = rx.recv().await.expect("应有一条出站消息");
        assert!(
            s.contains("\"type\":\"screenshot_resp\""),
            "缺 screenshot_resp tag: {s}"
        );
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(env.from, "ep-self");
        assert_eq!(
            env.to.as_deref(),
            Some("admin-x"),
            "to 必须是请求方，供 server forward_by_to"
        );
        match env.payload {
            Message::ScreenshotResp {
                req_id,
                endpoint_id,
                w,
                h,
                ..
            } => {
                assert_eq!(req_id, "req-1");
                assert_eq!(
                    endpoint_id, "ep-self",
                    "endpoint_id 必须是本机 id（前端按此 key 入缓存）"
                );
                assert_eq!((w, h), (1280, 720));
            }
            _ => panic!("payload 类型错误"),
        }
    }

    /// Bug 修复回归：被控端「同意」（上行 AuthDecision accept）必须 → 进入被控态 +
    /// 启动截屏推帧（CAPTURE_CTRL.Start）。此前错挂在永不到达的下行 AuthResult 分支，致主控黑屏。
    #[tokio::test]
    async fn auth_accept_uplink_enters_controlled_and_starts_capture() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let (cap_tx, mut cap_rx) = mpsc::unbounded_channel::<CaptureCtrl>();
        CAPTURE_CTRL.init(cap_tx);

        handle_uplink(
            FromUi::AuthDecision {
                session_id: "sess-9".into(),
                accept: true,
            },
            "ep-victim",
            &tx,
            &session,
        )
        .await;

        // ① 进入被控态（截屏循环据此判活）
        assert_eq!(session.lock().await.controlled.as_deref(), Some("sess-9"));
        // ② 启动截屏推帧信号
        match cap_rx.try_recv() {
            Ok(CaptureCtrl::Start { session_id }) => assert_eq!(session_id, "sess-9"),
            other => panic!("应收到 CAPTURE_CTRL.Start，实际 {other:?}"),
        }
        // ③ 仍发出 AuthResult ok=true
        let s = rx.recv().await.unwrap();
        assert!(s.contains("\"type\":\"auth_result\""));
        assert!(s.contains("\"ok\":true"));
    }

    /// 辅助：把一条 payload 包成 Envelope 文本，喂给 handle_downlink。
    fn env_text(from: &str, payload: Message) -> String {
        serde_json::to_string(&Envelope {
            from: from.into(),
            to: None,
            ts: 0,
            payload,
        })
        .unwrap()
    }

    /// 主控端收 ExecResult → 投 ToUi::ExecResult（exec_id/exit_code/stdout 透传）。
    #[tokio::test]
    async fn downlink_exec_result_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        // 主控态门控：须为该会话主控方才投 UI。
        session.lock().await.controlling = Some("s-1".into());
        let t = env_text(
            "ep-peer",
            Message::ExecResult {
                session_id: "s-1".into(),
                exec_id: "exec-7".into(),
                exit_code: Some(0),
                stdout: "root".into(),
                stderr: String::new(),
                truncated: false,
                duration_ms: 12,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::ExecResult") {
            ToUi::ExecResult {
                exec_id,
                exit_code,
                stdout,
                ..
            } => {
                assert_eq!(exec_id, "exec-7");
                assert_eq!(exit_code, Some(0));
                assert_eq!(stdout, "root");
            }
            other => panic!("应为 ExecResult，实际 {other:?}"),
        }
    }

    /// 主控端收 FileListResp → 投 ToUi::RemoteEntries（path + entries + error 透传）。
    #[tokio::test]
    async fn downlink_file_list_resp_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        // 主控态门控：须为该会话主控方才投 UI。
        session.lock().await.controlling = Some("s-1".into());
        let t = env_text(
            "ep-peer",
            Message::FileListResp {
                session_id: "s-1".into(),
                transfer_id: "ls-1".into(),
                path: "/home/me".into(),
                entries: vec![protocol::FileEntry {
                    name: "docs".into(),
                    is_dir: true,
                    size: 0,
                }],
                error: None,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::RemoteEntries") {
            ToUi::RemoteEntries {
                path,
                entries,
                error,
            } => {
                assert_eq!(path, "/home/me");
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "docs");
                assert!(error.is_none());
            }
            other => panic!("应为 RemoteEntries，实际 {other:?}"),
        }
    }

    /// 主控端收对端 ChatMessage → 投 ToUi::ChatIncoming（text 透传）。
    #[tokio::test]
    async fn downlink_chat_message_to_ui() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let t = env_text(
            "ep-peer",
            Message::ChatMessage {
                session_id: "s-1".into(),
                msg_id: "msg-3".into(),
                text: "看下报错".into(),
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match to_ui_rx.try_recv().expect("应投 ToUi::ChatIncoming") {
            ToUi::ChatIncoming { msg_id, text, .. } => {
                assert_eq!(msg_id, "msg-3");
                assert_eq!(text, "看下报错");
            }
            other => panic!("应为 ChatIncoming，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn incoming_control_优先显示操作账号名() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        let t = env_text(
            "server",
            Message::IncomingControl {
                session_id: "s-1".into(),
                from: "admin-vazkcy".into(),
                operator_username: Some("caodan".into()),
                mode: protocol::Mode::B,
                auto_accept: true,
            },
        );

        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();

        match to_ui_rx.try_recv().expect("应进入被控态") {
            ToUi::BeingControlled { peer_name, .. } => assert_eq!(peer_name, "caodan"),
            other => panic!("应为 BeingControlled，实际 {other:?}"),
        }
    }

    #[test]
    fn 控制方显示名_缺账号名时回退连接id() {
        assert_eq!(
            control_peer_name(Some(" caodan "), "admin-vazkcy"),
            "caodan"
        );
        assert_eq!(
            control_peer_name(Some("   "), "admin-vazkcy"),
            "admin-vazkcy"
        );
        assert_eq!(control_peer_name(None, "admin-vazkcy"), "admin-vazkcy");
    }

    /// 主控态收 pull 回流（FileOpen{dir:Pull} + FileChunk last）→ 据 PULL_TARGETS 落盘到本机目录。
    #[tokio::test]
    async fn downlink_pull_flow_writes_local_file() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, mut to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        // 主控态：controlling = s-pull
        session.lock().await.controlling = Some("s-pull".into());

        // 模拟：上行 PullFile 已登记 transfer_id→本地目录
        let dst = std::env::temp_dir().join("ohmydesk-pull-recv");
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(&dst).unwrap();
        crate::transfer::set_pull_target("tx-pull", dst.clone());

        // 回流首包：FileOpen{dir:Pull}
        let open = env_text(
            "ep-peer",
            Message::FileOpen {
                session_id: "s-pull".into(),
                transfer_id: "tx-pull".into(),
                name: "report.log".into(),
                size: 5,
                dir: protocol::FileDir::Pull,
                dest: None,
            },
        );
        handle_downlink(&open, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        // 回流末块：FileChunk last=true
        let chunk = env_text(
            "ep-peer",
            Message::FileChunk {
                session_id: "s-pull".into(),
                transfer_id: "tx-pull".into(),
                seq: 0,
                data: STANDARD.encode(b"hello"),
                last: true,
            },
        );
        handle_downlink(&chunk, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();

        // 落盘到本机目录
        let saved = dst.join("report.log");
        assert!(saved.exists(), "取回文件应落到本机目录");
        assert_eq!(std::fs::read(&saved).unwrap(), b"hello");
        // 应投 FileDone/FileProgress 类通知给 UI
        let mut saw_done = false;
        while let Ok(ev) = to_ui_rx.try_recv() {
            if matches!(ev, ToUi::FileNotice { .. } | ToUi::FileProgress { .. }) {
                saw_done = true;
            }
        }
        assert!(saw_done, "应投 FileNotice/FileProgress 通知 UI");
        let _ = std::fs::remove_dir_all(&dst);
    }

    /// 被控端收 SetCapture{active:false} → CAPTURE_CTRL.Stop；active:true → Start。controlled 态不变。
    #[tokio::test]
    async fn downlink_set_capture_toggles_capture_only() {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        let (to_ui, _to_ui_rx) = mpsc::unbounded_channel::<ToUi>();
        let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));
        session.lock().await.controlled = Some("s-cap".into());

        let (cap_tx, mut cap_rx) = mpsc::unbounded_channel::<CaptureCtrl>();
        CAPTURE_CTRL.init(cap_tx);

        // active:false → Stop
        let t = env_text(
            "ep-ctrl",
            Message::SetCapture {
                session_id: "s-cap".into(),
                active: false,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        assert!(matches!(cap_rx.try_recv(), Ok(CaptureCtrl::Stop)));
        // controlled 态绝不被 SetCapture 改动
        assert_eq!(session.lock().await.controlled.as_deref(), Some("s-cap"));

        // active:true → Start
        let t = env_text(
            "ep-ctrl",
            Message::SetCapture {
                session_id: "s-cap".into(),
                active: true,
            },
        );
        handle_downlink(&t, "ep-self", &out_tx, &to_ui, &session)
            .await
            .unwrap();
        match cap_rx.try_recv() {
            Ok(CaptureCtrl::Start { session_id }) => assert_eq!(session_id, "s-cap"),
            other => panic!("应为 Start，实际 {other:?}"),
        }
    }
}
