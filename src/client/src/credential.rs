//! 客户端登录凭据本地持久化：token + 用户名，落状态目录 `credential.json`。
//!
//! 「记住登录」数据源：登录成功后 [`save`] 落盘；启动时 [`load`] 读盘，有效则跳过登录页自动上线；
//! 注销时 [`clear`] 删除。仿 [`crate::history`] 的容错落盘范式——文件缺失/损坏一律降级为「未登录」，
//! 不 panic、不报错（首启动/坏盘均正常回落登录页）。
//!
//! 安全（Security NFR / T031）：Unix 下落盘后置 `0600`（仅属主可读写），防同机他用户直接读取 token；
//! token 从不打印日志或渲染到界面。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一份登录凭据：服务端签发的 JWT + 归属用户名（仅用于顶栏展示，鉴权只认 token）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Creds {
    /// 服务端 /api/login 签发的 JWT（WS 建连 `?token=` + HTTP Bearer 用）。
    pub token: String,
    /// 登录用户名（顶栏「已登录:<user>」展示用；非鉴权凭据）。
    pub user: String,
    /// 签发该 token 的服务器地址。旧凭据没有此字段时为 None，回落启动默认地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

/// 凭据文件路径：`<state_dir>/credential.json`（与日志/config 同目录，见 `ohmydesk_state_dir`）。
fn credential_path() -> PathBuf {
    crate::ohmydesk_state_dir().join("credential.json")
}

/// 读取凭据。文件缺失/损坏一律返回 None（不 panic、不报错）。
pub fn load() -> Option<Creds> {
    load_from(&credential_path())
}

/// 落盘凭据（容错：目录缺失则创建；序列化/写盘失败仅 warn 不 panic）。写后置 0600（Unix）。
pub fn save(creds: &Creds) {
    save_to(&credential_path(), creds)
}

/// 清除凭据（注销）。文件不存在视为成功。
pub fn clear() {
    clear_at(&credential_path())
}

// ── 带路径参的内部实现（便于单测注入临时路径，不碰真实状态目录）─────────────────

fn load_from(path: &Path) -> Option<Creds> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<Creds>(&bytes).ok()
}

fn save_to(path: &Path, creds: &Creds) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = match serde_json::to_vec_pretty(creds) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("凭据序列化失败：{e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(path, json) {
        tracing::warn!("凭据落盘失败：{e}");
        return;
    }
    set_owner_only(path);
}

fn clear_at(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("凭据清除失败：{e}"),
    }
}

/// Unix：凭据文件仅属主可读写（0600），防同机他用户读取 token（Security NFR / T031）。
#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!("凭据权限设置失败（0600）：{e}");
    }
}
#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ohmydesk-cred-{}-{}.json",
            std::process::id(),
            name
        ))
    }

    /// T007：save→load 往返一致；clear 后 load 得 None；Unix 权限 0600。
    #[test]
    fn save_load_clear_往返() {
        let p = tmp("roundtrip");
        let _ = std::fs::remove_file(&p);

        // 缺失 → None
        assert_eq!(load_from(&p), None, "文件缺失应得 None");

        // save → load 往返一致
        let c = Creds {
            token: "tok-abc123".into(),
            user: "alice".into(),
            server: Some("ws://172.16.76.1:8765/ws".into()),
        };
        save_to(&p, &c);
        assert_eq!(load_from(&p), Some(c), "往返应一致");

        // 安全：Unix 权限 0600（仅属主可读写，T031）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "凭据文件应为 0600（仅属主可读写）");
        }

        // clear → load None
        clear_at(&p);
        assert_eq!(load_from(&p), None, "清除后应得 None");
        // 重复 clear 不报错（文件已不存在）
        clear_at(&p);
    }

    #[test]
    fn load_旧凭据缺server_兼容为空() {
        let p = tmp("legacy-no-server");
        std::fs::write(&p, r#"{"token":"tok-abc123","user":"alice"}"#).unwrap();
        let c = load_from(&p).expect("旧凭据应兼容读取");
        assert_eq!(c.token, "tok-abc123");
        assert_eq!(c.user, "alice");
        assert_eq!(c.server, None);
        let _ = std::fs::remove_file(&p);
    }

    /// T007：文件损坏（非法 JSON）→ load 返回 None，不 panic。
    #[test]
    fn load_损坏文件_返回空_不panic() {
        let p = tmp("corrupt");
        std::fs::write(&p, b"{not valid json at all").unwrap();
        assert_eq!(load_from(&p), None, "损坏文件应降级为 None");
        let _ = std::fs::remove_file(&p);
    }
}
