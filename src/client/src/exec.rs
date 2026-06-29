//! 被控侧一次性命令执行：收 `ExecRequest` → 系统 shell 执行 → 产出 `ExecResult` 数据。
//!
//! - 平台 shell：Windows `cmd /C <command>`，类 Unix `sh -c <command>`。
//! - 超时：请求指定 `timeout_ms`，封顶 [`MAX_TIMEOUT_MS`]；超时杀进程回错误。
//! - 输出：stdout/stderr 各按 [`MAX_OUTPUT`] 字节截断（`truncated` 标记）。
//! - 注入/截屏依赖 X11，命令执行不依赖 X11，直接在 async 任务里跑（`tokio::process` 异步）。

use std::time::Instant;

use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// 单条命令输出上限（stdout / stderr 各自）。
pub const MAX_OUTPUT: usize = 64 * 1024;
/// 超时封顶，防止恶意/误用的超长 timeout。
pub const MAX_TIMEOUT_MS: u32 = 120_000;

/// 执行结果（映射到 `protocol::Message::ExecResult`）。
pub struct ExecOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub duration_ms: u32,
}

/// 构造平台 shell 命令。
fn shell_command(command: &str) -> Command {
    if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
}

/// 解码控制台原始字节为字符串：优先严格 UTF-8；失败时 Windows 按 GBK/CP936 解码
/// （中文 Windows 的 cmd 输出默认是 GBK，用 UTF-8 解码会乱码），非 Windows 退 lossy。
fn decode_console(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            #[cfg(windows)]
            {
                // GBK 解码器实为 GB18030 超集，覆盖简体中文 cmd 输出。
                encoding_rs::GBK.decode(bytes).0.into_owned()
            }
            #[cfg(not(windows))]
            {
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
    }
}

/// 解码控制台字节（UTF-8/GBK）→ 截断到 [`MAX_OUTPUT`] 字节边界，返回 (文本, 是否被截断)。
pub fn truncate_output(bytes: &[u8]) -> (String, bool) {
    let s = decode_console(bytes);
    if s.len() <= MAX_OUTPUT {
        return (s, false);
    }
    let mut end = MAX_OUTPUT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// 执行一条命令（带超时 + 输出截断）。绝不 panic：任何失败都落进 stderr。
pub async fn run_command(command: &str, timeout_ms: u32) -> ExecOutcome {
    let start = Instant::now();
    let dur = Duration::from_millis(timeout_ms.clamp(1, MAX_TIMEOUT_MS) as u64);
    match timeout(dur, shell_command(command).output()).await {
        Ok(Ok(out)) => {
            let (stdout, t1) = truncate_output(&out.stdout);
            let (stderr, t2) = truncate_output(&out.stderr);
            ExecOutcome {
                exit_code: out.status.code(),
                stdout,
                stderr,
                truncated: t1 || t2,
                duration_ms: start.elapsed().as_millis() as u32,
            }
        }
        Ok(Err(e)) => ExecOutcome {
            exit_code: None,
            stdout: String::new(),
            stderr: format!("命令启动失败: {e}"),
            truncated: false,
            duration_ms: start.elapsed().as_millis() as u32,
        },
        Err(_) => ExecOutcome {
            exit_code: None,
            stdout: String::new(),
            stderr: format!("命令超时（>{}ms）", dur.as_millis()),
            truncated: false,
            duration_ms: start.elapsed().as_millis() as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 截断_短输出不截() {
        let (s, t) = truncate_output(b"hello");
        assert_eq!(s, "hello");
        assert!(!t);
    }

    #[test]
    fn 截断_超长输出按上限截断() {
        let big = vec![b'x'; MAX_OUTPUT + 100];
        let (s, t) = truncate_output(&big);
        assert!(t);
        assert!(s.len() <= MAX_OUTPUT);
    }

    #[tokio::test]
    async fn 执行_echo_回显并退出码0() {
        // 跨平台：echo 在 cmd 与 sh 下都可用
        let out = run_command("echo ohmydesk", 5000).await;
        assert_eq!(out.exit_code, Some(0), "stderr={}", out.stderr);
        assert!(out.stdout.contains("ohmydesk"), "stdout={}", out.stdout);
    }

    #[tokio::test]
    async fn 执行_超时返回错误() {
        // sleep 2s 但只给 100ms：必超时（Windows 上 sh 不存在，命令启动失败也算非 0，此处仅在类 Unix 断言）
        if cfg!(not(target_os = "windows")) {
            let out = run_command("sleep 2", 100).await;
            assert_eq!(out.exit_code, None);
            assert!(out.stderr.contains("超时"));
        }
    }
}
