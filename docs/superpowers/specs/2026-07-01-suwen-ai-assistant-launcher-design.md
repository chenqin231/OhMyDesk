# 客户端「AI助手」按钮:素问检测/静默安装/拉起 设计文档

> 日期:2026-07-01
> 范围:仅 Windows(其余平台按钮隐藏)
> 状态:待用户复审

## 1. 背景与目标

在 OhMyDesk 客户端(被控端,`src/client`,二进制 `client.exe`)右下角新增一个悬浮「AI助手」按钮。点击后确保「素问」已安装并拉起其 GUI:

1. 检测本机是否已安装素问(以 `suwen-daemon.exe` 是否存在为锚点);
2. 未安装 → 从 `https://ai-agent.guoziweb.com/downloads/client/suwen-setup.exe` 下载安装器,带 `/S` 静默安装;
3. 安装完成 → 拉起 `suwen-gui.exe`;
4. 已安装 → 直接拉起 `suwen-gui.exe`。

**一句话语义**:一个按钮 = 「确保素问已装,并拉起它」。

## 2. 非目标(YAGNI)

- 不做版本比对/升级:只判断存在性,不判断素问是否为最新版。
- 不做卸载/修复/多版本管理。
- 不做非 Windows 平台实现(按钮在非 Windows 隐藏)。
- v1 不做 Authenticode 显式验签(见 §9,列为可选加固)。

## 3. 关键决策(第一性原理:唯一真相是"文件是否存在")

