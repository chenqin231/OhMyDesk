# Spec C 设计：Windows 客户端在线静默自动更新

> 日期：2026-06-30（v2，含安全审阅整改）
> 范围：客户端（仅 Windows 自替换）+ 一个本地发布脚本。服务端 / protocol / admin-web **零改动**。
> 关联记忆：[[five-feature-roadmap]]、[[release-publish-process]]

---

## 0. 安全立场（先于一切）

`latest.json` 对客户端而言**就是一张代码执行授权令**：静默自更新意味着「清单说装什么，客户端就以高权限装什么并重启」。因此本设计的信任根**不是** TLS、**不是**「服务器是控制面信任锚」，而是**一把离线 Ed25519 私钥**——只有持有它的人能签发更新。

威胁模型明确防御：拿到 `/www/wwwroot/.../downloads/` 写权限的攻击者（webshell、泄露的 scp/发布凭据、nginx 配错、静态目录遍历写入）——这是独立于 WS 控制面的攻击面。无离线私钥即无法伪造合法 `.minisig`，客户端拒绝任何未签名/签名不符的清单。

---

## 1. 背景与目标

OhMyDesk 客户端正面向客服团队铺开（主力 Windows 单 exe），当前 0.2.1 **无自动更新**，每次发版逐台手动重装。本 Spec 让 Windows 客户端具备**带离线签名的**在线静默自更新。

**目标**：
- Windows 单 exe 无人值守静默自更新：检测→下载→**验签**→**校验完整性**→替换→重启，全自动。
- 复用既有 nginx `/downloads/` gzip 通道下载，单次省 ~4.5MB 公网带宽。
- 不打断**任何**进行中的远控会话（主控 cur_session 与被控 ctrl_session 皆然）。
- 具备**灰度 + 暂停**能力，坏包不至 6h 内扩散全网。

**非目标（v1 不做）**：
- Linux/macOS 自我替换（仅「有新版」日志/提示，走手动）。
- 自动回滚（用灰度 + 暂停 + 人工回退兜底，见 §9）。
- 密钥轮换机制（v1 单公钥内置；轮换需发客户端更新，见 §5）。

---

## 2. 架构选型

三方案选 **A（客户端轮询静态清单 + HTTPS 下载）**，理由不变（带宽最省、改动面最小、零服务端/协议耦合）：

| 方案 | 检测/下载 | 线上带宽 | 改动面 |
|---|---|---|---|
| **A 客户端轮询静态清单 + HTTPS 下载** ✅ | GET `latest.json`(+`.minisig`) + GET exe | **5.5MB（复用 gzip_static）** | 仅客户端 + 本地发布脚本 |
| B 服务端 WS 推「有新版」+ HTTPS 下载 | 混合 | 5.5MB | +服务端 +protocol +容器清单耦合 |
| C 全走 WebSocket 流式推二进制 | WS | ~13MB（base64 膨胀、丢 gzip） | +服务端 +protocol +hub 负载 |

签名不改变选型：分离签名 `latest.json.minisig` 同为 `/downloads/` 下静态文件，方案 A 仍零服务端改动。

---

## 3. 数据流

```
启动延迟 ~30s + 每 6h（间隔可经环境变量覆盖）
 └─① 解析更新基址（§7 同源 HTTPS 策略），非法/非 TLS → 更新禁用，记日志退出
 └─② GET <base>/latest.json   （≤64KB，连接超时 10s）
       GET <base>/latest.json.minisig
       └─③ minisign-verify(内置公钥, latest.json 原始字节, minisig)
             └─ 失败 → 丢弃 + 告警日志 + 下周期重试（绝不使用未验签清单）
       └─④ 解析 Manifest；校验 manifest.windows_x86_64.url 与 <base> 同源 https
       └─⑤ 灰度门控：!enabled → 跳过；is_newer 且
             (current < min_version 强制) 或 (bucket(endpoint_id) < rollout_percent) → 继续，否则跳过
       └─⑥ GET <url>（Accept-Encoding: gzip → 线上 5.5MB）
             流式写临时文件，**边写边增量 SHA-256**，
             解压后累计字节 > 50MB 上限 → 中止（防 gzip 炸弹）；读超时 120s
       └─⑦ 末字节数 == manifest.size 且 SHA-256 == manifest.sha256 ？
             ├─ 否 → 删临时文件 + 告警 + 下周期重试
             └─ 是 → ⑧ 应用窗口：
                       检查 cur_session 与 ctrl_session 皆空 → 置 UPDATING=true →
                       二次复检双会话仍空（防竞态）→ self_replace(临时文件) → spawn 新 exe → 本体退出
                       （任一会话非空 → 解除 UPDATING，推迟到下周期）
```

