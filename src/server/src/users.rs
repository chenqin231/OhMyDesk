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
    Permission::ManageAssets,
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
    ManageAssets,
    ViewGrid,
    UseRemote,
    ViewAudit,
    ViewLoginLogs,
    ManageUsers,
    ManageSettings,
}

#[allow(dead_code)]
impl Permission {
    /// 新权限模型全集（7 项，含 superadmin 独占的 manage_users）。
    /// 刻意不含已退役的 manage_settings（个人设置人人可达，见 plan Task 7）。
    /// `PermissionSet` 的规范化顺序、superadmin 隐式全集均以此顺序为准。
    pub const ALL: &'static [Permission] = &[
        Permission::ViewAssets,
        Permission::ManageAssets,
        Permission::ViewGrid,
        Permission::UseRemote,
        Permission::ViewAudit,
        Permission::ViewLoginLogs,
        Permission::ManageUsers,
    ];

    /// 可配给普通账户的菜单权限（6 项，= ALL 去掉 manage_users）。
    /// manage_users（账户管理）为 superadmin 独占，不可授予普通账户。
    pub const ASSIGNABLE: &'static [Permission] = &[
        Permission::ViewAssets,
        Permission::ManageAssets,
        Permission::ViewGrid,
        Permission::UseRemote,
        Permission::ViewAudit,
        Permission::ViewLoginLogs,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::ViewAssets => "view_assets",
            Permission::ManageAssets => "manage_assets",
            Permission::ViewGrid => "view_grid",
            Permission::UseRemote => "use_remote",
            Permission::ViewAudit => "view_audit",
            Permission::ViewLoginLogs => "view_login_logs",
            Permission::ManageUsers => "manage_users",
            Permission::ManageSettings => "manage_settings",
        }
    }

    /// 权限键字符串 → 枚举；未知键返回 None（供 `PermissionSet::parse` 过滤脏数据）。
    pub fn from_str(s: &str) -> Option<Permission> {
        Some(match s {
            "view_assets" => Permission::ViewAssets,
            "manage_assets" => Permission::ManageAssets,
            "view_grid" => Permission::ViewGrid,
            "use_remote" => Permission::UseRemote,
            "view_audit" => Permission::ViewAudit,
            "view_login_logs" => Permission::ViewLoginLogs,
            "manage_users" => Permission::ManageUsers,
            "manage_settings" => Permission::ManageSettings,
            _ => return None,
        })
    }
}

/// 按账户存储的菜单权限集（运行期权限源）。
///
/// 不变式：内部 `perms` 始终按 `Permission::ALL` 顺序、去重、且只含 ALL 成员
/// （`manage_settings` 等非模型成员在构造时被剔除）。所有构造入口都经 `from_perms`
/// 归一，故 `to_storage`/序列化输出顺序稳定。superadmin 隐式全权，用 `superadmin_all()`
/// 表示（不落库，见 db.rs 迁移：superadmin 的 permissions 列留空）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionSet {
    // serde(transparent)：序列化为 ["view_assets", ...]（前端消费）；反序列化不强制归一，
    // 但 UserRecord 实际不被反序列化，故无副作用。
    perms: Vec<Permission>,
}

#[allow(dead_code)]
impl PermissionSet {
    /// 由权限切片构造，按 ALL 顺序归一 + 去重 + 剔除非 ALL 成员。
    fn from_perms(perms: &[Permission]) -> Self {
        let perms = Permission::ALL
            .iter()
            .copied()
            .filter(|p| perms.contains(p))
            .collect();
        Self { perms }
    }

    /// 解析逗号分隔的权限键串（脏数据/未知键静默剔除）。
    pub fn parse(s: &str) -> Self {
        let raw: Vec<Permission> = s
            .split(',')
            .filter_map(|t| Permission::from_str(t.trim()))
            .collect();
        Self::from_perms(&raw)
    }

    /// superadmin 隐式全集（含 manage_users）。
    pub fn superadmin_all() -> Self {
        Self {
            perms: Permission::ALL.to_vec(),
        }
    }

    /// 旧固定角色 → 权限集（仅供旧库/旧 fixture 兼容派生；新账户一律走存储集）。
    fn from_role_legacy(role: Role) -> Self {
        match role {
            Role::Superadmin => Self::superadmin_all(),
            // admin=拥有全部可配菜单（不含账户管理 manage_users）
            Role::Admin => Self::from_perms(Permission::ASSIGNABLE),
            Role::Operator => Self::parse("view_assets,view_grid,use_remote"),
            Role::Auditor => Self::parse("view_audit,view_login_logs"),
        }
    }

