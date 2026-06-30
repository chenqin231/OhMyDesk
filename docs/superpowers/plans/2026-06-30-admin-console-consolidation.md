# 管理后台收敛 实现计划（Spec A）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给管理后台加「登录日志」菜单（记录管理员登录 IP/时间/UA/成败并分页查看），隐藏「系统设置」菜单，把改密改为服务器端 CLI 子命令。

**Architecture:** SQLite 新增 `login_log` 表；server 新增 `LoginLogStore`（仿 `AuditStore`，无库 no-op）+ `GET /api/login-logs` 查询接口，并在 `POST /api/login` 埋点（IP 优先 `X-Forwarded-For`，经 axum `ConnectInfo` 兜底）；`LoginLogEntry` 走 protocol crate + ts-rs 导出给前端；前端新增页面/菜单/transport 方法，移除系统设置菜单与路由；改密通过 server `set-password` 子命令复用现有 bcrypt + settings 落库。

**Tech Stack:** Rust（axum 0.7 / sqlx 0.8 sqlite / bcrypt / ts-rs）+ React 19 / Zustand / react-router 7 / Base UI / Vite。

**对应 Spec:** `docs/superpowers/specs/2026-06-30-admin-console-consolidation-design.md`

---

## Task 1：新增 login_log 表（DDL）

**Files:**
- Modify: `scripts/db/schema.sqlite.sql`（生效版，追加 DDL）
- Modify: `scripts/db/schema.sql`（MySQL 参考版，同步）

- [ ] **Step 1：在 SQLite schema 追加表**

在 `scripts/db/schema.sqlite.sql` 末尾追加：

```sql
-- 管理员登录日志（功能②）：记录每次登录尝试的 IP/UA/时间/成败
CREATE TABLE IF NOT EXISTS login_log (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  ts         INTEGER NOT NULL,          -- unix 秒
  username   TEXT    NOT NULL,          -- 尝试登录的用户名
  ip         TEXT,                       -- 客户端 IP（可空）
  user_agent TEXT,                       -- User-Agent（可空）
  success    INTEGER NOT NULL,          -- 1 成功 / 0 失败
  reason     TEXT                        -- 失败原因（成功为 NULL）
);
CREATE INDEX IF NOT EXISTS idx_login_log_ts ON login_log(ts);
```

- [ ] **Step 2：在 MySQL schema 同步追加表**

在 `scripts/db/schema.sql` 末尾追加：

```sql
-- 管理员登录日志（功能②）
CREATE TABLE IF NOT EXISTS login_log (
  id         BIGINT AUTO_INCREMENT PRIMARY KEY,
  ts         BIGINT NOT NULL,
  username   VARCHAR(128) NOT NULL,
  ip         VARCHAR(64),
  user_agent VARCHAR(512),
  success    TINYINT NOT NULL,
  reason     VARCHAR(255),
  INDEX idx_login_log_ts (ts)
);
```

- [ ] **Step 3：提交**

```bash
git add scripts/db/schema.sqlite.sql scripts/db/schema.sql
git commit -m "feat(db): 新增 login_log 表(管理员登录日志)"
```

---

## Task 2：protocol 新增 LoginLogEntry 类型 + ts-rs 导出

**Files:**
- Modify: `src/protocol/src/lib.rs`（新增结构体）
- Modify: `src/protocol/src/tests.rs:142-149`（export_all 追加导出）
- 生成: `src/admin-web/src/lib/types/LoginLogEntry.ts`（cargo test 自动产出）

- [ ] **Step 1：在 lib.rs 审计实体区追加结构体**

在 `src/protocol/src/lib.rs` 末尾（`AuditType` 枚举之后、`#[cfg(test)] mod tests;` 之前）追加：

```rust
/// 管理员登录日志条目（功能②；server → admin-web，ts-rs 导出）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginLogEntry {
    pub id: i64,
    pub ts: i64,
    pub username: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub reason: Option<String>,
}
```

- [ ] **Step 2：在 export_all 测试追加导出**

修改 `src/protocol/src/tests.rs` 的 `export_all`（第 143-149 行），在 `Session::export_all_to` 后加一行：

```rust
#[test]
fn export_all() {
    let dir = "../admin-web/src/lib/types";
    EndpointInfo::export_all_to(dir).unwrap();
    Envelope::export_all_to(dir).unwrap();
    AuditLog::export_all_to(dir).unwrap();
    Session::export_all_to(dir).unwrap();
    LoginLogEntry::export_all_to(dir).unwrap(); // 功能②：登录日志类型
}
```

