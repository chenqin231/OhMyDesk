//! 后台 worker（依赖 X11，阻塞 / `!Send`）：注入消费 / 截图消费 / 被控端推帧。
//!
//! enigo（注入）与 xcap（截屏）都是阻塞 X11 调用且句柄非 Send，故各自留在专用 `std::thread`
//! 内独占持有，不混进 async select；与 tokio 侧用 mpsc 通道交互（mpsc 的 send 同步非阻塞）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{capture, geom, inject, net};

/// 最近一次"可见输入"(button/key)注入的 Unix 毫秒时刻。
/// 注入线程写、抓帧线程读——事件驱动抓帧的唯一跨线程信号（KISS：只传时间戳，非业务态）。
static LAST_INPUT_MS: AtomicU64 = AtomicU64::new(0);

/// 注入线程在注入 button/key 后调用，标记"刚发生输入"。
pub fn mark_input_now() {
    LAST_INPUT_MS.store(now_ms(), Ordering::Relaxed);
}

/// LAST_INPUT_MS 是否晚于给定时刻（抓帧线程判断"上次抓帧后是否有新输入"）。
fn last_input_after(since_ms: u64) -> bool {
    LAST_INPUT_MS.load(Ordering::Relaxed) > since_ms
}

/// 当前 Unix 毫秒。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 是否应把本地剪贴板变化推给对端:非空且与上次同步值不同(防回环核心)。
pub fn should_push_clipboard(current: &str, last_synced: &str) -> bool {
    !current.is_empty() && current != last_synced
}

/// 注入消费：被控态收到的 Input 事件 → enigo 注入。
///
/// 注入器留在专用线程独占持有。按本机截屏 `real_size` + 等比缩放帧尺寸构造，坐标按 real/frame 还原。
pub async fn consume_inject(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<(String, protocol::InputEvent)>,
) {
    // 真实屏尺寸：构造截屏器拿 real_size（失败则注入退化为 1:1）。
    let real = tokio::task::spawn_blocking(|| capture::Capturer::new().ok().map(|c| c.real_size()))
        .await
        .ok()
        .flatten();
    let (real_w, real_h) = real.unwrap_or((geom::MAX_W, geom::MAX_H));

    // 注入器在专用线程内构造并独占；用 std::sync::mpsc 把事件转进去。
    // 帧尺寸（坐标还原基准）随画质档位实时派生，不在此固定，避免切高清后点击错位。
    let (blk_tx, blk_rx) = std::sync::mpsc::channel::<protocol::InputEvent>();
    std::thread::spawn(move || {
        let mut injector = match inject::Injector::new(real_w, real_h) {
            Ok(i) => {
                tracing::info!("注入器就绪 real={real_w}x{real_h}（帧尺寸随档位动态）");
                i
            }
            Err(e) => {
                tracing::warn!("注入器构造失败（无 X11/注入后端？）：{e}，注入禁用");
                return;
            }
        };
        while let Ok(ev) = blk_rx.recv() {
            match injector.apply(&ev) {
                Ok(()) => {}
                Err(e) => tracing::warn!("注入失败 ev={ev:?}：{e}"),
            }
        }
    });

    while let Some((_sid, ev)) = rx.recv().await {
        let _ = blk_tx.send(ev);
    }
}

/// 截图消费：收 ScreenshotReq → 截一帧 → 回发 ScreenshotResp 给请求方（I3）。
///
/// 截屏（xcap）阻塞且 `!Send`，留 spawn_blocking。截到帧经 `from_ui_tx`（与推帧同一出站泵）
/// 回流为 `FromUi::ScreenshotResp`，由 net 出站填 `to=requester` 发出，server forward_by_to 路由回 admin。
pub async fn consume_screenshot(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<(String, String)>,
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
) {
    while let Some((req_id, requester)) = rx.recv().await {
        // OHMYDESK_FAKE_CAPTURE=1：真实截屏不可用的环境用占位帧，保证批量截图链路可演示。
        let r = if capture::fake_capture_enabled() {
            Ok(capture::placeholder_frame(0))
        } else {
            tokio::task::spawn_blocking(|| capture::Capturer::new().and_then(|c| c.frame())).await
        };
        match r {
            Ok(Ok((b64, w, h))) => {
                tracing::info!(
                    "截图就绪 req_id={req_id} from={requester} size={w}x{h} bytes={}",
                    b64.len()
                );
                let _ = from_ui_tx.send(net::FromUi::ScreenshotResp {
                    req_id,
                    requester,
                    data: b64,
                    w,
                    h,
                });
            }
            _ => tracing::warn!("截图失败 req_id={req_id}（无显示器/X11？）"),
        }
    }
}

