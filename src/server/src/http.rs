//! HTTP 接口：登录鉴权 + 只读查询。
//! M-SRV2：CorsLayer::permissive()（admin :5173 跨端口 fetch）
//! 鉴权：/api/login 公开；其余 /api/* 需 Bearer JWT（AuthUser 提取器，401 拦截）。
//! P-MCP2：/api/endpoints 返回 EndpointView[] 裸数组。

use std::sync::Arc;

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts, Query, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::auth::Auth;
use crate::audit::AuditStore;
use crate::hub::{now_sec, Hub};
use crate::login_log::LoginLogStore;
use crate::settings::SettingsStore;

/// HTTP layer 的共享状态（M-SRV3 + 鉴权）
#[derive(Clone)]
pub struct HttpState {
    pub hub: Arc<Hub>,
    pub audit: Arc<AuditStore>,
    pub auth: Arc<Auth>,
    pub settings: Arc<SettingsStore>,
    pub login_log: Arc<LoginLogStore>,
}

/// 已认证管理员（提取器）：校验 Authorization: Bearer <jwt>，失败 401。
pub struct AuthUser(#[allow(dead_code)] pub String);

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
        match state.auth.validate(token) {
            Some(c) => Ok(AuthUser(c.sub)),
            None => Err(unauth("token 无效或已过期")),
        }
    }
}

fn unauth(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg })))
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
    use axum::http::HeaderName;

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
}

/// 构建 HTTP 路由，State = HttpState（与 WS router 分别挂 State，最终在 main.rs merge）
pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/me", get(me))
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
    if s.auth.verify_login(&req.user, &req.pass) {
        let token = s.auth.issue_token(&req.user, now_sec());
        s.login_log
            .record(&req.user, Some(&ip), Some(&ua), true, None)
            .await;
        (StatusCode::OK, Json(json!({ "token": token, "user": req.user }))).into_response()
    } else {
        s.login_log
            .record(&req.user, Some(&ip), Some(&ua), false, Some("账号或密码错误"))
            .await;
        unauth("账号或密码错误").into_response()
    }
}

/// GET /api/me（需登录）→ 回当前用户名（系统设置页回显）。
async fn me(State(s): State<HttpState>, _user: AuthUser) -> impl IntoResponse {
    Json(json!({ "user": s.auth.current_user() }))
}

#[derive(Deserialize)]
struct CredReq {
    current_pass: String,
    new_user: Option<String>,
    new_pass: Option<String>,
}

/// POST /api/settings/credential（需登录）→ 验旧密码后改账号密码 + 落库。
async fn change_credential(
    State(s): State<HttpState>,
    _user: AuthUser,
    Json(req): Json<CredReq>,
) -> impl IntoResponse {
    match s.auth.change_credential(
        &req.current_pass,
        req.new_user.as_deref(),
        req.new_pass.as_deref(),
    ) {
        Ok((user, hash)) => {
            s.settings.save_credential(&user, &hash).await;
            (StatusCode::OK, Json(json!({ "user": user }))).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

// ── 只读查询 Handler（均需登录）──────────────────────────────────────────────

/// 返回 EndpointView[] 裸数组（P-MCP2）；读内存注册表（M-SRV3）
async fn list_endpoints(State(s): State<HttpState>, _user: AuthUser) -> impl IntoResponse {
    let views = s.hub.reg.views(now_sec());
    Json(views)
}

#[derive(Deserialize)]
struct DeleteEndpointsReq {
    ids: Vec<String>,
}

/// POST /api/endpoints/delete（需登录）→ 从注册表删除指定终端（单个/批量），
/// 删完推送最新 endpoint_list 给所有 admin（列表即时刷新）。返回实际删除条数。
async fn delete_endpoints(
    State(s): State<HttpState>,
    _user: AuthUser,
    Json(req): Json<DeleteEndpointsReq>,
) -> impl IntoResponse {
    let deleted = req.ids.iter().filter(|id| s.hub.reg.remove(id)).count();
    s.hub.push_list(now_sec()); // 广播刷新后的列表给所有 admin
    (StatusCode::OK, Json(json!({ "deleted": deleted })))
}

async fn list_sessions(State(s): State<HttpState>, _user: AuthUser) -> impl IntoResponse {
    let sessions = s.audit.query_sessions().await;
    Json(sessions)
}

#[derive(Deserialize)]
pub struct AuditQuery {
    endpoint: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

async fn query_audit(
    State(s): State<HttpState>,
    _user: AuthUser,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let logs = s
        .audit
        .query_audit(q.endpoint.as_deref(), q.from, q.to)
        .await;
    (StatusCode::OK, Json(logs))
}

#[derive(Deserialize)]
pub struct LoginLogQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

/// GET /api/login-logs?limit=&offset=（需登录）→ 倒序分页返回登录日志。
async fn query_login_logs(
    State(s): State<HttpState>,
    _user: AuthUser,
    Query(q): Query<LoginLogQuery>,
) -> impl IntoResponse {
    let logs = s
        .login_log
        .query(q.limit.unwrap_or(100), q.offset.unwrap_or(0))
        .await;
    (StatusCode::OK, Json(logs))
}
