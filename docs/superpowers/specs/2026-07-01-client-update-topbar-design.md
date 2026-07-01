# 客户端顶部更新条布局改造 设计文档（Spec ①）

> 日期：2026-07-01 · 状态：待评审 · 类型：UI/UX 小改
> 设计取向：Dieter Rams「尽可能少的设计」+ 状态即时反馈

## 目标

一句话：把「更新状态 + 检查更新按钮」从设备卡内上移到窗口顶部，合成单行顶部条 `[S标志 | 状态文本 | 检查更新]`，并为自动更新的各阶段提供**颜色 + 转圈**的即时视觉反馈。

## 背景（现状事实）

- **顶部区**（`src/client/ui/app.slint:690-719`）：目前只有一个 34px「S」方块。它承载**连点 5 次打开隐藏诊断面板**的 TouchArea（`:709-717`）——这是导出诊断包的**唯一入口**，不可丢失。品牌标题「信创内网安全管控客户端」与在线胶囊已在 v0.4.1 删除。
- **更新状态行**（`src/client/ui/app.slint:785-800`）：现位于「您的设备」卡内，`[状态文本(elide/stretch) | 检查更新按钮]`。数据管线已在 0.4.3 就绪：
  - UI 侧：`in property <string> update_status`（`:570`）、`callback check_update()`（`:571`）。
  - Rust 侧：`update.rs` 在各状态转换处已发 `ToUi::UpdateStatus { text }`（`net/mod.rs:109`），ui_glue 收到后 `set_update_status`。`main.rs` 已注入 `app_version`。
- **update.rs 当前会发的状态文案**（`src/client/src/update.rs`）：
  - `:330` 检查开始 → 「正在检查更新…」
  - `:351` 已最新 → 「已是最新（当前 vX）」
  - `:366` 开始下载 → 「正在下载更新 vX…」
  - `:385` 会话中延迟 → 「有远控会话，稍后自动更新 vX」
  - `:322` 失败 → 「更新检查失败，稍后重试」

## 已定决策（brainstorming 确认）

1. **布局**：保留小 S 标志 + 状态文本 + 检查更新按钮，**单行**替代原 S 行；设备卡内的原更新行**删除**（消除重复）。
2. **视觉反馈**：颜色 + 文字 + **转圈动画**（检查中/下载中转圈）。
3. **诊断入口**：S 标志的连点 5 次 TouchArea **原样保留**。

## 架构

顶部条是一个 `HorizontalLayout`，三元素横向排布，数据全部由 Rust 单向推送（`ToUi` → ui_glue → set property）。核心设计点：**引入类型化阶段量 `update_phase`（int），不让 UI 靠中文文案字符串匹配来判断状态**——文案可变、翻译可变，阶段量稳定，颜色/转圈只依赖阶段量。

```
Rust: update.rs 状态转换
        │  ToUi::UpdateStatus { text, phase }
        ▼
      ui_glue  ──►  ui.set_update_status(text)
                    ui.set_update_phase(phase)
        │
        ▼
app.slint 顶部条：
  [S标志(诊断5连点)] [转圈?(phase∈{1,2})] [状态文本(色随phase,elide,stretch)] [检查更新按钮]
```

## 组件分解（MECE）

### 组件 1：顶部条布局（app.slint Header）
将 `:690-719` 的裸 S 方块 `HorizontalLayout` 改为三元素单行：
- **S 标志**：保持 34px 方块 + 内含 TouchArea（连点 5 次逻辑 `:709-717` 一字不改）。
- **转圈占位**：一个小图元（见组件 3），`visible: root.update_phase == 1 || root.update_phase == 2`。
- **状态文本**：`Text`，`text: root.update_status == "" ? "点右侧按钮检查更新" : root.update_status`，`horizontal-stretch: 1`，`overflow: elide`，`color` 随 phase（见组件 3 映射表）。
- **检查更新按钮**：`GhostButton { label: "检查更新"; clicked => { root.check_update(); } }`。

### 组件 2：删除设备卡内重复行
移除 `src/client/ui/app.slint:785-800` 的更新状态 `HorizontalLayout`（含其 Text 与 GhostButton）。设备卡回到「本机 ID / 临时密码」两行。

