# Spec C 设计：Windows 客户端在线静默自动更新

> 日期：2026-06-30（v4，含三轮安全/完整性审阅整改）
> 范围：客户端（Windows 自替换 + 全平台 UI 更新提示）+ 发布流水线。服务端 / protocol / admin-web **零改动**。
> 关联记忆：[[five-feature-roadmap]]、[[release-publish-process]]

---

## 0. 安全立场（先于一切）

`latest.json` 对客户端而言**就是一张代码执行授权令**：静默自更新意味着「清单说装什么，客户端就以高权限装什么并重启」。因此信任根**不是** TLS、**不是**「服务器是控制面信任锚」，而是**一把离线 Ed25519 私钥**——只有持有它的人能签发更新。

威胁模型明确防御：拿到 `/www/wwwroot/.../downloads/` 写权限的攻击者（webshell、泄露的 scp/发布凭据、nginx 配错、静态目录遍历写入）——这是独立于 WS 控制面的攻击面。无离线私钥即无法伪造合法 `.minisig`，客户端拒绝任何未签名/签名不符的清单。

---

## 1. 背景与目标

OhMyDesk 客户端正面向客服团队铺开（主力 Windows 单 exe），当前 0.2.1 **无自动更新**，每次发版逐台手动重装。本 Spec 让客户端具备**带离线签名的**在线更新能力。

**目标**：
- **Windows** 单 exe 无人值守静默自更新：检测→下载→验签→校完整性→替换→重启，全自动。
- **Linux/macOS** 检测到新版时 **UI 弹更新提示**（版本 + 说明 + 下载链接），引导手动更新。
- 复用既有 nginx `/downloads/` gzip 通道下载，单次省 ~4.5MB 公网带宽。
- 不打断**任何**进行中或发起中的远控会话（主控 / 被控 / 发起中皆然）。
- 具备**灰度 + 暂停 + 签名降级回退**能力，坏包可控、可回退、不必触达每台客服机。

**非目标（v1 不做）**：
- Linux/macOS **自我替换**（deb 需 root/dpkg、mac 无打包脚本），仅做 UI 提示。
- 进程级自动回滚（崩溃自检自动回退），用灰度+暂停+签名降级替代（见 §9）。
- 公钥轮换（v1 单公钥；轮换需发客户端更新，见 §5）。

---

## 2. 架构选型

三方案选 **A（客户端轮询静态清单 + HTTPS 下载）**：带宽最省、改动面最小、零服务端/协议耦合。

| 方案 | 检测/下载 | 线上带宽 | 改动面 |
|---|---|---|---|
| **A 客户端轮询静态清单 + HTTPS 下载** ✅ | GET `latest.json`(+`.minisig`) + GET exe | **5.5MB（复用 gzip_static）** | 仅客户端 + 发布脚本 |
| B 服务端 WS 推「有新版」+ HTTPS 下载 | 混合 | 5.5MB | +服务端 +protocol +容器清单耦合 |
| C 全走 WebSocket 流式推二进制 | WS | ~13MB（base64 膨胀、丢 gzip） | +服务端 +protocol +hub 负载 |

签名（分离 `.minisig`）、跨平台 `assets`、版本化 URL 均为 `/downloads/` 下静态文件，方案 A 仍零服务端改动。

---

## 3. 数据流

```
触发：启动延迟 ~30s + 每 6h（env 可调）+ 每次 WS 重连/重注册成功后
 └─① resolve_base：解析更新基址（§7 同源 HTTPS 策略）；非法/非 TLS → 更新禁用，记日志退出
 └─② GET <base>/latest.json（≤64KB，连接超时 10s）+ GET <base>/latest.json.minisig
       └─③ minisign-verify(内置公钥, latest.json 原始字节, minisig)
             └─ 失败 → 丢弃 + 告警 + 下周期重试（绝不用未验签清单）
       └─④ 解析 Manifest；选当前平台 asset = assets[<platform>]
             ├─ 无该平台 asset → 跳过
             └─ 校验 asset.url 与 <base> 同源 https
       └─⑤ 选定 asset，按 auto 分流（提示不受 rollout 限制——灰度只控自动替换风险，不控知情）：
       └─⑥ • auto==false（Linux/macOS）：enabled && is_newer → 发 ToUi::UpdateNotice（版本/notes/url），结束，不下载不替换
             • auto==true（Windows）：should_update（!enabled→跳过；allow_downgrade→version!=current；
               否则 is_newer 且 current<min_version 强制 或 bucket<rollout_percent）通过 → 继续下载，否则等下周期
       └─⑦ GET asset.url（Accept-Encoding: gzip → 线上 5.5MB）
             流式写临时文件，**边写边增量 SHA-256**；解压累计 > 50MB → 中止（防 gzip 炸弹）；读超时 120s
       └─⑧ 末字节数 == asset.size 且 SHA-256 == asset.sha256 ？
             ├─ 否 → 删临时 + 告警 + 下周期重试（连续失败 N 次 → 发 UpdateNotice 提示手动下载）
             └─ 是 → ⑨ 应用窗口：
                       state.try_enter_updating()（原子：仅 idle 才占用）成功 →
                       self_replace(临时文件)（重命名运行中 exe→放入新，无需独立 updater 进程）→
                       spawn 新 exe（继承当前令牌、不 runas）→ slint::quit_event_loop + 本体退出
                       （非 idle / 占用失败 → exit_updating，推迟到下周期）
```

