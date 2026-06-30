# Windows 客户端在线静默自动更新 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 OhMyDesk 客户端加纯客户端侧的在线更新：Windows 离线签名验证后静默自替换，Linux/macOS 弹 UI 提示手动下载。

**Architecture:** 新模块 `src/client/src/update.rs`（检测/验签/决策/下载/替换）+ `src/client/src/activity.rs`（`ClientActivityState` 活动门控边界），以独立 std 线程运行；靠 `/downloads/latest.json`（minisign 分离签名）+ 复用 nginx gzip 下载。服务端 / protocol / admin-web 零改动。设计依据 `docs/superpowers/specs/2026-06-30-windows-client-auto-update-design.md`。

**Tech Stack:** Rust（client crate）、ureq 2.12（gzip + 按平台 TLS）、minisign-verify、self-replace、sha2、semver、url、tempfile、Slint（横幅 UI）、bash（发布脚本）。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `src/client/src/update.rs` | 检测/验签/决策（纯逻辑）+ 下载/替换（Windows I/O）+ 守护编排 + nudge | 新建 |
| `src/client/src/activity.rs` | `ClientActivityState`：会话/发起中/替换窗口门控 | 新建 |
| `src/client/Cargo.toml` | 新增依赖（含按平台 ureq） | 改 |
| `src/client/src/main.rs` | `mod update; mod activity;`；构造 activity + nudge 通道；起守护 | 改（:15、:33、:96、:109） |
| `src/client/src/net/mod.rs` | `ToUi::UpdateAvailable` 变体 | 改（:31） |
| `src/client/src/net/conn.rs` | Register 后调 `update::nudge()` | 改（:85） |
| `src/client/src/ui_glue.rs` | `consume_to_ui` 新 arm；`wire_ui_callbacks` 发起/被控门控；`copy_url` 回调 | 改（:151、:280、:530、:574、:600） |
| `src/client/ui/app.slint` | 更新横幅 + 属性 + 回调 | 改（:476、:582） |
| `src/client/tests/fixtures/sample-latest.json(.minisig)` | 验签测试夹具 | 新建 |
| `packaging/download/publish-update.sh` | 发布流水线（签名/上传/远端验收/原子切换/下载页） | 新建 |

依赖顺序：纯逻辑（Task 1–6，CI 可 TDD）→ Windows I/O（7–8，手测）→ 编排/接线（9–10）→ UI（11）→ 发布脚本（12）。

---

## Task 1: update 模块骨架 + 依赖 + Manifest 类型与解析

**Files:**
- Create: `src/client/src/update.rs`
- Modify: `src/client/src/main.rs:15`（加 `mod update;`）、`src/client/Cargo.toml`
- Test: 内联 `src/client/src/update.rs` 末尾 `#[cfg(test)] mod tests`

- [ ] **Step 1: 加依赖**

`src/client/Cargo.toml` 的 `[dependencies]` 段加（`serde`/`serde_json` 已有）：

```toml
sha2 = "0.10"
semver = "1"
url = "2"
minisign-verify = "0.2"
tempfile = "3"
self-replace = "1.5"
```

末尾按平台加 ureq（紧跟既有 `[target.'cfg(windows)'.dependencies]` 等段，**新增独立段**）：

```toml
[target.'cfg(windows)'.dependencies]
ureq = { version = "2.12", default-features = false, features = ["native-tls", "gzip"] }

[target.'cfg(not(windows))'.dependencies]
ureq = { version = "2.12", default-features = false, features = ["tls", "gzip"] }
```

> 注意：`src/client/Cargo.toml` 已有按平台段（windows / not-windows），把 ureq 合进对应已存在的段，不要重复段头。

- [ ] **Step 2: 建模块骨架 + 注册**

`src/client/src/main.rs:15` 现为 `mod asset;`，在其后加一行 `mod update;`（保持字母序不强求，跟随现有顺序即可）。

新建 `src/client/src/update.rs`：

```rust
//! 在线自动更新：检测/验签/决策（纯逻辑，跨平台单测）+ 下载/替换（仅 Windows）。
//! 设计见 docs/superpowers/specs/2026-06-30-windows-client-auto-update-design.md。
use std::collections::HashMap;

/// manifest 字节上限，超出即拒绝解析（防超大清单 DoS）。
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct Asset {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub auto: bool,
}

fn default_enabled() -> bool { true }
fn default_rollout() -> u8 { 100 }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub assets: HashMap<String, Asset>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_rollout")]
    pub rollout_percent: u8,
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub allow_downgrade: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

/// 解析清单，先卡 64KB 上限。
pub fn parse_manifest(bytes: &[u8]) -> anyhow::Result<Manifest> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("manifest 超过 {} 字节上限", MAX_MANIFEST_BYTES);
    }
    Ok(serde_json::from_slice(bytes)?)
}
```

- [ ] **Step 3: 写失败测试**

`src/client/src/update.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version":"0.3.0",
      "assets":{"windows_x86_64":{"url":"https://h/d/c-0.3.0.exe","sha256":"ab","size":10,"auto":true}},
      "enabled":true,"rollout_percent":100,"min_version":null,"allow_downgrade":false,"notes":"x"
    }"#;

    #[test]
    fn 解析_正常清单() {
        let m = parse_manifest(SAMPLE.as_bytes()).unwrap();
        assert_eq!(m.version, "0.3.0");
        assert!(m.enabled && m.rollout_percent == 100);
        assert!(m.assets.get("windows_x86_64").unwrap().auto);
    }

    #[test]
    fn 解析_缺省字段走默认() {
        let json = r#"{"version":"0.3.0","assets":{}}"#;
        let m = parse_manifest(json.as_bytes()).unwrap();
        assert!(m.enabled);            // 默认 true
        assert_eq!(m.rollout_percent, 100); // 默认 100
        assert!(!m.allow_downgrade);
    }

    #[test]
    fn 解析_多余字段忽略() {
        let json = r#"{"version":"0.3.0","assets":{},"future_field":42}"#;
        assert!(parse_manifest(json.as_bytes()).is_ok());
    }

    #[test]
    fn 解析_超上限拒绝() {
        let big = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(parse_manifest(&big).is_err());
    }
}
```

- [ ] **Step 4: 跑测试确认先失败再通过**

