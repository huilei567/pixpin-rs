//! System-wide hotkey registration.
//!
//! `RegisterHotKey` posts `WM_HOTKEY` to the thread that registered it, so a
//! dedicated thread with its own message pump is all we need.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

/// Ctrl + Alt + X
const VK_X: u32 = 0x58;
const HOTKEY_ID: i32 = 0x5250;

/// 注册结果，供设置窗口显示「热键被占用」告警。启动时写入一次。
pub static REGISTERED: AtomicBool = AtomicBool::new(true);

pub fn spawn(flag: Arc<AtomicBool>) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        // 测试开关：模拟「热键被别的程序占用」，验证告警横幅和气泡通知。
        let occupied = std::env::var_os("PIXPIN_NO_HOTKEY").is_some();
        let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
        let registered = !occupied && RegisterHotKey(None, HOTKEY_ID, mods, VK_X).is_ok();
        crate::platform::diag::trace(&format!("hotkey: RegisterHotKey -> {registered}"));
        let _ = tx.send(registered);
        if !registered {
            return;
        }
        let mut msg = MSG::default();
        crate::platform::diag::trace("hotkey: pump entered");
        let mut seen = 0u32;
        loop {
            // `GetMessageW` returns -1 on error and 0 on WM_QUIT; both end the
            // pump, so treat them the same and say so in the trace log.
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 == 0 || r.0 == -1 {
                crate::platform::diag::trace(&format!(
                    "hotkey: pump ended (GetMessageW -> {})",
                    r.0
                ));
                return;
            }
            if seen < 10 {
                seen += 1;
                crate::platform::diag::trace(&format!(
                    "hotkey: msg 0x{:04X} wparam=0x{:X}",
                    msg.message, msg.wParam.0
                ));
            }
            if msg.message == WM_HOTKEY {
                crate::platform::diag::trace("hotkey: WM_HOTKEY received");
                flag.store(true, Ordering::SeqCst);
            }
        }
    });
    rx.recv_timeout(std::time::Duration::from_millis(800))
        .unwrap_or(false)
}
