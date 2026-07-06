//! 键鼠注入：收 [`InputEvent`] → enigo 注入，坐标按 frame→real 还原（P-CLI4）。
//!
//! 坑点（见 references/xcap-enigo.md）：
//! - 必须 `use enigo::{Mouse, Keyboard}`，否则 trait 方法不可见。
//! - 0.6：`Enigo::new(&Settings)` 返回 Result；move_mouse/button/key/text 全返回 Result；
//!   字符键用 `Key::Unicode(char)`；`release_keys_when_dropped` 默认 true 防卡键。
//! - 坐标换算用 crate::geom::map_frame_to_real，逻辑可单测（注入本身依赖 X11 不强测）。

use crate::geom::map_frame_to_real;
use enigo::{
    Axis, Button,
    Coordinate::Abs,
    Direction::{Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use protocol::InputEvent;

/// 修饰键位掩码（用于组合键判定，仅含会改变按键语义的非 Shift 修饰键）。
const MOD_CTRL: u8 = 1;
const MOD_ALT: u8 = 2;
const MOD_META: u8 = 4;

/// 键鼠注入器：持有 enigo + 被控真实屏尺寸（坐标换算的帧尺寸按当前画质档位实时派生）。
pub struct Injector {
    enigo: Enigo,
    real_w: u32,
    real_h: u32,
    /// 当前按住的非 Shift 修饰键（Ctrl/Alt/Meta）位掩码，从事件流推断，用于组合键注入。
    mods_held: u8,
    /// 最近一次鼠标移动的坐标映射快照 (帧内x, 帧内y, 帧w, 帧h, 注入rx, 注入ry)，
    /// 点击(MouseButton down)时打诊断日志用——定位「点击错位」到底错在哪一段。
    last_click_dbg: (i32, i32, u32, u32, i32, i32),
}

impl Injector {
    /// `real_*` = 被控真实屏。帧尺寸不再构造时固定，而是每次注入按当前画质档位实时取
    /// （见 [`to_real`]），保证主控切换高清/流畅后点击坐标不偏。
    pub fn new(real_w: u32, real_h: u32) -> anyhow::Result<Self> {
        Ok(Injector {
            enigo: Enigo::new(&Settings::default())?,
            real_w,
            real_h,
            mods_held: 0,
            last_click_dbg: (0, 0, 0, 0, 0, 0),
        })
    }

    /// 注入一个输入事件。
    pub fn apply(&mut self, ev: &InputEvent) -> anyhow::Result<()> {
        match ev {
            InputEvent::MouseMove { x, y } => {
                // 帧内坐标 → 真实屏坐标。帧尺寸优先取「最近实际发出帧」的真实尺寸（含 adaptive
                // 降档结果），尚无帧发出时回退当前档位标称尺寸。记录映射快照供点击诊断日志。
                let (frame_w, frame_h) = crate::capture::last_frame_dims().unwrap_or_else(|| {
                    crate::capture::current_frame_dims(self.real_w, self.real_h)
                });
                let (rx, ry) =
                    map_frame_to_real(*x, *y, frame_w, frame_h, self.real_w, self.real_h);
                self.last_click_dbg = (*x, *y, frame_w, frame_h, rx, ry);
                self.enigo.move_mouse(rx, ry, Abs)?;
            }
            InputEvent::MouseButton { button, down } => {
                // 点击坐标映射诊断（debug 级：默认不进日志，避免逐次点击的隐私/噪音；
                // 需排查点击错位时用 RUST_LOG=client=debug 开启）。坐标映射已实证正确。
                if *down {
                    let (ix, iy, fw, fh, rx, ry) = self.last_click_dbg;
                    tracing::debug!(
                        "点击注入诊断 收到帧内=({ix},{iy}) 帧尺寸={fw}x{fh} 真实屏={}x{} → 注入=({rx},{ry})",
                        self.real_w,
                        self.real_h
                    );
                }
                let b = match button {
                    0 => Button::Left,
                    1 => Button::Middle,
                    _ => Button::Right,
                };
                self.enigo.button(b, if *down { Press } else { Release })?;
            }
            InputEvent::Key { code, down } => {
                // 先更新修饰键按下态（Ctrl/Alt/Meta），供组合键判定。
                if let Some(m) = modifier_bit(code) {
                    if *down {
                        self.mods_held |= m;
                    } else {
                        self.mods_held &= !m;
                    }
                }
                // 可打印字符 + 无 Ctrl/Alt/Meta 组合：走 enigo.text() 可靠注入 Unicode。
                // 主控已把 Shift 上档符解析成最终字符（如 Shift+6→"%"、Shift+2→"@"），text() 用
                // KEYEVENTF_UNICODE 直输该字符、与键盘修饰态无关，避免 key(Key::Unicode) 对部分
                // 符号/应用注入不稳导致「上档符打不出」。仅按下时落字一次，松开无需处理。
                // 组合键（Ctrl+C 等）与具名键（Enter/Tab/方向键…）仍走下方 key() 路径。
                if self.mods_held == 0 {
                    if let Some(c) = printable_char(code) {
                        if *down {
                            self.enigo.text(&c.to_string())?;
                        }
                        return Ok(());
                    }
                }
                if let Some(key) = resolve_key_with_mods(code, self.mods_held) {
                    self.enigo.key(key, if *down { Press } else { Release })?;
                }
            }
            InputEvent::Text { text } => {
                self.enigo.text(text)?;
            }
            InputEvent::Scroll { dx, dy } => {
                // 符号已逐层核实:winit/Slint delta-y>0=向上 → 协议 dy>0=向上 → enigo 负=向上,
                // 故 SCROLL_SIGN=-1(enigo length>0=向下)。如某平台实测相反,只翻此常量。
                const SCROLL_SIGN: i32 = -1;
                tracing::debug!("被控注入·滚轮 dx={dx} dy={dy}");
                if *dy != 0 {
                    self.enigo.scroll(SCROLL_SIGN * *dy, Axis::Vertical)?;
                }
                if *dx != 0 {
                    self.enigo.scroll(SCROLL_SIGN * *dx, Axis::Horizontal)?;
                }
            }
            // 未知/未来变体(本端 protocol 不认识):无法注入,忽略。见 protocol InputEvent::Unknown。
            InputEvent::Unknown => {}
        }
        Ok(())
    }
}

/// 单个可打印字符（用于 `text()` 直输路径）。多字符（具名键如 "Enter"）或控制符
/// （`< 0x20` / DEL）返回 None——它们交给 [`code_to_key`] 走 enigo `key()` 注入。
fn printable_char(code: &str) -> Option<char> {
    let mut it = code.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None; // 多字符 = 具名键
    }
    if (c as u32) >= 0x20 && c != '\u{7f}' {
        Some(c)
    } else {
        None
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
        // enigo 的 Key::Insert 在 macOS 不存在（mac 键盘无 Insert 键），仅非 macOS 映射；
        // macOS 上 "Insert" 落入下方兜底逻辑（忽略，不注入）。
        #[cfg(not(target_os = "macos"))]
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

/// 把修饰键标识映射为 [`MOD_CTRL`]/[`MOD_ALT`]/[`MOD_META`] 位。Shift 不计入：
/// Shift+字母经 `.key` 已解析成大写/上档字符，Unicode 注入即正确，无需扫描码组合。
fn modifier_bit(code: &str) -> Option<u8> {
    match code {
        "ControlLeft" | "ControlRight" | "Control" => Some(MOD_CTRL),
        "AltLeft" | "AltRight" | "Alt" | "AltGraph" => Some(MOD_ALT),
        "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" | "Meta" => Some(MOD_META),
        _ => None,
    }
}

/// 在 [`code_to_key`] 基础上叠加「组合键」修正。
///
/// 根因（Windows）：字母/数字默认走 `Key::Unicode`，其底层 `KEYEVENTF_UNICODE` 直接注入字符、
/// **绕过键盘修饰键状态**，故 Ctrl+C / Ctrl+V 之类组合键失效（只输入字面 'c'/'v'）。
/// 修正：当 Ctrl/Alt/Meta 处于按下态且按键是单个字母/数字时，改用 Windows VK 扫描码变体
/// （`Key::A`/`Key::Num1`…），让 OS 把修饰键与该键组合。
///
/// 非 Windows（信创 X11）：`Key::Unicode` 在 X 服务层本就与真实按下的修饰键组合，无此问题，
/// 故保持原映射不变（`mods_held` 不参与）。
pub fn resolve_key_with_mods(code: &str, mods_held: u8) -> Option<Key> {
    #[cfg(target_os = "windows")]
    {
        if mods_held != 0 && code.chars().count() == 1 {
            let c = code.chars().next().unwrap();
            if c.is_ascii_alphanumeric() {
                if let Some(k) = vk_key_for_char(c) {
                    return Some(k);
                }
            }
        }
    }
    let _ = mods_held; // 非 Windows 不使用，避免未用告警
    code_to_key(code)
}

/// 字母/数字 → Windows VK 扫描码 `Key` 变体（这些变体仅在 Windows 存在，故 cfg 门控）。
#[cfg(target_os = "windows")]
fn vk_key_for_char(c: char) -> Option<Key> {
    let key = match c.to_ascii_lowercase() {
        'a' => Key::A,
        'b' => Key::B,
        'c' => Key::C,
        'd' => Key::D,
        'e' => Key::E,
        'f' => Key::F,
        'g' => Key::G,
        'h' => Key::H,
        'i' => Key::I,
        'j' => Key::J,
        'k' => Key::K,
        'l' => Key::L,
        'm' => Key::M,
        'n' => Key::N,
        'o' => Key::O,
        'p' => Key::P,
        'q' => Key::Q,
        'r' => Key::R,
        's' => Key::S,
        't' => Key::T,
        'u' => Key::U,
        'v' => Key::V,
        'w' => Key::W,
        'x' => Key::X,
        'y' => Key::Y,
        'z' => Key::Z,
        '0' => Key::Num0,
        '1' => Key::Num1,
        '2' => Key::Num2,
        '3' => Key::Num3,
        '4' => Key::Num4,
        '5' => Key::Num5,
        '6' => Key::Num6,
        '7' => Key::Num7,
        '8' => Key::Num8,
        '9' => Key::Num9,
        _ => return None,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::{
        code_to_key, modifier_bit, resolve_key_with_mods, Key, MOD_ALT, MOD_CTRL, MOD_META,
    };
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

    #[test]
    fn 可打印字符_识别上档符与普通字符_排除具名键和控制符() {
        use super::printable_char;
        // 上档符（主控已解析）：走 text() 直输路径
        assert_eq!(printable_char("%"), Some('%'));
        assert_eq!(printable_char("@"), Some('@'));
        assert_eq!(printable_char("^"), Some('^'));
        assert_eq!(printable_char("a"), Some('a'));
        assert_eq!(printable_char("A"), Some('A'));
        assert_eq!(printable_char("中"), Some('中'));
        // 具名键（多字符）→ None，交给 key() 路径
        assert_eq!(printable_char("Enter"), None);
        assert_eq!(printable_char("ArrowUp"), None);
        // 控制符（Slint 修饰键/控制字符）→ None
        assert_eq!(printable_char("\u{15}"), None); // 右 Shift 残留控制符
        assert_eq!(printable_char("\r"), None);
        assert_eq!(printable_char(""), None);
    }

    #[test]
    fn 修饰键_位识别() {
        assert_eq!(modifier_bit("Control"), Some(MOD_CTRL));
        assert_eq!(modifier_bit("ControlLeft"), Some(MOD_CTRL));
        assert_eq!(modifier_bit("AltGraph"), Some(MOD_ALT));
        assert_eq!(modifier_bit("Meta"), Some(MOD_META));
        // 非修饰键 / Shift（Shift 不计入组合判定）返回 None
        assert_eq!(modifier_bit("ShiftLeft"), None);
        assert_eq!(modifier_bit("KeyA"), None);
        assert_eq!(modifier_bit("c"), None);
    }

    // Windows：组合键（Ctrl/Alt/Meta 按下）下字母/数字走 VK 扫描码，与修饰键组合；
    // 无修饰键时仍走 Unicode（跨布局/符号正确）。
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_组合键走扫描码() {
        assert_eq!(resolve_key_with_mods("c", MOD_CTRL), Some(Key::C));
        assert_eq!(resolve_key_with_mods("v", MOD_CTRL), Some(Key::V));
        assert_eq!(resolve_key_with_mods("1", MOD_CTRL), Some(Key::Num1));
        // 无修饰键：仍是 Unicode
        assert_eq!(resolve_key_with_mods("c", 0), Some(Key::Unicode('c')));
        // 具名键不受影响
        assert_eq!(
            resolve_key_with_mods("Backspace", MOD_CTRL),
            Some(Key::Backspace)
        );
    }

    // 非 Windows（信创 X11）：Unicode 本就与修饰键组合，mods_held 不改变解析结果。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn 非windows_组合键不改变映射() {
        assert_eq!(
            resolve_key_with_mods("c", MOD_CTRL),
            Some(Key::Unicode('c'))
        );
        assert_eq!(resolve_key_with_mods("c", 0), Some(Key::Unicode('c')));
        assert_eq!(
            resolve_key_with_mods("Backspace", MOD_CTRL),
            Some(Key::Backspace)
        );
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