- **gzip 透明解压**：`sha256`/`size` 均针对**解压后最终 exe**。客户端带 `Accept-Encoding: gzip`，nginx `gzip_static` 回 `.exe.gz`，ureq（`gzip` 特性）透明解压。
- **替换后**：新进程重走 `elevate::ensure_elevated()` + 反连重注册（`net::run` 重连循环天然恢复），仅短暂断连。
- **资源上限汇总**：manifest ≤64KB；exe 下载与解压均 ≤50MB（且 manifest.size 必须 ≤50MB）；连接超时 10s、读超时 120s；全程流式、不在内存缓存整包。

### 清单格式 `latest.json`（分离签名 `latest.json.minisig`）

```json
{
  "version": "0.3.0",
  "windows_x86_64": {
    "url": "https://rc.guoziweb.com/downloads/ohmydesk-client-windows-x86_64.exe",
    "sha256": "<解压后 exe 的 64 位十六进制 SHA-256>",
    "size": 10420736
  },
  "enabled": true,
  "rollout_percent": 100,
  "min_version": null,
  "notes": "本次更新说明（可选）"
}
```

- 签名是**分离文件** `latest.json.minisig`，覆盖 `latest.json` 的精确字节（标准 minisign，无需 JSON 规范化）。
- `enabled`：总开关，false 即全网暂停更新（秒级生效，客户端拉到即跳过）。
- `rollout_percent`（0–100）：客户端按 `bucket = sha256(endpoint_id)[0..] % 100` 确定性分桶，`bucket < rollout_percent` 才更新。同一台机器分桶恒定，便于「10% 灰度→观察→100%」逐步放量。
- `min_version`（可选 semver）：客户端当前版本低于它则**无视灰度强制更新**（安全补丁强推全网）；缺省不强制。

---

## 4. 模块设计

新增 `src/client/src/update.rs`，在 `main.rs` 起 runtime 处（约 `main.rs:109`）以**独立 std 线程**挂起（ureq 同步阻塞、更新低频，独立线程最简，不触碰 async select 与 X11 worker）。

### 公开接口

```rust
/// 启动更新守护线程。接收双会话共享态，空闲才替换。
pub fn spawn_update_daemon(server_url: String, cur_session: SharedSession, ctrl_session: SharedSession);

/// 会话准入侧查询：替换窗口内拒绝新会话。
pub fn is_updating() -> bool;   // 读 static AtomicBool UPDATING
```

`is_updating()` 在被控端「收到 ConnectRequest / 即将建立会话」的准入路径加一处判断，UPDATING 时拒绝（回执「正在更新，请稍后」），避免替换窗口内开新会话。这是本模块对外**唯一**新增耦合点。

### 内部单元（可独立单测者标注）

| 单元 | 职责 | 依赖 | 可测 |
|---|---|---|---|
| `resolve_base(server_url, env_override) -> Option<Url>` | 同源 HTTPS 基址推导（§7） | url | ✅ 纯函数 |
| `Manifest` + `parse(&[u8]) -> Result` | 清单反序列化（容缺 `min_version`/多余字段） | serde_json | ✅ |
| `verify_manifest_sig(pubkey, json_bytes, sig_bytes) -> bool` | Ed25519 验签 | minisign-verify | ✅（正/错/篡改） |
| `same_origin(base, url) -> bool` | 下载 URL 同源校验 | url | ✅ |
| `is_newer(latest, current) -> bool` | semver 比对 | semver | ✅ |
| `should_update(manifest, current, endpoint_id) -> bool` | enabled + 强制 + 分桶门控 | — | ✅ |
| `bucket(endpoint_id) -> u8` | 确定性分桶 0–99 | sha2 | ✅（同 id 恒定） |
| `download_verified(url, sha256, size) -> Result<TempPath>` | 流式下载+增量哈希+上限 | ureq, sha2, tempfile | 手测（真实网络） |
| `apply(tmp) ` | 双会话门控 + UPDATING + self_replace + relaunch | self_replace | 手测（Windows） |

