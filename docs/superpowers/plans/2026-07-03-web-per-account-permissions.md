# WEB 按账户动态菜单授权 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把管理平台权限从「固定 4 角色写死映射」改为「每账户自定义功能菜单集」，superadmin 唯一 god 独占账户管理，人人可自助改密。

**Architecture:** `users` 表由 role→permissions 列（每账户存菜单键集）驱动；tier 二值 `superadmin`/`user`。superadmin 隐式全权。WS 远控闸/审计归属/前端菜单过滤链路**复用现网 RBAC，仅换权限来源**。演进自 master `7c285b3`。

**Tech Stack:** Rust/Axum/SQLx/SQLite/bcrypt/jsonwebtoken/ts-rs，React/Vite/Zustand/TypeScript，Docker(Track A)。

## 🔴 全程铁律
**只加管理平台功能，绝不影响 WEB 远控 / 客户端远控。** `src/client` 零改动；远控传输/注入/会话不改；WS 闸仅换权限来源，superadmin 与被授 `use_remote` 账户远控行为与现网一致。每任务须能论证不触远控路径。

## 权限键（Permission keys）
可配给普通账户：`view_assets` / `manage_assets`(终端资产子项，依赖 view_assets) / `view_grid` / `use_remote` / `view_audit` / `view_login_logs`。
不可配：`manage_users`(superadmin 独占)、自助改密(人人默认)。superadmin 隐式全部含 manage_users。

## File Structure
- `scripts/db/schema.sqlite.sql` — users 表 schema 改（role CHECK→`superadmin`/`user`，加 permissions 列）。
- `src/server/src/db.rs` — 加 `migrate_users_to_per_account_permissions()` 幂等迁移。
- `src/server/src/users.rs` — Role→tier；UserRecord 加 `permissions`；权限解析/存取；CRUD 加 set_permissions/set_username。
- `src/server/src/auth.rs` — Claims/AuthUser 携带 permissions 或 tier；validate 组装用户权限集。
- `src/server/src/http.rs` — `/api/me` 下发用户集；update_user 接受 permissions/username；`POST /api/me/password` 自助改密；用户管理 gate 改 tier==superadmin。
- `src/server/src/hub.rs`/`handlers.rs` — WS 远控闸 use_remote 判定改读用户集；operator_role 写 tier。
- `src/admin-web/src/lib/permissions.ts` — 去 role→perm 硬映射；tier + 菜单元数据。
- `src/admin-web/src/store/auth.ts` — CRUD 加 permissions/username；自助改密 action。
- `src/admin-web/src/pages/Users.tsx` — 菜单勾选器 + 改名。
- `src/admin-web/src/pages/Settings.tsx` — 「个人设置」人人可见自助改密。
- `src/admin-web/src/App.tsx`/`nav-config.ts` — 个人设置路由人人可达；系统设置菜单去 manage_settings 门控。

---

### Task 1: DB 迁移——users 表 role→tier + permissions 列（幂等）

**Files:**
- Modify: `scripts/db/schema.sqlite.sql`
- Modify: `src/server/src/db.rs`（新建表版 schema 常量 + 迁移函数 + 调用点）
- Test: `src/server/src/db.rs`（`#[cfg(test)] mod tests`，仿现有 `ensure_identity_columns_migrates_old_schema_idempotently`）

- [ ] **Step 1: 写迁移失败测试**

在 db.rs tests 加（用内存/临时 sqlite，先造旧 schema 塞 superadmin+admin+operator+auditor 各一行，跑迁移，断言：permissions 列存在、role 被映射成 superadmin/user、各行 permissions backfill 正确、再跑一次不报错不改数据）：

