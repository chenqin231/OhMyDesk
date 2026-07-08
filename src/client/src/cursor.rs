//! 被控端光标采集：取当前系统鼠标光标的形状位图（裸 RGBA）+ 热点 + 位置，供光标同步。
//! - Windows：`GetCursorInfo` → `GetIconInfoExW` → GDI `GetDIBits`（BGRA→RGBA）。
//! - Linux/X11：XFixes `GetCursorImage`（ARGB→RGBA）。Wayland 无 X → 采集失败返回 None，优雅降级。
//! 形状按 RGBA 的 XxHash64 指纹去重：仅当形状 id 与主控已缓存的不同才带 shape，否则 shape=None
//! （主控复用缓存），大幅省带宽（光标≤32×32，且形状很少变）。
//!
//! 平台 FFI 无法在 headless Linux 上运行时验证（需真机）；纯逻辑（BGRA→RGBA、指纹去重）有单测覆盖。

use std::hash::Hasher;

/// 一次采集到的光标形状位图（裸 RGBA，行优先，top-down）。
#[derive(Debug, Clone)]
pub struct CapturedShape {
    pub id: u64,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// 一次光标采集结果。x/y 为屏幕坐标（real px）；调用方按帧缩放映射到帧坐标。
#[derive(Debug, Clone)]
pub struct CapturedCursor {
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    /// 形状：仅当形状 id 与上次（主控缓存）不同才 Some；否则 None（复用缓存）。
    pub shape: Option<CapturedShape>,
}

/// RGBA 像素指纹（XxHash64）：形状去重用。带上 w/h 避免不同尺寸偶合。
pub fn rgba_fingerprint(rgba: &[u8], w: u32, h: u32) -> u64 {
    let mut h64 = twox_hash::XxHash64::with_seed(0);
    h64.write_u32(w);
    h64.write_u32(h);
    h64.write(rgba);
    h64.finish()
}

/// BGRA（Windows GetDIBits 输出）→ RGBA：原地交换每像素的 B/R 通道。
pub fn bgra_to_rgba_inplace(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// 光标采集器：持有平台状态（X11 连接缓存，避免每 tick 重连）。在采集线程内 new 一次、逐 tick 调用。
pub struct CursorCapturer {
    #[cfg(target_os = "linux")]
    x11: Option<linux_impl::X11Cursor>,
    // 非 linux 无状态；用 () 占位保持结构体非空、字段私有。
    #[cfg(not(target_os = "linux"))]
    _priv: (),
}

impl Default for CursorCapturer {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorCapturer {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            x11: linux_impl::X11Cursor::connect(),
            #[cfg(not(target_os = "linux"))]
            _priv: (),
        }
    }

    /// 采集当前光标。last_id=主控已缓存的形状 id（去重：相同则 shape=None）。
    /// 无法采集（平台不支持/无光标/出错）返回 None，调用方跳过本次（不发 CursorUpdate）。
    pub fn capture(&mut self, last_id: Option<u64>) -> Option<CapturedCursor> {
        #[cfg(windows)]
        {
            let _ = &self;
            windows_impl::capture(last_id)
        }
        #[cfg(target_os = "linux")]
        {
            self.x11.as_mut().and_then(|c| c.capture(last_id))
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (self, last_id);
            None
        }
    }
}

// ── Windows 实现 ─────────────────────────────────────────────────────────────
#[cfg(windows)]
mod windows_impl {
    use super::{bgra_to_rgba_inplace, rgba_fingerprint, CapturedCursor, CapturedShape};
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorInfo, GetIconInfoExW, CURSORINFO, CURSOR_SHOWING, ICONINFOEXW,
    };

    pub fn capture(last_id: Option<u64>) -> Option<CapturedCursor> {
        unsafe {
            let mut ci: CURSORINFO = zeroed();
            ci.cbSize = size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) == 0 {
                return None;
            }
            let x = ci.ptScreenPos.x;
            let y = ci.ptScreenPos.y;
            let visible = (ci.flags & CURSOR_SHOWING) != 0;
            if ci.hCursor.is_null() || !visible {
                // 光标隐藏或无句柄：只报位置与可见性，主控隐藏同步光标。
                return Some(CapturedCursor {
                    x,
                    y,
                    visible: false,
                    shape: None,
                });
            }

            let mut ii: ICONINFOEXW = zeroed();
            ii.cbSize = size_of::<ICONINFOEXW>() as u32;
            if GetIconInfoExW(ci.hCursor, &mut ii) == 0 {
                return Some(CapturedCursor {
                    x,
                    y,
                    visible,
                    shape: None,
                });
            }

            let monochrome = ii.hbmColor.is_null();
            let dims_bmp = if monochrome { ii.hbmMask } else { ii.hbmColor };
            let mut bm: BITMAP = zeroed();
            if GetObjectW(
                dims_bmp as _,
                size_of::<BITMAP>() as i32,
                &mut bm as *mut _ as *mut core::ffi::c_void,
            ) == 0
            {
                cleanup(&ii);
                return Some(CapturedCursor {
                    x,
                    y,
                    visible,
                    shape: None,
                });
            }
            let w = bm.bmWidth.max(0) as u32;
            // 单色光标：mask 位图高度为 2*h（上半 AND、下半 XOR），真实光标高取一半。
            let h = if monochrome {
                (bm.bmHeight.max(0) as u32) / 2
            } else {
                bm.bmHeight.max(0) as u32
            };
            if w == 0 || h == 0 {
                cleanup(&ii);
                return None;
            }

            let hdc = GetDC(0 as HWND);
            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32); // 负高 = top-down，读回无需翻转
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB as u32;

