//! 「AI助手」按钮后端:检测/静默安装/拉起素问(Suwen)。仅 Windows 有实体逻辑。
//!
//! 流程:检测 suwen-daemon.exe 是否存在 → 不存在则下载 suwen-setup.exe 静默安装(/S)
//! → 等安装器退出 + 轮询 daemon.exe 落盘 → 拉起 suwen-gui.exe。
//! 客户端启动即提权(main.rs ensure_elevated),安装器子进程继承管理员令牌,不二次弹 UAC。
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;

/// 素问安装器下载地址(带 /S 静默安装)。
pub const SETUP_URL: &str = "https://ai-agent.guoziweb.com/downloads/client/suwen-setup.exe";
/// 素问守护进程名(检测锚点)。
pub const DAEMON_EXE: &str = "suwen-daemon.exe";
/// 素问 GUI 进程名(拉起目标)。
pub const GUI_EXE: &str = "suwen-gui.exe";

// UI 相位:与 app.slint `suwen_phase` 取值一一对应。
pub const PHASE_IDLE: i32 = 0;
pub const PHASE_DOWNLOAD: i32 = 1;
pub const PHASE_INSTALL: i32 = 2;
pub const PHASE_LAUNCH: i32 = 3;
pub const PHASE_FAILED: i32 = 4;

