# Spec C 设计：Windows 客户端在线静默自动更新

> 日期：2026-06-30
> 范围：客户端（仅 Windows 自替换）。服务端 / protocol / admin-web **零改动**。
> 关联记忆：[[five-feature-roadmap]]、[[release-publish-process]]

---

## 1. 背景与目标

OhMyDesk 客户端正面向客服团队铺开（主力为 Windows 单 exe）。当前版本（0.2.1）**无任何自动更新能力**：每次发新版都要逐台手动重装。本 Spec 让客户端具备「检测→下载→校验→替换→重启」的在线自更新，消除手动重装。

**目标**：
- Windows 单 exe 无人值守静默自更新（下载、校验、替换、重启全自动）。
- 复用既有 nginx `/downloads/` gzip 通道下载，单次省 ~4.5MB 公网带宽。
- 不打断进行中的远控会话；仅在空闲时替换。

**非目标（v1 不做）**：
- Linux/macOS 自我替换（仅「有新版」日志/提示，走手动）。
- 离线密钥签名（清单预留 `sig` 字段，二期可加）。
- 新版自损坏的自动回滚。

---

## 2. 架构选型

三方案对比，选 **A（客户端轮询静态清单 + HTTPS 下载）**：

| 方案 | 检测/下载 | 线上带宽 | 改动面 |
|---|---|---|---|
| **A 客户端轮询静态清单 + HTTPS 下载** ✅ | 客户端 GET `latest.json` + GET exe | **5.5MB（复用 gzip_static）** | 仅客户端，+1 个精简 HTTP 依赖 |
| B 服务端 WS 推「有新版」+ HTTPS 下载 | 混合 | 5.5MB | +服务端 +protocol +容器清单耦合 |
| C 全走 WebSocket 流式推二进制 | WS | ~13MB（base64 膨胀、丢 gzip） | +服务端 +protocol +hub 负载 |

**选 A 的第一性理由**：出口字节无论走哪条路都只从 rc.guoziweb.com 公网网卡离开一次，egress 成本相同；唯有 A 复用刚配好的 `gzip_static`（详见 [[release-publish-process]]），每次下载省 ~4.5MB。代价是客户端新增一个精简 HTTP 客户端（ureq），按平台复用既有 TLS 后端、不引第二套 TLS；二进制净增约 1MB，被 gzip 下载收益抵消有余。A 同时是**改动面最小、耦合最低**的方案：服务端、protocol、admin-web 全不动。

---

## 3. 数据流（五步）

```
启动延迟 ~30s + 每 6h
  └─① GET https://<host>/downloads/latest.json   (TLS)
        └─② semver 比对 latest.version vs env!("CARGO_PKG_VERSION")
              ├─ 不更新 → 等下个周期
              └─ 更新   → ③ GET <url> (Accept-Encoding: gzip → 线上 5.5MB，ureq 透明解压回 10MB)
                          └─④ SHA-256(下载内容) == latest.windows_x86_64.sha256 ?
                                ├─ 否 → 丢弃 + 记日志 + 下周期重试
                                └─ 是 → 暂存于 临时文件
                                        └─⑤ 仅当 ctrl_session 空闲：
                                              self_replace(临时文件) → spawn 新 exe → 本体退出
```

- **host 推导**：从 `OHMYDESK_SERVER`（`wss://host/ws`）推导更新基址 `https://host/downloads/`，使自定义服务器也一致重定向更新源；缺省 host = `rc.guoziweb.com`。
- **gzip 透明解压**：清单中的 `sha256` 是**解压后**（最终 exe）的哈希。客户端带 `Accept-Encoding: gzip` 请求 `.exe` URL，nginx `gzip_static` 回 `.exe.gz`（content-encoding: gzip），ureq（启 `gzip` 特性）透明解压，校验对得上。
- **空闲判定**：被控端共享 `ctrl_session: Arc<Mutex<Option<String>>>`；为 `None` 即无人远控，方可替换；否则推迟到下个周期。
- **替换后**：新进程重新走 `elevate::ensure_elevated()` 提权 + 反连重注册（`net::run` 重连循环天然恢复），仅短暂断连。

### 清单格式 `latest.json`

```json
{
  "version": "0.3.0",
  "windows_x86_64": {
    "url": "https://rc.guoziweb.com/downloads/ohmydesk-client-windows-x86_64.exe",
    "sha256": "<解压后 exe 的 64 位十六进制 SHA-256>",
    "size": 10420736
  },
  "notes": "本次更新说明（可选，UI/日志展示）",
  "sig": null
}
```

`sig` 为二期离线签名预留，v1 恒为 `null`，解析时忽略。

---

## 4. 模块设计

新增 `src/client/src/update.rs`，在 `main.rs` 起 runtime 处（约 `main.rs:109`）以**独立 std 线程**挂起 —— 因 ureq 为同步阻塞 I/O 且更新低频，独立线程比塞进 tokio/spawn_blocking 更简单，且不触碰 async select 与 X11 worker。

### 公开接口

