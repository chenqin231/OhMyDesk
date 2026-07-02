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
    // users 表按账户权限模型迁移（role→tier + permissions 列）；幂等，现网每次启动都会跑。
    if let Err(e) = migrate_users_to_per_account_permissions(&pool).await {
        tracing::warn!("迁移 users 为按账户权限模型失败，审计存储已降级（M-SRV1）: {e}");
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

/// 判断 users 表是否已有指定列（迁移幂等判据）。
async fn users_has_column(pool: &Db, col: &str) -> sqlx::Result<bool> {
    let rows = sqlx::query("PRAGMA table_info(users)")
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|row| row.get::<String, _>("name") == col))
}

/// 旧固定角色 → 按账户菜单权限键串。
/// 仅迁移期使用；运行期权限一律走 users.permissions（superadmin 隐式全权，无需存储）。
fn perms_for_legacy_role(role: &str) -> &'static str {
    match role {
        // admin 迁为拥有全部可配菜单的普通账户（含 manage_assets，但账户管理 manage_users 归 superadmin 独占）
        "admin" => "view_assets,manage_assets,view_grid,use_remote,view_audit,view_login_logs",
        "operator" => "view_assets,view_grid,use_remote",
        "auditor" => "view_audit,view_login_logs",
        _ => "",
    }
}