- [ ] **Step 3：运行测试生成 TS 类型**

Run: `cargo test -p protocol export_all`
Expected: PASS，并生成文件 `src/admin-web/src/lib/types/LoginLogEntry.ts`

- [ ] **Step 4：确认生成的 TS 类型**

Run: `cat src/admin-web/src/lib/types/LoginLogEntry.ts`
Expected: 含 `export type LoginLogEntry = { id: number; ts: number; username: string; ip: string | null; user_agent: string | null; success: boolean; reason: string | null; }`（字段名以 ts-rs 实际产出为准）

- [ ] **Step 5：提交**

```bash
git add src/protocol/src/lib.rs src/protocol/src/tests.rs src/admin-web/src/lib/types/LoginLogEntry.ts
git commit -m "feat(protocol): 新增 LoginLogEntry 类型 + ts-rs 导出"
```

---

## Task 3：LoginLogStore（record + query，TDD）

**Files:**
- Create: `src/server/src/login_log.rs`
- Modify: `src/server/src/main.rs:4-12`（追加 `mod login_log;`）

- [ ] **Step 1：写存储模块（含单测）**

创建 `src/server/src/login_log.rs`：

```rust
//! 登录日志存储：管理员登录痕迹落库（IP/UA/时间/成败）。仿 audit::AuditStore。
//! 持 Option<Db>，无库时 no-op + 告警（与审计一致，不阻断登录）。

use protocol::LoginLogEntry;

use crate::db::Db;

pub struct LoginLogStore {
    db: Option<Db>,
}

impl LoginLogStore {
    pub fn new(db: Option<Db>) -> Self {
        LoginLogStore { db }
    }

    fn now_sec() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// 写一条登录日志（best-effort）。username/ua 截断防滥用（失败登录可携带任意串）。
    pub async fn record(
        &self,
        username: &str,
        ip: Option<&str>,
        user_agent: Option<&str>,
        success: bool,
        reason: Option<&str>,
    ) {
        let Some(db) = &self.db else {
            tracing::warn!("登录日志降级，跳过写入: user={username} success={success}");
            return;
        };
        let user = truncate(username, 128);
        let ua = user_agent.map(|s| truncate(s, 512));
        let ts = Self::now_sec();
        if let Err(e) = sqlx::query(
            "INSERT INTO login_log (ts, username, ip, user_agent, success, reason) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(&user)
        .bind(ip)
        .bind(ua)
        .bind(success as i64)
        .bind(reason)
        .execute(db)
        .await
        {
            tracing::warn!("登录日志写入失败（best-effort 跳过）: {e}");
        }
    }

    /// 分页查询（按 ts 倒序）。limit 钳制 [1,200]，offset >=0。
    pub async fn query(&self, limit: i64, offset: i64) -> Vec<LoginLogEntry> {
        let Some(db) = &self.db else {
            return vec![];
        };
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        match sqlx::query_as::<_, LoginLogRow>(
            "SELECT id, ts, username, ip, user_agent, success, reason \
             FROM login_log ORDER BY ts DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await
        {
            Ok(rows) => rows.into_iter().map(LoginLogEntry::from).collect(),
            Err(e) => {
                tracing::warn!("查询 login_log 失败: {e}");
                vec![]
            }
        }
    }
}

/// 按字符（非字节）截断，避免中文 UA 切坏。
fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(sqlx::FromRow)]
struct LoginLogRow {
    id: i64,
    ts: i64,
    username: String,
    ip: Option<String>,
    user_agent: Option<String>,
    success: i64,
    reason: Option<String>,
}

impl From<LoginLogRow> for LoginLogEntry {
    fn from(r: LoginLogRow) -> Self {
        LoginLogEntry {
            id: r.id,
            ts: r.ts,
            username: r.username,
            ip: r.ip,
            user_agent: r.user_agent,
            success: r.success != 0,
            reason: r.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_chars_and_limits() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("一二三四", 2), "一二");
    }

    #[test]
    fn row_to_entry_success_flag() {
        let row = LoginLogRow {
            id: 7,
            ts: 100,
            username: "admin".into(),
            ip: Some("1.2.3.4".into()),
            user_agent: None,
            success: 1,
            reason: None,
        };
        let e = LoginLogEntry::from(row);
        assert_eq!(e.id, 7);
        assert!(e.success);
        assert_eq!(e.ip.as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn record_no_db_is_noop() {
        // 无 DB 时 record 不 panic（best-effort 降级）
        let store = LoginLogStore::new(None);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(store.record("admin", Some("1.1.1.1"), None, true, None));
        rt.block_on(async {
            assert!(store.query(50, 0).await.is_empty());
        });
    }
}
```

