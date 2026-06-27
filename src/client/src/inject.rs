//! 键鼠注入：收 [`InputEvent`] → enigo 注入，坐标按 frame→real 还原（P-CLI4）。
//!
//! 坑点（见 references/xcap-enigo.md）：
//! - 必须 `use enigo::{Mouse, Keyboard}`，否则 trait 方法不可见。
//! - 0.6：`Enigo::new(&Settings)` 返回 Result；move_mouse/button/key/text 全返回 Result；
//!   字符键用 `Key::Unicode(char)`；`release_keys_when_dropped` 默认 true 防卡键。
//! - 坐标换算用 crate::geom::map_frame_to_real，逻辑可单测（注入本身依赖 X11 不强测）。

use crate::geom::map_frame_to_real;
use enigo::{
    Button,
    Coordinate::Abs,
    Direction::{Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use protocol::InputEvent;

/// 键鼠注入器：持有 enigo + 被控真实屏尺寸 + 当前帧尺寸（用于坐标换算）。
pub struct Injector {
    enigo: Enigo,
    real_w: u32,
    real_h: u32,
    frame_w: u32,
    frame_h: u32,
}

impl Injector {
    /// `real_*` = 被控真实屏，`frame_*` = 首帧缩放后尺寸（从 Frame 消息取）。
    pub fn new(real_w: u32, real_h: u32, frame_w: u32, frame_h: u32) -> anyhow::Result<Self> {
        Ok(Injector {
            enigo: Enigo::new(&Settings::default())?,
            real_w,
            real_h,
            frame_w: frame_w.max(1),
            frame_h: frame_h.max(1),
        })
    }

    /// 帧内坐标 → 真实屏坐标（纯换算，单测覆盖）。
    fn to_real(&self, x: i32, y: i32) -> (i32, i32) {
        map_frame_to_real(x, y, self.frame_w, self.frame_h, self.real_w, self.real_h)
    }

    /// 注入一个输入事件。
    pub fn apply(&mut self, ev: &InputEvent) -> anyhow::Result<()> {
        match ev {
            InputEvent::MouseMove { x, y } => {
                let (rx, ry) = self.to_real(*x, *y);
                self.enigo.move_mouse(rx, ry, Abs)?;
            }
            InputEvent::MouseButton { button, down } => {
                let b = match button {
                    0 => Button::Left,
                    1 => Button::Middle,
                    _ => Button::Right,
                };
                self.enigo
                    .button(b, if *down { Press } else { Release })?;
            }
            InputEvent::Key { code, down } => {
                if let Some(c) = code.chars().next() {
                    self.enigo
                        .key(Key::Unicode(c), if *down { Press } else { Release })?;
                }
            }
            InputEvent::Text { text } => {
                self.enigo.text(text)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::geom::map_frame_to_real;

    // 注入器构造依赖 X11 DISPLAY，CI/无头环境不可用；此处只单测坐标换算逻辑（与 Injector::to_real 同源）。
    #[test]
    fn 注入坐标换算_与geom一致() {
        // frame 1280×720，real 1920×1080：帧内 (640,360) → 真实中心 (960,540)
        assert_eq!(map_frame_to_real(640, 360, 1280, 720, 1920, 1080), (960, 540));
    }

    #[test]
    fn 注入坐标换算_非16比9() {
        // frame 960×720，real 1600×1200：帧角 → 真实角，不偏
        assert_eq!(map_frame_to_real(960, 720, 960, 720, 1600, 1200), (1600, 1200));
    }
}
