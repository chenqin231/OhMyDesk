//! 管理平台鉴权：JWT(HS256) 签发/校验 + bcrypt 密码校验。

use std::sync::Arc;

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::users::{Role, UserRecord, UserStore};

/// 旧版默认管理员账号，用于兼容 set-password 的缺省用户名。
pub const DEFAULT_USER: &str = "admin";
/// token 有效期（秒）：12 小时。
const TOKEN_TTL_SECS: i64 = 12 * 3600;

/// JWT 载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub role: String,
    pub exp: i64,
}

/// 鉴权状态：用户仓储 + JWT 签名密钥。
pub struct Auth {
    users: Arc<UserStore>,
    secret: Vec<u8>,
}

impl Auth {
    pub fn new(secret: Vec<u8>, users: Arc<UserStore>) -> Self {
        Auth { users, secret }
    }

    /// 校验登录（用户表读取 + enabled + bcrypt 验密码）。
    pub async fn verify_login(&self, user: &str, pass: &str) -> Option<UserRecord> {
        let record = self.users.get_by_username(user).await.ok().flatten()?;
        if !record.enabled {
            return None;
        }
        if bcrypt::verify(pass, &record.password_hash).unwrap_or(false) {
            Some(record)
        } else {
            None
        }
    }

    pub async fn change_credential(
        &self,
        user_id: &str,
        current_pass: &str,
        new_user: Option<&str>,
        new_pass: Option<&str>,
    ) -> Result<UserRecord, String> {
        self.users
            .change_credential(user_id, current_pass, new_user, new_pass)
            .await
            .map_err(|e| e.to_string())
    }

    /// 签发 JWT；`now` 为秒级时间戳，sub 使用 users.id。
    pub fn issue_token(&self, user_id: &str, username: &str, role: Role, now: i64) -> String {
        let claims = Claims {
            sub: user_id.to_string(),
            username: username.to_string(),
            role: role.as_str().to_string(),
            exp: now + TOKEN_TTL_SECS,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(&self.secret),
        )
        .unwrap_or_default()
    }

    /// 校验 JWT（签名 + 过期）；通过返回 Claims，否则 None。
    pub fn validate_token_only(&self, token: &str) -> Option<Claims> {
        let validation = Validation::new(Algorithm::HS256);
        decode::<Claims>(token, &DecodingKey::from_secret(&self.secret), &validation)
            .ok()
            .map(|d| d.claims)
    }

    /// 校验 JWT 并回查用户表，确保旧 token 在用户禁用、改名或改角色后立即失效。
    pub async fn validate(&self, token: &str) -> Option<crate::http::AuthUser> {
        let claims = self.validate_token_only(token)?;
        let role: Role = claims.role.parse().ok()?;
        let user = self.users.get_by_id(&claims.sub).await.ok().flatten()?;
        if !user.enabled || user.username != claims.username || user.role != role {
            return None;
        }
        Some(crate::http::AuthUser {
            id: user.id,
            username: user.username,
            role: user.role,
        })
    }
}

/// bcrypt 哈希（默认 cost）。
pub fn hash_password(p: &str) -> String {
    bcrypt::hash(p, bcrypt::DEFAULT_COST).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::users::{Role, UserStore};
    use sqlx::sqlite::SqlitePoolOptions;

    // 保留旧 4 角色 CHECK（本模块测试仍用 create(Role) 走旧角色 API，Task3 改造），
    // 仅补 permissions 列以对齐 UserRecord 读取（SELECT 含 permissions）。
    const USERS_DDL: &str = r#"
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'admin', 'operator', 'auditor')),
  permissions TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
