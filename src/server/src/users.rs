//! 管理端用户、固定角色与权限仓储。

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::db::Db;

#[allow(dead_code)]
const ALL_PERMISSIONS: &[Permission] = &[
    Permission::ViewAssets,
    Permission::ViewGrid,
    Permission::UseRemote,
    Permission::ViewAudit,
    Permission::ViewLoginLogs,
    Permission::ManageUsers,
    Permission::ManageSettings,
];
#[allow(dead_code)]
const OPERATOR_PERMISSIONS: &[Permission] = &[
    Permission::ViewAssets,
    Permission::ViewGrid,
    Permission::UseRemote,
];
#[allow(dead_code)]
const AUDITOR_PERMISSIONS: &[Permission] = &[Permission::ViewAudit, Permission::ViewLoginLogs];

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Superadmin,
    Admin,
    Operator,
    Auditor,
}

#[allow(dead_code)]
impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Superadmin => "superadmin",
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Auditor => "auditor",
        }
    }

    pub fn permissions(self) -> &'static [Permission] {
        match self {
            Role::Superadmin | Role::Admin => ALL_PERMISSIONS,
            Role::Operator => OPERATOR_PERMISSIONS,
            Role::Auditor => AUDITOR_PERMISSIONS,
        }
    }
}

impl FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "superadmin" => Ok(Role::Superadmin),
            "admin" => Ok(Role::Admin),
            "operator" => Ok(Role::Operator),
            "auditor" => Ok(Role::Auditor),
            other => Err(anyhow!("未知角色: {other}")),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ViewAssets,
    ViewGrid,
    UseRemote,
    ViewAudit,
    ViewLoginLogs,
    ManageUsers,
    ManageSettings,
}

#[allow(dead_code)]
impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::ViewAssets => "view_assets",
            Permission::ViewGrid => "view_grid",
            Permission::UseRemote => "use_remote",
            Permission::ViewAudit => "view_audit",
            Permission::ViewLoginLogs => "view_login_logs",
            Permission::ManageUsers => "manage_users",
            Permission::ManageSettings => "manage_settings",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: Role,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct UserStore {
    db: Db,
}

#[allow(dead_code)]
impl UserStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn bootstrap(&self, legacy: Option<(String, String)>) -> Result<()> {
        if self.get_by_username("superadmin").await?.is_none() {
            self.create_with_hash(
                "superadmin",
                &crate::auth::hash_password("infogo123"),
                Role::Superadmin,
            )
            .await?;
        }