/// 被控端推帧：CAPTURE_CTRL Start/Stop 驱动。
///
/// 截屏（xcap）是阻塞调用，故 Capturer 留在专用 std::thread 内独占持有，按 ~350ms 节奏截帧；
/// 当前活跃会话经 `Arc<Mutex<Option<String>>>` 共享（Start 写入、Stop 清空）。每帧经 tokio mpsc
/// 的 `from_ui_tx`（同步非阻塞 send，可在普通线程调用）回流到 net 出站泵发对端。
pub async fn consume_capture(
    mut ctrl_rx: tokio::sync::mpsc::UnboundedReceiver<net::CaptureCtrl>,
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
) {
    let active: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

    // 截屏推帧线程：独占 Capturer，按节奏截帧。
    {
        let active = active.clone();
        std::thread::spawn(move || {
            let fake = capture::fake_capture_enabled();
            let mut capturer: Option<capture::Capturer> = None;
            let mut seq: u64 = 0;
            // 已就「截屏不可用」回执过的会话 id：每会话只回一次，避免刷屏。
            let mut notified_for: Option<String> = None;
            loop {
                // 帧率随画质档位：流畅优先 ~16fps，高清优先 ~10fps（被控端 CPU 弱时由档位调控）。
                let qp = capture::current_params();
                std::thread::sleep(std::time::Duration::from_millis(qp.interval_ms));
                let sid = match active.lock().unwrap().clone() {
                    Some(s) => s,
                    None => continue, // 未在被控态，空转
                };
                // 取一帧：fake 模式走占位帧（WSLg 等无法真实截屏的环境验证链路）；否则真实截屏。
                let frame = if fake {
                    seq += 1;
                    capture::placeholder_frame(seq)
                } else {
                    // Wayland 会话抓不到桌面：明确回执主控端（替代「无限等待第一帧」）并停推帧。
                    if capture::is_wayland_session() {
                        if notified_for.as_deref() != Some(sid.as_str()) {
                            tracing::warn!("Wayland 会话无法截屏，已通知主控端；请切换 X11（UKUI 兼容）会话");
                            let _ = from_ui_tx.send(net::FromUi::Notice {
                                session_id: sid.clone(),
                                text: "被控端为 Wayland 会话，无法截屏。请在登录界面切换到 X11（UKUI 兼容）会话后重新连接。".into(),
                            });
                            notified_for = Some(sid.clone());
                        }
                        *active.lock().unwrap() = None;
                        continue;
                    }
                    // 懒构造截屏器（依赖 X11；失败则回执主控端并停推帧，避免刷屏告警）
                    if capturer.is_none() {
                        match capture::Capturer::new() {
                            Ok(c) => {
                                // 诊断画面发虚：打印被控实际抓屏分辨率。DPI 感知前(虚拟化)会偏小
                                // (如 1280x720)，感知后应为物理分辨率(如 1920x1080)。
                                let (cw, ch) = c.real_size();
                                tracing::info!("被控截屏器就绪 抓屏分辨率={cw}x{ch}");
                                capturer = Some(c);
                            }
                            Err(e) => {
                                if notified_for.as_deref() != Some(sid.as_str()) {
                                    let _ = from_ui_tx.send(net::FromUi::Notice {
                                        session_id: sid.clone(),
                                        text: format!("被控端截屏不可用：{e}。请确认在 X11 桌面会话下运行。"),
                                    });
                                    notified_for = Some(sid.clone());
                                }
                                tracing::warn!("截屏器构造失败（无显示器/X11？）：{e}，推帧禁用；WSLg 可设 OHMYDESK_FAKE_CAPTURE=1 验链路");
                                *active.lock().unwrap() = None;
                                continue;
                            }
                        }
                    }
                    let f = capturer.as_ref().unwrap().frame_q(&qp);
                    if f.is_ok() {
                        seq += 1;
                    }
                    f
                };
                match frame {
                    Ok((data, w, h)) => {
                        if from_ui_tx
                            .send(net::FromUi::Frame {
                                session_id: sid,
                                data,
                                w,
                                h,
                                seq,
                            })
                            .is_err()
                        {
                            break; // net 已退出
                        }
                    }
                    Err(e) => tracing::debug!("截帧失败：{e}"),
                }
            }
        });
    }

    // 控制信号消费：更新活跃会话
    while let Some(c) = ctrl_rx.recv().await {
        match c {
            net::CaptureCtrl::Start { session_id } => {
                *active.lock().unwrap() = Some(session_id);
            }
            net::CaptureCtrl::Stop => {
                *active.lock().unwrap() = None;
            }
        }
    }
}