- **检测时机（三触发，运行中持续生效）**：① 进程启动延迟 ~30s ② 每 6h 周期（env 可调）③ **每次 WS 重连/重注册成功后立即一次**。第③条覆盖「发版时机器离线/睡眠、醒来仅短暂在线」的间歇场景，纯客户端、零服务端改动。daemon 用 `recv_timeout(间隔)` 阻塞等待，net 在每次成功 `Register` 后经轻量 nudge 通道（`std::sync::mpsc`，一条 `()`）唤醒其早检；故无人值守长跑机器不会因「从不重启」而永停旧版。
- **gzip 透明解压**：`sha256`/`size` 均针对**解压后最终 exe**。客户端带 `Accept-Encoding: gzip`，nginx `gzip_static` 回 `.exe.gz`，ureq（`gzip` 特性）透明解压。
- **替换后**：新进程重走 `elevate::ensure_elevated()` + 反连重注册（`net::run` 重连循环天然恢复），仅短暂断连。
- **资源上限汇总**：manifest ≤64KB；exe 下载与解压均 ≤50MB（且 asset.size 必须 ≤50MB）；连接超时 10s、读超时 120s；全程流式、不在内存缓存整包。
- **Windows 自替换可行性（无需独立 updater 进程）**：Windows 允许**重命名正在运行的 exe**（文件句柄跟踪文件对象，改目录项不破坏已映射映像），`self_replace` 用「现 exe 改名 → 新 exe 放回原路径」绕开「运行中 exe 不可覆盖」的限制——这正是**不需要**单独 updater 辅助进程的根因。三点工程约束：① self_replace 只换不重启，重启 = 本进程 spawn 新 exe 后退出（新 exe 即新版应用本身，非额外 helper）；② 被改名的旧映像运行中无法删除，残留临时文件留待**下次启动 best-effort 清理**；③ spawn 新 exe **继承当前令牌（不用 runas）**——已提权则新进程仍提权、不二次弹 UAC（无人值守关键），未提权则维持原状，更新不恶化提权态；短暂双实例（旧退出前）以相同 id 上线，旧进程退出即自愈，要求 spawn 后尽快退出。
- **临时文件落盘（仅 Windows 下载）**：新 exe 流式写入**与运行中 exe 同目录**的隐藏临时文件（`<exe_dir>/.ohmydesk-update-<rand>.tmp`）。理由：① self_replace 就位是**同卷**操作（跨卷 rename 在 Windows 失败），同目录天然同卷；② 建为**属主独占**（Windows 继承目录 ACL、Unix 0600），杜绝「校验通过 → self_replace」之间被本地他进程掉包（TOCTOU）；③ 落盘前检查可用磁盘 ≥ asset.size（叠加 50MB 上限双兜底）；④ 残留按 `.ohmydesk-update-*` 前缀于下次启动 best-effort 清理；⑤ exe 目录不可写（如装 Program Files 且未提权）则回退系统临时目录、接受 self_replace 跨卷复制。

### 清单格式 `latest.json`（分离签名 `latest.json.minisig`）

