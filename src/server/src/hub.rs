//! WS 消息中枢：连接管理 + 信封路由分发主干。
//! 具体消息处理逻辑见 handlers.rs。
//! W0-1：Register 回发 RegisterAck。
//! M-SRV4：转发 Input 时对对应会话的 InputAggregator.bump()。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use protocol::{AuditType, Envelope, Message};
use tokio::sync::mpsc;

use crate::audit::AuditStore;
use crate::handlers;
use crate::registry::Registry;
use crate::session::SessionStore;
use crate::users::PermissionSet;

/// 帧 lane 客户端：watch 发送端 + 入队计数（enqueued）。sent 由对应连接出站泵持有。
pub struct FrameClient {
    pub tx: tokio::sync::watch::Sender<Option<String>>,
    pub enqueued: std::sync::Arc<AtomicU64>,
}

/// frame_lane_drop = 入队 − 实发（clamp≥0），不数 send_replace 覆盖（防过报，spec §4.1 HIGH②）。
pub fn frame_lane_drop(enqueued: u64, sent: u64) -> u64 {
    enqueued.saturating_sub(sent)
}

/// WEB 管理端已认证人员身份：由 WS token 绑定到该 admin 连接，用于远控 RBAC 闸 + 审计归属。
/// role 存 tier 字符串（`superadmin`/`user`），落审计/会话时直接写库。
/// 远控闸判定的权限来源为账户权限集 `permissions`（+ superadmin 隐式全权 `is_superadmin`），
/// 不再靠 role 字符串反解旧固定角色。
#[derive(Debug, Clone)]
pub struct ActorIdentity {
    pub user_id: String,
    pub username: String,
    /// 审计/会话 operator_role 落库的 tier 字符串（`superadmin`/`user`）。
    pub role: String,
    /// 该账户的菜单权限集（远控闸判定 use_remote 的真源；superadmin 为隐式全集）。
    pub permissions: PermissionSet,
    /// 是否 superadmin（隐式全权，远控闸直接放行）。
    pub is_superadmin: bool,
}

impl ActorIdentity {
    /// 从会话已存的 operator_* 字段重建身份，供会话生命周期审计（AuthResult/结束/断开）归属。
    /// 三列齐备才算有身份（agent 侧或匿名会话为 None）。
    ///
    /// 会话仅落 operator_* 三列（user_id/username/role），不存权限集——因本重建体**只用于审计归属**，
    /// 从不参与远控闸判定（闸只用 `actor_of` 拿到的连接绑定体，见 ConnectRequest）。故 `permissions`
    /// 置空、`is_superadmin` 由 role 字符串派生即可，对审计写入无影响。
    pub(crate) fn from_session(s: &protocol::Session) -> Option<ActorIdentity> {
        match (
            s.operator_user_id.as_ref(),
            s.operator_username.as_ref(),
            s.operator_role.as_ref(),
        ) {
            (Some(user_id), Some(username), Some(role)) => Some(ActorIdentity {
                user_id: user_id.clone(),
                username: username.clone(),
                is_superadmin: role == "superadmin",
                role: role.clone(),
                permissions: PermissionSet::default(),
            }),
            _ => None,
        }
    }
}

pub struct Hub {
    pub reg: Arc<Registry>,
    pub sessions: Arc<SessionStore>,
    pub audit: Arc<AuditStore>,
    /// endpoint_id / admin_id → 出站消息通道
    clients: DashMap<String, mpsc::UnboundedSender<String>>,
    /// 帧专用 lane（drop-stale）：endpoint_id/admin_id → 帧 watch + enqueued 计数（与 clients 并存）。
    frame_clients: DashMap<String, FrameClient>,
    /// admin 连接 id → 已认证人员身份（WS token 绑定）。仅 admin 连接登记，agent 不登记。
    actors: DashMap<String, ActorIdentity>,
}

impl Hub {
    pub fn new(reg: Arc<Registry>, sessions: Arc<SessionStore>, audit: Arc<AuditStore>) -> Self {
        Hub {
            reg,
            sessions,
            audit,
            clients: DashMap::new(),
            frame_clients: DashMap::new(),
            actors: DashMap::new(),
        }
    }

