#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeSize {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticePosition {
    pub x: i32,
    pub y: i32,
}

pub const NOTICE_SIZE: NoticeSize = NoticeSize {
    width: 340,
    height: 148,
};
pub const NOTICE_MARGIN: i32 = 18;

pub fn auto_dismiss_ms() -> Option<u64> {
    None
}

pub fn bottom_right_position(
    work_area: WorkArea,
    notice_size: NoticeSize,
    margin: i32,
) -> NoticePosition {
    let margin = margin.max(0);
    NoticePosition {
        x: (work_area.right - notice_size.width.max(1) - margin).max(work_area.left),
        y: (work_area.bottom - notice_size.height.max(1) - margin).max(work_area.top),
    }
}

pub fn desktop_bottom_right_position() -> Option<NoticePosition> {
    current_work_area().map(|area| bottom_right_position(area, NOTICE_SIZE, NOTICE_MARGIN))
}

#[cfg(windows)]
fn current_work_area() -> Option<WorkArea> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            &mut rect as *mut RECT as *mut core::ffi::c_void,
            0,
        )
    };
    if ok == 0 {
        return None;
    }

    Some(WorkArea {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

#[cfg(not(windows))]
fn current_work_area() -> Option<WorkArea> {
    None
}