```json
{
  "version": "0.3.0",
  "assets": {
    "windows_x86_64": {
      "url": "https://rc.guoziweb.com/downloads/ohmydesk-client-windows-x86_64-0.3.0.exe",
      "sha256": "<解压后 exe 的 64 位十六进制 SHA-256>",
      "size": 10420736,
      "auto": true
    },
    "linux_x86_64_deb": { "url": "https://rc.guoziweb.com/downloads/ohmydesk-client_0.3.0_amd64.deb", "auto": false },
    "linux_arm64_deb":  { "url": "https://rc.guoziweb.com/downloads/ohmydesk-client_0.3.0_arm64.deb", "auto": false },
    "macos_arm64":      { "url": "https://rc.guoziweb.com/downloads/ohmydesk-client-macos-arm64-0.3.0.tar.gz", "auto": false }
  },
  "enabled": true,
  "rollout_percent": 100,
  "min_version": null,
  "allow_downgrade": false,
  "notes": "本次更新说明（可选）"
}
```

- **`assets` map**：键为平台标识，客户端只取自身平台项。`auto: true`（仅 Windows）走自更新，`sha256`/`size` **必填**；`auto: false`（Linux/macOS）仅做 UI 提示，`sha256`/`size` 可选（手动下载，完整性由用户/浏览器保证）。
- **版本化 URL**：自动更新只用版本化不可变文件名（`*-0.3.0.exe`、`.gz` 同名），避免新清单命中旧 exe 缓存，且天然保留上一版供回退。下载页另用稳定别名（见 §8）。
- 签名是**分离文件** `latest.json.minisig`，覆盖 `latest.json` 精确字节（标准 minisign，无需 JSON 规范化）。
- `enabled`：总开关，false 即全网暂停更新（秒级生效）。
- `rollout_percent`（0–100）：客户端按 `bucket = sha256(endpoint_id) 前若干字节 % 100` 确定性分桶，`bucket < rollout_percent` 才更新；同机分桶恒定，便于「10%→观察→100%」放量。
- `min_version`（可选 semver）：当前版本低于它则**无视灰度强制更新**（安全补丁强推）；缺省不强制。
- `allow_downgrade`（默认 false）：true 时只要 `version != current` 即更新（含降级）且**无视灰度全网生效**，专用于紧急回退/钉版（见 §9）。常态恒 false。

---

## 4. 模块设计

新增 `src/client/src/update.rs`，在 `main.rs` 起 runtime 处（约 `main.rs:109`）以**独立 std 线程**挂起（ureq 同步阻塞、更新低频，独立线程最简，不触碰 async select 与 X11 worker）。

### 模块边界：`ClientActivityState`（依赖反转）

为避免「远控发起流程依赖更新模块的全局状态」，活动状态抽成独立小边界，由 `main` 持有 `Arc` 并注入 update / net / ui。**update 只读活动、占用替换窗口、发提示，不拥有远控生命周期语义**。

```rust
/// 客户端活动状态：唯一真相源。复用既有 cur_session/ctrl_session（Arc 不重复造），加两个原子态。
pub struct ClientActivityState {
    cur_session: SharedSession,   // 既有：主控活动会话
    ctrl_session: SharedSession,  // 既有：被控活动会话
    pending_connect: AtomicBool,  // 主控发起中（RemoteAck 前窗口）
    updating: AtomicBool,         // 替换窗口
}
impl ClientActivityState {
    pub fn is_idle(&self) -> bool;             // cur 空 && ctrl 空 && !pending_connect
    pub fn is_updating(&self) -> bool;         // net/ui 准入/发起侧查询
    pub fn begin_pending_connect(&self);       // ui 发起回调
    pub fn end_pending_connect(&self);         // 收 RemoteAck/Reject/超时
    pub fn try_enter_updating(&self) -> bool;  // 原子：is_idle 则置 updating 返回 true，否则 false
    pub fn exit_updating(&self);
}
```

写入方（net/ui）：
- **被控准入侧**：即将建立被控会话时，`is_updating()` 为真则拒绝（回执「正在更新，请稍后」）。
- **主控发起侧**：UI 发起远控回调发送 ConnectRequest 前，`is_updating()` 为真则拒绝发起；否则 `begin_pending_connect()`，并在收 RemoteAck/Reject 或发起超时时 `end_pending_connect()`。

读取方（update 守护）：应用窗口仅在 `try_enter_updating()` 成功（idle 且原子占用）时替换，杜绝「发起中（cur_session 尚未写入）窗口被替换」竞态。

### UI 提示通道：`ToUi::UpdateNotice`

新增 `ToUi::UpdateNotice { version: String, notes: Option<String>, url: String }`（客户端内部枚举，**非协议消息**，不碰 server/protocol）。`ui_glue::consume_to_ui` 收到后置 Slint 属性，UI 显示非侵入横幅「发现新版 vX.Y.Z — 复制下载链接」。**动作用既有 `arboard` 把 URL 复制到剪贴板**（零新依赖、全平台一致；一键打开浏览器需 `open` crate，留二期）。触发场景：① 非 Windows 发现新版；② Windows 自动更新连续失败兜底。

