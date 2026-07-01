# 顶部更新条布局改造 实现计划（Spec ①）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「更新状态 + 检查更新按钮」从设备卡上移到窗口顶部单行条 `[S标志 | 转圈 | 状态文本 | 检查更新]`，并按 `update_phase` 阶段量做颜色 + 转圈的即时反馈。

**Architecture:** UI 单向数据流——Rust `update.rs` 各状态转换发 `ToUi::UpdateStatus { text, phase }`，`ui_glue` 收后 `set_update_status`+`set_update_phase`，`app.slint` 顶部条按 phase 渲染颜色与转圈。引入类型化 `update_phase`(u8/int)避免 UI 靠中文文案字符串判断状态。转圈用 `animation-tick()` 且仅在瞬态 phase 存在于渲染树，把软渲染开销限制在几秒窗口。

**Tech Stack:** Rust（client crate）、Slint UI、tokio mpsc（ToUi 通道）。

参考 spec：`docs/superpowers/specs/2026-07-01-client-update-topbar-design.md`

---

## 文件结构

| 文件 | 职责 | 改动 |
|---|---|---|
| `src/client/ui/app.slint` | 顶部条布局 + `update_phase` 属性 + 转圈 + 删设备卡重复行 | 改 `:690-719`(Header) 与删 `:785-800`(设备卡更新行) |
| `src/client/src/net/mod.rs` | `ToUi::UpdateStatus` 加 `phase` 字段 + phase 常量 | 改 `:109` + 加常量模块 |
| `src/client/src/update.rs` | 5 个状态发送点补 phase | 改 `:322,330,351,366,385` |
| `src/client/src/ui_glue.rs` | 收 UpdateStatus 时 `set_update_phase`；手动检查设 phase=1 | 改 `:313,934-937` |

**任务顺序**：先改 app.slint（新增属性/布局，`set_update_phase` 生成器就绪，整 crate 仍可编译）→ 再改 Rust（加 phase 字段 + 接线，含单测）。此序保证每步可编译。

---

### Task 1：app.slint 顶部条布局 + update_phase 属性 + 转圈 + 删重复行

**Files:**
- Modify: `src/client/ui/app.slint:570`（属性区，加 `update_phase`）
- Modify: `src/client/ui/app.slint:690-719`（Header → 顶部条）
- Modify: `src/client/ui/app.slint:785-800`（删设备卡内更新行）

- [ ] **Step 1：加 `update_phase` 属性**

在 `:570` 现有 `in property <string> update_status;` 下一行加：

```slint
    in property <int> update_phase;               // 更新阶段：0空闲/最新 1检查中 2下载中 3失败（驱动颜色/转圈）
```

- [ ] **Step 2：把 Header（`:690-719`）替换为顶部条**

将现有 `// ── Header ──` 那个只含 S 方块的 `HorizontalLayout { ... }` 整块替换为：

```slint
                        // ── 顶部条：S标志(诊断入口,保留) + 转圈(仅瞬态) + 更新状态 + 检查更新 ──
                        HorizontalLayout {
                            spacing: 10px;
                            // S 标志（连点 5 次开诊断面板，逻辑一字不改）
                            Rectangle {
                                width: 34px;
                                height: 34px;
                                border-radius: 9px;
                                background: #3b82f626;
                                border-width: 1px;
                                border-color: #3b82f640;
                                Text {
                                    text: "S";
                                    color: Theme.primary;
                                    font-size: 18px;
                                    font-weight: 800;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }
                                TouchArea {
                                    clicked => {
                                        root.diag_taps += 1;
                                        if (root.diag_taps >= 5) {
                                            root.diag_visible = true;
                                            root.diag_taps = 0;
                                        }
                                    }
                                }
                            }
                            // 转圈：仅检查中/下载中时存在于渲染树（animation-tick 只在此期间驱动重绘）
                            if root.update_phase == 1 || root.update_phase == 2: Rectangle {
                                width: 14px;
                                height: 14px;
                                vertical-alignment: center;
                                rotation-angle: (animation-tick() / 1s) * 360deg;
                                rotation-origin-x: 7px;
                                rotation-origin-y: 7px;
                                // 偏离中心的小圆点，随父旋转 → 呈现转圈
                                Rectangle {
                                    width: 5px;
                                    height: 5px;
                                    border-radius: 2.5px;
                                    background: root.update_phase == 2 ? Theme.primary : Theme.fg-muted;
                                    x: 9px;
                                    y: 4.5px;
                                }
                            }
                            // 状态文本：颜色随 phase；空则占位
                            Text {
                                text: root.update_status == "" ? "点右侧按钮检查更新" : root.update_status;
                                color: root.update_status == "" ? Theme.fg-muted
                                     : root.update_phase == 0 ? Theme.emerald
                                     : root.update_phase == 2 ? Theme.primary
                                     : root.update_phase == 3 ? Theme.amber
                                     : Theme.fg-muted;
                                font-size: 12px;
                                vertical-alignment: center;
                                overflow: elide;
                                horizontal-stretch: 1;
                            }
                            GhostButton {
                                label: "检查更新";
                                clicked => { root.check_update(); }
                            }
                        }
```

