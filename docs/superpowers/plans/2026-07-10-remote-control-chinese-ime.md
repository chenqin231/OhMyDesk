# 远控中文输入（主控端输入法为准）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Slint 原生客户端「主控→被控」远控时能输入中文，方式为「主控端输入法为准」——主控本地 IME 组字上屏成文本发给被控。

**Architecture:** 主控用 IME-capable 的 `TextInput` 接收器替换原 0px `FocusScope`。`TextInput` 获焦时才开 IME，其 `edited` 回调拿到上屏串（含中文/英文/上档符）走 `InputEvent::Text` 通道；具名键（Enter/退格/方向）与组合键（Ctrl+C）经 `key-pressed` 走 `InputEvent::Key` 通道。二者互斥，靠一个纯 Rust 分类器 `key_route` 判定，避免双发/漏发。注入端零改（`InputEvent::Text` 已走 `enigo.text()`）。

**Tech Stack:** Rust 2021 + Slint 1.17（`TextInput`/`KeyEvent.modifiers`/`preedit-text`）、enigo 0.6、protocol crate（`InputEvent::{Key,Text}`）。

---

## 背景事实（已核实，勿重新调研）

- **根因 1（采集吃字）**：`FocusScope` 不是文本编辑器，winit 不对它 `set_ime_allowed(true)`，IME 组合/上屏事件根本不进来 → 主控采集不到中文。
- **根因 2（注入绕 IME）**：被控 `enigo.text()` 走 `KEYEVENTF_UNICODE`，不经键盘管线，被控 IME 看不到按键 → 被控端为准不可行。故选主控端为准。
- **Slint 1.17 已证实的 API**（`~/.cargo/registry/.../i-slint-compiler-1.17.0/builtins.slint`、`i-slint-common-1.17.0/{builtin_structs,key_codes}.rs`）：
  - `TextInput` 有 `callback edited;`（读 `self.text`）、`callback key_pressed(event:KeyEvent)->EventResult`（在 TextInput 处理前触发，返回 `accept` 吃掉/`reject` 交它编辑）、`out property <string> preedit-text;`（组字期非空）、`text` 读写、`text-cursor-width`、`single-line`、`focus()`。
  - `KeyEvent` 字段：`text`(SharedString)、`modifiers`(`.control/.alt/.shift/.meta` 均 bool)、`repeat`。
  - Slint 具名键 `text` 固定码点（关键值）：Backspace `\u{8}`、Tab `\u{9}`、Return `\u{a}`、Escape `\u{1b}`、Delete `\u{7f}`、Shift `\u{10}`、Control `\u{11}`、Alt `\u{12}`、AltGr `\u{13}`、CapsLock `\u{14}`、ShiftR `\u{15}`、ControlR `\u{16}`、Meta `\u{17}`、MetaR `\u{18}`、Space `\u{20}`、Arrows `\u{F700}`–`\u{F703}`、F1–F24 `\u{F704}`–`\u{F71B}`、Insert `\u{F727}`、Home `\u{F729}`、End `\u{F72B}`、PageUp `\u{F72C}`、PageDown `\u{F72D}`。私有区范围 `0xE000..=0xF8FF`。
- **被控 `code_to_key`（`src/client/src/inject.rs`）已认得的名字串**：`Enter`/`Backspace`/`Tab`/`Space`/`Escape`/`Delete`/`Insert`/`ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`/`Home`/`End`/`PageUp`/`PageDown`/`Shift`/`Control`/`Alt`/`Meta`。**不认得 F 键**（会落 `Key::Unicode` 兜底 → 私有区字符注入怪字符）。故 F 键与其它未支持功能键在主控侧归为 `Ignore`，不转发。

---

## 文件结构

- **新建** `src/client/src/key_route.rs` — 纯函数键分类器 `key_route()` + `KeyRoute` 枚举，附单元测试（CI 可跑，无需 Windows）。
- **修改** `src/client/src/main.rs` — 加 `mod key_route;`。
- **修改** `src/client/ui/app.slint` — `FocusScope` → `TextInput` IME 接收器；换回调声明为 `on_key_ev`(带修饰态、返回 bool) + `on_text`；删 `on_key`/`key_code`。
- **修改** `src/client/src/ui_glue.rs` — 绑定 `on_on_key_ev`（调 `key_route` 发 Key）+ `on_on_text`（发 Text）；删旧 `on_on_key` 绑定。
- **不改** `src/client/src/inject.rs` — `InputEvent::Text` 已正确。

