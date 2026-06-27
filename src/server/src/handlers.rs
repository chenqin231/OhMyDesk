//! 具体消息处理逻辑，由 hub::Hub::handle() 分发调用。
//! 职责：ConnectRequest 鉴权路由、AuthResult 会话建立、SessionEnd 审计落库。

use protocol::{AuditType, Envelope, Message, Mode, SessionStatus};
use uuid::Uuid;

use crate::hub::Hub;

/// ConnectRequest A/B 鉴权路由
pub async fn handle_connect_request(
    hub: &Hub,
    from_id: &str,
    mode: &Mode,
    target: &str,
    password: Option<&str>,
    now: i64,
) {
    match mode {
        Mode::B => {
            // 模式 B：服务端校验密码
            let pw = password.unwrap_or("");
            if !hub.reg.check_password(target, pw) {
                // 密码错 → 回 Reject + 落审计
                let session_id = Uuid::new_v4().to_string();
                hub.audit
                    .log(&session_id, from_id, AuditType::AuthFail, "密码错误")
                    .await;
                let reject_env = Envelope {
                    from: "server".into(),
                    to: Some(from_id.to_string()),
                    ts: now,
                    payload: Message::Reject {
                        session_id,
                        reason: "密码错误".into(),
                    },
                };
                if let Ok(json) = serde_json::to_string(&reject_env) {
                    hub.send_to(from_id, &json);
                }
                return;
            }
            // 密码正确 → 转发给被控端，等 AuthResult
            hub.forward_by_to(&Envelope {
                from: from_id.to_string(),
                to: Some(target.to_string()),
                ts: now,
                payload: Message::ConnectRequest {
                    mode: *mode,
                    target: target.to_string(),
                    password: password.map(|s| s.to_string()),
                },
            });
        }
        Mode::A => {
            // 模式 A：直接转发给被控端弹授权弹窗
            hub.forward_by_to(&Envelope {
                from: from_id.to_string(),
                to: Some(target.to_string()),
                ts: now,
                payload: Message::ConnectRequest {
                    mode: *mode,
                    target: target.to_string(),
                    password: None,
                },
            });
        }
    }
}

/// AuthResult 被控端授权结果：ok → 落审计建会话；否 → 落拒绝审计
pub async fn handle_auth_result(
    hub: &Hub,
    session_id: &str,
    ok: bool,
    reason: Option<&str>,
) {
    if ok {
        tracing::info!("会话建立 session_id={session_id}");
        hub.audit
            .log(session_id, session_id, AuditType::Connect, "会话建立")
            .await;
    } else {
        let reason_text = reason.unwrap_or("被拒绝");
        hub.audit
            .log(session_id, session_id, AuditType::Reject, reason_text)
            .await;
    }
}

/// SessionEnd：结束会话，落聚合输入审计 + 断开审计，更新 DB 会话终态（M-SRV4）
pub async fn handle_session_end(hub: &Hub, session_id: &str, now: i64) {
    if let Some((session, input_summary)) = hub
        .sessions
        .end_session(session_id, now, SessionStatus::Ended)
    {
        // 落输入聚合审计（M-SRV4）
        hub.audit
            .log(
                session_id,
                &session.from_id,
                AuditType::Input,
                &input_summary,
            )
            .await;
        // 更新 DB 会话终态
        hub.audit
            .end_session(session_id, now, SessionStatus::Ended)
            .await;
        // 落断开审计
        hub.audit
            .log(
                session_id,
                &session.from_id,
                AuditType::Disconnect,
                "会话结束",
            )
            .await;
    } else {
        tracing::debug!("SessionEnd 收到但无对应活跃会话: {session_id}");
    }
}
