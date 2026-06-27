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
        };
        store.insert(sess);
        assert_eq!(store.initiator_of("s-002"), Some("admin-1".into()));
        assert_eq!(store.initiator_of("nonexistent"), None);
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
        };
        store.insert(sess);
        store.end_session("s-003", 100, SessionStatus::Ended);
        // 会话已移除，initiator_of 应返回 None
        assert_eq!(store.initiator_of("s-003"), None);
    }
}