- [ ] **Step 2：注册模块**

修改 `src/server/src/main.rs` 顶部模块声明区（第 4-12 行），按字母序插入 `mod login_log;`：

```rust
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
```

- [ ] **Step 3：运行单测**

Run: `cargo test -p server login_log`
Expected: PASS（3 个测试通过）

- [ ] **Step 4：提交**

```bash
git add src/server/src/login_log.rs src/server/src/main.rs
git commit -m "feat(server): 新增 LoginLogStore(登录日志存储)"
```

---

## Task 4：装配 LoginLogStore 到 HttpState

**Files:**
- Modify: `src/server/src/http.rs:25-32`（HttpState 加字段）
- Modify: `src/server/src/main.rs:50,76-81`（构造 + 注入）

- [ ] **Step 1：HttpState 加字段**

修改 `src/server/src/http.rs`，先在顶部 `use crate::audit::AuditStore;` 下方加：

```rust
use crate::login_log::LoginLogStore;
```

再把 `HttpState` 结构体（第 26-32 行）改为：

```rust
#[derive(Clone)]
pub struct HttpState {
    pub hub: Arc<Hub>,
    pub audit: Arc<AuditStore>,
    pub auth: Arc<Auth>,
    pub settings: Arc<SettingsStore>,
    pub login_log: Arc<LoginLogStore>,
}
```

- [ ] **Step 2：main.rs 构造并注入**

在 `src/server/src/main.rs`：顶部 `use audit::AuditStore;` 下方加 `use login_log::LoginLogStore;`。

在构造区（第 50 行 `let audit = ...` 附近）加：

```rust
    let login_log = Arc::new(LoginLogStore::new(db.clone()));
```

> 注意：`db` 在第 51 行 `let settings = Arc::new(SettingsStore::new(db));` 处发生 move（最后一次使用），故 `login_log` 这一行须放在第 51 行**之前**，用 `db.clone()`。

把 `HttpState { ... }`（第 76-81 行）改为追加 `login_log`：

```rust
    let http_state = HttpState {
        hub: Arc::clone(&hub),
        audit: Arc::clone(&audit),
        auth: Arc::clone(&auth),
        settings: Arc::clone(&settings),
        login_log: Arc::clone(&login_log),
    };
```

- [ ] **Step 3：编译验证**

Run: `cargo build -p server`
Expected: 编译通过（暂未使用 login_log 的 record/query 会有 dead_code 警告，Task 6/7 消除）

- [ ] **Step 4：提交**

```bash
git add src/server/src/http.rs src/server/src/main.rs
git commit -m "feat(server): 装配 LoginLogStore 进 HttpState"
```

---

## Task 5：axum ConnectInfo + client_ip 辅助

**Files:**
- Modify: `src/server/src/main.rs:138`（serve 改造）
- Modify: `src/server/src/http.rs`（新增 `client_ip` 辅助 + import）

- [ ] **Step 1：serve 注入 ConnectInfo**

修改 `src/server/src/main.rs` 第 138 行：

```rust
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
```

> 这让任意 handler 可用 `ConnectInfo<SocketAddr>` 提取器拿到直连对端地址；不影响 `/ws`、`/api/*`、静态托管。

- [ ] **Step 2：http.rs 加 client_ip 辅助 + 引入类型**

在 `src/server/src/http.rs` 顶部 `use axum::{...}` 中，给 `extract` 增加 `ConnectInfo`，并引入 `HeaderMap`：

```rust
use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts, Query, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
```

在 `unauth` 函数（第 58-60 行）下方加：

```rust
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
```

- [ ] **Step 3：运行单测**

Run: `cargo test -p server client_ip`
Expected: PASS（`xff_takes_first` + `falls_back_to_real_ip_then_peer`）

- [ ] **Step 4：提交**

```bash
git add src/server/src/main.rs src/server/src/http.rs
git commit -m "feat(server): axum ConnectInfo + client_ip(XFF优先) 辅助"
```

