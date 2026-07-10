//! 窗口恢复时强制重绘，修「最小化→恢复后白板」。
use crate::AppWindow;
use slint::ComponentHandle;

/// 修「最小化→托盘图标恢复后白板」：Slint 软渲染器按 softbuffer buffer age 做脏区复用——
/// 假设上一帧像素仍在缓冲里。Windows 最小化→恢复不重建表面(age 仍有效),但窗口内容已被 OS 清空,
/// 于是渲染器只重绘「新脏区」(如点中的控件)、其余留白 = 白板+局部。仅 request_redraw 无效(脏区为空)。
/// 根治：恢复瞬间把窗口逻辑高度 +1px 再复原,强制 winit 重建 softbuffer 表面(buffer age=0 → 整窗重绘)。
/// 触发门：最小化态→非最小化态(Windows)或 Occluded(false)(X11/Wayland/macOS);最大化/全屏不 nudge
/// (winit 忽略尺寸变更,且此类恢复通常自带 Resized 会整窗重绘)。
pub fn wire_repaint_on_restore(ui: &AppWindow) {
    use i_slint_backend_winit::winit::event::WindowEvent;
    use i_slint_backend_winit::{EventResult, WinitWindowAccessor};
    use std::cell::Cell;

    let ui_weak = ui.as_weak();
    // 最小化状态跟踪：true→false = 恢复。回调对多数 winit 事件触发,故正常使用(未最小化)恒 false,不误触。
    let was_minimized = Cell::new(false);
    ui.window().on_winit_window_event(move |win, ev| {
        let now_min = win.is_minimized();
        let restored = was_minimized.get() && !now_min;
        was_minimized.set(now_min);

        if restored || matches!(ev, WindowEvent::Occluded(false)) {
            win.request_redraw();
            if let Some(ui) = ui_weak.upgrade() {
                let w = ui.window();
                if !w.is_maximized() && !w.is_fullscreen() {
                    let sf = w.scale_factor();
                    let sz = w.size(); // 物理像素 → 逻辑像素(set_size 要逻辑)
                    let lw = sz.width as f32 / sf;
                    let lh = sz.height as f32 / sf;
                    w.set_size(slint::LogicalSize::new(lw, lh + 1.0));
                    let back = ui.as_weak();
                    slint::Timer::single_shot(std::time::Duration::from_millis(32), move || {
                        if let Some(ui) = back.upgrade() {
                            ui.window().set_size(slint::LogicalSize::new(lw, lh));
                        }
                    });
                }
            }
        }
        EventResult::Propagate
    });
}
