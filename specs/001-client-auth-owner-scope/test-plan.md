# Test Plan: 客户端账号认证与按用户数据隔离

**Branch**: `001-client-auth-owner-scope` | **人格**: 测试工程师（破坏性思维 / 边界 / 组合 / 时间）
**原则**: 测试为发现问题，不为证明能跑。每个 AC 至少一条正常 + 一条异常；高风险区（`hub.rs` 广播、src/client 首改、协议兼容）加对抗与回归。

## 风险导向优先级
| 区域 | 风险 | 重点手段 |
|---|---|---|
| R1 push_list 广播改造（T019） | **高**（核心链路，泄露/回归） | 集成 + 推送级对抗 + 真机 |
| R2 反伪造 owner（T009） | **高**（越权=安全） | 对抗单测（伪造字段/越权控制） |
| R3 src/client 首改（T012-15） | 中（远控劣化） | 手工 + 回归基线对比 |
| R4 协议加字段（T003/T026） | 中（旧端兼容） | 旧 JSON 反序列化单测 |
| R5 一机多账户/换绑（T015） | 中（归属错乱） | 组合 + 时序 |

---

## 一、服务端自动化用例（cargo test · 单元/集成）

### TC-S01：owner 落库与可见性过滤（覆盖 AC-002-H1 / AC-005-H1/H2 · T002/T005/T016）
- **前置**: 内存 SQLite；账号 A(id=ua)、B(id=ub)。
- **步骤**: upsert(ep1, owner=ua)、upsert(ep2, owner=ub)、upsert(ep3, owner=None)。
- **预期**: `views_visible_to(Some(ua),false)`={ep1}；`(Some(ub),false)`={ep2}；`(None,true)`={ep1,ep2,ep3}；`(Some(ua),false)` 不含 ep3。
- **状态**: ⬜

### TC-S02：反伪造 owner（覆盖 AC-002-E1 · SC-003 · T006/T009）**对抗**
- **前置**: token=A 的连接。
- **步骤**: 发 Register，`info` 内注入伪造字段 `owner="ub"` / `owner_id="ub"` / 篡改 name 夹带。
- **预期**: 落库 `endpoint_registry.owner_id == ua`（≠ub）；B 视图不含该 ep。
- **边界追加**: token=A 但 `env.from` 伪装成别的 ep id → owner 仍取连接身份 ua，不被 from 欺骗。
- **状态**: ⬜

### TC-S03：无 token 旧端归 NULL（覆盖 AC-008-H1 · SC-004 · T025/T027）
- **步骤**: 不带 token 连接发 Register。
- **预期**: 上线成功、owner_id=NULL；普通 A 不可见、superadmin 可见。**不因缺 token 被 close**（gate 只拦 admin- 前缀）。
- **状态**: ⬜

### TC-S04：推送级隔离（覆盖 AC-005-E2 · T016/T019）**对抗·高风险**
- **步骤**: A、B 各在线 admin 连接；ep(owner=B) 触发上线 → push_list。
- **预期**: A 连接 mpsc 收到的 EndpointList **任何一帧**都不含 ep(B)；superadmin 连接收全量。
- **边界**: 三触发点各测一遍（Register / Heartbeat / admin 首帧）；owner=None 的 ep 只进 superadmin 帧。
- **状态**: ⬜

### TC-S05：远控范围闸（覆盖 AC-006-H1/E1/E2 · SC-002 · T017/T020）**对抗**
- **步骤**: actor=A 对 target(owner=B) 发 ConnectRequest；再对 target(owner=A)；superadmin 对任意；A 对 target(owner=None)。
- **预期**: B→拒绝且**不建会话**+审计 result=被拒；A→放行；superadmin→放行；None→拒绝（非 superadmin）。
- **越权追加**: A 持 use_remote 权限但非 owner → 仍拒绝（权限位≠范围）；连发 5 次拒绝率 100%。
- **状态**: ⬜

