//! 终端注册表：DashMap 内存存储 + 在线超时判定 + SQLite 持久化。
//!
//! 内存判定逻辑（在线超时/视图/密码校验）纯逻辑、不依赖 IO，TDD 可直接跑。
//! 持久化（修复「服务器重启/升级后终端列表为空」）：构造时可注入 `Db`，upsert/remove 同步落库
//! （fire-and-forget，不阻塞实时链路），启动调 [`Registry::load_from_db`] 回灌——重启后终端先以
//! 离线态恢复，agent 重连重注册后转在线。无 `Db`（db=None / 单测）时全部退化为纯内存，零副作用。

use dashmap::DashMap;
use protocol::{xinchuang_label, EndpointInfo, EndpointView};

use crate::db::Db;

/// 超过此秒数未收到心跳视为离线
const ONLINE_TIMEOUT_SEC: i64 = 15;

struct Entry {
    info: EndpointInfo,
    password: String,
    last_seen: i64,
    /// 归属账号 user_id（服务端从连接 JWT 派生）。None=无归属旧端，仅 superadmin 可见。
    owner: Option<String>,
}

pub struct Registry {
    map: DashMap<String, Entry>,
    /// 持久化后端（None 时纯内存，不落库）。
    db: Option<Db>,
}

impl Registry {
    /// 纯内存构造（单测用；生产走 [`Registry::with_db`]）。
    #[allow(dead_code)]
    pub fn new() -> Self {
        Registry {
            map: DashMap::new(),
            db: None,
        }
    }

    /// 带持久化后端构造（生产用）。db=None 时与 [`Registry::new`] 等价（纯内存）。
    pub fn with_db(db: Option<Db>) -> Self {
        Registry {
            map: DashMap::new(),
            db,
        }
    }

    /// 启动时从 DB 回灌终端（恢复为离线态；agent 重连后转在线）。db=None 时空操作。
    ///
    /// `now` 为当前秒级时间戳：回灌项的 last_seen 会被钳到「至少早于在线阈值」，确保即使停机
    /// 时间 < ONLINE_TIMEOUT_SEC 也一律先呈现离线（避免误判刚回灌、实际未重连的终端为在线）。
    /// password 不回灌（DB 不存密码）：离线终端不可被控，待 agent 重连重注册再填真实密码。
    pub async fn load_from_db(&self, now: i64) {
        let db = match &self.db {
            Some(d) => d,
            None => return,
        };
        let offline_cap = now - ONLINE_TIMEOUT_SEC - 1; // 强制回灌项落在离线区间
        match db_load_all(db).await {
            Ok(rows) => {
                for (info, last_seen, owner) in rows {
                    self.map.insert(
                        info.id.clone(),
                        Entry {
                            info,
                            password: String::new(),
                            last_seen: last_seen.min(offline_cap),
                            owner,
                        },
                    );
                }
                tracing::info!("终端注册表已从 DB 恢复 {} 项（均为离线态）", self.map.len());
            }
            Err(e) => tracing::warn!("终端注册表回灌失败（降级为空表）：{e}"),
        }
    }

    /// 注册或更新终端信息；now 为秒级 Unix 时间戳。
    /// `owner` 为服务端从连接 JWT 派生的归属账号 user_id（无 token 旧端为 None）。
    /// 换绑语义：后一次 upsert 覆盖归属（一机多账户接管，非并发多归属）。
    pub fn upsert(&self, info: EndpointInfo, password: String, now: i64, owner: Option<String>) {
        // 持久化（best-effort，仅在有 db 时；fire-and-forget 不阻塞）。先序列化再移动 info 进表。
        // 不落 password（见 load_from_db / schema 注释）：只持久化终端身份、最后可见时间与归属。
        if let Some(db) = self.db.clone() {
            if let Ok(info_json) = serde_json::to_string(&info) {
                let id = info.id.clone();
                let owner_db = owner.clone();
                tokio::spawn(async move {
                    if let Err(e) = db_save(&db, &id, &info_json, now, owner_db.as_deref()).await {
                        tracing::warn!("终端落库失败 id={id}：{e}");
                    }
                });
            }
        }
        self.map.insert(
            info.id.clone(),
            Entry {
                info,
                password,
                last_seen: now,
                owner,
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
                    owner_id: e.owner.clone(),
                }
            })
            .collect()
    }

    /// 按归属过滤的视图快照（行级数据隔离核心）。
    ///
    /// - `viewer`=当前登录账号 user_id（服务端从 JWT 派生，非自报）。
    /// - `is_superadmin`=true → 全量（含 owner=None 旧端），绕过过滤。
    /// - 否则仅返回 `owner == Some(viewer)` 的终端；owner=None 旧端对普通账号不可见。
    ///
    /// 安全不变量：viewer=None 且非 superadmin（异常连接）→ 返回空集，绝不泄露。
    pub fn views_visible_to(
        &self,
        now: i64,
        viewer: Option<&str>,
        is_superadmin: bool,
    ) -> Vec<EndpointView> {
        self.views(now)
            .into_iter()
            .filter(|v| is_superadmin || (viewer.is_some() && v.owner_id.as_deref() == viewer))
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
        // 同步删库（best-effort，fire-and-forget），避免重启后被删终端「复活」。
        if let Some(db) = self.db.clone() {
            let id2 = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = db_delete(&db, &id2).await {
                    tracing::warn!("终端删库失败 id={id2}：{e}");
                }
            });
        }
        self.map.remove(id).is_some()
    }
}

