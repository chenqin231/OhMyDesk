# Quickstart / 验证手册: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope`

## 构建
```bash
cargo build -p server -p client                       # workspace crate 名：server / client / protocol
cargo test  -p server                                 # 服务端隔离/迁移/闸单测
# admin-web: cd src/admin-web && <pkg> build（auth store 补 user_id 改动极小）
```

## 端到端隔离验证（对应 SC-001/002/003/004）

### 准备
1. WEB 用户管理建两个普通账号 A、B（superadmin 内置）。
2. 起服务端（本地或 Docker）。

### 场景 1 — 归属绑定 + 列表隔离（SC-001）
1. 机器 M1 起新版客户端 → 用 A 登录 → M1 上线。
2. 机器 M2 起新版客户端 → 用 B 登录 → M2 上线。
3. 管理端以 A 登录：**只见 M1**，无 M2。
4. 管理端以 B 登录：**只见 M2**，无 M1。
5. 管理端以 superadmin 登录：见 M1 + M2 +（若有）旧端。
   - ✅ 通过标准：A/B 视图各自终端数=1，交叉可见数=0。

### 场景 2 — 远控范围闸（SC-002）
1. A 登录管理端，对 M2（归属 B）发起远控（含手动构造请求）≥5 次。
   - ✅ 全部被拒 + Toast「无权远控该终端」+ 审计出现「被拒」记录。
2. A 对 M1（自己的）远控 → 正常建会话（行为同现状）。

### 场景 3 — 反伪造（SC-003）
1. 改造客户端在 `EndpointInfo` 注入伪造 `owner=B`，用 A 登录上线。
   - ✅ 服务端 `endpoint_registry.owner_id` 仍为 A 的 user_id；B 视图看不到该机。
   - 验证：`sqlite3 <db> "SELECT id, owner_id FROM endpoint_registry"` → owner_id = A.id。

### 场景 4 — 旧端兼容（SC-004）
1. 一台旧客户端（无登录能力）不升级，直接上线。
   - ✅ 正常注册在线；owner_id=NULL；普通账号列表不可见；superadmin 可见且可远控。
   - ✅ 旧端截屏/注入/远控功能无回归。

### 场景 5 — 记住凭据 + 换绑（SC-005/006）
1. M1 用 A 登录后重启 3 次 → 每次自动上线、归属 A、无需人工输入。
2. M1 点「注销」→ 确认 → M1 离线（superadmin 视图仍显示归属 A 的离线终端）。
3. M1 用 B 登录 → 重新上线，归属翻转为 B；A 视图 ≤1 次刷新后不再可见 M1，B 视图可见。

### 场景 6 — 登录/异常反馈（FR-001）
- 错密码 → inline「账号或密码错误」，密码清空、账号保留。
- 断网点登录 → inline「无法连接服务器，请检查网络后重试」，按钮恢复。
- 空值 → inline「请输入账号/密码」。

## 审计口径核对（C1 修正）
- A 远控 M1 产生的审计，在 A 的审计页可见（经 `session.to_id→owner` 归属）。
- 涉 M2 的审计不出现在 A 的审计页。
- 登录日志：A 只见自己的登录记录（按 username）。
