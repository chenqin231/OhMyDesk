//! 主控采集侧键分类：把 Slint `KeyEvent.text`(+ Ctrl/Alt/Meta 修饰态) 归类为
//! 「作为具名/组合键转发(Key 通道)」/「留给文本通道(edited→Text)」/「吞掉不转发」。
//!
//! 远控中文以「主控端输入法为准」：主控本地 IME 组字后由 TextInput `edited` 上屏成 Text；
//! 具名键(Enter/退格/方向…)与组合键(Ctrl+C)走 Key。二者互斥，避免双发。
//!
//! Slint 具名键 `text` 是固定码点(i-slint-common key_codes.rs)：控制符 <0x20、DEL 0x7f、
//! 私有区 0xE000–0xF8FF。这里按码点归一成被控 `code_to_key` 认得的串。

/// 键分类结果。
#[derive(Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// 作为 `InputEvent::Key` 转发；`String` 为归一后、被控 `code_to_key` 认得的 code。
    Key(String),
    /// 可打印字符：交给 TextInput→`edited`→`InputEvent::Text`，本函数不转发。
    Text,
    /// 未支持的控制/功能键：吃掉不本地编辑，也不转发（防私有区字符被注入成怪字符）。
    Ignore,
}

#[cfg(test)]
mod tests {
    use super::{key_route, KeyRoute};

    fn key(s: &str) -> KeyRoute {
        KeyRoute::Key(s.to_string())
    }

    #[test]
    fn 具名键_归一成被控名字串() {
        assert_eq!(key_route("\u{8}", false, false, false), key("Backspace"));
        assert_eq!(key_route("\u{9}", false, false, false), key("Tab"));
        assert_eq!(key_route("\u{a}", false, false, false), key("Enter"));
        assert_eq!(key_route("\u{d}", false, false, false), key("Enter"));
        assert_eq!(key_route("\u{1b}", false, false, false), key("Escape"));
        assert_eq!(key_route("\u{7f}", false, false, false), key("Delete"));
        assert_eq!(key_route("\u{F700}", false, false, false), key("ArrowUp"));
        assert_eq!(key_route("\u{F701}", false, false, false), key("ArrowDown"));
        assert_eq!(key_route("\u{F702}", false, false, false), key("ArrowLeft"));
        assert_eq!(key_route("\u{F703}", false, false, false), key("ArrowRight"));
        assert_eq!(key_route("\u{F727}", false, false, false), key("Insert"));
        assert_eq!(key_route("\u{F729}", false, false, false), key("Home"));
        assert_eq!(key_route("\u{F72B}", false, false, false), key("End"));
        assert_eq!(key_route("\u{F72C}", false, false, false), key("PageUp"));
        assert_eq!(key_route("\u{F72D}", false, false, false), key("PageDown"));
    }

    #[test]
    fn 修饰键本身_归一_含左右变体() {
        assert_eq!(key_route("\u{10}", false, false, false), key("Shift"));
        assert_eq!(key_route("\u{15}", false, false, false), key("Shift"));
        assert_eq!(key_route("\u{11}", false, false, false), key("Control"));
        assert_eq!(key_route("\u{16}", false, false, false), key("Control"));
        assert_eq!(key_route("\u{12}", false, false, false), key("Alt"));
        assert_eq!(key_route("\u{13}", false, false, false), key("Alt"));
        assert_eq!(key_route("\u{17}", false, false, false), key("Meta"));
        assert_eq!(key_route("\u{18}", false, false, false), key("Meta"));
    }

    #[test]
    fn 组合键_可打印字符加修饰_走key透传原字符() {
        assert_eq!(key_route("c", true, false, false), key("c"));
        assert_eq!(key_route("v", true, false, false), key("v"));
        assert_eq!(key_route("1", true, false, false), key("1"));
        assert_eq!(key_route("a", false, true, false), key("a")); // Alt+a
    }

    #[test]
    fn 可打印无修饰_走文本通道_含上档符与大写与cjk() {
        assert_eq!(key_route("a", false, false, false), KeyRoute::Text);
        assert_eq!(key_route("A", false, false, false), KeyRoute::Text);
        assert_eq!(key_route("@", false, false, false), KeyRoute::Text); // Shift+2 已由 Slint 给成 '@'
        assert_eq!(key_route("%", false, false, false), KeyRoute::Text);
        assert_eq!(key_route(" ", false, false, false), KeyRoute::Text); // 空格
        assert_eq!(key_route("中", false, false, false), KeyRoute::Text); // 防御：CJK 归 Text
    }

    #[test]
    fn 未支持功能键_归ignore_不注入怪字符() {
        assert_eq!(key_route("\u{14}", false, false, false), KeyRoute::Ignore); // CapsLock
        assert_eq!(key_route("\u{F708}", false, false, false), KeyRoute::Ignore); // F5
        assert_eq!(key_route("\u{F708}", true, false, false), KeyRoute::Ignore); // Ctrl+F5 也 Ignore
    }
}

/// 把 Slint 具名键 `text` 映射为被控 `code_to_key` 认得的名字串；非具名键返回 None。
fn slint_named_key(text: &str) -> Option<&'static str> {
    let mut it = text.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None; // 多字符不是 Slint 具名键常量
    }
    Some(match c {
        '\u{8}' => "Backspace",
        '\u{9}' => "Tab",
        '\u{a}' | '\u{d}' => "Enter",
        '\u{1b}' => "Escape",
        '\u{7f}' => "Delete",
        '\u{10}' | '\u{15}' => "Shift",   // Shift / ShiftR
        '\u{11}' | '\u{16}' => "Control",  // Control / ControlR
        '\u{12}' | '\u{13}' => "Alt",      // Alt / AltGr
        '\u{17}' | '\u{18}' => "Meta",     // Meta / MetaR
        '\u{F700}' => "ArrowUp",
        '\u{F701}' => "ArrowDown",
        '\u{F702}' => "ArrowLeft",
        '\u{F703}' => "ArrowRight",
        '\u{F727}' => "Insert",
        '\u{F729}' => "Home",
        '\u{F72B}' => "End",
        '\u{F72C}' => "PageUp",
        '\u{F72D}' => "PageDown",
        _ => return None,
    })
}

/// 单字符且为控制符/DEL/私有区 → 非文本键（不可塞进文本缓冲）。
fn is_nontext_char(text: &str) -> bool {
    let mut it = text.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => {
            let u = c as u32;
            u < 0x20 || u == 0x7f || (0xE000..=0xF8FF).contains(&u)
        }
        _ => false,
    }
}

/// 采集侧键路由。`ctrl/alt/meta` = 事件发生时的非 Shift 修饰态（Slint `ev.modifiers`）。
pub fn key_route(text: &str, ctrl: bool, alt: bool, meta: bool) -> KeyRoute {
    // ① 具名键（含 Ctrl+Enter 这类；组合态由被控 mods_held 处理）
    if let Some(name) = slint_named_key(text) {
        return KeyRoute::Key(name.to_string());
    }
    // ② 未列名的控制符/私有区键（CapsLock/F 键/ScrollLock…）：吃掉不注入，防怪字符
    if is_nontext_char(text) {
        return KeyRoute::Ignore;
    }
    // ③ 组合键：可打印字符 + Ctrl/Alt/Meta → 走 Key，被控 VK 路径与修饰键合成（Ctrl+C 等）
    if ctrl || alt || meta {
        return KeyRoute::Key(text.to_string());
    }
    // ④ 纯可打印字符、无非 Shift 修饰 → 文本通道（上档符/大写/CJK 上屏均由 edited 出）
    KeyRoute::Text
}
