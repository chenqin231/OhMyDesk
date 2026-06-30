//! 反连 WebSocket：注册 + 心跳 + mpsc 出站泵 + 断线重连（M-CLI1/2/3）。
//!
//! 架构（与 server handle_socket 同构）：
//! ```text
//!   ws.split() → (write sink, read stream)
//!   write 只被「出站泵任务」独占持有，从 out_rx 取串行发送 —— 注册/心跳/下行回发统一走它，
//!   绝不把 write move 进心跳 task（否则 Phase 4 回发 Frame/Input 撞所有权墙）。
//!   read → 解析 Envelope → 经 to_ui 通知 UI 层；UI 的动作（授权结果/发起/Frame/Input）
//!   经 from_ui 回到本任务 → 再 out_tx.send 出站。
//! ```
//! 外层 [`run`] 包断线重连：任一环断开 → sleep 3s → 重连重注册。
//!
//! 子模块：
//! - [`conn`]：单次连接生命周期（出站泵 / 心跳 / 主循环）。
//! - [`dispatch`]：下行/上行消息分发。
//! - 本文件：对外类型（ToUi/FromUi/CaptureCtrl）、旁路桥、工具函数、[`run`] 重连循环。

mod conn;
mod dispatch;

use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{EndpointInfo, RamInfo};
use rand::Rng;
use tokio::sync::mpsc;

use crate::asset;

/// net → UI：下行事件（UI 据此更新提示条/弹窗/贴帧）。
#[derive(Debug, Clone)]
pub enum ToUi {
    /// 注册成功，携本机 id + 明文密码（展示给用户报给主控方）。
    Registered { id: String, password: String },
    /// 收到远程控制请求（被控端弹授权框）。requester 为请求方展示名，session_id 为 server 分配的会话，source 为来源中文标签。
    ControlRequest {
        requester: String,
        session_id: String,
        source: String,
    },
    /// 会话已建立为被控态（授权通过 / 对端 ack）。forced=true 表示管理员强制控制。
    BeingControlled {
        peer_name: String,
        forced: bool,
        session_id: String,
    },
    /// 主控发起结果：收到对端首帧前的 ack。
    RemoteAck { session_id: String },
    /// 主控发起被拒（密码错/被拒）。
    RemoteRejected { reason: String },
    /// 收到一帧画面（主控态贴帧）：会话 id + JPEG base64 + 缩放后尺寸。
    /// 带 session_id 让 UI 侧据帧统一会话态——输入发送与画面用同一事实源，
    /// 避免「有画面但 cur_session 未设、输入被丢弃」。
    Frame {
        session_id: String,
        data: String,
        w: u32,
        h: u32,
        seq: u64,
    },
    /// 会话结束（任一端断开）。
    SessionEnded,
    /// 连接断开（UI 可提示"重连中…"）。
    Disconnected,
    /// 主控端收被控回执的命令执行结果（远程命令标签渲染）。
    /// 下行不带 command 原文：主控发命令时已在 UI 本地回显命令，回执仅追加结果块。
    ExecResult {
        /// 执行 id（契约字段，关联请求/回执；UI 当前按发送顺序串行追加，故暂不读取）。
        #[allow(dead_code)]
        exec_id: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        truncated: bool,
        duration_ms: u32,
    },
    /// 主控端收远端目录列表（远程文件标签右栏渲染）。path 为实际列出的绝对目录。
    RemoteEntries {
        path: String,
        entries: Vec<protocol::FileEntry>,
        /// 列目录失败原因（被控回 FileListResp.error）；None 表示成功。
        error: Option<String>,
    },
    /// 文件传输进度（下发/取回通用）：done/total 字节。total=0 表示未知。
    FileProgress {
        /// 传输 id（契约字段，标识具体传输；UI 当前按 name 展示，故暂不读取）。
        #[allow(dead_code)]
        transfer_id: String,
        name: String,
        done: u64,
        total: u64,
    },
    /// 文件传输一次性通知（完成/失败提示，远程文件标签底部状态行）。
    FileNotice { text: String },
    /// 传输完成后刷新对应文件栏：`local=true` 刷左栏（取回完成）、`false` 刷右栏（下发完成）。
    /// 修复「取回/下发成功但 UI 不重列目录 → 文件已在盘上却看不到」。文件夹按文件逐次刷新。
    PaneRefresh { local: bool },
    /// 收到会话内即时消息（即时消息标签 / 被控聊天面板渲染，对端发来）。
    ChatIncoming {
        session_id: String,
        /// 消息 id（契约字段，未来去重/已读回执用；UI 当前仅追加文本，故暂不读取）。
        #[allow(dead_code)]
        msg_id: String,
        text: String,
    },
    /// 发现新版：UI 弹更新横幅（version/url/notes）。
    UpdateAvailable { version: String, url: String, notes: Option<String> },
}

