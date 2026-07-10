//! 登录/注销回调 与 token 监听。
use crate::AppWindow;
use slint::ComponentHandle;

/// 登录 / 注销回调（客户端账号登录 + 归属绑定）。独立于 net 上行通道：
/// - `on_do_login`：后台线程阻塞 ureq 调 `/api/login` → `invoke_from_event_loop` 回填 UI；
///   成功则存盘凭据 + 经 `token_tx`(watch) 通知 `net::run` 携 token 上线（放行登录门，服务端派生 owner）。
/// - `on_do_logout`：清凭据 + token 置 None（令 net 主动断开 WS + 回登录门）+ 回登录页。
pub fn wire_login_callbacks(
    ui: &AppWindow,
    token_tx: std::sync::Arc<tokio::sync::watch::Sender<Option<String>>>,
    server_url: String,
    active_server_url: crate::SharedServerUrl,
) {
    // on_do_login(user, pass, server_override)
    {
        let ui_weak = ui.as_weak();
        let token_tx = token_tx.clone();
        let default_server = server_url;
        let active_server_url = active_server_url.clone();
        ui.on_do_login(move |user, pass, server_override| {
            let user = user.to_string();
            let pass = pass.to_string();
            // 服务器地址：高级项非空则用它，否则用默认（env/OHMYDESK_SERVER）。
            let server = selected_login_server(&default_server, &server_override);
            let ui_weak = ui_weak.clone();
            let token_tx = token_tx.clone();
            let active_server_url = active_server_url.clone();
            // 进入 loading 态（回调在 UI 线程，直接 set）。
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_login_loading(true);
                ui.set_login_error("".into());
            }
            // 阻塞式 ureq 放后台线程，完成后投回 UI 线程回填。
            std::thread::spawn(move || {
                let result = crate::login::login(&server, &user, &pass);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else {
                        return;
                    };
                    ui.set_login_loading(false);
                    match result {
                        Ok(creds) => {
                            crate::credential::save(&creds);
                            *active_server_url.lock().unwrap() = server;
                            ui.set_logged_user(creds.user.clone().into());
                            ui.set_login_pass("".into()); // 清密码框
                            ui.set_login_error("".into());
                            ui.set_logged_in(true);
                            // 放行 net 登录门：携 token 上线（服务端据此派生 owner）。
                            let _ = token_tx.send(Some(creds.token));
                        }
                        Err(e) => {
                            // 错密码：清空密码框、账号保留（AC-001-E1）；网络错则保留密码便于直接重试。
                            if e == crate::login::LoginErr::BadCredential {
                                ui.set_login_pass("".into());
                            }
                            ui.set_login_error(e.message().into());
                        }
                    }
                });
            });
        });
    }
    // on_do_logout（S3 确定）
    {
        let ui_weak = ui.as_weak();
        let token_tx = token_tx.clone();
        ui.on_do_logout(move || {
            crate::credential::clear();
            let _ = token_tx.send(None); // 令 net 主动断开 WS + 回登录门（终端离线，归属服务端保留）
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_logged_in(false);
                ui.set_connected(false);
                ui.set_login_pass("".into());
                ui.set_login_error("".into());
            }
        });
    }
}

fn selected_login_server(default_server: &str, server_override: &str) -> String {
    let server = server_override.trim();
    if server.is_empty() {
        default_server.to_string()
    } else {
        server.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 登录服务器地址_高级项覆盖默认地址() {
        assert_eq!(
            selected_login_server("wss://rc.guoziweb.com/ws", " ws://172.16.76.1:8765/ws "),
            "ws://172.16.76.1:8765/ws"
        );
        assert_eq!(
            selected_login_server("wss://rc.guoziweb.com/ws", "   "),
            "wss://rc.guoziweb.com/ws"
        );
    }
}