```rust
#[tokio::test]
async fn migrate_users_maps_roles_to_tier_and_backfills_permissions_idempotently() {
    let pool = new_temp_pool().await; // 复用测试建池助手
    // 造旧表(带旧 CHECK)+4 行
    sqlx::raw_sql(OLD_USERS_DDL).execute(&pool).await.unwrap();
    for (u, r) in [("superadmin","superadmin"),("admin","admin"),("op","operator"),("aud","auditor")] {
        sqlx::query("INSERT INTO users(id,username,password_hash,role,enabled,created_at,updated_at) VALUES(?,?,?,?,1,0,0)")
            .bind(u).bind(u).bind("h").bind(r).execute(&pool).await.unwrap();
    }
    migrate_users_to_per_account_permissions(&pool).await.unwrap();
    // superadmin: tier=superadmin, permissions 空(隐式)
    let (role, perms): (String,String) = sqlx::query_as("SELECT role,permissions FROM users WHERE username='superadmin'").fetch_one(&pool).await.unwrap();
    assert_eq!(role, "superadmin"); assert_eq!(perms, "");
    // admin → user + 全功能(含 manage_assets, 不含 manage_users)
    let (role, perms): (String,String) = sqlx::query_as("SELECT role,permissions FROM users WHERE username='admin'").fetch_one(&pool).await.unwrap();
    assert_eq!(role, "user");
    assert!(perms.contains("view_assets") && perms.contains("manage_assets") && perms.contains("use_remote") && perms.contains("view_login_logs"));
    assert!(!perms.contains("manage_users"));
    // operator → user + view_assets,view_grid,use_remote
    let (_r, perms): (String,String) = sqlx::query_as("SELECT role,permissions FROM users WHERE username='op'").fetch_one(&pool).await.unwrap();
    assert!(perms.contains("view_assets") && perms.contains("view_grid") && perms.contains("use_remote"));
    assert!(!perms.contains("view_audit"));
    // auditor → user + view_audit,view_login_logs
    let (_r, perms): (String,String) = sqlx::query_as("SELECT role,permissions FROM users WHERE username='aud'").fetch_one(&pool).await.unwrap();
    assert!(perms.contains("view_audit") && perms.contains("view_login_logs") && !perms.contains("use_remote"));
    // 幂等:再跑不炸、行数不变
    migrate_users_to_per_account_permissions(&pool).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 4);
}
```
（`OLD_USERS_DDL` = 现网旧 users 建表语句，含 `role ... CHECK(role IN ('superadmin','admin','operator','auditor'))`，无 permissions 列。`new_temp_pool` 若无现成助手则新建一个用 `sqlx::SqlitePool::connect("sqlite::memory:")`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server migrate_users_maps_roles_to_tier -- --nocapture`
Expected: 编译失败/未定义 `migrate_users_to_per_account_permissions`。

- [ ] **Step 3: 实现迁移函数**

在 db.rs 加（幂等靠 PRAGMA 查 permissions 列是否存在；SQLite 无法改 CHECK 故建新表迁移；用事务）：

```rust
async fn users_has_column(pool: &Db, col: &str) -> sqlx::Result<bool> {
    let rows = sqlx::query("PRAGMA table_info(users)").fetch_all(pool).await?;
    Ok(rows.iter().any(|r| { use sqlx::Row; r.try_get::<String,_>("name").map(|n| n==col).unwrap_or(false) }))
}

fn perms_for_legacy_role(role: &str) -> &'static str {
    match role {
        "admin" => "view_assets,manage_assets,view_grid,use_remote,view_audit,view_login_logs",
        "operator" => "view_assets,view_grid,use_remote",
        "auditor" => "view_audit,view_login_logs",
        _ => "",
    }
}