/// UI → net：上行动作（用户操作转成出站消息）。
#[derive(Debug, Clone)]
pub enum FromUi {
    /// 被控端授权裁决。
    AuthDecision { session_id: String, accept: bool },
    /// 主控端发起模式 B 远控。
    StartRemote { target_id: String, password: String },
    /// 主控端键鼠事件回传（坐标已是帧内像素）。
    Input {
        session_id: String,
        event: protocol::InputEvent,
    },
    /// 被控端截屏推帧回流（main 截屏线程产出 → 经当前连接出站泵发对端）。
    Frame {
        session_id: String,
        data: String,
        w: u32,
        h: u32,
        seq: u64,
    },
    /// 被控端批量截图回发：收 ScreenshotReq 截一帧后回流给请求方（admin）。
    /// requester=请求方 id（出站时填信封 to，server 据此 forward_by_to 路由）。
    ScreenshotResp {
        req_id: String,
        requester: String,
        data: String,
        w: u32,
        h: u32,
    },
    /// 被控端会话内提示回流（如 Wayland 无法截屏）→ 主控端展示，替代「无限等待第一帧」。
    Notice { session_id: String, text: String },
    /// 主控端切换画质档位（高清/流畅）→ 发 SetQuality 给被控端。
    SetQuality {
        session_id: String,
        mode: protocol::QualityMode,
    },
    /// 主动断开当前会话。
    Disconnect { session_id: String },
    /// 本端剪贴板变化 → 推给对端(会话内双向同步)。
    ClipboardSync { session_id: String, text: String },
    /// 被控端主动断开当前被控会话。
    StopControlled { session_id: String },
    /// 刷新本机临时密码：重新生成并重发 Register（server DashMap 按 id upsert 覆盖旧密码）。
    RefreshPassword,
    /// 主控取消尚未建立的申请(无 session_id):置本地取消标记(迟到 ConnectAck 时收尾) +
    /// 带 target 发 CancelRequest 给 server,令其撤销被控端授权弹窗。
    CancelRemote { target: String },
    /// 主控端发起一次性远程命令 → 发 ExecRequest 给被控端。
    ExecCommand { session_id: String, command: String },
    /// 主控端浏览远端目录 → 发 FileListRequest 给被控端。
    ListRemote { session_id: String, path: String },
    /// 主控端下发本机文件到远端当前目录（push）。
    PushFile {
        session_id: String,
        local_path: String,
        dest_dir: String,
    },
    /// 主控端从远端取回文件到本机目录（pull）：记 transfer_id→local_dir 后发 FilePullRequest。
    PullFile {
        session_id: String,
        remote_path: String,
        local_dir: String,
    },
    /// 会话内发送即时消息（主控/被控通用）→ 发 ChatMessage 给对端。
    SendChat { session_id: String, text: String },
    /// 切换桌面帧推流（懒推流）：主控切到/离开「远程桌面」标签 → 发 SetCapture 给被控端。
    SetCapture { session_id: String, active: bool },
}

// ── M-CLI3：工具函数 ──────────────────────────────────────────────

/// 随机 6 位数字密码（000000~999999）。
pub fn rand_6() -> u32 {
    rand::thread_rng().gen_range(0..1_000_000)
}

/// 当前 Unix 毫秒时间戳。
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 当前内存用量（心跳上报）。委托 asset，集中实现。
pub fn cur_ram() -> RamInfo {
    asset::cur_ram()
}