    pub fn contains(&self, p: Permission) -> bool {
        self.perms.contains(&p)
    }

    pub fn is_empty(&self) -> bool {
        self.perms.is_empty()
    }

    /// 落库形态：按 ALL 顺序的逗号分隔键串。
    pub fn to_storage(&self) -> String {
        self.perms
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// 权限键列表（按 ALL 顺序）——供前端/序列化消费。
    pub fn keys(&self) -> Vec<&'static str> {
        self.perms.iter().map(|p| p.as_str()).collect()
    }
}

/// 校验一批权限键是否可配给普通账户：仅 ASSIGNABLE、且 manage_assets 依赖 view_assets。
/// 通过返回归一后的权限向量；否则返回稳定错误信息。
fn validate_assignable(perms: &[&str]) -> Result<Vec<Permission>> {
    let mut out: Vec<Permission> = Vec::new();
    for key in perms {
        let p = Permission::from_str(key.trim()).ok_or_else(|| anyhow!("未知权限键: {key}"))?;
        if !Permission::ASSIGNABLE.contains(&p) {
            bail!("不可配权限: {key}");
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.contains(&Permission::ManageAssets) && !out.contains(&Permission::ViewAssets) {
        bail!("manage_assets 依赖 view_assets，必须同时授予");
    }
    Ok(out)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    // role：兼容旧 Role 枚举的运行期消费方（auth/http/hub 暂用；Task3/4 改读 permissions）。
    // 新库 role 列存 tier('superadmin'/'user')，'user' 在读取时映射为 Role::Admin 门面。
    pub role: Role,
    // permissions：按账户菜单权限集，运行期权限的真源（superadmin 为隐式全集）。
    pub permissions: PermissionSet,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[allow(dead_code)]
impl UserRecord {
    /// 是否超级管理员（tier=superadmin）。role 门面对 superadmin 唯一映射为 Role::Superadmin。
    pub fn is_superadmin(&self) -> bool {
        self.role == Role::Superadmin
    }

    /// tier 字符串：superadmin / user。
    pub fn tier(&self) -> &'static str {
        if self.is_superadmin() {
            "superadmin"
        } else {
            "user"
        }
    }
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
                    // 旧库无 tier 概念的 legacy admin → 迁为「tier=user + 全部可配菜单」的普通账户。
                    // 不再用 Role::Admin（会写 role='admin'，撞新 schema CHECK(superadmin/user)）。
                    self.insert_tier_user(
                        &username,
                        legacy_hash,
                        "user",
                        &PermissionSet::from_perms(Permission::ASSIGNABLE),
                    )
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

    /// 按账户权限模型建普通账户（tier=user）：校验权限集（仅 ASSIGNABLE、manage_assets⇒view_assets），
    /// 存储所授菜单集。运行期权限一律走此存储集（非旧 Role 硬映射）。
    pub async fn create_user_v2(
        &self,
        username: &str,
        password: &str,
        perms: &[&str],
    ) -> Result<UserRecord> {
        if password.is_empty() {
            bail!("密码不能为空");
        }
        let validated = validate_assignable(perms)?;
        let username = username.trim();
        if username.is_empty() {
            bail!("用户名不能为空");
        }
        self.ensure_username_available(username, None).await?;
        let password_hash = crate::auth::hash_password(password);
        if password_hash.is_empty() {
            bail!("密码哈希不能为空");
        }
        self.insert_tier_user(
            username,
            &password_hash,
            "user",
            &PermissionSet::from_perms(&validated),
        )
        .await
    }

    /// 覆盖普通账户的菜单权限集。校验：仅 ASSIGNABLE、manage_assets⇒view_assets、superadmin 目标拒改。
    pub async fn set_permissions(&self, id: &str, perms: &[&str]) -> Result<()> {
        let user = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))?;
        if user.is_superadmin() {
            bail!("不能修改超级管理员权限");
        }
        let validated = validate_assignable(perms)?;
        let storage = PermissionSet::from_perms(&validated).to_storage();
        sqlx::query("UPDATE users SET permissions = ?, updated_at = ? WHERE id = ?")
            .bind(storage)
            .bind(now_sec())
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, role, permissions, enabled, created_at, updated_at
             FROM users
             ORDER BY created_at ASC, username ASC",
        )
        .fetch_all(&self.db)
        .await?;
        rows.into_iter().map(row_to_user).collect()
    }

    pub async fn get_by_username(&self, username: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, permissions, enabled, created_at, updated_at
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
            "SELECT id, username, password_hash, role, permissions, enabled, created_at, updated_at
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
        // superadmin 目标拒改登录名（唯一 god，登录名锁定）。
        let user = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("用户不存在: {id}"))?;
        if user.is_superadmin() {
            bail!("不能修改超级管理员用户名");
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
        // 兼容路径（旧 Role 创建）：role 列写 role.as_str()，permissions 列不写（默认 ''）。
        // 内存 permissions 由 Role 派生，与读回一致（row_to_user 对空存储集按 Role 兜底）。
        let user = UserRecord {
            id: uuid::Uuid::new_v4().to_string(),
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            role,
            permissions: PermissionSet::from_role_legacy(role),
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

    /// 按账户权限模型写库（tier + permissions 列）：create_user_v2 与 bootstrap 迁移共用。
    /// role 列写 tier('superadmin'/'user')，permissions 列写规范化存储串。
    async fn insert_tier_user(
        &self,
        username: &str,
        password_hash: &str,
        tier: &str,
        permissions: &PermissionSet,
    ) -> Result<UserRecord> {
        let username = username.trim();
        if username.is_empty() {
            bail!("用户名不能为空");
        }
        if password_hash.is_empty() {
            bail!("密码哈希不能为空");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_sec();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, permissions, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(password_hash)
        .bind(tier)
        .bind(permissions.to_storage())
        .bind(true)
        .bind(now)
        .bind(now)
        .execute(&self.db)
        .await
        .map_err(map_username_write_error)?;
        self.get_by_id(&id)
            .await?
            .ok_or_else(|| anyhow!("创建后读取用户失败: {id}"))
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
    let role_str: String = row.get("role");
    let role = parse_role_compat(&role_str)?;
    // permissions 列可能缺失（旧 fixture）→ try_get 兜底为空串。
    let perms_raw: String = row.try_get("permissions").unwrap_or_default();
    let permissions = if role_str == "superadmin" {
        // superadmin 隐式全集（存储列留空，运行期用 superadmin_all）
        PermissionSet::superadmin_all()
    } else if perms_raw.trim().is_empty() {
        // 空存储集：旧库/旧 fixture 兼容——按 Role 门面派生
        PermissionSet::from_role_legacy(role)
    } else {
        PermissionSet::parse(&perms_raw)
    };
    Ok(UserRecord {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
        role,
        permissions,
        enabled: row.get::<bool, _>("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// role 列字符串 → Role 门面。新库 tier 'user' 映射为 Role::Admin（供旧 Role 消费方兼容）；
/// 'superadmin' 唯一映射为 Role::Superadmin，故 `UserRecord::is_superadmin` 判据稳定。
/// 旧 fixture 的 'admin'/'operator'/'auditor' 仍按 Role FromStr 解析。
fn parse_role_compat(role: &str) -> Result<Role> {
    match role {
        "user" => Ok(Role::Admin),
        other => other.parse(),
    }
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

    // 新权限模型 fixture：CHECK(superadmin/user) + permissions 列，与生产 schema.sqlite.sql 对齐。
    const USERS_DDL: &str = r#"
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'user')),
  permissions TEXT NOT NULL DEFAULT '',
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
        assert!(!has(Role::Operator, Permission::ManageAssets));
        assert!(!has(Role::Operator, Permission::ViewAudit));
        assert!(!has(Role::Operator, Permission::ManageUsers));

        assert!(has(Role::Auditor, Permission::ViewAudit));
        assert!(has(Role::Auditor, Permission::ViewLoginLogs));
        assert!(!has(Role::Auditor, Permission::ManageAssets));
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

        assert!(store
            .create_user_v2("   ", "secret", &["view_grid"])
            .await
            .is_err());
        assert!(store
            .create_user_v2("alice", "", &["view_grid"])
            .await
            .is_err());

        let user = store
            .create_user_v2("  alice  ", "secret", &["view_grid"])
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
            .create_user_v2("alice", "secret", &["view_grid"])
            .await
            .unwrap();

        let err = store
            .create_user_v2(" alice ", "another-secret", &["view_audit"])
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
            .create_user_v2("alice", "a-pass", &["view_assets", "view_grid", "use_remote"])
            .await
            .unwrap();
        let bob = store
            .create_user_v2("bob", "b-pass", &["view_audit", "view_login_logs"])
            .await
            .unwrap();

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
            .create_user_v2("operator", "secret", &["view_assets", "view_grid", "use_remote"])
            .await
            .unwrap();

        assert!(store.set_enabled(&superadmin.id, false).await.is_err());
        store.set_enabled(&operator.id, false).await.unwrap();

        let operator = store.get_by_id(&operator.id).await.unwrap().unwrap();
        assert!(!operator.enabled);
    }

    // 取代旧 set_role 的 superadmin 保护测试：按账户权限模型下，改权限走 set_permissions，
    // superadmin 目标一律拒改（隐式全权，不可被降权/配菜单）。
    #[tokio::test]
    async fn set_permissions_protects_superadmin_and_updates_regular_users() {
        let store = test_store().await;
        store.bootstrap(None).await.unwrap();
        let superadmin = store.get_by_username("superadmin").await.unwrap().unwrap();
        let operator = store
            .create_user_v2("operator", "secret", &["view_grid"])
            .await
            .unwrap();

        assert!(store
            .set_permissions(&superadmin.id, &["view_assets"])
            .await
            .is_err());
        store
            .set_permissions(&operator.id, &["view_audit", "view_login_logs"])
            .await
            .unwrap();

        let operator = store.get_by_id(&operator.id).await.unwrap().unwrap();
        assert!(operator.permissions.contains(Permission::ViewAudit));
        assert!(!operator.permissions.contains(Permission::ViewGrid));
    }

    #[tokio::test]
    async fn reset_password_rejects_empty_password_and_updates_hash() {
        let store = test_store().await;
        let user = store
            .create_user_v2("alice", "old-pass", &["view_grid"])
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
            .create_user_v2("alice", "secret", &["view_grid"])
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
            .create_user_v2("alice", "secret", &["view_grid"])
            .await
            .unwrap();
        let bob = store
            .create_user_v2("bob", "secret", &["view_audit"])
            .await
            .unwrap();

        let err = store.set_username(&bob.id, " alice ").await.unwrap_err();

        assert_eq!(err.to_string(), "用户名已存在");
        let bob = store.get_by_id(&bob.id).await.unwrap().unwrap();
        assert_eq!(bob.username, "bob");
    }

    #[tokio::test]
    async fn change_credential_verifies_current_password_and_updates_self() {
        let store = test_store().await;
        let user = store
            .create_user_v2("alice", "old-pass", &["view_grid"])
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
        // 改密不动权限集：仍为普通账户、菜单权限不变
        assert!(!updated.is_superadmin());
        assert!(updated.permissions.contains(Permission::ViewGrid));
        assert!(bcrypt::verify("new-pass", &updated.password_hash).unwrap());
        assert!(!bcrypt::verify("old-pass", &updated.password_hash).unwrap());
    }

    #[tokio::test]
    async fn change_credential_returns_stable_error_when_username_already_exists() {
        let store = test_store().await;
        store
            .create_user_v2("alice", "secret", &["view_grid"])
            .await
            .unwrap();
        let bob = store
            .create_user_v2("bob", "old-pass", &["view_audit"])
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
        // legacy 迁为 tier=user + 全部可配菜单（含 manage_assets，不含账户管理 manage_users）
        assert!(!legacy.is_superadmin());
        assert_eq!(legacy.tier(), "user");
        assert!(legacy.permissions.contains(Permission::ManageAssets));
        assert!(legacy.permissions.contains(Permission::ViewLoginLogs));
        assert!(!legacy.permissions.contains(Permission::ManageUsers));
        assert!(bcrypt::verify("legacy-pass", &legacy.password_hash).unwrap());
    }

    #[tokio::test]
    async fn bootstrap_when_regular_user_exists_still_creates_superadmin() {
        let store = test_store().await;
        store
            .create_user_v2("existing", "secret", &["view_grid"])
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

    // ── Task 2：按账户权限集（PermissionSet）+ superadmin 隐式全集 ─────────────

    #[test]
    fn permission_parse_roundtrip_and_superadmin_is_implicit_all() {
        // 字符串集解析：contains 命中/未命中
        let set = PermissionSet::parse("view_assets,use_remote,manage_assets");
        assert!(set.contains(Permission::UseRemote) && set.contains(Permission::ManageAssets));
        assert!(!set.contains(Permission::ViewAudit));
        // to_storage 规范化顺序稳定（按 Permission::ALL 顺序）
        assert_eq!(set.to_storage(), "view_assets,manage_assets,use_remote");
        // superadmin 隐式全集（含 manage_users）
        let sa = PermissionSet::superadmin_all();
        for p in Permission::ALL {
            assert!(sa.contains(*p));
        }
        assert!(sa.contains(Permission::ManageUsers));
    }

    #[tokio::test]
    async fn set_permissions_persists_and_manage_assets_requires_view_assets() {
        let store = test_store().await;
        let u = store.create_user_v2("op1", "pw", &["view_grid"]).await.unwrap();

        // 合法覆盖：view_assets + manage_assets 生效，且是「覆盖」语义（旧 view_grid 被替换）
        store
            .set_permissions(&u.id, &["view_assets", "manage_assets"])
            .await
            .unwrap();
        let got = store.get_by_id(&u.id).await.unwrap().unwrap();
        assert!(got.permissions.contains(Permission::ManageAssets));
        assert!(got.permissions.contains(Permission::ViewAssets));
        assert!(!got.permissions.contains(Permission::ViewGrid));

        // manage_assets 缺 view_assets → 拒
        let err = store
            .set_permissions(&u.id, &["manage_assets"])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("view_assets"));

        // 非法键：manage_users 不可配给普通账户 → 拒
        assert!(store.set_permissions(&u.id, &["manage_users"]).await.is_err());
        // 未知键 → 拒
        assert!(store.set_permissions(&u.id, &["not_a_menu"]).await.is_err());

        // superadmin 目标拒改
        store.bootstrap(None).await.unwrap();
        let sa = store.get_by_username("superadmin").await.unwrap().unwrap();
        assert!(store.set_permissions(&sa.id, &["view_assets"]).await.is_err());
    }

    #[tokio::test]
    async fn create_user_v2_stores_tier_user_and_validates_dependency() {
        let store = test_store().await;
        // 普通账户建成 tier=user，permissions 为所授集
        let u = store
            .create_user_v2("alice", "pw", &["view_assets", "use_remote"])
            .await
            .unwrap();
        assert!(!u.is_superadmin());
        assert_eq!(u.tier(), "user");
        assert!(u.permissions.contains(Permission::UseRemote));
        // manage_assets 缺 view_assets → 建号即拒
        assert!(store
            .create_user_v2("bob", "pw", &["manage_assets"])
            .await
            .is_err());
        // 不可配键 manage_users → 拒
        assert!(store
            .create_user_v2("carol", "pw", &["manage_users"])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn fresh_schema_bootstrap_backfills_tiers_without_check_violation() {
        // 用「真实生产 schema」（非测试 fixture）建全新内存库，跑 bootstrap，
        // 证明全新库启动路径不撞 CHECK（superadmin/user）且 legacy 迁为全功能普通账户。
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../../scripts/db/schema.sqlite.sql"))
            .execute(&pool)
            .await
            .unwrap();
        let store = UserStore::new(pool.clone());
        let legacy_hash = bcrypt::hash("legacy-pass", bcrypt::DEFAULT_COST).unwrap();
        store
            .bootstrap(Some(("legacy_admin".to_string(), legacy_hash)))
            .await
            .unwrap();

        // superadmin：tier=superadmin，permissions 空存储（隐式全权）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='superadmin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "superadmin");
        assert_eq!(perms, "");

        // legacy_admin：tier=user + 全部可配菜单（含 manage_assets，不含 manage_users）
        let (role, perms): (String, String) =
            sqlx::query_as("SELECT role, permissions FROM users WHERE username='legacy_admin'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "user");
        for key in [
            "view_assets",
            "manage_assets",
            "view_grid",
            "use_remote",
            "view_audit",
            "view_login_logs",
        ] {
            assert!(perms.contains(key), "legacy 应含菜单键 {key}");
        }
        assert!(!perms.contains("manage_users"));

        // 经 store 读回：superadmin 隐式全集（运行期权限源）
        let sa = store.get_by_username("superadmin").await.unwrap().unwrap();
        assert!(sa.is_superadmin());
        assert!(sa.permissions.contains(Permission::ManageUsers));
        let legacy = store.get_by_username("legacy_admin").await.unwrap().unwrap();
        assert!(!legacy.is_superadmin());
        assert!(legacy.permissions.contains(Permission::ManageAssets));
        assert!(!legacy.permissions.contains(Permission::ManageUsers));
    }
}
