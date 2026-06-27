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
    /// 收到远程控制请求（被控端弹授权框）。requester 为请求方展示名，session_id 为 server 分配的会话。
    ControlRequest { requester: String, session_id: String },
    /// 会话已建立为被控态（授权通过 / 对端 ack）。
    BeingControlled { peer_name: String },
    /// 主控发起结果：收到对端首帧前的 ack。
    RemoteAck { session_id: String },
    /// 主控发起被拒（密码错/被拒）。
    RemoteRejected { reason: String },
    /// 收到一帧画面（主控态贴帧）：JPEG base64 + 缩放后尺寸。
    Frame { data: String, w: u32, h: u32 },
    /// 会话结束（任一端断开）。
    SessionEnded,
    /// 连接断开（UI 可提示"重连中…"）。
    Disconnected,
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
    /// 主动断开当前会话。
    Disconnect { session_id: String },
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
) {
    let password = format!("{:06}", rand_6());
    loop {
        match conn::connect_once(&server_url, &info, &password, &to_ui, &mut from_ui).await {
            Ok(()) => tracing::warn!("连接正常关闭，3s 后重连"),
            Err(e) => tracing::warn!("连接异常：{e}，3s 后重连"),
        }
        let _ = to_ui.send(ToUi::Disconnected);
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
pub static CAPTURE_CTRL: CaptureCtrlBridge = CaptureCtrlBridge(OnceLock::new());

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

pub struct CaptureCtrlBridge(OnceLock<mpsc::UnboundedSender<CaptureCtrl>>);
impl CaptureCtrlBridge {
    pub fn init(&self, tx: mpsc::UnboundedSender<CaptureCtrl>) {
        let _ = self.0.set(tx);
    }
    fn send(&self, c: CaptureCtrl) {
        if let Some(tx) = self.0.get() {
            let _ = tx.send(c);
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