/// 把 users 表从「固定 4 角色」迁移为「tier(superadmin/user) + 按账户 permissions 列」。
///
/// 幂等：permissions 列已存在（全新库 schema 已是新版，或已迁移过）直接跳过。
/// SQLite 无法 ALTER 既有 CHECK，故建新表 users_new（新 CHECK + permissions）→ 拷贝映射数据
/// → DROP 旧表 → RENAME，全程单事务原子提交，失败自动回滚不留半态。
/// role 映射：superadmin→superadmin（permissions 空）；admin/operator/auditor→user（按旧角色 backfill 菜单键）。
pub(crate) async fn migrate_users_to_per_account_permissions(pool: &Db) -> sqlx::Result<()> {
    // users 表不存在（异常/极早期）或已含 permissions 列（全新库/已迁移）→ 跳过
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'")
            .fetch_one(pool)
            .await?;
    if exists == 0 {
        return Ok(());
    }
    if users_has_column(pool, "permissions").await? {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    // DROP IF EXISTS：防御上一次非原子中断遗留的 users_new（正常情况下事务回滚不会留残表）。
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS users_new;\
         CREATE TABLE users_new (\
           id TEXT PRIMARY KEY,\
           username TEXT NOT NULL UNIQUE,\
           password_hash TEXT NOT NULL,\
           role TEXT NOT NULL CHECK(role IN ('superadmin','user')),\
           permissions TEXT NOT NULL DEFAULT '',\
           enabled INTEGER NOT NULL DEFAULT 1,\
           created_at INTEGER NOT NULL,\
           updated_at INTEGER NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;

    let rows = sqlx::query(
        "SELECT id, username, password_hash, role, enabled, created_at, updated_at FROM users",
    )
    .fetch_all(&mut *tx)
    .await?;
    for r in rows {
        let role: String = r.get("role");
        let (tier, perms) = if role == "superadmin" {
            ("superadmin", String::new())
        } else {
            ("user", perms_for_legacy_role(&role).to_string())
        };
        sqlx::query(
            "INSERT INTO users_new \
             (id, username, password_hash, role, permissions, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.get::<String, _>("id"))
        .bind(r.get::<String, _>("username"))
        .bind(r.get::<String, _>("password_hash"))
        .bind(tier)
        .bind(perms)
        .bind(r.get::<i64, _>("enabled"))
        .bind(r.get::<i64, _>("created_at"))
        .bind(r.get::<i64, _>("updated_at"))
        .execute(&mut *tx)
        .await?;
    }

    sqlx::raw_sql("DROP TABLE users; ALTER TABLE users_new RENAME TO users;")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!("users 表已迁移为按账户权限模型（role→tier + permissions 列）");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 现网旧 users 建表（4 固定角色 CHECK、无 permissions 列），迁移测试的输入 fixture。
    const OLD_USERS_DDL: &str = "\
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'admin', 'operator', 'auditor')),
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)";

    async fn new_memory_pool() -> Db {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migrate_users_maps_roles_to_tier_and_backfills_permissions_idempotently() {
        let pool = new_memory_pool().await;
        sqlx::raw_sql(OLD_USERS_DDL).execute(&pool).await.unwrap();
        for (u, r) in [
            ("superadmin", "superadmin"),
            ("admin", "admin"),
            ("op", "operator"),
            ("aud", "auditor"),
        ] {
            sqlx::query(
                "INSERT INTO users(id, username, password_hash, role, enabled, created_at, updated_at) \
                 VALUES(?, ?, ?, ?, 1, 0, 0)",
            )
            .bind(u)
            .bind(u)
            .bind("h")
            .bind(r)
            .execute(&pool)
            .await
            .unwrap();
        }

        migrate_users_to_per_account_permissions(&pool).await.unwrap();

        // permissions 列已建
        assert!(users_has_column(&pool, "permissions").await.unwrap());

        // superadmin：tier=superadmin，permissions 空（隐式全权）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='superadmin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "superadmin");
        assert_eq!(perms, "");

        // admin → user + 全功能（含 manage_assets，不含 manage_users）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='admin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "user");
        assert!(
            perms.contains("view_assets")
                && perms.contains("manage_assets")
                && perms.contains("view_grid")
                && perms.contains("use_remote")
                && perms.contains("view_audit")
                && perms.contains("view_login_logs")
        );
        assert!(!perms.contains("manage_users"));

        // operator → user + view_assets,view_grid,use_remote（不含 view_audit / manage_assets）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='op'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "user");
        assert!(
            perms.contains("view_assets")
                && perms.contains("view_grid")
                && perms.contains("use_remote")
        );
        assert!(!perms.contains("view_audit"));
        assert!(!perms.contains("manage_assets"));

        // auditor → user + view_audit,view_login_logs（不含 use_remote）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='aud'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "user");
        assert!(perms.contains("view_audit") && perms.contains("view_login_logs"));
        assert!(!perms.contains("use_remote"));

        // 幂等：再跑一次不报错、行数不变、数据不变
        migrate_users_to_per_account_permissions(&pool).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 4);
        let (role2, perms2): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='admin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role2, "user");
        assert!(perms2.contains("manage_assets") && !perms2.contains("manage_users"));
    }

    #[tokio::test]
    async fn migrate_users_is_noop_on_fresh_schema_with_permissions_column() {
        // 全新库：users 已是新版（含 permissions 列）→ 迁移直接跳过、不报错、不改数据
        let pool = new_memory_pool().await;
        sqlx::raw_sql(
            "CREATE TABLE users (
               id TEXT PRIMARY KEY,
               username TEXT NOT NULL UNIQUE,
               password_hash TEXT NOT NULL,
               role TEXT NOT NULL CHECK(role IN ('superadmin','user')),
               permissions TEXT NOT NULL DEFAULT '',
               enabled INTEGER NOT NULL DEFAULT 1,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO users(id, username, password_hash, role, permissions, enabled, created_at, updated_at) \
             VALUES('1', 'superadmin', 'h', 'superadmin', '', 1, 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        migrate_users_to_per_account_permissions(&pool).await.unwrap();

        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='superadmin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "superadmin");
        assert_eq!(perms, "");
    }

    #[tokio::test]
    async fn migrate_users_is_noop_when_users_table_absent() {
        // users 表不存在（异常/极早期）→ 迁移应静默返回 Ok，不报错
        let pool = new_memory_pool().await;
        migrate_users_to_per_account_permissions(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn ensure_identity_columns_migrates_old_schema_idempotently() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                mode TEXT,
                from_id TEXT,
                to_id TEXT,
                start_at INTEGER,
                end_at INTEGER,
                status TEXT
            );
            CREATE TABLE audit_logs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                ts INTEGER,
                actor_id TEXT,
                event_type TEXT,
                text TEXT
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        ensure_identity_columns(&pool).await.unwrap();

        let session_columns = table_columns(&pool, "sessions").await;
        assert!(session_columns.contains("operator_user_id"));
        assert!(session_columns.contains("operator_username"));
        assert!(session_columns.contains("operator_role"));

        let audit_columns = table_columns(&pool, "audit_logs").await;
        assert!(audit_columns.contains("actor_user_id"));
        assert!(audit_columns.contains("actor_username"));
        assert!(audit_columns.contains("actor_role"));

        ensure_identity_columns(&pool).await.unwrap();
    }

    async fn table_columns(pool: &Db, table: &str) -> HashSet<String> {
        sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get("name"))
            .collect()
    }
}
