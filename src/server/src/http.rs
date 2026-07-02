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
use crate::users::{Permission, Role, UserStore};

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
    pub role: Role,
}

impl AuthUser {
    fn can(&self, permission: Permission) -> bool {
        self.role.permissions().contains(&permission)
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

fn permissions(role: Role) -> Vec<&'static str> {
    role.permissions().iter().map(|p| p.as_str()).collect()
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
    use crate::users::{Role, UserStore};
    use axum::body::to_bytes;
    use axum::http::HeaderName;
    use protocol::EndpointInfo;
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

    fn auth_user(id: impl Into<String>, username: impl Into<String>, role: Role) -> AuthUser {
        AuthUser {
            id: id.into(),
            username: username.into(),
            role,
        }
    }

    fn admin_user() -> AuthUser {
        auth_user("admin-id", "admin", Role::Admin)
    }

    fn superadmin_user() -> AuthUser {
        auth_user("superadmin-id", "superadmin", Role::Superadmin)
    }

    fn operator_user() -> AuthUser {
        auth_user("operator-id", "operator", Role::Operator)
    }

    fn auditor_user() -> AuthUser {
        auth_user("auditor-id", "auditor", Role::Auditor)
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
    async fn login_response_contains_token_user_role_and_permissions() {
        let (state, users) = test_state().await;
        users
            .create("operator", "secret", Role::Operator)
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
        assert_eq!(body["role"], "operator");
        assert_eq!(
            body["permissions"],
            json!(["view_assets", "view_grid", "use_remote"])
        );
    }

    #[tokio::test]
    async fn me_response_uses_authenticated_user_identity() {
        let (state, users) = test_state().await;
        let user = users
            .create("auditor", "secret", Role::Auditor)
            .await
            .unwrap();
        let auth_user = AuthUser {
            id: user.id,
            username: user.username,
            role: user.role,
        };

        let response = me(State(state), auth_user).await.into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["user"], "auditor");
        assert_eq!(body["role"], "auditor");
        assert_eq!(
            body["permissions"],
            json!(["view_audit", "view_login_logs"])
        );
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
    async fn admin_can_list_users_without_password_hash() {
        let (state, users) = test_state().await;
        users
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        let response = list_users(State(state), admin_user()).await.into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["username"], "operator");
        assert_eq!(body[0]["role"], "operator");
        assert!(body[0].get("password_hash").is_none());
    }

    #[tokio::test]
    async fn superadmin_can_list_users_without_password_hash() {
        let (state, users) = test_state().await;
        users
            .create("auditor", "secret", Role::Auditor)
            .await
            .unwrap();

        let response = list_users(State(state), superadmin_user())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body[0]["username"], "auditor");
        assert!(body[0].get("password_hash").is_none());
    }