/// 由 %ProgramFiles% / %ProgramFiles(x86)% 推出素问安装目录候选(纯函数,便于测试)。
/// 主目录在前、兜底 x86 在后;跳过 None/空串;去重。
pub fn candidate_dirs(pf: Option<String>, pf86: Option<String>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in [pf, pf86].into_iter().flatten() {
        if base.is_empty() {
            continue;
        }
        let d = PathBuf::from(base).join("Suwen");
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

/// 读环境变量得到安装目录候选。
fn install_dirs() -> Vec<PathBuf> {
    candidate_dirs(
        std::env::var("ProgramFiles").ok(),
        std::env::var("ProgramFiles(x86)").ok(),
    )
}

/// 在候选目录中查找已存在的指定 exe;返回第一个存在者。
fn find_exe(name: &str) -> Option<PathBuf> {
    install_dirs()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

/// 素问是否已安装(以 daemon.exe 存在为锚点)。
pub fn is_installed() -> bool {
    find_exe(DAEMON_EXE).is_some()
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 同 exec.rs:38,静默不弹黑框
    const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);
    const POLL_INTERVAL: Duration = Duration::from_millis(500);
    const DOWNLOAD_CAP: u64 = 50 * 1024 * 1024;

    /// 下载素问安装器到 %TEMP% 临时文件,返回临时路径。
    /// 复用 update::build_agent(必须显式 SChannel,否则 ureq 报 "no TLS backend")+ CapReader(50MB 上限)。
    ///
    /// **调用方须持有返回的 `TempPath` 直到 `run_installer` 的 `.status()` 返回**:
    /// `TempPath` 一旦 drop 即删除临时安装包。正确用法:`let setup = download_setup()?; run_installer(&setup)?;`
    /// 切勿写成 `run_installer(&download_setup()?)`——临时值会在语句结束即 drop,安装器运行中文件被删。
    pub fn download_setup() -> anyhow::Result<tempfile::TempPath> {
        let agent = crate::update::build_agent(10, 300);
        let resp = agent.get(SETUP_URL).call()?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".suwen-setup-")
            .suffix(".exe")
            .tempfile_in(std::env::temp_dir())?;
        let mut reader = crate::update::CapReader::new(resp.into_reader(), DOWNLOAD_CAP);
        std::io::copy(&mut reader, tmp.as_file_mut())?;
        Ok(tmp.into_temp_path())
    }

    /// 运行安装器静默安装(/S)。客户端已提权,子进程继承管理员令牌。等待退出并校验退出码。
    pub fn run_installer(setup: &Path) -> anyhow::Result<()> {
        let status = Command::new(setup)
            .arg("/S")
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        if !status.success() {
            anyhow::bail!("安装器退出码非 0:{status}");
        }
        Ok(())
    }

    /// 轮询等待 daemon.exe 落盘(安装真正完成),超时报错。
    /// 双保险:安装器 `.status()` 已等到进程退出,但仍轮询确认 daemon.exe 可见,
    /// 兜底"安装器早退但文件系统写入/可见性延迟"及安装器行为差异,避免过早拉起 GUI。
    pub fn wait_installed() -> anyhow::Result<()> {
        let deadline = Instant::now() + INSTALL_TIMEOUT;
        loop {
            if is_installed() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("安装超时:{}s 内未见 {DAEMON_EXE}", INSTALL_TIMEOUT.as_secs());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 拉起素问 GUI(不 wait、不加窗口 flag)。工作目录设为安装目录。
    pub fn launch_gui() -> anyhow::Result<()> {
        let gui = find_exe(GUI_EXE).ok_or_else(|| anyhow::anyhow!("未找到 {GUI_EXE}"))?;
        let mut cmd = Command::new(&gui);
        if let Some(dir) = gui.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn()?;
        Ok(())
    }
}

/// UI 线程调用:后台线程编排「检测→[下载→安装→等待]→拉起」,全程回写 suwen_phase/suwen_status。
/// 防重入:UI 侧按钮 enabled 仅在 phase 0/4 可点;此处 AtomicBool 二次兜底。
#[cfg(windows)]
pub fn ensure_and_launch(ui: slint::Weak<crate::AppWindow>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    // 函数内 static:全进程唯一实例(非每次调用新建),用作防重入门闩。
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return; // 已有任务在跑,忽略连点
    }
    std::thread::spawn(move || {
        // catch_unwind 兜底:release 为 panic=unwind,run_flow 调用链若 panic,
        // 线程静默退出会让 RUNNING 永停 true + phase 卡在进行中→按钮永久锁死。
        // 故三种结局(成功/错误/panic)都必须落到终态并在末尾释放门闩。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_flow(&ui)));
        match outcome {
            Ok(Ok(())) => set_state(&ui, PHASE_IDLE, String::new()),
            Ok(Err(e)) => {
                tracing::warn!("素问部署失败:{e:#}");
                set_state(&ui, PHASE_FAILED, format!("失败:{e}"));
            }
            Err(_) => {
                tracing::error!("素问部署线程 panic");
                set_state(&ui, PHASE_FAILED, "失败,点击重试".into());
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// 线性状态机主体。已安装则跳过下载/安装直接拉起。
#[cfg(windows)]
fn run_flow(ui: &slint::Weak<crate::AppWindow>) -> anyhow::Result<()> {
    if !is_installed() {
        set_state(ui, PHASE_DOWNLOAD, "正在下载素问…".into());
        let setup = win::download_setup()?;
        set_state(ui, PHASE_INSTALL, "正在安装素问…".into());
        win::run_installer(&setup)?;
        win::wait_installed()?;
        // setup(TempPath)在此 drop:安装已完成,可安全删除临时安装包。
    }
    set_state(ui, PHASE_LAUNCH, "正在启动素问…".into());
    win::launch_gui()?;
    Ok(())
}

/// 跨线程回写 UI 相位与状态文本(AppWindow 句柄非 Send,必须经事件循环)。
#[cfg(windows)]
fn set_state(ui: &slint::Weak<crate::AppWindow>, phase: i32, status: String) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui.upgrade() {
            // set_suwen_* 为 Slint 生成的固有方法;upgrade() 为 Weak 固有方法,均无需 ComponentHandle。
            ui.set_suwen_phase(phase);
            ui.set_suwen_status(status.into());
        }
    });
}

/// 非 Windows:按钮已被 suwen_supported=false 隐藏,此处兜底空实现(保证跨平台可编译)。
#[cfg(not(windows))]
pub fn ensure_and_launch(_ui: slint::Weak<crate::AppWindow>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_dirs_prefers_program_files_then_x86() {
        let dirs = candidate_dirs(
            Some(r"C:\Program Files".into()),
            Some(r"C:\Program Files (x86)".into()),
        );
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], PathBuf::from(r"C:\Program Files").join("Suwen"));
        assert_eq!(dirs[1], PathBuf::from(r"C:\Program Files (x86)").join("Suwen"));
    }

    #[test]
    fn candidate_dirs_skips_missing_and_empty() {
        assert!(candidate_dirs(None, Some(String::new())).is_empty());
        assert_eq!(
            candidate_dirs(Some(r"C:\PF".into()), None),
            vec![PathBuf::from(r"C:\PF").join("Suwen")]
        );
    }

    #[test]
    fn candidate_dirs_dedups_identical_bases() {
        let dirs = candidate_dirs(Some("/opt".into()), Some("/opt".into()));
        assert_eq!(dirs.len(), 1);
    }
}