        if let Some((legacy_user, legacy_hash)) = legacy {
            let legacy_hash = legacy_hash.trim();
            if !legacy_hash.is_empty() {
                let username = legacy_admin_username(&legacy_user);
                if self.get_by_username(&username).await?.is_none() {
                    self.create_with_hash(&username, legacy_hash, Role::Admin)
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub async fn create(&self, username: &str, password: &str, role: Role) -> Result<UserRecord> {
        if role == Role::Superadmin {
            bail!("不能通过普通路径创建超级管理员");
        }
        if password.is_empty() {
            bail!("密码不能为空");
        }
        let password_hash = crate::auth::hash_password(password);
        self.create_with_hash(username, &password_hash, role).await
    }

    pub async fn list(&self) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, role, enabled, created_at, updated_at
             FROM users
             ORDER BY created_at ASC, username ASC",
        )
        .fetch_all(&self.db)
        .await?;
        rows.into_iter().map(row_to_user).collect()
    }

    pub async fn get_by_username(&self, username: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, enabled, created_at, updated_at
             FROM users
             WHERE username = ?",
        )
        .bind(username.trim())
        .fetch_optional(&self.db)
        .await?;
        row.map(row_to_user).transpose()
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, enabled, created_at, updated_at
             FROM users
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        row.map(row_to_user).transpose()
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let user = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))?;
        if user.role == Role::Superadmin && !enabled {
            bail!("不能停用超级管理员");
        }
        sqlx::query("UPDATE users SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(enabled)
            .bind(now_sec())
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn set_role(&self, id: &str, role: Role) -> Result<()> {
        let user = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))?;
        if user.role == Role::Superadmin {
            bail!("不能修改超级管理员角色");
        }
        if role == Role::Superadmin {
            bail!("不能将普通用户晋升为超级管理员");
        }
        sqlx::query("UPDATE users SET role = ?, updated_at = ? WHERE id = ?")
            .bind(role.as_str())
            .bind(now_sec())
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn reset_password(&self, id: &str, password: &str) -> Result<()> {
        if password.is_empty() {
            bail!("密码不能为空");
        }
        let password_hash = crate::auth::hash_password(password);
        if password_hash.is_empty() {
            bail!("密码哈希不能为空");
        }
        let result = sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(password_hash)
            .bind(now_sec())
            .bind(id)
            .execute(&self.db)
            .await?;
        if result.rows_affected() == 0 {
            bail!("用户不存在: {id}");
        }
        Ok(())
    }

    pub async fn set_username(&self, id: &str, username: &str) -> Result<()> {
        let username = username.trim();
        if username.is_empty() {
            bail!("用户名不能为空");
        }
        self.ensure_username_available(username, Some(id)).await?;
        let result = sqlx::query("UPDATE users SET username = ?, updated_at = ? WHERE id = ?")
            .bind(username)
            .bind(now_sec())
            .bind(id)
            .execute(&self.db)
            .await
            .map_err(map_username_write_error)?;
        if result.rows_affected() == 0 {
            bail!("用户不存在: {id}");
        }
        Ok(())
    }

    pub async fn change_credential(
        &self,
        id: &str,
        current_pass: &str,
        new_username: Option<&str>,
        new_pass: Option<&str>,
    ) -> Result<UserRecord> {
        let user = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))?;
        if !bcrypt::verify(current_pass, &user.password_hash).unwrap_or(false) {
            bail!("当前密码错误");
        }

        let username = new_username
            .map(str::trim)
            .filter(|username| !username.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| user.username.clone());
        self.ensure_username_available(&username, Some(id)).await?;
        let password_hash = match new_pass {
            Some(pass) if !pass.is_empty() => crate::auth::hash_password(pass),
            _ => user.password_hash.clone(),
        };
        if password_hash.is_empty() {
            bail!("密码哈希不能为空");
        }

        sqlx::query(
            "UPDATE users SET username = ?, password_hash = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&username)
        .bind(&password_hash)
        .bind(now_sec())
        .bind(id)
        .execute(&self.db)
        .await
        .map_err(map_username_write_error)?;
        self.get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))
    }

    async fn create_with_hash(
        &self,
        username: &str,
        password_hash: &str,
        role: Role,
    ) -> Result<UserRecord> {
        let username = username.trim();
        if username.is_empty() {
            bail!("用户名不能为空");
        }
        if password_hash.is_empty() {
            bail!("密码哈希不能为空");
        }
        self.ensure_username_available(username, None).await?;

        let now = now_sec();
        let user = UserRecord {
            id: uuid::Uuid::new_v4().to_string(),
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            role,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(user.role.as_str())
        .bind(user.enabled)
        .bind(user.created_at)
        .bind(user.updated_at)
        .execute(&self.db)
        .await
        .map_err(map_username_write_error)?;
        Ok(user)
    }

    async fn ensure_username_available(
        &self,
        username: &str,
        except_id: Option<&str>,
    ) -> Result<()> {
        if let Some(existing) = self.get_by_username(username).await? {
            if Some(existing.id.as_str()) != except_id {
                bail!("用户名已存在");
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn row_to_user(row: sqlx::sqlite::SqliteRow) -> Result<UserRecord> {
    Ok(UserRecord {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
        role: row.get::<String, _>("role").parse()?,
        enabled: row.get::<bool, _>("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[allow(dead_code)]
fn now_sec() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[allow(dead_code)]
fn legacy_admin_username(legacy_user: &str) -> String {
    let legacy_user = legacy_user.trim();
    if legacy_user.is_empty() || legacy_user == "superadmin" {
        "admin".to_string()
    } else {
        legacy_user.to_string()
    }
}

fn map_username_write_error(err: sqlx::Error) -> anyhow::Error {
    if let sqlx::Error::Database(db_err) = &err {
        if is_username_unique_violation(db_err.message()) {
            return anyhow!("用户名已存在");
        }
    }
    err.into()
}

fn is_username_unique_violation(message: &str) -> bool {
    message.contains("users.username")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

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

    async fn test_store() -> UserStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(USERS_DDL).execute(&pool).await.unwrap();
        UserStore::new(pool)
    }

    fn has(role: Role, permission: Permission) -> bool {
        role.permissions().contains(&permission)
    }

    fn all_permissions() -> Vec<Permission> {
        ALL_PERMISSIONS.to_vec()
    }

    #[test]
    fn operator_and_auditor_have_fixed_permissions() {
        assert!(has(Role::Operator, Permission::UseRemote));
        assert!(has(Role::Operator, Permission::ViewGrid));
        assert!(!has(Role::Operator, Permission::ViewAudit));
        assert!(!has(Role::Operator, Permission::ManageUsers));

        assert!(has(Role::Auditor, Permission::ViewAudit));
        assert!(has(Role::Auditor, Permission::ViewLoginLogs));
        assert!(!has(Role::Auditor, Permission::UseRemote));
        assert!(!has(Role::Auditor, Permission::ManageSettings));
    }

    #[test]
    fn admin_and_superadmin_have_all_permissions() {
        assert_eq!(Role::Admin.permissions(), all_permissions().as_slice());
        assert_eq!(Role::Superadmin.permissions(), all_permissions().as_slice());
    }

    #[test]
    fn known_roles_parse_and_unknown_role_returns_error() {
        assert_eq!("superadmin".parse::<Role>().unwrap(), Role::Superadmin);
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
        assert_eq!("operator".parse::<Role>().unwrap(), Role::Operator);
        assert_eq!("auditor".parse::<Role>().unwrap(), Role::Auditor);
        assert!("unknown".parse::<Role>().is_err());
    }

    #[test]
    fn sqlite_username_unique_violation_maps_to_stable_error() {
        assert!(is_username_unique_violation(
            "UNIQUE constraint failed: users.username"
        ));
        assert!(!is_username_unique_violation(
            "UNIQUE constraint failed: users.id"
        ));
    }

    #[tokio::test]
    async fn create_rejects_superadmin_role() {
        let store = test_store().await;

        assert!(store
            .create("root", "secret", Role::Superadmin)
            .await
            .is_err());
        assert!(store.get_by_username("root").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_trims_username_rejects_empty_input_and_stores_bcrypt_hash() {
        let store = test_store().await;

        assert!(store.create("   ", "secret", Role::Operator).await.is_err());
        assert!(store.create("alice", "", Role::Operator).await.is_err());

        let user = store
            .create("  alice  ", "secret", Role::Operator)
            .await
            .unwrap();
        assert_eq!(user.username, "alice");
        assert_ne!(user.password_hash, "secret");
        assert!(bcrypt::verify("secret", &user.password_hash).unwrap());
    }

    #[tokio::test]
    async fn create_returns_stable_error_when_username_already_exists() {
        let store = test_store().await;
        store
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();

        let err = store
            .create(" alice ", "another-secret", Role::Auditor)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "用户名已存在");
        let users = store.list().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "alice");
    }

    #[tokio::test]
    async fn get_by_username_get_by_id_and_list_read_created_users() {
        let store = test_store().await;
        let alice = store
            .create("alice", "a-pass", Role::Operator)
            .await
            .unwrap();
        let bob = store.create("bob", "b-pass", Role::Auditor).await.unwrap();

        assert_eq!(
            store.get_by_username("alice").await.unwrap().unwrap().id,
            alice.id
        );
        assert_eq!(
            store.get_by_id(&bob.id).await.unwrap().unwrap().username,
            "bob"
        );

        let users = store.list().await.unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "alice");
        assert_eq!(users[1].username, "bob");
    }

    #[tokio::test]
    async fn set_enabled_protects_superadmin_and_allows_regular_users() {
        let store = test_store().await;
        store.bootstrap(None).await.unwrap();
        let superadmin = store.get_by_username("superadmin").await.unwrap().unwrap();
        let operator = store
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        assert!(store.set_enabled(&superadmin.id, false).await.is_err());
        store.set_enabled(&operator.id, false).await.unwrap();

        let operator = store.get_by_id(&operator.id).await.unwrap().unwrap();
        assert!(!operator.enabled);
    }

    #[tokio::test]
    async fn set_role_protects_superadmin_and_allows_regular_users() {
        let store = test_store().await;
        store.bootstrap(None).await.unwrap();
        let superadmin = store.get_by_username("superadmin").await.unwrap().unwrap();
        let operator = store
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        assert!(store.set_role(&superadmin.id, Role::Auditor).await.is_err());
        store.set_role(&operator.id, Role::Auditor).await.unwrap();

        let operator = store.get_by_id(&operator.id).await.unwrap().unwrap();
        assert_eq!(operator.role, Role::Auditor);
    }

    #[tokio::test]
    async fn set_role_rejects_promoting_regular_user_to_superadmin() {
        let store = test_store().await;
        let operator = store
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        assert!(store
            .set_role(&operator.id, Role::Superadmin)
            .await
            .is_err());

        let operator = store.get_by_id(&operator.id).await.unwrap().unwrap();
        assert_eq!(operator.role, Role::Operator);
    }

    #[tokio::test]
    async fn reset_password_rejects_empty_password_and_updates_hash() {
        let store = test_store().await;
        let user = store
            .create("alice", "old-pass", Role::Operator)
            .await
            .unwrap();
        let old_hash = user.password_hash.clone();

        assert!(store.reset_password(&user.id, "").await.is_err());
        store.reset_password(&user.id, "new-pass").await.unwrap();

        let user = store.get_by_id(&user.id).await.unwrap().unwrap();
        assert_ne!(user.password_hash, old_hash);
        assert!(bcrypt::verify("new-pass", &user.password_hash).unwrap());
        assert!(!bcrypt::verify("old-pass", &user.password_hash).unwrap());
    }

    #[tokio::test]
    async fn reset_password_returns_error_for_missing_user() {
        let store = test_store().await;

        assert!(store
            .reset_password("missing-user-id", "new-pass")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn set_username_rejects_empty_input_and_updates_trimmed_username() {
        let store = test_store().await;
        let user = store
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();

        assert!(store.set_username(&user.id, "   ").await.is_err());
        store.set_username(&user.id, "  alice2  ").await.unwrap();

        assert!(store.get_by_username("alice").await.unwrap().is_none());
        let user = store.get_by_id(&user.id).await.unwrap().unwrap();
        assert_eq!(user.username, "alice2");
    }

    #[tokio::test]
    async fn set_username_returns_stable_error_when_username_already_exists() {
        let store = test_store().await;
        store
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();
        let bob = store.create("bob", "secret", Role::Auditor).await.unwrap();

        let err = store.set_username(&bob.id, " alice ").await.unwrap_err();

        assert_eq!(err.to_string(), "用户名已存在");
        let bob = store.get_by_id(&bob.id).await.unwrap().unwrap();
        assert_eq!(bob.username, "bob");
    }

    #[tokio::test]
    async fn change_credential_verifies_current_password_and_updates_self() {
        let store = test_store().await;
        let user = store
            .create("alice", "old-pass", Role::Operator)
            .await
            .unwrap();

        assert!(store
            .change_credential(&user.id, "wrong-pass", Some("alice2"), Some("new-pass"))
            .await
            .is_err());

        let updated = store
            .change_credential(&user.id, "old-pass", Some("alice2"), Some("new-pass"))
            .await
            .unwrap();

        assert_eq!(updated.username, "alice2");
        assert_eq!(updated.role, Role::Operator);
        assert!(bcrypt::verify("new-pass", &updated.password_hash).unwrap());
        assert!(!bcrypt::verify("old-pass", &updated.password_hash).unwrap());
    }

    #[tokio::test]
    async fn change_credential_returns_stable_error_when_username_already_exists() {
        let store = test_store().await;
        store
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();
        let bob = store
            .create("bob", "old-pass", Role::Auditor)
            .await
            .unwrap();

        let err = store
            .change_credential(&bob.id, "old-pass", Some(" alice "), Some("new-pass"))
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "用户名已存在");
        let bob = store.get_by_id(&bob.id).await.unwrap().unwrap();
        assert_eq!(bob.username, "bob");
        assert!(bcrypt::verify("old-pass", &bob.password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_without_legacy_creates_enabled_superadmin_with_default_password() {
        let store = test_store().await;

        store.bootstrap(None).await.unwrap();

        let users = store.list().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "superadmin");
        assert_eq!(users[0].role, Role::Superadmin);
        assert!(users[0].enabled);
        assert!(bcrypt::verify("infogo123", &users[0].password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_with_legacy_creates_superadmin_and_migrated_admin_user() {
        let store = test_store().await;
        let legacy_hash = bcrypt::hash("legacy-pass", bcrypt::DEFAULT_COST).unwrap();

        store
            .bootstrap(Some(("legacy_admin".to_string(), legacy_hash.clone())))
            .await
            .unwrap();

        let superadmin = store.get_by_username("superadmin").await.unwrap().unwrap();
        assert_eq!(superadmin.role, Role::Superadmin);

        let legacy = store
            .get_by_username("legacy_admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.role, Role::Admin);
        assert!(bcrypt::verify("legacy-pass", &legacy.password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_when_regular_user_exists_still_creates_superadmin() {
        let store = test_store().await;
        store
            .create("existing", "secret", Role::Operator)
            .await
            .unwrap();

        store.bootstrap(None).await.unwrap();

        let superadmin = store.get_by_username("superadmin").await.unwrap().unwrap();
        assert_eq!(superadmin.role, Role::Superadmin);
        assert!(bcrypt::verify("infogo123", &superadmin.password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_when_only_superadmin_exists_still_creates_legacy_admin() {
        let store = test_store().await;
        let legacy_hash = bcrypt::hash("legacy-pass", bcrypt::DEFAULT_COST).unwrap();
        store.bootstrap(None).await.unwrap();

        store
            .bootstrap(Some(("legacy_admin".to_string(), legacy_hash)))
            .await
            .unwrap();

        let legacy = store
            .get_by_username("legacy_admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.role, Role::Admin);
        assert!(bcrypt::verify("legacy-pass", &legacy.password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_renames_blank_or_superadmin_legacy_user_to_admin() {
        for legacy_user in ["", "superadmin"] {
            let store = test_store().await;
            let legacy_hash = bcrypt::hash("legacy-pass", bcrypt::DEFAULT_COST).unwrap();

            store
                .bootstrap(Some((legacy_user.to_string(), legacy_hash)))
                .await
                .unwrap();

            assert!(store.get_by_username("admin").await.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn bootstrap_does_not_create_duplicates_when_called_repeatedly() {
        let store = test_store().await;
        let legacy_hash = bcrypt::hash("legacy-pass", bcrypt::DEFAULT_COST).unwrap();

        store
            .bootstrap(Some(("legacy_admin".to_string(), legacy_hash.clone())))
            .await
            .unwrap();
        store
            .bootstrap(Some(("legacy_admin".to_string(), legacy_hash)))
            .await
            .unwrap();

        let users = store.list().await.unwrap();
        assert_eq!(users.len(), 2);
        assert!(store.get_by_username("superadmin").await.unwrap().is_some());
        assert!(store
            .get_by_username("legacy_admin")
            .await
            .unwrap()
            .is_some());
    }
}