### TC-S06：审计/会话按 session.to_id 过滤（覆盖 AC-007-H1/H2 · T018/T021）
- **前置**: sessions: s1(to=ep_A), s2(to=ep_B)；audit: a1(session=s1), a2(session=s2), a3(截图 req_id 无 session)。
- **步骤**: `query_audit(A)`、`query_sessions(A)`、superadmin 全量。
- **预期**: A 审计={a1}（不含 a2；a3 因无 session 不呈现，**验证无泄露**）；A 会话={s1}；superadmin 含全部。
- **状态**: ⬜

### TC-S07：登录日志按 username（覆盖 AC-007-H3 · T018/T022）
- **步骤**: login_log 有 A、B 各若干行；`query(username=A)`、superadmin。
- **预期**: A 只见自己行；superadmin 全量。
- **边界**: username 大小写/同名边界（若系统 username 唯一则 N/A）。
- **状态**: ⬜

### TC-S08：协议向后兼容（覆盖 AC-008-E1 · R4 · T026）
- **步骤**: 用**旧版** EndpointView/EndpointInfo JSON（无 owner_id 键）反序列化。
- **预期**: `owner_id=None`，其余字段完整，不报错、不丢消息。
- **对抗**: 未知多余字段 JSON 也不使整条失败。
- **状态**: ⬜

### TC-S09：迁移幂等（覆盖 T004）
- **步骤**: 对已有 owner_id 列的库再跑 `ensure_identity_columns` 两次。
- **预期**: PRAGMA 守卫下不重复加列、不报错；存量行 owner_id 保持。
- **状态**: ⬜

### TC-S10：重启回灌保留 owner（覆盖 data-model 换绑语义 · T005）
- **步骤**: upsert(ep, owner=A)→db_save→新 Registry `load_from_db`。
- **预期**: 回灌后 ep 离线但 owner_id=A 保留（历史归属可追溯）。
- **状态**: ⬜

---

## 二、客户端手工用例（Slint UI）

### TC-C01：登录正常路径（覆盖 AC-001-H1 · T011/T014/T015）
- **步骤**: 输有效账号密码 → 点登录。
- **预期**: 按钮转「登录中…」→ 成功 → 登录页消失，顶栏「已登录:<user> ●在线」；WS 以 `?token=` 建连。
- **状态**: ⬜

### TC-C02：登录异常四态（覆盖 AC-001-E1/E2/E3/E4）**边界**
| 输入 | 预期 inline 文案 | 附加 |
|---|---|---|
| 错密码 | 账号或密码错误 | 密码框清空、账号保留、焦点回密码 |
| 断网 | 无法连接服务器，请检查网络后重试 | 按钮由 loading 恢复可点 |
| 禁用账号 | 账号已被禁用，请联系管理员（不可区分则兜底 E1） | — |
| 空账号/空密码 | 请输入账号 / 请输入密码 | 焦点定位首个空框；不发请求 |
- **边界追加**: 超长账号(>256)、含空格/中文/emoji、仅空格密码 → 不崩溃，按后端结果或本地校验处理。
- **状态**: ⬜

### TC-C03：记住凭据自动上线（覆盖 AC-003-H1 · SC-005 · T010/T013）
- **步骤**: 登录成功后重启客户端 3 次。
- **预期**: 每次跳过登录页自动上线、归属原账号，人工输入次数=0。
- **状态**: ⬜

### TC-C04：凭据失效回落（覆盖 AC-003-E1/E2 · AC-002-E2）**时间/对抗**
- **步骤**: (a) 篡改/等 token 过期后重启；(b) 服务端删除该账号后重启；(c) 在线中 token 被 close 1008。
- **预期**: (a) 回登录页 +「登录已过期，请重新登录」；(b)「账号不可用，请重新登录」；(c) 运行中掉线 → 回登录页提示重登，不静默卡死。
- **状态**: ⬜

