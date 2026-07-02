//! 审计存储：会话/审计日志落库。
//! AuditStore 持 Option<Db>（M-SRV1），None 时 no-op + 告警 log。

use protocol::{AuditLog, AuditType, Session, SessionStatus};
use uuid::Uuid;

use crate::db::Db;

pub struct AuditStore {
    db: Option<Db>,
}

impl AuditStore {
    pub fn new(db: Option<Db>) -> Self {
        AuditStore { db }
    }

    // ── 私有辅助 ────────────────────────────────────────────────────────────

    /// 生成 UUID
    fn new_id() -> String {
        Uuid::new_v4().to_string()
    }

    /// 当前秒级时间戳
    fn now_sec() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    // ── 公开接口 ─────────────────────────────────────────────────────────────

    /// 写一条 audit_log（B-DB1：列名 event_type；C-1：枚举含 input）
    /// `actor_id` = 发起该操作的连接 id（admin-/endpoint id）；`actor` = 该连接绑定的 WEB
    /// 人员身份（Task5），有则写入 actor_user_id/username/role 三列，无则留空（agent 侧操作）。
    pub async fn log(
        &self,
        session_id: &str,
        actor_id: &str,
        kind: AuditType,
        text: &str,
        actor: Option<&crate::hub::ActorIdentity>,
    ) {
        let Some(db) = &self.db else {
            tracing::warn!("审计降级（M-SRV1），跳过写入: kind={kind:?} text={text}");
            return;
        };
        let kind_str = audit_type_str(kind);
        let id = Self::new_id();
        let ts = Self::now_sec();
        if let Err(e) = sqlx::query(
            "INSERT INTO audit_logs (id, session_id, ts, actor_id, actor_user_id, actor_username, actor_role, event_type, text) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(ts)
        .bind(actor_id)
        .bind(actor.map(|a| a.user_id.clone()))
        .bind(actor.map(|a| a.username.clone()))
        .bind(actor.map(|a| a.role.clone()))
        .bind(kind_str)
        .bind(text)
        .execute(db)
        .await
        {
            tracing::warn!("审计写入失败（best-effort 跳过）: {e}");
        }
    }

    /// 写一条 sessions 记录（会话建立时）
    pub async fn insert_session(&self, session: &Session) {
        let Some(db) = &self.db else {
            tracing::warn!("审计降级（M-SRV1），跳过写 session: {}", session.id);
            return;
        };
        let mode_str = mode_str(session.mode);
        if let Err(e) = sqlx::query(
            "INSERT INTO sessions (id, mode, from_id, to_id, start_at, end_at, status, operator_user_id, operator_username, operator_role) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&session.id)
        .bind(mode_str)
        .bind(&session.from_id)
        .bind(&session.to_id)
        .bind(session.start_at)
        .bind(session.end_at)
        .bind(status_str(session.status))
        .bind(&session.operator_user_id)
        .bind(&session.operator_username)
        .bind(&session.operator_role)
        .execute(db)
        .await
        {
            tracing::warn!("sessions 写入失败（best-effort 跳过）: {e}");
        }
    }

    /// 更新会话终态（会话结束时）
    pub async fn end_session(&self, session_id: &str, end_at: i64, status: SessionStatus) {
        let Some(db) = &self.db else {
            tracing::warn!("审计降级（M-SRV1），跳过更新 session: {session_id}");
            return;
        };
        if let Err(e) = sqlx::query("UPDATE sessions SET end_at = ?, status = ? WHERE id = ?")
            .bind(end_at)
            .bind(status_str(status))
            .bind(session_id)
            .execute(db)
            .await
        {
            tracing::warn!("sessions 更新失败（best-effort 跳过）: {e}");
        }
    }

    // ── HTTP 查询接口（/api/audit, /api/sessions）────────────────────────────

    /// 查询审计日志（供 http.rs `/api/audit`）
    pub async fn query_audit(
        &self,
        endpoint: Option<&str>,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
    ) -> Vec<AuditLog> {
        let Some(db) = &self.db else {
            return vec![];
        };
        // 动态构造带可选过滤的 SQL（不用 ORM，保持简单）
        let mut sql = String::from(
            "SELECT id, session_id, ts, actor_id, actor_user_id, actor_username, actor_role, event_type, text FROM audit_logs WHERE 1=1",
        );
        if endpoint.is_some() {
            sql.push_str(" AND actor_id = ?");
        }
        if from_ts.is_some() {
            sql.push_str(" AND ts >= ?");
        }
        if to_ts.is_some() {
            sql.push_str(" AND ts <= ?");
        }
        sql.push_str(" ORDER BY ts DESC LIMIT 500");

        let mut q = sqlx::query_as::<_, AuditLogRow>(&sql);
        if let Some(ep) = endpoint {
            q = q.bind(ep);
        }
        if let Some(ft) = from_ts {
            q = q.bind(ft);
        }
        if let Some(tt) = to_ts {
            q = q.bind(tt);
        }

        match q.fetch_all(db).await {
            Ok(rows) => rows.into_iter().map(AuditLog::from).collect(),
            Err(e) => {
                tracing::warn!("查询 audit_logs 失败: {e}");
                vec![]
            }
        }
    }

    /// 查询历史会话（供 http.rs `/api/sessions`）
    pub async fn query_sessions(&self) -> Vec<Session> {
        let Some(db) = &self.db else {
            return vec![];
        };
        match sqlx::query_as::<_, SessionRow>(
            "SELECT id, mode, from_id, to_id, start_at, end_at, status, operator_user_id, operator_username, operator_role FROM sessions ORDER BY start_at DESC LIMIT 200",
        )
        .fetch_all(db)
        .await
        {
            Ok(rows) => rows.into_iter().filter_map(session_from_row).collect(),
            Err(e) => {
                tracing::warn!("查询 sessions 失败: {e}");
                vec![]
            }
        }
    }
}

// ── 辅助转换 ─────────────────────────────────────────────────────────────────

fn audit_type_str(t: AuditType) -> &'static str {
    match t {
        AuditType::Connect => "connect",
        AuditType::AuthFail => "auth_fail",
        AuditType::Reject => "reject",
        AuditType::Screenshot => "screenshot",
        AuditType::Input => "input",
        AuditType::Disconnect => "disconnect",
        AuditType::Command => "command",
        AuditType::FileTransfer => "file_transfer",
        AuditType::Chat => "chat",
    }
}

#[allow(dead_code)]
fn mode_str(m: protocol::Mode) -> &'static str {
    match m {
        protocol::Mode::A => "A",
        protocol::Mode::B => "B",
    }
}

