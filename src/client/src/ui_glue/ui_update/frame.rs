//! 帧/光标类 ToUi 事件处理：Frame、Cursor。
//! decode_frame_rgba 在本模块（tokio 线程解码，UI 线程构造 Image）。
use crate::{AppWindow, SharedSession};
use protocol;
use slint::ComponentHandle;
use std::sync::atomic::Ordering as AtomicOrdering;

/// JPEG base64 → 裸 RGBA 字节 + 尺寸（在非 UI 线程解码；Image 在 UI 线程构造，避免跨线程 Send）。
fn decode_frame_rgba(data: &str) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let bytes = STANDARD.decode(data)?;
    let dyn_img = image::load_from_memory(&bytes)?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok((rgba.into_raw(), w, h))
}

pub(super) fn handle_frame(
    ui_weak: &slint::Weak<AppWindow>,
    cur_session: &SharedSession,
    ended_session: &SharedSession,
    last_frame_dims: &mut Option<(u32, u32)>,
    recv_stats: &mut crate::telemetry::MainRecvStats,
    session_id: String,
    data: String,
    w: u32,
    h: u32,
    seq: u64,
) {
    // 丢弃已断开会话的迟到帧：否则在途帧会把已断开的远程态「复活」（需点两次断开）。
    if super::session::frame_belongs_to_ended(&ended_session.lock().unwrap(), &session_id) {
        return;
    }
    // 首帧标志:仅连上远程收到的第一帧才自动贴合窗口尺寸(见下方 set_size)。
    // 之后 adaptive 过载降档会让分辨率不停变，若每次都 set_size，窗口会在用户
    // 拖动/操作时被强行改尺寸+重定位 → 表现为「窗口随机变大小和位置」「最大化下字体割裂」。
    let is_first_frame = last_frame_dims.is_none();
    let dims_changed = *last_frame_dims != Some((w, h));
    if dims_changed {
        tracing::info!(
            "主控收到帧分辨率={w}x{h}（流畅档≤1280×720 / 高清档≤1920×1080）"
        );
        *last_frame_dims = Some((w, h));
    }
    // 统一会话态：收到帧即把 cur_session 设为该会话——保证「有画面时输入一定有目标」，
    // 即便 RemoteAck 因时序/路由未设上 cur_session，输入也不会被静默丢弃。
    {
        let mut s = cur_session.lock().unwrap();
        if s.as_deref() != Some(session_id.as_str()) {
            *s = Some(session_id.clone());
        }
    }
    // 在本（tokio）线程解码 JPEG→RGBA（产出 Vec<u8> 是 Send）；Image 非 Send，
    // 故只把裸 RGBA + 尺寸传进闭包，在 UI 线程内构造 Image（slint.md §3 坑 2）。
    let t_dec = std::time::Instant::now();
    let decoded = decode_frame_rgba(&data);
    let decode_ms = t_dec.elapsed().as_millis() as u32;
    if let Some(line) = recv_stats.on_frame(seq, decode_ms, crate::update::now_ms()) {
        tracing::info!("{line}");
    }
    if let Ok((rgba, iw, ih)) = decoded {
        let ui_weak = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut buffer =
                    slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(iw, ih);
                buffer.make_mut_bytes().copy_from_slice(&rgba);
                ui.set_frame_w(w as i32);
                ui.set_frame_h(h as i32);
                ui.set_frame(slint::Image::from_rgba8(buffer));
                ui.set_remote_active(true);
                // 把主控窗口调到接近被控分辨率，让远程桌面尽量 1:1 显示，避免被压进小窗
                // 强制下采样导致发虚。仅尺寸变化时调整。
                // DPI 感知：set_size 用逻辑像素，除以主控缩放系数，使窗口的「物理」尺寸≈帧尺寸
                // （高 DPI 主控上才不会把窗口放大到溢出屏幕）。上限取常见屏物理 1920×1080。
                //
                // 【仅首帧贴合，且非最大化/全屏】只在连上远程的第一帧把窗口调到接近被控
                // 分辨率。之后 adaptive 过载降档让分辨率频繁跳变（1920↔1632↔1344↔1056…），
                // 若每次都 set_size，会与窗口管理器/用户拖动抢状态 → 窗口随机变大小和位置、
                // 最大化下渲染表面与布局 desync 致字体割裂。首帧后一律不再动窗口，画面靠
                // frame_scale 在窗口内自适应缩放。
                let win = ui.window();
                // 首帧贴合 或 用户切分辨率档触发的一次性重贴合（REFIT_PENDING）。
                // 均要求「尺寸真变 + 非最大化/全屏」，且重贴合后清位——单次 set_size，
                // 不受 adaptive 降档抖动影响，避免窗口风暴/最大化字体割裂（见 900-946 注释）。
                let want_refit =
                    is_first_frame || super::REFIT_PENDING.load(AtomicOrdering::Relaxed);
                if want_refit
                    && dims_changed
                    && !win.is_maximized()
                    && !win.is_fullscreen()
                {
                    let sf = win.scale_factor().max(1.0);
                    let win_w = (w.min(1920) as f32) / sf;
                    let win_h = (h.min(1080) as f32) / sf;
                    win.set_size(slint::LogicalSize::new(win_w, win_h));
                    super::REFIT_PENDING.store(false, AtomicOrdering::Relaxed);
                }
            }
        });
    }
}

pub(super) fn handle_cursor(
    ui_weak: &slint::Weak<AppWindow>,
    visible: bool,
    shape: Option<protocol::CursorShape>,
) {
    // 解码裸 RGBA(base64)→ UI 线程构造 Image 并设光标属性;shape=None 仅改可见性。
    let decoded = shape.as_ref().and_then(|s| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&s.rgba)
            .ok()
            .filter(|b| b.len() == (s.w * s.h * 4) as usize)
            .map(|b| (b, s.w, s.h, s.hotspot_x, s.hotspot_y))
    });
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_cursor_visible(visible);
            if let Some((bytes, cw, ch, hx, hy)) = decoded {
                let mut buffer =
                    slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(cw, ch);
                buffer.make_mut_bytes().copy_from_slice(&bytes);
                ui.set_cursor_img(slint::Image::from_rgba8(buffer));
                ui.set_cursor_w(cw as i32);
                ui.set_cursor_h(ch as i32);
                ui.set_cursor_hotspot_x(hx as i32);
                ui.set_cursor_hotspot_y(hy as i32);
                ui.set_cursor_ready(true);
            }
        }
    });
}
