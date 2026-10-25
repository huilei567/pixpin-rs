//! 平台集成层：抓屏 / 托盘 / 全局热键 / 窗口管理。
//!
//! GPUI 只覆盖 UI 层，这四样系统能力必须直接调 OS API —— Windows 上是
//! Win32（`win32/` 子模块），Linux/macOS 目前是桩实现（`stub.rs`），签名
//! 与 Windows 版完全一致，调用方无感知。桩会返回明确的错误或 no-op，
//! 等待跨平台 crate（xcap / tray-icon / global-hotkey）替换。

pub mod diag;
pub mod icons;

#[cfg(windows)]
pub mod win32;

#[cfg(not(windows))]
pub mod stub;

#[cfg(windows)]
pub use win32::{capture, hotkey, tray, winpick, winutil};

#[cfg(not(windows))]
pub use stub::{capture, hotkey, tray, winpick, winutil};

/// 托盘菜单命令：托盘线程 → 主线程。
#[derive(Clone, Copy, Debug)]
pub enum TrayCommand {
    Capture,
    Settings,
    Quit,
}

/// 一块显示器的几何信息（物理像素）。
#[derive(Clone, Copy, Debug)]
pub struct MonitorGeom {
    pub width: u32,
    pub height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    /// DPI scale (1.0 = 96 dpi)。
    pub scale: f32,
}

/// 冻结的屏幕快照（单显示器）。
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// 紧密排列的 BGRA，自上而下。
    pub bgra: Vec<u8>,
    /// 显示器原点（虚拟桌面物理像素）。
    pub origin_x: i32,
    pub origin_y: i32,
    /// DPI scale (1.0 = 96 dpi)。
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

    /// BGRA -> RGBA，用于剪贴板 / PNG 导出。
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.bgra.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }
}

/// 悬停命中的其他窗口矩形（覆盖层本地逻辑坐标）。
#[derive(Clone, Copy, Debug)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
