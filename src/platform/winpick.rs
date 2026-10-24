//! 光标下的窗口探测 —— PixPin 那种「悬停高亮窗口、单击直接选中」的能力。
//!
//! 覆盖层自己盖住了整个屏幕，所以不能用 `WindowFromPoint`（它只会返回我们自己）。
//! 改成按 z 序从上往下走一遍顶层窗口，跳过本进程的窗口、不可见窗口、最小化窗口、
//! DWM 隐藏的窗口，取第一个包含光标的。

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetTopWindow, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, GW_HWNDNEXT,
};

/// 桌面、任务栏、手写画布这些 shell 窗口铺满全屏，选中它们没有意义；
/// 而且它们排在最底层却包含任何坐标点 —— 不排掉的话窗口识别永远命中桌面。
///
/// 注意 `ShellHandwritingCanvas` 后面还跟一个 GUID，所以按前缀匹配。
const SHELL_PREFIXES: [&str; 12] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "ShellHandwritingCanvas",
    "SysShadow",
    "ForegroundStaging",
    "Windows.UI.Core.CoreWindow",
    "MultitaskingViewFrame",
    "TaskListThumbnailWnd",
    "XamlExplorerHostIslandWindow",
    "Microsoft.UI.Content.DesktopChildSiteBridge",
];

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

fn is_shell(hwnd: HWND) -> bool {
    let c = class_name(hwnd);
    SHELL_PREFIXES.iter().any(|p| c.starts_with(p))
}

/// 屏幕物理像素矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn cloaked(hwnd: HWND) -> bool {
    let mut v: u32 = 0;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut v as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && v != 0
}

/// 用 DWM 的扩展边框，能去掉投影，比 `GetWindowRect` 更贴合视觉边界。
fn frame(hwnd: HWND) -> Option<WinRect> {
    let mut r = RECT::default();
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if dwm.is_err() {
        r = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            return None;
        }
    }
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    // 太小的基本是托盘小窗、输入法候选框之类，选中没意义。
    if w < 24 || h < 24 {
        return None;
    }
    Some(WinRect {
        x: r.left,
        y: r.top,
        w,
        h,
    })
}

/// 找出屏幕物理坐标 `(px, py)` 处最上层的那个普通窗口。
pub fn window_at(px: i32, py: i32) -> Option<WinRect> {
    let me = std::process::id();
    let pt = POINT { x: px, y: py };
    let mut cur = unsafe { GetTopWindow(None) }.ok()?;

    // 兜底上限，防止某些 shell 窗口把 z 序走成环。
    for _ in 0..600 {
        let hwnd = cur;
        let mut pid = 0u32;
        unsafe {
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        let usable = pid != me
            && unsafe { IsWindowVisible(hwnd).as_bool() }
            && !unsafe { IsIconic(hwnd).as_bool() }
            && !cloaked(hwnd)
            && !is_shell(hwnd);

        if usable {
            if let Some(r) = frame(hwnd) {
                if pt.x >= r.x && pt.x < r.x + r.w && pt.y >= r.y && pt.y < r.y + r.h {
                    if std::env::var_os("PIXPIN_DEBUG_PICK").is_some() {
                        eprintln!(
                            "[winpick] hit class={} rect=({},{} {}x{})",
                            class_name(hwnd),
                            r.x,
                            r.y,
                            r.w,
                            r.h
                        );
                    }
                    return Some(r);
                }
            }
        }

        match unsafe { GetWindow(hwnd, GW_HWNDNEXT) } {
            Ok(next) if !next.is_invalid() => cur = next,
            _ => break,
        }
    }
    None
}
