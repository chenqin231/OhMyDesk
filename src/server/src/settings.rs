//! 管理凭据持久化：MySQL `settings` 表（key-value）。
//! 无 DB 时 load 返回 None（用写死默认）、save 为 no-op（改密本次生效，重启复位）。

use crate::db::Db;

pub struct SettingsStore {
    db: Option<Db>,
}

impl SettingsStore {
    pub fn new(db: Option<Db>) -> Self {
        SettingsStore { db }
    }

    /// 读持久化的 (admin_user, admin_pass_hash)；任一缺失返回 None（回退默认凭据）。
    pub async fn load_credential(&self) -> Option<(String, String)> {
        let db = self.db.as_ref()?;
        let user = get(db, "admin_user").await?;
        let hash = get(db, "admin_pass_hash").await?;
        Some((user, hash))
    }

    /// 落库新凭据（best-effort，失败仅告警）。
    pub async fn save_credential(&self, user: &str, pass_hash: &str) {
        let Some(db) = &self.db else {
            tracing::warn!("无 DB，凭据改动未持久化（重启复位）");
            return;
        };
        put(db, "admin_user", user).await;
        put(db, "admin_pass_hash", pass_hash).await;
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