/// 外层重连循环：断开后 sleep 3s 重连重注册，永不退出（M-CLI2）。
///
/// `info` 每轮 clone（id/密码保持稳定，硬件信息不变）；`to_ui` 投递下行给 UI；
/// `from_ui` 是 UI 上行的接收端（**注意**：单消费者，重连时复用同一接收端）。
pub async fn run(
    server_url: String,
    info: EndpointInfo,
    to_ui: mpsc::UnboundedSender<ToUi>,
    mut from_ui: mpsc::UnboundedReceiver<FromUi>,
    telemetry_tx: mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) {
    // 密码用 Arc<Mutex> 共享：刷新时 connect_once 内就地更新，重连后续轮沿用最新值。
    let password = std::sync::Arc::new(std::sync::Mutex::new(format!("{:06}", rand_6())));
    loop {
        match conn::connect_once(&server_url, &info, &password, &to_ui, &mut from_ui, &telemetry_tx).await {
            Ok(()) => tracing::warn!("连接正常关闭，3s 后重连"),
            Err(e) => tracing::warn!("连接异常：{e}，3s 后重连"),
        }
        let _ = to_ui.send(ToUi::Disconnected);
        let _ = telemetry_tx.send(crate::telemetry::TelemetryMsg::Event("reconnect".into()));
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

// ── 注入 / 截图 / 推帧旁路通道：net 收到下行后投递给 main 的执行侧 ──
// 用全局 OnceLock 持 sender，main 启动时初始化接收端（注入/截图/截屏依赖 X11，与 UI 线程外执行）。

use std::sync::OnceLock;

/// 注入事件旁路：(session_id, event)。
pub static INJECT_TX: InjectBridge = InjectBridge(OnceLock::new());
/// 截图请求旁路：(req_id, requester_from)。
pub static SCREENSHOT_TX: ScreenshotBridge = ScreenshotBridge(OnceLock::new());
/// 被控端推帧启停旁路。
pub static CAPTURE_CTRL: CaptureCtrlBridge = CaptureCtrlBridge(std::sync::Mutex::new(None));
/// 剪贴板控制/写入旁路:net 收下行/会话启停 → 交 clipboard worker 线程(独占 arboard)。
pub static CLIPBOARD_TX: ClipboardBridge = ClipboardBridge(OnceLock::new());

pub struct InjectBridge(OnceLock<mpsc::UnboundedSender<(String, protocol::InputEvent)>>);
impl InjectBridge {
    pub fn init(&self, tx: mpsc::UnboundedSender<(String, protocol::InputEvent)>) {
        let _ = self.0.set(tx);
    }
    fn with_send(&self, session_id: String, event: protocol::InputEvent) {
        if let Some(tx) = self.0.get() {
            let _ = tx.send((session_id, event));
        }
    }
}

pub struct ScreenshotBridge(OnceLock<mpsc::UnboundedSender<(String, String)>>);
impl ScreenshotBridge {
    pub fn init(&self, tx: mpsc::UnboundedSender<(String, String)>) {
        let _ = self.0.set(tx);
    }
    fn with_send(&self, req_id: String, requester: String) {
        if let Some(tx) = self.0.get() {
            let _ = tx.send((req_id, requester));
        }
    }
}

/// 被控端推帧启停信号：net 在被控会话激活/结束时通知 main 的截屏线程启停 2-3fps 推帧。
#[derive(Debug, Clone)]
pub enum CaptureCtrl {
    /// 开始向 `session_id` 推帧。
    Start { session_id: String },
    /// 停止推帧（会话结束/断开）。
    Stop,
}

// 用 Mutex<Option> 而非 OnceLock：init 可重入（production 仅启动调一次，语义不变；
// 测试中允许多个用例各自重装接收端，规避 OnceLock「首次 init 独占、后续静默失效」导致的跨用例串扰）。
pub struct CaptureCtrlBridge(std::sync::Mutex<Option<mpsc::UnboundedSender<CaptureCtrl>>>);
impl CaptureCtrlBridge {
    pub fn init(&self, tx: mpsc::UnboundedSender<CaptureCtrl>) {
        *self.0.lock().unwrap() = Some(tx);
    }
    fn send(&self, c: CaptureCtrl) {
        if let Some(tx) = self.0.lock().unwrap().as_ref() {
            let _ = tx.send(c);
        }
    }
}

/// 剪贴板 worker 控制消息:会话启停 + 对端写入。
#[derive(Debug, Clone)]
pub enum ClipboardMsg {
    /// 会话激活:开始轮询本地剪贴板,变化推 `session_id` 对端。
    Start { session_id: String },
    /// 会话结束:停止轮询并清空 last_synced。
    Stop,
    /// 收到对端剪贴板文本:写本地 + 更新 last_synced(防回环)。
    Incoming { text: String },
}

pub struct ClipboardBridge(OnceLock<mpsc::UnboundedSender<ClipboardMsg>>);
impl ClipboardBridge {
    pub fn init(&self, tx: mpsc::UnboundedSender<ClipboardMsg>) {
        let _ = self.0.set(tx);
    }
    // 私有 fn(与 CaptureCtrlBridge::send 一致):私有项对子模块 net::dispatch 可见。
    fn send(&self, m: ClipboardMsg) {
        if let Some(tx) = self.0.get() {
            let _ = tx.send(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{Envelope, Message};

    #[test]
    fn rand_6_在六位范围内() {
        for _ in 0..1000 {
            let v = rand_6();
            assert!(v < 1_000_000, "rand_6 越界：{v}");
            // 格式化后必为 6 位
            assert_eq!(format!("{v:06}").len(), 6);
        }
    }

    #[test]
    fn now_是合理毫秒时间戳() {
        let t = now();
        // 2020-01-01 之后（毫秒），证明是毫秒非秒
        assert!(t > 1_577_836_800_000, "时间戳过小，疑似秒级：{t}");
    }

    #[test]
    fn 上行_发起远控_序列化为_connect_request_mode_b() {
        // 直接验证 StartRemote 映射出的 Envelope 序列化契约
        let env = Envelope {
            from: "ep-a".into(),
            to: Some("ep-b".into()),
            ts: now(),
            payload: Message::ConnectRequest {
                mode: protocol::Mode::B,
                target: "ep-b".into(),
                password: Some("123456".into()),
                force: false,
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"type\":\"connect_request\""));
        assert!(json.contains("\"mode\":\"b\""));
    }

    #[test]
    fn 上行_授权拒绝_带reason() {
        let env = Envelope {
            from: "ep-b".into(),
            to: None,
            ts: 0,
            payload: Message::AuthResult {
                session_id: "s1".into(),
                ok: false,
                reason: Some("用户拒绝".into()),
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"type\":\"auth_result\""));
        assert!(json.contains("\"ok\":false"));
    }
}