// ── SQLite 持久化（自由函数，便于单测直接 await，避开 fire-and-forget 竞态）─────────────

/// upsert 一条终端（info 为 EndpointInfo 的 JSON）。不存密码。
/// owner_id 随 upsert 覆盖（换绑：后写覆盖前写）。
async fn db_save(
    db: &Db,
    id: &str,
    info_json: &str,
    last_seen: i64,
    owner_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO endpoint_registry(id, info, last_seen, owner_id) VALUES(?,?,?,?) \
         ON CONFLICT(id) DO UPDATE SET info=excluded.info, last_seen=excluded.last_seen, \
         owner_id=excluded.owner_id",
    )
    .bind(id)
    .bind(info_json)
    .bind(last_seen)
    .bind(owner_id)
    .execute(db)
    .await?;
    Ok(())
}

/// 删除一条终端。
async fn db_delete(db: &Db, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM endpoint_registry WHERE id=?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// 读取全部终端 → (EndpointInfo, last_seen, owner_id)。跳过 JSON 解析失败的脏行。
async fn db_load_all(db: &Db) -> anyhow::Result<Vec<(EndpointInfo, i64, Option<String>)>> {
    let rows: Vec<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT info, last_seen, owner_id FROM endpoint_registry")
            .fetch_all(db)
            .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (info_json, last_seen, owner_id) in rows {
        match serde_json::from_str::<EndpointInfo>(&info_json) {
            Ok(info) => out.push((info, last_seen, owner_id)),
            Err(e) => tracing::warn!("终端记录 JSON 解析失败，跳过：{e}"),
        }
    }
    Ok(out)
}

// ── 单元测试（TDD 红绿步骤） ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::EndpointInfo;

    #[test]
    fn upsert_and_view() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000, None);
        let views = reg.views(1000);
        assert_eq!(views.len(), 1);
        assert!(views[0].online);
        assert_eq!(views[0].xinchuang, "信创·麒麟·龙芯");
    }

    #[test]
    fn offline_after_timeout() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000, None);
        // now 比 last_seen 晚 16s，超过 15s 阈值
        let views = reg.views(1016);
        assert!(!views[0].online);
    }

    #[test]
    fn touch_refreshes_online() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000, None);
        reg.touch("ep-001", 1016);
        let views = reg.views(1016);
        assert!(views[0].online);
    }

    #[test]
    fn remove_删除终端记录() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 1000, None);
        assert_eq!(reg.views(1000).len(), 1);
        assert!(reg.remove("ep-001"), "删除已存在终端返回 true");
        assert_eq!(reg.views(1000).len(), 0, "删除后列表为空");
        assert!(!reg.remove("ep-001"), "重复删除返回 false");
        assert!(!reg.remove("nonexist"), "删除不存在终端返回 false");
    }

    #[test]
    fn mode_b_password_check() {
        let reg = Registry::new();
        reg.upsert(EndpointInfo::sample(), "123456".into(), 0, None);
        assert!(reg.check_password("ep-001", "123456"));
        assert!(!reg.check_password("ep-001", "000000"));
        assert!(!reg.check_password("nonexist", "123456"));
    }

    /// 回归（修复「服务器重启后终端列表为空」）：终端落库后，新注册表能从同一 DB 回灌，
    /// 视图非空——即重启后列表不再清零。同时验证：回灌项一律离线、且不带密码（安全）。
    #[tokio::test]
    async fn 持久化_落库后新注册表可回灌() {
        // 内存 SQLite + 建表（与生产 schema 等价的 endpoint_registry 子集，无 password 列）
        let db: Db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE endpoint_registry (id TEXT PRIMARY KEY, info TEXT NOT NULL, \
             last_seen INTEGER NOT NULL, owner_id TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();

        // 直接走自由函数落库（避开 upsert 内 fire-and-forget 的竞态）。
        // last_seen=1000 模拟「曾在线」；回灌时刻 now=1005（停机仅 5s < 15s 阈值）。
        let info = EndpointInfo::sample();
        let info_json = serde_json::to_string(&info).unwrap();
        super::db_save(&db, &info.id, &info_json, 1000, None)
            .await
            .unwrap();

        // 模拟服务器重启：全新注册表从同一 DB 回灌
        let reg2 = Registry::with_db(Some(db.clone()));
        reg2.load_from_db(1005).await;

        let views = reg2.views(1005);
        assert_eq!(views.len(), 1, "回灌后终端列表不应为空");
        assert_eq!(views[0].info.id, "ep-001");
        // 即便停机 < 15s，回灌项也必须呈现离线（last_seen 被钳到离线区间）。
        assert!(!views[0].online, "回灌项应一律离线，待 agent 重连才转在线");
        // 安全：不回灌密码，离线终端不可被模式 B 鉴权（待重连重注册再填）。
        assert!(!reg2.check_password("ep-001", "123456"), "回灌后不应带密码");

        // 删除后再回灌应为空（验证 db_delete）
        super::db_delete(&db, "ep-001").await.unwrap();
        let reg3 = Registry::with_db(Some(db.clone()));
        reg3.load_from_db(1005).await;
        assert_eq!(reg3.views(1005).len(), 0, "删除后回灌应为空");
    }

    /// 构造带自定义 id 的 EndpointInfo（owner 可见性测试用多台终端）。
    fn ep(id: &str) -> EndpointInfo {
        let mut e = EndpointInfo::sample();
        e.id = id.to_string();
        e
    }

    fn sorted_ids(vs: Vec<EndpointView>) -> Vec<String> {
        let mut v: Vec<String> = vs.into_iter().map(|x| x.info.id).collect();
        v.sort();
        v
    }

    /// T002【RED】owner 可见性过滤：普通账号仅见自己归属；superadmin 全量（含 owner=None）。
    #[test]
    fn owner_可见性过滤() {
        let reg = Registry::new();
        reg.upsert(ep("ep-a"), "pw".into(), 1000, Some("A".to_string()));
        reg.upsert(ep("ep-b"), "pw".into(), 1000, Some("B".to_string()));
        reg.upsert(ep("ep-n"), "pw".into(), 1000, None);

        // A 普通账号：仅见自己归属 ep-a
        assert_eq!(sorted_ids(reg.views_visible_to(1000, Some("A"), false)), vec!["ep-a"]);
        // B 普通账号：仅见 ep-b
        assert_eq!(sorted_ids(reg.views_visible_to(1000, Some("B"), false)), vec!["ep-b"]);
        // superadmin：全量（含 owner=None 旧端）
        assert_eq!(
            sorted_ids(reg.views_visible_to(1000, None, true)),
            vec!["ep-a", "ep-b", "ep-n"]
        );
        // A 不含 owner=None 的旧端
        assert!(!reg
            .views_visible_to(1000, Some("A"), false)
            .iter()
            .any(|v| v.info.id == "ep-n"));
        // owner_id 落在视图字段上
        let a_view = reg.views_visible_to(1000, Some("A"), false);
        assert_eq!(a_view[0].owner_id.as_deref(), Some("A"));
    }

    /// T002【RED】落库→回灌保留 owner_id（历史归属可追溯；换绑语义基石）。
    #[tokio::test]
    async fn 落库回灌保留_owner_id() {
        let db: Db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE endpoint_registry (id TEXT PRIMARY KEY, info TEXT NOT NULL, \
             last_seen INTEGER NOT NULL, owner_id TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();

        let info = EndpointInfo::sample();
        let info_json = serde_json::to_string(&info).unwrap();
        super::db_save(&db, &info.id, &info_json, 1000, Some("A"))
            .await
            .unwrap();

        let reg2 = Registry::with_db(Some(db.clone()));
        reg2.load_from_db(1005).await;

        // superadmin 视角回灌 1 项，owner_id 保留
        let sup = reg2.views_visible_to(1005, None, true);
        assert_eq!(sup.len(), 1);
        assert_eq!(sup[0].owner_id.as_deref(), Some("A"));
        // 普通账号 A 仍可见该离线机（归属未随重启丢失）
        assert_eq!(reg2.views_visible_to(1005, Some("A"), false).len(), 1);
    }
}
