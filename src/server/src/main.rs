//! OhMyDesk Server — axum WebSocket 星型中转 + MySQL 文本审计 + MCP HTTP
//! 端口：8765（WS + HTTP 合一）

mod audit;
mod auth;
mod db;
mod handlers;
mod hub;
mod http;
mod login_log;
mod registry;
mod session;
mod settings;

use std::sync::Arc;

use axum::{
    extract::{
        ws::{CloseFrame, Message as WsMsg, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use protocol::Envelope;
use serde::Deserialize;
use tower_http::services::ServeDir;

use audit::AuditStore;
use auth::Auth;
use hub::{now_sec, Hub};
use http::{router as http_router, HttpState};
use registry::Registry;
use session::SessionStore;
use settings::SettingsStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ── DB 降级连接（M-SRV1）───────────────────────────────────────────────
    let db = db::connect().await; // Option<Db>，None 时审计 best-effort 跳过

    // ── 共享状态构造 ─────────────────────────────────────────────────────────
    // 注册表带 DB 持久化：启动回灌历史终端（修复升级/重启后终端列表为空）。
    let reg = Arc::new(Registry::with_db(db.clone()));
    reg.load_from_db(hub::now_sec()).await;
    let sessions = Arc::new(SessionStore::new());
    let audit = Arc::new(AuditStore::new(db.clone()));
    let settings = Arc::new(SettingsStore::new(db));

    // ── 鉴权：JWT secret 取环境（缺省随机，重启失效 token）；凭据取持久化或写死默认 ──
    let secret = std::env::var("OHMYDESK_JWT_SECRET")
        .map(String::into_bytes)
        .unwrap_or_else(|_| {
            tracing::warn!("OHMYDESK_JWT_SECRET 未设置，使用随机密钥（重启后已登录失效）");
            format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).into_bytes()
        });
    let (loaded_user, loaded_hash) = match settings.load_credential().await {
        Some((u, h)) => (Some(u), Some(h)),
        None => (None, None),
    };
    let auth = Arc::new(Auth::new(secret, loaded_user, loaded_hash));
    tracing::info!(
        "管理平台登录账号 {}（默认密码见 auth::DEFAULT_PASS，可在系统设置页修改）",
        auth.current_user()
    );

    let hub = Arc::new(Hub::new(
        Arc::clone(&reg),
        Arc::clone(&sessions),
        Arc::clone(&audit),
    ));

    let http_state = HttpState {
        hub: Arc::clone(&hub),
        audit: Arc::clone(&audit),
        auth: Arc::clone(&auth),
        settings: Arc::clone(&settings),
    };

    // ── 静态托管 admin-web/dist（P-SRV5：单一内网 URL 同时供 UI + API + WS）──────
    //   vite 产物全在 dist/static 下（assetsDir=static），挂 nest_service("/static")；
    //   index.html 读一次缓存。**不能挂 /assets**——会和 SPA 路由 /assets（终端资产页）撞名，
    //   导致刷新 /assets 被静态服务拦截返回 404。
    //   未命中 /ws、/api、/static 的路径（含 / 与 /assets /audit 等前端路由）一律 fallback
    //   回 index.html(200) —— 用 axum 原生 fallback 而非 ServeDir not_found_service
    //   （后者会把状态强戳成 404，破坏 SPA 深链/刷新语义）。
    let web_dir = std::env::var("OHMYDESK_WEB_DIR")
        .unwrap_or_else(|_| "src/admin-web/dist".to_string());
    let static_dir = ServeDir::new(format!("{web_dir}/static"));
    let index_body = std::fs::read_to_string(format!("{web_dir}/index.html")).unwrap_or_default();

    // ── 客户端下载（P-DL）：/download 返回下载页，/downloads/* 提供安装包产物 ──────────
    //   产物与 download.html 放数据卷（OHMYDESK_DOWNLOAD_DIR，默认 {web_dir}/downloads），
    //   与镜像解耦：重建镜像不丢产物，CI 编出的多架构包 scp 到该目录即可热更新。
    //   download.html 每次请求实时读，更新产物列表无需重启容器。
    let download_dir = std::env::var("OHMYDESK_DOWNLOAD_DIR")
        .unwrap_or_else(|_| format!("{web_dir}/downloads"));
    let downloads_service = ServeDir::new(download_dir.clone());
    let download_page_path = format!("{download_dir}/download.html");

    // ── axum Router：
    //   WS 路由挂 State<Arc<Hub>>；HTTP 路由已 with_state 固化为 Router<()>，可被 merge。
    // M-SRV2 CORS 已在 http_router 内层挂好，WS 端点额外挂一层供 admin-web。
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(WsState {
            hub: Arc::clone(&hub),
            auth: Arc::clone(&auth),
        }) // WS handler State = WsState（中枢 + 鉴权）
        .merge(http_router(http_state))
        .nest_service("/static", static_dir)
        .route(
            "/download",
            get(move || {
                let p = download_page_path.clone();
                async move {
                    axum::response::Html(
                        tokio::fs::read_to_string(&p)
                            .await
                            .unwrap_or_else(|_| "<h1>下载页未部署</h1>".to_string()),
                    )
                }
            }),
        )
        .nest_service("/downloads", downloads_service)
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

/// WS 路由状态：中枢 + 鉴权（admin 连接需 ?token=<jwt>）。
#[derive(Clone)]
struct WsState {
    hub: Arc<Hub>,
    auth: Arc<Auth>,
}

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
    State(st): State<WsState>,
) -> Response {
    // admin 连接需带有效 token；agent（终端）连接无需。
    let token_present = q.token.is_some();
    let authed = q
        .token
        .as_deref()
        .and_then(|t| st.auth.validate(t))
        .is_some();
    ws.on_upgrade(move |sock| handle_socket(sock, st.hub, authed, token_present))
}

async fn handle_socket(socket: WebSocket, hub: Arc<Hub>, authed: bool, token_present: bool) {
    let (mut sink, mut stream) = socket.split();

    // 带了 token 但校验失败（如已过期）→ 立即以 close code 1008(Policy Violation) 关闭。
    // admin-web 监听该 code → 清登录态跳登录页（token 过期自动重新登录）。
    if token_present && !authed {
        let _ = sink
            .send(WsMsg::Close(Some(CloseFrame {
                code: 1008,
                reason: "token 无效或已过期".into(),
            })))
            .await;
        return;
    }

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
            // 鉴权闸：admin 连接必须带有效 token，否则丢弃（防公网未授权操控内网终端）。
            if id.starts_with("admin-") && !authed {
                tracing::warn!("拒绝未认证 admin 连接: {id}");
                break;
            }
            my_id = Some(id.clone());
            hub.add_client(id.clone(), tx.clone());
            // admin 连上立即推一次终端列表
            if id.starts_with("admin-") {
                hub.push_list(now_sec());
            }
            tracing::info!("客户端连接: {id}");
        }

        // 防伪造：连接 id 绑定后，每条消息的 from 必须等于该 id，否则丢弃。
        // 否则非 admin 连接可在后续消息伪造 from:"admin-" 发 force 强控，绕过被控同意。
        if my_id.as_deref() != Some(env.from.as_str()) {
            tracing::warn!("拒绝 from 伪造: bound={:?} got={}", my_id, env.from);
            continue;
        }

        hub.handle(env, now_sec()).await;
    }

    if let Some(id) = &my_id {
        hub.remove_client(id);
        tracing::info!("客户端断开: {id}");
    }
    pump.abort();
}
