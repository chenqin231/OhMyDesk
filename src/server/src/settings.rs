//! 旧管理凭据读取：SQLite `settings` 表（key-value）。
//! 启动时读取旧 `admin_user/admin_pass_hash`，作为 users 表 bootstrap 迁移输入。

use crate::db::Db;

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

    /// 旧版凭据写入接口保留给兼容路径；新登录系统以 users 表为准。
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
