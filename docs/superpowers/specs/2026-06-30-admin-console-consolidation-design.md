# Spec A：管理后台收敛（登录日志 + 隐藏设置/后台改密）

> 日期：2026-06-30
> 范围：功能② 登录日志菜单 + 功能③ 隐藏系统设置菜单 + 后台改密方法
> 状态：已确认设计，待转实现计划

---

## 1. 背景与目标

OhMyDesk 管理后台（`src/admin-web`，React 19 + Zustand + react-router 7 + Base UI）当前：

- **无登录日志**：`POST /api/login`（`src/server/src/http.rs:85`）签发 JWT，但不记录任何登录痕迹（IP/时间/成败）。
- **系统设置页面暴露改密入口**：`pages/Settings.tsx` 允许在网页上改管理员账号密码，菜单项在 `nav-config.ts` 的 `systemNavItems`。

本 spec 目标：

1. 新增「登录日志」菜单，记录管理员每次登录的用户名、IP、UA、时间、成败，并在后台分页查看。
2. 隐藏「系统设置」菜单，移除网页改密入口；改密改由**服务器端 CLI 子命令**完成，确保只有持服务器 shell 的管理员能改密。

---

## 2. 关键决策（已确认）

| # | 决策 | 理由 |
|---|------|------|
| 1 | 登录成功**与失败都记录** | 失败记录可观测暴力破解尝试，安全价值更高 |
| 2 | IP 提取**优先 `X-Forwarded-For` / `X-Real-IP`，回退 axum `ConnectInfo` 直连 IP** | 部署经宝塔 nginx 反代，直连 IP 会是反代地址 |
| 3 | 后台改密用 **server CLI 子命令**（非保留隐藏网页接口） | 「仅持服务器 shell 的你能改」，比隐藏网页更可控 |
| 4 | `/settings` 路由**移除**，`Settings.tsx` 文件保留但不挂载 | 彻底去除网页改密入口；保留文件便于将来恢复 |

---

## 3. 数据层

新增表，追加到 `scripts/db/schema.sqlite.sql`（幂等 `CREATE TABLE IF NOT EXISTS`，重启自动建表，无独立 migration 框架）。MySQL 等价版同步到 `scripts/db/schema.sql`。