---

## Task 1: 主控采集键分类器（纯 Rust，TDD，CI 可测）

**Files:**
- Create: `src/client/src/key_route.rs`
- Modify: `src/client/src/main.rs:27`（加 `mod key_route;`）

- [ ] **Step 1: 写失败测试**

在 `src/client/src/key_route.rs` 写（先只放测试与占位 API，让它编译失败）：

```rust
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

pub fn key_route(_text: &str, _ctrl: bool, _alt: bool, _meta: bool) -> KeyRoute {
    KeyRoute::Text // 占位，待实现
}
```

在 `src/client/src/main.rs` 第 27 行 `mod inject;` 之后加一行：

```rust
mod key_route;
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p client key_route`
Expected: FAIL（多条 `assertion failed`，如 `具名键_归一成被控名字串` 断言不等）。

- [ ] **Step 3: 写最小实现**

用真实实现替换 `key_route.rs` 中的占位 `key_route`（保留上方枚举与测试）：

```rust
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p client key_route`
Expected: PASS（5 个测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add src/client/src/key_route.rs src/client/src/main.rs
git commit -m "feat(client): 主控采集键分类器 key_route（具名/组合/文本/忽略四路）"
```

---

## Task 2: Slint UI — TextInput IME 接收器替换 FocusScope

**Files:**
- Modify: `src/client/ui/app.slint`（回调声明约 765；`key_code` 函数 770-776；`FocusScope` 1349-1359）

- [ ] **Step 1: 换回调声明**

把 `src/client/ui/app.slint` 第 765 行：

```slint
    callback on_key(string /*code*/, bool /*down*/);
```

替换为：

```slint
    // 采集侧键盘：on_key_ev 返回 true=已作为 Key 事件发出(Slint 应 accept 吃掉)，
    // false=可打印字符(reject 交 TextInput→edited→Text)。on_text=IME/文本上屏串。
    callback on_key_ev(string /*text*/, bool /*ctrl*/, bool /*alt*/, bool /*meta*/, bool /*down*/) -> bool;
    callback on_text(string /*上屏串*/);
```

- [ ] **Step 2: 删除已无用的 `key_code` 纯函数**

删除 `src/client/ui/app.slint` 第 767-776 行整段（注释 + `pure function key_code(...) {...}`）——其唯一调用点是即将替换的 `FocusScope`，归一逻辑已移入 Rust `key_route`。

- [ ] **Step 3: FocusScope → TextInput 接收器**

把 `src/client/ui/app.slint` 第 1349-1359 行的：

```slint
                    // 键盘焦点捕获
                    fs := FocusScope {
                        width: 0px;
                        key-pressed(ev) => {
                            root.on_key(root.key_code(ev.text), true);
                            accept
                        }
                        key-released(ev) => {
                            root.on_key(root.key_code(ev.text), false);
                            accept
                        }
                    }
```

替换为：

```slint
                    // 键盘焦点 + IME 上屏接收器（主控端输入法为准）。
                    // TextInput 获焦才开 IME；组字期让位 IME；具名/组合键走 on_key_ev→Key，
                    // 可打印字符/中文上屏走 edited→Text。底部 2px 透明条给 OS 候选框一个锚点。
                    fs := TextInput {
                        x: parent.frame_display_x;
                        y: parent.frame_display_y + parent.frame_display_h - 2px;
                        width: parent.frame_display_w;
                        height: 2px;
                        color: transparent;
                        text-cursor-width: 0px;
                        single-line: true;
                        key-pressed(ev) => {
                            // 组字中：拼音/退格/空格/回车全交 IME，不转发、不吃
                            if (self.preedit-text != "") {
                                return reject;
                            }
                            return root.on_key_ev(ev.text, ev.modifiers.control, ev.modifiers.alt, ev.modifiers.meta, true)
                                ? EventResult.accept : EventResult.reject;
                        }
                        key-released(ev) => {
                            if (self.preedit-text != "") {
                                return reject;
                            }
                            return root.on_key_ev(ev.text, ev.modifiers.control, ev.modifiers.alt, ev.modifiers.meta, false)
                                ? EventResult.accept : EventResult.reject;
                        }
                        edited => {
                            if (self.text != "") {
                                root.on_text(self.text);
                                self.text = "";  // 清空，永不累积；programmatic 赋值不触发 edited
                            }
                        }
                    }
