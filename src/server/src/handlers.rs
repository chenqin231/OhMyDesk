//! 具体消息处理逻辑，由 hub::Hub::handle() 分发调用。
//! 职责：ConnectRequest 鉴权路由、AuthResult 会话建立、SessionEnd 审计落库。

use protocol::{AuditType, Envelope, Message, Mode, Session, SessionStatus};
use uuid::Uuid;

use crate::hub::Hub;

/// ConnectRequest A/B 鉴权路由：
/// - 模式 B：密码错 → Reject 回主控 + 落 AuthFail 审计；
/// - 密码正确(B) 或 模式 A：server 生成 session_id，建内存会话，
///   发 IncomingControl{session_id, from, mode} 给被控端（取代旧的转发 ConnectRequest）。
pub async fn handle_connect_request(
    hub: &Hub,
    from_id: &str,
    mode: &Mode,
    target: &str,
    password: Option<&str>,
    now: i64,
) {
    // 鉴权闸：模式 A（管理端→终端）只允许已认证 admin 连接发起。
    // admin 连接已在 WS 升级处用 token 校验过；非 admin 前缀发模式 A 一律拒绝（防伪造发起远控）。
    if *mode == Mode::A && !from_id.starts_with("admin-") {
        tracing::warn!("拒绝非 admin 的模式A远控发起: from={from_id}");
        return;
    }

    // 模式 B：先校验密码
    if *mode == Mode::B {
        let pw = password.unwrap_or("");
        if !hub.reg.check_password(target, pw) {
            // 密码错 → 回 Reject + 落 AuthFail 审计
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
    }

    // 密码正确（B）或模式 A：server 生成 session_id，建会话
    let session_id = Uuid::new_v4().to_string();
    let session = Session {
        id: session_id.clone(),
        mode: *mode,
        from_id: from_id.to_string(),
        to_id: target.to_string(),
        start_at: now,
        end_at: None,
        status: SessionStatus::Active,
    };
    // 持久化会话（M4：/api/sessions 查询 + end_session 终态 UPDATE 有行可改）
    hub.audit.insert_session(&session).await;
    hub.sessions.insert(session);

    // 发 IncomingControl 给被控端（携带 server 生成的 session_id）
    let incoming = Envelope {
        from: "server".into(),
        to: Some(target.to_string()),
        ts: now,
        payload: Message::IncomingControl {
            session_id,
            from: from_id.to_string(),
            mode: *mode,
        },
    };
    if let Ok(json) = serde_json::to_string(&incoming) {
        hub.send_to(target, &json);
    }
}

/// AuthResult 被控端授权结果：
/// - ok → 发 ConnectAck 给主控，落 Connect 审计；会话保持 Active。
/// - 否 → 发 Reject 给主控，结束会话（Rejected），落 Reject 审计。
pub async fn handle_auth_result(
    hub: &Hub,
    session_id: &str,
    ok: bool,
    reason: Option<&str>,
    now: i64,
) {
    let Some(from_id) = hub.sessions.initiator_of(session_id) else {
        tracing::warn!("AuthResult 收到但无对应活跃会话: {session_id}");
        return;
    };

    if ok {
        tracing::info!("会话建立 session_id={session_id} from={from_id}");
        // 回 ConnectAck 给主控端
        let ack = Envelope {
            from: "server".into(),
            to: Some(from_id.clone()),
            ts: now,
            payload: Message::ConnectAck {
                session_id: session_id.to_string(),
            },
        };
        if let Ok(json) = serde_json::to_string(&ack) {
            hub.send_to(&from_id, &json);
        }
        hub.audit
            .log(session_id, &from_id, AuditType::Connect, "会话建立")
            .await;
    } else {
        let reason_text = reason.unwrap_or("被拒绝");
        // 回 Reject 给主控端
        let reject = Envelope {
            from: "server".into(),
            to: Some(from_id.clone()),
            ts: now,
            payload: Message::Reject {
                session_id: session_id.to_string(),
                reason: reason_text.to_string(),
            },
        };
        if let Ok(json) = serde_json::to_string(&reject) {
            hub.send_to(&from_id, &json);
        }
        hub.sessions
            .end_session(session_id, now, SessionStatus::Rejected);
        hub.audit
            .log(session_id, &from_id, AuditType::Reject, reason_text)
            .await;
    }
}

/// SessionEnd：结束会话，落聚合输入审计 + 断开审计，更新 DB 会话终态（M-SRV4）
pub async fn handle_session_end(hub: &Hub, session_id: &str, now: i64) {
    if let Some((session, input_summary)) =
        hub.sessions
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
