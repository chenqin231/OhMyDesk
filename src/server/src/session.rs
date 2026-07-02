//! 会话状态 + 输入聚合计数（M-SRV4）。
//! 纯逻辑，无 IO 依赖，可直接 TDD。

use dashmap::DashMap;
use protocol::{Mode, Session, SessionStatus};

// ── InputAggregator —— 一个会话内的输入操作计数 ───────────────────────────────

/// 累计输入事件次数，会话结束时输出审计摘要文本（M-SRV4）
pub struct InputAggregator {
    count: u64,
}

impl InputAggregator {
    pub fn new() -> Self {
        InputAggregator { count: 0 }
    }

    /// 每次转发一条 `Message::Input` 时调用
    pub fn bump(&mut self) {
        self.count += 1;
    }

    /// 生成审计落库的摘要文本
    pub fn summary(&self) -> String {
        format!("输入操作 {} 次", self.count)
    }

    #[allow(dead_code)]
    pub fn count(&self) -> u64 {
        self.count
    }
}

// ── SessionStore —— 进行中会话表 ────────────────────────────────────────────

/// 单个进行中会话；持 InputAggregator，会话结束时取 summary() 落库
pub struct ActiveSession {
    pub meta: Session,
    pub aggregator: InputAggregator,
}

/// 内存会话表（DashMap 保证并发安全）
pub struct SessionStore {
    sessions: DashMap<String, ActiveSession>,
}

impl SessionStore {
    pub fn new() -> Self {
        SessionStore {
            sessions: DashMap::new(),
        }
    }

    /// 插入新会话（ConnectRequest 鉴权通过后调用）
    pub fn insert(&self, session: Session) {
        self.sessions.insert(
            session.id.clone(),
            ActiveSession {
                meta: session,
                aggregator: InputAggregator::new(),
            },
        );
    }

    /// 返回会话的发起方 id（handle_auth_result 用于定位主控推 ConnectAck）
    pub fn initiator_of(&self, session_id: &str) -> Option<String> {
        self.sessions
            .get(session_id)
            .map(|s| s.meta.from_id.clone())
    }

    /// 对某会话的输入计数器 +1（M-SRV4）
    pub fn bump_input(&self, session_id: &str) {
        if let Some(mut s) = self.sessions.get_mut(session_id) {
            s.aggregator.bump();
        }
    }

    /// 结束会话，返回 (Session, 输入摘要)，供审计落库
    pub fn end_session(
        &self,
        session_id: &str,
        now: i64,
        status: SessionStatus,
    ) -> Option<(Session, String)> {
        let (_, mut active) = self.sessions.remove(session_id)?;
        active.meta.end_at = Some(now);
        active.meta.status = status;
        let summary = active.aggregator.summary();
        Some((active.meta, summary))
    }

    /// 获取所有活跃会话快照（用于 /api/sessions HTTP 查询）
    #[allow(dead_code)]
    pub fn active_sessions(&self) -> Vec<Session> {
        self.sessions.iter().map(|e| e.meta.clone()).collect()
    }

    /// 查某会话是否存在（路由/鉴权时用）
    #[allow(dead_code)]
    pub fn contains(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// 找由 `from_id` 发起、指向 `target` 的进行中会话 id（主控取消挂起申请时定位）。
    /// 同一对理论上仅一条挂起会话；存在多条时返回任意一条（取消语义对哪条无差别）。
    pub fn outbound_session(&self, from_id: &str, target: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|e| e.meta.from_id == from_id && e.meta.to_id == target)
            .map(|e| e.meta.id.clone())
    }

    /// 移除并返回该客户端参与（作 from_id 或 to_id）的所有会话，供断开时批量结束。
    /// 返回 `Vec<(Session, 输入摘要)>`，每条 set end_at=now/status，供审计落库 + 通知对端。
    /// 先收集要删的 key 再删，避免 DashMap 迭代中删的借用冲突。
    pub fn remove_sessions_of(
        &self,
        client_id: &str,
        now: i64,
        status: SessionStatus,
    ) -> Vec<(Session, String)> {
        let keys: Vec<String> = self
            .sessions
            .iter()
            .filter(|e| e.meta.from_id == client_id || e.meta.to_id == client_id)
            .map(|e| e.meta.id.clone())
            .collect();
        keys.into_iter()
            .filter_map(|k| self.end_session(&k, now, status))
            .collect()
    }

    /// 返回会话中 sender 的对端 id（Frame/Input 按 session 路由）：
    /// sender=主控(from_id) → 被控(to_id)；sender=被控(to_id) → 主控(from_id)
    pub fn peer_of(&self, session_id: &str, sender: &str) -> Option<String> {
        let s = self.sessions.get(session_id)?;
        if s.meta.from_id == sender {
            Some(s.meta.to_id.clone())
        } else if s.meta.to_id == sender {
            Some(s.meta.from_id.clone())
        } else {
            None
        }
    }
}