### 组件 3：阶段量 + 颜色 + 转圈
- **新增 property**：`in property <int> update_phase;`（默认 0）。
- **阶段→视觉映射**：

  | phase | 语义 | 文案示例 | 颜色 | 转圈 |
  |---|---|---|---|---|
  | 0 | 空闲/最新 | 已是最新（当前 vX） | 绿（Theme.emerald） | 否 |
  | 1 | 检查中 | 正在检查更新… | 灰蓝（Theme.fg-muted/primary） | **是** |
  | 2 | 下载中 | 正在下载更新 vX… | 蓝（Theme.primary） | **是** |
  | 3 | 失败/延迟 | 检查失败，稍后重试 / 有远控会话，稍后自动更新 vX | 琥珀（Theme.amber，缺则用 warn 色） | 否 |

  > 空闲初始态（从未检查）文案用占位「点右侧按钮检查更新」，phase=0，无色强调。

- **转圈实现（软渲染友好）**：用 `Timer { interval: 100ms; running: root.update_phase == 1 || root.update_phase == 2; triggered => { angle += 36deg; } }` 驱动一个 `rotation-angle` 属性，套在一个小圆弧/圆点图元上。**仅瞬态运行**（检查/下载的几秒），静止态 Timer `running: false` 不空转 → 把软渲染重绘开销限制在瞬态窗口。10fps 足够表达「进行中」，避免高频重绘。

### 组件 4：Rust 接线
- **协议扩字段**：`ToUi::UpdateStatus { text: String }` → `ToUi::UpdateStatus { text: String, phase: u8 }`（`net/mod.rs:109`）。
- **update.rs 发送点补 phase**：`:330`→1、`:351`→0、`:366`→2、`:385`→3、`:322`→3。
- **ui_glue 处理**：`UpdateStatus { text, phase }` → `ui.set_update_status(text); ui.set_update_phase(phase as i32);`
- **手动检查回调**：现有 `on_check_update`（发「正在检查更新…」+ `update::nudge()`）同步设 phase=1。

## 数据流

`update.rs 状态转换` → `ToUi::UpdateStatus{text,phase}`（mpsc）→ `ui_glue` → `set_update_status + set_update_phase` → `app.slint` 顶部条按 phase 渲染文本色 + 转圈可见性。单向、无回流。

## 错误处理与边界

- **phase 缺省**：任何未显式设置的路径 phase=0，转圈不跑（安全默认）。
- **文案为空**：显示占位「点右侧按钮检查更新」。
- **被控中检查**：若本机正被远控（弱 Xeon）同时触发检查，转圈重绘与编码争 CPU——但检查/下载是**几秒瞬态**且 10fps，代价有界；下载完成即停转。
- **诊断入口回归**：连点 5 次逻辑不动，改版后必须验证仍能打开诊断面板。

## 测试策略

- **编译门**：`cargo build -p client`（slint 宏编译通过）。
- **phase 映射单测**：对 update.rs 的 5 个发送点，断言各自发出的 phase 值正确（0/1/2/3）。可对 `run_once`/`apply_auto` 的状态转换做纯逻辑断言（提取 phase 决策为可测函数，或断言发出的 `ToUi` 变体携带的 phase）。
- **视觉自检（人工）**：4 状态各截一次——检查中(转圈灰蓝)/最新(绿)/下载中(转圈蓝)/失败(琥珀)；确认诊断面板仍可连点 5 次打开；确认设备卡不再有重复更新行。

## 非目标（YAGNI）

- 不做更新进度条百分比（下载是自替换，进度粒度价值低）。
- 不做「立即更新/稍后」确认弹窗（Windows 静默自替换是既定策略）。
- 不改 Header 之外的任何卡片布局。

## 验收标准

1. 顶部条单行呈现 `[S | (转圈) | 状态文本 | 检查更新]`，设备卡内无重复更新行。
2. 点「检查更新」→ 立即转圈 + 文案「正在检查更新…」；结束后按结果切到最新(绿)/有更新下载(蓝转圈)/失败(琥珀)。
3. 连点 S 标志 5 次仍能打开诊断面板。
4. 静止态（非检查/下载）无转圈、无持续重绘。
