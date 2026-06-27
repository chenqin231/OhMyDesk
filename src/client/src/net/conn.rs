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
}

/// 单次连接生命周期：连接 → 注册 → 起出站泵/心跳 → 收下行 + 处理 UI 上行。
pub(super) async fn connect_once(
    server_url: &str,
    info: &EndpointInfo,
    password: &str,
    to_ui: &mpsc::UnboundedSender<ToUi>,
    from_ui: &mut mpsc::UnboundedReceiver<FromUi>,
) -> anyhow::Result<()> {
    let (ws, _) = connect_async(server_url).await?;
    let (mut write, mut read) = ws.split();
    let id = info.id.clone();

    // ── M-CLI1：mpsc 出站泵 —— write 只被本任务独占 ──
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let pump = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if write.send(WsMsg::Text(text)).await.is_err() {
                break;
            }
        }
    });

    // 注册（出站泵发，不直接碰 write）
    let reg = Envelope {
        from: id.clone(),
        to: None,
        ts: now(),
        payload: Message::Register {
            info: Box::new(info.clone()),
            password: password.to_string(),
        },
    };
    out_tx.send(serde_json::to_string(&reg)?)?;
    let _ = to_ui.send(ToUi::Registered {
        id: id.clone(),
        password: password.to_string(),
    });
    tracing::info!("已注册 id={id} password={password}");

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
                    Some(a) => handle_uplink(a, &id, &out_tx, &session).await,
                    None => break Ok(()), // UI 关闭
                }
            }
        }
    };

    // 清理：停泵/心跳
    hb.abort();
    drop(out_tx);
    let _ = pump.await;
    result
}