/// 生成会话的发起端→被控端 key（用于查找由哪条连接发起）
#[allow(dead_code)]
pub fn make_mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::A => "A",
        Mode::B => "B",
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_events_aggregate_count() {
        let mut agg = InputAggregator::new();
        for _ in 0..47 {
            agg.bump();
        }
        assert_eq!(agg.summary(), "输入操作 47 次");
    }

    #[test]
    fn input_aggregator_zero() {
        let agg = InputAggregator::new();
        assert_eq!(agg.summary(), "输入操作 0 次");
    }

    #[test]
    fn session_store_insert_and_bump() {
        let store = SessionStore::new();
        let sess = Session {
            id: "s-001".into(),
            mode: Mode::B,
            from_id: "ep-002".into(),
            to_id: "ep-001".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        };
        store.insert(sess);
        for _ in 0..10 {
            store.bump_input("s-001");
        }
        let (ended, summary) = store.end_session("s-001", 100, SessionStatus::Ended).unwrap();
        assert_eq!(ended.status, SessionStatus::Ended);
        assert_eq!(ended.end_at, Some(100));
        assert_eq!(summary, "输入操作 10 次");
    }

    #[test]
    fn session_store_end_nonexistent_returns_none() {
        let store = SessionStore::new();
        assert!(store.end_session("nonexistent", 0, SessionStatus::Ended).is_none());
    }

    #[test]
    fn initiator_of_returns_correct_from() {
        let store = SessionStore::new();
        let sess = Session {
            id: "s-002".into(),
            mode: Mode::A,
            from_id: "admin-1".into(),
            to_id: "ep-001".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        };
        store.insert(sess);
        assert_eq!(store.initiator_of("s-002"), Some("admin-1".into()));
        assert_eq!(store.initiator_of("nonexistent"), None);
    }

    #[test]
    fn outbound_session_finds_pending_by_from_and_target() {
        let store = SessionStore::new();
        store.insert(Session {
            id: "s-out".into(),
            mode: Mode::B,
            from_id: "ep-a".into(),
            to_id: "ep-b".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });
        assert_eq!(store.outbound_session("ep-a", "ep-b"), Some("s-out".into()));
        assert_eq!(store.outbound_session("ep-a", "ep-x"), None, "target 不符不应命中");
        assert_eq!(store.outbound_session("ep-x", "ep-b"), None, "发起方不符不应命中");
        // 结束后不再命中
        store.end_session("s-out", 10, SessionStatus::Ended);
        assert_eq!(store.outbound_session("ep-a", "ep-b"), None);
    }

    #[test]
    fn remove_sessions_of_only_removes_participating() {
        let store = SessionStore::new();
        // client 作 from
        store.insert(Session {
            id: "s-from".into(),
            mode: Mode::A,
            from_id: "ep-x".into(),
            to_id: "ep-1".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });
        // client 作 to
        store.insert(Session {
            id: "s-to".into(),
            mode: Mode::B,
            from_id: "ep-2".into(),
            to_id: "ep-x".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });
        // 无关会话
        store.insert(Session {
            id: "s-other".into(),
            mode: Mode::B,
            from_id: "ep-3".into(),
            to_id: "ep-4".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        });

        let mut removed = store.remove_sessions_of("ep-x", 500, SessionStatus::Ended);
        assert_eq!(removed.len(), 2, "应仅移除 ep-x 参与的 2 条会话");
        // 每条已置 end_at/status
        for (sess, summary) in &removed {
            assert_eq!(sess.end_at, Some(500));
            assert_eq!(sess.status, SessionStatus::Ended);
            assert_eq!(summary, "输入操作 0 次");
        }
        // 返回的会话 id 恰为两条参与会话
        removed.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        assert_eq!(removed[0].0.id, "s-from");
        assert_eq!(removed[1].0.id, "s-to");

        // 无关会话仍在册
        let remaining = store.active_sessions();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "s-other");
    }

    #[test]
    fn initiator_of_returns_none_after_end() {
        let store = SessionStore::new();
        let sess = Session {
            id: "s-003".into(),
            mode: Mode::B,
            from_id: "ep-002".into(),
            to_id: "ep-001".into(),
            start_at: 0,
            end_at: None,
            status: SessionStatus::Active,
            operator_user_id: None,
            operator_username: None,
            operator_role: None,
        };
        store.insert(sess);
        store.end_session("s-003", 100, SessionStatus::Ended);
        // 会话已移除，initiator_of 应返回 None
        assert_eq!(store.initiator_of("s-003"), None);
    }
}