---

## Task 6：登录埋点（记录成功/失败）

**Files:**
- Modify: `src/server/src/http.rs:84-92`（login handler）

- [ ] **Step 1：改写 login handler 加埋点**

把 `src/server/src/http.rs` 的 `login`（第 84-92 行）改为：

```rust
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
```

- [ ] **Step 2：编译验证**

Run: `cargo build -p server`
Expected: 编译通过

- [ ] **Step 3：提交**

```bash
git add src/server/src/http.rs
git commit -m "feat(server): /api/login 埋点记录登录日志(成功+失败)"
```

---

## Task 7：GET /api/login-logs 查询接口

**Files:**
- Modify: `src/server/src/http.rs`（router 注册 + handler）

- [ ] **Step 1：router 注册路由**

在 `src/server/src/http.rs` 的 `router()`（第 63-74 行）中，`/api/audit` 之后加一行：

```rust
        .route("/api/audit", get(query_audit))
        .route("/api/login-logs", get(query_login_logs))
```

- [ ] **Step 2：新增 handler + Query 结构**

在文件末尾（`query_audit` 之后）追加：

```rust
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
```

- [ ] **Step 3：编译验证（dead_code 应消除）**

Run: `cargo build -p server`
Expected: 编译通过，无 login_log 相关 dead_code 警告

- [ ] **Step 4：手测接口鉴权**

Run（需先启动 server，另开终端）:
```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8765/api/login-logs
```
Expected: `401`（无 token 被 AuthUser 拦截）

- [ ] **Step 5：提交**

```bash
git add src/server/src/http.rs
git commit -m "feat(server): 新增 GET /api/login-logs 查询接口"
```

---

## Task 8：set-password CLI 子命令

**Files:**
- Modify: `src/server/src/main.rs`（main 入口拦截 + 新增 run_set_password）

- [ ] **Step 1：main 入口拦截子命令**

在 `src/server/src/main.rs` 的 `main()` 中，`tracing_subscriber::fmt::init();`（第 40 行）之后、`let db = db::connect().await;`（第 43 行）之前插入：

```rust
    // CLI 子命令：set-password —— 复用 bcrypt + settings 落库，改完即退（不起服务）。
    let cli_args: Vec<String> = std::env::args().collect();
    if cli_args.get(1).map(String::as_str) == Some("set-password") {
        return run_set_password(&cli_args).await;
    }
```

- [ ] **Step 2：新增 run_set_password 函数**

在 `src/server/src/main.rs` 文件末尾（`handle_socket` 之后）追加：

```rust
/// `ohmydesk-server set-password <新密码> [--user <用户名>]`
/// 写入 SQLite settings 表（admin_user / admin_pass_hash）；重启 server 生效。
async fn run_set_password(args: &[String]) -> anyhow::Result<()> {
    let new_pass = args
        .get(2)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("用法: ohmydesk-server set-password <新密码> [--user <用户名>]")
        })?;

    // 解析可选 --user
    let mut new_user: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "--user" {
            new_user = args.get(i + 1).cloned();
            i += 2;
        } else {
            i += 1;
        }
    }

    let db = db::connect().await;
    if db.is_none() {
        anyhow::bail!("无法连接数据库，改密未生效；请检查 DATABASE_URL / 数据卷挂载");
    }
    let settings = SettingsStore::new(db);

    // 用户名：--user 指定 > 现有持久化 > 默认 admin
    let user = match new_user {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => settings
            .load_credential()
            .await
            .map(|(u, _)| u)
            .unwrap_or_else(|| auth::DEFAULT_USER.to_string()),
    };

    let hash = auth::hash_password(new_pass);
    settings.save_credential(&user, &hash).await;
    println!("已更新管理员凭据：user={user}（重启 server 生效）");
    Ok(())
}
```

- [ ] **Step 3：编译验证**

Run: `cargo build -p server`
Expected: 编译通过

- [ ] **Step 4：端到端验证改密（需可写 DB）**

Run:
```bash
cd /tmp && rm -f ohmydesk.db && \
  cargo run -p server --manifest-path /data/code/OhMyDesk/Cargo.toml -- set-password 'TestPass@9' --user boss
```
Expected: 打印 `已更新管理员凭据：user=boss（重启 server 生效）`，且 `/tmp/ohmydesk.db` 的 settings 表含 admin_user=boss