    /// 绑定 admin 连接的人员身份（WS 升级时按 token 解析出的 AuthUser 调用）。
    pub fn bind_actor(&self, conn_id: &str, actor: ActorIdentity) {
        self.actors.insert(conn_id.to_string(), actor);
    }

    /// 解绑（连接断开时调用）。
    pub fn remove_actor(&self, conn_id: &str) {
        self.actors.remove(conn_id);
    }

    /// 查连接绑定的人员身份（远控 RBAC 闸 + 直发型操作审计归属）。
    fn actor_of(&self, conn_id: &str) -> Option<ActorIdentity> {
        self.actors.get(conn_id).map(|a| a.clone())
    }

    pub fn add_client(&self, id: String, tx: mpsc::UnboundedSender<String>) {
        self.clients.insert(id, tx);
    }

    pub fn add_frame_client(
        &self,
        id: String,
        frame_tx: tokio::sync::watch::Sender<Option<String>>,
        enqueued: std::sync::Arc<AtomicU64>,
    ) {
        self.frame_clients.insert(id, FrameClient { tx: frame_tx, enqueued });
    }

    /// 帧定向推送（drop-stale）：覆盖目标的单槽最新帧；累加 enqueued（入队计数）。
    pub fn send_frame_to(&self, id: &str, json: &str) {
        if let Some(fc) = self.frame_clients.get(id) {
            fc.enqueued.fetch_add(1, Ordering::Relaxed);
            let _ = fc.tx.send_replace(Some(json.to_string()));
        }
    }

