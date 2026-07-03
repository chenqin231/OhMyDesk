//! 登录日志存储：管理员登录痕迹落库（IP/UA/时间/成败）。仿 audit::AuditStore。
//! 持 Option<Db>，无库时 no-op + 告警（与审计一致，不阻断登录）。

use protocol::LoginLogEntry;

use crate::db::Db;

pub struct LoginLogStore {
    db: Option<Db>,
}

impl LoginLogStore {
    pub fn new(db: Option<Db>) -> Self {
        LoginLogStore { db }
    }

    fn now_sec() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// 写一条登录日志（best-effort）。username/ua 截断防滥用（失败登录可携带任意串）。
    pub async fn record(
        &self,
        username: &str,
        ip: Option<&str>,
        user_agent: Option<&str>,
        success: bool,
        reason: Option<&str>,
    ) {
        let Some(db) = &self.db else {
            tracing::warn!("登录日志降级，跳过写入: user={username} success={success}");
            return;
        };
        let user = truncate(username, 128);
        let ua = user_agent.map(|s| truncate(s, 512));
        let ts = Self::now_sec();
        if let Err(e) = sqlx::query(
            "INSERT INTO login_log (ts, username, ip, user_agent, success, reason) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(&user)
        .bind(ip)
        .bind(ua)
        .bind(success as i64)
        .bind(reason)
        .execute(db)
        .await
        {
            tracing::warn!("登录日志写入失败（best-effort 跳过）: {e}");
        }
    }

    /// 分页查询（按 ts 倒序）。limit 钳制 [1,200]，offset >=0。
    /// `username_scope`：None=superadmin 全量；Some(name)=仅该账号自己的登录记录
    /// （登录日志无终端归属，按 username 过滤——普通账号只看自己的登录历史）。
    pub async fn query(
        &self,
        limit: i64,
        offset: i64,
        username_scope: Option<&str>,
    ) -> Vec<LoginLogEntry> {
        let Some(db) = &self.db else {
            return vec![];
        };
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let mut sql = String::from(
            "SELECT id, ts, username, ip, user_agent, success, reason FROM login_log",
        );
        if username_scope.is_some() {
            sql.push_str(" WHERE username = ?");
        }
        sql.push_str(" ORDER BY ts DESC LIMIT ? OFFSET ?");

        let mut q = sqlx::query_as::<_, LoginLogRow>(&sql);
        if let Some(u) = username_scope {
            q = q.bind(u);
        }
        match q.bind(limit).bind(offset).fetch_all(db).await {
            Ok(rows) => rows.into_iter().map(LoginLogEntry::from).collect(),
            Err(e) => {
                tracing::warn!("查询 login_log 失败: {e}");
                vec![]
            }
        }
    }
}

/// 按字符（非字节）截断，避免中文 UA 切坏。
fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(sqlx::FromRow)]
struct LoginLogRow {
    id: i64,
    ts: i64,
    username: String,
    ip: Option<String>,
    user_agent: Option<String>,
    success: i64,
    reason: Option<String>,
}

impl From<LoginLogRow> for LoginLogEntry {
    fn from(r: LoginLogRow) -> Self {
        LoginLogEntry {
            id: r.id,
            ts: r.ts,
            username: r.username,
            ip: r.ip,
            user_agent: r.user_agent,
            success: r.success != 0,
            reason: r.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_chars_and_limits() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("一二三四", 2), "一二");
    }

    #[test]
    fn row_to_entry_success_flag() {
        let row = LoginLogRow {
            id: 7,
            ts: 100,
            username: "admin".into(),
            ip: Some("1.2.3.4".into()),
            user_agent: None,
            success: 1,
            reason: None,
        };
        let e = LoginLogEntry::from(row);
        assert_eq!(e.id, 7);
        assert!(e.success);
        assert_eq!(e.ip.as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn record_no_db_is_noop() {
        // 无 DB 时 record 不 panic（best-effort 降级）
        let store = LoginLogStore::new(None);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(store.record("admin", Some("1.1.1.1"), None, true, None));
        rt.block_on(async {
            assert!(store.query(50, 0, None).await.is_empty());
        });
    }

    /// T018：登录日志按 username 隔离——普通账号只看自己的登录记录；superadmin 全量。
    #[tokio::test]
    async fn query_按username隔离() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE login_log (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, \
             username TEXT NOT NULL, ip TEXT, user_agent TEXT, success INTEGER NOT NULL, reason TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = LoginLogStore::new(Some(pool));
        store.record("alice", Some("1.1.1.1"), None, true, None).await;
        store
            .record("alice", Some("1.1.1.1"), None, false, Some("密码错"))
            .await;
        store.record("bob", Some("2.2.2.2"), None, true, None).await;

        // alice 仅见自己 2 条。
        let a = store.query(50, 0, Some("alice")).await;
        assert_eq!(a.len(), 2, "普通账号只看自己登录记录");
        assert!(a.iter().all(|e| e.username == "alice"));
        // superadmin 全量 3 条。
        assert_eq!(store.query(50, 0, None).await.len(), 3);
    }
}