- [ ] **Step 5：提交**

```bash
git add src/server/src/main.rs
git commit -m "feat(server): 新增 set-password CLI 子命令(后台改密)"
```

---

## Task 9：前端 transport 新增 fetchLoginLogs

**Files:**
- Modify: `src/admin-web/src/lib/transport/types.ts`（接口 + import）
- Modify: `src/admin-web/src/lib/transport/real.ts`（实现）
- Modify: `src/admin-web/src/lib/transport/mock.ts`（mock 实现）

- [ ] **Step 1：types.ts 加接口方法**

在 `src/admin-web/src/lib/transport/types.ts` 顶部 import 区加：

```ts
import type { LoginLogEntry } from "@/lib/types/LoginLogEntry";
```

在 `Transport` 接口的 `fetchSessions(): Promise<Session[]>;` 下方加：

```ts
  // 获取管理员登录日志（分页）
  fetchLoginLogs(limit?: number, offset?: number): Promise<LoginLogEntry[]>;
```

- [ ] **Step 2：real.ts 实现**

在 `src/admin-web/src/lib/transport/real.ts` 顶部 import 区加：

```ts
import type { LoginLogEntry } from "@/lib/types/LoginLogEntry";
```

在 `fetchSessions`（第 99-110 行）之后加：

```ts
  async fetchLoginLogs(limit = 100, offset = 0): Promise<LoginLogEntry[]> {
    const params = new URLSearchParams();
    params.set("limit", String(limit));
    params.set("offset", String(offset));
    const token = getToken();
    const res = await fetch(apiUrl(`/api/login-logs?${params.toString()}`), {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    });
    if (res.status === 401) {
      onUnauthorized();
      return [];
    }
    if (!res.ok) return [];
    return res.json() as Promise<LoginLogEntry[]>;
  },
```

- [ ] **Step 3：mock.ts 实现**

在 `src/admin-web/src/lib/transport/mock.ts` 顶部 import 区加：

```ts
import type { LoginLogEntry } from "@/lib/types/LoginLogEntry";
```

在 `fetchSessions`（第 269-271 行）之后加：

```ts
  async fetchLoginLogs(limit = 100, _offset = 0): Promise<LoginLogEntry[]> {
    const now = Math.floor(Date.now() / 1000);
    // 注意：ts-rs 把 i64 映射为 bigint，故 id/ts 必须用 bigint 字面量(3n)/BigInt(...)
    const sample: LoginLogEntry[] = [
      { id: 3n, ts: BigInt(now - 60), username: "admin", ip: "10.0.0.21", user_agent: "Mozilla/5.0", success: true, reason: null },
      { id: 2n, ts: BigInt(now - 3600), username: "admin", ip: "10.0.0.21", user_agent: "Mozilla/5.0", success: false, reason: "账号或密码错误" },
      { id: 1n, ts: BigInt(now - 86400), username: "boss", ip: "192.168.1.8", user_agent: "Mozilla/5.0", success: true, reason: null },
    ];
    return sample.slice(0, limit);
  },
```

> **重要（ts-rs 实测）**：`LoginLogEntry` 的 `id`、`ts` 是 **bigint**（ts-rs 把 i64 映射为 bigint），`success` 是 boolean，`ip`/`user_agent`/`reason` 是 `string | null`。前端处理时间用 `Number(r.ts)` 转换、key 用 `String(r.id)`。

- [ ] **Step 4：类型检查**

Run: `cd src/admin-web && npx tsc -b --noEmit`
Expected: 无类型错误（两个 transport 都实现了新接口方法）

- [ ] **Step 5：提交**

```bash
git add src/admin-web/src/lib/transport/types.ts src/admin-web/src/lib/transport/real.ts src/admin-web/src/lib/transport/mock.ts
git commit -m "feat(admin): transport 新增 fetchLoginLogs"
```

---

## Task 10：前端 登录日志 页面

**Files:**
- Create: `src/admin-web/src/pages/LoginLogs.tsx`

- [ ] **Step 1：写页面组件**

创建 `src/admin-web/src/pages/LoginLogs.tsx`：