```rust
/// 启动更新守护线程。infrequent、阻塞 I/O，独占一条 std 线程。
/// - server_url: 用于推导更新基址 host
/// - ctrl_session: 被控会话共享态，空闲才替换
pub fn spawn_update_daemon(server_url: String, ctrl_session: SharedSession);
```

### 内部单元（各自可独立单测）

| 单元 | 职责 | 依赖 | 可测性 |
|---|---|---|---|
| `manifest_url(server_url) -> String` | host 推导 + 拼 `/downloads/latest.json` | 纯函数 | 单测 |
| `Manifest`（serde 结构）+ `parse(&str)` | 清单反序列化 | serde_json | 单测（含缺字段/多余字段） |
| `is_newer(latest: &str, current: &str) -> bool` | semver 比对 | semver | 单测（>/=/< 三态 + 非法版本） |
| `verify_sha256(bytes, expect) -> bool` | 完整性校验 | sha2 | 单测（正/误哈希） |
| `fetch_manifest` / `download_exe` | HTTPS GET（gzip） | ureq | 手测（真实网络） |
| `apply_update(tmp_path)` | self_replace + relaunch | self_replace | 手测（Windows） |
| 守护循环 | 编排 + 周期 + 空闲门控 | 上述 | 手测 |

### 平台分流

- **Windows**：完整 ①~⑤。
- **非 Windows**（`#[cfg(not(windows))]`）：仅 ①~②，发现新版 → 记日志（"发现新版 vX，请手动更新"），不下载、不替换。编译期裁掉 self_replace 路径。

---

## 5. 安全模型（v1）

**v1 = TLS 传输认证 + SHA-256 完整性校验**，不做离线密钥签名。

- **防什么**：传输篡改/中间人（TLS）、下载损坏/截断（SHA-256）。清单与包都经 TLS 从同一台服务器获取，而该服务器**已是整个远控控制面的信任锚**（WS 中转、鉴权都依赖它）。
- **不防什么（v1 明确接受）**：服务器自身被攻陷后同时替换 exe + 清单哈希。理由：服务器一旦失陷，攻击者已掌控远控中转本身，单独给更新通道加离线签名的 v1 边际收益有限。
- **二期升级路径**：清单 `sig` 字段承载 ed25519/minisign 离线签名，客户端内置公钥校验；格式已预留，平滑升级，不破坏 v1 清单。

---

## 6. 依赖与体积

新增客户端依赖（按平台 TLS 特性分流，复用既有后端，不引第二套 TLS）：

```toml
# 跨平台
sha2 = "0.10"
semver = "1"

# Windows：复用 native-tls（SChannel），与 tokio-tungstenite 一致
[target.'cfg(windows)'.dependencies]
ureq = { version = "2", default-features = false, features = ["native-tls", "gzip"] }

# 非 Windows：复用 rustls，与 tokio-tungstenite 一致
[target.'cfg(not(windows))'.dependencies]
ureq = { version = "2", default-features = false, features = ["tls", "gzip"] }
```

二进制净增约 1MB（pre-gzip）；gzip 下载每次省 ~4.5MB，净带宽为正。

---

## 7. 测试策略

- **单测**（CI 可跑）：`manifest_url` 推导、`Manifest::parse`（正常/缺字段/多余字段）、`is_newer`（>/=/< + 非法）、`verify_sha256`（正/误）。
- **手测**（真实环境）：真实 `latest.json` 拉取、gzip 下载 + 解压 + 哈希、Windows self_replace + relaunch、空闲门控（远控中不替换、断开后替换）。
- **不回归**：现有 client/server/protocol 全部既有测试保持绿（本 Spec 不碰服务端/协议，理应零影响）。

---

## 8. 发版流程变更

发版时在既有「放新 exe + `gzip -9 -kf`」之后，新增一步写 `latest.json`：

1. 算新 exe 的 SHA-256：`sha256sum ohmydesk-client-windows-x86_64.exe`。
2. 写 `/www/wwwroot/rc.guoziweb.com/downloads/latest.json`（version + url + sha256 + size），`sudo` 入位、`chown root:root`、`chmod 644`。
3. 校验：`curl -sk https://rc.guoziweb.com/downloads/latest.json` 返回新版本号。

落档到 [[release-publish-process]] 记忆。

---

## 9. Bootstrap

带本更新器的**首版（拟 0.3.0）仍需手动装一次**——因为现网 0.2.1 没有自更新逻辑，无法自我拉起新版。0.3.0 装好后，0.3.x→0.4.x→… 全自动。这是一次性成本，本 Spec 无法绕过（更新器必须先在场才能更新）。

---

## 10. 已知边界（v1 不做，二期候选）

- 新版自身损坏无自动回滚（缓解：SHA-256 拦损坏包 + 由人控制清单发布节奏 + 可手动重推覆盖）。
- 周期固定 6h（可经环境变量覆盖），无服务端可调下发。
- Linux/macOS 自我替换（deb 需 root/dpkg、mac 无打包脚本）。
- 离线密钥签名。
