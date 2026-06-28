//! 管理平台鉴权：JWT(HS256) 签发/校验 + bcrypt 密码 + 可改凭据（内存态，持久化由 settings 落库）。
//!
//! 默认凭据写死（首次启动且无持久化时用）；管理员可在「系统设置」改账号密码。
//! 凭据放 RwLock，登录走内存（不每次查库）；改密时同步更新内存 + 由调用方落 settings 表。

use std::sync::RwLock;

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// 写死的默认管理员账号/密码（系统设置页可改）。
pub const DEFAULT_USER: &str = "admin";
pub const DEFAULT_PASS: &str = "OhMyDesk@2026";
/// token 有效期（秒）：12 小时。
const TOKEN_TTL_SECS: i64 = 12 * 3600;

/// JWT 载荷。
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // 用户名
    pub exp: i64,
}

struct Cred {
    user: String,
    pass_hash: String,
}

/// 鉴权状态：当前凭据 + JWT 签名密钥。
pub struct Auth {
    cred: RwLock<Cred>,
    secret: Vec<u8>,
}

impl Auth {
    /// 用持久化凭据（无则默认）+ JWT secret 构造。
    pub fn new(secret: Vec<u8>, user: Option<String>, pass_hash: Option<String>) -> Self {
        let user = user.unwrap_or_else(|| DEFAULT_USER.to_string());
        let pass_hash = pass_hash.unwrap_or_else(|| hash_password(DEFAULT_PASS));
        Auth {
            cred: RwLock::new(Cred { user, pass_hash }),
            secret,
        }
    }

    /// 校验登录（用户名匹配 + bcrypt 验密码）。
    pub fn verify_login(&self, user: &str, pass: &str) -> bool {
        let c = self.cred.read().unwrap();
        c.user == user && bcrypt::verify(pass, &c.pass_hash).unwrap_or(false)
    }

    /// 当前用户名（系统设置回显）。
    pub fn current_user(&self) -> String {
        self.cred.read().unwrap().user.clone()
    }

    /// 改凭据：先验当前密码 → 改内存。返回新 (user, pass_hash) 供调用方落库。
    pub fn change_credential(
        &self,
        current_pass: &str,
        new_user: Option<&str>,
        new_pass: Option<&str>,
    ) -> Result<(String, String), String> {
        let mut c = self.cred.write().unwrap();
        if !bcrypt::verify(current_pass, &c.pass_hash).unwrap_or(false) {
            return Err("当前密码错误".into());
        }
        if let Some(u) = new_user {
            if !u.trim().is_empty() {
                c.user = u.trim().to_string();
            }
        }
        if let Some(p) = new_pass {
            if !p.is_empty() {
                c.pass_hash = hash_password(p);
            }
        }
        Ok((c.user.clone(), c.pass_hash.clone()))
    }

    /// 签发 JWT；`now` 为秒级时间戳。
    pub fn issue_token(&self, user: &str, now: i64) -> String {
        let claims = Claims {
            sub: user.to_string(),
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
    pub fn validate(&self, token: &str) -> Option<Claims> {
        let validation = Validation::new(Algorithm::HS256);
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(&self.secret),
            &validation,
        )
        .ok()
        .map(|d| d.claims)
    }
}

/// bcrypt 哈希（默认 cost）。
pub fn hash_password(p: &str) -> String {
    bcrypt::hash(p, bcrypt::DEFAULT_COST).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> Auth {
        Auth::new(b"test-secret-32-bytes-long-xxxxxx".to_vec(), None, None)
    }

    #[test]
    fn 默认凭据可登录_错密码拒绝() {
        let a = auth();
        assert!(a.verify_login(DEFAULT_USER, DEFAULT_PASS), "默认账号密码应可登录");
        assert!(!a.verify_login(DEFAULT_USER, "wrong"), "错密码应拒绝");
        assert!(!a.verify_login("hacker", DEFAULT_PASS), "错用户名应拒绝");
    }

    #[test]
    fn jwt_签发后可校验_含用户名() {
        let a = auth();
        let token = a.issue_token("admin", 10_000_000_000); // 远未来 exp
        let claims = a.validate(&token).expect("自签 token 应校验通过");
        assert_eq!(claims.sub, "admin");
    }

    #[test]
    fn jwt_过期_或_篡改_校验失败() {
        let a = auth();
        // exp = 0+43200（1970 年），早已过期
        let expired = a.issue_token("admin", 0);
        assert!(a.validate(&expired).is_none(), "过期 token 应失败");
        // 篡改
        assert!(a.validate("not.a.jwt").is_none(), "非法 token 应失败");
        // 换密钥签的 token 不被本实例接受
        let other = Auth::new(b"another-secret-different-bytes!!".to_vec(), None, None);
        let foreign = other.issue_token("admin", 10_000_000_000);
        assert!(a.validate(&foreign).is_none(), "异密钥 token 应失败");
    }

    #[test]
    fn 改密_旧密码错则拒绝_对则生效() {
        let a = auth();
        assert!(a.change_credential("wrong", None, Some("new")).is_err(), "旧密码错应拒绝");
        let (u, _h) = a
            .change_credential(DEFAULT_PASS, Some("boss"), Some("NewPass@1"))
            .expect("旧密码对应成功");
        assert_eq!(u, "boss");
        // 旧密码失效，新凭据生效
        assert!(!a.verify_login(DEFAULT_USER, DEFAULT_PASS), "改后旧账号应失效");
        assert!(a.verify_login("boss", "NewPass@1"), "新账号密码应可登录");
    }

    #[test]
    fn 改密_仅改密码_保留用户名() {
        let a = auth();
        a.change_credential(DEFAULT_PASS, None, Some("OnlyPass@2"))
            .unwrap();
        assert!(a.verify_login(DEFAULT_USER, "OnlyPass@2"));
    }
}
