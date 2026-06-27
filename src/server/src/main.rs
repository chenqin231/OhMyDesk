//! OhMyDesk Server — axum WebSocket 星型中转 + MySQL 文本审计 + MCP HTTP
//! 端口：8765（WS + HTTP 合一）

mod audit;
mod db;
mod handlers;
mod hub;
mod http;
mod registry;
mod session;

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message as WsMsg, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use protocol::Envelope;
use tower_http::services::ServeDir;

use audit::AuditStore;
use hub::{now_sec, Hub};
use http::{router as http_router, HttpState};
use registry::Registry;
use session::SessionStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ── DB 降级连接（M-SRV1）───────────────────────────────────────────────
    let db = db::connect().await; // Option<Db>，None 时审计 best-effort 跳过

    // ── 共享状态构造 ─────────────────────────────────────────────────────────
    let reg = Arc::new(Registry::new());
    let sessions = Arc::new(SessionStore::new());
    let audit = Arc::new(AuditStore::new(db));

    let hub = Arc::new(Hub::new(
        Arc::clone(&reg),
        Arc::clone(&sessions),
        Arc::clone(&audit),
    ));

    let http_state = HttpState {
        hub: Arc::clone(&hub),
        audit: Arc::clone(&audit),
    };

    // ── 静态托管 admin-web/dist（P-SRV5：单一内网 URL 同时供 UI + API + WS）──────
    //   vite 产物全在 dist/assets 下，挂 nest_service("/assets")；index.html 读一次缓存。
    //   未命中 /ws、/api、/assets 的路径（含 / 与 /audit /grid 等前端路由）一律 fallback
    //   回 index.html(200) —— 用 axum 原生 fallback 而非 ServeDir not_found_service
    //   （后者会把状态强戳成 404，破坏 SPA 深链/刷新语义）。
    let web_dir = std::env::var("OHMYDESK_WEB_DIR")
        .unwrap_or_else(|_| "src/admin-web/dist".to_string());
    let assets_dir = ServeDir::new(format!("{web_dir}/assets"));
    let index_body = std::fs::read_to_string(format!("{web_dir}/index.html")).unwrap_or_default();

    // ── axum Router：
    //   WS 路由挂 State<Arc<Hub>>；HTTP 路由已 with_state 固化为 Router<()>，可被 merge。
    // M-SRV2 CORS 已在 http_router 内层挂好，WS 端点额外挂一层供 admin-web。
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(hub) // WS handler State = Arc<Hub>
        .merge(http_router(http_state))
        .nest_service("/assets", assets_dir)
        .fallback(move || {
            let body = index_body.clone();
            async move { axum::response::Html(body) }
        });

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8765").await?;
    tracing::info!(
        "OhMyDesk server on http://0.0.0.0:8765/  (UI + /api/* + /ws)  web_dir={web_dir}"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

// ── WS 连接处理 ───────────────────────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(hub): State<Arc<Hub>>,
) -> Response {
    ws.on_upgrade(move |sock| handle_socket(sock, hub))
}

async fn handle_socket(socket: WebSocket, hub: Arc<Hub>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // 出站泵：从 mpsc 收消息后写到 WS sink
    let pump = tokio::spawn(async move {
        while let Some(s) = rx.recv().await {
            if sink.send(WsMsg::Text(s)).await.is_err() {
                break;
            }
        }
    });

    let mut my_id: Option<String> = None;

    while let Some(msg) = stream.next().await {
        let text = match msg {
            Ok(WsMsg::Text(t)) => t,
            Ok(WsMsg::Close(_)) | Err(_) => break,
            _ => continue,
        };

        let env = match serde_json::from_str::<Envelope>(&text) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("反序列化信封失败: {e}  raw={text}");
                continue;
            }
        };

        // 首条消息登记连接 id
        if my_id.is_none() {
            let id = env.from.clone();
            my_id = Some(id.clone());
            hub.add_client(id.clone(), tx.clone());
            // admin 连上立即推一次终端列表
            if id.starts_with("admin-") {
                hub.push_list(now_sec());
            }
            tracing::info!("客户端连接: {id}");
        }

        hub.handle(env, now_sec()).await;
    }

    if let Some(id) = &my_id {
        hub.remove_client(id);
        tracing::info!("客户端断开: {id}");
    }
    pump.abort();
}