pub(crate) async fn migrate_users_to_per_account_permissions(pool: &Db) -> sqlx::Result<()> {
    // 表不存在(全新库,schema 已是新版)或已迁移 → 跳过
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'").fetch_one(pool).await?;
    if exists == 0 { return Ok(()); }
    if users_has_column(pool, "permissions").await? { return Ok(()); }
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(
        "CREATE TABLE users_new (\
           id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL,\
           role TEXT NOT NULL CHECK(role IN ('superadmin','user')),\
           permissions TEXT NOT NULL DEFAULT '',\
           enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)"
    ).execute(&mut *tx).await?;
    let rows = sqlx::query("SELECT id,username,password_hash,role,enabled,created_at,updated_at FROM users").fetch_all(&mut *tx).await?;
    for r in rows {
        use sqlx::Row;
        let role: String = r.get("role");
        let (tier, perms) = if role == "superadmin" { ("superadmin","".to_string()) } else { ("user", perms_for_legacy_role(&role).to_string()) };
        sqlx::query("INSERT INTO users_new(id,username,password_hash,role,permissions,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(r.get::<String,_>("id")).bind(r.get::<String,_>("username")).bind(r.get::<String,_>("password_hash"))
            .bind(tier).bind(perms).bind(r.get::<i64,_>("enabled")).bind(r.get::<i64,_>("created_at")).bind(r.get::<i64,_>("updated_at"))
            .execute(&mut *tx).await?;
    }
    sqlx::raw_sql("DROP TABLE users; ALTER TABLE users_new RENAME TO users;").execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!("users 表已迁移为按账户权限模型(role→tier + permissions)");
    Ok(())
}
```
在 `connect()` 里、`ensure_identity_columns` 之后调用 `migrate_users_to_per_account_permissions(&pool).await`（失败则 warn 并按现有降级策略处理，勿 panic）。

- [ ] **Step 4: 更新 schema.sqlite.sql（新库直接新版）**

把 users 建表改为 `role TEXT NOT NULL CHECK(role IN ('superadmin','user'))` 且加 `permissions TEXT NOT NULL DEFAULT ''`。users.rs 内若有重复 DDL 常量（第 430 行附近）同步改。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p server migrate_users_maps_roles_to_tier`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add scripts/db/schema.sqlite.sql src/server/src/db.rs src/server/src/users.rs
git commit -m "feat(rbac): users 表迁移为按账户权限模型(role→tier + permissions 列)"
```

---

### Task 2: 后端权限存取——UserRecord.permissions + tier + 存储集为权限源

**Files:**
- Modify: `src/server/src/users.rs`
- Test: `src/server/src/users.rs` tests

- [ ] **Step 1: 写权限存取测试**

```rust
#[test]
fn permission_parse_roundtrip_and_superadmin_is_implicit_all() {
    // 字符串集解析
    let set = PermissionSet::parse("view_assets,use_remote,manage_assets");
    assert!(set.contains(Permission::UseRemote) && set.contains(Permission::ManageAssets));
    assert!(!set.contains(Permission::ViewAudit));
    assert_eq!(set.to_storage(), "view_assets,manage_assets,use_remote"); // 规范化顺序稳定
    // superadmin 隐式全集(含 manage_users)
    let sa = PermissionSet::superadmin_all();
    for p in Permission::ALL { assert!(sa.contains(*p)); }
    assert!(sa.contains(Permission::ManageUsers));
}

#[tokio::test]
async fn set_permissions_persists_and_manage_assets_requires_view_assets() {
    let store = temp_user_store().await;
    let u = store.create_user_v2("op1", "pw", &["view_grid"]).await.unwrap(); // 普通账户
    // 合法覆盖
    store.set_permissions(&u.id, &["view_assets","manage_assets"]).await.unwrap();
    let got = store.get_by_id(&u.id).await.unwrap();
    assert!(got.permissions.contains(Permission::ManageAssets));
    // manage_assets 缺 view_assets → 拒
    let err = store.set_permissions(&u.id, &["manage_assets"]).await.unwrap_err();
    assert!(err.contains("view_assets"));
    // 非法键 → 拒
    assert!(store.set_permissions(&u.id, &["manage_users"]).await.is_err()); // manage_users 不可配给普通账户
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server users::tests::permission_parse_roundtrip users::tests::set_permissions_persists`
Expected: 编译失败（`PermissionSet`/`set_permissions`/`create_user_v2` 未定义）。

- [ ] **Step 3: 实现 PermissionSet + tier + 存取**

- `Permission` 加 `pub const ALL: &[Permission]`（全 7 项含 ManageUsers）与 `ASSIGNABLE: &[Permission]`（6 项可配，不含 ManageUsers）。加 `fn from_str`。
- 新 `PermissionSet`（内部 `Vec<Permission>` 或 bitflags）：`parse(&str)`、`to_storage()->String`（按 ALL 顺序规范化、逗号连接）、`contains`、`superadmin_all()`。
- `UserRecord` 加 `pub permissions: PermissionSet`；`tier`（由 role 派生：`role=="superadmin"`）。序列化：`permissions` 输出为 `Vec<&str>`（前端消费）；`role` 输出 tier 字符串。
- `UserStore`：`create_user_v2(username, pw, perms: &[&str])`（tier=user，存 permissions）；`set_permissions(id, perms)`（校验：仅 ASSIGNABLE 键、`manage_assets`⇒需含 `view_assets`、superadmin 目标拒改）；从行读 permissions 列构造。
- 旧 `Role::permissions()` 硬映射：保留仅供 Task1 迁移用，运行期权限一律走 `UserRecord.permissions`（superadmin 用 `superadmin_all()`）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p server users::tests`
Expected: PASS（含既有用例，必要时同步旧用例到新 API）。

- [ ] **Step 5: 提交**

```bash
git add src/server/src/users.rs
git commit -m "feat(rbac): 用户权限改按账户存储集(PermissionSet)+superadmin 隐式全集"
```

---

### Task 3: 后端 API——/api/me 下发用户集 + 配菜单/改名 + 自助改密

**Files:**
- Modify: `src/server/src/auth.rs`（validate 组装权限集到 AuthUser）
- Modify: `src/server/src/http.rs`
- Test: `src/server/src/http.rs` tests

- [ ] **Step 1: 写 API 测试**

```rust
#[tokio::test]
async fn me_returns_stored_permissions_and_superadmin_gets_all() {
    // 造 superadmin + 普通账户(view_audit)，分别登录取 token，GET /api/me
    // superadmin: permissions 含全部；普通: 仅 ["view_audit"]
}
#[tokio::test]
async fn patch_user_permissions_superadmin_only_and_validated() {
    // 普通账户 token PATCH /api/users/:id {permissions:[...]} → 403(非 superadmin)
    // superadmin PATCH 合法集 → 200，回读生效
    // superadmin PATCH ["manage_assets"](缺 view_assets) → 400
    // superadmin PATCH 改 superadmin 目标 → 400/403
}
#[tokio::test]
async fn patch_username_unique_and_superadmin_target_locked() {
    // superadmin 改普通账户名 → 200；撞名 → 400；改 superadmin 自己名 → 拒
}
#[tokio::test]
async fn self_password_change_verifies_old_and_updates() {
    // 任意账户 POST /api/me/password {old,new}: 旧密对→200 且新密可登录；旧密错→400
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server http::tests::me_returns_stored_permissions http::tests::patch_user_permissions http::tests::patch_username http::tests::self_password_change`
Expected: FAIL/未实现。

- [ ] **Step 3: 实现**

- `AuthUser` 加 `permissions: PermissionSet`（validate 时：superadmin→`superadmin_all()`，否则用户存储集）。`can()` 改查该集。`require()` 不变（读 AuthUser.permissions）。
- `me`：返回 `{ user, tier, permissions: [...] }`（role 字段可保留为 tier）。`/api/login` 同样下发 permissions。
- `UpdateUserReq` 加 `permissions: Option<Vec<String>>` 与 `username: Option<String>`。update_user：仍 `require(ManageUsers)`（superadmin 隐式有，普通账户无=403，天然 superadmin-only）；分别调用 `set_permissions`/`set_username`/`set_enabled`；superadmin 目标各项拒改（改名/降权/停用）。
- 新路由 `POST /api/me/password`（**仅需登录，不 require 任何 menu 权限**）：`{old,new}`，`verify_login(user, old)` 成功则 `reset_password_self`。加 `POST /api/users` 的 create 改用 `create_user_v2` + 接受 `permissions`。
- **远控无关性论证写进 commit body**：本任务仅改管理 API 与 /me，未触 hub/handlers 远控路径。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p server http::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/server/src/auth.rs src/server/src/http.rs
git commit -m "feat(rbac): /api/me 下发用户权限集 + 配菜单/改名(superadmin) + 自助改密"
```

---

### Task 4: 后端 WS 远控闸 + 审计 tier（不回归远控）

**Files:**
- Modify: `src/server/src/hub.rs`、`src/server/src/handlers.rs`
- Test: `src/server/src/hub.rs` tests（复用现有三态闸测试，改造权限来源）

- [ ] **Step 1: 改造/新增闸测试**

```rust
#[tokio::test]
async fn connect_request_allowed_when_actor_permissions_include_use_remote() {
    // 绑定一个 permissions 含 use_remote 的 admin actor → ConnectRequest 放行 + operator_* 盖章
}
#[tokio::test]
async fn connect_request_denied_when_actor_permissions_lack_use_remote() {
    // 绑定 permissions=[view_audit] 的 actor → 拒(不建会话)
}
```
（保留原「未绑定拒」用例。superadmin actor 视为含 use_remote。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p server hub::tests::connect_request_allowed_when_actor_permissions hub::tests::connect_request_denied_when_actor_permissions`
Expected: FAIL（闸仍按旧 role 映射判定）。

- [ ] **Step 3: 改闸判定 + ActorIdentity 携带权限**

- `ActorIdentity` 增加权限来源：绑定时（main.rs 由 AuthUser）带上 `permissions: PermissionSet`（或 tier+perms）。远控闸判定从 `role.parse::<Role>().permissions().contains(UseRemote)` 改为 `actor.permissions.contains(UseRemote) || actor.is_superadmin`。
- `operator_role`/审计 tier：写 `superadmin`/`user`。
- **务必**：仅改「是否放行」的判定输入，转发/会话/审计写入结构不动。main.rs bind_actor 处同步传 permissions。

- [ ] **Step 4: 跑测试确认通过 + 远控回归自检**

Run: `cargo test -p server hub::tests session::tests audit::tests`
Expected: PASS。人工核对：superadmin 与含 use_remote 的普通账户仍放行；auditor 类拒绝。

- [ ] **Step 5: 提交**

```bash
git add src/server/src/hub.rs src/server/src/handlers.rs src/server/src/main.rs
git commit -m "feat(rbac): WS 远控闸改按账户权限集判定 use_remote(远控行为不变)"
```

---

### Task 5: 前端 permissions.ts + auth store（tier + CRUD + 自助改密）

**Files:**
- Modify: `src/admin-web/src/lib/permissions.ts`、`src/admin-web/src/store/auth.ts`
- Test: 复用现有 `pnpm test`（permissions 纯逻辑若有测试则补）

- [ ] **Step 1: 改 permissions.ts**

- 去掉任何 role→permission 硬映射（若存在）。`Permission` 联合保留 6 可配键 + `manage_users`。新增 `ASSIGNABLE_MENUS`（元数据：key/label/父子，`manage_assets` 标注 parent=`view_assets`）。`tierLabel(tier)`：superadmin→超级管理员、user→普通用户。`hasPermission`/`defaultPathForPermissions` 不变。

- [ ] **Step 2: 改 auth store**

- state 加 `tier`；login/loadMe 解析 `{tier, permissions}`。
- `AdminUser` 加 `permissions: Permission[]`、`tier`。
- CRUD：`updateUser(id,{permissions?,username?,enabled?})`；`createUser({username,password,permissions})`；新增 `changeOwnPassword(old,new)` → `POST /api/me/password`。

- [ ] **Step 3: build + test**

Run: `cd src/admin-web && pnpm test && pnpm build`
Expected: 绿。

- [ ] **Step 4: 提交**

```bash
git add src/admin-web/src/lib/permissions.ts src/admin-web/src/store/auth.ts
git commit -m "feat(rbac-web): auth store 支持 tier/按账户 permissions/自助改密"
```

---

### Task 6: 前端 Users 页——菜单勾选器 + 改名

**Files:**
- Modify: `src/admin-web/src/pages/Users.tsx`
- Test: `pnpm build`（类型门）+ `pnpm test`

- [ ] **Step 1: 菜单勾选器**

- 新增/编辑账户：用 `ASSIGNABLE_MENUS` 渲染复选框组；`manage_assets` 缩进为 `view_assets` 子项，`view_assets` 未勾时禁用并自动取消。
- 账户列表行：展示已授菜单（chips/勾选态）；「编辑权限」保存调 `updateUser(id,{permissions})`。
- 改名：行内可编辑 username，保存调 `updateUser(id,{username})`。
- superadmin 行：权限勾选器、改名、停用、重置密码 全部禁用/锁定文案（tier==superadmin）。
- 复用现有 busy/error/Dialog 惯例；本次改动若使 Users.tsx 过大，按职责抽 `<MenuPermissionEditor>` 子组件（回应既有 backlog）。

- [ ] **Step 2: build + test**

Run: `cd src/admin-web && pnpm test && pnpm build`
Expected: 绿。

- [ ] **Step 3: 提交**

```bash
git add src/admin-web/src/pages/Users.tsx
git commit -m "feat(rbac-web): 用户管理页加菜单勾选器 + 改用户名(superadmin 独占)"
```

---

### Task 7: 前端 个人设置——人人可自助改密

**Files:**
- Modify: `src/admin-web/src/pages/Settings.tsx`、`src/admin-web/src/App.tsx`、`src/admin-web/src/components/shell/nav-config.ts`
- Test: `pnpm build` + `pnpm test`

- [ ] **Step 1: 改 Settings 为个人设置(自助改密)**

- Settings 改为「个人设置」：调 `changeOwnPassword(old,new)`（旧密+新密+确认）。**移除 manage_settings 门控**——路由与菜单人人可达。
- nav-config：「个人设置」项无 permission 约束（人人显示）；去掉旧「系统设置」的 manage_settings 依赖。
- App.tsx：`/settings` 路由的 `RequireAuth` 去掉 permission 参数（仅需登录）。

- [ ] **Step 2: build + test**

Run: `cd src/admin-web && pnpm test && pnpm build`
Expected: 绿。

- [ ] **Step 3: 提交**

```bash
git add src/admin-web/src/pages/Settings.tsx src/admin-web/src/App.tsx src/admin-web/src/components/shell/nav-config.ts
git commit -m "feat(rbac-web): 个人设置人人可见自助改密(去 manage_settings 门控)"
```

---

### Task 8: 全量验证（不含部署，部署由主控 Track A 执行）

**Files:** 无（验证 + 可能的回归修复）

- [ ] **Step 1: 后端全量**

Run: `cargo test --workspace`
Expected: 全绿（server 用例数增加，其它 crate 不变）。

- [ ] **Step 2: 客户端交叉编译门（护远控协议兼容）**

Run: `cargo check -p client --target x86_64-pc-windows-gnu`
Expected: Finished 无 error（若 target 未装，记录跳过，不算失败）。

- [ ] **Step 3: 前端全量**

Run: `cd src/admin-web && pnpm test && pnpm build`
Expected: 绿。

- [ ] **Step 4: 本地 smoke（含远控无回归自检）**

Run: `cargo run -p server`，curl `/api/login`(superadmin) 看 permissions 全集；建一个仅 `view_audit` 账户登录看 permissions 只含 view_audit；确认无 client 改动（`git diff --stat master...HEAD -- src/client` 为空）。kill server。
Expected: 权限集正确；`src/client` 零改动。

- [ ] **Step 5: 提交（若有回归修复）**

```bash
git add -A && git commit -m "test(rbac): 全量验证与回归修复"
```

---

## Self-Review 覆盖对照（spec→task）
- 按账户 permissions 存储：Task1(列/迁移)+Task2(存取)✓
- superadmin 隐式全集/独占账户管理：Task2(superadmin_all)+Task3(update gate)✓
- 配菜单/manage_assets 依赖：Task2(校验)+Task3(API)+Task6(UI)✓
- 改用户名/superadmin 拒改名：Task3(API)+Task6(UI)✓
- 自助改密人人可见：Task3(/api/me/password)+Task7(UI)✓
- 迁移 admin→普通全功能账户：Task1(perms_for_legacy_role)✓
- 远控不回归：Task4(仅换判定来源)+Task8(交叉编译+smoke+client 零改动断言)✓
- 前端菜单过滤复用：Task5/6/7 消费 /api/me，未改过滤逻辑 ✓
