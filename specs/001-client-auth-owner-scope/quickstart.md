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

## 部署注意（强制登录模型 · rbac3 交叉审查结论）

被控端本版**强制登录**才能上线，与「无人值守 + 自动更新」现网模型冲突。合入自动更新通道前必须：

1. **闸住自动更新**：本版**不要**经 `latest.json`/rollout 推给现网 0.4.x fleet（会全部弹登录页掉线，需每台手动登录）。改手动/分批部署，或先在可运维机器验证。
2. **固定 JWT secret**：服务端生产环境**必须**设 `OHMYDESK_JWT_SECRET` 固定值。缺省随机 → 服务端每次重启即令**全 fleet 令牌失效** → 被控端集体掉线回登录页。
3. **拉长 token TTL**：默认已从 12h 调到 **7 天**；无人值守 fleet 可用 `OHMYDESK_TOKEN_TTL_SECS=<秒>` 调更大（权衡：本 token 同用于 web 管理端，过大增会话暴露面）。
4. **无人值守机器仍需人工登录一次**：接受此前提（本轮决策）；令牌过期后重连仍需重登（根治需 refresh token / 设备长效令牌，backlog）。

## 已知限制（backlog，不阻塞本特性）

- **归属可被 id 冒用（#3）**：9 位终端 id 非机密可自报，持有效账号者用自己 token 连 `from=<他人id>` 发 Register 可把该终端 owner 覆盖为自己 + 顶掉真机通道（越权 + DoS）。根治需设备身份（证书/机器绑定）。**本轮暂不处理**。
- **登录日志按 username（#5）**：superadmin 改用户名后本人历史不可见；用户名复用给他人会看到前任登录历史。根治需 login_log 补 user_id 列。
- **token 走 WS URL query（#6）**：`?token=` 易进反代/访问日志。D1 已定 query；如需可迁握手 header。
- **审计归属随换绑漂移（#8）**：审计经「session.to_id→当前 owner」关联，换绑后历史审计可见性随之变化；无 session 映射的审计（如截图 req_id）普通账号不可见。