```sql
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

---

## 4. 服务端（src/server）

### 4.1 LoginLogStore（新建 `src/server/src/login_log.rs`）

仿 `src/server/src/audit.rs::AuditStore` 模式：

- 持 `Option<Db>`，无库时所有方法 no-op + `tracing::warn`（实时链路不依赖 DB，与现有 store 一致）。
- `record(username: &str, ip: Option<&str>, ua: Option<&str>, success: bool, reason: Option<&str>)`：INSERT 一行（仿 `audit.rs:36-59`）。
- `query(limit: i64, offset: i64) -> Vec<LoginLogRow>`：按 `ts DESC` 分页（仿 `audit.rs:106-148`）。
- `LoginLogRow`：sqlx `FromRow`（仿 `audit.rs:204-236`），字段对齐表结构。

在 `main.rs` 装配处构造 `LoginLogStore`（与 `AuditStore` 同位置，共用同一个 `Option<Db>`），注入 `HttpState`（`http.rs:27-32` 追加字段 `login_log: Arc<LoginLogStore>`）。

### 4.2 登录埋点（改 `src/server/src/http.rs::login` 85-92）

- 提取 IP：新增辅助函数 `client_ip(headers, connect_info)`：优先 `X-Forwarded-For`（取第一个）→ `X-Real-IP` → `ConnectInfo<SocketAddr>`。
- 提取 UA：读 `User-Agent` 头。
- 成功分支（issue_token 后）：`login_log.record(user, ip, ua, true, None)`。
- 失败分支（unauth 前）：`login_log.record(user, ip, ua, false, Some("账号或密码错误"))`。
- **前置依赖**：axum 需 `ConnectInfo` 才能拿直连 IP——确认 `main.rs` 的 serve 是否用 `into_make_service_with_connect_info::<SocketAddr>()`，未用则改造（这是实现计划的一个子任务，需回归确认不影响 `/ws` 升级）。

### 4.3 查询接口（改 `src/server/src/http.rs::router` 63-74）

新增 `GET /api/login-logs`（需登录），仿 `query_audit`（`http.rs:162`）：

- 签名：`async fn query_login_logs(_user: AuthUser, State(s), Query(q: LoginLogQuery)) -> Json<...>`
- `LoginLogQuery { limit: Option<i64>, offset: Option<i64> }`，默认 limit=50、offset=0，limit 上限钳制（如 ≤200）。
- 返回 `Json<Vec<LoginLogRow>>`（或带 total 的分页包，与现有 audit 返回风格保持一致——以 audit 实际返回结构为准）。

---

## 5. 前端（src/admin-web）

### 5.1 登录日志菜单与页面

- `src/admin-web/src/components/shell/nav-config.ts`：`systemNavItems` 改为仅含「登录日志」项 `{ key: "login-logs", title: "登录日志", href: "/login-logs", icon: ScrollText }`（lucide 图标，最终图标实现时定）。
- `src/admin-web/src/App.tsx`：新增 `<Route path="/login-logs" element={<RequireAuth><LoginLogs/></RequireAuth>} />`，import 新页面。
- 新建 `src/admin-web/src/pages/LoginLogs.tsx`：用 `<AppShell title="登录日志">` 包裹表格，列＝时间 / 用户名 / IP / UA / 结果（成功绿、失败红），分页。仿 `pages/Audit.tsx`。
- 数据获取：`src/admin-web/src/lib/transport/types.ts` 加 `fetchLoginLogs` 签名 + `LoginLogRow` 类型；`real.ts` 实现（仿 `fetchAudit` 82-97：`apiUrl` + Bearer + 401→onUnauthorized）；`mock.ts` 补 mock 数据。

### 5.2 隐藏系统设置

- `src/admin-web/src/components/shell/nav-config.ts`：移除「系统设置」项（原 `systemNavItems` 第 12-14 行那项）。
- `src/admin-web/src/components/shell/app-sidebar.tsx`：footer 管理员头像（第 99 行 `<Link to="/settings">`）去掉链接，改为纯展示（或指向无害目标）。
- `src/admin-web/src/App.tsx`：移除 `/settings` 路由（第 115-122 行）。`Settings.tsx` 文件保留、不再被 import。深链 `/settings` 落到默认/404。

---

## 6. 后台改密 CLI（src/server）

给 server 二进制加子命令，复用现有 `auth.rs::hash_password`（bcrypt）+ `settings.rs::save_credential`：

```
ohmydesk-server set-password <新密码> [--user <用户名>]
```

- 解析 `args()`：若首参为 `set-password`，进入 CLI 分支（在 `main.rs` 启动早期、起 tokio runtime 前拦截），不启动 HTTP 服务。
- 逻辑：打开同一 SQLite（复用 `db::connect`/`settings`）→ `hash_password(新密码)` → `save_credential(user_or_current, hash)` → 打印成功并 `exit(0)`；无库则报错退出非 0。
- `--user` 省略时仅改密码、用户名沿用 `settings` 现值（无值则默认 `admin`）。
- 提供使用文档：Docker 下 `docker exec <容器> ohmydesk-server set-password '新密码'`；裸机下直接运行二进制。写入部署 skill 或 README 对应处。

---

## 7. 数据流

**登录**：浏览器 `POST /api/login` →（经 nginx 反代，带 `X-Forwarded-For`）→ axum `login` handler → `verify_login` → 提取 IP/UA → `login_log.record(成败)` → 成功签发 JWT / 失败 401。

**查看**：登录日志页 `fetchLoginLogs()` → `GET /api/login-logs`（Bearer）→ `AuthUser` 校验 → `login_log.query(limit, offset)` → SQLite `SELECT ... ORDER BY ts DESC` → JSON → 表格渲染。

**改密**：管理员服务器 shell → `ohmydesk-server set-password` → bcrypt 哈希 → 写 `settings` 表 → 下次登录用新凭据（运行中实例需重启或在内存 `Auth` 同步——CLI 改的是持久层，运行中实例下次启动生效；如需即时生效在文档注明重启服务）。

---

## 8. 错误处理与边界

- 无 DB（`Option<Db>` 为 None）：登录日志 record/query no-op + warn，登录功能本身不受影响（与 audit 一致）。
- IP 缺失：`X-Forwarded-For`、`X-Real-IP`、`ConnectInfo` 全无时存 NULL，不阻断登录。
- limit 越界：服务端钳制上限，防止全表拉取。
- CLI 改密运行中实例不即时生效：文档注明改密后重启 server 进程（Docker `restart`）。
- 失败登录的 `username` 可能是攻击者输入的任意串：仅做长度截断后入库，不回显到任何 HTML（表格用 React 文本节点天然转义）。

---

## 9. 测试清单

- 服务端单测：`LoginLogStore.record` + `query` 分页（含 success/failure 两类行）；`client_ip` 解析（XFF 多值取首、回退顺序）。
- 服务端集成：`POST /api/login` 成功与失败各产生一条 login_log；`GET /api/login-logs` 需鉴权（无 token 401）、返回按 ts 倒序。
- CLI：`set-password` 改密后旧密码失败、新密码成功（重启实例后）。
- 前端：登录日志页渲染、分页、401 跳登录；菜单不再出现「系统设置」；深链 `/settings` 不再命中改密页。
- 回归：`ConnectInfo` 改造不破坏 `/ws` 升级与现有 `/api/*` 接口。

---

## 10. 涉及文件清单

**新建**：`src/server/src/login_log.rs`、`src/admin-web/src/pages/LoginLogs.tsx`

**修改**：
- `scripts/db/schema.sqlite.sql`、`scripts/db/schema.sql`
- `src/server/src/http.rs`（HttpState 字段、login 埋点、新接口、router）
- `src/server/src/main.rs`（声明 `mod login_log;`、装配 LoginLogStore、CLI 子命令拦截、ConnectInfo serve 改造）
- `src/admin-web/src/components/shell/nav-config.ts`
- `src/admin-web/src/components/shell/app-sidebar.tsx`
- `src/admin-web/src/App.tsx`
- `src/admin-web/src/lib/transport/{types.ts,real.ts,mock.ts}`
- 部署文档（改密用法）

**保留不挂载**：`src/admin-web/src/pages/Settings.tsx`