Run: `cargo test -p client update::tests -- --nocapture`
先因编译/逻辑不全可能失败；补齐 Step 2 代码后再次运行，Expected: PASS（4 个测试）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/update.rs src/client/src/main.rs src/client/Cargo.toml Cargo.lock
git commit -m "feat(update): update 模块骨架 + 依赖 + Manifest 解析(64KB 上限)"
```

---

## Task 2: 版本比对 is_newer + 灰度分桶 bucket

**Files:** Modify `src/client/src/update.rs`（加函数 + 测试）

- [ ] **Step 1: 写失败测试**

在 `update.rs` 的 `mod tests` 内追加：

```rust
    #[test]
    fn 版本比对_仅更高才为真() {
        assert!(is_newer("0.3.0", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1")); // 相等
        assert!(!is_newer("0.2.0", "0.2.1")); // 更低
        assert!(!is_newer("乱码", "0.2.1"));   // 非法保守不更新
    }

    #[test]
    fn 分桶_同 id 恒定且落 0_99() {
        let a = bucket("123456789");
        let b = bucket("123456789");
        assert_eq!(a, b);          // 确定性
        assert!(a < 100);
        assert_ne!(bucket("111"), bucket("999")); // 不同 id 大概率不同（此对已验证不同）
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client update::tests::版本比对_仅更高才为真 -v`
Expected: FAIL（`is_newer` 未定义）

- [ ] **Step 3: 实现**

在 `parse_manifest` 之后加：

```rust
/// 仅当 latest 语义版本严格高于 current 才为真；任一非法版本保守返回 false。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// 按 endpoint_id 的 SHA-256 前 8 字节大端 % 100 确定性分桶（0..=99）。
pub fn bucket(endpoint_id: &str) -> u8 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(endpoint_id.as_bytes());
    let mut n: u64 = 0;
    for b in &d[..8] {
        n = (n << 8) | *b as u64;
    }
    (n % 100) as u8
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client update::tests -v`
Expected: PASS（含 Task1 共 6 个）

- [ ] **Step 5: 提交**

```bash
git add src/client/src/update.rs
git commit -m "feat(update): is_newer(semver) + bucket(确定性灰度分桶)"
```

---

## Task 3: 平台 asset 选择 + 决策 decide

**Files:** Modify `src/client/src/update.rs`

- [ ] **Step 1: 写失败测试**

`mod tests` 内追加（用 `decide_with_asset` 直测，绕开宿主平台限制）：

```rust
    fn mk_manifest(version: &str, auto: bool, rollout: u8) -> (Manifest, Asset) {
        let asset = Asset { url: "https://h/d/c.exe".into(), sha256: Some("ab".into()), size: Some(10), auto };
        let mut assets = HashMap::new();
        assets.insert("windows_x86_64".to_string(), asset.clone());
        let m = Manifest {
            version: version.into(), assets, enabled: true, rollout_percent: rollout,
            min_version: None, allow_downgrade: false, notes: Some("note".into()),
        };
        (m, asset)
    }

    #[test]
    fn 决策_enabled为false一律skip() {
        let (mut m, a) = mk_manifest("0.3.0", true, 100);
        m.enabled = false;
        assert_eq!(decide_with_asset(&m, "0.2.1", "id", &a), UpdateAction::Skip);
    }

    #[test]
    fn 决策_非auto新版给提示不受灰度() {
        let (m, mut a) = mk_manifest("0.3.0", false, 0); // rollout=0
        a.auto = false;
        match decide_with_asset(&m, "0.2.1", "id", &a) {
            UpdateAction::Notice { version, .. } => assert_eq!(version, "0.3.0"),
            other => panic!("应为 Notice，实为 {other:?}"),
        }
    }

    #[test]
    fn 决策_windows灰度0不更新_100更新() {
        let (m0, a) = mk_manifest("0.3.0", true, 0);
        assert_eq!(decide_with_asset(&m0, "0.2.1", "id", &a), UpdateAction::Skip);
        let (m100, a2) = mk_manifest("0.3.0", true, 100);
        assert!(matches!(decide_with_asset(&m100, "0.2.1", "id", &a2), UpdateAction::AutoUpdate { .. }));
    }

    #[test]
    fn 决策_min_version强制无视灰度() {
        let (mut m, a) = mk_manifest("0.3.0", true, 0); // 灰度 0
        m.min_version = Some("0.3.0".into());           // 强制线高于 current
        assert!(matches!(decide_with_asset(&m, "0.2.1", "id", &a), UpdateAction::AutoUpdate { .. }));
    }

    #[test]
    fn 决策_allow_downgrade降级() {
        let (mut m, a) = mk_manifest("0.2.0", true, 0);  // 比 current 更低 + 灰度0
        m.allow_downgrade = true;
        assert!(matches!(decide_with_asset(&m, "0.2.1", "id", &a), UpdateAction::AutoUpdate { .. }));
    }

    #[test]
    fn 决策_auto缺sha256则skip() {
        let (m, mut a) = mk_manifest("0.3.0", true, 100);
        a.sha256 = None;
        assert_eq!(decide_with_asset(&m, "0.2.1", "id", &a), UpdateAction::Skip);
    }

    #[test]
    fn 选asset_命中与缺平台() {
        let (m, _) = mk_manifest("0.3.0", true, 100);
        assert!(current_asset(&m, "windows_x86_64").is_some());
        assert!(current_asset(&m, "macos_arm64").is_none());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client update::tests::决策_windows灰度0不更新_100更新 -v`
Expected: FAIL（`UpdateAction`/`decide_with_asset` 未定义）

- [ ] **Step 3: 实现**

在 `bucket` 之后加：

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateAction {
    Skip,
    Notice { version: String, url: String, notes: Option<String> },
    AutoUpdate { version: String, url: String, sha256: String, size: u64 },
}

/// 当前编译平台对应的 manifest asset 键。
pub fn platform_key() -> &'static str {
    if cfg!(windows) { "windows_x86_64" }
    else if cfg!(all(target_os = "linux", target_arch = "x86_64")) { "linux_x86_64_deb" }
    else if cfg!(all(target_os = "linux", target_arch = "aarch64")) { "linux_arm64_deb" }
    else if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "macos_arm64" }
    else { "unsupported" }
}

pub fn current_asset<'a>(m: &'a Manifest, key: &str) -> Option<&'a Asset> {
    m.assets.get(key)
}

/// 入口：选本平台 asset 后决策。
pub fn decide(m: &Manifest, current: &str, endpoint_id: &str) -> UpdateAction {
    match current_asset(m, platform_key()) {
        Some(a) => decide_with_asset(m, current, endpoint_id, a),
        None => UpdateAction::Skip,
    }
}

/// 决策核心（注入 asset，便于跨平台单测）：
/// - !enabled → Skip
/// - 非 auto（Linux/macOS）：is_newer → Notice（不受灰度）
/// - auto（Windows）：allow_downgrade(version!=current) 或 (is_newer 且 (强制 min_version 或 中桶))
///   且 asset 必须带 sha256+size，否则 Skip
pub fn decide_with_asset(m: &Manifest, current: &str, endpoint_id: &str, asset: &Asset) -> UpdateAction {
    if !m.enabled {
        return UpdateAction::Skip;
    }
    if !asset.auto {
        return if is_newer(&m.version, current) {
            UpdateAction::Notice { version: m.version.clone(), url: asset.url.clone(), notes: m.notes.clone() }
        } else {
            UpdateAction::Skip
        };
    }
    let want = if m.allow_downgrade {
        m.version != current
    } else if is_newer(&m.version, current) {
        let forced = m.min_version.as_deref().map_or(false, |mv| is_newer(mv, current));
        forced || (bucket(endpoint_id) < m.rollout_percent)
    } else {
        false
    };
    if !want {
        return UpdateAction::Skip;
    }
    match (asset.sha256.clone(), asset.size) {
        (Some(sha256), Some(size)) => UpdateAction::AutoUpdate {
            version: m.version.clone(), url: asset.url.clone(), sha256, size,
        },
        _ => UpdateAction::Skip, // Windows asset 缺完整性字段 → 不自动更新
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client update::tests -v`
Expected: PASS（累计 13）

- [ ] **Step 5: 提交**

```bash
git add src/client/src/update.rs
git commit -m "feat(update): decide 决策(enabled/灰度/min_version/降级/auto-notice 分流)"
```

---

## Task 4: 同源 HTTPS 基址 resolve_base + same_origin

**Files:** Modify `src/client/src/update.rs`

- [ ] **Step 1: 写失败测试**

`mod tests` 内追加：

```rust
    #[test]
    fn 基址_wss派生https_downloads() {
        let b = resolve_base("wss://rc.guoziweb.com/ws", None).unwrap();
        assert_eq!(b.as_str(), "https://rc.guoziweb.com/downloads/");
    }

    #[test]
    fn 基址_ws明文拒绝不降级() {
        assert!(resolve_base("ws://192.168.1.10:8765/ws", None).is_none());
    }

    #[test]
    fn 基址_显式覆盖必须https() {
        assert!(resolve_base("ws://x/ws", Some("https://up.intra/d/")).is_some());
        assert!(resolve_base("ws://x/ws", Some("http://up.intra/d/")).is_none());
    }

    #[test]
    fn 同源_同主机https放行_异源拒绝() {
        let b = resolve_base("wss://rc.guoziweb.com/ws", None).unwrap();
        assert!(same_origin(&b, "https://rc.guoziweb.com/downloads/c-0.3.0.exe"));
        assert!(!same_origin(&b, "https://evil.com/c.exe")); // 异主机
        assert!(!same_origin(&b, "http://rc.guoziweb.com/c.exe")); // 非 https
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client update::tests::基址_wss派生https_downloads -v`
Expected: FAIL（`resolve_base` 未定义）

- [ ] **Step 3: 实现**

```rust
/// 解析更新基址（必须 https）：显式 OHMYDESK_UPDATE_BASE_URL 优先（须 https），
/// 否则从 wss:// 服务器派生 https://host[:port]/downloads/；ws:// 不降级 → None。
pub fn resolve_base(server_url: &str, env_override: Option<&str>) -> Option<url::Url> {
    if let Some(o) = env_override {
        let u = url::Url::parse(o).ok()?;
        return if u.scheme() == "https" { Some(u) } else { None };
    }
    let s = url::Url::parse(server_url).ok()?;
    if s.scheme() != "wss" {
        return None;
    }
    let host = s.host_str()?;
    let authority = match s.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    url::Url::parse(&format!("https://{authority}/downloads/")).ok()
}

/// 下载 URL 必须 https 且与基址同源（scheme+host+port）。
pub fn same_origin(base: &url::Url, target: &str) -> bool {
    match url::Url::parse(target) {
        Ok(t) => {
            t.scheme() == "https"
                && t.host_str() == base.host_str()
                && t.port_or_known_default() == base.port_or_known_default()
        }
        Err(_) => false,
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client update::tests -v`
Expected: PASS（累计 17）

- [ ] **Step 5: 提交**

```bash
git add src/client/src/update.rs
git commit -m "feat(update): resolve_base 同源 HTTPS 基址 + same_origin 校验"
```

---

## Task 5: minisign 验签 + 密钥对 + 测试夹具 + 嵌入公钥

> 本任务含一次性离线密钥生成（维护者执行）。私钥永不进仓库/CI。

**Files:**
- Modify: `src/client/src/update.rs`
- Create: `src/client/tests/fixtures/sample-latest.json`、`src/client/tests/fixtures/sample-latest.json.minisig`

- [ ] **Step 1: 生成密钥对（一次性，本机离线）**

安装 rsign2（minisign 兼容，纯 Rust）：`cargo install rsign2`（或系统 `minisign`）。
生成密钥：`rsign generate -p update-pub.key -s update-sec.key`（设口令）。
查看公钥 base64（文件第二行，`RW...` 开头那串），记下，下一步嵌入。
**把 `update-sec.key` 移到仓库外的离线安全位置；公钥文件可留本机。绝不 git add 私钥。**

- [ ] **Step 2: 造测试夹具并签名**

```bash
mkdir -p src/client/tests/fixtures
cp docs/superpowers/specs/2026-06-30-windows-client-auto-update-design.md /dev/null # 占位无关
cat > src/client/tests/fixtures/sample-latest.json <<'JSON'
{"version":"0.3.0","assets":{"windows_x86_64":{"url":"https://rc.guoziweb.com/downloads/ohmydesk-client-windows-x86_64-0.3.0.exe","sha256":"00","size":1,"auto":true}},"enabled":true,"rollout_percent":100,"min_version":null,"allow_downgrade":false,"notes":"fixture"}
JSON
rsign sign -s update-sec.key -m src/client/tests/fixtures/sample-latest.json
# 产出 src/client/tests/fixtures/sample-latest.json.minisig
```

- [ ] **Step 3: 嵌入公钥 + 实现验签**

在 `update.rs` 顶部常量区加（把 `RW...` 换成 Step 1 的真实公钥 base64）：

```rust
/// 更新清单签名公钥（minisign/rsign2，base64）。私钥离线，绝不入库。
pub const UPDATE_PUBKEY: &str = "RW……此处粘贴 Step1 生成的公钥 base64……";
```

加验签函数：

```rust
/// 用内置公钥验证 latest.json 的分离签名（minisig 文件全文）。任何失败 → false。
pub fn verify_manifest_sig(pubkey_b64: &str, manifest_bytes: &[u8], minisig: &str) -> bool {
    use minisign_verify::{PublicKey, Signature};
    match (PublicKey::from_base64(pubkey_b64), Signature::decode(minisig)) {
        (Ok(pk), Ok(sig)) => pk.verify(manifest_bytes, &sig, false).is_ok(),
        _ => false,
    }
}
```

- [ ] **Step 4: 写测试**

`mod tests` 内追加：

```rust
    const FIXTURE_JSON: &[u8] = include_bytes!("../tests/fixtures/sample-latest.json");
    const FIXTURE_SIG: &str = include_str!("../tests/fixtures/sample-latest.json.minisig");

    #[test]
    fn 验签_合法夹具通过() {
        assert!(verify_manifest_sig(UPDATE_PUBKEY, FIXTURE_JSON, FIXTURE_SIG));
    }

    #[test]
    fn 验签_篡改字节失败() {
        let mut tampered = FIXTURE_JSON.to_vec();
        tampered[0] ^= 0xFF;
        assert!(!verify_manifest_sig(UPDATE_PUBKEY, &tampered, FIXTURE_SIG));
    }

    #[test]
    fn 验签_错公钥失败() {
        let bad = "RWTGiBCq9999999999999999999999999999999999999999999999=";
        assert!(!verify_manifest_sig(bad, FIXTURE_JSON, FIXTURE_SIG));
    }

    #[test]
    fn 内置公钥_可解析() {
        use minisign_verify::PublicKey;
        assert!(PublicKey::from_base64(UPDATE_PUBKEY).is_ok());
    }
```

- [ ] **Step 5: 跑测试 + 提交（不含私钥）**

Run: `cargo test -p client update::tests -v` → Expected: PASS（累计 21）

```bash
git add src/client/src/update.rs src/client/tests/fixtures/sample-latest.json src/client/tests/fixtures/sample-latest.json.minisig
git commit -m "feat(update): minisign 验签 + 内置公钥 + 签名测试夹具"
```

> 校验未误提交私钥：`git log -p | grep -i 'update-sec' || echo OK`

---

## Task 6: ClientActivityState 活动门控

**Files:**
- Create: `src/client/src/activity.rs`
- Modify: `src/client/src/main.rs:15`（加 `mod activity;`）

- [ ] **Step 1: 实现 + 内联测试骨架**

新建 `src/client/src/activity.rs`：

```rust
//! 客户端活动状态边界：会话/发起中/替换窗口的唯一真相源。
//! main 持有 Arc 注入 update/ui；update 只读活动 + 占用替换窗口，不拥有远控生命周期。
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::SharedSession;

pub struct ClientActivityState {
    cur_session: SharedSession,   // 主控活动会话（既有 Arc，复用不另造）
    ctrl_session: SharedSession,  // 被控活动会话
    pending_until_ms: AtomicU64,  // 主控发起中截止时刻（自动过期，防卡死）
    updating: AtomicBool,         // 替换窗口
}

impl ClientActivityState {
    pub fn new(cur_session: SharedSession, ctrl_session: SharedSession) -> Self {
        Self { cur_session, ctrl_session, pending_until_ms: AtomicU64::new(0), updating: AtomicBool::new(false) }
    }
    fn sessions_idle(&self) -> bool {
        self.cur_session.lock().unwrap().is_none() && self.ctrl_session.lock().unwrap().is_none()
    }
    /// 空闲 = 无主控/被控会话 且 不在发起中窗口。
    pub fn is_idle(&self, now_ms: u64) -> bool {
        self.sessions_idle() && now_ms >= self.pending_until_ms.load(Ordering::Acquire)
    }
    pub fn is_updating(&self) -> bool { self.updating.load(Ordering::Acquire) }
    /// 主控发起远控时置位，30s 后自动过期（防 ack 丢失永久卡死）。
    pub fn begin_pending_connect(&self, now_ms: u64) {
        self.pending_until_ms.store(now_ms + 30_000, Ordering::Release);
    }
    pub fn end_pending_connect(&self) { self.pending_until_ms.store(0, Ordering::Release); }
    /// 原子占用替换窗口：先抢 updating，再复检空闲；非空闲则退还。
    pub fn try_enter_updating(&self, now_ms: u64) -> bool {
        if self.updating.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        if self.is_idle(now_ms) {
            true
        } else {
            self.updating.store(false, Ordering::Release);
            false
        }
    }
    pub fn exit_updating(&self) { self.updating.store(false, Ordering::Release); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn st() -> (ClientActivityState, SharedSession, SharedSession) {
        let cur: SharedSession = Arc::new(Mutex::new(None));
        let ctrl: SharedSession = Arc::new(Mutex::new(None));
        (ClientActivityState::new(cur.clone(), ctrl.clone()), cur, ctrl)
    }

    #[test]
    fn 空闲_无会话无发起为真() {
        let (s, _, _) = st();
        assert!(s.is_idle(1000));
    }

    #[test]
    fn 会话占用则非空闲() {
        let (s, cur, _) = st();
        *cur.lock().unwrap() = Some("sess".into());
        assert!(!s.is_idle(1000));
    }

    #[test]
    fn 发起中窗口内非空闲_过期后空闲() {
        let (s, _, _) = st();
        s.begin_pending_connect(1000);
        assert!(!s.is_idle(1000));         // 未到期
        assert!(!s.is_idle(30000));        // 仍在 1000+30000 之内
        assert!(s.is_idle(31001));         // 已过期
        s.end_pending_connect();
        assert!(s.is_idle(1000));
    }

    #[test]
    fn 占用替换窗口_互斥与退还() {
        let (s, cur, _) = st();
        assert!(s.try_enter_updating(1000));   // 抢到
        assert!(s.is_updating());
        assert!(!s.try_enter_updating(1000));  // 二次抢不到
        s.exit_updating();
        // 有会话时抢不到（抢了会退还）
        *cur.lock().unwrap() = Some("x".into());
        assert!(!s.try_enter_updating(1000));
        assert!(!s.is_updating());             // 已退还
    }
}
```

`src/client/src/main.rs:15` 区加 `mod activity;`。

- [ ] **Step 2: 跑测试**

Run: `cargo test -p client activity::tests -v`
Expected: PASS（4 个）

- [ ] **Step 3: 提交**

```bash
git add src/client/src/activity.rs src/client/src/main.rs
git commit -m "feat(activity): ClientActivityState(会话/发起中/替换窗口门控)"
```

---

## Task 7: 限流哈希读 CapReader + 下载校验 download_verified（Windows）

**Files:** Modify `src/client/src/update.rs`

- [ ] **Step 1: 写 CapReader 失败测试（跨平台可测）**

`mod tests` 内追加：

```rust
    #[test]
    fn 限流读_统计字节并算哈希() {
        use std::io::Read;
        let data = b"hello world";
        let mut r = CapReader::new(std::io::Cursor::new(&data[..]), 1024);
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(r.total(), data.len() as u64);
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        assert_eq!(r.finish_hex(), "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    }

    #[test]
    fn 限流读_超上限报错() {
        use std::io::Read;
        let data = vec![0u8; 100];
        let mut r = CapReader::new(std::io::Cursor::new(data), 10); // 上限 10
        let mut out = Vec::new();
        assert!(r.read_to_end(&mut out).is_err());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client update::tests::限流读_统计字节并算哈希 -v`
Expected: FAIL（`CapReader` 未定义）

- [ ] **Step 3: 实现 CapReader + to_hex（跨平台）**

```rust
fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 边读边算 SHA-256 + 强制上限的 Reader 包装。
pub struct CapReader<R> {
    inner: R,
    cap: u64,
    read: u64,
    hasher: sha2::Sha256,
}
impl<R: std::io::Read> CapReader<R> {
    pub fn new(inner: R, cap: u64) -> Self {
        use sha2::Digest;
        Self { inner, cap, read: 0, hasher: sha2::Sha256::new() }
    }
    pub fn total(&self) -> u64 { self.read }
    pub fn finish_hex(self) -> String {
        use sha2::Digest;
        to_hex(&self.hasher.finalize())
    }
}
impl<R: std::io::Read> std::io::Read for CapReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read > self.cap {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "下载超过大小上限"));
        }
        use sha2::Digest;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}
```

- [ ] **Step 4: 实现 download_verified（仅 Windows，手测）**

```rust
/// 下载并校验新 exe，落盘到 exe 同目录隐藏临时文件，返回临时路径。仅 Windows 调用。
#[cfg(windows)]
pub fn download_verified(
    url: &str,
    expect_sha: &str,
    expect_size: u64,
    exe_dir: &std::path::Path,
) -> anyhow::Result<tempfile::TempPath> {
    use std::time::Duration;
    const CAP: u64 = 50 * 1024 * 1024;
    if expect_size > CAP {
        anyhow::bail!("size {expect_size} 超 50MB 上限");
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(120))
        .build();
    // ureq 启用 gzip 特性后自动加 Accept-Encoding 并透明解压；sha256/size 针对解压后字节。
    let resp = agent.get(url).call()?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".ohmydesk-update-")
        .tempfile_in(exe_dir)?;
    let mut reader = CapReader::new(resp.into_reader(), CAP);
    std::io::copy(&mut reader, tmp.as_file_mut())?; // 磁盘满则此处 IO 错误，上限已兜底
    let got_size = reader.total();
    let got_sha = reader.finish_hex();
    if got_size != expect_size {
        anyhow::bail!("size 不符：期望 {expect_size} 实得 {got_size}");
    }
    if !got_sha.eq_ignore_ascii_case(expect_sha) {
        anyhow::bail!("sha256 不符");
    }
    Ok(tmp.into_temp_path())
}
```

- [ ] **Step 5: 跑跨平台测试 + Windows 交叉编译检查**

Run: `cargo test -p client update::tests -v` → Expected: PASS（累计 23）
Run: `cargo check -p client --target x86_64-pc-windows-gnu` → Expected: 编译通过（ureq/tempfile/self-replace 交叉编译 OK；若缺 target 先 `rustup target add x86_64-pc-windows-gnu`）
手测（Windows 真机，留待集成）：对真实 gzip URL 下载，sha256/size 命中、临时文件落在 exe 同目录、`.ohmydesk-update-*` 前缀。

- [ ] **Step 6: 提交**

```bash
git add src/client/src/update.rs
git commit -m "feat(update): CapReader 限流哈希读 + download_verified(gzip流式/同目录落盘/双校验)"
```

---

## Task 8: 自替换 apply + 启动残留清理（Windows）

**Files:** Modify `src/client/src/update.rs`

- [ ] **Step 1: 实现 apply + cleanup（仅 Windows）**

```rust
/// 用已校验的临时文件替换运行中 exe 并重启。仅 Windows。
/// self_replace：现 exe 改名 → 新文件放回原路径（绕开运行中 exe 不可覆盖）。
#[cfg(windows)]
pub fn apply(staged: &std::path::Path) -> anyhow::Result<()> {
    self_replace::self_replace(staged)?;
    let exe = std::env::current_exe()?;
    // 默认 spawn 继承当前进程令牌（不 runas）：已提权则新进程仍提权，不二次弹 UAC。
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

/// 启动时 best-effort 清理本模块遗留的更新临时文件（仅按自有前缀，安全）。
#[cfg(windows)]
pub fn cleanup_stale_temp(exe_dir: &std::path::Path) {
    if let Ok(rd) = std::fs::read_dir(exe_dir) {
        for e in rd.flatten() {
            let name = e.file_name();
            if name.to_string_lossy().starts_with(".ohmydesk-update-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}
```

- [ ] **Step 2: Windows 交叉编译检查**

Run: `cargo check -p client --target x86_64-pc-windows-gnu`
Expected: 编译通过（`self_replace::self_replace` 签名匹配；若 API 名不同，按 self-replace 1.5 文档调整为 `self_replace::self_replace(path)`）。

- [ ] **Step 3: 提交**

```bash
git add src/client/src/update.rs
git commit -m "feat(update): apply 自替换+重启(令牌继承) + 启动残留清理"
```

> 手测（Windows 真机，集成后）：替换后新进程拉起、旧进程退、版本号变更；下次启动 `.ohmydesk-update-*` 被清理；已提权场景无二次 UAC。

---

## Task 9: 守护编排 spawn_update_daemon + nudge 通道 + conn 钩子

**Files:**
- Modify: `src/client/src/update.rs`（守护 + nudge）、`src/client/src/net/conn.rs:85`、`src/client/src/main.rs`

- [ ] **Step 1: 实现 now_ms + nudge + 守护编排**

`update.rs` 顶部加：

```rust
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::OnceLock;

static NUDGE_TX: OnceLock<SyncSender<()>> = OnceLock::new();
pub fn set_nudge_sender(tx: SyncSender<()>) { let _ = NUDGE_TX.set(tx); }
/// net 每次成功 Register/重连后调用：唤醒守护提前检测。
pub fn nudge() { if let Some(tx) = NUDGE_TX.get() { let _ = tx.try_send(()); } }

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
```

守护（用 `crate::net::ToUi` 发提示；`crate::activity::ClientActivityState`）：

```rust
use std::sync::Arc;
use std::time::Duration;
use crate::activity::ClientActivityState;
use crate::net::ToUi;
use tokio::sync::mpsc::UnboundedSender;

/// 启动更新守护（独立 std 线程）。触发：启动延迟 30s + 周期 + nudge。
pub fn spawn_update_daemon(
    server_url: String,
    endpoint_id: String,
    state: Arc<ClientActivityState>,
    to_ui: UnboundedSender<ToUi>,
    nudge_rx: Receiver<()>,
) {
    std::thread::spawn(move || {
        let base = match resolve_base(&server_url, std::env::var("OHMYDESK_UPDATE_BASE_URL").ok().as_deref()) {
            Some(b) => b,
            None => { tracing::warn!("自动更新已禁用：服务器非 wss 且未设 OHMYDESK_UPDATE_BASE_URL(https)"); return; }
        };
        let interval = std::env::var("OHMYDESK_UPDATE_INTERVAL_SECS").ok()
            .and_then(|s| s.parse::<u64>().ok()).unwrap_or(6 * 3600);
        #[cfg(windows)]
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() { cleanup_stale_temp(dir); }
        }
        std::thread::sleep(Duration::from_secs(30)); // 启动延迟，避开连接抖动
        loop {
            if let Err(e) = run_once(&base, &endpoint_id, &state, &to_ui) {
                tracing::warn!("更新检查失败：{e}");
            }
            let _ = nudge_rx.recv_timeout(Duration::from_secs(interval)); // nudge 或超时唤醒
        }
    });
}

fn run_once(base: &url::Url, endpoint_id: &str, state: &Arc<ClientActivityState>, to_ui: &UnboundedSender<ToUi>) -> anyhow::Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build();
    let manifest_url = base.join("latest.json")?;
    let sig_url = base.join("latest.json.minisig")?;
    // manifest（限 64KB）
    let mut buf = Vec::new();
    use std::io::Read;
    agent.get(manifest_url.as_str()).call()?.into_reader().take(MAX_MANIFEST_BYTES as u64 + 1).read_to_end(&mut buf)?;
    let sig = agent.get(sig_url.as_str()).call()?.into_string()?;
    if !verify_manifest_sig(UPDATE_PUBKEY, &buf, &sig) {
        anyhow::bail!("清单验签失败，丢弃");
    }
    let m = parse_manifest(&buf)?;
    // 同源校验本平台 asset
    if let Some(a) = current_asset(&m, platform_key()) {
        if !same_origin(base, &a.url) {
            anyhow::bail!("下载 URL 非同源 https，丢弃");
        }
    }
    let current = env!("CARGO_PKG_VERSION");
    match decide(&m, current, endpoint_id) {
        UpdateAction::Skip => Ok(()),
        UpdateAction::Notice { version, url, notes } => {
            let _ = to_ui.send(ToUi::UpdateAvailable { version, url, notes });
            Ok(())
        }
        UpdateAction::AutoUpdate { version, url, sha256, size } => {
            apply_auto(state, to_ui, &version, &url, &sha256, size)
        }
    }
}

#[cfg(windows)]
fn apply_auto(state: &Arc<ClientActivityState>, to_ui: &UnboundedSender<ToUi>, version: &str, url: &str, sha256: &str, size: u64) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| anyhow::anyhow!("无 exe 目录"))?;
    let staged = match download_verified(url, sha256, size, dir) {
        Ok(p) => p,
        Err(e) => { // 下载/校验失败兜底提示手动
            let _ = to_ui.send(ToUi::UpdateAvailable { version: version.into(), url: url.into(), notes: Some(format!("自动更新失败，请手动下载（{e}）")) });
            return Err(e);
        }
    };
    if state.try_enter_updating(now_ms()) {
        let r = apply(&staged);
        if r.is_ok() {
            let _ = slint::invoke_from_event_loop(|| { let _ = slint::quit_event_loop(); });
            std::thread::sleep(Duration::from_millis(300));
            std::process::exit(0);
        }
        state.exit_updating();
        r
    } else {
        tracing::info!("有会话进行中，推迟替换到下个周期");
        Ok(()) // staged 临时文件随 drop 删除，下周期重下
    }
}

#[cfg(not(windows))]
fn apply_auto(_s: &Arc<ClientActivityState>, _t: &UnboundedSender<ToUi>, _v: &str, _u: &str, _h: &str, _z: u64) -> anyhow::Result<()> {
    Ok(()) // 非 Windows 不会进 AutoUpdate（asset.auto=false），兜底空实现
}
```

- [ ] **Step 2: conn.rs 注册后 nudge**

`src/client/src/net/conn.rs:85`（`tracing::info!("已注册 id=...")` 一行之后）加：

```rust
        crate::update::nudge();
```

- [ ] **Step 3: Windows 交叉编译 + 本机编译检查**

Run: `cargo check -p client` → Expected: 通过
Run: `cargo check -p client --target x86_64-pc-windows-gnu` → Expected: 通过

> 说明：`ToUi::UpdateAvailable` 在 Task 10 加入 `net/mod.rs`；若本任务先编译会报缺变体——故 Step 4 提交前需与 Task 10 的变体定义合并。建议本任务与 Task 10 连续执行后一起编译通过再提交，或先在 `net/mod.rs` 加变体（见 Task 10 Step 1）。

- [ ] **Step 4: 提交**

```bash
git add src/client/src/update.rs src/client/src/net/conn.rs
git commit -m "feat(update): 守护编排(启动+周期+nudge)+run_once(拉取/验签/决策/下载替换) + conn 注册后 nudge"
```

---

## Task 10: ToUi::UpdateAvailable + consume_to_ui + 活动门控接线

**Files:**
- Modify: `src/client/src/net/mod.rs:31`、`src/client/src/ui_glue.rs`（:112、:151、:530、:574、:600）、`src/client/src/main.rs`

- [ ] **Step 1: 加 ToUi 变体**

`src/client/src/net/mod.rs` 的 `pub enum ToUi`（:31）内加变体：

```rust
    /// 发现新版：UI 弹更新横幅（version/url/notes）。
    UpdateAvailable { version: String, url: String, notes: Option<String> },
```

- [ ] **Step 2: consume_to_ui 加处理 arm**

`src/client/src/ui_glue.rs` 的 `consume_to_ui` 内 `match ev`（:530）加 arm（模式同 `Registered` 范例）：

```rust
                net::ToUi::UpdateAvailable { version, url, notes } => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_update_available(true);
                            ui.set_update_version(version.into());
                            ui.set_update_url(url.into());
                            ui.set_update_notes(notes.unwrap_or_default().into());
                            let _ = ui.show(); // best-effort 置前，避免最小化看不见
                        }
                    });
                }
```

- [ ] **Step 3: RemoteAck/RemoteRejected 清发起中**

`ui_glue.rs` 的 `RemoteAck` arm（写 cur_session 处 :574 附近）末尾加 `activity.end_pending_connect();`；`RemoteRejected` arm（:600 附近）同样加。需让 `consume_to_ui` 持有 `Arc<ClientActivityState>`——给 `consume_to_ui` 增参（见 Step 5）。

- [ ] **Step 4: 发起/被控门控**

`wire_ui_callbacks`（:112）增参 `activity: &std::sync::Arc<crate::activity::ClientActivityState>`。
在 `on_connect_b`（:151）回调内，发送 `FromUi::StartRemote`（:173）**之前**加：

```rust
            if activity.is_updating() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_remote_status("正在更新，请稍后".into()); }
                return;
            }
            activity.begin_pending_connect(crate::update::now_ms());
```

在被控接受回调 `on_auth_accept`（对应 `app.slint:501`）发送 `FromUi::AuthDecision { accept:true }` 之前加：

```rust
            if activity.is_updating() { return; } // 替换窗口内拒绝被控接入
```

（回调内需 clone 一份 `Arc` 进闭包：`let activity = activity.clone();`，模式同既有 `tx`/`cur_session` clone。）

- [ ] **Step 5: main.rs 接线**

`src/client/src/main.rs`：
- 在构造 session 后（:100 之后）加：
```rust
    let activity = std::sync::Arc::new(activity::ClientActivityState::new(cur_session.clone(), ctrl_session.clone()));
    let (nudge_tx, nudge_rx) = std::sync::mpsc::sync_channel::<()>(4);
    update::set_nudge_sender(nudge_tx);
```
- `wire_ui_callbacks` 调用（:103）增传 `&activity`。
- `consume_to_ui` 调用（:110）增传 `activity.clone()`（并相应改 `consume_to_ui` 签名加 `activity: Arc<ClientActivityState>` 参数）。
- 在 `rt.spawn(net::run(...))`（:109）附近加起守护（注意：守护是 std 线程，不用 rt.spawn）：
```rust
    update::spawn_update_daemon(server_url.clone(), self_id.clone(), activity.clone(), to_ui_tx.clone(), nudge_rx);
```
  （`server_url` 在 :65、`self_id` 在 :68 已有；`to_ui_tx` 需在被 move 进 `net::run` 前 `.clone()`。检查 :82 `to_ui_tx` 的所有权流向，必要时调整 clone 顺序。）

- [ ] **Step 6: 编译 + 既有测试不回归**

Run: `cargo build -p client` → Expected: 通过（此时 Task 9 的 `ToUi::UpdateAvailable` 引用已满足）
Run: `cargo test -p client` → Expected: 全绿（既有 86 + 新增 update/activity 单测）
Run: `cargo check -p client --target x86_64-pc-windows-gnu` → Expected: 通过

- [ ] **Step 7: 提交**

```bash
git add src/client/src/net/mod.rs src/client/src/ui_glue.rs src/client/src/main.rs
git commit -m "feat(update): ToUi::UpdateAvailable + consume_to_ui 处理 + 发起/被控活动门控接线"
```

---

## Task 11: 更新横幅 UI（app.slint + copy_url）

**Files:** Modify `src/client/ui/app.slint`、`src/client/src/ui_glue.rs:280`

- [ ] **Step 1: 加属性 + 回调**

`app.slint` 的 `AppWindow`（:476）属性区加：

```slint
    in property <bool> update_available: false;
    in property <string> update_version;
    in property <string> update_url;
    in property <string> update_notes;
    callback copy_url(string);
```

- [ ] **Step 2: 加横幅（仿被控横幅 :584）**

在 `if !root.remote_active: VerticalLayout {`（:582）内顶部、`if root.being_controlled` 横幅之前，加：

```slint
        if root.update_available: Rectangle {
            height: 46px;
            background: Theme.accent;
            HorizontalLayout {
                padding-left: 14px; padding-right: 14px; spacing: 10px;
                Text {
                    vertical-alignment: center; horizontal-alignment: left;
                    color: white; font-size: 13px;
                    text: "发现新版 v" + root.update_version + "，建议更新";
                }
                Rectangle { horizontal-stretch: 1; }
                GhostButton {
                    label: "复制下载链接";
                    clicked => { root.copy_url(root.update_url); }
                }
            }
        }
```

> `GhostButton`、`Theme.accent` 是既有组件/主题项（被控横幅同款）。若 `Theme.accent` 不存在，用被控横幅同色（查 :584 Rectangle 的 background 值照搬）。

- [ ] **Step 3: copy_url 回调（不过滤空白，避免吃掉 URL 字符）**

`src/client/src/ui_glue.rs` 在 `on_copy_text`（:280）附近加：

```rust
    ui.on_copy_url(move |s| {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(s.to_string()); // URL 原样复制，不做白空格过滤
        }
    });
```

- [ ] **Step 4: 编译 + 手测**

Run: `cargo build -p client` → Expected: 通过（Slint 编译横幅）
手测：临时在 `main.rs` 起一处 `to_ui_tx.send(ToUi::UpdateAvailable{version:"9.9.9".into(),url:"https://rc.guoziweb.com/downloads/x".into(),notes:None})` 验证横幅出现 + 复制链接可用，验证后删除该临时代码。

- [ ] **Step 5: 提交**

```bash
git add src/client/ui/app.slint src/client/src/ui_glue.rs
git commit -m "feat(update): 更新横幅 UI(置前+复制链接 copy_url 不过滤空白)"
```

---

## Task 12: 发布流水线脚本 publish-update.sh

**Files:** Create `packaging/download/publish-update.sh`

- [ ] **Step 1: 写脚本**

```bash
#!/usr/bin/env bash
# OhMyDesk 在线更新发布：版本化产物 → 离线签名 → 上传 → 远端验收 → 原子切换 latest.json。
# 私钥在本机（rsign2/minisign），绝不进 CI。用法：
#   publish-update.sh <版本号> <win-exe路径> [--rollout 10] [--min-version 0.3.0] [--enabled false] [--allow-downgrade]
# 依赖：rsign(or minisign)、sha256sum、gzip、jq、curl、ssh/scp。需远端 chin@rc.guoziweb.com 免密 sudo。
set -euo pipefail

VER="${1:?需版本号}"; WIN_EXE="${2:?需 Windows exe 路径}"; shift 2
ROLLOUT=100; MINV="null"; ENABLED=true; DOWNGRADE=false
while [ $# -gt 0 ]; do case "$1" in
  --rollout) ROLLOUT="$2"; shift 2;;
  --min-version) MINV="\"$2\""; shift 2;;
  --enabled) ENABLED="$2"; shift 2;;
  --allow-downgrade) DOWNGRADE=true; shift;;
  *) echo "未知参数 $1" >&2; exit 1;;
esac; done

HOST="chin@rc.guoziweb.com"
DLDIR="/www/wwwroot/rc.guoziweb.com/downloads"
SECKEY="${OHMYDESK_UPDATE_SECKEY:?设 OHMYDESK_UPDATE_SECKEY=离线私钥路径}"
PUBKEY="${OHMYDESK_UPDATE_PUBKEY:?设 OHMYDESK_UPDATE_PUBKEY=公钥路径}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

WIN_NAME="ohmydesk-client-windows-x86_64-${VER}.exe"
echo "==> 1/8 版本化 + sha256 + gzip"
cp "$WIN_EXE" "$WORK/$WIN_NAME"
SHA="$(sha256sum "$WORK/$WIN_NAME" | cut -d' ' -f1)"
SIZE="$(stat -c%s "$WORK/$WIN_NAME")"
gzip -9 -kf "$WORK/$WIN_NAME"   # → $WIN_NAME.gz

echo "==> 2/8 生成 latest.json（必须填全平台 asset）"
# Linux/macOS 仅提示：填 url(版本化) + auto:false；如无对应产物则该平台客户端不提示。
jq -n --arg ver "$VER" --arg wurl "https://rc.guoziweb.com/downloads/$WIN_NAME" \
   --arg sha "$SHA" --argjson size "$SIZE" --argjson rollout "$ROLLOUT" \
   --argjson minv "$MINV" --argjson enabled "$ENABLED" --argjson dg "$DOWNGRADE" '
{
  version: $ver,
  assets: {
    windows_x86_64: { url: $wurl, sha256: $sha, size: $size, auto: true },
    linux_x86_64_deb: { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client_" + $ver + "_amd64.deb"), auto: false },
    linux_arm64_deb:  { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client_" + $ver + "_arm64.deb"),  auto: false },
    macos_arm64:      { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client-macos-arm64-" + $ver + ".tar.gz"), auto: false }
  },
  enabled: $enabled, rollout_percent: $rollout, min_version: $minv, allow_downgrade: $dg,
  notes: ("版本 " + $ver)
}' > "$WORK/latest.json"

echo "==> 3/8 离线签名"
rsign sign -s "$SECKEY" -m "$WORK/latest.json" -x "$WORK/latest.json.minisig"

echo "==> 4/8 上传版本化产物(先产物后清单)"
scp "$WORK/$WIN_NAME" "$WORK/$WIN_NAME.gz" "$HOST:/tmp/"
ssh "$HOST" "sudo mv /tmp/$WIN_NAME /tmp/$WIN_NAME.gz $DLDIR/ && sudo chown root:root $DLDIR/$WIN_NAME $DLDIR/$WIN_NAME.gz && sudo chmod 644 $DLDIR/$WIN_NAME $DLDIR/$WIN_NAME.gz"

echo "==> 5/8 远端验收(切清单前)：拉远端 exe(gzip)解压比对 sha/size + 验签"
RMT_SHA="$(curl -fsSk -H 'Accept-Encoding: gzip' --compressed "https://rc.guoziweb.com/downloads/$WIN_NAME" | sha256sum | cut -d' ' -f1)"
[ "$RMT_SHA" = "$SHA" ] || { echo "远端 exe sha256 不符，中止" >&2; exit 1; }
rsign verify -P "$(tail -1 "$PUBKEY")" -m "$WORK/latest.json" -x "$WORK/latest.json.minisig" \
  || minisign -Vm "$WORK/latest.json" -x "$WORK/latest.json.minisig" -p "$PUBKEY"

echo "==> 6/8 上传清单 + 原子切换"
scp "$WORK/latest.json" "$HOST:/tmp/latest.json.tmp"
scp "$WORK/latest.json.minisig" "$HOST:/tmp/latest.json.minisig"
ssh "$HOST" "sudo mv /tmp/latest.json.minisig $DLDIR/latest.json.minisig && sudo mv /tmp/latest.json.tmp $DLDIR/latest.json && sudo chown root:root $DLDIR/latest.json $DLDIR/latest.json.minisig && sudo chmod 644 $DLDIR/latest.json $DLDIR/latest.json.minisig"

echo "==> 7/8 更新下载页稳定别名(供人工下载页) + download.html 特定行"
ssh "$HOST" "sudo cp $DLDIR/$WIN_NAME $DLDIR/ohmydesk-client-windows-x86_64.exe && sudo cp $DLDIR/$WIN_NAME.gz $DLDIR/ohmydesk-client-windows-x86_64.exe.gz"
# download.html 版本/size 用 sudo sed -i 改特定行(禁整文件覆盖，详见 release-publish-process 记忆)

echo "==> 8/8 校验"
curl -fsSk "https://rc.guoziweb.com/downloads/latest.json" | jq -r .version
echo "发布完成：$VER（保留上一版 exe 以备回退）"
```

- [ ] **Step 2: 静态检查**

Run: `shellcheck packaging/download/publish-update.sh`
Expected: 无 error（warning 视情况修）；若无 shellcheck 跳过，至少 `bash -n packaging/download/publish-update.sh` 语法检查通过。

- [ ] **Step 3: 提交**

```bash
chmod +x packaging/download/publish-update.sh
git add packaging/download/publish-update.sh
git commit -m "feat(release): publish-update.sh 发布流水线(版本化/签名/远端验收/原子切换/别名)"
```

> 真实发布留待 0.3.0 tag 后由维护者执行（rsign 命令参数以本机 rsign2 版本为准微调）。

---

## Self-Review（写完计划自查）

**Spec 覆盖：** §3 数据流→Task1-9；§4 模块/ClientActivityState/UpdateNotice→Task6/9/10/11；§5 安全(签名/同源)→Task4/5；§6 依赖→Task1；§7 同源策略→Task4；§8 发布流水线→Task12；§9 灰度/降级→Task3；三触发(启动+6h+重连)→Task9；临时文件落盘/清理→Task7/8；横幅可见性→Task10/11。无遗漏。

**类型一致性：** `Manifest`/`Asset`/`UpdateAction`/`decide_with_asset`/`ClientActivityState`/`CapReader`/`download_verified`/`apply`/`spawn_update_daemon`/`ToUi::UpdateAvailable`/`set_update_available` 跨任务命名统一。

**已知执行注意：** Task9 引用的 `ToUi::UpdateAvailable` 定义在 Task10 Step1——执行时 Task9 与 Task10 需连续完成再整体编译（计划已在 Task9 Step3/Step4 标注）。`self_replace`/`ureq`/`minisign-verify` 精确 API 以锁定版本文档为准，交叉编译检查(`cargo check --target x86_64-pc-windows-gnu`)是硬门槛。