    #[tokio::test]
    async fn superadmin_can_create_regular_user_without_password_hash() {
        let (state, users) = test_state().await;

        let response = create_user(
            State(state),
            superadmin_user(),
            Ok(Json(CreateUserReq {
                username: "operator".to_string(),
                password: "secret".to_string(),
                role: Role::Operator,
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["role"], "operator");
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());

        let user = users.get_by_username("operator").await.unwrap().unwrap();
        assert_eq!(user.role, Role::Operator);
        assert!(!user.enabled);
    }

    #[tokio::test]
    async fn superadmin_can_patch_regular_user_role_and_enabled_without_password_hash() {
        let (state, users) = test_state().await;
        let user = users
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        let response = update_user(
            State(state),
            axum::extract::Path(user.id.clone()),
            superadmin_user(),
            Ok(Json(UpdateUserReq {
                role: Some(Role::Auditor),
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["role"], "auditor");
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());

        let user = users.get_by_id(&user.id).await.unwrap().unwrap();
        assert_eq!(user.role, Role::Auditor);
        assert!(!user.enabled);
    }

    #[tokio::test]
    async fn superadmin_can_reset_regular_user_password_without_password_hash() {
        let (state, users) = test_state().await;
        let user = users
            .create("operator", "old-pass", Role::Operator)
            .await
            .unwrap();

        let response = reset_user_password(
            State(state),
            axum::extract::Path(user.id.clone()),
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
    async fn admin_can_create_operator_and_disabled_user() {
        let (state, _users) = test_state().await;

        let enabled = create_user(
            State(state.clone()),
            admin_user(),
            Ok(Json(CreateUserReq {
                username: "operator".to_string(),
                password: "secret".to_string(),
                role: Role::Operator,
                enabled: None,
            })),
        )
        .await
        .into_response();
        let disabled = create_user(
            State(state),
            admin_user(),
            Ok(Json(CreateUserReq {
                username: "disabled".to_string(),
                password: "secret".to_string(),
                role: Role::Operator,
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(enabled.status(), StatusCode::OK);
        let body = response_json(enabled).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["role"], "operator");
        assert_eq!(body["enabled"], true);
        assert!(body.get("password_hash").is_none());

        assert_eq!(disabled.status(), StatusCode::OK);
        let body = response_json(disabled).await;
        assert_eq!(body["username"], "disabled");
        assert_eq!(body["role"], "operator");
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());
    }

    #[tokio::test]
    async fn admin_can_patch_regular_user_role_and_enabled() {
        let (state, users) = test_state().await;
        let user = users
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        let response = update_user(
            State(state),
            axum::extract::Path(user.id),
            admin_user(),
            Ok(Json(UpdateUserReq {
                role: Some(Role::Auditor),
                enabled: Some(false),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["username"], "operator");
        assert_eq!(body["role"], "auditor");
        assert_eq!(body["enabled"], false);
        assert!(body.get("password_hash").is_none());
    }

    #[tokio::test]
    async fn admin_patch_missing_user_returns_404() {
        let (state, _users) = test_state().await;

        let response = update_user(
            State(state),
            axum::extract::Path("missing-id".to_string()),
            admin_user(),
            Ok(Json(UpdateUserReq {
                role: Some(Role::Auditor),
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
    async fn admin_can_reset_regular_user_password_without_leaking_hash() {
        let (state, users) = test_state().await;
        let user = users
            .create("operator", "old-pass", Role::Operator)
            .await
            .unwrap();

        let response = reset_user_password(
            State(state),
            axum::extract::Path(user.id.clone()),
            admin_user(),
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
    async fn admin_cannot_create_or_promote_superadmin() {
        let (state, users) = test_state().await;
        let user = users
            .create("operator", "secret", Role::Operator)
            .await
            .unwrap();

        let create_response = create_user(
            State(state.clone()),
            admin_user(),
            Ok(Json(CreateUserReq {
                username: "root".to_string(),
                password: "secret".to_string(),
                role: Role::Superadmin,
                enabled: None,
            })),
        )
        .await
        .into_response();
        let promote_response = update_user(
            State(state),
            axum::extract::Path(user.id),
            admin_user(),
            Ok(Json(UpdateUserReq {
                role: Some(Role::Superadmin),
                enabled: None,
            })),
        )
        .await
        .into_response();

        assert_eq!(create_response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(promote_response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_user_invalid_role_returns_400_json_from_router() {
        let (state, users) = test_state().await;
        users.bootstrap(None).await.unwrap();
        let (addr, server) = spawn_test_server(state).await;
        let token = login_token(addr, "superadmin", "infogo123").await;

        let (status, body) = raw_request(
            addr,
            "POST",
            "/api/users",
            Some(&token),
            r#"{"username":"bad-role","password":"secret","role":"root","enabled":true}"#,
        )
        .await;

        server.abort();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_str(&body).expect("响应体应为 JSON");
        assert!(body.get("error").is_some());
    }

    #[tokio::test]
    async fn change_credential_requires_manage_settings_permission() {
        let (state, users) = test_state().await;
        let admin = users
            .create("admin", "old-pass", Role::Admin)
            .await
            .unwrap();

        let forbidden = change_credential(
            State(state.clone()),
            operator_user(),
            Json(CredReq {
                current_pass: "old-pass".to_string(),
                new_user: None,
                new_pass: None,
            }),
        )
        .await
        .into_response();
        let allowed = change_credential(
            State(state),
            auth_user(admin.id, admin.username, admin.role),
            Json(CredReq {
                current_pass: "old-pass".to_string(),
                new_user: None,
                new_pass: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(forbidden).await,
            json!({ "error": "权限不足" })
        );
        assert_eq!(allowed.status(), StatusCode::OK);
    }
}

/// 构建 HTTP 路由，State = HttpState（与 WS router 分别挂 State，最终在 main.rs merge）
pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:id", patch(update_user))
        .route("/api/users/:id/reset-password", post(reset_user_password))
        .route("/api/settings/credential", post(change_credential))
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
                "role": user.role.as_str(),
                "permissions": permissions(user.role),
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

/// GET /api/me（需登录）→ 回请求 token 对应的用户身份。
async fn me(State(_s): State<HttpState>, user: AuthUser) -> impl IntoResponse {
    Json(json!({
        "user": user.username,
        "role": user.role.as_str(),
        "permissions": permissions(user.role),
    }))
}

// ── 用户管理 Handler ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateUserReq {
    username: String,
    password: String,
    role: Role,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct UpdateUserReq {
    role: Option<Role>,
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
        Ok(users) => (StatusCode::OK, Json(json!(users))).into_response(),
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

    let created = match s.users.create(&req.username, &req.password, req.role).await {
        Ok(user) => user,
        Err(e) => return bad_request(e).into_response(),
    };
    if req.enabled == Some(false) {
        if let Err(e) = s.users.set_enabled(&created.id, false).await {
            return bad_request(e).into_response();
        }
    }

    match s.users.get_by_id(&created.id).await {
        Ok(Some(user)) => (StatusCode::OK, Json(json!(user))).into_response(),
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

    if let Some(role) = req.role {
        if let Err(e) = s.users.set_role(&id, role).await {
            return bad_request(e).into_response();
        }
    }
    if let Some(enabled) = req.enabled {
        if let Err(e) = s.users.set_enabled(&id, enabled).await {
            return bad_request(e).into_response();
        }
    }

    match s.users.get_by_id(&id).await {
        Ok(Some(user)) => (StatusCode::OK, Json(json!(user))).into_response(),
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
        Ok(Some(user)) => (StatusCode::OK, Json(json!(user))).into_response(),
        Ok(None) => not_found("用户不存在").into_response(),
        Err(e) => bad_request(e).into_response(),
    }
}

#[derive(Deserialize)]
struct CredReq {
    current_pass: String,
    new_user: Option<String>,
    new_pass: Option<String>,
}

/// POST /api/settings/credential（需登录）→ 当前用户验旧密码后修改自己的账号密码。
async fn change_credential(
    State(s): State<HttpState>,
    user: AuthUser,
    Json(req): Json<CredReq>,
) -> impl IntoResponse {
    if let Err(err) = require(&user, Permission::ManageSettings) {
        return err.into_response();
    }

    match s
        .auth
        .change_credential(
            &user.id,
            &req.current_pass,
            req.new_user.as_deref(),
            req.new_pass.as_deref(),
        )
        .await
    {
        Ok(user) => (
            StatusCode::OK,
            Json(json!({
                "user": user.username,
                "role": user.role.as_str(),
                "permissions": permissions(user.role),
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
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
