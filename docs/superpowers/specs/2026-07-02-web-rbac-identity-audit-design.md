# WEB 人员身份、RBAC 与审计归因设计

## 结论

一期实现固定 4 类角色的 RBAC：`superadmin`、`admin`、`operator`、`auditor`。用户账号由 WEB 后台动态管理，远程会话和审计日志必须记录真实登录人员身份，解决客服团队多人使用远程工具时无法追溯操作者的问题。

本设计只做固定角色，不做动态角色/菜单权限配置。这样能在当前系统结构上尽快补齐“人员身份”链路，同时避免引入过重的权限平台。

## 目标

1. 支持 WEB 后台动态新增用户、停用/启用用户、重置密码、修改角色。
2. 支持固定 4 类角色登录管理端，并按角色控制菜单、路由和后端 API。
3. 将登录人员身份贯穿 HTTP、WS、远程会话、操作审计与登录日志。
4. 保留现有线上 `admin` 凭据，升级后不破坏已有管理员登录。
5. 初始化 `superadmin / infogo123`，作为全权限账号和首个用户管理入口。

## 非目标

1. 不实现自定义角色。
2. 不实现细粒度到按钮级的后台权限配置。
3. 不实现组织架构、部门、工单系统或客服排班。
4. 不修改客户端安装包和被控端协议中与身份无关的逻辑。

## 角色与权限

权限采用服务端固定枚举，前端只消费 `/api/me` 返回的权限结果。

| 角色 | 定位 | 菜单权限 | 操作权限 |
| --- | --- | --- | --- |
| `superadmin` | 超级管理员 | 全部菜单 | 全部权限；不可被停用、不可被降权 |
| `admin` | 管理员 | 终端资产、批量监控、远程控制、会话审计、登录日志、用户管理、系统设置 | 可管理非 `superadmin` 用户；可查看审计；可远程 |
| `operator` | 操作员/客服 | 终端资产、批量监控、远程控制 | 可正常使用 WEB 远程与客户端远程；不可查看审计和登录日志；不可管理用户 |
| `auditor` | 审计员 | 会话审计、登录日志 | 只读审计与登录日志；不可发起远程；不可管理用户 |

默认登录落点：
- `superadmin`、`admin`、`operator`：`/assets`
- `auditor`：`/audit`

## 数据模型

新增 `users` 表：

```sql
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'admin', 'operator', 'auditor')),
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
```

扩展会话与审计归因字段。若现有表结构不适合直接强改，可用 `ALTER TABLE ... ADD COLUMN` 兼容迁移：

```sql
ALTER TABLE sessions ADD COLUMN operator_user_id TEXT;
ALTER TABLE sessions ADD COLUMN operator_username TEXT;
ALTER TABLE sessions ADD COLUMN operator_role TEXT;

ALTER TABLE audit_logs ADD COLUMN actor_user_id TEXT;
ALTER TABLE audit_logs ADD COLUMN actor_username TEXT;
ALTER TABLE audit_logs ADD COLUMN actor_role TEXT;
```

字段语义：
- `operator_*`：远程会话发起人身份。
- `actor_*`：审计事件操作者身份。
- 历史数据为空时前端显示为“旧版本记录”或原有连接 ID。

## 迁移策略

启动时执行幂等迁移：

1. 建表和补列。
2. 若 `users` 为空，创建 `superadmin / infogo123`。
3. 若旧 `settings.admin_user/admin_pass_hash` 存在，迁移为 `admin` 角色账号。
4. 若旧 `admin_user` 与 `superadmin` 冲突，保留 `superadmin`，旧账号改名为 `admin` 或 `admin_legacy`，并写日志提示。
5. 迁移完成后，登录只查 `users`，旧 `settings` 单账号凭据不再作为登录来源。

## 后端设计

### Auth 模型

`Auth` 从“单账号内存凭据”升级为“用户仓储 + JWT 签发/校验”：

- 登录时按用户名查询 `users`。
- 校验 `enabled=1`。
- bcrypt 校验密码。
- JWT Claims 包含：
  - `sub`: `user_id`
  - `username`
  - `role`
  - `exp`

`AuthUser` 提取器返回完整身份：

```rust
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub role: Role,
}
```

### 权限校验

后端定义固定权限：

- `ViewAssets`
- `UseRemote`
- `ViewGrid`
- `ViewAudit`
- `ViewLoginLogs`
- `ManageUsers`
- `ManageSettings`

HTTP handler 入口按权限显式校验。前端菜单过滤只是体验优化，后端权限是最终边界。

### 用户管理 API

新增 API，均需要 `ManageUsers`：

- `GET /api/users`：用户列表，不返回密码 hash。
- `POST /api/users`：创建用户，字段 `username/password/role/enabled`。
- `PATCH /api/users/:id`：修改角色、启用状态。
- `POST /api/users/:id/reset-password`：重置密码。

