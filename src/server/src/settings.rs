//! 旧管理凭据读取：SQLite `settings` 表（key-value）。
//! 启动时读取旧 `admin_user/admin_pass_hash`，作为 users 表 bootstrap 迁移输入。

use crate::db::Db;

const ADMIN_CREDENTIAL_MIGRATED: &str = "admin_credential_migrated";

pub struct SettingsStore {
    db: Option<Db>,
}

impl SettingsStore {
    pub fn new(db: Option<Db>) -> Self {
        SettingsStore { db }
    }

    /// 读旧版持久化的 (admin_user, admin_pass_hash)；任一缺失返回 None。
    pub async fn load_credential(&self) -> Option<(String, String)> {
        let db = self.db.as_ref()?;
        let user = get(db, "admin_user").await?;
        let hash = get(db, "admin_pass_hash").await?;
        Some((user, hash))
    }

    /// 仅在旧凭据尚未完成迁移时读取，避免 users 改名/改密后被旧 settings 复活。
    pub async fn load_legacy_credential_for_migration(&self) -> Option<(String, String)> {
        let db = self.db.as_ref()?;
        if get(db, ADMIN_CREDENTIAL_MIGRATED).await.as_deref() == Some("1") {
            return None;
        }
        self.load_credential().await
    }

    /// 标记旧凭据已迁移，并清理旧键，后续启动不再读取 `admin_user/admin_pass_hash`。
    pub async fn mark_credential_migrated(&self) {
        let Some(db) = &self.db else {
            tracing::warn!("无 DB，旧管理凭据迁移标记未持久化");
            return;
        };
        put(db, ADMIN_CREDENTIAL_MIGRATED, "1").await;
        delete(db, "admin_user").await;
        delete(db, "admin_pass_hash").await;
    }
}

async fn get(db: &Db, k: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT v FROM settings WHERE k = ?")
        .bind(k)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

async fn put(db: &Db, k: &str, v: &str) {
    if let Err(e) = sqlx::query(
        "INSERT INTO settings (k, v) VALUES (?, ?) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
    )
    .bind(k)
    .bind(v)
    .execute(db)
    .await
    {
        tracing::warn!("settings 写入失败（best-effort 跳过）: {e}");
    }
}

async fn delete(db: &Db, k: &str) {
    if let Err(e) = sqlx::query("DELETE FROM settings WHERE k = ?")
        .bind(k)
        .execute(db)
        .await
    {
        tracing::warn!("settings 删除失败（best-effort 跳过）: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::UserStore;
    use sqlx::sqlite::SqlitePoolOptions;

    const SETTINGS_DDL: &str = r#"
CREATE TABLE settings (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
)
"#;

    const USERS_DDL: &str = r#"
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'admin', 'operator', 'auditor')),
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
"#;

    async fn test_db() -> Db {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(SETTINGS_DDL).execute(&pool).await.unwrap();
        sqlx::raw_sql(USERS_DDL).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn legacy_migration_marker_prevents_reading_old_credentials_and_deletes_old_keys() {
        let db = test_db().await;
        let settings = SettingsStore::new(Some(db.clone()));

        put(&db, "admin_user", "legacy_admin").await;
        put(&db, "admin_pass_hash", "legacy_hash").await;

        assert_eq!(
            settings.load_legacy_credential_for_migration().await,
            Some(("legacy_admin".to_string(), "legacy_hash".to_string()))
        );

        settings.mark_credential_migrated().await;

        assert_eq!(settings.load_legacy_credential_for_migration().await, None);
        assert_eq!(settings.load_credential().await, None);
    }

    #[tokio::test]
    async fn legacy_credential_for_migration_returns_none_when_old_keys_are_missing() {
        let db = test_db().await;
        let settings = SettingsStore::new(Some(db));

        assert_eq!(settings.load_legacy_credential_for_migration().await, None);
    }

    #[tokio::test]
    async fn marked_legacy_migration_does_not_resurrect_old_admin_after_restart() {
        let db = test_db().await;
        let settings = SettingsStore::new(Some(db.clone()));
        let users = UserStore::new(db);
        let legacy_hash = crate::auth::hash_password("legacy-pass");

        if let Some(db) = settings.db.as_ref() {
            put(db, "admin_user", "legacy_admin").await;
            put(db, "admin_pass_hash", &legacy_hash).await;
        }
        let legacy = settings.load_legacy_credential_for_migration().await;
        users.bootstrap(legacy).await.unwrap();
        settings.mark_credential_migrated().await;

        let legacy_admin = users
            .get_by_username("legacy_admin")
            .await
            .unwrap()
            .unwrap();
        users
            .change_credential(
                &legacy_admin.id,
                "legacy-pass",
                Some("renamed_admin"),
                Some("rotated-pass"),
            )
            .await
            .unwrap();

        let second_start_legacy = settings.load_legacy_credential_for_migration().await;
        assert_eq!(second_start_legacy, None);
        users.bootstrap(second_start_legacy).await.unwrap();

        assert!(users
            .get_by_username("legacy_admin")
            .await
            .unwrap()
            .is_none());
        let renamed = users
            .get_by_username("renamed_admin")
            .await
            .unwrap()
            .unwrap();
        assert!(bcrypt::verify("rotated-pass", &renamed.password_hash).unwrap());
        assert!(!bcrypt::verify("legacy-pass", &renamed.password_hash).unwrap());
    }
}
