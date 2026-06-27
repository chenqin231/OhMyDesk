//! WS 消息中枢：连接管理 + 信封路由分发主干。
//! 具体消息处理逻辑见 handlers.rs。
//! W0-1：Register 回发 RegisterAck。
//! M-SRV4：转发 Input 时对对应会话的 InputAggregator.bump()。

use std::sync::Arc;

use dashmap::DashMap;
use protocol::{AuditType, Envelope, Message};
use tokio::sync::mpsc;

use crate::audit::AuditStore;
use crate::handlers;
use crate::registry::Registry;
use crate::session::SessionStore;

pub struct Hub {
    pub reg: Arc<Registry>,
    pub sessions: Arc<SessionStore>,
    pub audit: Arc<AuditStore>,
    /// endpoint_id / admin_id → 出站消息通道
    clients: DashMap<String, mpsc::UnboundedSender<String>>,
}

impl Hub {
    pub fn new(
        reg: Arc<Registry>,
        sessions: Arc<SessionStore>,
        audit: Arc<AuditStore>,
    ) -> Self {
        Hub {
            reg,
            sessions,
            audit,
            clients: DashMap::new(),
        }
    }

    pub fn add_client(&self, id: String, tx: mpsc::UnboundedSender<String>) {
        self.clients.insert(id, tx);
    }

    pub fn remove_client(&self, id: &str) {
        self.clients.remove(id);
    }

    /// 定向推送给某个已注册的 client
    pub fn send_to(&self, id: &str, json: &str) {
        if let Some(tx) = self.clients.get(id) {
            let _ = tx.send(json.to_string());
        }
    }

    /// 广播给所有以 "admin-" 开头的连接
    pub fn broadcast_admins(&self, json: &str) {
        for kv in self.clients.iter() {
            if kv.key().starts_with("admin-") {
                let _ = kv.value().send(json.to_string());
            }
        }
    }

    /// 向所有在线 agent（非 admin）广播截图指令
    pub fn broadcast_agents(&self, json: &str) {
        for kv in self.clients.iter() {
            if !kv.key().starts_with("admin-") {
                let _ = kv.value().send(json.to_string());
            }
        }
    }

    /// 推送最新 endpoint_list 给所有 admin 连接；now 为秒级时间戳
    pub fn push_list(&self, now: i64) {
        let env = Envelope {
            from: "server".into(),
            to: None,
            ts: now,
            payload: Message::EndpointList {
                endpoints: self.reg.views(now),
            },
        };
        if let Ok(json) = serde_json::to_string(&env) {
            self.broadcast_admins(&json);
        }
    }

    /// 发送 RegisterAck 给刚注册的 endpoint（W0-1）
    fn send_ack(&self, to_id: &str, ts: i64) {
        let ack = Envelope {
            from: "server".into(),
            to: Some(to_id.to_string()),
            ts,
            payload: Message::RegisterAck {
                id: to_id.to_string(),
            },
        };
        if let Ok(json) = serde_json::to_string(&ack) {
            self.send_to(to_id, &json);
        }
    }

    /// 按信封 to 字段定向转发
    pub fn forward_by_to(&self, env: &Envelope) {
        if let Some(to) = &env.to {
            if let Ok(json) = serde_json::to_string(env) {
                self.send_to(to, &json);
            }
        }
    }

    /// 按 session 对端路由：Frame/Input 上行 to:None，server 据 session_id 查对端转发
    fn route_to_peer(&self, session_id: &str, env: &Envelope) {
        if let Some(peer) = self.sessions.peer_of(session_id, &env.from) {
            if let Ok(json) = serde_json::to_string(env) {
                self.send_to(&peer, &json);
            }
        }
    }

