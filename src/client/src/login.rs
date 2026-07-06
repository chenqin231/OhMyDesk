//! 客户端登录：复用 [`crate::update::build_agent`] 的 ureq（含 TLS 后端）调 `/api/login` 取 JWT。
//! 零新依赖（Phase 0 D1）。ureq 未启 "json" 特性 → 手工 serde_json 序列化请求体、`into_string` 读响应。

use crate::credential::Creds;

/// 登录失败分类（映射到 UI inline 文案）。
#[derive(Debug, Clone, PartialEq)]
pub enum LoginErr {
    /// 账号或密码错误（服务端 401）。禁用账号在 /api/login 亦返回 401 且不可区分，兜底同此（D1）。
    BadCredential,
    /// 无法连接服务器（网络/超时/DNS/地址非法）。
    Network,
    /// 服务端异常状态或响应体无法解析（非预期，兜底）。
    Server,
}

impl LoginErr {
    /// 映射到 UI inline 文案（错误 = 问题 + 恢复建议）。
    pub fn message(&self) -> &'static str {
        match self {
            LoginErr::BadCredential => "账号或密码错误",
            LoginErr::Network => "无法连接服务器，请检查网络后重试",
            LoginErr::Server => "服务器返回异常，请稍后重试",
        }
    }
}

/// 用账号密码登录 `server`，成功返回 `Creds{token,user}`。
///
/// `server` 形如 `wss://host:port/ws` / `ws://host:port` / `https://host` —— 内部归一为
/// http(s) base + `/api/login`。阻塞式 ureq 调用，应在后台线程执行（见 `ui_glue::on_login`）。
pub fn login(server: &str, user: &str, pass: &str) -> Result<Creds, LoginErr> {
    let url = login_url(server).ok_or(LoginErr::Network)?;
    let agent = crate::update::build_agent(8, 8);
    let body = serde_json::to_string(&serde_json::json!({ "user": user, "pass": pass }))
        .map_err(|_| LoginErr::Server)?;
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body);
    match resp {
        Ok(r) => {
            let mut creds = parse_ok(&r.into_string().map_err(|_| LoginErr::Server)?, user)?;
            creds.server = Some(server.trim().to_string());
            Ok(creds)
        }
        Err(ureq::Error::Status(401, _)) => Err(LoginErr::BadCredential),
        Err(ureq::Error::Status(_, _)) => Err(LoginErr::Server),
        Err(ureq::Error::Transport(_)) => Err(LoginErr::Network),
    }
}

/// 解析成功响应体 `{token,user,...}` → Creds。token 缺失/空 → Server 错。user 缺失回落输入账号。
fn parse_ok(text: &str, fallback_user: &str) -> Result<Creds, LoginErr> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| LoginErr::Server)?;
    let token = v
        .get("token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or(LoginErr::Server)?;
    let user = v
        .get("user")
        .and_then(|u| u.as_str())
        .unwrap_or(fallback_user);
    Ok(Creds {
        token: token.to_string(),
        user: user.to_string(),
        server: None,
    })
}

/// 归一服务器地址 → `scheme://authority/api/login`。
/// wss→https、ws→http；无 scheme 默认 https；剥离原路径（/ws 等）。地址非法返回 None。
fn login_url(server: &str) -> Option<String> {
    let s = server.trim();
    if s.is_empty() {
        return None;
    }
    let base = if let Some(rest) = s.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = s.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let u = url::Url::parse(&base).ok()?;
    let host = u.host_str()?;
    let scheme = u.scheme();
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}/api/login"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_归一各种_scheme() {
        assert_eq!(
            login_url("wss://a.com:8443/ws").as_deref(),
            Some("https://a.com:8443/api/login")
        );
        assert_eq!(
            login_url("ws://a.com:9000").as_deref(),
            Some("http://a.com:9000/api/login")
        );
        assert_eq!(
            login_url("https://a.com").as_deref(),
            Some("https://a.com/api/login")
        );
        // 无 scheme → 默认 https
        assert_eq!(
            login_url("a.com:8443").as_deref(),
            Some("https://a.com:8443/api/login")
        );
        // 空/非法
        assert_eq!(login_url("   "), None);
        assert_eq!(login_url("wss://"), None);
    }

    #[test]
    fn parse_ok_提取_token_与_user() {
        let ok = parse_ok(
            r#"{"token":"jwt-xyz","user":"alice","tier":"user"}"#,
            "fallback",
        );
        assert_eq!(
            ok,
            Ok(Creds {
                token: "jwt-xyz".into(),
                user: "alice".into(),
                server: None
            })
        );
        // user 缺失 → 回落输入账号
        let ok2 = parse_ok(r#"{"token":"jwt-xyz"}"#, "bob");
        assert_eq!(ok2.unwrap().user, "bob");
    }

    #[test]
    fn parse_ok_token缺失或空_报_server错() {
        assert_eq!(parse_ok(r#"{"user":"a"}"#, "a"), Err(LoginErr::Server));
        assert_eq!(parse_ok(r#"{"token":""}"#, "a"), Err(LoginErr::Server));
        assert_eq!(parse_ok("not json", "a"), Err(LoginErr::Server));
    }

    #[test]
    fn error_文案映射() {
        assert_eq!(LoginErr::BadCredential.message(), "账号或密码错误");
        assert_eq!(
            LoginErr::Network.message(),
            "无法连接服务器，请检查网络后重试"
        );
    }
}