> 非 Windows 检测链（①~⑥）平台无关：ureq 在非 Windows 用 rustls 特性（既有），故 Linux/macOS 能正常拉清单、验签、选 `linux_*`/`macos_*` asset 并弹提示——**前提是清单必须含其平台 asset 项**（见 §8）；缺项则 ④「无该平台 asset」直接跳过、收不到提示。

> **横幅可见性**：被控端常最小化/隐藏到托盘，纯窗口内横幅会看不见。故 UpdateNotice 触发时 **best-effort 将窗口置前**（Slint `window().show()`/raise），且横幅**持久不自动消失**（直到用户复制链接，或下次更新检测覆盖），确保用户下次查看时仍在。OS 级 toast / 托盘通知留二期（避免引托盘依赖）。

### 公开接口

```rust
pub fn spawn_update_daemon(
    server_url: String,                 // 推导更新基址
    endpoint_id: String,                // 灰度确定性分桶（main 传 info.id.clone()）
    state: Arc<ClientActivityState>,    // 只读活动 + 占用替换窗口
    to_ui: tokio::sync::mpsc::UnboundedSender<ToUi>,  // 发 UpdateNotice
    reconnect_rx: std::sync::mpsc::Receiver<()>,      // net 每次成功 Register 后 nudge，唤醒早检；recv_timeout(间隔) 兜底周期
);
```

### 内部单元（可独立单测者标注）

| 单元 | 职责 | 依赖 | 可测 |
|---|---|---|---|
| `resolve_base(server_url, env_override) -> Option<Url>` | 同源 HTTPS 基址推导（§7） | url | ✅ 纯函数 |
| `Manifest` + `parse(&[u8])` | 清单反序列化（assets map、容缺/多余字段、超 64KB 拒绝） | serde_json | ✅ |
| `verify_manifest_sig(pubkey, json, sig)` | Ed25519 验签 | minisign-verify | ✅（正/错/篡改） |
| `current_asset(&Manifest) -> Option<&Asset>` | 按编译平台选 asset | — | ✅ |
| `same_origin(base, url) -> bool` | 下载 URL 同源 https 校验 | url | ✅ |
| `is_newer` / `should_update` | 版本比对 + enabled/allow_downgrade/强制/分桶 | semver, sha2 | ✅ |
| `bucket(endpoint_id) -> u8` | 确定性分桶 0–99 | sha2 | ✅（同 id 恒定） |
| `ClientActivityState` | idle / updating / pending 原子语义 | — | ✅（并发占用、竞态） |
| `download_verified(url, sha256, size)` | 流式下载+增量哈希+上限 | ureq, sha2, tempfile | 手测 |
| `apply(tmp, &state)` | try_enter_updating + self_replace + relaunch | self_replace | 手测（Windows） |

### 平台分流

- **Windows**：完整 ①~⑨。
- **非 Windows**（`#[cfg(not(windows))]`）：①~⑥ 检测 + `UpdateNotice` UI 提示，不下载、不替换；编译期裁掉 self_replace 路径。

---

## 5. 安全模型（v1 即完整）

| 防御 | 机制 | 防御对象 |
|---|---|---|
| **真实性** | 内置 Ed25519 公钥 verify `latest.json.minisig` | 静态目录被写/发布凭据泄露/中间人伪造清单 |
| **完整性** | 签名清单内 `sha256` + `size`，下载流式校验 | 包损坏/截断/被换 |
| **同源** | 下载 URL 必须与更新基址同源 https | 清单被改指向恶意主机 |
| **传输** | 全程 TLS（wss/https 派生） | 窃听/篡改 |
| **资源** | 大小/超时/流式/防炸弹上限 | DoS、内存/磁盘耗尽 |
| **爆炸半径** | enabled 暂停 + rollout 灰度 + 签名 allow_downgrade 回退 | 坏包扩散、回退窗口 |

**信任根管理**：
- 一次性 `minisign -G` 生成密钥对；**私钥仅留本机/离线、绝不进仓库、绝不进 CI**；公钥（base64）作 `const` 编入 `update.rs`。
- 发版时本机 `minisign -Sm latest.json` 产 `.minisig`（见 §8）。
- 公钥轮换需发一次客户端更新（鸡生蛋），v1 接受单公钥约束；二期可内置「当前+下一把」双公钥平滑轮换。

