//! 启动自提权（Windows UAC）。
//!
//! 背景：Windows 的 UIPI（User Interface Privilege Isolation）禁止低完整性级别进程向
//! 高完整性级别窗口（以管理员运行的程序、UAC 提权窗口、登录/安全桌面等）发送输入。
//! 故被控端若以普通权限运行，对这些窗口的键鼠注入会被系统静默丢弃——表现为「部分窗口点不动」。
//!
//! 策略：进程启动时检测自身是否已提权；未提权则用 `ShellExecuteW("runas")` 以管理员身份
//! 重启自身（触发 UAC 弹窗），成功后退出当前非提权本体；用户拒绝提权则降级继续以普通权限运行
//! （不阻断启动，只是无法操作受保护窗口）。设环境变量 `OHMYDESK_NO_ELEVATE=1` 可禁用此行为。
//!
//! 非 Windows 平台 [`ensure_elevated`] 为 no-op。

/// 确保（尽力）以管理员权限运行：未提权则自重启提权，成功即退出本体。
#[cfg(windows)]
pub fn ensure_elevated() {
    if std::env::var("OHMYDESK_NO_ELEVATE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return;
    }
    if is_elevated() {
        return;
    }
    if relaunch_as_admin() {
        // 提权副本已启动，退出非提权本体，避免两个实例同时反连。
        std::process::exit(0);
    }
    tracing::warn!("未获管理员权限（提权被拒或失败），继续以普通权限运行；对管理员/UAC 窗口的注入可能被系统拦截");
}

/// 非 Windows：无 UAC 概念，空实现。
#[cfg(not(windows))]
pub fn ensure_elevated() {}

/// 声明进程为 DPI 感知（Windows）。**必须在创建任何窗口/GDI/截屏之前调用**。
///
/// 背景：未声明 DPI 感知的进程在缩放 >100% 的 Windows 显示器（笔记本/多数桌面默认 125%/150%）
/// 上会被系统「DPI 虚拟化」——进程看到的是缩小后的逻辑分辨率，xcap 据此抓到的是低分辨率、被
/// GDI 拉伸的**模糊**画面（远程画面整体发虚的根因，且高清档也无高分辨率可抓）。声明 DPI 感知后，
/// xcap 抓到真实物理分辨率，画面清晰。
///
/// 用 `SetProcessDPIAware`（Vista+ 全版本存在，静态导入零启动风险）而非 PerMonitorV2
/// （需 Win10 1703+，静态导入在更老系统会导致进程无法启动）；对单显示器被控已足够消除发虚。
#[cfg(windows)]
pub fn set_dpi_aware() {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware;
    // 失败忽略：保持系统默认，不阻断启动。
    unsafe {
        let _ = SetProcessDPIAware();
    }
}

/// 非 Windows：无 DPI 虚拟化问题，空实现。
#[cfg(not(windows))]
pub fn set_dpi_aware() {}

/// 查询当前进程令牌是否已提权（TokenElevation）。
#[cfg(windows)]
fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false; // 拿不到令牌时保守按未提权处理（最多多弹一次 UAC，不会误判为已提权）
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 以管理员身份（"runas" 动词，触发 UAC）重启自身，透传原命令行参数。返回是否成功发起。
#[cfg(windows)]
fn relaunch_as_admin() -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let exe_w = to_wide(&exe.to_string_lossy());
    let verb_w = to_wide("runas");
    // 透传除 argv[0] 外的原始参数（保持重启后行为一致）。含空格的参数加双引号，
    // 避免被 ShellExecute 拆成多参（如带空格的用户名「演示 终端」）。
    let params = std::env::args()
        .skip(1)
        .map(quote_arg)
        .collect::<Vec<_>>()
        .join(" ");
    let params_w = to_wide(&params);

    // ShellExecuteW 返回值 > 32 表示成功（HINSTANCE 历史语义）。
    let h = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb_w.as_ptr(),
            exe_w.as_ptr(),
            if params.is_empty() {
                std::ptr::null()
            } else {
                params_w.as_ptr()
            },
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    (h as isize) > 32
}

/// 命令行参数加引号（仅当含空格/制表/已含引号时），转义内部双引号为 `\"`。
#[cfg(windows)]
fn quote_arg(a: String) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        format!("\"{}\"", a.replace('"', "\\\""))
    } else {
        a
    }
}

/// UTF-16 + NUL 结尾的宽字符串（Win32 W 系 API 入参）。
#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(all(windows, test))]
mod tests {
    use super::{quote_arg, to_wide};

    #[test]
    fn to_wide_以_nul_结尾且为_utf16() {
        let w = to_wide("ab");
        assert_eq!(w, vec![0x61, 0x62, 0x00]);
    }

    #[test]
    fn quote_arg_仅在含空格时加引号() {
        assert_eq!(quote_arg("plain".into()), "plain");
        assert_eq!(quote_arg("演示 终端".into()), "\"演示 终端\"");
        assert_eq!(quote_arg("a\"b".into()), "\"a\\\"b\"");
        assert_eq!(quote_arg(String::new()), "\"\"");
    }
}