"#;

    async fn user_store() -> Arc<UserStore> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(USERS_DDL).execute(&pool).await.unwrap();
        Arc::new(UserStore::new(pool))
    }

    async fn auth() -> (Auth, Arc<UserStore>) {
        let users = user_store().await;
        (
            Auth::new(
                b"test-secret-32-bytes-long-xxxxxx".to_vec(),
                Arc::clone(&users),
            ),
            users,
        )
    }

    #[tokio::test]
    async fn jwt_claims_include_user_id_username_role_and_exp() {
        let (auth, users) = auth().await;
        let user = users
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();

        let token = auth.issue_token(&user.id, &user.username, user.role, 10_000_000_000);
        let claims = auth
            .validate_token_only(&token)
            .expect("自签 token 应校验通过");

        assert_eq!(claims.sub, user.id);
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.role, "operator");
        assert_eq!(claims.exp, 10_000_000_000 + TOKEN_TTL_SECS);
    }

    #[tokio::test]
    async fn verify_login_reads_enabled_users_from_store() {
        let (auth, users) = auth().await;
        let user = users
            .create("alice", "correct-pass", Role::Operator)
            .await
            .unwrap();

        let logged_in = auth
            .verify_login("alice", "correct-pass")
            .await
            .expect("正确密码应返回用户记录");
        assert_eq!(logged_in.id, user.id);
        assert_eq!(logged_in.username, "alice");
        assert_eq!(logged_in.role, Role::Operator);

        assert!(auth.verify_login("alice", "wrong-pass").await.is_none());

        users.set_enabled(&user.id, false).await.unwrap();
        assert!(auth.verify_login("alice", "correct-pass").await.is_none());
    }

    #[tokio::test]
    async fn validate_returns_auth_user_when_token_matches_current_enabled_user() {
        let (auth, users) = auth().await;
        let user = users
            .create("alice", "secret", Role::Auditor)
            .await
            .unwrap();
        let token = auth.issue_token(&user.id, &user.username, user.role, 10_000_000_000);

        let auth_user = auth.validate(&token).await.expect("有效 token 应通过");

        assert_eq!(auth_user.id, user.id);
        assert_eq!(auth_user.username, "alice");
        assert_eq!(auth_user.role, Role::Auditor);
    }

    #[tokio::test]
    async fn validate_rejects_old_token_after_user_is_disabled_or_identity_changes() {
        let (auth, users) = auth().await;
        let disabled = users
            .create("disabled", "secret", Role::Operator)
            .await
            .unwrap();
        let disabled_token = auth.issue_token(
            &disabled.id,
            &disabled.username,
            disabled.role,
            10_000_000_000,
        );
        users.set_enabled(&disabled.id, false).await.unwrap();
        assert!(auth.validate(&disabled_token).await.is_none());

        let renamed = users
            .create("renamed", "secret", Role::Operator)
            .await
            .unwrap();
        let renamed_token =
            auth.issue_token(&renamed.id, &renamed.username, renamed.role, 10_000_000_000);
        users
            .set_username(&renamed.id, "renamed-now")
            .await
            .unwrap();
        assert!(auth.validate(&renamed_token).await.is_none());

        let rerolled = users
            .create("rerolled", "secret", Role::Operator)
            .await
            .unwrap();
        let rerolled_token = auth.issue_token(
            &rerolled.id,
            &rerolled.username,
            rerolled.role,
            10_000_000_000,
        );
        users.set_role(&rerolled.id, Role::Auditor).await.unwrap();
        assert!(auth.validate(&rerolled_token).await.is_none());
    }

    #[tokio::test]
    async fn validate_token_only_rejects_expired_tampered_or_foreign_tokens() {
        let (auth, users) = auth().await;
        let user = users
            .create("alice", "secret", Role::Operator)
            .await
            .unwrap();
        let expired = auth.issue_token(&user.id, &user.username, user.role, 0);
        assert!(auth.validate_token_only(&expired).is_none());
        assert!(auth.validate_token_only("not.a.jwt").is_none());

        let foreign_users = user_store().await;
        let foreign_auth = Auth::new(
            b"another-secret-different-bytes!!".to_vec(),
            Arc::clone(&foreign_users),
        );
        let foreign = foreign_auth.issue_token(&user.id, &user.username, user.role, 10_000_000_000);
        assert!(auth.validate_token_only(&foreign).is_none());
    }
}
