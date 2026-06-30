//! 单次连接生命周期：连接 → 注册 → 出站泵 + 心跳 → 主循环 select 收发。

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use protocol::{EndpointInfo, Envelope, Message};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};

use super::dispatch::{handle_downlink, handle_uplink};
use super::{cur_ram, now, FromUi, ToUi};

/// 会话上下文：当前活跃会话 id + 本端在该会话的角色。
#[derive(Default)]
pub(super) struct SessionCtx {
    /// 主控态：本端作为发起方控制的会话 id（贴帧/发 Input 用）。
    pub(super) controlling: Option<String>,
    /// 被控态：本端被控制的会话 id（收 Input 注入用）。
    pub(super) controlled: Option<String>,
    /// 主控已取消/超时本次申请:收到迟到的 ConnectAck 时据此发 SessionEnd 收尾、不进主控态。
    pub(super) initiate_cancelled: bool,
}

/// 单次连接生命周期：连接 → 注册 → 起出站泵/心跳 → 收下行 + 处理 UI 上行。
pub(super) async fn connect_once(
    server_url: &str,
    info: &EndpointInfo,
    password: &Arc<std::sync::Mutex<String>>,
    to_ui: &mpsc::UnboundedSender<ToUi>,
    from_ui: &mut mpsc::UnboundedReceiver<FromUi>,
    telemetry_tx: &mpsc::UnboundedSender<crate::telemetry::TelemetryMsg>,
) -> anyhow::Result<()> {
    let (ws, _) = connect_async(server_url).await?;
    let (mut write, mut read) = ws.split();
    let id = info.id.clone();
    let cur_pw = || password.lock().unwrap().clone();

    // ── 出站泵：控制消息走可靠 FIFO out_tx；帧走 frame watch（单槽最新，drop-stale）──
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<(u64, String)>>(None);
    let tele_pump = telemetry_tx.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased; // 控制消息优先（input/心跳/控制绝不被帧延迟）
                ctrl = out_rx.recv() => {
                    match ctrl {
                        Some(text) => {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        None => break, // 控制通道关闭 = 连接结束
                    }
                }
                changed = frame_rx.changed() => {
                    if changed.is_err() {
                        // frame_tx 已 drop：转入纯控制循环直到关闭（不丢控制消息）
                        while let Some(text) = out_rx.recv().await {
                            if write.send(WsMsg::Text(text)).await.is_err() { break; }
                        }
                        break;
                    }
                    let latest = frame_rx.borrow_and_update().clone();
                    if let Some((seq, text)) = latest {
                        let t0 = std::time::Instant::now();
                        let res = write.send(WsMsg::Text(text)).await;
                        let stall = t0.elapsed().as_millis() as u32;
                        let ws_error = res.is_err();
                        let _ = tele_pump.send(crate::telemetry::TelemetryMsg::Egress(crate::telemetry::EgressSample {
                            seq, send_stall_ms: stall, sent_ok: !ws_error, ws_error,
                        }));
                        if ws_error { break; }
                    }
                }
            }
        }
    });

    // 注册（出站泵发，不直接碰 write）
    let reg_pw = cur_pw();
    let reg = Envelope {
        from: id.clone(),
        to: None,
        ts: now(),
        payload: Message::Register {
            info: Box::new(info.clone()),
            password: reg_pw.clone(),
        },
    };
    out_tx.send(serde_json::to_string(&reg)?)?;
    let _ = to_ui.send(ToUi::Registered {
        id: id.clone(),
        password: reg_pw.clone(),
    });
    tracing::info!("已注册 id={id} password={reg_pw}");
    crate::update::nudge();

    // ── 心跳任务：只持 out_tx（克隆），绝不持 write ──
    let hb_tx = out_tx.clone();
    let hb_id = id.clone();
    let hb = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let env = Envelope {
                from: hb_id.clone(),
                to: None,
                ts: now(),
                payload: Message::Heartbeat {
                    id: hb_id.clone(),
                    ram: cur_ram(),
                },
            };
            match serde_json::to_string(&env) {
                Ok(s) => {
                    if hb_tx.send(s).is_err() {
                        break; // 出站泵已关 → 退出心跳
                    }
                }
                Err(_) => break, // 序列化失败 → 退出心跳
            }
        }
    });

    // 主控/被控会话上下文（当前活跃 session + 角色）。Arc<Mutex> 供下行与 UI 分支共享。
    let session = Arc::new(tokio::sync::Mutex::new(SessionCtx::default()));

    // ── 主循环：select 收 下行帧 / UI 上行 ──
    let result = loop {
        tokio::select! {
            // 下行：server/对端来的消息
            msg = read.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(t))) => {
                        if let Err(e) = handle_downlink(&t, &id, &out_tx, to_ui, &session).await {
                            tracing::debug!("处理下行失败：{e}");
                        }
                    }
                    Some(Ok(WsMsg::Close(_))) | None => break Ok(()),
                    Some(Ok(_)) => {} // ping/pong/binary 忽略
                    Some(Err(e)) => break Err(anyhow::anyhow!(e)),
                }
            }
            // 上行：UI 动作
            act = from_ui.recv() => {
                match act {
                    // 刷新密码：就地重生成 + 重发 Register（server upsert 覆盖），并回推 UI 展示。
                    Some(FromUi::RefreshPassword) => {
                        let newpw = format!("{:06}", super::rand_6());
                        *password.lock().unwrap() = newpw.clone();
                        let reg = Envelope {
                            from: id.clone(),
                            to: None,
                            ts: now(),
                            payload: Message::Register {
                                info: Box::new(info.clone()),
                                password: newpw.clone(),
                            },
                        };
                        if let Ok(s) = serde_json::to_string(&reg) {
                            let _ = out_tx.send(s);
                        }
                        let _ = to_ui.send(ToUi::Registered { id: id.clone(), password: newpw });
                    }
                    // 帧走单槽 watch：网络慢时陈旧帧被覆盖，只发最新（drop-stale 核心）。
                    Some(FromUi::Frame { session_id, data, w, h, seq }) => {
                        let env = Envelope {
                            from: id.clone(),
                            to: None,
                            ts: now(),
                            payload: Message::Frame { session_id, data, w, h, seq },
                        };
                        if let Ok(s) = serde_json::to_string(&env) {
                            let _ = frame_tx.send_replace(Some((seq, s)));
                        }
                    }
                    Some(a) => handle_uplink(a, &id, &out_tx, &session).await,
                    None => break Ok(()), // UI 关闭
                }
            }
        }
    };

    // 清理：停泵/心跳
    hb.abort();
    drop(out_tx);
    drop(frame_tx); // 关帧 lane，泵转入纯控制收尾后退出
    let _ = pump.await;
    result
}
