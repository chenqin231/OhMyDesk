//! 只读 HTTP 接口：/api/endpoints / /api/sessions / /api/audit
//! M-SRV2：router 挂 CorsLayer::permissive()（admin :5173 跨端口 fetch）
//! M-SRV3：State 同时持 Arc<Hub> + Arc<AuditStore>，endpoints 读内存、audit/sessions 读 DB
//! P-MCP2：/api/endpoints 返回 EndpointView[] 裸数组

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use crate::audit::AuditStore;
use crate::hub::{now_sec, Hub};

/// HTTP layer 的共享状态（M-SRV3）
#[derive(Clone)]
pub struct HttpState {
    pub hub: Arc<Hub>,
    pub audit: Arc<AuditStore>,
}

/// 构建 HTTP 路由，State = HttpState（与 WS router 分别挂 State，最终在 main.rs merge）
pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/api/endpoints", get(list_endpoints))
        .route("/api/sessions", get(list_sessions))
        .route("/api/audit", get(query_audit))
        .layer(CorsLayer::permissive()) // M-SRV2：允许 admin dev :5173 跨端口
        .with_state(state)
}

// ── Handler：/api/endpoints ──────────────────────────────────────────────────

/// 返回 EndpointView[] 裸数组（P-MCP2）；读内存注册表（M-SRV3）
async fn list_endpoints(State(s): State<HttpState>) -> impl IntoResponse {
    let views = s.hub.reg.views(now_sec());
    Json(views)
}

// ── Handler：/api/sessions ───────────────────────────────────────────────────

async fn list_sessions(State(s): State<HttpState>) -> impl IntoResponse {
    let sessions = s.audit.query_sessions().await;
    Json(sessions)
}

// ── Handler：/api/audit ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuditQuery {
    endpoint: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

async fn query_audit(
    State(s): State<HttpState>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let logs = s
        .audit
        .query_audit(q.endpoint.as_deref(), q.from, q.to)
        .await;
    (StatusCode::OK, Json(logs))
}