### TC-C05：注销与换绑（覆盖 AC-004-H1/H2/E1 · SC-006 · T015）**组合**
- **步骤**: A 登录在线 → 点注销 → Modal「注销后本机将下线,确定?」→ 确定；再用 B 登录。
- **预期**: 确定 → WS 断、credential.json 删、回登录页；服务端该终端离线且归属仍 A。B 登录后重新上线、归属翻转 B；A 视图 ≤1 次刷新后不见该机、B 视图可见。取消 → 保持在线不变。
- **状态**: ⬜

### TC-C06：服务器地址高级项（覆盖 FR-001 UI）
- **步骤**: 展开「高级」改服务器地址为错误地址登录。
- **预期**: 折叠默认隐藏；改错地址 → 网络错 inline；改回正确可登录。
- **状态**: ⬜

---

## 三、Web 端手工用例（admin-web）

### TC-W01：三页隔离与空态（覆盖 AC-005-E1 / AC-007-E1 · SC-001）
- **步骤**: A、B 各绑 1 终端；以 A 登录 web 看 终端/监控/远程/审计/会话/登录日志。
- **预期**: 只见 A 的；他人记录数=0；A 无终端时列表空态「暂无你负责的终端」、审计空态「暂无记录」。
- **状态**: ⬜

### TC-W02：越权远控 Toast（覆盖 AC-006-E1）
- **步骤**: A 尝试远控 B 的终端（含手动构造请求）。
- **预期**: 被拒 + 拒连结果卡片 RejectedCard 显示原因「无权远控该终端」；审计出现被拒记录。（web 无 Toast，走 `remoteRejectReason`→RejectedCard）
- **状态**: ⬜

### TC-W03：superadmin 全量（覆盖 AC-005-H2 / AC-006-E2）
- **步骤**: superadmin 登录。
- **预期**: 见全部终端（含 owner=NULL 旧端）+ 可远控任意 + 审计/日志全量。
- **状态**: ⬜

---

## 四、回归与非功能

### TC-R01：远控零回归（覆盖 T029 · Constraint 零回归）**高风险**
- **步骤**: superadmin 远控自有在线终端，逐项操作截屏刷新 / 键鼠注入 / 命令 / 文件传输 / 聊天。
- **预期**: 帧率、时延、功能与改动前基线一致，无卡顿/错位/掉线新增。
- **状态**: ⬜

### TC-R02：新旧端并存（覆盖 SC-004 · Compatibility NFR）
- **步骤**: 旧端(不升级) + 新端(登录) 同时在网。
- **预期**: 两者都在线；旧端上线成功率≥升级前；superadmin 均可控。
- **状态**: ⬜

### TC-N01：凭据安全（覆盖 Security NFR · T031）
- **步骤**: 检查客户端日志 / 界面 / credential.json。
- **预期**: token 不以明文出现在日志与界面；credential.json 非世界可读（Unix 0600）。
- **状态**: ⬜

### TC-N02：一机多账户时序（覆盖边界 · R5）**组合/时间**
- **步骤**: 同机 A 登录→注销→B 登录→注销→A 再登录，快速连续。
- **预期**: 每次 owner 正确覆盖为最后登录者；无残留 B 归属；无并发多归属错乱。
- **状态**: ⬜

---

## 覆盖度自检
- 每条 FR 的 Happy + Error 均有用例：FR-001(TC-C01/C02)、FR-002(TC-S01/S02)、FR-003(TC-C03/C04)、FR-004(TC-C05)、FR-005(TC-S01/S04/W01)、FR-006(TC-S05/W02)、FR-007(TC-S06/S07/W01)、FR-008(TC-S03/S08)。
- 每条 SC 有验证：SC-001(TC-W01/S06)、SC-002(TC-S05)、SC-003(TC-S02)、SC-004(TC-R02/S03)、SC-005(TC-C03)、SC-006(TC-C05)。
- 破坏性补充：反伪造(TC-S02)、推送级泄露(TC-S04)、越权(TC-S05/W02)、时序错乱(TC-N02)、协议兼容(TC-S08)、凭据泄露(TC-N01)、远控回归(TC-R01)。
