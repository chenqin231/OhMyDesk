//! 文件传输被控侧：接收下发(push)落盘 + 响应取回(pull)回流。分块 base64 over WS。
//!
//! 安全约束：单文件 ≤ [`MAX_FILE`]；接收一律落到固定目录 [`recv_dir`] 且文件名经
//! [`safe_name`] basename 化（防目录穿越）；取回前校验路径为常规文件且未超限。

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use protocol::{Envelope, FileDir, Message};
use tokio::sync::mpsc::UnboundedSender;

/// 单文件大小上限：50MB。
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
/// 分块大小：64KB（取回回流用）。
pub const CHUNK: usize = 64 * 1024;

/// 在途的下发接收态：transfer_id → 写入句柄 + 计数。
struct RecvState {
    file: File,
    written: u64,
    size: u64,
    path: PathBuf,
}

static RECEIVERS: LazyLock<Mutex<HashMap<String, RecvState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 固定接收目录：`<配置目录>/recv`（信创 Linux 上 directories 会小写化 app 名）。
pub fn recv_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "OhMyDesk")
        .map(|d| d.config_dir().join("recv"))
        .unwrap_or_else(|| std::env::temp_dir().join("ohmydesk-recv"))
}

/// 文件名 basename 化：剥离任何路径分隔，拒绝 `.`/`..`，防目录穿越。
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if base.is_empty() || base == "." || base == ".." {
        "received.bin".to_string()
    } else {
        base.to_string()
    }
}

/// push 下发：打开接收文件。`Err(reason)` 时调用方回 `FileError`。
pub fn open_recv(transfer_id: &str, name: &str, size: u64) -> Result<PathBuf, String> {
    if size > MAX_FILE {
        return Err(format!("文件超过上限 {}MB", MAX_FILE / 1024 / 1024));
    }
    let dir = recv_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建接收目录失败: {e}"))?;
    let path = dir.join(safe_name(name));
    let file = File::create(&path).map_err(|e| format!("创建文件失败: {e}"))?;
    RECEIVERS.lock().unwrap().insert(
        transfer_id.to_string(),
        RecvState {
            file,
            written: 0,
            size,
            path: path.clone(),
        },
    );
    Ok(path)
}

/// 写一块（base64 解码后落盘）；`last` 时收尾并返回最终路径。
/// `Err(reason)` 时调用方回 `FileError`，本函数已清理半成品。
pub fn write_chunk(transfer_id: &str, data_b64: &str, last: bool) -> Result<Option<PathBuf>, String> {
    let bytes = STANDARD
        .decode(data_b64)
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    let mut map = RECEIVERS.lock().unwrap();
    let st = map.get_mut(transfer_id).ok_or("未知的传输 id")?;
    st.written += bytes.len() as u64;
    // 防超出声明大小（留一块容差）或硬上限
    if st.written > MAX_FILE || st.written > st.size.saturating_add(CHUNK as u64) {
        let path = st.path.clone();
        map.remove(transfer_id);
        drop(map);
        let _ = std::fs::remove_file(&path);
        return Err("写入超过声明大小/上限".into());
    }
    st.file
        .write_all(&bytes)
        .map_err(|e| format!("写入失败: {e}"))?;
    if last {
        let _ = st.file.flush();
        let path = st.path.clone();
        map.remove(transfer_id);
        return Ok(Some(path));
    }
    Ok(None)
}

/// 放弃一个在途接收（控制方发来 FileError 时清理）。
pub fn abort(transfer_id: &str) {
    RECEIVERS.lock().unwrap().remove(transfer_id);
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn send(out_tx: &UnboundedSender<String>, self_id: &str, payload: Message) {
    let env = Envelope {
        from: self_id.to_string(),
        to: None, // server 按 session_id route_to_peer 路由给控制方
        ts: now_ms(),
        payload,
    };
    if let Ok(s) = serde_json::to_string(&env) {
        let _ = out_tx.send(s);
    }
}

/// pull 取回：读取 `path` 文件并以 `FileOpen{dir:pull}` + `FileChunk` 流回控制方；
/// 失败回 `FileError`。在独立任务中调用（读 ≤50MB 文件进内存再分块）。
pub async fn send_file(
    out_tx: UnboundedSender<String>,
    self_id: String,
    session_id: String,
    transfer_id: String,
    path: String,
) {
    let err = |reason: String| {
        send(
            &out_tx,
            &self_id,
            Message::FileError {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                reason,
            },
        );
    };

    let p = Path::new(&path);
    match tokio::fs::metadata(p).await {
        Ok(m) if !m.is_file() => return err("目标不是常规文件".into()),
        Ok(m) if m.len() > MAX_FILE => {
            return err(format!("文件超过上限 {}MB", MAX_FILE / 1024 / 1024))
        }
        Err(e) => return err(format!("无法读取文件: {e}")),
        _ => {}
    }
    let bytes = match tokio::fs::read(p).await {
        Ok(b) => b,
        Err(e) => return err(format!("读取失败: {e}")),
    };
    let name = safe_name(p.file_name().and_then(|s| s.to_str()).unwrap_or("file.bin"));

    send(
        &out_tx,
        &self_id,
        Message::FileOpen {
            session_id: session_id.clone(),
            transfer_id: transfer_id.clone(),
            name,
            size: bytes.len() as u64,
            dir: FileDir::Pull,
        },
    );

    if bytes.is_empty() {
        send(
            &out_tx,
            &self_id,
            Message::FileChunk {
                session_id,
                transfer_id,
                seq: 0,
                data: String::new(),
                last: true,
            },
        );
        return;
    }
    let total = bytes.len();
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        let last = (i + 1) * CHUNK >= total;
        send(
            &out_tx,
            &self_id,
            Message::FileChunk {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                seq: i as u64,
                data: STANDARD.encode(chunk),
                last,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文件名_basename化防穿越() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_name("normal.txt"), "normal.txt");
        assert_eq!(safe_name(".."), "received.bin");
        assert_eq!(safe_name(""), "received.bin");
    }

    #[test]
    fn 接收_分块写盘并组装() {
        let tid = "t-test-1";
        let payload = b"hello-ohmydesk-file";
        // 拆 2 块
        let (a, b) = payload.split_at(5);
        open_recv(tid, "demo.txt", payload.len() as u64).unwrap();
        assert_eq!(write_chunk(tid, &STANDARD.encode(a), false).unwrap(), None);
        let done = write_chunk(tid, &STANDARD.encode(b), true).unwrap();
        let path = done.expect("末块应返回最终路径");
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got, payload);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn 接收_超声明大小被拒并清理() {
        let tid = "t-test-2";
        open_recv(tid, "small.bin", 4).unwrap();
        // 写远超 4 字节 + 容差(64KB) 的数据
        let big = vec![b'z'; CHUNK + 1024];
        let r = write_chunk(tid, &STANDARD.encode(&big), false);
        assert!(r.is_err());
        // 已清理：再写同 id 应报未知 id
        assert!(write_chunk(tid, &STANDARD.encode(b"x"), true).is_err());
    }
}