| 决策点 | 结论 | 依据 |
|---|---|---|
| 安装路径 | 固定 `%ProgramFiles%\Suwen\`,兜底 `%ProgramFiles(x86)%\Suwen\` | 始终用 `/S` 驱动安装,目录确定;兜底防 32 位 NSIS 落到 x86 目录。用环境变量而非硬编码 `C:\` |
| 检测锚点 | `<安装目录>\suwen-daemon.exe` 是否存在 | 需求指定;存在即视为已装 |
| 拉起目标 | `<安装目录>\suwen-gui.exe`,普通 `spawn()`(不 wait) | 需求指定 |
| 提权 | 无需额外提权 | 客户端启动即 `elevate::ensure_elevated()`(`main.rs:73`),已以管理员运行;安装器作为子进程继承管理员令牌,不二次弹 UAC |
| 完成判定 | 安装器进程退出(exit=0)**且**轮询到 `daemon.exe` 落盘(超时 ~60s) | `/S` 为异步,单看进程退出不足以保证文件就绪,双条件防过早拉起 |
| 并发 | 全流程后台线程,状态经 `slint::invoke_from_event_loop` 回写 | AppWindow 句柄非 Send,不能在后台线程直接碰 UI(见 `ui_glue.rs` 顶注) |
| 防重入 | 进行中禁用按钮 + `AtomicBool` 门闩 | 防连点触发多次下载/安装 |

## 4. 架构

新增单一模块 `src/client/src/suwen.rs`,对外只暴露一个入口,内部为一条线性状态机。零新依赖(复用 `ureq`/`native-tls`/`tempfile`/`windows-sys`)。

```
ui_glue.rs           app.slint                 suwen.rs (新增, #[cfg(windows)])
  on_launch_suwen ──▶ callback launch_suwen ──▶ ensure_and_launch(ui_weak)
                     property suwen_status         └─ std::thread:
                     property suwen_phase              detect → [download → install → wait] → launch
                     property suwen_supported          每步 invoke_from_event_loop 回写 status/phase
```

### 4.1 `suwen.rs` 职责拆分(每个函数单一职责)

- `install_dir() -> Option<PathBuf>`:按 §3 解析安装目录(主 + 兜底)。
- `daemon_path()/gui_path() -> Option<PathBuf>`:拼 `suwen-daemon.exe`/`suwen-gui.exe`。
- `is_installed() -> bool`:`daemon_path().exists()`。
- `download_setup() -> Result<TempPath>`:复用 `update::build_agent`(SChannel,`update.rs:281`)+ 仿 `download_verified`(`update.rs:229`)去掉 sha/size 强校验,落盘到 `%TEMP%` 临时文件;沿用 50MB 上限的 `CapReader`(`update.rs:197`)。
- `run_installer(setup: &Path) -> Result<()>`:`Command::new(setup).arg("/S").creation_flags(CREATE_NO_WINDOW).status()`,校验 `exit=0`。`CREATE_NO_WINDOW=0x0800_0000`(同 `exec.rs:38`)。
- `wait_installed(timeout) -> Result<()>`:轮询 `is_installed()`,间隔 500ms,超时 ~60s。
- `launch_gui() -> Result<()>`:`Command::new(gui_path).current_dir(install_dir).spawn()`(不 wait、不加窗口 flag)。
- `ensure_and_launch(ui_weak)`:在 `std::thread` 中编排上述步骤 + 回写状态 + 门闩控制。

## 5. 点击行为(状态机)

```
点击「AI助手」
  └─ 门闩已占用? ── 是 ─▶ 忽略(防重入)
                   否
                    ▼ 占用门闩,phase=下载中(仅当需安装)
        is_installed()?
          ├─ 是 ─────────────────────────────▶ launch_gui() ─▶ 完成
          └─ 否 ─▶ download_setup()   (phase=下载中)
                    └─ run_installer() (phase=安装中)
                        └─ wait_installed(60s) (phase=安装中)
                            └─ launch_gui() (phase=启动中) ─▶ 完成
  完成/失败 ─▶ 释放门闩,phase 归位/失败
```

## 6. UI 接线(照抄现有「检查更新」范式)

参考实现:`GhostButton`(`app.slint:102`)、`callback check_update`(`app.slint:570`)、`GhostButton` 实例(`app.slint:748`)、`ui.on_check_update`(`ui_glue.rs:308`)、跨线程回写 `ui.set_update_status`(`ui_glue.rs:935`)。

**app.slint(AppWindow,`:476`,460×620 面板)**:
- 新增 `in property <string> suwen_status;`
- 新增 `in property <int> suwen_phase;`(0 空闲 / 1 下载中 / 2 安装中 / 3 启动中 / 4 失败)
- 新增 `in property <bool> suwen_supported;`(仅 Windows 为 true)
- 新增 `callback launch_suwen();`
- 新增一个悬浮按钮:锚定 AppWindow **右下角**(`x: parent.width - self.width - 16px; y: parent.height - self.height - 16px;`),`visible: root.suwen_supported;`,`enabled: root.suwen_phase == 0 || root.suwen_phase == 4;`,标签「AI助手」,进行中显示 `suwen_status`。

**ui_glue.rs**:
- 新增 `ui.on_launch_suwen(move || { ... crate::suwen::ensure_and_launch(ui_weak.clone()); })`。
- Windows 下 `ui.set_suwen_supported(true)`,其余平台 `false`。

**按钮文案随 phase**:空闲「AI助手」→「下载中…」→「安装中…」→「启动中…」→ 失败「失败,点击重试」。

## 7. 错误处理(三类,回写可读中文)

| 阶段 | 失败条件 | 回写文案(示例) |
|---|---|---|
| 下载 | 网络/TLS/HTTP 非 2xx/超 50MB | `下载失败:<原因>` |
| 安装 | 安装器 exit≠0,或 60s 内未见 daemon.exe | `安装失败,请重试` |
| 拉起 | `spawn` 失败 | `启动失败:<原因>` |

失败一律 `phase=4`,释放门闩,按钮恢复可点(「失败,点击重试」)。全程 `tracing` 落日志。

## 8. 跨平台

`suwen.rs` 整体 `#[cfg(windows)]`。非 Windows:提供同名空实现(`ensure_and_launch` 无操作),且 `suwen_supported=false` 使按钮隐藏。保证 `cargo build` 在 Linux/macOS 仍通过(客户端非 Windows 亦编译,`Cargo.toml` 有非 Windows 分支)。

## 9. 安全

- 下载的是可执行文件并以管理员权限运行。传输侧由 **HTTPS** 保证完整性(必须复用 `build_agent` 显式装配 SChannel,否则 ureq 报 "no TLS backend");素问安装器已具 Authenticode 企业签名,Windows 启动时会验签。
- **可选加固(v1 不做)**:执行前用 `WinVerifyTrust` 显式校验 `suwen-setup.exe` 的 Authenticode 签名与签发主体,拒绝未签名/签名不符的二进制。列为后续项。

## 10. 测试计划

- **单元**:`install_dir` 环境变量解析(主/兜底/都缺失)、`is_installed` 路径拼接、phase 状态迁移纯函数化后可测。
- **手动(Windows 真机)**:
  1. 未装素问 → 点击 → 观察 下载中/安装中/启动中 文案 → Program Files\Suwen 出现两 exe → gui 拉起;
  2. 已装素问 → 点击 → 跳过下载直接拉起;
  3. 断网 → 点击 → 「下载失败」且按钮恢复可点;
  4. 连点按钮 → 仅触发一次(门闩生效);
  5. 非 Windows 构建 → 按钮隐藏、`cargo build` 通过。

## 11. 待确认/可选项

- ⬜ `launch_gui` 是否加 `DETACHED_PROCESS`(0x0000_0008)让素问 GUI 完全脱离 OhMyDesk 生命周期?v1 用普通 `spawn`(GUI 子进程默认可独立存活),按需再加。
- ⬜ §9 Authenticode 显式验签是否提前到 v1。

## 12. 涉及文件清单

| 文件 | 改动 |
|---|---|
| `src/client/src/suwen.rs` | 新增模块(核心逻辑 + 非 Windows 空实现) |
| `src/client/src/main.rs` | `mod suwen;` |
| `src/client/ui/app.slint` | 新增 3 property + 1 callback + 右下角「AI助手」按钮 |
| `src/client/src/ui_glue.rs` | 绑定 `on_launch_suwen` + 设置 `suwen_supported` |
| `src/client/src/update.rs` | 可能抽出 `build_agent`/下载工具为 `pub(crate)` 复用(若当前非 pub) |
