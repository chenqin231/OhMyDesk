//! SQLite 连接池（竞赛部署：零外部依赖，单文件持久化）。
//! M-SRV1：连接失败降级为 Option<Db>=None，实时链路（注册/心跳/远控）不依赖 DB。
//!
//! 默认在工作目录创建 `ohmydesk.db`；`DATABASE_URL` 可覆盖，例如容器部署：
//!   DATABASE_URL=sqlite:/app/data/ohmydesk.db （挂数据卷持久化）。

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::Row;

pub type Db = sqlx::SqlitePool;

/// 连接 SQLite：文件不存在自动创建（WAL 模式提升并发），建表 DDL 幂等执行。
/// 任一步失败 → 返回 None（审计 best-effort 跳过），不影响实时链路。
pub async fn connect() -> Option<Db> {
    // DATABASE_URL 覆盖；缺省用工作目录下 ohmydesk.db（避免 URL 相对路径歧义）。
    let opts = match std::env::var("DATABASE_URL") {
        Ok(url) => SqliteConnectOptions::from_str(&url),
        Err(_) => Ok(SqliteConnectOptions::new().filename("ohmydesk.db")),
    };
    let opts = match opts {
        Ok(o) => o
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5)),
        Err(e) => {
            tracing::warn!("DATABASE_URL 解析失败，审计存储已降级（M-SRV1）: {e}");
            return None;
        }
    };

    let pool = match SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("SQLite 连接失败，审计存储已降级（M-SRV1）: {e}");
            return None;
        }
    };

    // 执行建表脚本（幂等 IF NOT EXISTS）
    let ddl = include_str!("../../../scripts/db/schema.sqlite.sql");
    if let Err(e) = sqlx::raw_sql(ddl).execute(&pool).await {
        tracing::warn!("建表失败，审计存储已降级（M-SRV1）: {e}");
        return None;
    }
    if let Err(e) = ensure_identity_columns(&pool).await {
        tracing::warn!("补齐审计身份列失败，审计存储已降级（M-SRV1）: {e}");
        return None;
    }

    tracing::info!("SQLite 就绪，审计存储已启用");
    Some(pool)
}

async fn ensure_identity_columns(pool: &Db) -> sqlx::Result<()> {
    add_column_if_missing(
        pool,
        "sessions",
        "operator_user_id",
        "ALTER TABLE sessions ADD COLUMN operator_user_id TEXT",
    )
    .await?;
    add_column_if_missing(
        pool,
        "sessions",
        "operator_username",
        "ALTER TABLE sessions ADD COLUMN operator_username TEXT",
    )
    .await?;
    add_column_if_missing(
        pool,
        "sessions",
        "operator_role",
        "ALTER TABLE sessions ADD COLUMN operator_role TEXT",
    )
    .await?;
    add_column_if_missing(
        pool,
        "audit_logs",
        "actor_user_id",
        "ALTER TABLE audit_logs ADD COLUMN actor_user_id TEXT",
    )
    .await?;
    add_column_if_missing(
        pool,
        "audit_logs",
        "actor_username",
        "ALTER TABLE audit_logs ADD COLUMN actor_username TEXT",
    )
    .await?;
    add_column_if_missing(
        pool,
        "audit_logs",
        "actor_role",
        "ALTER TABLE audit_logs ADD COLUMN actor_role TEXT",
    )
    .await
}

async fn add_column_if_missing(
    pool: &Db,
    table: &str,
    column: &str,
    ddl: &str,
) -> sqlx::Result<()> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    let exists = rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column);
    if !exists {
        sqlx::query(ddl).execute(pool).await?;
    }
    Ok(())
}