/// 剪贴板双向同步 worker。
///
/// arboard 句柄非 Send 且 Linux/X11 下需常驻持有,故独占一个 std::thread;`last_synced` 线程私有,
/// 收(写本地)与发(poll)都更新它,值相同即不发 —— 单线程内无竞态、无 A→B→A 回弹。
/// 控制信号(Start/Stop/Incoming)经 tokio mpsc 转进线程的 std mpsc。
pub async fn consume_clipboard(
    mut ctrl_rx: tokio::sync::mpsc::UnboundedReceiver<net::ClipboardMsg>,
    from_ui_tx: tokio::sync::mpsc::UnboundedSender<net::FromUi>,
) {
    let active: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let (blk_tx, blk_rx) = std::sync::mpsc::channel::<String>(); // 对端写入文本

    // 剪贴板线程:独占 arboard,poll 本地 + 写对端。
    {
        let active = active.clone();
        std::thread::spawn(move || {
            if crate::capture::is_wayland_session() {
                tracing::warn!("Wayland 会话:剪贴板同步禁用(arboard 在 Wayland 不可靠)");
                return;
            }
            let mut clip = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("剪贴板不可用:{e},同步禁用");
                    return;
                }
            };
            let mut last_synced = String::new();
            let mut prev_active: Option<String> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                // 先处理对端写入(可能多条,取最后一条即可)。
                while let Ok(text) = blk_rx.try_recv() {
                    if clip.set_text(text.clone()).is_ok() {
                        last_synced = text;
                    }
                }
                let cur_active = active.lock().unwrap().clone();
                // 会话切换:新会话用当前剪贴板做基线(不推送会话前的旧内容);结束清空。
                if cur_active != prev_active {
                    last_synced = clip.get_text().unwrap_or_default();
                    prev_active = cur_active.clone();
                }
                let sid = match cur_active {
                    Some(s) => s,
                    None => continue,
                };
                let cur = match clip.get_text() {
                    Ok(t) => t,
                    Err(_) => continue, // 空剪贴板/非文本:跳过
                };
                if cur.len() > 256 * 1024 {
                    tracing::debug!("剪贴板文本过大({} 字节),跳过同步", cur.len());
                    last_synced = cur; // 记为已同步,避免反复命中
                    continue;
                }
                if should_push_clipboard(&cur, &last_synced) {
                    last_synced = cur.clone();
                    if from_ui_tx
                        .send(net::FromUi::ClipboardSync { session_id: sid, text: cur })
                        .is_err()
                    {
                        break; // net 已退出
                    }
                }
            }
        });
    }

    // 控制信号消费:更新活跃会话 / 转发对端写入。
    while let Some(m) = ctrl_rx.recv().await {
        match m {
            net::ClipboardMsg::Start { session_id } => {
                *active.lock().unwrap() = Some(session_id);
            }
            net::ClipboardMsg::Stop => {
                *active.lock().unwrap() = None;
            }
            net::ClipboardMsg::Incoming { text } => {
                let _ = blk_tx.send(text);
            }
        }
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::should_push_clipboard;

    #[test]
    fn 非空且变化_应推送() {
        assert!(should_push_clipboard("hello", ""));
        assert!(should_push_clipboard("world", "hello"));
    }
    #[test]
    fn 空文本_不推送() {
        assert!(!should_push_clipboard("", "hello"));
    }
    #[test]
    fn 未变化_不推送() {
        assert!(!should_push_clipboard("same", "same"));
    }
}

#[cfg(test)]
mod input_signal_tests {
    use super::{last_input_after, mark_input_now, now_ms};

    #[test]
    fn mark_后_last_input_after_为真() {
        let before = now_ms().saturating_sub(1);
        mark_input_now();
        assert!(last_input_after(before), "mark 后应晚于此前时刻");
        // 远未来时刻：不应晚于
        assert!(!last_input_after(now_ms() + 10_000));
    }
}
