//! 文件传输被控侧：接收下发(push)落盘 + 响应取回(pull)回流。分块 base64 over WS。
//!
//! 安全约束：单文件 ≤ [`MAX_FILE`]；文件名始终经 [`safe_name`] basename 化（防目录穿越）。
//! 接收目录：控制方在远端文件浏览器选定的 dest 目录（经 canonicalize 解析软链后须为现存目录），
//! dest 缺省/非法时回退固定目录 [`recv_dir`]；取回前校验路径为常规文件且未超限。
//! 注：已授权会话内，控制方可向被控端任意可写目录落文件、读取任意 ≤MAX_FILE 文件——
//! 这是远控文件管理的预期能力，由会话鉴权 + server 端审计约束。

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
/// `dest` 为控制方选定的目标目录（远端文件浏览器当前目录）；为 None 或不是已存在目录时
/// 回退到固定接收目录 [`recv_dir`]。文件名始终经 [`safe_name`] basename 化防穿越。
pub fn open_recv(
    transfer_id: &str,
    name: &str,
    size: u64,
    dest: Option<&str>,
) -> Result<PathBuf, String> {
    if size > MAX_FILE {
        return Err(format!("文件超过上限 {}MB", MAX_FILE / 1024 / 1024));
    }
    // 目标目录：dest 经 canonicalize 解析软链后须为现存目录，命中则用之，否则回退 recv_dir
    // （canonicalize 避免 dest 是指向别处的软链时落点与显示路径不一致）
    let dir = match dest {
        Some(d) if !d.trim().is_empty() => match std::fs::canonicalize(d) {
            Ok(real) if real.is_dir() => real,
            _ => recv_dir(),
        },
        _ => recv_dir(),
    };
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

/// 列出被控端某目录的条目（供主控端远端文件浏览）。
/// `path` 为空时回退到用户主目录；目录优先、其次按名称（不区分大小写）排序。
/// 返回 `(实际目录绝对路径, 条目列表)`；失败返回 `Err(reason)`，调用方回 `FileListResp{error}`。
pub fn list_dir(path: &str) -> Result<(String, Vec<protocol::FileEntry>), String> {
    let trimmed = path.trim();

    // Windows：空路径 = 「此电脑」盘符列表（无单一根，盘符各自为根；从盘内向上回到此列表）。
    #[cfg(windows)]
    if trimmed.is_empty() {
        return Ok(list_windows_drives());
    }

    let target: PathBuf = if trimmed.is_empty() {
        directories::UserDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .or_else(dirs_home_fallback)
            .ok_or("无法确定默认目录")?
    } else {
        PathBuf::from(path)
    };

    let canonical = std::fs::canonicalize(&target)
        .map_err(|e| format!("目录不可访问: {e}"))?;
    if !canonical.is_dir() {
        return Err("目标不是目录".into());
    }

    let mut entries: Vec<protocol::FileEntry> = Vec::new();
    let rd = std::fs::read_dir(&canonical).map_err(|e| format!("读取目录失败: {e}"))?;
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        // 元数据失败（权限/损坏链接）的条目跳过，不让整次列目录失败
        let Ok(meta) = item.metadata() else { continue };
        entries.push(protocol::FileEntry {
            name,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    // 目录优先；同类按名称不区分大小写排序
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok((display_path(&canonical), entries))
}

/// 规整返回给前端的目录路径：Windows 上 `canonicalize` 会加 `\\?\` 扩展长度前缀，剥掉以免前端
/// 显示/拼接出现 `\\?\C:\...`。非 Windows 原样返回。
fn display_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        return s.strip_prefix(r"\\?\").map(|x| x.to_string()).unwrap_or(s);
    }
    #[cfg(not(windows))]
    {
        s
    }
}

/// Windows 盘符列表（A:..Z: 中实际存在的），作为「此电脑」根；返回路径用空串标识该根。
#[cfg(windows)]
fn list_windows_drives() -> (String, Vec<protocol::FileEntry>) {
    let mut entries = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if std::path::Path::new(&root).is_dir() {
            entries.push(protocol::FileEntry {
                name: format!("{}:", letter as char),
                is_dir: true,
                size: 0,
            });
        }
    }
    (String::new(), entries)
}

/// home 目录兜底（UserDirs 在部分精简信创环境可能为 None）。
fn dirs_home_fallback() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
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
            dest: None,
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
        open_recv(tid, "demo.txt", payload.len() as u64, None).unwrap();
        assert_eq!(write_chunk(tid, &STANDARD.encode(a), false).unwrap(), None);
        let done = write_chunk(tid, &STANDARD.encode(b), true).unwrap();
        let path = done.expect("末块应返回最终路径");
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got, payload);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn 列目录_目录优先且能列出条目() {
        // 造一个临时目录：含 1 子目录 + 1 文件
        let base = std::env::temp_dir().join("ohmydesk-ls-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("subdir")).unwrap();
        std::fs::write(base.join("a.txt"), b"hi").unwrap();

        let (dir, entries) = list_dir(base.to_str().unwrap()).unwrap();
        assert!(dir.contains("ohmydesk-ls-test"));
        assert_eq!(entries.len(), 2);
        // 目录优先：第一个必是 subdir
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "subdir");
        let file = entries.iter().find(|e| e.name == "a.txt").unwrap();
        assert!(!file.is_dir);
        assert_eq!(file.size, 2);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn 列目录_不存在路径报错() {
        assert!(list_dir("/no/such/ohmydesk/path/xyz").is_err());
    }

    #[test]
    fn 接收_超声明大小被拒并清理() {
        let tid = "t-test-2";
        open_recv(tid, "small.bin", 4, None).unwrap();
        // 写远超 4 字节 + 容差(64KB) 的数据
        let big = vec![b'z'; CHUNK + 1024];
        let r = write_chunk(tid, &STANDARD.encode(&big), false);
        assert!(r.is_err());
        // 已清理：再写同 id 应报未知 id
        assert!(write_chunk(tid, &STANDARD.encode(b"x"), true).is_err());
    }
}
