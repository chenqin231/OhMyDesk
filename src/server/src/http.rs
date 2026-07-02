//! HTTP 接口：登录鉴权 + 只读查询。
//! M-SRV2：CorsLayer::permissive()（admin :5173 跨端口 fetch）
//! 鉴权：/api/login 公开；其余 /api/* 需 Bearer JWT（AuthUser 提取器，401 拦截）。
//! P-MCP2：/api/endpoints 返回 EndpointView[] 裸数组。

use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    async_trait,
    extract::{rejection::JsonRejection, ConnectInfo, FromRequestParts, Path, Query, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;

use crate::audit::AuditStore;
use crate::auth::Auth;
use crate::hub::{now_sec, Hub};
use crate::login_log::LoginLogStore;
use crate::users::{Permission, PermissionSet, Role, UserRecord, UserStore};

/// HTTP layer 的共享状态（M-SRV3 + 鉴权）
#[derive(Clone)]
pub struct HttpState {
    pub hub: Arc<Hub>,
    pub audit: Arc<AuditStore>,
    pub auth: Arc<Auth>,
    pub login_log: Arc<LoginLogStore>,
    pub users: Arc<UserStore>,
}

/// 已认证管理员（提取器）：校验 Authorization: Bearer <jwt>，失败 401。
pub struct AuthUser {
    pub id: String,
    pub username: String,
    // role：superadmin/user 门面（tier 判据）。运行期权限一律读 permissions。
    pub role: Role,
    // permissions：按账户菜单权限集，运行期权限的真源（superadmin 为隐式全集，validate 组装）。
    pub permissions: PermissionSet,
}

impl AuthUser {
    fn can(&self, permission: Permission) -> bool {
        self.permissions.contains(permission)
    }

    fn is_superadmin(&self) -> bool {
        self.role == Role::Superadmin
    }

    /// tier 字符串：superadmin / user。
    fn tier(&self) -> &'static str {
        if self.is_superadmin() {
            "superadmin"
        } else {
            "user"
        }
    }
}

#[async_trait]
impl FromRequestParts<HttpState> for AuthUser {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &HttpState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| unauth("缺少 token"))?;
        match state.auth.validate(token).await {
            Some(user) => Ok(user),
            None => Err(unauth("token 无效或已过期")),
        }
    }
}

fn unauth(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg })))
}

fn forbidden(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::FORBIDDEN, Json(json!({ "error": msg })))
}

fn bad_request(msg: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.to_string() })),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": msg })))
}

fn require(
    user: &AuthUser,
    permission: Permission,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if user.can(permission) {
        Ok(())
    } else {
        Err(forbidden("权限不足"))
    }
}

/// 用户对外视图 JSON：输出 tier + permissions（按账户权限模型），绝不泄露 password_hash。
/// 供 list_users / create_user / update_user / reset_user_password 统一消费。
fn user_view_json(u: &UserRecord) -> serde_json::Value {
    json!({
        "id": u.id,
        "username": u.username,
        "tier": u.tier(),
        "permissions": u.permissions.keys(),
        "enabled": u.enabled,
        "created_at": u.created_at,
        "updated_at": u.updated_at,
    })
}