### 平台分流

- **Windows**：完整 ①~⑧。
- **非 Windows**（`#[cfg(not(windows))]`）：仅 ①~⑤ 检测，发现新版仅记日志「发现新版 vX，请手动更新」，不下载、不替换；编译期裁掉 self_replace 路径。

---

## 5. 安全模型（v1 即完整）

| 防御 | 机制 | 防御对象 |
|---|---|---|
| **真实性** | 内置 Ed25519 公钥 verify `latest.json.minisig` | 静态目录被写入/发布凭据泄露/中间人伪造清单 |
| **完整性** | 签名清单内 `sha256` + `size`，下载流式校验 | 包损坏/截断/被换 |
| **同源** | 下载 URL 必须与更新基址同源 https | 清单被改指向恶意主机 |
| **传输** | 全程 TLS（wss/https 派生） | 窃听/篡改 |
| **资源** | 大小/超时/流式/防炸弹上限 | DoS、内存/磁盘耗尽 |
| **爆炸半径** | enabled 暂停 + rollout 灰度 + min_version | 坏包扩散、人工回退窗口 |

**信任根管理**：
- 一次性 `minisign -G` 生成密钥对；**私钥仅留本机/离线、绝不进仓库、绝不进 CI**；公钥（base64）作 `const` 编入 `update.rs`。
- 发版时本机 `minisign -Sm latest.json` 产 `.minisig`（见 §8）。
- 公钥轮换需发一次客户端更新（鸡生蛋），v1 接受此约束，单公钥；二期可内置「当前+下一把」双公钥平滑轮换。

---

## 6. 依赖与体积

```toml
# 跨平台
sha2 = "0.10"
semver = "1"
minisign-verify = "0.2"   # 仅验签，纯 Rust，体积小，离线签名用 minisign CLI
self_replace = "1"        # Windows 运行中 exe 自替换（rename .old → 移入新）
tempfile = "3"            # 安全临时文件落盘
url = "2"                 # 基址/同源解析

# Windows：复用 native-tls（SChannel），与 tokio-tungstenite 一致
[target.'cfg(windows)'.dependencies]
ureq = { version = "2", default-features = false, features = ["native-tls", "gzip"] }

# 非 Windows：复用 rustls，与 tokio-tungstenite 一致
[target.'cfg(not(windows))'.dependencies]
ureq = { version = "2", default-features = false, features = ["tls", "gzip"] }
```

**Windows 交叉编译验证项（windows-gnu）**：`self_replace`、`minisign-verify`、`ureq+native-tls` 三者须能在 `x86_64-pc-windows-gnu` 下交叉编译通过（CI/本地 `packaging/windows/build-windows.sh`），且产物仍为单 exe 无 mingw DLL 依赖（既有脚本第 4 步会校验）。二进制净增预估 ~1–1.5MB（pre-gzip），被 gzip 下载收益抵消有余。

---

## 7. 同源 HTTPS 更新基址策略

更新基址**必须 https**，推导规则：

1. `OHMYDESK_UPDATE_BASE_URL`（显式覆盖）存在：必须 `https://`，否则更新禁用。供开发/内网无 TLS 时显式开启。
2. 否则从 `OHMYDESK_SERVER` 推导：`wss://host[/...]` → `https://host/downloads/`。
3. `OHMYDESK_SERVER` 为 `ws://`（明文）：**禁止**自动降级到 `http://`；更新禁用，记日志「非 TLS 服务器，未设 OHMYDESK_UPDATE_BASE_URL，更新已禁用」。
4. manifest 内 `url` 必须与最终基址**同源**（同 scheme+host+port）且 https，否则拒绝该清单。

