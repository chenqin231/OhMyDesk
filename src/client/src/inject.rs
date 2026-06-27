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
                self.enigo.button(b, if *down { Press } else { Release })?;
            }
            InputEvent::Key { code, down } => {
                if let Some(key) = code_to_key(code) {
                    self.enigo
                        .key(key, if *down { Press } else { Release })?;
                }
            }
            InputEvent::Text { text } => {
                self.enigo.text(text)?;
            }
        }
        Ok(())
    }
}

/// 把控制端送来的键标识映射为 enigo [`Key`]。
///
/// 兼容两路来源，因为两种主控发的 `code` 格式不同：
/// - admin-web 发浏览器 `KeyboardEvent`（`.code` 如 `"KeyA"`/`"Enter"`/`"ShiftLeft"`，
///   或 `.key` 如 `"a"`/`"A"`/`"/"`）；
/// - Slint 客户端（模式 B 主控）发 `ev.text`（可见字符或控制符如 `"\u{8}"`）。
///
/// 旧实现直接取 `code.chars().next()` → `"KeyA"` 注入成 `'K'`、`"Enter"` 注入成 `'E'`，
/// 致键盘全错位（Bug：右键正常但键盘不生效）。这里按整串语义还原。
///
/// 仅使用 enigo **跨平台**变体（信创 Linux + Windows 同源可编）：字母/数字一律走
/// `Key::Unicode`（`Key::A`/`Key::Num0` 在非 Windows 不存在），具名功能键用确认过的跨平台变体。
pub fn code_to_key(code: &str) -> Option<Key> {
    // ① 单字符来源：.key 的可见字符 / Slint ev.text（可能是控制符）
    if code.chars().count() == 1 {
        let c = code.chars().next().unwrap();
        return match c {
            '\r' | '\n' => Some(Key::Return),
            '\t' => Some(Key::Tab),
            '\u{8}' | '\u{7f}' => Some(Key::Backspace),
            '\u{1b}' => Some(Key::Escape),
            // 其余不可见控制符无法稳定注入，忽略
            c if (c as u32) < 0x20 => None,
            c => Some(Key::Unicode(c)),
        };
    }
    // ② 具名键：浏览器 KeyboardEvent.code / .key（两种写法都收）
    match code {
        "Enter" | "NumpadEnter" | "Return" => Some(Key::Return),
        "Backspace" => Some(Key::Backspace),
        "Tab" => Some(Key::Tab),
        "Space" => Some(Key::Space),
        "Escape" | "Esc" => Some(Key::Escape),
        "Delete" | "Del" => Some(Key::Delete),
        "Insert" => Some(Key::Insert),
        "ArrowUp" | "Up" => Some(Key::UpArrow),
        "ArrowDown" | "Down" => Some(Key::DownArrow),
        "ArrowLeft" | "Left" => Some(Key::LeftArrow),
        "ArrowRight" | "Right" => Some(Key::RightArrow),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),
        "ShiftLeft" | "ShiftRight" | "Shift" => Some(Key::Shift),
        "ControlLeft" | "ControlRight" | "Control" => Some(Key::Control),
        "AltLeft" | "AltRight" | "Alt" | "AltGraph" => Some(Key::Alt),
        "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" | "Meta" => Some(Key::Meta),
        _ => code_char_fallback(code),
    }
}

