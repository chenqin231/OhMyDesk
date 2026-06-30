//! 具体消息处理逻辑，由 hub::Hub::handle() 分发调用。
//! 职责：ConnectRequest 鉴权路由、AuthResult 会话建立、SessionEnd 审计落库。

use protocol::{AuditType, Envelope, Message, Mode, Session, SessionStatus};
use uuid::Uuid;

use crate::hub::Hub;

const SELF_REMOTE_REJECT_REASON: &str = "您不能远程自己！";

fn send_reject(hub: &Hub, to: &str, session_id: String, reason: &str, now: i64) {
    let reject_env = Envelope {
        from: "server".into(),
        to: Some(to.to_string()),
        ts: now,
        payload: Message::Reject {
            session_id,
            reason: reason.to_string(),
        },
    };
    if let Ok(json) = serde_json::to_string(&reject_env) {
        hub.send_to(to, &json);
    }
}

/// 连接决策结果（纯逻辑，便于单测）。
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectDecision {
    /// 免同意直连（密码正确 / 管理员强制）。
    AutoAccept,
    /// 需被控端弹框同意。
    Consent,
    /// 模式 B 密码错误，拒绝。
    RejectBadPassword,
}

/// 纯决策：`password_ok` 由调用方对模式 B 有密码场景预先 `check_password` 求值。
pub fn decide_connect(mode: Mode, has_password: bool, password_ok: bool, force: bool) -> ConnectDecision {
    match mode {
        Mode::B => {
            if has_password {
                if password_ok { ConnectDecision::AutoAccept } else { ConnectDecision::RejectBadPassword }
            } else {
                ConnectDecision::Consent
            }
        }
        Mode::A => {
            if force { ConnectDecision::AutoAccept } else { ConnectDecision::Consent }
        }
    }
}

/// ConnectRequest A/B 鉴权路由：
/// - 自连/模式A越权：Reject；
/// - 模式 B 密码错：Reject + AuthFail 审计；
/// - 免同意（B 密码对 / A force）：建 Active 会话 + 回 ConnectAck + 发 IncomingControl{auto_accept:true}；
/// - 需同意（B 无密码 / A 非 force）：建会话 + 发 IncomingControl{auto_accept:false}（等被控端 AuthResult）。
pub async fn handle_connect_request(
    hub: &Hub,
    from_id: &str,
    mode: &Mode,
    target: &str,
    password: Option<&str>,
    force: bool,
    now: i64,
) {
    if from_id == target {
        let session_id = Uuid::new_v4().to_string();
        hub.audit
            .log(&session_id, from_id, AuditType::Reject, SELF_REMOTE_REJECT_REASON)
            .await;
        send_reject(hub, from_id, session_id, SELF_REMOTE_REJECT_REASON, now);
        return;
    }

    // 模式 A 越权闸：仅已认证 admin 可发起（同时拦截非 admin 的 force，满足强制远程仅 admin）。
    if *mode == Mode::A && !from_id.starts_with("admin-") {
        tracing::warn!("拒绝非 admin 的模式A远控发起: from={from_id}");
        return;
    }

    // 模式 B 有密码时预先校验，供 decide_connect 决策。
    let has_pw = password.map(|p| !p.is_empty()).unwrap_or(false);
    let pw_ok = *mode == Mode::B && has_pw && hub.reg.check_password(target, password.unwrap_or(""));

    match decide_connect(*mode, has_pw, pw_ok, force) {
        ConnectDecision::RejectBadPassword => {
            let session_id = Uuid::new_v4().to_string();
            hub.audit
                .log(&session_id, from_id, AuditType::AuthFail, "密码错误")
                .await;
            send_reject(hub, from_id, session_id, "密码错误", now);
        }
        ConnectDecision::AutoAccept => {
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
            hub.audit.insert_session(&session).await;
            hub.sessions.insert(session);

            // 免同意：立即回主控 ConnectAck + 落 Connect 审计（标记发起方与免同意来源）。
            let ack = Envelope {
                from: "server".into(),
                to: Some(from_id.to_string()),
                ts: now,
                payload: Message::ConnectAck { session_id: session_id.clone() },
            };
            if let Ok(json) = serde_json::to_string(&ack) {
                hub.send_to(from_id, &json);
            }
            let how = if force { "强制远程(免同意)" } else { "密码连接(免同意)" };
            hub.audit
                .log(&session_id, from_id, AuditType::Connect, how)
                .await;

            send_incoming(hub, target, session_id, from_id, *mode, true, now);
        }
        ConnectDecision::Consent => {
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
            hub.audit.insert_session(&session).await;
            hub.sessions.insert(session);

            send_incoming(hub, target, session_id, from_id, *mode, false, now);
        }
    }
}

