# WEB 按账户动态菜单授权 设计文档

> 状态：设计已与用户确认，待写实现计划。
> 前置：本 feature 演进自已上线的固定 4 角色 RBAC（master `7c285b3`）。复用其大部分链路，仅替换「权限来源」并新增 superadmin 配菜单/改名 UI。

## 目标（Goal）

把权限模型从「固定 4 角色 → 写死的菜单映射」改为「**按账户自定义菜单授权**」：superadmin 是唯一 god 账户，掌管所有账户的增删、菜单分配、改名、重置密码；每个普通账户持有一份独立的、可被 superadmin 编辑的功能菜单子集；人人可自助改自己密码。

## 非目标（Non-goals）

- 不做可编辑角色/角色模板（已否决，改按账户）。
- 不做多 superadmin（账户管理仅唯一 superadmin）。
- 不改客户端（被控端）——无角色/登录概念，零改动。
- 不改远控/审计的传输与注入逻辑，仅换权限来源。

## 架构（Architecture）

**两层权限模型**：
1. **superadmin**：唯一，隐式拥有全部权限 + 账户管理能力。由 bootstrap 建立，不可停用/降权/删除。
2. **普通账户**：持有一份存储的菜单权限集（`permissions`），无角色概念。只能拿到功能菜单子集，拿不到账户管理。

服务端仍是权限唯一权威：`/api/me` 下发该账户 permissions，前端据此过滤菜单/路由；每个敏感 HTTP/WS 操作服务端二次校验。前端不硬编码任何 role→permission 映射（延续已上线设计）。

**技术栈**：Rust/Axum/SQLx/SQLite/bcrypt/jsonwebtoken/ts-rs，React/Vite/Zustand/TypeScript，现有 Docker 部署。

## 权限清单（Permission keys）

**可被 superadmin 勾给普通账户的功能菜单**：
| key | 菜单 | 备注 |
|---|---|---|
| `view_assets` | 终端资产 | |
| `manage_assets` | └ 可删除终端 | 「终端资产」下子权限；仅 `view_assets` 已勾时可勾 |
| `view_grid` | 批量监控 | |
| `use_remote` | 远程控制 | 服务端 WS 闸强制（安全不变） |
| `view_audit` | 会话审计 | |
| `view_login_logs` | 登录日志 | |

**不可配（不出现在勾选器）**：
- `manage_users`（用户管理）——superadmin 独占。
- 自助改密——人人默认有，与菜单授权无关，不是可勾项。
- `manage_settings`（旧「系统设置」）——废弃为可配菜单；其唯一用途「自助改密」下沉为人人可见的「个人设置」。

superadmin 隐式拥有以上全部 + `manage_users`。

## 数据模型 / 迁移（Data model）

**`users` 表**：新增 `permissions TEXT NOT NULL DEFAULT ''`——普通账户存逗号分隔菜单键（如 `view_assets,manage_assets,use_remote`）。superadmin 不读此列，隐式全给。

**`role` 列**：现有 CHECK 约束（superadmin/admin/operator/auditor）与「按账户」模型冲突。SQLite 无法直接改 CHECK，走「建新表 + 拷数据 + 换名」迁移：`role` 退化为二值 tier 标记 `'superadmin' | 'user'`（保留供审计 operator_role 与「是否 god」判定）。迁移幂等，跑在 `db.rs` 现有 `connect()` 迁移链里。

**现网数据迁移**（生产已有 superadmin + admin）：
- `superadmin`：role 保持 `superadmin`，隐式全给，密码不变。
- `admin`：role → `user`，permissions → **全部功能菜单**（`view_assets,manage_assets,view_grid,use_remote,view_audit,view_login_logs`），密码不变。**失去 manage_users**（账户管理归 superadmin），符合确认。
- 任何遗留 operator/auditor（现网无）：role → `user`，permissions 按其旧角色映射迁移一次。

## 后端变更（Backend）