```

> 说明：`fs.focus()` 的两处调用（第 1241 行 `init`、第 1319 行点画面兜底）保持有效——元素名仍是 `fs`，`TextInput` 同样有 `focus()`。z 序上 `fs` 位于 `ta := TouchArea`（1305）之后即在其上层，但仅占底部 2px；若真机发现该 2px 抢走点击，把整个 `fs := TextInput {...}` 块移到 `ta := TouchArea` 之前（同一父级，`fs.focus()` 仍解析）。

- [ ] **Step 4: 编译（Slint 语法门）**

Run: `cargo build -p client`
Expected: 编译通过。若报 `EventResult` 相关错误，确认返回写法与本文件既有用法一致（本项目 `scroll-event` 用 `EventResult.accept`，见 app.slint:1332）。

- [ ] **Step 5: 提交**

```bash
git add src/client/ui/app.slint
git commit -m "feat(client): 远控键盘焦点换成 IME TextInput 接收器（双通道采集）"
```

---

## Task 3: ui_glue.rs 绑定 on_key_ev / on_text

**Files:**
- Modify: `src/client/src/ui_glue.rs:337-356`（原 `on_on_key` 块）

- [ ] **Step 1: 替换绑定**

把 `src/client/src/ui_glue.rs` 第 337-356 行整段（`{ let tx=...; ui.on_on_key(...) }`）替换为：

```rust
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        // 采集侧键盘：分类器决定走 Key 通道还是交 TextInput 出 Text。
        // 返回 true → Slint accept（吃掉不本地编辑）；false → reject（交 edited→on_text）。
        ui.on_on_key_ev(move |text, ctrl, alt, meta, down| -> bool {
            match crate::key_route::key_route(&text, ctrl, alt, meta) {
                crate::key_route::KeyRoute::Key(code) => {
                    let sid = sess.lock().unwrap().clone();
                    tracing::info!(
                        "主控采集·键 code={code:?} down={down} session={}",
                        sid.as_deref().unwrap_or("<无>")
                    );
                    if let Some(sid) = sid {
                        let _ = tx.send(net::FromUi::Input {
                            session_id: sid,
                            event: protocol::InputEvent::Key { code, down },
                        });
                    }
                    true // 已作为 Key 发出，吃掉本地编辑
                }
                // 吃掉但不转发（未支持功能键，防注入怪字符）
                crate::key_route::KeyRoute::Ignore => true,
                // 可打印字符：交给 TextInput 编辑，稍后 edited→on_text 出 Text
                crate::key_route::KeyRoute::Text => false,
            }
        });
    }
    {
        let tx = from_ui_tx.clone();
        let sess = cur_session.clone();
        // IME/文本上屏串 → InputEvent::Text（被控 enigo.text() Unicode 直塞）
        ui.on_on_text(move |text| {
            if text.is_empty() {
                return;
            }
            let sid = sess.lock().unwrap().clone();
            tracing::info!(
                "主控采集·文本 len={} session={}",
                text.len(),
                sid.as_deref().unwrap_or("<无>")
            );
            if let Some(sid) = sid {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::Text {
                        text: text.to_string(),
                    },
                });
            }
        });
    }