fn status_str(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Active => "active",
        SessionStatus::Ended => "ended",
        SessionStatus::Rejected => "rejected",
    }
}

// ── sqlx 行映射结构 ──────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct AuditLogRow {
    id: String,
    session_id: String,
    ts: i64,
    actor_id: String,
    actor_user_id: Option<String>,
    actor_username: Option<String>,
    actor_role: Option<String>,
    event_type: String,
    text: String,
}

impl From<AuditLogRow> for AuditLog {
    fn from(r: AuditLogRow) -> Self {
        let kind = match r.event_type.as_str() {
            "connect" => AuditType::Connect,
            "auth_fail" => AuditType::AuthFail,
            "reject" => AuditType::Reject,
            "screenshot" => AuditType::Screenshot,
            "input" => AuditType::Input,
            "command" => AuditType::Command,
            "file_transfer" => AuditType::FileTransfer,
            "chat" => AuditType::Chat,
            _ => AuditType::Disconnect,
        };
        AuditLog {
            id: r.id,
            session_id: r.session_id,
            ts: r.ts,
            actor_id: r.actor_id,
            actor_user_id: r.actor_user_id,
            actor_username: r.actor_username,
            actor_role: r.actor_role,
            kind,
            text: r.text,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    mode: String,
    from_id: String,
    to_id: String,
    start_at: i64,
    end_at: Option<i64>,
    status: String,
    operator_user_id: Option<String>,
    operator_username: Option<String>,
    operator_role: Option<String>,
}

fn session_from_row(r: SessionRow) -> Option<Session> {
    let mode = match r.mode.as_str() {
        "A" => protocol::Mode::A,
        "B" => protocol::Mode::B,
        _ => return None,
    };
    let status = match r.status.as_str() {
        "active" => SessionStatus::Active,
        "ended" => SessionStatus::Ended,
        "rejected" => SessionStatus::Rejected,
        _ => return None,
    };
    Some(Session {
        id: r.id,
        mode,
        from_id: r.from_id,
        to_id: r.to_id,
        start_at: r.start_at,
        end_at: r.end_at,
        status,
        operator_user_id: r.operator_user_id,
        operator_username: r.operator_username,
        operator_role: r.operator_role,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const AUDIT_DDL: &str = r#"
CREATE TABLE audit_logs (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  ts INTEGER NOT NULL,
  actor_id TEXT NOT NULL,
  actor_user_id TEXT,
  actor_username TEXT,
  actor_role TEXT,
  event_type TEXT NOT NULL,
  text TEXT NOT NULL
)
"#;

    async fn audit_store() -> AuditStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(AUDIT_DDL).execute(&pool).await.unwrap();
        AuditStore::new(Some(pool))
    }

    /// 归属回归：带 actor 落审计 → actor_user_id/username/role 三列写入，可回查。
    #[tokio::test]
    async fn log_with_actor_persists_actor_identity_columns() {
        let store = audit_store().await;
        let actor = crate::hub::ActorIdentity {
            user_id: "u-1".into(),
            username: "alice".into(),
            role: "user".into(),
            permissions: crate::users::PermissionSet::parse("view_assets,use_remote"),
            is_superadmin: false,
        };
        store
            .log("sess-1", "admin-1", AuditType::Connect, "会话建立", Some(&actor))
            .await;

        let logs = store.query_audit(None, None, None).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].actor_user_id.as_deref(), Some("u-1"));
        assert_eq!(logs[0].actor_username.as_deref(), Some("alice"));
        assert_eq!(logs[0].actor_role.as_deref(), Some("user"));
    }

    /// 无 actor（agent 侧操作）→ actor_* 三列留空。
    #[tokio::test]
    async fn log_without_actor_leaves_identity_columns_null() {
        let store = audit_store().await;
        store
            .log("sess-2", "ep-9", AuditType::Chat, "hi", None)
            .await;

        let logs = store.query_audit(None, None, None).await;
        assert_eq!(logs.len(), 1);
        assert!(logs[0].actor_user_id.is_none());
        assert!(logs[0].actor_username.is_none());
        assert!(logs[0].actor_role.is_none());
    }

    #[test]
    fn chat_audit_type_str_and_back() {
        // 枚举 → 字符串
        assert_eq!(audit_type_str(AuditType::Chat), "chat");
        // 字符串行 → 枚举(往返)
        let row = AuditLogRow {
            id: "a1".into(),
            session_id: "s1".into(),
            ts: 0,
            actor_id: "ep-1".into(),
            actor_user_id: None,
            actor_username: None,
            actor_role: None,
            event_type: "chat".into(),
            text: "你好".into(),
        };
        let log = AuditLog::from(row);
        assert!(matches!(log.kind, AuditType::Chat));
        assert_eq!(log.text, "你好");
    }
}
