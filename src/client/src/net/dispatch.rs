//! 下行/上行消息分发：下行解析 Envelope → 通知 UI / 旁路执行；上行动作 → 出站。

use std::sync::Arc;

use protocol::{Envelope, Message};
use tokio::sync::mpsc;

use super::conn::SessionCtx;
use super::{now, CaptureCtrl, FromUi, ToUi, CAPTURE_CTRL, INJECT_TX, SCREENSHOT_TX};

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
        // 被控端收到 server 转发的来控通知 → 通知 UI 弹授权框。
        // server 已生成会话并分配真 session_id（I2 时序：主控 ConnectRequest → server 建会话 →
        // 推 IncomingControl 给被控端），被控端授权时按此真 session_id 回 AuthResult。
        Message::IncomingControl {
            session_id, from, ..
        } => {
            let _ = to_ui.send(ToUi::ControlRequest {
                requester: from,
                session_id,
            });
        }
        // 鉴权结果（server 下发）：被控端据此进入被控态并回 ConnectAck 由 server 处理
        Message::AuthResult {
            session_id,
            ok,
            reason,
        } => {
            if ok {
                session.lock().await.controlled = Some(session_id.clone());
                // 进入被控态：启动 2-3fps 截屏推帧（main 截屏线程消费此信号）
                CAPTURE_CTRL.send(CaptureCtrl::Start {
                    session_id: session_id.clone(),
                });
                let _ = to_ui.send(ToUi::BeingControlled {
                    peer_name: "远程方".into(),
                });
            } else {
                let _ = to_ui.send(ToUi::RemoteRejected {
                    reason: reason.unwrap_or_else(|| "鉴权失败".into()),
                });
            }
        }
        // 主控端收到 ack：进入主控态
        Message::ConnectAck { session_id } => {
            session.lock().await.controlling = Some(session_id.clone());
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
            ..
        } => {
            let _ = to_ui.send(ToUi::Frame {
                session_id,
                data,
                w,
                h,
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
        Message::SetQuality { session_id, mode } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            tracing::info!("被控收到画质切换 mode={mode:?} controlled={controlled} session={session_id}");
            if controlled {
                crate::capture::set_quality(mode);
                let p = crate::capture::current_params();
                tracing::info!("被控已应用画质 上限={}x{} q={} 间隔={}ms", p.max_w, p.max_h, p.jpeg_q, p.interval_ms);
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
                CAPTURE_CTRL.send(CaptureCtrl::Stop); // 停被控端推帧
            }
            let _ = to_ui.send(ToUi::SessionEnded);
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

        // ── 被控端：收 push 下发首包 → 打开接收文件（失败回 FileError）──────────
        Message::FileOpen {
            session_id,
            transfer_id,
            name,
            size,
            dir,
            dest,
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled && dir == protocol::FileDir::Push {
                if let Err(reason) =
                    crate::transfer::open_recv(&transfer_id, &name, size, dest.as_deref())
                {
                    send_file_error(out_tx, self_id, session_id, transfer_id, reason);
                }
            }
            // dir==Pull：本端作为控制方收到回流首包 → P1（端到端 UI）
        }

        // ── 被控端：收 push 数据块 → 落盘；末块成功回 FileDone(带最终路径)，失败回 FileError ──
        Message::FileChunk {
            session_id,
            transfer_id,
            data,
            last,
            ..
        } => {
            let controlled =
                session.lock().await.controlled.as_deref() == Some(session_id.as_str());
            if controlled {
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
            }
            // 控制方收 pull 回流块 → P1
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

        // ── 传输失败：清理在途接收 ─────────────────────────────────────────────
        Message::FileError { transfer_id, .. } => {
            crate::transfer::abort(&transfer_id);
        }
        _ => {}
    }
    Ok(())
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
        FromUi::StartRemote {
            target_id,
            password,
        } => Envelope {
            from: self_id.to_string(),
            to: Some(target_id.clone()),
            ts: now(),
            payload: Message::ConnectRequest {
                mode: protocol::Mode::B,
                target: target_id,
                password: Some(password),
                force: false,
            },
        },
        FromUi::AuthDecision { session_id, accept } => {
            if accept {
                // 被控端授权通过 → 进入被控态 + 启动截屏推帧（主控才有画面）。
                // 关键：Start 必须挂在此「上行授权」处——被控端不会收到 AuthResult 下行回执
                //（server 消费 AuthResult 后只把 ConnectAck 回给主控），挂下行分支等于永不触发。
                session.lock().await.controlled = Some(session_id.clone());
                CAPTURE_CTRL.send(CaptureCtrl::Start {
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
            payload: Message::SetQuality { session_id, mode },
        },
        FromUi::Disconnect { session_id } => {
            session.lock().await.controlling = None;
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
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
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
}