生产（`wss://rc.guoziweb.com/ws`）自然得 `https://rc.guoziweb.com/downloads/`，无需任何额外配置。

---

## 8. 发版流程与原子发布脚本

新增 `packaging/download/publish-windows-update.sh`（**本机运行**，私钥在本机）：

1. 入参：新 exe 路径 + 版本号（+ 可选 rollout_percent/min_version/enabled）。
2. 算 `sha256sum`、`size`；`gzip -9 -kf` 产 `.exe.gz`。
3. 生成 `latest.json`（含上述字段）→ `minisign -Sm latest.json` 产 `latest.json.minisig`（提示输入私钥口令）。
4. **原子发布顺序**（关键）：
   先 scp/上传 **exe + exe.gz**（新产物）→ 确认到位 →
   最后上传 `latest.json` + `latest.json.minisig`，且服务器侧 `mv latest.json.tmp latest.json` 原子切换。
   保证：客户端永远不会读到「指向尚未上线/半传 exe」的新清单。
5. 校验：`curl -sk https://rc.guoziweb.com/downloads/latest.json` 返回新版本；`minisign -Vm latest.json` 本地用公钥自验通过。
6. **保留上一版 exe**（不立即删）以备人工回退：回退即把 `latest.json` 改回旧版本 + 旧 sha256 重签发布。

落档到 [[release-publish-process]]。私钥与 minisign 操作单独记一条参考记忆（不含私钥本体）。

---

## 9. 爆炸半径控制与回退（替代自动回滚）

v1 不做进程级自动回滚，改用**发布侧三道闸 + 人工回退**：

- **灰度**：新版先 `rollout_percent: 10` 发布 → 观察少量客服机 → 逐步 50→100。坏包最多波及当前百分比。
- **暂停**：发现异常立即把 `enabled` 改 false 重签发布 → 全网秒级停更。
- **人工回退**：保留上一版 exe，回退即用旧版本号 + 旧 sha256 重签 `latest.json`；已更新到坏版的机器在下个周期「降级」回旧版（`is_newer` 对降级返回 false，故回退需 min_version=0 语义或显式允许降级——v1 简化：回退场景由人工现场重装兜底，灰度阶段坏包面足够小）。

> 说明：v1 不实现「自动降级」（`is_newer` 只升不降）。灰度把坏包面压到可人工处理的规模，是 v1 对「无自动回滚」的等价替代。

---

## 10. 测试策略

- **单测（CI 可跑，纯函数/纯逻辑）**：`resolve_base`（wss 派生 / ws 拒绝 / 显式 https 覆盖 / 非 https 拒绝）、`Manifest::parse`（正常/缺 min_version/多余字段/超 64KB 拒绝）、`verify_manifest_sig`（合法签 / 错签 / 篡改 json）、`same_origin`（同源/异源/异端口）、`is_newer`、`should_update`（enabled=false / 强制 min_version / 分桶边界 0·99·阈值）、`bucket`（同 id 恒定、分布合理）、`verify_sha256` + size 双校验。
- **手测（真实环境）**：真实拉 `latest.json`+`.minisig`、gzip 下载解压、50MB 上限与超时、Windows self_replace + relaunch、双会话门控（主控中/被控中不替换、皆空闲替换）、UPDATING 拒新会话。
- **不回归**：client/server/protocol 既有测试保持绿（本 Spec 不碰服务端/协议）。

---

## 11. Bootstrap

带本更新器的**首版（拟 0.3.0）仍需手动装一次**（现网 0.2.1 无自更新逻辑，无法自我拉起）。0.3.0 装好后全自动。一次性成本，无法绕过。发布 0.3.0 时同步：生成密钥对、把公钥编入客户端、首次 `publish-windows-update.sh` 产出带签清单。

---

## 12. 已知边界（v1 不做，二期候选）

- 自动回滚 / 自动降级（v1 用灰度+暂停+人工回退替代）。
- Linux/macOS 自我替换（deb 需 root/dpkg、mac 无打包脚本）。
- 公钥轮换（v1 单公钥；二期内置双公钥）。
- 服务端可调下发周期 / 服务端驱动灰度（v1 周期固定 6h、灰度靠静态 manifest 字段）。
