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

/// 主控端取回保存目录：transfer_id → 本地保存目录。
/// PullFile 上行时登记，被控端 FileOpen{dir:pull} 回流首包到达时取出（一次取用即移除）。
static PULL_TARGETS: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 登记一次取回的本地保存目录（主控端发 FilePullRequest 前调用）。
pub fn set_pull_target(transfer_id: &str, dir: PathBuf) {
    PULL_TARGETS
        .lock()
        .unwrap()
        .insert(transfer_id.to_string(), dir);
}

/// 取回目标目录的副本（peek，不移除）。文件夹取回会按同一 transfer_id 顺序回流多个文件，
/// 每个文件的回流首包都要查到同一本地基目录，故不能取一次就删；条目在会话结束时统一清理。
pub fn peek_pull_target(transfer_id: &str) -> Option<PathBuf> {
    PULL_TARGETS.lock().unwrap().get(transfer_id).cloned()
}

/// 清空全部取回目标登记（会话结束调用）。客户端同一时刻只有一个主控会话，整表清理安全，
/// 规避「按 transfer_id 逐条清理时无从判断文件夹是否传完」的难题。
pub fn clear_pull_targets() {
    PULL_TARGETS.lock().unwrap().clear();
}

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

/// 把传入的名字规整为「安全的相对路径」：按 `/` 和 `\` 切分，丢弃空段、`.`、`..` 及带盘符冒号
/// 的段（防目录穿越 / 绝对路径 / 盘符逃逸），其余段用 [`PathBuf::push`] 逐段重组为相对路径。
/// 单纯 basename（无分隔符）原样返回，与 [`safe_name`] 行为一致 → 向后兼容单文件传输；
/// 文件夹传输时承载子目录结构（如 `"docs/a/b.txt"`），收方据此在落点下重建层级。
/// 规整后为空 → `"received.bin"`。
pub fn safe_rel_path(name: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for seg in name.split(['/', '\\']) {
        let seg = seg.trim();
        // 跳过：空段、当前/上级目录、含冒号段（Windows 盘符如 "C:"，防盘符逃逸）。
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(':') {
            continue;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        out.push("received.bin");
    }
    out
}

/// 递归遍历 `root` 目录，收集其下所有常规文件，产出 `(相对展示路径, 绝对路径)`。
/// 相对路径以 `prefix` 起头（首次传被选文件夹名，使收方重建该文件夹本身）；用 `/` 作分隔，
/// 收方 [`safe_rel_path`] 再按本机分隔符重组。`DirEntry::metadata` 不跟随软链 → 软链目录/文件
/// 不计入，天然防环。空子目录不产出条目（不重建空目录，可接受）。
fn walk_files(root: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for item in rd.flatten() {
        let Ok(meta) = item.metadata() else { continue };
        let name = item.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if meta.is_dir() {
            walk_files(&item.path(), &rel, out);
        } else if meta.is_file() {
            out.push((rel, item.path()));
        }
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
    // 用 safe_rel_path 而非 safe_name：单文件时等价 basename（向后兼容），文件夹传输时保留
    // `name` 承载的子目录结构（如 "docs/a/b.txt"）在落点下重建层级；先建好父目录再创建文件。
    let path = dir.join(safe_rel_path(name));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建子目录失败: {e}"))?;
    }
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
pub fn write_chunk(
    transfer_id: &str,
    data_b64: &str,
    last: bool,
) -> Result<Option<PathBuf>, String> {
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

    let canonical = std::fs::canonicalize(&target).map_err(|e| format!("目录不可访问: {e}"))?;
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
        s.strip_prefix(r"\\?\").map(|x| x.to_string()).unwrap_or(s)
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

/// 发送单个文件：发 `FileOpen`(带 name/size/dir/dest) + 分块 `FileChunk`。
/// `name` 已是收方可直接用的展示名（单文件=basename，文件夹成员=相对路径）。
/// 读失败/超限返回 `Err(reason)`，由调用方决定回 `FileError`（单文件）还是跳过（文件夹成员）。
#[allow(clippy::too_many_arguments)]
async fn send_one_file(
    out_tx: &UnboundedSender<String>,
    self_id: &str,
    session_id: &str,
    transfer_id: &str,
    name: String,
    abs_path: &Path,
    dir: FileDir,
    dest: Option<String>,
) -> Result<(), String> {
    let meta = tokio::fs::metadata(abs_path)
        .await
        .map_err(|e| format!("无法读取文件: {e}"))?;
    if !meta.is_file() {
        return Err("目标不是常规文件".into());
    }
    if meta.len() > MAX_FILE {
        return Err(format!("文件超过上限 {}MB", MAX_FILE / 1024 / 1024));
    }
    let bytes = tokio::fs::read(abs_path)
        .await
        .map_err(|e| format!("读取失败: {e}"))?;

    send(
        out_tx,
        self_id,
        Message::FileOpen {
            session_id: session_id.to_string(),
            transfer_id: transfer_id.to_string(),
            name,
            size: bytes.len() as u64,
            dir,
            dest,
        },
    );

    if bytes.is_empty() {
        send(
            out_tx,
            self_id,
            Message::FileChunk {
                session_id: session_id.to_string(),
                transfer_id: transfer_id.to_string(),
                seq: 0,
                data: String::new(),
                last: true,
            },
        );
        return Ok(());
    }
    let total = bytes.len();
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        let last = (i + 1) * CHUNK >= total;
        send(
            out_tx,
            self_id,
            Message::FileChunk {
                session_id: session_id.to_string(),
                transfer_id: transfer_id.to_string(),
                seq: i as u64,
                data: STANDARD.encode(chunk),
                last,
            },
        );
    }
    Ok(())
}

/// 发送整个目录（取回/下发通用）：递归遍历 → 按**同一 transfer_id 顺序**逐文件发送，
/// `name` 承载相对路径（含被选文件夹名作首段，使收方重建该文件夹本身）。单文件超限/读失败
/// 则跳过并继续，不因一个文件中断整目录；空目录回一条 `FileError` 文案让控制方有反馈。
async fn send_dir(
    out_tx: &UnboundedSender<String>,
    self_id: &str,
    session_id: &str,
    transfer_id: &str,
    dir_path: &Path,
    dir: FileDir,
    dest: Option<String>,
) {
    let root = dir_path.to_path_buf();
    let folder = root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "folder".to_string());
    // 遍历是阻塞 IO，放 spawn_blocking，避免占用 async 工作线程。
    let files = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        walk_files(&root, &folder, &mut out);
        out
    })
    .await
    .unwrap_or_default();

    if files.is_empty() {
        send(
            out_tx,
            self_id,
            Message::FileError {
                session_id: session_id.to_string(),
                transfer_id: transfer_id.to_string(),
                reason: "文件夹为空或无可传输文件".into(),
            },
        );
        return;
    }
    for (rel, abs) in files {
        // 单文件失败/超限跳过，继续其余（文件夹整体尽力而为）。
        let _ = send_one_file(
            out_tx,
            self_id,
            session_id,
            transfer_id,
            rel,
            &abs,
            dir,
            dest.clone(),
        )
        .await;
    }
}

