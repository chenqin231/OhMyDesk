//! 在线自动更新：检测/验签/决策（纯逻辑，跨平台单测）+ 下载/替换（仅 Windows）。
//! 设计见 docs/superpowers/specs/2026-06-30-windows-client-auto-update-design.md。
use std::collections::HashMap;
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

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
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

/// 更新清单签名公钥（minisign/rsign2，base64）。生产离线私钥在仓库外（~/.ohmydesk/update-sec.key）。
/// 仅持有该私钥者能签发更新；公钥变更需发一次客户端更新。
pub const UPDATE_PUBKEY: &str = "RWS0AI/ZpzOZlNDOZTZgprG0z8RXAYlVp44zo22Zo6Kkm2wOidzPz+Cl";

/// 解析清单，先卡 64KB 上限。
pub fn parse_manifest(bytes: &[u8]) -> anyhow::Result<Manifest> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("manifest 超过 {} 字节上限", MAX_MANIFEST_BYTES);
    }
    Ok(serde_json::from_slice(bytes)?)
}

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

/// 用内置公钥验证 latest.json 的分离签名（minisig 文件全文）。任何失败 → false。
pub fn verify_manifest_sig(pubkey_b64: &str, manifest_bytes: &[u8], minisig: &str) -> bool {
    use minisign_verify::{PublicKey, Signature};
    match (PublicKey::from_base64(pubkey_b64), Signature::decode(minisig)) {
        (Ok(pk), Ok(sig)) => pk.verify(manifest_bytes, &sig, false).is_ok(),
        _ => false,
    }
}

#[cfg_attr(not(windows), allow(dead_code))] // 仅 windows download_verified + 单测用
fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 边读边算 SHA-256 + 强制上限的 Reader 包装。
#[cfg_attr(not(windows), allow(dead_code))] // 仅 windows download_verified + 单测构造
pub struct CapReader<R> {
    inner: R,
    cap: u64,
    read: u64,
    hasher: sha2::Sha256,
}
#[cfg_attr(not(windows), allow(dead_code))]
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
    let agent = build_agent(10, 120);
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

use std::sync::Arc;
use std::time::Duration;
use crate::activity::ClientActivityState;
use crate::net::ToUi;
use tokio::sync::mpsc::UnboundedSender;

/// 构建带 TLS 后端的 ureq Agent（自动更新 HTTPS 用）。
/// Windows：ureq 仅启 `native-tls` 特性**不会自动接管** TLS，必须显式 `tls_connector` 装配
/// SChannel，否则运行时报「no TLS backend is configured」→ HTTPS 全废、自动更新永久失败
/// （2026-07-01 实测根因：Windows 自更新从上线起从未成功）。非 Windows 用 rustls(`tls`)默认后端。
#[cfg(windows)]
fn build_agent(connect_s: u64, read_s: u64) -> ureq::Agent {
    let b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(connect_s))
        .timeout_read(Duration::from_secs(read_s));
    match native_tls::TlsConnector::new() {
        Ok(c) => b.tls_connector(Arc::new(c)).build(),
        Err(e) => { tracing::warn!("native-tls 初始化失败：{e}"); b.build() }
    }
}
#[cfg(not(windows))]
fn build_agent(connect_s: u64, read_s: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(connect_s))
        .timeout_read(Duration::from_secs(read_s))
        .build()
}

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
        std::thread::sleep(Duration::from_secs(5)); // 启动延迟：短暂避开连接抖动，尽快「打开即检查」
        loop {
            if let Err(e) = run_once(&base, &endpoint_id, &state, &to_ui) {
                tracing::warn!("更新检查失败：{e}");
                let _ = to_ui.send(ToUi::UpdateStatus { text: "更新检查失败，稍后重试".into() });
            }
            let _ = nudge_rx.recv_timeout(Duration::from_secs(interval)); // nudge 或超时唤醒
        }
    });
}

fn run_once(base: &url::Url, endpoint_id: &str, state: &Arc<ClientActivityState>, to_ui: &UnboundedSender<ToUi>) -> anyhow::Result<()> {
    let _ = to_ui.send(ToUi::UpdateStatus { text: "正在检查更新…".into() });
    let agent = build_agent(10, 30);
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
        UpdateAction::Skip => { let _ = to_ui.send(ToUi::UpdateStatus { text: format!("已是最新（当前 v{}）", env!("CARGO_PKG_VERSION")) }); Ok(()) }
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
    let _ = to_ui.send(ToUi::UpdateStatus { text: format!("正在下载更新 v{version}…") });
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
        let _ = to_ui.send(ToUi::UpdateStatus { text: format!("有远控会话，稍后自动更新 v{version}") });
        Ok(()) // staged 临时文件随 drop 删除，下周期重下
    }
}