```tsx
import { useEffect, useState } from "react";
import { CheckCircle2, RefreshCw, ShieldX } from "lucide-react";

import { AppShell } from "@/components/shell/app-shell";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { transport } from "@/lib/transport";
import type { LoginLogEntry } from "@/lib/types/LoginLogEntry";

function fmtTime(tsSec: number): string {
  return new Date(tsSec * 1000).toLocaleString("zh-CN", { hour12: false });
}

export function LoginLogs() {
  const [rows, setRows] = useState<LoginLogEntry[]>([]);
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      setRows(await transport.fetchLoginLogs(200, 0));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <AppShell title="登录日志">
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <p className="text-xs leading-relaxed text-muted-foreground">
            记录每次管理员登录尝试的时间、用户名、来源 IP、客户端与结果（成功 / 失败）。
          </p>
          <Button
            variant="outline"
            size="icon"
            aria-label="刷新"
            disabled={loading}
            onClick={() => void load()}
          >
            <RefreshCw className="size-4" />
          </Button>
        </div>

        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="border-border hover:bg-transparent">
                <TableHead className="w-48">时间</TableHead>
                <TableHead className="w-32">用户名</TableHead>
                <TableHead className="w-40">来源 IP</TableHead>
                <TableHead className="w-24">结果</TableHead>
                <TableHead>客户端 / 备注</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((r) => (
                <TableRow key={String(r.id)} className="border-border">
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {fmtTime(Number(r.ts))}
                  </TableCell>
                  <TableCell className="text-sm">{r.username}</TableCell>
                  <TableCell className="font-mono text-xs">{r.ip ?? "-"}</TableCell>
                  <TableCell>
                    {r.success ? (
                      <span className="inline-flex items-center gap-1 text-online">
                        <CheckCircle2 className="size-4" /> 成功
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 text-warning">
                        <ShieldX className="size-4" /> 失败
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="max-w-md truncate text-xs text-muted-foreground">
                    {r.reason ?? r.user_agent ?? "-"}
                  </TableCell>
                </TableRow>
              ))}
              {rows.length === 0 && (
                <TableRow className="hover:bg-transparent">
                  <TableCell colSpan={5} className="h-32 text-center text-sm text-muted-foreground">
                    暂无登录记录
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>

        <p className="text-xs text-muted-foreground">
          共 <span className="font-mono text-foreground">{rows.length}</span> 条登录记录
        </p>
      </div>
    </AppShell>
  );
}
```

> 说明：本页直接用 `transport`（同 store.ts 的 `import { transport } from "@/lib/transport"`），不进全局 store——登录日志是独立只读视图，无需共享状态。`AppShell` 的 `online/total` 参数可选（见 `Audit.tsx` 传了，但非必填；若 AppShell 要求必填，传 `online={0} total={0}` 或读 store endpoints）。

- [ ] **Step 2：确认 AppShell 入参**

Run: `sed -n '1,40p' src/admin-web/src/components/shell/app-shell.tsx`
Expected: 查看 `title` 是否唯一必填、`online/total` 是否可选。若 `online/total` 必填，则在页面顶部加 `const endpoints = useStore((s) => s.endpoints);` 并传 `online={endpoints.filter(e=>e.online).length} total={endpoints.length}`。

- [ ] **Step 3：类型检查**

Run: `cd src/admin-web && npx tsc -b --noEmit`
Expected: 无类型错误

- [ ] **Step 4：提交**

```bash
git add src/admin-web/src/pages/LoginLogs.tsx
git commit -m "feat(admin): 新增登录日志页面"
```

---

## Task 11：菜单与路由（加登录日志 / 去系统设置）

**Files:**
- Modify: `src/admin-web/src/components/shell/nav-config.ts`
- Modify: `src/admin-web/src/App.tsx`
- Modify: `src/admin-web/src/components/shell/app-sidebar.tsx:99`

- [ ] **Step 1：nav-config 用登录日志替换系统设置**

把 `src/admin-web/src/components/shell/nav-config.ts` 整文件改为：

```ts
import { LayoutList, Monitor, MonitorPlay, ScrollText, Bot, History } from "lucide-react";

export const navItems = [
  { key: "assets", title: "终端资产", href: "/assets", icon: LayoutList },
  { key: "grid", title: "批量监控", href: "/grid", icon: Monitor },
  { key: "remote", title: "远程控制", href: "/remote", icon: MonitorPlay },
  { key: "audit", title: "会话审计", href: "/audit", icon: ScrollText },
  { key: "assistant", title: "AI 助手", href: "/assistant", icon: Bot },
] as const;

// 系统级入口，单独分组渲染在管控功能下方
export const systemNavItems = [
  { key: "login-logs", title: "登录日志", href: "/login-logs", icon: History },
] as const;
```