/// pull 取回：读取 `path`（文件或目录）以 `FileOpen{dir:pull}` + `FileChunk` 流回控制方；
/// 文件夹则递归逐文件回流（见 [`send_dir`]）。失败回 `FileError`。在独立任务中调用。
pub async fn send_file(
    out_tx: UnboundedSender<String>,
    self_id: String,
    session_id: String,
    transfer_id: String,
    path: String,
) {
    let p = Path::new(&path);
    let meta = match tokio::fs::metadata(p).await {
        Ok(m) => m,
        Err(e) => {
            send(
                &out_tx,
                &self_id,
                Message::FileError {
                    session_id,
                    transfer_id,
                    reason: format!("无法读取: {e}"),
                },
            );
            return;
        }
    };
    if meta.is_dir() {
        send_dir(
            &out_tx,
            &self_id,
            &session_id,
            &transfer_id,
            p,
            FileDir::Pull,
            None,
        )
        .await;
    } else {
        let name = safe_name(p.file_name().and_then(|s| s.to_str()).unwrap_or("file.bin"));
        if let Err(reason) = send_one_file(
            &out_tx,
            &self_id,
            &session_id,
            &transfer_id,
            name,
            p,
            FileDir::Pull,
            None,
        )
        .await
        {
            send(
                &out_tx,
                &self_id,
                Message::FileError {
                    session_id,
                    transfer_id,
                    reason,
                },
            );
        }
    }
}