    /// 处理一条入站信封；now 为秒级 Unix 时间戳
    pub async fn handle(&self, env: Envelope, now: i64) {
        match &env.payload {
            // ── 注册（W0-1：回发 RegisterAck；刷新注册表 + 广播列表）──────────
            Message::Register { info, password } => {
                self.reg.upsert(*info.clone(), password.clone(), now);
                self.send_ack(&env.from, now);
                self.push_list(now);
            }

            // ── 心跳（刷新在线时间 + 更新列表）──────────────────────────────
            Message::Heartbeat { id, .. } => {
                self.reg.touch(id, now);
                self.push_list(now);
            }

            // ── 发起连接请求（A/B 鉴权路由，委托 handlers）────────────────────
            Message::ConnectRequest {
                mode,
                target,
                password,
            } => {
                handlers::handle_connect_request(
                    self,
                    &env.from,
                    mode,
                    target,
                    password.as_deref(),
                    now,
                )
                .await;
            }

            // ── 被控端授权结果（委托 handlers）────────────────────────────────
            Message::AuthResult {
                session_id,
                ok,
                reason,
            } => {
                handlers::handle_auth_result(self, session_id, *ok, reason.as_deref(), now).await;
            }

            // ── Input：主控→被控，bump 计数(M-SRV4) + 按 session 对端路由 ──────
            Message::Input { session_id, .. } => {
                self.sessions.bump_input(session_id);
                self.route_to_peer(session_id, &env);
            }

            // ── 截图请求：仅认证 admin 可发；落审计 + 广播全 agent ────────────
            Message::ScreenshotReq { req_id } => {
                if !env.from.starts_with("admin-") {
                    tracing::warn!("拒绝非 admin 的截图请求: from={}", env.from);
                    return;
                }
                tracing::debug!("截图广播 req_id={req_id}");
                self.audit
                    .log(req_id, &env.from, AuditType::Screenshot, "批量截图指令")
                    .await;
                if let Ok(json) = serde_json::to_string(&env) {
                    self.broadcast_agents(&json);
                }
            }

            // ── 会话结束（委托 handlers）──────────────────────────────────────
            Message::SessionEnd { session_id } => {
                // 先把结束通知转发给对端：被控端据此清除"正在被远程控制"态并停推帧。
                // 必须在 handle_session_end 之前——后者 end_session 会移除会话，
                // route_to_peer 依赖会话仍在册才能查到对端（Bug：断开后被控端横幅常驻）。
                self.route_to_peer(session_id, &env);
                handlers::handle_session_end(self, session_id, now).await;
            }

            // ── Frame：被控→主控，按 session 对端路由 ─────────────────────────
            Message::Frame { session_id, .. } => {
                self.route_to_peer(session_id, &env);
            }

            // ── ConnectAck/Reject/ScreenshotResp：按 to 定向转发 ──────────────
            Message::ConnectAck { .. } | Message::Reject { .. } | Message::ScreenshotResp { .. } => {
                self.forward_by_to(&env);
            }

            // ── 远程命令执行：按 session 对端路由；ExecRequest 落 Command 审计 ──
            Message::ExecRequest {
                session_id, command, ..
            } => {
                let summary: String = command.chars().take(200).collect();
                self.audit
                    .log(
                        session_id,
                        &env.from,
                        AuditType::Command,
                        &format!("执行命令: {summary}"),
                    )
                    .await;
                self.route_to_peer(session_id, &env);
            }
            Message::ExecResult { session_id, .. } => {
                self.route_to_peer(session_id, &env);
            }

            // ── 文件传输：按 session 对端路由；FileOpen 落 FileTransfer 审计 ────
            Message::FileOpen {
                session_id,
                name,
                size,
                dir,
                ..
            } => {
                let way = match dir {
                    protocol::FileDir::Push => "下发",
                    protocol::FileDir::Pull => "取回",
                };
                self.audit
                    .log(
                        session_id,
                        &env.from,
                        AuditType::FileTransfer,
                        &format!("文件{way}: {name} ({size} 字节)"),
                    )
                    .await;
                self.route_to_peer(session_id, &env);
            }
            Message::FileChunk { session_id, .. }
            | Message::FilePullRequest { session_id, .. }
            | Message::FileError { session_id, .. } => {
                self.route_to_peer(session_id, &env);
            }

            // server 单向发出的消息，不处理客户端发来的
            Message::RegisterAck { .. }
            | Message::EndpointList { .. }
            | Message::IncomingControl { .. } => {}
        }
    }
}

/// 秒级 Unix 时间戳
pub fn now_sec() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionStore;
    use protocol::{Mode, Session, SessionStatus};

    fn test_hub() -> Hub {
        Hub::new(
            Arc::new(Registry::new()),
            Arc::new(SessionStore::new()),
            Arc::new(AuditStore::new(None)),
        )
    }

    /// Bug 回归：主控发 SessionEnd 时，server 必须把结束通知转发给被控端，
    /// 否则被控端永不清除"正在被远程控制"态（断开后横幅常驻）。
    #[tokio::test]
    async fn session_end_forwarded_to_controlled_peer() {
        let hub = test_hub();
        let (admin_tx, mut admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-1".into(), admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);

        let sid = "sess-1".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::A,
            from_id: "admin-1".into(),
            to_id: "ep-victim".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
        });

        // 主控（admin）发起 SessionEnd
        let env = Envelope {
            from: "admin-1".into(),
            to: None,
            ts: 200,
            payload: Message::SessionEnd {
                session_id: sid.clone(),
            },
        };
        hub.handle(env, 200).await;

        // 被控端必须收到一条 SessionEnd
        let got = victim_rx
            .try_recv()
            .expect("被控端应收到 SessionEnd 结束通知");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::SessionEnd { session_id } => assert_eq!(session_id, sid),
            other => panic!("被控端收到的应为 SessionEnd，实际 {other:?}"),
        }
        // 主控自己不应收到回发（它就是发起方）
        assert!(
            admin_rx.try_recv().is_err(),
            "结束通知不应回发给发起方 admin"
        );
    }

    /// 高危路径回归：ExecRequest 必须按 session 路由给被控端，且落审计不 panic。
    #[tokio::test]
    async fn exec_request_forwarded_to_controlled_peer() {
        let hub = test_hub();
        let (_admin_tx, _admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-1".into(), _admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);

        let sid = "sess-x".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::A,
            from_id: "admin-1".into(),
            to_id: "ep-victim".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
        });

        let env = Envelope {
            from: "admin-1".into(),
            to: None,
            ts: 200,
            payload: Message::ExecRequest {
                session_id: sid.clone(),
                exec_id: "e-1".into(),
                command: "whoami".into(),
                timeout_ms: 5000,
            },
        };
        hub.handle(env, 200).await;

        let got = victim_rx.try_recv().expect("被控端应收到 ExecRequest");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::ExecRequest { command, .. } => assert_eq!(command, "whoami"),
            other => panic!("应为 ExecRequest，实际 {other:?}"),
        }
    }
}