- [ ] **Step 3：删除设备卡内的重复更新行**

删除 `:785-800` 整段（注释 `// 更新状态行（始终可见）...` + 其下的 `HorizontalLayout { ... 检查更新 ... }`）。设备卡回到「本机 ID / 临时密码」两行结束。

- [ ] **Step 4：编译门**

Run: `cargo build -p client 2>&1 | tail -20`
Expected: 编译通过（slint 宏无报错）。此时 `update_phase` 属性存在但暂无人写入（保持 0），顶部条已呈现、转圈不出现（phase 恒 0）。

- [ ] **Step 5：Commit**

```bash
git add src/client/ui/app.slint
git commit -m "feat(ui): 顶部条上移更新状态+检查按钮,加 update_phase 转圈,删设备卡重复行"
```

---

### Task 2：Rust 侧 phase 接线（ToUi 字段 + 常量 + update.rs + ui_glue）

**Files:**
- Modify: `src/client/src/net/mod.rs:109`（字段）+ 加 phase 常量模块 + 加单测
- Modify: `src/client/src/update.rs`（`:322,330,351,366,385` 补 phase）
- Modify: `src/client/src/ui_glue.rs`（`:313` 设 phase=1；`:934-937` set_update_phase）

- [ ] **Step 1：写失败测试（phase 常量 + 字段）**

在 `src/client/src/net/mod.rs` 末尾（或其 `#[cfg(test)]` 区）加：

```rust
#[cfg(test)]
mod update_phase_tests {
    use super::*;

    #[test]
    fn phase常量取值() {
        assert_eq!(
            (update_phase::IDLE, update_phase::CHECKING, update_phase::DOWNLOADING, update_phase::FAILED),
            (0u8, 1u8, 2u8, 3u8)
        );
    }

    #[test]
    fn update_status_携带_phase() {
        let m = ToUi::UpdateStatus { text: "检查中".into(), phase: update_phase::CHECKING };
        match m {
            ToUi::UpdateStatus { phase, .. } => assert_eq!(phase, 1),
            _ => panic!("变体不符"),
        }
    }
}
```

- [ ] **Step 2：运行测试确认失败**

Run: `cargo test -p client update_phase_tests 2>&1 | tail -20`
Expected: FAIL（编译错误：`update_phase` 模块不存在、`ToUi::UpdateStatus` 无 `phase` 字段）。

- [ ] **Step 3：加 phase 常量模块 + 字段**

在 `src/client/src/net/mod.rs` 的 `pub enum ToUi {` 定义**之前**加常量模块：

```rust
/// 更新状态阶段：UI 的颜色/转圈只依赖此量，不匹配中文文案（文案可变、阶段稳定）。
pub mod update_phase {
    pub const IDLE: u8 = 0;        // 空闲/已最新
    pub const CHECKING: u8 = 1;    // 检查中（转圈）
    pub const DOWNLOADING: u8 = 2; // 下载中（转圈）
    pub const FAILED: u8 = 3;      // 失败/会话延迟
}
```

把 `:109` 的 `UpdateStatus { text: String },` 改为：

```rust
    UpdateStatus { text: String, phase: u8 },
```

- [ ] **Step 4：update.rs 5 个发送点补 phase**

依次改（用 `net::update_phase::*` 或按 update.rs 内已有的 `ToUi` 引用路径，保持与文件现有 import 一致）：