```

> 注：`from_ui_tx`、`cur_session`、`net::FromUi::Input` 均沿用原 `on_on_key` 块的同名绑定（见替换前上下文），此处两个 `{}` 作用域各自 `clone` 一份，模式与文件内其它回调一致。

- [ ] **Step 2: 编译 + clippy**

Run: `cargo build -p client && cargo clippy -p client -- -D warnings`
Expected: 通过，无告警。若报 `InputEvent::Text` 字段名不符，核对 `src/client/src/inject.rs:106`（`InputEvent::Text { text }`）确认字段为 `text`。

- [ ] **Step 3: 提交**

```bash
git add src/client/src/ui_glue.rs
git commit -m "feat(client): ui_glue 绑定 on_key_ev/on_text，接采集双通道"
```

---

## Task 4: 全量测试 + 静态门

- [ ] **Step 1: 单元测试**

Run: `cargo test -p client`
Expected: PASS——含新 `key_route` 5 测 + 既有 `inject` 键码/坐标测试，全绿。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p client -- -D warnings && cargo fmt -p client -- --check`
Expected: 无告警、格式合规（如 `fmt --check` 失败则 `cargo fmt -p client` 后重提交）。

- [ ] **Step 3: 提交（若 fmt 有改动）**

```bash
git add -A src/client && git commit -m "chore(client): fmt" || true
```

---

## Task 5: Windows 真机验收门（R1，用户执行）— ⛔ STOP GATE

> Linux 开发机验不了 Slint IME。此门必须在**双 Windows + 搜狗**真机跑。前置 Task 1–4 全绿后，构建 Windows 客户端并按下表逐条验证。**任一「四行为」项失败 → 立即 STOP，不要继续打磨，回到设计层评估备选（经 `i-slint-backend-winit` 直取 winit `Ime` 事件），并知会用户。**

- [ ] **Step 1: 构建 Windows 客户端**（按项目既有 Windows 构建流程，见 release skill / dist/windows）

- [ ] **Step 2: 验「四行为」（阻断级）**
  - [ ] ① TextInput 底部 2px 透明获焦后，搜狗切中文态能弹候选框（IME 已开）。
  - [ ] ② 打「nihao」选词「你好」→ 被控端出现「你好」（`edited`→Text→注入成功）。
  - [ ] ③ 组字期间按退格/空格/回车只作用于 IME 组字，不误发到被控（`preedit-text` 门生效）。
  - [ ] ④ 具名键路径正常：无组字态下 Enter/退格/方向作用于被控。

- [ ] **Step 3: 回归验证（不得因双通道拆分退化）**
  - [ ] 英文直打：abc… 正常上屏被控。
  - [ ] 上档符：`%` `@` `^` `!` 正常（走 Text，主控 IME 英文态直接给上档符）。
  - [ ] 组合键：Ctrl+C / Ctrl+V / Ctrl+A 在被控生效。
  - [ ] Enter / Backspace / 方向键在被控生效。
  - [ ] 大写：Shift+字母 出大写。

- [ ] **Step 4: 候选框位置（R2）**
  - [ ] 搜狗候选框弹在可视区（可接受即通过）；若跑到不可见处，调整 `fs` 的 y/height 或参照 Task 2 Step 3 的 z 序备注。

- [ ] **Step 5: 验收通过后合并**

```bash
git checkout master && git merge --no-ff feature/remote-control-chinese-ime
```

> 发布走 `release` skill；本计划到「合并」为止，发版另议。

---

## 自查（Self-Review 已过）

- **spec 覆盖**：spec §4 接收器→Task 2；§5 双通道→Task 1(分类)+Task 2/3(接线)；§6 改动范围→Task 1–3；§7 测试→Task 1(单测)+Task 4+Task 5;§8 R1/R2→Task 5;§9 非目标(admin-web/被控 IME/低层钩子)→未触及。
- **占位符**：无 TBD/TODO；每步含真实代码/命令/预期。
- **类型一致**：`KeyRoute::{Key,Text,Ignore}`、`key_route(text,ctrl,alt,meta)`、回调 `on_key_ev`/`on_text`、`InputEvent::{Key{code,down},Text{text}}` 全计划一致。
- **已知取舍**（非回归）：F 键/CapsLock 等未支持功能键从「注入私有区怪字符」改为「Ignore 不注入」——严格优于现状；F 键端到端支持列为未来另议。
