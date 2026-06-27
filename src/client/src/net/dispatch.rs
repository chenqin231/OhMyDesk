//! 下行/上行消息分发：下行解析 Envelope → 通知 UI / 旁路执行；上行动作 → 出站。

use std::sync::Arc;

use protocol::{Envelope, Message};
use tokio::sync::mpsc;

use super::conn::SessionCtx;
use super::{now, CaptureCtrl, FromUi, ToUi, CAPTURE_CTRL, INJECT_TX, SCREENSHOT_TX};

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
        // 被控端收到控制请求 → 通知 UI 弹授权框
        Message::ConnectRequest { target, .. } if target == self_id => {
            // protocol 的 ConnectRequest 不带 session_id；用请求方 from 作临时关联键，
            // 被控端授权时按此回传，server 端按连接/会话关联校正（集成期对齐）。
            let _ = to_ui.send(ToUi::ControlRequest {
                requester: env.from.clone(),
                session_id: env.from.clone(),
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
        // 主控端收到画面帧 → 通知 UI 贴帧
        Message::Frame { data, w, h, .. } => {
            let _ = to_ui.send(ToUi::Frame { data, w, h });
        }
        // 被控端收到键鼠 → 经旁路交 main 注入侧（注入依赖 X11，不在 net 任务里执行）
        Message::Input { session_id, event } => {
            let ctx = session.lock().await;
            if ctx.controlled.as_deref() == Some(session_id.as_str()) {
                drop(ctx);
                let _ = out_tx; // Input 不回发，交注入侧
                INJECT_TX.with_send(session_id, event);
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
        FromUi::StartRemote { target_id, password } => Envelope {
            from: self_id.to_string(),
            to: Some(target_id.clone()),
            ts: now(),
            payload: Message::ConnectRequest {
                mode: protocol::Mode::B,
                target: target_id,
                password: Some(password),
            },
        },
        FromUi::AuthDecision { session_id, accept } => Envelope {
            from: self_id.to_string(),
            to: None,
            ts: now(),
            payload: Message::AuthResult {
                session_id,
                ok: accept,
                reason: if accept { None } else { Some("用户拒绝".into()) },
            },
        },
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
        FromUi::Disconnect { session_id } => {
            session.lock().await.controlling = None;
            Envelope {
                from: self_id.to_string(),
                to: None,
                ts: now(),
                payload: Message::SessionEnd { session_id },
            }
        }
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
    }
}
