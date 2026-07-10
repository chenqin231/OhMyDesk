//! 采集回调·输入域：键盘(on_key_ev/on_text)、指针移动/按键、滚轮、画质。
use super::UiCtx;
use crate::{net, AppWindow};

pub(super) fn wire(ui: &AppWindow, cx: &UiCtx) {
    // int 档位语义与 app.slint res_tier/clarity_tier/fps_tier 注释及数组顺序一一对应,改动需两侧同步
    // 主控切换三轴显示参数（分辨率/清晰度/帧率）→ 发 SetQuality 给被控端
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        ui.on_set_display_params(move |res, clarity, fps| {
            if let Some(sid) = sess.lock().unwrap().clone() {
                // 分辨率轴变化 → 请求主控窗口重贴合（清晰度/帧率不改帧尺寸，不触发）。
                if super::LAST_RES_TIER.swap(res, std::sync::atomic::Ordering::Relaxed) != res {
                    super::REFIT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let resolution = match res {
                    1 => protocol::ResolutionTier::R900p,
                    2 => protocol::ResolutionTier::R1080p,
                    3 => protocol::ResolutionTier::Native,
                    _ => protocol::ResolutionTier::R720p,
                };
                let clarity_t = match clarity {
                    1 => protocol::ClarityTier::High,
                    _ => protocol::ClarityTier::Standard,
                };
                let fps_t = match fps {
                    1 => protocol::FpsTier::Standard,
                    2 => protocol::FpsTier::Saver,
                    _ => protocol::FpsTier::Smooth,
                };
                // 旧被控端（≤0.5.0）兜底：mode 按清晰度映射
                let mode = if matches!(clarity_t, protocol::ClarityTier::High) {
                    protocol::QualityMode::HighQuality
                } else {
                    protocol::QualityMode::Smooth
                };
                let _ = tx.send(net::FromUi::SetQuality {
                    session_id: sid,
                    mode,
                    resolution: Some(resolution),
                    clarity: Some(clarity_t),
                    fps: Some(fps_t),
                });
            }
        });
    }
    // 键鼠捕获 → Input（坐标已是帧内像素，Slint 侧换算）
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        ui.on_on_pointer_move(move |x, y| {
            let sid = sess.lock().unwrap().clone();
            if let Some(sid) = sid {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::MouseMove { x, y },
                });
            }
        });
    }
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        ui.on_on_pointer_button(move |x, y, btn, down| {
            let sid = sess.lock().unwrap().clone();
            tracing::debug!(
                "主控采集·鼠标键 btn={btn} down={down} pos=({x},{y}) session={}",
                sid.as_deref().unwrap_or("<无>")
            );
            if let Some(sid) = sid {
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid.clone(),
                    event: protocol::InputEvent::MouseMove { x, y },
                });
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::MouseButton {
                        button: btn as u8,
                        down,
                    },
                });
            }
        });
    }
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        // 像素累加器:触摸板/惯性一次手势产生几十个小 delta,若每个都保底 ±1 会滚几十格(飞很远)。
        // 改为累加 px、满一格步长才发整数格、余量留到下次——发出的总格数 ≈ 物理滚动距离/步长,
        // 与事件个数无关。竖直/水平各自累加。
        let acc_x = std::cell::Cell::new(0.0f32);
        let acc_y = std::cell::Cell::new(0.0f32);
        ui.on_on_pointer_scroll(move |dx_px, dy_px| {
            const SCROLL_STEP_PX: f32 = 40.0;
            let take_notch = |acc: &std::cell::Cell<f32>, d: f32| -> i32 {
                let sum = acc.get() + d;
                let n = (sum / SCROLL_STEP_PX).trunc() as i32; // 满格数(向零取整)
                acc.set(sum - (n as f32) * SCROLL_STEP_PX); // 余量留到下次,不丢距离
                n
            };
            let dx = take_notch(&acc_x, dx_px);
            let dy = take_notch(&acc_y, dy_px);
            if dx == 0 && dy == 0 {
                return;
            }
            if let Some(sid) = sess.lock().unwrap().clone() {
                tracing::debug!("主控采集·滚轮 notch=({dx},{dy})");
                let _ = tx.send(net::FromUi::Input {
                    session_id: sid,
                    event: protocol::InputEvent::Scroll { dx, dy },
                });
            }
        });
    }
    {
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        // 采集侧键盘：分类器决定走 Key 通道还是交 TextInput 出 Text。
        // 返回 true → Slint accept（吃掉不本地编辑）；false → reject（交 edited→on_text）。
        ui.on_on_key_ev(move |text, ctrl, alt, meta, down| -> bool {
            match crate::key_route::key_route(&text, ctrl, alt, meta) {
                crate::key_route::KeyRoute::Key(code) => {
                    let sid = sess.lock().unwrap().clone();
                    tracing::debug!(
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
        let tx = cx.from_ui_tx.clone();
        let sess = cx.cur_session.clone();
        // IME/文本上屏串 → InputEvent::Text（被控 enigo.text() Unicode 直塞）
        ui.on_on_text(move |text| {
            if text.is_empty() {
                return;
            }
            let sid = sess.lock().unwrap().clone();
            tracing::debug!(
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
}