/// 发 IncomingControl 给被控端（携带 server 生成的 session_id 与 auto_accept 标记）。
fn send_incoming(hub: &Hub, target: &str, session_id: String, from_id: &str, mode: Mode, auto_accept: bool, now: i64) {
    let incoming = Envelope {
        from: "server".into(),
        to: Some(target.to_string()),
        ts: now,
        payload: Message::IncomingControl {
            session_id,
            from: from_id.to_string(),
            mode,
            auto_accept,
        },
    };
    if let Ok(json) = serde_json::to_string(&incoming) {
        hub.send_to(target, &json);
    }
}

/// 主控取消尚未建立的申请：定位其发起、指向 `target` 的挂起会话，向被控端转发 SessionEnd
/// 让其撤销授权弹窗，并结束会话 + 落审计。无对应会话时静默忽略（取消已完成/超时竞态）。
pub async fn handle_cancel_request(hub: &Hub, from_id: &str, target: &str, now: i64) {
    let Some(session_id) = hub.sessions.outbound_session(from_id, target) else {
        tracing::debug!("CancelRequest 无对应挂起会话: from={from_id} target={target}");
        return;
    };
    // 通知被控端撤销弹窗：被控端 SessionEnd 处理会关弹窗 + 复位被控态。
    let env = Envelope {
        from: "server".into(),
        to: Some(target.to_string()),
        ts: now,
        payload: Message::SessionEnd {
            session_id: session_id.clone(),
        },
    };
    if let Ok(json) = serde_json::to_string(&env) {
        hub.send_to(target, &json);
    }
    hub.sessions
        .end_session(&session_id, now, SessionStatus::Ended);
    hub.audit
        .log(&session_id, from_id, AuditType::Disconnect, "主控取消申请")
        .await;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use protocol::{EndpointInfo, Message};
    use tokio::sync::mpsc;

    use super::*;
    use crate::{audit::AuditStore, registry::Registry, session::SessionStore};

    // ── decide_connect 纯函数单测 ──────────────────────────────────────────────
    use super::{decide_connect, ConnectDecision};
    use protocol::Mode;

    #[test]
    fn 模式b_有密码且正确_免同意() {
        assert_eq!(decide_connect(Mode::B, true, true, false), ConnectDecision::AutoAccept);
    }
    #[test]
    fn 模式b_有密码但错_拒绝() {
        assert_eq!(decide_connect(Mode::B, true, false, false), ConnectDecision::RejectBadPassword);
    }
    #[test]
    fn 模式b_无密码_需同意() {
        assert_eq!(decide_connect(Mode::B, false, false, false), ConnectDecision::Consent);
    }
    #[test]
    fn 模式a_强制_免同意() {
        assert_eq!(decide_connect(Mode::A, false, false, true), ConnectDecision::AutoAccept);
    }
    #[test]
    fn 模式a_非强制_需同意() {
        assert_eq!(decide_connect(Mode::A, false, false, false), ConnectDecision::Consent);
    }
    #[test]
    fn 模式a_强制忽略密码态() {
        assert_eq!(decide_connect(Mode::A, true, false, true), ConnectDecision::AutoAccept);
    }

    fn test_hub() -> Hub {
        Hub::new(
            Arc::new(Registry::new()),
            Arc::new(SessionStore::new()),
            Arc::new(AuditStore::new(None)),
        )
    }

    #[tokio::test]
    async fn connect_request_rejects_remote_self() {
        let hub = test_hub();
        let mut info = EndpointInfo::sample();
        info.id = "ep-self".into();
        hub.reg.upsert(info, "123456".into(), 100);

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-self".into(), tx);

        handle_connect_request(&hub, "ep-self", &Mode::B, "ep-self", Some("123456"), false, 100).await;

        let sent = rx.recv().await.expect("自连应向发起方返回拒绝消息");
        let env: Envelope = serde_json::from_str(&sent).unwrap();
        assert_eq!(env.to.as_deref(), Some("ep-self"));
        match env.payload {
            Message::Reject { reason, .. } => {
                assert_eq!(reason, "您不能远程自己！");
            }
            other => panic!("自连应返回 Reject，实际为 {other:?}"),
        }
        assert!(
            hub.sessions.active_sessions().is_empty(),
            "自连不应创建活跃会话"
        );
    }
}
