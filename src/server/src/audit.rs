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
    pub async fn log(&self, session_id: &str, actor_id: &str, kind: AuditType, text: &str) {
        let Some(db) = &self.db else {
            tracing::warn!("审计降级（M-SRV1），跳过写入: kind={kind:?} text={text}");
            return;
        };
        let kind_str = audit_type_str(kind);
        let id = Self::new_id();
        let ts = Self::now_sec();
        if let Err(e) = sqlx::query(
            "INSERT INTO audit_logs (id, session_id, ts, actor_id, event_type, text) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(ts)
        .bind(actor_id)
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
            "INSERT INTO sessions (id, mode, from_id, to_id, start_at, end_at, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&session.id)
        .bind(mode_str)
        .bind(&session.from_id)
        .bind(&session.to_id)
        .bind(session.start_at)
        .bind(session.end_at)
        .bind(status_str(session.status))
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
        if let Err(e) = sqlx::query(
            "UPDATE sessions SET end_at = ?, status = ? WHERE id = ?",
        )
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
            "SELECT id, session_id, ts, actor_id, event_type, text FROM audit_logs WHERE 1=1",
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
            "SELECT id, mode, from_id, to_id, start_at, end_at, status FROM sessions ORDER BY start_at DESC LIMIT 200",
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
            _ => AuditType::Disconnect,
        };
        AuditLog {
            id: r.id,
            session_id: r.session_id,
            ts: r.ts,
            actor_id: r.actor_id,
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
    })
}
