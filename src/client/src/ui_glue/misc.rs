//! 采集回调·杂项：剪贴板、更新检查、素问、tab 切换、密码刷新、渲染模式、诊断导出/路径复制。
use super::UiCtx;
use crate::{net, AppWindow};
use slint::ComponentHandle;

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // 复制 ID/密码到剪贴板（ID 分组带空格，先去白）
    {
        ui.on_copy_text(move |s| {
            let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(cleaned);
            }
        });
    }
    // 复制更新下载链接（URL 原样复制，不过滤空白，避免吃掉 URL 字符）
    {
        ui.on_copy_url(move |s| {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(s.to_string()); // URL 原样复制，不做白空格过滤
            }
        });
    }
    // 手动检查更新：先置「检查中」状态再唤醒守护立即做一次检查（nudge）
    {
        let ui_weak = ui.as_weak();
        ui.on_check_update(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_update_status("正在检查更新…".into());
                ui.set_update_phase(1);
            }
            crate::update::nudge();
        });
    }
    // AI助手:检测/静默安装/拉起素问。仅 Windows 显示按钮(其余平台 supported=false 隐藏)。
    {
        ui.set_suwen_supported(cfg!(windows));
        let ui_weak = ui.as_weak();
        ui.on_launch_suwen(move || {
            crate::suwen::ensure_and_launch(ui_weak.clone());
        });
    }
    // 刷新临时密码（重发 Register，server upsert 覆盖；新密码经 Registered 回推展示）
    {
        let tx = cx.from_ui_tx.clone();
        ui.on_refresh_password(move || {
            let _ = tx.send(net::FromUi::RefreshPassword);
        });
    }
    // ── tab 切换 → 懒推流：tab 0(远程桌面)发 SetCapture{active:true}，其余 false ──
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        ui.on_tab_changed(move |tab| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                let _ = tx.send(net::FromUi::SetCapture {
                    session_id: sid,
                    active: tab == 0,
                });
            }
        });
    }
    // ── 诊断菜单：模式热切（render_mode 原子，运行期即时生效，无需重启） ──
    ui.on_set_render_mode(move |m| {
        if let Some(mode) = crate::render_mode::parse_mode(&m.to_string()) {
            crate::render_mode::apply(mode);
            tracing::warn!("UI 热切渲染模式 → {:?}", crate::render_mode::current_mode());
        }
    });
    // ── 诊断菜单：导出诊断包（发 ExportNow 给 collector 落盘） ──
    {
        let tele = cx.telemetry_tx.clone();
        ui.on_export_diag(move || {
            let _ = tele.send(crate::telemetry::TelemetryMsg::ExportNow);
        });
    }
    // ── 诊断菜单：复制诊断目录路径（复用已有 copy_text callback） ──
    {
        let ui_weak = ui.as_weak();
        ui.on_copy_diag_path(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.invoke_copy_text(ui.get_diag_dir());
            }
        });
    }
}