- [ ] **Step 2：App.tsx 加登录日志路由、删系统设置路由**

在 `src/admin-web/src/App.tsx`：

删除第 8 行 `import { Settings } from "@/pages/Settings";`，改为 import 新页面：

```tsx
import { LoginLogs } from "@/pages/LoginLogs";
```

删除 `/settings` 整段 Route（第 115-122 行）。在 `/audit` Route（第 99-106 行）之后加 `/login-logs` Route：

```tsx
        <Route
          path="/login-logs"
          element={
            <RequireAuth>
              <LoginLogs />
            </RequireAuth>
          }
        />
```

> `Settings.tsx` 文件保留在磁盘但不再被 import；通配 `*` 路由（第 123 行）会把深链 `/settings` 重定向到 `/assets`。

- [ ] **Step 3：sidebar footer 去掉 /settings 链接**

把 `src/admin-web/src/components/shell/app-sidebar.tsx` 第 99 行：

```tsx
            <SidebarMenuButton size="lg" render={<Link to="/settings" />}>
```

改为（移除 render 链接，变纯展示按钮）：

```tsx
            <SidebarMenuButton size="lg">
```

- [ ] **Step 4：类型检查（确认无未用 import）**

Run: `cd src/admin-web && npx tsc -b --noEmit`
Expected: 无类型错误、无 `Settings` / `Link` 未用报错（`Link` 仍用于 navItems 渲染，保留 import）

- [ ] **Step 5：构建验证**

Run: `cd src/admin-web && npm run build`
Expected: 构建成功

- [ ] **Step 6：提交**

```bash
git add src/admin-web/src/components/shell/nav-config.ts src/admin-web/src/App.tsx src/admin-web/src/components/shell/app-sidebar.tsx
git commit -m "feat(admin): 菜单加登录日志、移除系统设置入口与路由"
```

---

## Task 12：后台改密方法文档

**Files:**
- Modify: `.agent/skills/deploy/SKILL.md`（或部署文档对应处，追加改密章节）

- [ ] **Step 1：定位部署文档**

Run: `ls .agent/skills/deploy/ && sed -n '1,20p' .agent/skills/deploy/SKILL.md`
Expected: 确认部署文档存在，找到合适的「运维操作」位置

- [ ] **Step 2：追加改密说明**

在部署文档末尾追加章节：

```markdown
## 修改管理员密码（后台 CLI）

系统设置网页改密入口已下线，改密只能在服务器端用 CLI 子命令（仅持服务器 shell 的管理员可操作）：

Docker 部署：
\`\`\`bash
docker exec <容器名> ohmydesk-server set-password '新密码' [--user 新用户名]
docker restart <容器名>   # 重启生效
\`\`\`

裸机部署：
\`\`\`bash
DATABASE_URL=sqlite:/app/data/ohmydesk.db ./ohmydesk-server set-password '新密码'
# 重启 server 进程生效
\`\`\`

- 不传 `--user` 时仅改密码、保留现有用户名（无持久化则默认 `admin`）。
- 写入 SQLite `settings` 表（bcrypt 哈希）；改密**需重启 server 进程**才会被运行中的实例加载。
- 无可写 DB 时命令会报错退出（改密不会静默失败）。
```

- [ ] **Step 3：提交**

```bash
git add .agent/skills/deploy/SKILL.md
git commit -m "docs(deploy): 补充后台 CLI 改密方法"
```

---

## 验收（全部 Task 完成后）

- [ ] 后端：`cargo test -p server && cargo test -p protocol` 全绿
- [ ] 后端：`cargo build --release -p server` 通过
- [ ] 前端：`cd src/admin-web && npx tsc -b --noEmit && npm run build` 通过
- [ ] 手测：启动 server（有 DB）→ admin-web 登录一次（成功）+ 故意错密码一次（失败）→ 「登录日志」页应出现两条（含 IP、UA、成败）
- [ ] 手测：菜单无「系统设置」；侧栏底部头像不再跳转 /settings；深链 `/settings` 重定向 `/assets`
- [ ] 手测：`ohmydesk-server set-password` 改密后，旧密码登录失败、新密码成功（重启 server 后）
- [ ] 合并前回归：远控/截图/文件等既有 `/ws` 与 `/api/*` 功能不受 ConnectInfo 改造影响
```