---

## 6. 依赖与体积

```toml
# 跨平台
sha2 = "0.10"
semver = "1"
minisign-verify = "0.2"   # 仅验签，纯 Rust，体积小；离线签名用 minisign CLI
self-replace = "1.5"      # crates.io 包名带连字符，代码内 `use self_replace`；Windows 运行中 exe 自替换
tempfile = "3"            # 安全临时文件落盘
url = "2"                 # 基址/同源解析

# Windows：复用 native-tls（SChannel），与 tokio-tungstenite 一致
[target.'cfg(windows)'.dependencies]
ureq = { version = "2.12", default-features = false, features = ["native-tls", "gzip"] }

# 非 Windows：复用 rustls，与 tokio-tungstenite 一致
[target.'cfg(not(windows))'.dependencies]
ureq = { version = "2.12", default-features = false, features = ["tls", "gzip"] }
```

- **锁 ureq 2.x**：v3 已改 API 与 TLS 模型，本设计按 v2 阻塞 API + `native-tls`/`tls`/`gzip` 特性；锁 `2.12` 防实现期漂移。
- `UpdateNotice` 复用既有 `ToUi` + Slint + `arboard`（复制链接），**无新依赖**；`assets` map 为 serde 结构，**无新依赖**。
- **Windows 交叉编译验证项（windows-gnu）**：`self-replace`、`minisign-verify`、`ureq+native-tls` 须在 `x86_64-pc-windows-gnu` 交叉编译通过，产物仍单 exe 无 mingw DLL 依赖（`build-windows.sh` 第 4 步校验）。二进制净增预估 ~1–1.5MB（pre-gzip），被 gzip 下载收益抵消有余。

---

## 7. 同源 HTTPS 更新基址策略

更新基址**必须 https**，推导规则：

1. `OHMYDESK_UPDATE_BASE_URL`（显式覆盖）存在：必须 `https://`，否则更新禁用。供内网自签 / 专用 HTTPS 更新源时显式指定（仍强制 TLS，绝不允许无 TLS）。
2. 否则从 `OHMYDESK_SERVER` 推导：`wss://host[/...]` → `https://host/downloads/`。
3. `OHMYDESK_SERVER` 为 `ws://`（明文）：**禁止**自动降级到 `http://`；更新禁用，记日志「非 TLS 服务器，未设 OHMYDESK_UPDATE_BASE_URL，更新已禁用」。
4. manifest 内 asset `url` 必须与最终基址**同源**（同 scheme+host+port）且 https，否则拒绝该清单。

生产（`wss://rc.guoziweb.com/ws`）自然得 `https://rc.guoziweb.com/downloads/`，无需额外配置。

---

## 8. 发布流水线（CI → 本机签名 → 远端验收 → 原子切换 → 下载页）

打通「现有 GitHub Actions 产物 + 离线签名 + 静态发布 + 下载页」完整链路。

**A. CI（`release.yml`，打 tag 触发，基本沿用）**：构建各平台产物（Windows exe、amd64/arm64 deb、macOS tar.gz）上传 GitHub Release。CI **不签名、不接触私钥**。

**B. 本机发布脚本 `packaging/download/publish-update.sh`（私钥在本机运行；覆盖全平台清单，Windows 是唯一 auto）**：

1. 取本版各平台产物（`gh release download v0.3.0 -R ...` 或本地构建）。
2. **版本化命名**：`ohmydesk-client-windows-x86_64-0.3.0.exe`、`ohmydesk-client-macos-arm64-0.3.0.tar.gz`（deb 本带版本）。
3. Windows exe：算 `sha256` + `size`；`gzip -9 -kf` 产同名 `.exe.gz`。
4. 生成 `assets` map → `latest.json`：**必须填全平台 asset URL**（windows_x86_64 含 sha256/size/auto:true；linux_x86_64_deb / linux_arm64_deb / macos_arm64 至少含 url、auto:false）——否则对应平台客户端「无 asset」直接跳过、永远收不到提示。再附 version、enabled/rollout_percent/min_version/allow_downgrade、notes。
5. `minisign -Sm latest.json` 产 `latest.json.minisig`（提示离线私钥口令）。
6. **上传（先产物后清单）**：scp 版本化产物 + `.gz` + `.minisig` 入 `/downloads/`（`sudo` 入位、`chown root:root`、`chmod 644`），先到位。
7. **远端验收**（切清单前强制）：`curl -k -H "Accept-Encoding: gzip"` 拉远端版本化 exe，本地解压比对 `sha256`/`size` 一致；`minisign -Vm` 用公钥验证远端 `latest.json.tmp` 的 `.minisig`。任一不符即中止、不切换。
8. **原子切换**：远端 `mv latest.json.tmp latest.json`（最后一步）。保证客户端永不读到「指向尚未上线/半传 exe」的新清单。
9. **下载页**：更新稳定别名 `ohmydesk-client-windows-x86_64.exe`（copy 自本版，供人工下载页）；按 [[release-publish-process]] 用 `sudo sed -i` 改 `download.html` 特定行（版本号/文件名/size，**禁整文件覆盖**——生产页已与仓库分叉）。
10. **保留上一版** exe（不删），回退前提。