- `:330` 检查开始：
  ```rust
  let _ = to_ui.send(ToUi::UpdateStatus { text: "正在检查更新…".into(), phase: crate::net::update_phase::CHECKING });
  ```
- `:351` 已最新（Skip）：
  ```rust
  UpdateAction::Skip => { let _ = to_ui.send(ToUi::UpdateStatus { text: format!("已是最新（当前 v{}）", env!("CARGO_PKG_VERSION")), phase: crate::net::update_phase::IDLE }); Ok(()) }
  ```
- `:366` 开始下载：
  ```rust
  let _ = to_ui.send(ToUi::UpdateStatus { text: format!("正在下载更新 v{version}…"), phase: crate::net::update_phase::DOWNLOADING });
  ```
- `:385` 会话中延迟：
  ```rust
  let _ = to_ui.send(ToUi::UpdateStatus { text: format!("有远控会话，稍后自动更新 v{version}"), phase: crate::net::update_phase::FAILED });
  ```
- `:322` 检查失败：
  ```rust
  let _ = to_ui.send(ToUi::UpdateStatus { text: "更新检查失败，稍后重试".into(), phase: crate::net::update_phase::FAILED });
  ```

> 注：`ToUi` 在 update.rs 的实际引用前缀以文件顶部 `use` 为准（可能是 `use crate::net::ToUi;`）。常量同理用可解析的路径。

- [ ] **Step 5：ui_glue 接线**

- `:934-937` 处理 UpdateStatus，改为同时写 text 与 phase：
  ```rust
  net::ToUi::UpdateStatus { text, phase } => {
      if let Some(ui) = weak.upgrade() {
          ui.set_update_status(text.into());
          ui.set_update_phase(phase as i32);
      }
  }
  ```
  > 以现有该分支的实际闭包/upgrade 写法为准，仅在原 `set_update_status` 旁增补 `set_update_phase(phase as i32)`；`phase` 从模式解构取出。

- `:313` 手动检查回调，设「检查中」同时设 phase=1：
  ```rust
  ui.set_update_status("正在检查更新…".into());
  ui.set_update_phase(1);
  ```
  （紧挨现有 `set_update_status` 之后。）

- [ ] **Step 6：运行测试确认通过 + 全量编译**

Run: `cargo test -p client update_phase_tests 2>&1 | tail -20`
Expected: PASS（2 测试通过）。

Run: `cargo build -p client 2>&1 | tail -5`
Expected: 编译通过（所有 `UpdateStatus` 构造/解构点已带 phase）。

- [ ] **Step 7：Commit**

```bash
git add src/client/src/net/mod.rs src/client/src/update.rs src/client/src/ui_glue.rs
git commit -m "feat(update): UpdateStatus 带 phase 阶段量,5 状态发送点+ui_glue 接线"
```

---

## 人工验收清单（Rust+UI 合并后跑一次真机/开发机）

- [ ] 顶部条单行呈现 `[S | (转圈) | 状态文本 | 检查更新]`；设备卡内已无重复更新行。
- [ ] 点「检查更新」→ 立即出现转圈 + 灰蓝「正在检查更新…」；结束后：已最新=绿「已是最新（当前 vX）」/ 有更新=蓝转圈「正在下载更新 vX…」/ 失败=琥珀「更新检查失败，稍后重试」。
- [ ] 连点 S 标志 5 次仍能打开诊断面板。
- [ ] 静止态（非检查/下载）无转圈、无持续重绘（观察 CPU 不因空闲 UI 升高）。

---

## Self-Review 记录

- **Spec 覆盖**：spec 组件 1(布局)=Task1 Step2；组件 2(删重复)=Task1 Step3；组件 3(phase+颜色+转圈)=Task1 Step1/2 + Task2；组件 4(Rust 接线)=Task2。✔ 全覆盖。
- **占位符扫描**：无 TBD/TODO；发送点均给出完整代码。对「ToUi 在 update.rs 的引用前缀」「UpdateStatus 分支现有写法」标注了「以文件现状为准」——这是让实现者对齐既有 import 风格，非省略代码。
- **类型一致**：`update_phase` 常量 u8；slint 属性 `int`，接线处 `phase as i32`；ToUi 字段 `phase: u8`。三处一致。✔