    pub fn remove_client(&self, id: &str) {
        self.clients.remove(id);
        self.frame_clients.remove(id);
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

    /// 按 session 对端路由：Frame/Input 上行 to:None，server 据 session_id 查对端转发。
    ///
    /// 路由失败（会话不存在 / from 不属于该会话 / 对端离线）一律 `warn!`，不再静默吞——
    /// 静默丢弃曾让「被控发聊天、主控收不到」极难定位（被控会话 id 漂移，详见
    /// docs/superpowers/specs/2026-07-01-controlled-chat-session-divergence-bug.md）。
    /// `raw` = 收到的原始 JSON 文本，原样转发(不重序列化)。这样 server 无需理解 payload
    /// 内容，未来新增的 InputEvent/Message 变体即便本 server 不认识(反序列化落 Unknown)，
    /// 也能把原始字节端到端透传给对端，新端仍可还原——协议演进不再破坏旧 server。
    fn route_to_peer(&self, session_id: &str, env: &Envelope, raw: &str) {
        let Some(peer) = self.sessions.peer_of(session_id, &env.from) else {
            tracing::warn!(
                "route_to_peer 丢弃: 查无对端(会话不存在或 from 不属于该会话) session={session_id} from={}",
                env.from
            );
            return;
        };
        if !self.clients.contains_key(&peer) {
            tracing::warn!(
                "route_to_peer 丢弃: 对端离线 session={session_id} from={} peer={peer}",
                env.from
            );
            return;
        }
        self.send_to(&peer, raw);
    }

    /// 客户端断开时结束其参与的所有活跃会话（修 orphan active 泄漏）。
    /// 对每条被移除的会话：向对端发 SessionEnd（清「被控/控制」态）+ audit.end_session
    /// 更新 DB 终态 + 落输入聚合审计 + 落 Disconnect「对端断开」审计。镜像 handle_session_end。
    pub async fn end_client_sessions(&self, client_id: &str, now: i64) {
        let ended = self
            .sessions
            .remove_sessions_of(client_id, now, protocol::SessionStatus::Ended);
        for (session, input_summary) in ended {
            let session_id = &session.id;
            // 通知对端（会话里 ≠ 断开方的一侧）：据此清除"正在被控/控制"态。
            let peer = if session.from_id == client_id {
                &session.to_id
            } else {
                &session.from_id
            };
            let end_env = Envelope {
                from: "server".into(),
                to: Some(peer.clone()),
                ts: now,
                payload: Message::SessionEnd {
                    session_id: session_id.clone(),
                },
            };
            if let Ok(json) = serde_json::to_string(&end_env) {
                self.send_to(peer, &json);
            }
            // 审计归属：从会话已存 operator_* 重建操作人身份。
            let actor = ActorIdentity::from_session(&session);
            // 落输入聚合审计（M-SRV4）
            self.audit
                .log(session_id, &session.from_id, AuditType::Input, &input_summary, actor.as_ref())
                .await;
            // 更新 DB 会话终态
            self.audit
                .end_session(session_id, now, protocol::SessionStatus::Ended)
                .await;
            // 落断开审计（对端断开）
            self.audit
                .log(session_id, &session.from_id, AuditType::Disconnect, "对端断开", actor.as_ref())
                .await;
        }
    }

    /// 处理一条入站信封；now 为秒级 Unix 时间戳
    /// 便捷入口(测试/内部)：重序列化 env 作为 raw。生产路径应走 [`handle_raw`] 传原始 text，
    /// 以保未知变体内容(见 route_to_peer)。已知变体重序列化与原文等价，测试无差异。
    pub async fn handle(&self, env: Envelope, now: i64) {
        let raw = serde_json::to_string(&env).unwrap_or_default();
        self.handle_raw(env, &raw, now).await;
    }

    pub async fn handle_raw(&self, env: Envelope, raw: &str, now: i64) {
        match &env.payload {
            // ── 注册（W0-1：回发 RegisterAck；刷新注册表 + 广播列表）──────────
            Message::Register { info, password } => {
                // 归属绑定（反自报伪造）：owner_id 由服务端从连接绑定身份（WS token→JWT 派生）注入，
                // **忽略 info 内任何自报归属**——EndpointInfo 结构上无 owner 字段，且此处只读连接身份。
                // 无 token 旧端 actor_of=None → owner=None（仅 superadmin 可见），不破坏现网。
                let owner = self.actor_of(&env.from).map(|a| a.user_id);
                self.reg.upsert(*info.clone(), password.clone(), now, owner);
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
                force,
            } => {
                let actor = self.actor_of(&env.from);
                // RBAC 闸：admin 来源发起远控须持有 UseRemote 权限（或为 superadmin 隐式全权）。
                // 无身份 / 账户权限集无 UseRemote 且非 superadmin → warn 并拒绝（不建会话），
                // 挡住无远控权限的账号越权发起（HTTP 已挡，WS 直连再挡一道）。
                // 权限来源已从旧固定角色映射改为账户权限集（`actor.permissions`），superadmin 直接放行。
                if env.from.starts_with("admin-") {
                    let allowed = actor
                        .as_ref()
                        .map(|a| {
                            a.permissions.contains(crate::users::Permission::UseRemote)
                                || a.is_superadmin
                        })
                        .unwrap_or(false);
                    if !allowed {
                        tracing::warn!(
                            "拒绝无 UseRemote 权限的 admin 远控发起: from={}",
                            env.from
                        );
                        return;
                    }
                }
                handlers::handle_connect_request(
                    self,
                    &env.from,
                    mode,
                    target,
                    password.as_deref(),
                    *force,
                    now,
                    actor.as_ref(),
                )
                .await;
            }

            // ── 主控取消挂起申请（委托 handlers：通知被控撤弹窗 + 结束会话）──────
            Message::CancelRequest { target } => {
                handlers::handle_cancel_request(self, &env.from, target, now).await;
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
                self.route_to_peer(session_id, &env, raw);
            }

            // ── 截图请求：仅认证 admin 可发；落审计 + 广播全 agent ────────────
            Message::ScreenshotReq { req_id } => {
                if !env.from.starts_with("admin-") {
                    tracing::warn!("拒绝非 admin 的截图请求: from={}", env.from);
                    return;
                }
                tracing::debug!("截图广播 req_id={req_id}");
                self.audit
                    .log(
                        req_id,
                        &env.from,
                        AuditType::Screenshot,
                        "批量截图指令",
                        self.actor_of(&env.from).as_ref(),
                    )
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
                self.route_to_peer(session_id, &env, raw);
                handlers::handle_session_end(self, session_id, now).await;
            }

            // ── Frame：走帧 lane（drop-stale），按 session 对端路由 ──────────────
            Message::Frame { session_id, .. } => {
                if let Some(peer) = self.sessions.peer_of(session_id, &env.from) {
                    if let Ok(json) = serde_json::to_string(&env) {
                        self.send_frame_to(&peer, &json);
                    }
                }
            }

            // ── RemoteNotice / SetQuality / SetCapture / Clipboard：可靠 control lane ──
            Message::RemoteNotice { session_id, .. }
            | Message::SetQuality { session_id, .. }
            | Message::SetCapture { session_id, .. }
            | Message::ClipboardSync { session_id, .. } => {
                self.route_to_peer(session_id, &env, raw);
            }

            // ── ConnectAck/Reject/ScreenshotResp：按 to 定向转发 ──────────────
            Message::ConnectAck { .. }
            | Message::Reject { .. }
            | Message::ScreenshotResp { .. } => {
                self.forward_by_to(&env);
            }

            // ── 远程命令执行：按 session 对端路由；ExecRequest 落 Command 审计 ──
            Message::ExecRequest {
                session_id,
                command,
                ..
            } => {
                let summary: String = command.chars().take(200).collect();
                self.audit
                    .log(
                        session_id,
                        &env.from,
                        AuditType::Command,
                        &format!("执行命令: {summary}"),
                        self.actor_of(&env.from).as_ref(),
                    )
                    .await;
                self.route_to_peer(session_id, &env, raw);
            }
            Message::ExecResult { session_id, .. } => {
                self.route_to_peer(session_id, &env, raw);
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
                        self.actor_of(&env.from).as_ref(),
                    )
                    .await;
                self.route_to_peer(session_id, &env, raw);
            }
            Message::FileChunk { session_id, .. }
            | Message::FilePullRequest { session_id, .. }
            | Message::FileError { session_id, .. }
            | Message::FileDone { session_id, .. }
            | Message::FileListResp { session_id, .. } => {
                self.route_to_peer(session_id, &env, raw);
            }

            // ── 远端目录浏览请求：按 session 路由；落 FileTransfer 审计 ──────────
            Message::FileListRequest {
                session_id, path, ..
            } => {
                self.audit
                    .log(
                        session_id,
                        &env.from,
                        AuditType::FileTransfer,
                        &format!("浏览目录: {path}"),
                        self.actor_of(&env.from).as_ref(),
                    )
                    .await;
                self.route_to_peer(session_id, &env, raw);
            }

            // ── 会话内即时消息:按 session 对端路由 + 落 Chat 审计(全文)──────────
            Message::ChatMessage {
                session_id, text, ..
            } => {
                self.audit
                    .log(
                        session_id,
                        &env.from,
                        AuditType::Chat,
                        text,
                        self.actor_of(&env.from).as_ref(),
                    )
                    .await;
                self.route_to_peer(session_id, &env, raw);
            }

            // server 单向发出的消息，不处理客户端发来的
            Message::RegisterAck { .. }
            | Message::EndpointList { .. }
            | Message::IncomingControl { .. } => {}

            // 未知/未来 Message 变体：本 server 不认识，无 session_id 可路由，安全忽略。
            // (端到端演进的新语义应走 InputEvent 变体，经 route_to_peer 原始转发透传。)
            Message::Unknown => {}
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
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
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

    /// Bug 回归（issue#4）：主控取消挂起申请 → server 必须把 SessionEnd 转发给被控端
    /// （撤销其授权弹窗），并移除会话；不得回发给主控自身。
    #[tokio::test]
    async fn cancel_request_notifies_controlled_and_ends_session() {
        let hub = test_hub();
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-ctrl".into(), ctrl_tx);
        hub.add_client("ep-victim".into(), victim_tx);

        let sid = "sess-pending".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-ctrl".into(),
            to_id: "ep-victim".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        // 主控发 CancelRequest（带 target=被控）
        let env = Envelope {
            from: "ep-ctrl".into(),
            to: None,
            ts: 200,
            payload: Message::CancelRequest {
                target: "ep-victim".into(),
            },
        };
        hub.handle(env, 200).await;

        // 被控端收到 SessionEnd（据此撤弹窗）
        let got = victim_rx
            .try_recv()
            .expect("被控端应收到 SessionEnd 以撤销授权弹窗");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::SessionEnd { session_id } => assert_eq!(session_id, sid),
            other => panic!("被控端收到的应为 SessionEnd，实际 {other:?}"),
        }
        // 会话已移除
        assert!(!hub.sessions.contains(&sid), "取消后会话应被移除");
        // 主控不应收到回发
        assert!(ctrl_rx.try_recv().is_err(), "取消通知不应回发给发起方主控");
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
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
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

    /// 远端目录浏览回归：FileListRequest 必须按 session 路由给被控端（落审计不 panic）。
    #[tokio::test]
    async fn file_list_request_forwarded_to_controlled_peer() {
        let hub = test_hub();
        let (_admin_tx, _admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-1".into(), _admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);

        let sid = "sess-ls".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::A,
            from_id: "admin-1".into(),
            to_id: "ep-victim".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        let env = Envelope {
            from: "admin-1".into(),
            to: None,
            ts: 200,
            payload: Message::FileListRequest {
                session_id: sid.clone(),
                transfer_id: "t-ls".into(),
                path: "/tmp".into(),
            },
        };
        hub.handle(env, 200).await;

        let got = victim_rx.try_recv().expect("被控端应收到 FileListRequest");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::FileListRequest { path, .. } => assert_eq!(path, "/tmp"),
            other => panic!("应为 FileListRequest，实际 {other:?}"),
        }
    }

    /// 即时消息:必须按 session 路由给对端,且落审计不 panic,不回发给发送方。
    #[tokio::test]
    async fn chat_message_forwarded_to_peer() {
        let hub = test_hub();
        let (a_tx, mut a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-chat".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        let env = Envelope {
            from: "ep-a".into(),
            to: None,
            ts: 200,
            payload: Message::ChatMessage {
                session_id: sid.clone(),
                msg_id: "m-1".into(),
                text: "你好".into(),
            },
        };
        hub.handle(env, 200).await;

        let got = b_rx.try_recv().expect("对端应收到 ChatMessage");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::ChatMessage { text, .. } => assert_eq!(text, "你好"),
            other => panic!("应为 ChatMessage,实际 {other:?}"),
        }
        assert!(a_rx.try_recv().is_err(), "不应回发给发送方");
    }

    /// Bug 回归（被控主动发聊天）：被控端(to_id=ep-victim)→主控端(from_id=admin-x) 方向的
    /// ChatMessage 必须正确投递给主控。坐证「服务端路由本身正确，漂移 bug 在客户端」——
    /// 只要被控带的是权威 session_id，route_to_peer→peer_of 就能查到 admin 对端并送达。
    #[tokio::test]
    async fn chat_from_controlled_to_admin_master_delivered() {
        let hub = test_hub();
        let (admin_tx, mut admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-x".into(), admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);

        let sid = "sess-ctrl-chat".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::A, // 管理后台强制远程（force = auto_accept），复现用户现场
            from_id: "admin-x".into(),
            to_id: "ep-victim".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        // 被控端（to_id）主动发聊天，带权威 session_id
        let env = Envelope {
            from: "ep-victim".into(),
            to: None,
            ts: 200,
            payload: Message::ChatMessage {
                session_id: sid.clone(),
                msg_id: "cm-1".into(),
                text: "被控发给主控".into(),
            },
        };
        hub.handle(env, 200).await;

        // 主控端（admin）必须收到该 chat
        let got = admin_rx
            .try_recv()
            .expect("主控端应收到被控发来的 ChatMessage");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::ChatMessage { text, .. } => assert_eq!(text, "被控发给主控"),
            other => panic!("应为 ChatMessage,实际 {other:?}"),
        }
        // 被控自己不应收到回发（它是发送方）
        assert!(victim_rx.try_recv().is_err(), "不应回发给发送方被控端");
    }

    /// 懒推流信号:SetCapture 必须按 session 路由给对端(被控端据此启停采集)。
    #[tokio::test]
    async fn set_capture_forwarded_to_peer() {
        let hub = test_hub();
        let (a_tx, _a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-cap".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        let env = Envelope {
            from: "ep-a".into(),
            to: None,
            ts: 200,
            payload: Message::SetCapture {
                session_id: sid.clone(),
                active: false,
            },
        };
        hub.handle(env, 200).await;

        let got = b_rx.try_recv().expect("对端应收到 SetCapture");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        assert!(matches!(
            env.payload,
            Message::SetCapture { active: false, .. }
        ));
    }

    /// 前向兼容根治回归：主控发来一个**本 server 不认识的** InputEvent 变体(模拟未来客户端
    /// 新增的手势),server 反序列化落 InputEvent::Unknown(不再整条失败),并按 session 路由、
    /// **原始 text 原样转发**——对端(新客户端)仍能收到含全部原始字段的 payload。
    /// 坐实:协议演进(加 event 变体)不再破坏旧 server,滚轮不通的根因不复发。
    #[tokio::test]
    async fn unknown_input_event_relayed_raw_preserving_fields() {
        let hub = test_hub();
        let (a_tx, _a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-unknown".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        // 未来变体:本 server 的 protocol 不含 "future_gesture",event 反序列化应落 Unknown。
        // payload 为嵌套对象(内部 tag "type" 在 payload 内)。
        let raw = format!(
            r#"{{"from":"ep-a","to":null,"ts":200,"payload":{{"type":"input","session_id":"{sid}","event":{{"kind":"future_gesture","magnitude":42}}}}}}"#
        );
        // 复现生产路径:先反序列化(此时不再整条失败),再 handle_raw 传原始 text。
        let env: Envelope = serde_json::from_str(&raw).expect("未知 event 变体不应导致整条失败");
        assert!(
            matches!(env.payload, Message::Input { event: protocol::InputEvent::Unknown, .. }),
            "未知 kind 应落 InputEvent::Unknown"
        );
        hub.handle_raw(env, &raw, 200).await;

        let got = b_rx.try_recv().expect("对端应收到被转发的未知 Input");
        assert!(
            got.contains("future_gesture") && got.contains("\"magnitude\":42"),
            "应原样透传原始字段,实际转发={got}"
        );
    }

    /// 泄漏根治回归：客户端断开时，须结束其参与的所有活跃会话——
    /// 向对端发 SessionEnd + 从 active_sessions 移除（修 orphan active）。
    #[tokio::test]
    async fn end_client_sessions_notifies_peer_and_removes() {
        let hub = test_hub();
        let (a_tx, mut a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);

        let sid = "sess-leak".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        // ep-a 断开：结束其所有会话
        hub.end_client_sessions("ep-a", 300).await;

        // 对端 ep-b 收到 SessionEnd
        let got = b_rx
            .try_recv()
            .expect("对端应收到 SessionEnd 结束通知");
        let env: Envelope = serde_json::from_str(&got).unwrap();
        match env.payload {
            Message::SessionEnd { session_id } => assert_eq!(session_id, sid),
            other => panic!("对端收到的应为 SessionEnd，实际 {other:?}"),
        }
        // 断开方自己不应收到回发
        assert!(a_rx.try_recv().is_err(), "结束通知不应回发给断开方自身");
        // 会话已从内存移除（不再 orphan active）
        assert!(
            hub.sessions.active_sessions().is_empty(),
            "断开后不应残留活跃会话"
        );
    }

    /// RBAC 闸回归：admin 连接未绑定身份 → 远控发起被拒（不建会话、被控端收不到 IncomingControl）。
    #[tokio::test]
    async fn connect_request_denied_when_admin_unbound() {
        let hub = test_hub();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-victim".into(), victim_tx);

        let env = Envelope {
            from: "admin-x".into(),
            to: None,
            ts: 200,
            payload: Message::ConnectRequest {
                mode: Mode::A,
                target: "ep-victim".into(),
                password: None,
                force: true,
            },
        };
        hub.handle(env, 200).await;

        assert!(
            victim_rx.try_recv().is_err(),
            "无身份 admin 发起应被拒，被控端不应收到 IncomingControl"
        );
        assert!(
            hub.sessions.active_sessions().is_empty(),
            "被拒不应建会话"
        );
    }

    /// RBAC 闸：绑定的账户权限集不含 use_remote(仅 view_audit)→ 远控发起被拒(不建会话)。
    /// 权限来源已从旧 role 映射改为账户权限集，故此处直接给 permissions 而非靠 role 字符串 parse。
    #[tokio::test]
    async fn connect_request_denied_when_actor_permissions_lack_use_remote() {
        let hub = test_hub();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-victim".into(), victim_tx);
        hub.bind_actor(
            "admin-x",
            ActorIdentity {
                user_id: "u-aud".into(),
                username: "auditor1".into(),
                role: "user".into(),
                permissions: crate::users::PermissionSet::parse("view_audit"),
                is_superadmin: false,
            },
        );

        let env = Envelope {
            from: "admin-x".into(),
            to: None,
            ts: 200,
            payload: Message::ConnectRequest {
                mode: Mode::A,
                target: "ep-victim".into(),
                password: None,
                force: true,
            },
        };
        hub.handle(env, 200).await;

        assert!(
            victim_rx.try_recv().is_err(),
            "账户权限集无 use_remote，被控端不应收到 IncomingControl"
        );
        assert!(hub.sessions.active_sessions().is_empty(), "被拒不应建会话");
    }

    /// RBAC 闸 + 归属：绑定的账户权限集含 use_remote → 允许远控，且操作人身份(tier=user)写入会话。
    #[tokio::test]
    async fn connect_request_allowed_when_actor_permissions_include_use_remote() {
        let hub = test_hub();
        let (admin_tx, _admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-x".into(), admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);
        hub.bind_actor(
            "admin-x",
            ActorIdentity {
                user_id: "u-op".into(),
                username: "operator1".into(),
                role: "user".into(),
                permissions: crate::users::PermissionSet::parse("view_assets,view_grid,use_remote"),
                is_superadmin: false,
            },
        );

        let env = Envelope {
            from: "admin-x".into(),
            to: None,
            ts: 200,
            payload: Message::ConnectRequest {
                mode: Mode::A,
                target: "ep-victim".into(),
                password: None,
                force: true,
            },
        };
        hub.handle(env, 200).await;

        // 被控端收到 IncomingControl（force 免同意 auto_accept=true）
        let got = victim_rx
            .try_recv()
            .expect("含 use_remote，被控端应收到 IncomingControl");
        let got_env: Envelope = serde_json::from_str(&got).unwrap();
        assert!(
            matches!(got_env.payload, Message::IncomingControl { auto_accept: true, .. }),
            "force 直连应 auto_accept=true"
        );

        // 会话写入操作人身份（operator_*，tier 写 user）
        let sessions = hub.sessions.active_sessions();
        assert_eq!(sessions.len(), 1, "应建 1 条会话");
        let s = &sessions[0];
        assert_eq!(s.operator_user_id.as_deref(), Some("u-op"));
        assert_eq!(s.operator_username.as_deref(), Some("operator1"));
        assert_eq!(s.operator_role.as_deref(), Some("user"));
    }

    /// RBAC 闸：superadmin(隐式全权)→ 放行远控。
    /// 刻意用空权限集 + is_superadmin=true 构造，单独锁定「superadmin 隐式放行不依赖权限集」这一契约
    ///（生产路径中 superadmin 的 permissions 实为 superadmin_all()，两条判定分支任一成立即放行）。
    #[tokio::test]
    async fn connect_request_allowed_for_superadmin() {
        let hub = test_hub();
        let (admin_tx, _admin_rx) = mpsc::unbounded_channel::<String>();
        let (victim_tx, mut victim_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("admin-x".into(), admin_tx);
        hub.add_client("ep-victim".into(), victim_tx);
        hub.bind_actor(
            "admin-x",
            ActorIdentity {
                user_id: "u-sa".into(),
                username: "superadmin".into(),
                role: "superadmin".into(),
                permissions: crate::users::PermissionSet::default(),
                is_superadmin: true,
            },
        );

        let env = Envelope {
            from: "admin-x".into(),
            to: None,
            ts: 200,
            payload: Message::ConnectRequest {
                mode: Mode::A,
                target: "ep-victim".into(),
                password: None,
                force: true,
            },
        };
        hub.handle(env, 200).await;

        let got = victim_rx
            .try_recv()
            .expect("superadmin 隐式全权，被控端应收到 IncomingControl");
        let got_env: Envelope = serde_json::from_str(&got).unwrap();
        assert!(matches!(
            got_env.payload,
            Message::IncomingControl { auto_accept: true, .. }
        ));

        let sessions = hub.sessions.active_sessions();
        assert_eq!(sessions.len(), 1, "应建 1 条会话");
        assert_eq!(sessions[0].operator_role.as_deref(), Some("superadmin"));
    }

    #[test]
    fn frame_lane_drop_计算() {
        assert_eq!(super::frame_lane_drop(5, 3), 2, "入5发3→丢2");
        assert_eq!(super::frame_lane_drop(3, 3), 0, "1:1→不丢(不过报)");
        assert_eq!(super::frame_lane_drop(2, 5), 0, "sent>enqueued(并发瞬态)→clamp 0");
    }

    /// 帧 lane(drop-stale):Frame 走独立 frame_clients,连发两帧只保留最新(coalesce)。
    #[tokio::test]
    async fn frame_routed_to_frame_lane_latest_wins() {
        let hub = test_hub();
        let (a_tx, _a_rx) = mpsc::unbounded_channel::<String>();
        let (b_tx, _b_rx) = mpsc::unbounded_channel::<String>();
        hub.add_client("ep-a".into(), a_tx);
        hub.add_client("ep-b".into(), b_tx);
        let (bf_tx, bf_rx) = tokio::sync::watch::channel::<Option<String>>(None);
        let enqueued = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        hub.add_frame_client("ep-b".into(), bf_tx, enqueued);

        let sid = "sess-f".to_string();
        hub.sessions.insert(Session {
            id: sid.clone(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 100,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        // ep-a 连发两帧（seq 0,1）→ frame lane 只保留最新
        for seq in 0u64..2 {
            let env = Envelope {
                from: "ep-a".into(),
                to: None,
                ts: 200,
                payload: Message::Frame {
                    session_id: sid.clone(),
                    data: format!("d{seq}"),
                    w: 1,
                    h: 1,
                    seq,
                },
            };
            hub.handle(env, 200).await;
        }
        let latest = bf_rx.borrow().clone().expect("帧 lane 应有最新帧");
        let env: Envelope = serde_json::from_str(&latest).unwrap();
        match env.payload {
            Message::Frame { seq, .. } => assert_eq!(seq, 1, "drop-stale：应保留最新 seq=1"),
            other => panic!("应为 Frame，实际 {other:?}"),
        }
    }

    /// T006【RED】归属绑定 + 反伪造（AC-002-H1 / AC-002-E1）：
    /// token=A 的被控端连接（conn_id=终端 id）发 Register → owner_id 由服务端从连接绑定身份
    /// （JWT 派生）注入为 A.id，**与 info 内容无关**（EndpointInfo 结构上无 owner 字段，客户端无从自报）。
    /// A 视图可见该端、B 视图不可见——越权归属被结构性阻断。
    #[tokio::test]
    async fn register_从连接身份派生_owner_忽略自报() {
        let hub = test_hub();
        // 模拟 token=A 的被控端连接：连接 id = 终端 id = ep-001，绑定操作人身份 A(ua)。
        hub.bind_actor(
            "ep-001",
            ActorIdentity {
                user_id: "ua".into(),
                username: "alice".into(),
                role: "user".into(),
                permissions: crate::users::PermissionSet::default(),
                is_superadmin: false,
            },
        );

        let env = Envelope {
            from: "ep-001".into(),
            to: None,
            ts: 100,
            payload: Message::Register {
                info: Box::new(protocol::EndpointInfo::sample()), // id=ep-001
                password: "123456".into(),
            },
        };
        hub.handle(env, 100).await;

        // owner_id 落库为连接身份 ua，而非任何 info 自报值。
        let sup = hub.reg.views_visible_to(100, None, true);
        assert_eq!(sup.len(), 1);
        assert_eq!(
            sup[0].owner_id.as_deref(),
            Some("ua"),
            "owner 必须源自连接 JWT 身份，而非客户端自报"
        );
        // 反越权：A 可见、B 不可见。
        assert_eq!(hub.reg.views_visible_to(100, Some("ua"), false).len(), 1);
        assert_eq!(hub.reg.views_visible_to(100, Some("ub"), false).len(), 0);
    }
}