**C. 校验**：`curl -sk https://rc.guoziweb.com/downloads/latest.json` 返回新版本 + 验签通过；`/download` 页显示新版本。

私钥与 minisign 操作单独记一条参考记忆（不含私钥本体）。

---

## 9. 爆炸半径控制与回退（替代进程级自动回滚）

v1 用**发布侧三道闸**，回退闭环、无需现场重装：

- **灰度**：新版先 `rollout_percent: 10` 发布 → 观察少量客服机 → 逐步 50→100。坏包最多波及当前百分比。
- **暂停**：异常立即 `enabled: false` 重签发布 → 全网秒级停更（阻止未中招机器继续升级）。
- **回退（自动）**：用**旧版本号 + 旧 sha256 + `allow_downgrade: true`** 重签 `latest.json`；已升坏版机器下周期检测 `version != current` 即自动降级回旧版（保留的上一版 exe 仍在 `/downloads/`）。回退完成后再发一版复位 `allow_downgrade: false`。

> `allow_downgrade` 是 v1 对「无进程级自动回滚」的等价替代：常态只升不降（`is_newer`），紧急时签名授权全网降级，回退不必触达每台客服机。

---

## 10. 测试策略

- **单测（CI 可跑，纯逻辑）**：`resolve_base`（wss 派生 / ws 拒绝 / 显式 https 覆盖 / 非 https 拒绝）、`Manifest::parse`（assets map / 缺 min_version / 多余字段 / 超 64KB 拒绝）、`verify_manifest_sig`（合法/错签/篡改）、`current_asset`（命中/缺平台）、`same_origin`（同源/异源/异端口）、`is_newer`、`should_update`（enabled=false / allow_downgrade 降级 / 强制 min_version / 分桶边界 0·99·阈值）、`bucket`（同 id 恒定、分布合理）、sha256+size 双校验、`ClientActivityState`（idle 判定、try_enter_updating 并发占用、pending_connect 竞态）。
- **手测（真实环境）**：拉 `latest.json`+`.minisig`、gzip 下载解压、50MB 上限与超时、Windows self_replace + relaunch（含残留旧映像下次启动清理、令牌继承不二次弹 UAC、双实例自愈）、双会话+发起中门控（主控中/被控中/发起中不替换、皆空闲替换）、UpdateNotice UI 展示 + arboard 复制链接 + 横幅触发置前且持久（非 Windows + Win 失败兜底）、临时文件同目录落盘 + 属主权限 + 磁盘空间检查、发布脚本远端验收 + 原子切换。
- **不回归**：client/server/protocol 既有测试保持绿（本 Spec 不碰服务端/协议）。

---

## 11. Bootstrap

带本更新器的**首版（拟 0.3.0）仍需手动装一次**（现网 0.2.1 无自更新逻辑，无法自我拉起）。0.3.0 装好后 Windows 全自动、其它平台得 UI 提示。一次性成本，无法绕过。发布 0.3.0 时同步：生成密钥对、公钥编入客户端、首跑 `publish-update.sh` 产出带签清单 + 全平台版本化产物。

---

## 12. 已知边界（v1 不做，二期候选）

- Linux/macOS **自我替换**（v1 仅 UI 提示手动下载）。
- **进程级**自动回滚（崩溃自检自主回退）；v1 回退需运维签发 `allow_downgrade` 清单（非进程自主，但已闭环无需现场重装）。
- 公钥轮换（v1 单公钥；二期内置双公钥）。
- 服务端可调下发周期 / 服务端驱动灰度（v1 周期固定 6h、灰度靠静态 manifest 字段）。
