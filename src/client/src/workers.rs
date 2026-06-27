//! 后台 worker（依赖 X11，阻塞 / `!Send`）：注入消费 / 截图消费 / 被控端推帧。
//!
//! enigo（注入）与 xcap（截屏）都是阻塞 X11 调用且句柄非 Send，故各自留在专用 `std::thread`
//! 内独占持有，不混进 async select；与 tokio 侧用 mpsc 通道交互（mpsc 的 send 同步非阻塞）。

use std::sync::Arc;

use crate::{capture, geom, inject, net};

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
    // 帧尺寸 = 被控端推帧的等比缩放尺寸（主控点击坐标基于此帧），注入按 real/frame 还原。
    let (frame_w, frame_h) = geom::scaled_dims(real_w, real_h, geom::MAX_W, geom::MAX_H);

    // 注入器在专用线程内构造并独占；用 std::sync::mpsc 把事件转进去。
    let (blk_tx, blk_rx) = std::sync::mpsc::channel::<protocol::InputEvent>();
    std::thread::spawn(move || {
        let mut injector = match inject::Injector::new(real_w, real_h, frame_w, frame_h) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("注入器构造失败（无 X11？）：{e}，注入禁用");
                return;
            }
        };
        while let Ok(ev) = blk_rx.recv() {
            if let Err(e) = injector.apply(&ev) {
                tracing::debug!("注入失败：{e}");
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
        let r = tokio::task::spawn_blocking(|| capture::Capturer::new().and_then(|c| c.frame())).await;
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
            let mut capturer: Option<capture::Capturer> = None;
            let mut seq: u64 = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(350));
                let sid = match active.lock().unwrap().clone() {
                    Some(s) => s,
                    None => continue, // 未在被控态，空转
                };
                // 懒构造截屏器（依赖 X11；失败则停推帧，避免刷屏告警）
                if capturer.is_none() {
                    match capture::Capturer::new() {
                        Ok(c) => capturer = Some(c),
                        Err(e) => {
                            tracing::warn!("截屏器构造失败（无显示器/X11？）：{e}，推帧禁用");
                            *active.lock().unwrap() = None;
                            continue;
                        }
                    }
                }
                match capturer.as_ref().unwrap().frame() {
                    Ok((data, w, h)) => {
                        seq += 1;
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
