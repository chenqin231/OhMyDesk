//! 客户端活动状态边界：会话/发起中/替换窗口的唯一真相源。
//! main 持有 Arc 注入 update/ui；update 只读活动 + 占用替换窗口，不拥有远控生命周期。
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::SharedSession;

#[cfg_attr(not(windows), allow(dead_code))] // 字段仅被 windows 替换门控 + 单测读取
pub struct ClientActivityState {
    cur_session: SharedSession,   // 主控活动会话（既有 Arc，复用不另造）
    ctrl_session: SharedSession,  // 被控活动会话
    pending_until_ms: AtomicU64,  // 主控发起中截止时刻（自动过期，防卡死）
    updating: AtomicBool,         // 替换窗口
}

#[cfg_attr(not(windows), allow(dead_code))] // is_idle/try_enter_updating/exit_updating 仅 windows apply_auto + 单测用
impl ClientActivityState {
    pub fn new(cur_session: SharedSession, ctrl_session: SharedSession) -> Self {
        Self { cur_session, ctrl_session, pending_until_ms: AtomicU64::new(0), updating: AtomicBool::new(false) }
    }
    fn sessions_idle(&self) -> bool {
        self.cur_session.lock().unwrap().is_none() && self.ctrl_session.lock().unwrap().is_none()
    }
    /// 空闲 = 无主控/被控会话 且 不在发起中窗口。
    pub fn is_idle(&self, now_ms: u64) -> bool {
        self.sessions_idle() && now_ms >= self.pending_until_ms.load(Ordering::Acquire)
    }
    pub fn is_updating(&self) -> bool { self.updating.load(Ordering::Acquire) }
    /// 主控发起远控时置位，30s 后自动过期（防 ack 丢失永久卡死）。
    pub fn begin_pending_connect(&self, now_ms: u64) {
        self.pending_until_ms.store(now_ms + 30_000, Ordering::Release);
    }
    pub fn end_pending_connect(&self) { self.pending_until_ms.store(0, Ordering::Release); }
    /// 原子占用替换窗口：先抢 updating，再复检空闲；非空闲则退还。
    pub fn try_enter_updating(&self, now_ms: u64) -> bool {
        if self.updating.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        if self.is_idle(now_ms) {
            true
        } else {
            self.updating.store(false, Ordering::Release);
            false
        }
    }
    pub fn exit_updating(&self) { self.updating.store(false, Ordering::Release); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn st() -> (ClientActivityState, SharedSession, SharedSession) {
        let cur: SharedSession = Arc::new(Mutex::new(None));
        let ctrl: SharedSession = Arc::new(Mutex::new(None));
        (ClientActivityState::new(cur.clone(), ctrl.clone()), cur, ctrl)
    }

    #[test]
    fn 空闲_无会话无发起为真() {
        let (s, _, _) = st();
        assert!(s.is_idle(1000));
    }

    #[test]
    fn 会话占用则非空闲() {
        let (s, cur, _) = st();
        *cur.lock().unwrap() = Some("sess".into());
        assert!(!s.is_idle(1000));
    }

    #[test]
    fn 发起中窗口内非空闲_过期后空闲() {
        let (s, _, _) = st();
        s.begin_pending_connect(1000);
        assert!(!s.is_idle(1000));         // 未到期
        assert!(!s.is_idle(30000));        // 仍在 1000+30000 之内
        assert!(s.is_idle(31001));         // 已过期
        s.end_pending_connect();
        assert!(s.is_idle(1000));
    }

    #[test]
    fn 占用替换窗口_互斥与退还() {
        let (s, cur, _) = st();
        assert!(s.try_enter_updating(1000));   // 抢到
        assert!(s.is_updating());
        assert!(!s.try_enter_updating(1000));  // 二次抢不到
        s.exit_updating();
        // 有会话时抢不到（抢了会退还）
        *cur.lock().unwrap() = Some("x".into());
        assert!(!s.try_enter_updating(1000));
        assert!(!s.is_updating());             // 已退还
    }
}
