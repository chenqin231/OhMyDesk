//! 内存终端注册表：DashMap 存储 + 在线超时判定。
//! 纯逻辑，不依赖 IO，TDD 可直接跑。

use dashmap::DashMap;
use protocol::{xinchuang_label, EndpointInfo, EndpointView};

/// 超过此秒数未收到心跳视为离线
const ONLINE_TIMEOUT_SEC: i64 = 15;

struct Entry {
    info: EndpointInfo,
    password: String,
    last_seen: i64,
}

pub struct Registry {
    map: DashMap<String, Entry>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            map: DashMap::new(),
        }
    }

    /// 注册或更新终端信息；now 为秒级 Unix 时间戳
    pub fn upsert(&self, info: EndpointInfo, password: String, now: i64) {
        self.map.insert(
            info.id.clone(),
            Entry {
                info,
                password,
                last_seen: now,
            },
        );
    }

    /// 心跳刷新最后可见时间
    pub fn touch(&self, id: &str, now: i64) {
        if let Some(mut e) = self.map.get_mut(id) {
            e.last_seen = now;
        }
    }

    /// 校验 endpoint 密码（模式 B 鉴权）
    pub fn check_password(&self, id: &str, pw: &str) -> bool {
        self.map
            .get(id)
            .map(|e| e.password == pw)
            .unwrap_or(false)
    }

    /// 返回所有终端的视图快照；now 用于判断在线态
    pub fn views(&self, now: i64) -> Vec<EndpointView> {
        self.map
            .iter()
            .map(|e| {
                let online = now - e.last_seen <= ONLINE_TIMEOUT_SEC;
                EndpointView {
                    info: e.info.clone(),
                    online,
                    last_seen: e.last_seen,
                    xinchuang: xinchuang_label(&e.info.os, &e.info.cpu),
                }
            })
            .collect()
    }

    /// 获取某个 endpoint 的 EndpointInfo（HTTP /api/endpoints 按 id 查）
    #[allow(dead_code)]
    pub fn get_info(&self, id: &str) -> Option<EndpointInfo> {
        self.map.get(id).map(|e| e.info.clone())
    }

    /// 删除终端记录（管理端手动清理离线/冗余）。返回是否存在并删除。
    /// 注意：删除在线 agent 后，其心跳 touch 不会重建（仅刷新已存在项）；下次重连 Register 才会重新出现。
    pub fn remove(&self, id: &str) -> bool {
        self.map.remove(id).is_some()
    }
}

// ── 单元测试（TDD 红绿步骤） ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::EndpointInfo;

    #[test]
    fn upsert_and_view() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        let views = reg.views(1000);
        assert_eq!(views.len(), 1);
        assert!(views[0].online);
        assert_eq!(views[0].xinchuang, "信创·麒麟·龙芯");
    }

    #[test]
    fn offline_after_timeout() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        // now 比 last_seen 晚 16s，超过 15s 阈值
        let views = reg.views(1016);
        assert!(!views[0].online);
    }

    #[test]
    fn touch_refreshes_online() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        reg.touch("ep-001", 1016);
        let views = reg.views(1016);
        assert!(views[0].online);
    }

    #[test]
    fn remove_删除终端记录() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000);
        assert_eq!(reg.views(1000).len(), 1);
        assert!(reg.remove("ep-001"), "删除已存在终端返回 true");
        assert_eq!(reg.views(1000).len(), 0, "删除后列表为空");
        assert!(!reg.remove("ep-001"), "重复删除返回 false");
        assert!(!reg.remove("nonexist"), "删除不存在终端返回 false");
    }

    #[test]
    fn mode_b_password_check() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 0);
        assert!(reg.check_password("ep-001", "123456"));
        assert!(!reg.check_password("ep-001", "000000"));
        assert!(!reg.check_password("nonexist", "123456"));
    }
}