/// 提取客户端真实 IP：优先 X-Forwarded-For（取首个）→ X-Real-IP → 直连对端。
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    peer.ip().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use crate::session::SessionStore;
    use crate::users::{PermissionSet, Role, UserStore};
    use axum::body::to_bytes;
    use axum::http::HeaderName;
    use protocol::EndpointInfo;
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // 新权限模型 fixture：CHECK(superadmin/user) + permissions 列，与生产 schema 对齐。
    // Task3 已把本模块测试从旧 create(Role) 迁到 create_user_v2 + tier/permissions 断言。
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

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    async fn test_state() -> (HttpState, Arc<UserStore>) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(USERS_DDL).execute(&pool).await.unwrap();
        let users = Arc::new(UserStore::new(pool));
        let reg = Arc::new(Registry::with_db(None));
        let sessions = Arc::new(SessionStore::new());
        let audit = Arc::new(AuditStore::new(None));
        let hub = Arc::new(Hub::new(
            Arc::clone(&reg),
            Arc::clone(&sessions),
            Arc::clone(&audit),
        ));
        let state = HttpState {
            hub,
            audit,
            auth: Arc::new(Auth::new(
                b"test-secret-32-bytes-long-xxxxxx".to_vec(),
                Arc::clone(&users),
            )),
            login_log: Arc::new(LoginLogStore::new(None)),
            users: Arc::clone(&users),
        };
        (state, users)
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn raw_request(
        addr: SocketAddr,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> (StatusCode, String) {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let auth_header = token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{auth_header}\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        (StatusCode::from_u16(status).unwrap(), body.to_string())
    }

    async fn spawn_test_server(state: HttpState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (addr, handle)
    }

    async fn login_token(addr: SocketAddr, user: &str, pass: &str) -> String {
        let (status, body) = raw_request(
            addr,
            "POST",
            "/api/login",
            None,
            &json!({ "user": user, "pass": pass }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        body["token"].as_str().unwrap().to_string()
    }

    fn auth_user(
        id: impl Into<String>,
        username: impl Into<String>,
        role: Role,
        permissions: PermissionSet,
    ) -> AuthUser {
        AuthUser {
            id: id.into(),
            username: username.into(),
            role,
            permissions,
        }
    }

    // 普通账户：拥有全部可配菜单（含 manage_assets，不含账户管理 manage_users）。
    fn admin_user() -> AuthUser {
        auth_user(
            "admin-id",
            "admin",
            Role::Admin,
            PermissionSet::parse(
                "view_assets,manage_assets,view_grid,use_remote,view_audit,view_login_logs",
            ),
        )
    }

    fn superadmin_user() -> AuthUser {
        auth_user(
            "superadmin-id",
            "superadmin",
            Role::Superadmin,
            PermissionSet::superadmin_all(),
        )
    }

    fn operator_user() -> AuthUser {
        auth_user(
            "operator-id",
            "operator",
            Role::Operator,
            PermissionSet::parse("view_assets,view_grid,use_remote"),
        )
    }

    fn auditor_user() -> AuthUser {
        auth_user(
            "auditor-id",
            "auditor",
            Role::Auditor,
            PermissionSet::parse("view_audit,view_login_logs"),
        )
    }

    #[test]
    fn xff_takes_first() {
        let h = hm(&[("x-forwarded-for", "203.0.113.9, 10.0.0.1")]);
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&h, peer), "203.0.113.9");
    }

    #[test]
    fn falls_back_to_real_ip_then_peer() {
        let h = hm(&[("x-real-ip", "198.51.100.7")]);
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&h, peer), "198.51.100.7");
        let empty = HeaderMap::new();
        assert_eq!(client_ip(&empty, peer), "127.0.0.1");
    }

    #[tokio::test]
    async fn login_response_contains_token_user_tier_and_permissions() {
        let (state, users) = test_state().await;
        users
            .create_user_v2(
                "operator",
                "secret",
                &["view_assets", "view_grid", "use_remote"],
            )
            .await
            .unwrap();

        let response = login(
            State(state),
            ConnectInfo("127.0.0.1:5000".parse().unwrap()),
            HeaderMap::new(),
            Json(LoginReq {
                user: "operator".to_string(),
                pass: "secret".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body["token"].as_str().unwrap_or_default().len() > 20);
        assert_eq!(body["user"], "operator");
        assert_eq!(body["tier"], "user");
        assert_eq!(
            body["permissions"],
            json!(["view_assets", "view_grid", "use_remote"])
        );
    }

    #[tokio::test]
    async fn me_returns_stored_permissions_and_superadmin_gets_all() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap(); // superadmin / infogo123
        users
            .create_user_v2("viewer", "secret", &["view_audit"])
            .await
            .unwrap();
        let (addr, server) = spawn_test_server(state).await;

        // superadmin → tier=superadmin，permissions 全集（含 manage_users）
        let sa_token = login_token(addr, "superadmin", "infogo123").await;
        let (status, body) = raw_request(addr, "GET", "/api/me", Some(&sa_token), "").await;
        assert_eq!(status, StatusCode::OK);
        let sa: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sa["user"], "superadmin");
        assert_eq!(sa["tier"], "superadmin");
        assert_eq!(
            sa["permissions"],
            json!([
                "view_assets",
                "manage_assets",
                "view_grid",
                "use_remote",
                "view_audit",
                "view_login_logs",
                "manage_users"
            ])
        );

        // 普通账户（仅 view_audit）→ tier=user，permissions 只含 view_audit
        let v_token = login_token(addr, "viewer", "secret").await;
        let (status, body) = raw_request(addr, "GET", "/api/me", Some(&v_token), "").await;
        assert_eq!(status, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["user"], "viewer");
        assert_eq!(v["tier"], "user");
        assert_eq!(v["permissions"], json!(["view_audit"]));

        server.abort();
    }

    #[tokio::test]
    async fn operator_can_list_endpoints() {
        let (state, _users) = test_state().await;

        let response = list_endpoints(State(state), operator_user())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, json!([]));
    }

    #[tokio::test]
    async fn operator_cannot_delete_endpoints() {
        let (state, _users) = test_state().await;
        state
            .hub
            .reg
            .upsert(EndpointInfo::sample(), "123456".to_string(), now_sec());

        let response = delete_endpoints(
            State(state.clone()),
            operator_user(),
            Json(DeleteEndpointsReq {
                ids: vec!["ep-001".to_string()],
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await,
            json!({ "error": "权限不足" })
        );
        assert_eq!(state.hub.reg.views(now_sec()).len(), 1);
    }

    #[tokio::test]
    async fn admin_and_superadmin_can_delete_endpoints() {
        let (state, _users) = test_state().await;
        state
            .hub
            .reg
            .upsert(EndpointInfo::sample(), "123456".to_string(), now_sec());

        let admin_response = delete_endpoints(
            State(state.clone()),
            admin_user(),
            Json(DeleteEndpointsReq {
                ids: vec!["ep-001".to_string()],
            }),
        )
        .await
        .into_response();

        assert_eq!(admin_response.status(), StatusCode::OK);
        assert_eq!(response_json(admin_response).await, json!({ "deleted": 1 }));

        state
            .hub
            .reg
            .upsert(EndpointInfo::sample(), "123456".to_string(), now_sec());
        let superadmin_response = delete_endpoints(
            State(state),
            superadmin_user(),
            Json(DeleteEndpointsReq {
                ids: vec!["ep-001".to_string()],
            }),
        )
        .await
        .into_response();

        assert_eq!(superadmin_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(superadmin_response).await,
            json!({ "deleted": 1 })
        );
    }

    #[tokio::test]
    async fn operator_cannot_view_audit_sessions_audit_logs_or_login_logs() {
        let (state, _users) = test_state().await;

        let sessions = list_sessions(State(state.clone()), operator_user())
            .await
            .into_response();
        let audit_logs = query_audit(
            State(state.clone()),
            operator_user(),
            Query(AuditQuery {
                endpoint: None,
                from: None,
                to: None,
            }),
        )
        .await
        .into_response();
        let login_logs = query_login_logs(
            State(state),
            operator_user(),
            Query(LoginLogQuery {
                limit: None,
                offset: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(sessions.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(sessions).await,
            json!({ "error": "权限不足" })
        );
        assert_eq!(audit_logs.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(audit_logs).await,
            json!({ "error": "权限不足" })
        );
        assert_eq!(login_logs.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(login_logs).await,
            json!({ "error": "权限不足" })
        );
    }

    #[tokio::test]
    async fn auditor_can_view_audit_sessions_audit_logs_and_login_logs() {
        let (state, _users) = test_state().await;

        let sessions = list_sessions(State(state.clone()), auditor_user())
            .await
            .into_response();
        let audit_logs = query_audit(
            State(state.clone()),
            auditor_user(),
            Query(AuditQuery {
                endpoint: None,
                from: None,
                to: None,
            }),
        )
        .await
        .into_response();
        let login_logs = query_login_logs(
            State(state),
            auditor_user(),
            Query(LoginLogQuery {
                limit: None,
                offset: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(sessions.status(), StatusCode::OK);
        assert_eq!(response_json(sessions).await, json!([]));
        assert_eq!(audit_logs.status(), StatusCode::OK);
        assert_eq!(response_json(audit_logs).await, json!([]));
        assert_eq!(login_logs.status(), StatusCode::OK);
        assert_eq!(response_json(login_logs).await, json!([]));
    }

    #[tokio::test]
    async fn auditor_cannot_list_endpoints() {
        let (state, _users) = test_state().await;

        let response = list_endpoints(State(state), auditor_user())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await,
            json!({ "error": "权限不足" })
        );
    }

    #[tokio::test]
    async fn superadmin_can_list_users_without_password_hash() {
        let (state, users) = test_state().await;
        users
            .create_user_v2("auditor", "secret", &["view_audit", "view_login_logs"])
            .await
            .unwrap();

        let response = list_users(State(state), superadmin_user())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body[0]["username"], "auditor");
        assert_eq!(body[0]["tier"], "user");
        assert_eq!(
            body[0]["permissions"],
            json!(["view_audit", "view_login_logs"])
        );
        assert!(body[0].get("password_hash").is_none());
        assert!(body[0].get("role").is_none());
    }

    #[tokio::test]
    async fn superadmin_can_create_regular_user_with_permissions() {
        let (state, users) = test_state().await;

        let response = create_user(
            State(state),
            superadmin_user(),
            Ok(Json(CreateUserReq {
                username: "operator".to_string(),
                password: "secret".to_string(),
                permissions: vec![
                    "view_assets".to_string(),
                    "view_grid".to_string(),
                    "use_remote".to_string(),
                ],
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["tier"], "user");
        assert_eq!(
            body["permissions"],
            json!(["view_assets", "view_grid", "use_remote"])
        );
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());

        let user = users.get_by_username("operator").await.unwrap().unwrap();
        assert!(!user.is_superadmin());
        assert_eq!(user.tier(), "user");
        assert!(!user.enabled);
    }

    #[tokio::test]
    async fn superadmin_can_patch_regular_user_enabled_without_password_hash() {
        let (state, users) = test_state().await;
        let user = users
            .create_user_v2("operator", "secret", &["view_grid"])
            .await
            .unwrap();

        let response = update_user(
            State(state),
            Path(user.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: None,
                username: None,
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["tier"], "user");
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());

        let user = users.get_by_id(&user.id).await.unwrap().unwrap();
        assert!(!user.enabled);
    }

    #[tokio::test]
    async fn superadmin_can_reset_regular_user_password_without_password_hash() {
        let (state, users) = test_state().await;
        let user = users
            .create_user_v2("operator", "old-pass", &["view_grid"])
            .await
            .unwrap();

        let response = reset_user_password(
            State(state),
            Path(user.id.clone()),
            superadmin_user(),
            Ok(Json(ResetPasswordReq {
                password: "new-pass".to_string(),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert!(body.get("password_hash").is_none());

        let user = users.get_by_id(&user.id).await.unwrap().unwrap();
        assert!(bcrypt::verify("new-pass", &user.password_hash).unwrap());
    }

    #[tokio::test]
    async fn operator_cannot_list_users() {
        let (state, _users) = test_state().await;

        let response = list_users(State(state), operator_user())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await,
            json!({ "error": "权限不足" })
        );
    }

    #[tokio::test]
    async fn patch_missing_user_returns_404() {
        let (state, _users) = test_state().await;

        let response = update_user(
            State(state),
            Path("missing-id".to_string()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: Some(vec!["view_grid".to_string()]),
                username: None,
                enabled: None,
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response_json(response).await,
            json!({ "error": "用户不存在" })
        );
    }

    #[tokio::test]
    async fn patch_user_permissions_superadmin_only_and_validated() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap();
        let superadmin = users.get_by_username("superadmin").await.unwrap().unwrap();
        let target = users
            .create_user_v2("op", "secret", &["view_grid"])
            .await
            .unwrap();

        // 普通账户（无 manage_users）PATCH → 403
        let forbidden = update_user(
            State(state.clone()),
            Path(target.id.clone()),
            operator_user(),
            Ok(Json(UpdateUserReq {
                permissions: Some(vec!["view_audit".to_string()]),
                username: None,
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        // superadmin PATCH 合法集 → 200，回读生效（覆盖语义：旧 view_grid 被替换）
        let ok = update_user(
            State(state.clone()),
            Path(target.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: Some(vec!["view_assets".to_string(), "manage_assets".to_string()]),
                username: None,
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(ok.status(), StatusCode::OK);
        let body = response_json(ok).await;
        assert_eq!(body["tier"], "user");
        assert_eq!(body["permissions"], json!(["view_assets", "manage_assets"]));

        // 缺依赖的 manage_assets（无 view_assets）→ 400
        let bad = update_user(
            State(state.clone()),
            Path(target.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: Some(vec!["manage_assets".to_string()]),
                username: None,
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // 改 superadmin 目标降权 → 拒（400）
        let sa_target = update_user(
            State(state),
            Path(superadmin.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: Some(vec!["view_assets".to_string()]),
                username: None,
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(sa_target.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_username_unique_and_superadmin_target_locked() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap();
        let superadmin = users.get_by_username("superadmin").await.unwrap().unwrap();
        let alice = users
            .create_user_v2("alice", "secret", &["view_grid"])
            .await
            .unwrap();
        let bob = users
            .create_user_v2("bob", "secret", &["view_grid"])
            .await
            .unwrap();

        // 改普通账户名 → 200
        let ok = update_user(
            State(state.clone()),
            Path(alice.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: None,
                username: Some("alice2".to_string()),
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(response_json(ok).await["username"], "alice2");

        // 撞名（bob → alice2）→ 400
        let dup = update_user(
            State(state.clone()),
            Path(bob.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: None,
                username: Some("alice2".to_string()),
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(dup.status(), StatusCode::BAD_REQUEST);

        // 改 superadmin 自己登录名 → 拒（400）
        let sa = update_user(
            State(state),
            Path(superadmin.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                permissions: None,
                username: Some("root".to_string()),
                enabled: None,
            })),
        )
        .await
        .into_response();
        assert_eq!(sa.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn self_password_change_verifies_old_and_updates() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap();
        // 仅 view_grid 的普通账户：证明 /api/me/password 只需登录、不需任何菜单权限
        users
            .create_user_v2("alice", "old-pass", &["view_grid"])
            .await
            .unwrap();
        let (addr, server) = spawn_test_server(state).await;
        let token = login_token(addr, "alice", "old-pass").await;

        // 旧密错 → 400
        let (status, _) = raw_request(
            addr,
            "POST",
            "/api/me/password",
            Some(&token),
            &json!({ "old": "wrong-pass", "new": "new-pass" }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // 旧密对 → 200
        let (status, _) = raw_request(
            addr,
            "POST",
            "/api/me/password",
            Some(&token),
            &json!({ "old": "old-pass", "new": "new-pass" }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // 新密可登录（走真实 /api/login 路径，login_token 内部断言 200）
        let new_token = login_token(addr, "alice", "new-pass").await;
        assert!(new_token.len() > 20);
        server.abort();
    }

    #[tokio::test]
    async fn create_user_invalid_permission_returns_400_json_from_router() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap();
        let (addr, server) = spawn_test_server(state).await;
        let token = login_token(addr, "superadmin", "infogo123").await;

        let (status, body) = raw_request(
            addr,
            "POST",
            "/api/users",
            Some(&token),
            r#"{"username":"bad","password":"secret","permissions":["not_a_menu"]}"#,
        )
        .await;

        server.abort();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_str(&body).expect("响应体应为 JSON");
        assert!(body.get("error").is_some());
    }
}

/// 构建 HTTP 路由，State = HttpState（与 WS router 分别挂 State，最终在 main.rs merge）
pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/me/password", post(change_own_password))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:id", patch(update_user))
        .route("/api/users/:id/reset-password", post(reset_user_password))
        .route("/api/endpoints", get(list_endpoints))
        .route("/api/endpoints/delete", post(delete_endpoints))
        .route("/api/sessions", get(list_sessions))
        .route("/api/audit", get(query_audit))
        .route("/api/login-logs", get(query_login_logs))
        .layer(CorsLayer::permissive()) // M-SRV2：允许 admin dev :5173 跨端口
        .with_state(state)
}

// ── 鉴权 Handler ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginReq {
    user: String,
    pass: String,
}

/// POST /api/login → 验证账号密码，签发 JWT；记录登录日志（成功/失败均记）。
async fn login(
    State(s): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> impl IntoResponse {
    let ip = client_ip(&headers, peer);
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if let Some(user) = s.auth.verify_login(&req.user, &req.pass).await {
        let token = s
            .auth
            .issue_token(&user.id, &user.username, user.role, now_sec());
        s.login_log
            .record(&user.username, Some(&ip), Some(&ua), true, None)
            .await;
        (
            StatusCode::OK,
            Json(json!({
                "token": token,
                "user": user.username,
                "tier": user.tier(),
                "permissions": user.permissions.keys(),
            })),
        )
            .into_response()
    } else {
        s.login_log
            .record(
                &req.user,
                Some(&ip),
                Some(&ua),
                false,
                Some("账号或密码错误"),
            )
            .await;
        unauth("账号或密码错误").into_response()
    }
}

/// GET /api/me（需登录）→ 回请求 token 对应的用户 tier + 权限集。
async fn me(State(_s): State<HttpState>, user: AuthUser) -> impl IntoResponse {
    Json(json!({
        "user": user.username,
        "tier": user.tier(),
        "permissions": user.permissions.keys(),
    }))
}

/// POST /api/me/password（仅需登录，不 require 任何菜单权限）→ 验旧密码后改自己的密码。
#[derive(Deserialize)]
struct SelfPasswordReq {
    old: String,
    new: String,
}

async fn change_own_password(
    State(s): State<HttpState>,
    user: AuthUser,
    Json(req): Json<SelfPasswordReq>,
) -> impl IntoResponse {
    if req.new.trim().is_empty() {
        return bad_request(anyhow!("新密码不能为空")).into_response();
    }
    // 复用 change_credential：先 bcrypt 验旧密码，成功才改；不改用户名（None）。
    match s
        .auth
        .change_credential(&user.id, &req.old, None, Some(&req.new))
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

// ── 用户管理 Handler（均 require ManageUsers = superadmin 独占）─────────────

#[derive(Deserialize)]
struct CreateUserReq {
    username: String,
    password: String,
    #[serde(default)]
    permissions: Vec<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct UpdateUserReq {
    permissions: Option<Vec<String>>,
    username: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct ResetPasswordReq {
    password: String,
}

async fn list_users(State(s): State<HttpState>, user: AuthUser) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageUsers) {
        return err.into_response();
    }

    match s.users.list().await {
        Ok(users) => {
            let views: Vec<serde_json::Value> = users.iter().map(user_view_json).collect();
            (StatusCode::OK, Json(json!(views))).into_response()
        }
        Err(e) => bad_request(e).into_response(),
    }
}

async fn create_user(
    State(s): State<HttpState>,
    user: AuthUser,
    req: Result<Json<CreateUserReq>, JsonRejection>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageUsers) {
        return err.into_response();
    }
    let Json(req) = match req {
        Ok(req) => req,
        Err(_) => return bad_request(anyhow!("请求体格式错误")).into_response(),
    };

    // 走 create_user_v2（tier=user + 校验 ASSIGNABLE + manage_assets⇒view_assets），
    // 彻底消除旧 create(Role) 撞 CHECK(superadmin/user) 的隐患。
    let perms: Vec<&str> = req.permissions.iter().map(String::as_str).collect();
    let created = match s
        .users
        .create_user_v2(&req.username, &req.password, &perms)
        .await
    {
        Ok(user) => user,
        Err(e) => return bad_request(e).into_response(),
    };
    if req.enabled == Some(false) {
        if let Err(e) = s.users.set_enabled(&created.id, false).await {
            return bad_request(e).into_response();
        }
    }

    match s.users.get_by_id(&created.id).await {
        Ok(Some(user)) => (StatusCode::OK, Json(user_view_json(&user))).into_response(),
        Ok(None) => not_found("用户不存在").into_response(),
        Err(e) => bad_request(e).into_response(),
    }
}

async fn update_user(
    State(s): State<HttpState>,
    Path(id): Path<String>,
    user: AuthUser,
    req: Result<Json<UpdateUserReq>, JsonRejection>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageUsers) {
        return err.into_response();
    }
    let Json(req) = match req {
        Ok(req) => req,
        Err(_) => return bad_request(anyhow!("请求体格式错误")).into_response(),
    };
    match s.users.get_by_id(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("用户不存在").into_response(),
        Err(e) => return bad_request(e).into_response(),
    }

    // 分别调 set_permissions / set_username / set_enabled；三者内部对 superadmin 目标各自拒改
    // （改权限/改名/停用），故 superadmin 天然被锁。
    if let Some(perms) = &req.permissions {
        let perms: Vec<&str> = perms.iter().map(String::as_str).collect();
        if let Err(e) = s.users.set_permissions(&id, &perms).await {
            return bad_request(e).into_response();
        }
    }
    if let Some(username) = &req.username {
        if let Err(e) = s.users.set_username(&id, username).await {
            return bad_request(e).into_response();
        }
    }
    if let Some(enabled) = req.enabled {
        if let Err(e) = s.users.set_enabled(&id, enabled).await {
            return bad_request(e).into_response();
        }
    }

    match s.users.get_by_id(&id).await {
        Ok(Some(user)) => (StatusCode::OK, Json(user_view_json(&user))).into_response(),
        Ok(None) => not_found("用户不存在").into_response(),
        Err(e) => bad_request(e).into_response(),
    }
}

async fn reset_user_password(
    State(s): State<HttpState>,
    Path(id): Path<String>,
    user: AuthUser,
    req: Result<Json<ResetPasswordReq>, JsonRejection>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageUsers) {
        return err.into_response();
    }
    let Json(req) = match req {
        Ok(req) => req,
        Err(_) => return bad_request(anyhow!("请求体格式错误")).into_response(),
    };

    if let Err(e) = s.users.reset_password(&id, &req.password).await {
        return bad_request(e).into_response();
    }
    match s.users.get_by_id(&id).await {
        Ok(Some(user)) => (StatusCode::OK, Json(user_view_json(&user))).into_response(),
        Ok(None) => not_found("用户不存在").into_response(),
        Err(e) => bad_request(e).into_response(),
    }
}

// ── 只读查询 Handler（均需登录）──────────────────────────────────────────────

/// 返回 EndpointView[] 裸数组（P-MCP2）；读内存注册表（M-SRV3）
async fn list_endpoints(State(s): State<HttpState>, user: AuthUser) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ViewAssets) {
        return err.into_response();
    }

    let views = s.hub.reg.views(now_sec());
    Json(views).into_response()
}

#[derive(Deserialize)]
struct DeleteEndpointsReq {
    ids: Vec<String>,
}

/// POST /api/endpoints/delete（需登录）→ 从注册表删除指定终端（单个/批量），
/// 删完推送最新 endpoint_list 给所有 admin（列表即时刷新）。返回实际删除条数。
async fn delete_endpoints(
    State(s): State<HttpState>,
    user: AuthUser,
    Json(req): Json<DeleteEndpointsReq>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageAssets) {
        return err.into_response();
    }

    let deleted = req.ids.iter().filter(|id| s.hub.reg.remove(id)).count();
    s.hub.push_list(now_sec()); // 广播刷新后的列表给所有 admin
    (StatusCode::OK, Json(json!({ "deleted": deleted }))).into_response()
}

async fn list_sessions(State(s): State<HttpState>, user: AuthUser) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ViewAudit) {
        return err.into_response();
    }

    let sessions = s.audit.query_sessions().await;
    Json(sessions).into_response()
}

#[derive(Deserialize)]
pub struct AuditQuery {
    endpoint: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

async fn query_audit(
    State(s): State<HttpState>,
    user: AuthUser,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ViewAudit) {
        return err.into_response();
    }

    let logs = s
        .audit
        .query_audit(q.endpoint.as_deref(), q.from, q.to)
        .await;
    (StatusCode::OK, Json(logs)).into_response()
}

#[derive(Deserialize)]
pub struct LoginLogQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

/// GET /api/login-logs?limit=&offset=（需登录）→ 倒序分页返回登录日志。
async fn query_login_logs(
    State(s): State<HttpState>,
    user: AuthUser,
    Query(q): Query<LoginLogQuery>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ViewLoginLogs) {
        return err.into_response();
    }

    let logs = s
        .login_log
        .query(q.limit.unwrap_or(100), q.offset.unwrap_or(0))
        .await;
    (StatusCode::OK, Json(logs)).into_response()
}