/// `KeyboardEvent.code` 中的字母/数字/标点 → 基准 `Unicode` 字符。
fn code_char_fallback(code: &str) -> Option<Key> {
    // "KeyA".."KeyZ" → 'a'..'z'
    if let Some(rest) = code.strip_prefix("Key") {
        if let [b] = rest.as_bytes() {
            let c = b.to_ascii_lowercase() as char;
            if c.is_ascii_alphabetic() {
                return Some(Key::Unicode(c));
            }
        }
    }
    // "Digit0".."Digit9" / "Numpad0".."Numpad9" → '0'..'9'
    for p in ["Digit", "Numpad"] {
        if let Some(rest) = code.strip_prefix(p) {
            if let [b] = rest.as_bytes() {
                let c = *b as char;
                if c.is_ascii_digit() {
                    return Some(Key::Unicode(c));
                }
            }
        }
    }
    // 标点（KeyboardEvent.code）→ 基准字符（大小写/上档符由对端单独的 Shift 事件在系统层合成）
    let c = match code {
        "Minus" => '-',
        "Equal" => '=',
        "BracketLeft" => '[',
        "BracketRight" => ']',
        "Backslash" => '\\',
        "Semicolon" => ';',
        "Quote" => '\'',
        "Backquote" => '`',
        "Comma" => ',',
        "Period" => '.',
        "Slash" => '/',
        _ => return None,
    };
    Some(Key::Unicode(c))
}

#[cfg(test)]
mod tests {
    use super::{code_to_key, Key};
    use crate::geom::map_frame_to_real;

    #[test]
    fn 键码_浏览器code_字母数字() {
        assert_eq!(code_to_key("KeyA"), Some(Key::Unicode('a')));
        assert_eq!(code_to_key("KeyZ"), Some(Key::Unicode('z')));
        assert_eq!(code_to_key("Digit1"), Some(Key::Unicode('1')));
        assert_eq!(code_to_key("Numpad9"), Some(Key::Unicode('9')));
    }

    #[test]
    fn 键码_具名功能键() {
        assert_eq!(code_to_key("Enter"), Some(Key::Return));
        assert_eq!(code_to_key("Backspace"), Some(Key::Backspace));
        assert_eq!(code_to_key("ArrowUp"), Some(Key::UpArrow));
        assert_eq!(code_to_key("ShiftLeft"), Some(Key::Shift));
        assert_eq!(code_to_key("ControlRight"), Some(Key::Control));
        assert_eq!(code_to_key("Space"), Some(Key::Space));
    }

    #[test]
    fn 键码_单字符与控制符() {
        // admin .key / Slint ev.text 的可见字符
        assert_eq!(code_to_key("a"), Some(Key::Unicode('a')));
        assert_eq!(code_to_key("A"), Some(Key::Unicode('A')));
        assert_eq!(code_to_key("/"), Some(Key::Unicode('/')));
        // Slint ev.text 的控制符
        assert_eq!(code_to_key("\r"), Some(Key::Return));
        assert_eq!(code_to_key("\u{8}"), Some(Key::Backspace));
        assert_eq!(code_to_key("\t"), Some(Key::Tab));
    }

    #[test]
    fn 键码_回归_旧实现会把多字符首字母误当字符() {
        // 旧实现 code.chars().next()：'KeyA'→'K'、'Enter'→'E'。新实现绝不能再退化成这样。
        assert_ne!(code_to_key("KeyA"), Some(Key::Unicode('K')));
        assert_ne!(code_to_key("Enter"), Some(Key::Unicode('E')));
    }

    #[test]
    fn 键码_标点() {
        assert_eq!(code_to_key("Slash"), Some(Key::Unicode('/')));
        assert_eq!(code_to_key("Period"), Some(Key::Unicode('.')));
    }

    // 注入器构造依赖 X11 DISPLAY，CI/无头环境不可用；此处只单测坐标换算逻辑（与 Injector::to_real 同源）。
    #[test]
    fn 注入坐标换算_与geom一致() {
        // frame 1280×720，real 1920×1080：帧内 (640,360) → 真实中心 (960,540)
        assert_eq!(
            map_frame_to_real(640, 360, 1280, 720, 1920, 1080),
            (960, 540)
        );
    }

    #[test]
    fn 注入坐标换算_非16比9() {
        // frame 960×720，real 1600×1200：帧角 → 真实角，不偏
        assert_eq!(
            map_frame_to_real(960, 720, 960, 720, 1600, 1200),
            (1600, 1200)
        );
    }
}
