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

/// 解析清单，先卡 64KB 上限。
pub fn parse_manifest(bytes: &[u8]) -> anyhow::Result<Manifest> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("manifest 超过 {} 字节上限", MAX_MANIFEST_BYTES);
    }
    Ok(serde_json::from_slice(bytes)?)
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
}