- **权限来源**：新增 `UserRecord.permissions: Vec<Permission>`（superadmin 解析为全集）。`Role::permissions()` 写死映射废弃，改为读用户存储集。`/api/login`、`/api/me` 下发该集。
- **用户管理 API 扩展**（均 superadmin-only，即 tier==superadmin）：
  - `PATCH /api/users/:id` 接受 `permissions: string[]`（覆盖式），校验：仅允许可配菜单键；`manage_assets` 依赖 `view_assets`；superadmin 目标拒改。
  - 新增改用户名：`PATCH /api/users/:id` 接受 `username`，唯一性校验，superadmin 目标拒改名（或允许——见开放点）。
  - 保留重置任意账户密码 `POST /api/users/:id/reset-password`。
- **自助改密**（人人，非 superadmin-only）：`POST /api/me/password`，body `{old, new}`，验旧密后更新自己。取代旧 manage_settings-gated 改密。
- **WS 远控闸 / 审计归属**：逻辑不变；权限判定从 `role.permissions().contains(UseRemote)` 改为「用户存储集含 use_remote 或 superadmin」。`operator_role` 写 tier（`superadmin`/`user`）。

## 前端变更（Frontend）

- **permissions.ts**：`Permission` 联合保留现有键（去角色概念，`Role` 类型收敛为 tier 或移除；`roleLabel` 相应调整为 superadmin/普通用户）。
- **Users 页（superadmin）**：每账户行/编辑态加**菜单勾选器**（5 复选框 + `view_assets` 下 `manage_assets` 子勾）+ 改用户名输入 + 重置密码（已有）。superadmin 账户行锁定（不可改权限/改名/停用）。
- **个人设置页**：人人可见（不受菜单授权约束），自助改密（验旧密）；superadmin 在此改自己密码。取代旧「系统设置」。
- **菜单/路由过滤**：复用已上线逻辑，继续消费 `/api/me` 的 permissions，零改。
- **资产删除按钮**：已按 `manage_assets` 门控（上一轮修复），继续生效。

## 复用 vs 新建（相对已上线 RBAC）

| 复用不动 | 需改 | 新增 |
|---|---|---|
| WS 身份绑定 ActorIdentity（Task5） | 权限来源:角色映射→用户集 | `users.permissions` 列 + 迁移 |
| 审计归属链路（Task5） | `/api/me`·`/login` 下发逻辑 | 菜单勾选器 UI |
| 前端菜单/路由按 permissions 过滤（Task6） | 用户管理 API（+permissions/+username） | `POST /api/me/password` 自助改密 |
| 资产删除 manage_assets 门控 | Users 页扩展 | 个人设置页 |

## 错误处理

- 配菜单校验失败（非法键 / manage_assets 缺依赖 / 改 superadmin）→ 400 `{error}`，前端弹错。
- 自助改密旧密错 → 400，前端提示。
- 改用户名撞名 → 400 稳定错误信息。
- superadmin 目标的任何降权/改名/停用/删除 → 后端拒 + 前端 UI 锁定双保险。

## 测试

- 后端:权限存取往返、迁移幂等（旧 role→permissions）、manage_assets 依赖校验、superadmin 双锁、自助改密验旧密、WS 闸按存储集判定、改名唯一性。
- 前端:菜单勾选器交互、个人设置自助改密、菜单按 permissions 过滤、superadmin 行锁定、build+test 绿。
- 集成:superadmin 配一个「仅审计」账户→该账户只见审计菜单、不能远控/删资产;admin 迁移后全功能但无用户管理。

## 部署

纯服务端 + admin-web + 加性迁移 → Track A（Docker，不走 CI）。迁移在容器启动 `connect()` 跑；现网 admin/superadmin 平滑迁移不锁人。

## 已定决策

- **superadmin 不可被改用户名（含改自己的登录名）**：与不可降权/不可停用/不可删除一致，后端拒 + 前端 UI 锁定双保险。

## 🔴 全程铁律（贯穿实现与验证）

**只对管理平台（server + admin-web）做加法，绝不影响 WEB 远程控制与客户端远控功能。**
- 客户端（`src/client`）零改动。
- 远控传输/注入/会话链路不改；WS 远控闸仅换权限来源（角色映射→用户存储集），superadmin 与被授 `use_remote` 的账户远控行为与现网完全一致。
- 每个改动点须能论证「不触及远控路径」；发版后 e2e 必须实测远控仍通再收尾。
