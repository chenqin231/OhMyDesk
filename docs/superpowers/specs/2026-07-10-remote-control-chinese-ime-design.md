# 远控中文输入（主控端输入法为准）— 设计文档

> 日期：2026-07-10
> 状态：已批准设计，待写实现计划
> 范围：Slint 原生客户端「主控 → 被控」远控场景的键盘输入链路

## 1. 问题

原生客户端远控时无法输入中文（双方 Windows + 搜狗实测）。两处独立缺陷，缺一不可：

1. **采集端吃字**：主控采集用 Slint `FocusScope.key-pressed`，只转发 `ev.text` 单键文本。`FocusScope` 不是文本编辑器，winit 不会对它 `set_ime_allowed(true)`，IME 组合/上屏事件根本不进来；主控搜狗组出的「你好」永不触发 `key-pressed` → 主控采集不到中文。
2. **注入端绕过 IME**：被控 `enigo.text()` 走 `KEYEVENTF_UNICODE`，直接塞 Unicode 字符、不经被控键盘管线，被控搜狗永远看不到按键 → 「被控端输入法为准」在当前架构物理上不可能。

## 2. 决策：主控端输入法为准（Design A）

三平台（Windows / macOS / 国产 Linux）统一走「主控端输入法为准」，理由按重要性：

1. **采集跨平台统一**：只需拿「IME 上屏串」。winit `Ime::Commit` / Slint `TextInput` 上屏在三平台一套代码。被控端为准需主控发原始扫描码 → 必须绕过主控自身 IME → 需各平台低层键盘钩子（Win `WH_KEYBOARD_LL` / mac `CGEventTap` 要辅助功能授权 / Linux X grab），三套实现、权限重、易碎。
2. **注入跨平台统一**：主控端为准注入走 Unicode 直塞，三平台可靠，且**被控端有无中文输入法都不影响**。被控端为准靠合成扫描码触发被控 IME：Linux Fcitx/IBus 对合成事件（send_event）处理时灵时不灵，macOS 有 secure input 拦截 → 不稳。
3. **国产系统 IME 太杂**：被控可能是 Fcitx5 拼音/五笔/搜狗 Linux，发扫描码赌远端正确组字不现实。人坐在主控前，用主控 IME 组字最贴合真实。
4. **延迟**：主控端为准候选框本地弹=零延迟；被控端为准候选框在被控端弹，得透过本项目已知偏卡的视频看拼音，体验差。

**唯一代价**：被控端有主控没有的专用 IME（如五笔肌肉记忆）。但人物理在主控，肌肉记忆和输入法本就在主控 → 不成立。

## 3. 数据流

```
主控本地搜狗组字「你好」（候选框本地弹，零延迟）
  → IME 上屏 → InputEvent::Text{"你好"} → 网络 → 被控 enigo.text() Unicode 直塞
控制键（Enter/退格/方向/Esc/F键）→ InputEvent::Key → 被控 enigo.key() 扫描码/VK
组合键（Ctrl/Alt/Meta + X）    → InputEvent::Key → 被控 enigo.key()
```

## 4. 核心机制：IME 上屏串接收器

用 IME-capable 的 Slint `TextInput` 替换主控远控画面里 0px 的 `FocusScope` 作为键盘焦点持有者。

- **为何 TextInput**：`TextInput` 获焦时内部 `set_ime_allowed(true)`，才收得到组合/上屏事件——这正是 `FocusScope` 拿不到中文的根因。
- **接收器语义**：TextInput 只作 IME sink，不作真实编辑框。`edited` 回调触发 → 取新增文本 → 发 `InputEvent::Text` → 立即把 `text` 清空为 `""`（永不累积、本地不留字、退格无字可删）。
- **候选框位置**：TextInput 尺寸/位置需保证 OS 候选框弹在可视区（不能真的 0px 塞角落，否则搜狗浮窗跑到屏角）。实现期确定具体摆放（倾向远控画面底部一条不可见但有真实几何的输入位）。

## 5. 双通道拆分（防重复发送）

| 键类 | 通道 | 处理 |
|---|---|---|
| 可打印字符 + 中文上屏（ASCII/CJK/符号） | `edited` → `InputEvent::Text` | 被控 `enigo.text()` Unicode 直塞 |
| 具名/控制键（Enter/Tab/Backspace/Delete/方向/Home/End/PageUp/Down/Esc/F 键） | `key-pressed`/`key-released` → `InputEvent::Key`，回调返回 `accept` 吃掉、阻止本地编辑 | 被控 `enigo.key()` |
| 修饰组合键（Ctrl/Alt/Meta + 某键） | `key-pressed` → `InputEvent::Key` | 被控 `enigo.key()`（已有 `mods_held` VK 扫描码路径） |

**为何不冲突**：IME 组字期间，OS 把拼音/退格/空格/回车全喂给 IME，Slint `key-pressed` 不触发 → 组字键不会被误当控制键转发。只有「无组字态」的退格/回车才进 `key-pressed` 正常转发。空格/回车在组字态用于选词/上屏 → 走 `edited`→`Text`；无组字态才作控制键。

## 6. 改动范围

1. **`src/client/ui/app.slint`**：远控画面 `FocusScope`（约 1349 行）→ IME `TextInput` 接收器；新增 `on_text(string)` 回调；`key-pressed`/`key-released` 过滤为「只发具名/组合键」，可打印字符 reject 交给 `edited`。
2. **`src/client/src/ui_glue.rs`**：新增 `on_on_text` 处理器 → 发 `protocol::InputEvent::Text { text }`；`on_on_key` 保持（仍发具名/组合键）。
3. **注入端 `src/client/src/inject.rs`**：**零改**——`InputEvent::Text` 已走 `enigo.text()` 正确。
4. **`admin-web`（浏览器主控）**：本次不动，另论（浏览器有独立 IME 组合事件模型 `compositionend`，单开一张 spec）。

## 7. 测试策略

- **单测（Rust，可在 CI/Linux 跑）**：
  - key 分类纯函数（具名/组合键 vs 可打印字符）——判定哪些进 `Key` 通道、哪些让位给 `Text`。
  - Text 增量提取逻辑（从 TextInput `edited` 新值取新增串、清空语义）如落在 Rust 侧则单测。
- **真机验证（双 Windows + 搜狗，必须）**：Slint 1.17 `TextInput` 四行为——①隐形/低干扰获焦能否开 IME ②上屏触发 `edited` ③组字期是否吞 `key-pressed` ④具名键 `KeyEvent` 字段够用。Linux 开发机验不了。
- **回归**：英文打字、Enter/退格/方向、Ctrl+C/V、上档符（%@^）在改动后仍正确（不得因 `edited`↔`key-pressed` 拆分产生双发或丢键）。

## 8. 硬风险与验证门

- **R1（阻断级）**：Slint 1.17 `TextInput` 若不满足第 7 节四行为之一，方案需回炉（备选：经 `i-slint-backend-winit` 直接取 winit `Ime` 事件，代价更大）。**编码前须先在 Windows 做最小 spike 验证这四点，再展开全量实现。**
- **R2**：候选框位置若无法落在可视区，中文可打但用户看不到候选 → 需给接收器真实几何。
- **R3**：`edited` 清空 `text` 可能重入触发 `edited`，需去抖/守卫防递归。

## 9. 非目标（YAGNI）

- 不做被控端 IME 组字。
- 不改 admin-web 浏览器主控（另 spec）。
- 不引入低层键盘钩子。
- 不做输入法切换/状态同步 UI。