保护规则：
- 不能停用 `superadmin`。
- 不能把 `superadmin` 降权。
- 不能删除或禁用最后一个可用的高权限账号；一期不做删除，降低误操作风险。
- 用户名唯一，创建和修改时 trim 后不能为空。

### 系统设置 API

`/api/settings/credential` 改为“当前用户修改自己的密码”。仅 `superadmin` 和 `admin` 可访问系统设置页面。用户管理里的重置密码走单独 API。

如果保留“修改用户名”，需要同步更新 `users.username`，并避免与其他账号冲突。

### WS 归因

WS 连接继续使用前端生成的连接 ID 作为连接路由 ID，但人员身份必须来自 token：

1. WS 建连时校验 token。
2. 服务端保存 `connection_id -> AuthUser` 映射。
3. WEB 发起远程、截图、文件、命令、聊天等操作时，服务端从连接上下文读取 `AuthUser`。
4. 会话创建写入 `operator_user_id/operator_username/operator_role`。
5. 审计事件写入 `actor_user_id/actor_username/actor_role`。

关键原则：前端传来的 `from: admin-xxxx` 只用于路由，不作为审计身份。

## 前端设计

### Auth Store

`useAuthStore` 保存：

- `token`
- `user`
- `role`
- `permissions`

登录成功后存储 token 与用户信息；刷新后通过 `/api/me` 恢复。

### 菜单与路由

`nav-config.ts` 给每个菜单配置 `permission`：

- 终端资产：`ViewAssets`
- 批量监控：`ViewGrid`
- 远程控制：`UseRemote`
- 会话审计：`ViewAudit`
- 登录日志：`ViewLoginLogs`
- 用户管理：`ManageUsers`
- 系统设置：`ManageSettings`

`AppSidebar` 过滤不可见菜单。`RequireAuth` 增加权限判断，无权限直接跳到该角色默认首页。

### 用户管理页面

新增 `/users` 页面：

- 表格展示用户名、角色、启用状态、创建时间、更新时间。
- 新增用户弹窗：用户名、初始密码、角色、是否启用。
- 行操作：启用/停用、重置密码、修改角色。
- 对 `superadmin` 行禁用停用和降权操作。

### 审计展示

会话审计和日志列表增加“操作人”列：

- 优先显示 `operator_username` 或 `actor_username`。
- 旧数据为空时显示“旧版本记录”。

## 安全与错误处理

1. 密码只保存 bcrypt hash。
2. 登录失败继续记录登录日志。
3. 停用用户后，其旧 token 在 TTL 内可能仍有效；一期在 `AuthUser` 提取器中查询用户 enabled 状态，确保停用立即生效。
4. 重置密码成功后不返回明文之外的敏感信息；操作界面只在提交时显示新密码。
5. 所有用户管理操作写审计日志，记录操作者和目标用户。
6. 普通用户不能通过 API 修改自己的角色或启用状态。

## 测试策略

后端：
- 用户迁移：空库创建 `superadmin`，旧 settings 迁移为 `admin`。
- 登录：禁用账号拒绝；密码错误拒绝；JWT 包含身份。
- 权限：`operator` 不能访问审计和用户管理；`auditor` 不能发起远程；`admin` 不能停用 `superadmin`。
- WS：远程会话创建后写入真实 `operator_username`。

前端：
- 不同角色菜单过滤正确。
- 无权限路由跳转正确。
- 用户管理页面能创建、停用、重置密码、修改角色。

发布验收：
- `superadmin / infogo123` 可登录并看到全部菜单。
- 新建 `operator` 后可远程，审计记录显示该账号。
- `auditor` 可看审计和登录日志，不能发起远程。
- 停用用户后登录失败，已有 token 访问 API 失败。

## 发布策略

只需要发布服务器和管理端：

1. 合入代码。
2. 跑 Rust 后端测试和 admin-web 构建。
3. 部署 Docker 镜像到生产。
4. 用 `superadmin` 登录创建客服账号。
5. 用客服账号发起一次远程，验证审计归因。

不需要重新发布 Windows/macOS/Linux 客户端安装包。

## 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 迁移误伤现有 admin 登录 | 管理员无法登录 | 启动时保留旧 admin hash，新增 superadmin 作为兜底 |
| 只隐藏前端菜单造成越权 | 审计员/操作员绕过 API | 后端每个敏感接口显式校验权限 |
| WS 仍使用随机连接 ID | 审计无法归因 | 服务端保存连接 ID 到 AuthUser 的映射 |
| 停用账号 token 未失效 | 被停用用户短期仍可操作 | AuthUser 提取器每次检查 users.enabled |
| 一次性改动过大 | 回归风险上升 | 先完成后端身份与权限，再接前端页面，最后补审计展示 |
