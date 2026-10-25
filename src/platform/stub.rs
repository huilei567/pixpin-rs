//! Linux / macOS 桩实现：签名与 Windows 版一致，行为是明确的 no-op / 错误。
//!
//! 目的是让非 Windows 平台能编译出安装包（.deb / .tar.gz / .dmg）。
//! 真正的跨平台实现要用 xcap（抓屏）、tray-icon（托盘）、global-hotkey
//! （热键）替换这些桩 —— 见 workflow 里平台移植的注释。

pub const UNIMPLEMENTED: &str =
    "此能力目前仅 Windows 可用（抓屏/托盘/热键的跨平台实现尚未接入）";

pub mod capture {
    pub use super::{MonitorGeom, Shot};
    use super::UNIMPLEMENTED;

    pub fn capture_cursor_monitor() -> Result<Shot, String> {
        Err(UNIMPLEMENTED.into())
    }

    pub fn cursor_monitor_geometry() -> Result<MonitorGeom, String> {
        Err(UNIMPLEMENTED.into())
    }

    pub fn cursor_pos() -> (i32, i32) {
        (0, 0)
    }
}

pub mod hotkey {
    use super::UNIMPLEMENTED;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// 热键注册状态。桩实现下永远是 false（设置页会如实显示「未注册」）。
    pub static REGISTERED: AtomicBool = AtomicBool::new(false);

    pub fn spawn(_flag: Arc<AtomicBool>) -> bool {
        eprintln!("[pixpin-rs] 全局热键：{}，跳过注册", UNIMPLEMENTED);
        false
    }
}

pub mod tray {
    pub use super::TrayCommand;
    use super::UNIMPLEMENTED;
    use std::sync::mpsc::Sender;

    pub fn spawn(_tx: Sender<TrayCommand>) -> bool {
        eprintln!("[pixpin-rs] 托盘图标：{}，跳过创建", UNIMPLEMENTED);
        false
    }

    pub fn balloon(_title: &str, _msg: &str) {
        eprintln!("[pixpin-rs] 气泡通知：{}", UNIMPLEMENTED);
    }
}

pub mod winpick {
    pub use super::WinRect;
    use super::UNIMPLEMENTED;

    pub fn window_at(_px: i32, _py: i32) -> Option<WinRect> {
        None
    }
}

pub mod winutil {
    pub fn hide_own_windows() {}

    pub fn hide_for_capture() {}

    pub fn restore_after_capture() {}

    pub fn set_app_icon(_window: &gpui::Window) {}

    pub fn make_topmost(_window: &gpui::Window) {}

    pub fn present(_window: &gpui::Window, _topmost: bool) {}

    pub fn force_foreground(_window: &gpui::Window) {}

    pub fn set_window_pos(_window: &gpui::Window, _x: f32, _y: f32) {}

    pub fn stash_edit_pin(_window: &gpui::Window) {}

    pub fn restore_edit_pin() {}

    pub fn drop_edit_pin() {}

    pub fn start_window_drag(_window: &gpui::Window) {}

    pub fn dump_windows(_tag: &str) {}
}
