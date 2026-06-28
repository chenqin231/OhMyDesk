# Wayland 截屏支持评估

> **结论先行**：短期**不实现** Wayland 截屏，维持「被控端锁 X11 会话」为部署前提，并以本次新增的 **Wayland 检测提示**把失败显性化（主控端不再无限「等待第一帧」）。中长期若必须支持 Wayland，唯一通用路线是 `xdg-desktop-portal` ScreenCast + PipeWire，但它与「静默被控 Agent」定位冲突（每次弹授权框）且信创真机适配成本高，需产品层面决策。

## 1. 问题本质

xcap 0.4（X11 后端）在 Wayland 会话下**抓不到桌面**：
- Wayland 合成器出于安全隔离，**不允许任意客户端读取全屏内容**（X11 时代 `XGetImage(root)` 的能力被取消）。
- 我们启动时 `lock_x11_session()` 抹掉 `WAYLAND_DISPLAY` 强制走 X11，只会连到 **Xwayland**——而 Xwayland 是兼容层，**看不到原生 Wayland 窗口的像素**，截屏返回空/黑/报错。
- 实测麒麟 V11（2603，UKUI on Wayland）即此现象：会话建立、被控态进入，但截屏线程拿不到帧 → 主控端永久「等待第一帧」。

## 2. 三条技术路线对比

| 路线 | 适用合成器 | 对信创(UKUI/麒麟/统信)是否可行 | 关键代价 |
|---|---|---|---|
| **A. xdg-desktop-portal ScreenCast + PipeWire**（`ashpd` crate）| 通用：GNOME/KDE/UKUI/wlroots | ✅ 理论可行（取决于发行版 portal 后端成熟度）| **每次弹系统授权框**「是否共享屏幕」；引入 `pipewire`+`ashpd` 重依赖；异步协商 + DMA-BUF/SHM 帧解码再转 JPEG；真机适配（龙芯/麒麟 PipeWire 链路未必齐备）|
| **B. wlr-screencopy**（`libwayshot`/wayshot）| 仅 wlroots（sway/hyprland/labwc）| ❌ UKUI/统信/麒麟用 kwin 或 mutter，**不支持** | 对信创无效，否决 |
| **C. 合成器私有 DBus**（如 kwin `org.kde.KWin.ScreenShot`）| 仅 KDE/kwin 系 | ⚠️ 麒麟部分版本基于 kwin 可用，但**碎片化、非标准、逐发行版适配** | 脆弱、维护成本高，不同麒麟/统信版本接口不一致 |

## 3. 路线 A（唯一通用解）为何短期不做

1. **与产品定位冲突**：OhMyDesk 被控端是**静默 Agent**（注册即可被远控，无人值守）。Portal ScreenCast **每次会话强制弹授权框**，需被控机前用户手动点「共享」——无人值守场景直接破功，且无法绕过（这是 Wayland 的安全设计，不是 bug）。
2. **依赖与体积**：引入 `pipewire` + `ashpd` + DBus 运行时，二进制与运行时依赖显著增大；信创基线（glibc 2.28 / 龙芯）上 PipeWire 生态成熟度不确定，需逐机验证。
3. **工作量**：异步 portal 会话协商、PipeWire 流接入、DMA-BUF/SHM 帧格式转换、与现有 350ms JPEG 推帧管线对接、真机调试——估 **3–5 人日 + 多台信创真机联调**，风险偏高。
4. **ROI**：信创桌面（麒麟/统信）当前仍**普遍提供 X11 会话**，切换成本极低（登录界面选 X11）。用一条部署前提即可规避，无需承担上述成本。

## 4. 当前采取的处理（本次已实现）

- **检测显性化**：被控端启动时记录是否为 Wayland 会话（`OHMYDESK_WAYLAND` 标记，于抹除 `WAYLAND_DISPLAY` 前打）；进入被控态后若检测到 Wayland（或截屏器构造失败），**主动回一条 `RemoteNotice` 给主控端**，每会话一次。
- **主控端展示**：Web 管理端在「等待第一帧」处改为展示该提示（「被控端为 Wayland 会话，无法截屏，请切换 X11…」）；Slint 端到端主控复用拒绝态 UI 展示同一原因。
- 协议侧新增 `Message::RemoteNotice { session_id, text }`，按 session 对端路由（同 Frame），ts-rs 同步生成前端类型。

## 5. 建议

- **比赛/演示**：被控机统一 **X11 会话**（登录界面选「UKUI / 兼容(X11)」），评审环境固定 X11——零成本、最稳。
- **部署文档/启动器**：已声明「需 X11 桌面会话」；保持该前提。
- **中长期**：仅当出现「必须支持纯 Wayland、且可接受授权弹窗」的明确需求时，再立项走路线 A，并预留信创真机适配工时。届时静默 Agent 与授权弹窗的矛盾需产品决策（例如：首次授权后记住、或退化为「用户主动发起共享」模式）。