            let mut buf = vec![0u8; (w * h * 4) as usize];
            let src_bmp = if monochrome { ii.hbmMask } else { ii.hbmColor };
            let scanned = GetDIBits(
                hdc,
                src_bmp as _,
                0,
                h,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                &mut bmi,
                DIB_RGB_COLORS,
            );
            ReleaseDC(0 as HWND, hdc);
            cleanup(&ii);
            if scanned == 0 {
                return Some(CapturedCursor {
                    x,
                    y,
                    visible,
                    shape: None,
                });
            }

            // GetDIBits 输出 BGRA → RGBA。
            bgra_to_rgba_inplace(&mut buf);

            // 彩色光标 32bpp 自带 alpha。单色光标无 alpha（GetDIBits 把 1bpp mask 扩成不透明黑白），
            // 退化处理：把纯黑像素视为透明（I 型/十字光标背景），其余不透明。真机可再精修 AND/XOR。
            if monochrome {
                for px in buf.chunks_exact_mut(4) {
                    px[3] = if px[0] == 0 && px[1] == 0 && px[2] == 0 {
                        0
                    } else {
                        255
                    };
                }
            }

            let id = rgba_fingerprint(&buf, w, h);
            if Some(id) == last_id {
                return Some(CapturedCursor {
                    x,
                    y,
                    visible,
                    shape: None,
                });
            }
            Some(CapturedCursor {
                x,
                y,
                visible,
                shape: Some(CapturedShape {
                    id,
                    hotspot_x: ii.xHotspot,
                    hotspot_y: ii.yHotspot,
                    w,
                    h,
                    rgba: buf,
                }),
            })
        }
    }

    unsafe fn cleanup(ii: &ICONINFOEXW) {
        if !ii.hbmColor.is_null() {
            DeleteObject(ii.hbmColor as _);
        }
        if !ii.hbmMask.is_null() {
            DeleteObject(ii.hbmMask as _);
        }
    }
}

// ── Linux/X11 实现 ───────────────────────────────────────────────────────────
#[cfg(target_os = "linux")]
mod linux_impl {
    use super::{rgba_fingerprint, CapturedCursor, CapturedShape};
    use x11rb::protocol::xfixes::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    pub struct X11Cursor {
        conn: RustConnection,
    }

    impl X11Cursor {
        /// 连接 X server 并初始化 XFixes 扩展。失败（无 X / Wayland）返回 None。
        pub fn connect() -> Option<Self> {
            let (conn, _screen) = RustConnection::connect(None).ok()?;
            // XFixes 必须先协商版本才能用 GetCursorImage。
            conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
            Some(Self { conn })
        }

        pub fn capture(&mut self, last_id: Option<u64>) -> Option<CapturedCursor> {
            let img = self.conn.xfixes_get_cursor_image().ok()?.reply().ok()?;
            let w = img.width as u32;
            let h = img.height as u32;
            if w == 0 || h == 0 {
                return None;
            }
            // XFixes cursor_image 为 u32 ARGB（每元素一像素）→ 展开为 RGBA 字节。
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for px in &img.cursor_image {
                let a = ((px >> 24) & 0xff) as u8;
                let r = ((px >> 16) & 0xff) as u8;
                let g = ((px >> 8) & 0xff) as u8;
                let b = (px & 0xff) as u8;
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            let x = img.x as i32;
            let y = img.y as i32;
            let id = rgba_fingerprint(&rgba, w, h);
            if Some(id) == last_id {
                return Some(CapturedCursor {
                    x,
                    y,
                    visible: true,
                    shape: None,
                });
            }
            Some(CapturedCursor {
                x,
                y,
                visible: true,
                shape: Some(CapturedShape {
                    id,
                    hotspot_x: img.xhot as u32,
                    hotspot_y: img.yhot as u32,
                    w,
                    h,
                    rgba,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_rgba_交换br通道() {
        let mut buf = vec![1u8, 2, 3, 4, 10, 20, 30, 40]; // 两像素 BGRA
        bgra_to_rgba_inplace(&mut buf);
        assert_eq!(buf, vec![3, 2, 1, 4, 30, 20, 10, 40]); // B/R 互换，G/A 不动
    }

    #[test]
    fn 指纹_相同像素相同id_不同像素不同id() {
        let a = vec![255u8, 0, 0, 255];
        let b = vec![255u8, 0, 0, 255];
        let c = vec![0u8, 255, 0, 255];
        assert_eq!(rgba_fingerprint(&a, 1, 1), rgba_fingerprint(&b, 1, 1));
        assert_ne!(rgba_fingerprint(&a, 1, 1), rgba_fingerprint(&c, 1, 1));
    }

    #[test]
    fn 指纹_同像素不同尺寸_不同id() {
        let px = vec![1u8, 2, 3, 4];
        assert_ne!(rgba_fingerprint(&px, 1, 1), rgba_fingerprint(&px, 2, 1));
    }
}
