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
