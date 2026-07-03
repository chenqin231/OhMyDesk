# Data Model: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope` | **Date**: 2026-07-03

> 本功能唯一的持久化数据模型变更是给终端引入 **owner 维度**（软外键 → `users.id`）。其余为内存态/客户端态。

## 实体

### User（现有，不变）
| 字段 | 类型 | 说明 |
|---|---|---|
| id | TEXT (UUIDv4) | 主键，JWT `sub` 即此值 |
| username | TEXT UNIQUE | |
| tier | TEXT | `superadmin` \| `user`（列名 `role`） |
| permissions | TEXT | 逗号串菜单权限集（与本功能正交，不动） |

**不变更**。owner 借助其 `id` 作为归属主体。

### Endpoint（`endpoint_registry` 表）— 新增 owner_id
| 字段 | 类型 | 变更 | 说明 |
|---|---|---|---|
| id | TEXT | 现有 | 终端 id（机器指纹 9 位 / ep-*） |
| info | TEXT(JSON) | 现有 | `EndpointInfo` 序列化，**owner 不进此 JSON** |
| last_seen | INTEGER | 现有 | |
| **owner_id** | **TEXT NULL** | **🆕 新增列** | 软外键 → `users.id`；NULL=无归属（旧端/未登录） |

- **迁移**：`add_column_if_missing(pool, "endpoint_registry", "owner_id", "ALTER TABLE endpoint_registry ADD COLUMN owner_id TEXT")`（`db.rs` `ensure_identity_columns`）+ 基础 DDL `scripts/db/schema.sqlite.sql` 同步加列。存量行回填 NULL。
- **校验规则**：owner_id 只能由服务端从连接的 JWT `sub` 写入；Register 报文自报的任何归属字段一律忽略（FR-002-E1）。
- **不加 FK 约束**（SQLite `ADD COLUMN` 不支持后加 FK；软外键足够，用户删除时 owner_id 变悬空 → 表现为「仅 superadmin 可见」，可接受）。

### Registry::Entry（server 内存态）— 新增 owner
```rust
struct Entry { info: EndpointInfo, password: String, last_seen: i64, owner: Option<String> }  // owner 🆕
```
`upsert(info, password, now, owner: Option<String>)` 增 owner 参数。

### EndpointView（`protocol`，server→admin 出站）— 新增 owner_id（可选展示）
| 字段 | 变更 | 说明 |
|---|---|---|
| （现有 id/name/online/last_seen/xinchuang 等） | 不变 | |
| **owner_id** | 🆕 `Option<String>` | 用 `Option` 兼容旧端 serde（同 `department`/`gpu` 先例）。**隔离过滤在服务端完成**，此字段仅供 superadmin 视图展示归属（P2，可延后前端消费）。ts-rs 导出更新 admin-web 类型。 |

### ActorIdentity（server 内存态，现有）— 不变
`{ user_id, username, role, permissions, is_superadmin }`。`user_id` 即 owner 来源，已就绪。**改的是它现在也会为 agent 连接绑定**（D3），字段不变。

### Credential（client 本地态）— 新增
```rust
// src/client/src/credential.rs（新文件）
#[derive(Serialize, Deserialize)]
struct Creds { token: String, user: String }
```
落 `ohmydesk_state_dir()/credential.json`；`load/save/clear`。注销 = `clear()`。

## 关系图

```
users(id) ──1:0..N── endpoint_registry(owner_id)      [软外键，可空]
endpoint_registry(id) ──1:N── sessions(to_id)          [远控目标]
sessions(id) ──1:N── audit_logs(session_id)            [审计归属经会话目标终端]
login_log(username) ──N:1── users(username)            [登录日志按账号，不涉终端]
```

## owner 状态转换（终端归属生命周期）

```
[无归属 owner=NULL]
   │  带有效 token 的客户端登录并 Register（owner=JWT.sub）
   ▼
[归属 A owner=A] ──心跳/重注册（同连接）──> owner 保持 A
   │  注销（连接断开）
   ▼
[离线，owner 保留 A]        ← DB 落库保留，重启回灌仍为 A（历史归属可追溯）
   │  账号 B 登录并 Register（owner=B 覆盖）
   ▼
[归属 B owner=B]

[旧端/无 token] ──Register 无身份──> owner=NULL（仅 superadmin 可见，永不越权）
```

**可见性/可控性判据**（普通账号 A vs superadmin）：
| owner_id | 普通账号 A 可见 | 普通账号 A 可远控 | superadmin |
|---|---|---|---|
| = A | ✅ | ✅ | ✅ |
| = B | ❌ | ❌（闸拦截） | ✅ |
| = NULL | ❌ | ❌ | ✅ |