/// push 下发：读取主控本机 `local_path`（文件或目录），以 `FileOpen{dir:push, dest}` + `FileChunk`
/// 流给被控端；文件夹则递归逐文件下发（见 [`send_dir`]）。镜像 [`send_file`]，方向为 Push 且带
/// 目标目录 dest。失败回 `FileError`。在独立任务中调用。
pub async fn send_file_push(
    out_tx: UnboundedSender<String>,
    self_id: String,
    session_id: String,
    transfer_id: String,
    local_path: String,
    dest_dir: String,
) {
    let p = Path::new(&local_path);
    let meta = match tokio::fs::metadata(p).await {
        Ok(m) => m,
        Err(e) => {
            send(
                &out_tx,
                &self_id,
                Message::FileError {
                    session_id,
                    transfer_id,
                    reason: format!("无法读取: {e}"),
                },
            );
            return;
        }
    };
    if meta.is_dir() {
        send_dir(
            &out_tx,
            &self_id,
            &session_id,
            &transfer_id,
            p,
            FileDir::Push,
            Some(dest_dir),
        )
        .await;
    } else {
        let name = safe_name(p.file_name().and_then(|s| s.to_str()).unwrap_or("file.bin"));
        if let Err(reason) = send_one_file(
            &out_tx,
            &self_id,
            &session_id,
            &transfer_id,
            name,
            p,
            FileDir::Push,
            Some(dest_dir),
        )
        .await
        {
            send(
                &out_tx,
                &self_id,
                Message::FileError {
                    session_id,
                    transfer_id,
                    reason,
                },
            );
        }
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

    #[test]
    fn pull_target_peek不消费() {
        // 用唯一 tid，避免与并发用例 / 全表 clear 串扰（故本用例不调用 clear_pull_targets）。
        let tid = "t-pull-peek-uniq";
        let dir = std::env::temp_dir().join("ohmydesk-pull-target");
        set_pull_target(tid, dir.clone());
        // peek 不移除：文件夹多文件回流要按同一 transfer_id 多次查同一目标目录。
        assert_eq!(peek_pull_target(tid), Some(dir.clone()));
        assert_eq!(peek_pull_target(tid), Some(dir), "peek 不应移除条目");
    }

    #[test]
    fn 安全相对路径_保留子目录并拦穿越() {
        let p = |segs: &[&str]| segs.iter().collect::<PathBuf>();
        // basename 原样（向后兼容单文件）
        assert_eq!(safe_rel_path("a.txt"), PathBuf::from("a.txt"));
        // 保留子目录层级；反斜杠归一
        assert_eq!(safe_rel_path("docs/a/b.txt"), p(&["docs", "a", "b.txt"]));
        assert_eq!(safe_rel_path("docs\\a.txt"), p(&["docs", "a.txt"]));
        // 拦 ..（丢弃后仍是相对、落在 dest 下，安全）
        assert_eq!(safe_rel_path("../../etc/passwd"), p(&["etc", "passwd"]));
        // 拦绝对路径前导分隔
        assert_eq!(safe_rel_path("/abs/x"), p(&["abs", "x"]));
        // 拦 Windows 盘符段
        assert_eq!(safe_rel_path("C:\\Windows\\evil"), p(&["Windows", "evil"]));
        // 规整后为空 → 兜底名
        assert_eq!(safe_rel_path(".."), PathBuf::from("received.bin"));
        assert_eq!(safe_rel_path(""), PathBuf::from("received.bin"));
    }

    #[tokio::test]
    async fn 取回文件夹_逐文件回流且name带相对路径() {
        // 造：base/myfolder/{a.txt, sub/b.txt}
        let base = std::env::temp_dir().join("ohmydesk-pull-folder-src");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("myfolder/sub")).unwrap();
        std::fs::write(base.join("myfolder/a.txt"), b"AA").unwrap();
        std::fs::write(base.join("myfolder/sub/b.txt"), b"BBB").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file(
            tx,
            "ep".into(),
            "s".into(),
            "tx".into(),
            base.join("myfolder").to_string_lossy().to_string(),
        )
        .await;

        // 收集所有 FileOpen 的 name：应含文件夹名作首段，保留子目录层级。
        let mut names = Vec::new();
        while let Ok(s) = rx.try_recv() {
            let env: Envelope = serde_json::from_str(&s).unwrap();
            if let Message::FileOpen { name, dir, .. } = env.payload {
                assert_eq!(dir, FileDir::Pull, "取回方向应为 Pull");
                names.push(name);
            }
        }
        names.sort();
        assert_eq!(
            names,
            vec![
                "myfolder/a.txt".to_string(),
                "myfolder/sub/b.txt".to_string()
            ]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn 取回空文件夹_回file_error提示() {
        let base = std::env::temp_dir().join("ohmydesk-pull-empty");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("empty")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file(
            tx,
            "ep".into(),
            "s".into(),
            "tx".into(),
            base.join("empty").to_string_lossy().to_string(),
        )
        .await;
        let s = rx.recv().await.expect("空文件夹应回一条提示");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(env.payload, Message::FileError { .. }));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn 下发_推送首包为_file_open_push_带_dest() {
        // 造一个本机小文件
        let base = std::env::temp_dir().join("ohmydesk-push-src");
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("up.txt");
        std::fs::write(&f, b"push-payload").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file_push(
            tx,
            "ep-self".into(),
            "s-1".into(),
            "tx-1".into(),
            f.to_string_lossy().to_string(),
            "/remote/dest/dir".into(),
        )
        .await;

        // 首包：FileOpen{dir:push, dest:Some, name=up.txt, size=12}
        let s = rx.recv().await.expect("应有 FileOpen 首包");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileOpen {
                dir,
                dest,
                name,
                size,
                ..
            } => {
                assert_eq!(dir, FileDir::Push);
                assert_eq!(dest.as_deref(), Some("/remote/dest/dir"));
                assert_eq!(name, "up.txt");
                assert_eq!(size, 12);
            }
            other => panic!("首包应为 FileOpen，实际 {other:?}"),
        }
        // 次包：FileChunk last=true（小文件单块）
        let s = rx.recv().await.expect("应有 FileChunk");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        match env.payload {
            Message::FileChunk { last, data, .. } => {
                assert!(last, "12 字节单块即末块");
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .unwrap();
                assert_eq!(raw, b"push-payload");
            }
            other => panic!("次包应为 FileChunk，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn 下发_文件不存在回_file_error() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        send_file_push(
            tx,
            "ep-self".into(),
            "s-1".into(),
            "tx-err".into(),
            "/no/such/ohmydesk/file.bin".into(),
            "/remote/dir".into(),
        )
        .await;
        let s = rx.recv().await.expect("应有 FileError");
        let env: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(env.payload, Message::FileError { .. }));
    }
}
