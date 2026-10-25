//! Windows screen capture via GDI BitBlt.
//!
//! The returned buffer is BGRA (what GDI hands us), which happens to be exactly
//! the format GPUI's DirectX atlas expects, so it can be uploaded with no conversion.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, ROP_CODE,
    SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// A frozen snapshot of one monitor.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGRA, top-down.
    pub bgra: Vec<u8>,
    /// Monitor origin in virtual-screen physical pixels.
    pub origin_x: i32,
    pub origin_y: i32,
    /// DPI scale (1.0 = 96 dpi).
    pub scale: f32,
}

impl Shot {
    pub fn logical_x(&self) -> f32 {
        self.origin_x as f32 / self.scale
    }

    pub fn logical_y(&self) -> f32 {
        self.origin_y as f32 / self.scale
    }

    pub fn logical_w(&self) -> f32 {
        self.width as f32 / self.scale
    }

    pub fn logical_h(&self) -> f32 {
        self.height as f32 / self.scale
    }

    /// BGRA -> RGBA, for clipboard / PNG export.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.bgra.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// 光标所在显示器的几何信息（不抓屏）。
///
/// 单独拆出来是给「合成背景」用的：演示截图需要和真实抓屏一样的面板尺寸、
/// 原点和 DPI 比例，但像素内容由程序自己画，不去碰桌面。
pub struct MonitorGeom {
    pub width: u32,
    pub height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale: f32,
}

pub fn cursor_monitor_geometry() -> Result<MonitorGeom, String> {
    let (cx, cy) = cursor_pos();
    let pt = POINT { x: cx, y: cy };

    unsafe {
        let monitor: HMONITOR = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return Err("MonitorFromPoint failed".into());
        }

        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err("GetMonitorInfoW failed".into());
        }

        let rect = info.rcMonitor;
        let width = (rect.right - rect.left).max(1) as u32;
        let height = (rect.bottom - rect.top).max(1) as u32;

        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = if dpi_x == 0 { 1.0 } else { dpi_x as f32 / 96.0 };

        Ok(MonitorGeom {
            width,
            height,
            origin_x: rect.left,
            origin_y: rect.top,
            scale,
        })
    }
}

/// Capture the monitor that currently contains the mouse cursor.
pub fn capture_cursor_monitor() -> Result<Shot, String> {
    let geom = cursor_monitor_geometry()?;
    let (width, height) = (geom.width, geom.height);
    let (rect_left, rect_top) = (geom.origin_x, geom.origin_y);

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return Err("CreateCompatibleDC failed".into());
        }
        let bitmap = CreateCompatibleBitmap(screen_dc, width as i32, height as i32);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let old = SelectObject(mem_dc, bitmap.into());

        // CAPTUREBLT is what makes layered / translucent windows show up.
        let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        let blt = BitBlt(
            mem_dc,
            0,
            0,
            width as i32,
            height as i32,
            Some(screen_dc),
            rect_left,
            rect_top,
            rop,
        );

        let mut bgra = vec![0u8; (width * height * 4) as usize];
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = width as i32;
        // Negative height requests a top-down DIB so row 0 is the top row.
        bi.bmiHeader.biHeight = -(height as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0 as u32;

        let scanlines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if blt.is_err() || scanlines == 0 {
            return Err(format!(
                "capture failed (blt_ok={}, scanlines={})",
                blt.is_ok(),
                scanlines
            ));
        }

        Ok(Shot {
            width,
            height,
            bgra,
            origin_x: rect_left,
            origin_y: rect_top,
            scale: geom.scale,
        })
    }
}