#[cfg(not(windows))]
fn apply_auto(_s: &Arc<ClientActivityState>, _t: &UnboundedSender<ToUi>, _v: &str, _u: &str, _h: &str, _z: u64) -> anyhow::Result<()> {
    Ok(()) // 非 Windows 不会进 AutoUpdate（asset.auto=false），兜底空实现
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version":"0.3.0",
      "assets":{"windows_x86_64":{"url":"https://h/d/c-0.3.0.exe","sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","size":10,"auto":true}},
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
        assert!(m.enabled);
        assert_eq!(m.rollout_percent, 100);
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
        let err = parse_manifest(&big).unwrap_err();
        assert!(err.to_string().contains("超过"), "应命中上限分支: {err}");
    }

    // ── Task 2: is_newer + bucket ──

    #[test]
    fn 版本比对_仅更高才为真() {
        assert!(is_newer("0.3.0", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1")); // 相等
        assert!(!is_newer("0.2.0", "0.2.1")); // 更低
        assert!(!is_newer("乱码", "0.2.1"));   // 非法保守不更新
    }

    #[test]
    fn 分桶_同id恒定且落0_99() {
        let a = bucket("123456789");
        let b = bucket("123456789");
        assert_eq!(a, b);          // 确定性
        assert!(a < 100);
        assert_ne!(bucket("111"), bucket("999")); // 不同 id 大概率不同（此对已验证不同）
    }

    // ── Task 3: decide ──

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

    // ── Task 4: resolve_base + same_origin ──

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

    #[test]
    fn 同源_异端口拒绝() {
        let b = resolve_base("wss://rc.guoziweb.com/ws", None).unwrap();
        assert!(!same_origin(&b, "https://rc.guoziweb.com:8443/c.exe"));
    }

    // ── Task 5: verify_manifest_sig ──

    const FIXTURE_JSON: &[u8] = include_bytes!("../tests/fixtures/sample-latest.json");
    const FIXTURE_SIG: &str = include_str!("../tests/fixtures/sample-latest.json.minisig");
    // 测试密钥（无口令，仅测试用，与生产 UPDATE_PUBKEY 分离）
    // 对应私钥在 /tmp/test-sec.key（不进仓库），公钥见 tests/fixtures/test-pub.key（第二行）
    const TEST_PUBKEY: &str = "RWRzaczSwstueX4YhGyIp2a0OF4i9wXQgfyLNPMimff8W5ZI/lGvTYP4";

    #[test]
    fn 验签_合法夹具通过() {
        assert!(verify_manifest_sig(TEST_PUBKEY, FIXTURE_JSON, FIXTURE_SIG));
    }

    #[test]
    fn 验签_篡改字节失败() {
        let mut tampered = FIXTURE_JSON.to_vec();
        tampered[0] ^= 0xFF;
        assert!(!verify_manifest_sig(TEST_PUBKEY, &tampered, FIXTURE_SIG));
    }

    #[test]
    fn 验签_错公钥失败() {
        let bad = "RWTGiBCq9999999999999999999999999999999999999999999999=";
        assert!(!verify_manifest_sig(bad, FIXTURE_JSON, FIXTURE_SIG));
    }

    #[test]
    fn 生产公钥_拒绝非己签名() {
        // 夹具由测试密钥签名；生产 UPDATE_PUBKEY 必拒（验证内置公钥与签名绑定，非任意签名都收）。
        assert!(!verify_manifest_sig(UPDATE_PUBKEY, FIXTURE_JSON, FIXTURE_SIG));
    }

    #[test]
    fn 内置公钥_可解析() {
        // 防发版误填/截断：UPDATE_PUBKEY 必须是合法 minisign 公钥，否则 verify 恒 false 静默禁更。
        assert!(minisign_verify::PublicKey::from_base64(UPDATE_PUBKEY).is_ok());
    }

    // ── Task 7 Step1: CapReader ──

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
}
