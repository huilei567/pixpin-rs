//! Hide / show / move our own top-level windows.
//!
//! GPUI exposes no "move window" API and no tray, so anything that needs the
//! native window handle lives here.

use std::sync::Mutex;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, ShowWindow,
    SW_HIDE, SW_SHOW,
};

/// 抓屏前快照的本进程可见窗口（HWND 指针存 isize），抓完原样恢复。
///
/// 旧实现只跟踪主窗口可见性：主窗口缩到托盘后发起截图，会把贴图窗口
/// 一起藏掉且永不恢复。现在按「抓屏时谁可见，收尾时恢复谁」处理，
/// 覆盖层自己是在快照之后创建的，不会被误恢复。
static VISIBLE_BEFORE: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// EnumWindows 回调的指令，实例经 `lparam` 以指针传入（回调期间存活）。
enum Mode {
    /// 收集本进程可见窗口到给定 Vec
    Collect(*mut Vec<isize>),
    /// 隐藏本进程全部窗口
    Hide,
    /// 只显示列表里记录的窗口
    Show(Vec<isize>),
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == std::process::id() {
        let mode = &*(lparam.0 as *const Mode);
        match mode {
            Mode::Collect(list) => {
                // list 是 &*mut Vec：先把裸指针拷出来再解引用。
                let list = *list;
                if IsWindowVisible(hwnd).as_bool() {
                    unsafe { &mut *list }.push(hwnd.0 as isize);
                }
            }
            Mode::Hide => {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            Mode::Show(list) => {
                if list.contains(&(hwnd.0 as isize)) {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }
        }
    }
    BOOL(1)
}

fn enum_with(mode: &Mode) {
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(mode as *const Mode as isize));
    }
}

pub fn hide_own_windows() {
    enum_with(&Mode::Hide);
    // Make DWM actually composite the hide before we grab the framebuffer,
    // otherwise the window's shadow / frame can survive into the screenshot.
    unsafe {
        let _ = windows::Win32::Graphics::Dwm::DwmFlush();
    }
}

/// Called before grabbing the screen: snapshot which of our windows are
/// visible (settings, pins, text results …), then hide everything of ours.
pub fn hide_for_capture() {
    let mut list: Vec<isize> = Vec::new();
    enum_with(&Mode::Collect(&mut list));
    if let Ok(mut guard) = VISIBLE_BEFORE.lock() {
        *guard = list;
    }
    hide_own_windows();
}

/// Called when the overlay closes: bring back exactly what was visible before.
pub fn restore_after_capture() {
    let list = VISIBLE_BEFORE
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    if list.is_empty() {
        return;
    }
    enum_with(&Mode::Show(list));
    unsafe {
        let _ = windows::Win32::Graphics::Dwm::DwmFlush();
    }
}

/// 给窗口设置应用图标（标题栏小图标 + 任务栏大图标）。
///
/// GPUI 0.2.2 不暴露窗口图标 API，注册的窗口类也没带图标，所以所有
/// GPUI 窗口在任务栏/标题栏里都是系统默认的灰白框。这里给每个窗口
/// 发 WM_SETICON，HICON 与托盘共用同一套像素 —— 三处视觉一致。
pub fn set_app_icon(window: &gpui::Window) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, ICON_BIG, ICON_SMALL, WM_SETICON};
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    let hicon = crate::platform::tray::app_hicon();
    crate::platform::diag::trace(&format!(
        "set_app_icon: hicon={:p} null={}",
        hicon.0,
        hicon.0.is_null()
    ));
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
    }
}

/// Pin windows float above everything else — that is the whole point of pinning
/// an image, and it also keeps them out of the way of ordinary windows.
pub fn make_topmost(window: &gpui::Window) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };
    if let Some(hwnd) = hwnd_of(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

/// Force a window to the foreground even though our process may not own the
/// current foreground window.
///
/// Windows has a *foreground lock*: `SetForegroundWindow` is silently refused
/// (the window only blinks in the taskbar) unless the calling process is
/// already foreground or received the last input event. Two of our entry
/// points sit exactly on the wrong side of that rule:
///
/// * the global hotkey — the keystroke is swallowed by the hotkey, so no input
///   event reaches us;
/// * right after the fullscreen overlay closes — the foreground owner is the
///   window underneath, not us.
///
/// The symptom users report is "the hotkey does nothing on first launch" /
/// "the OCR window is not on top". Three measures stacked together fix it:
///
/// 1. `SetWindowPos(HWND_TOPMOST)` — even if activation is refused the window
///    is at least visible above everything, so the user *sees* the result;
/// 2. `AttachThreadInput` onto the foreground thread, which lifts the lock;
/// 3. `SetForegroundWindow` + `SetActiveWindow` + `SetFocus` for real focus.
///
/// `topmost` should stay `true` for transient windows the user must see right
/// now (overlay, OCR result). Pinned images already own topmost separately.
pub fn present(window: &gpui::Window, topmost: bool) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW,
    };

    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    unsafe {
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let me = GetCurrentThreadId();
        let attached =
            fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();

        let _ = ShowWindow(hwnd, SW_SHOW);
        if topmost {
            // SWP_NOACTIVATE is deliberately *not* set here: making the window
            // topmost also activates it when we are allowed to.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));

        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        // Make DWM composite the z-order change before anyone screenshots it.
        let _ = windows::Win32::Graphics::Dwm::DwmFlush();
    }
}

/// Backwards-compatible shorthand for [`present`] without the topmost nudge.
/// Kept for call sites that only want focus, not a z-order change.
#[allow(dead_code)]
pub fn force_foreground(window: &gpui::Window) {
    present(window, false);
}

pub fn hwnd_of(window: &gpui::Window) -> Option<HWND> {
    // `Window::window_handle` (inherent) returns an AnyWindowHandle, so the
    // trait method has to be called explicitly.
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win) => Some(HWND(win.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// 只移动窗口、不改尺寸（入参是屏幕逻辑坐标）。
///
/// GPUI 0.2.2 只暴露 `Window::resize`，没有移动 API。贴图缩放要「以光标
/// 位置为锚点」，就必须把窗口一起搬走，所以这里直接操作 HWND。
pub fn set_window_pos(window: &gpui::Window, x: f32, y: f32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    let s = window.scale_factor();
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            (x * s).round() as i32,
            (y * s).round() as i32,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// 二次编辑期间被暂时藏起来的贴图窗口（HWND 指针存 isize）。
///
/// 「编辑」是从贴图窗口发起的，但覆盖层要独占画面。直接把贴图销毁的话，
/// 用户按 Esc 就等于白丢一张图，所以先藏起来：
///
/// * 编辑取消（Esc / 右键）→ 原样放回去；
/// * 编辑结果被「贴图 / 复制 / 保存」消费掉 → 彻底关掉，避免出现两张。
static EDIT_PIN: Mutex<Option<isize>> = Mutex::new(None);

pub fn stash_edit_pin(window: &gpui::Window) {
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    if let Ok(mut guard) = EDIT_PIN.lock() {
        *guard = Some(hwnd.0 as isize);
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// 编辑取消：把之前藏起来的贴图窗口放回屏幕。
pub fn restore_edit_pin() {
    let stored = EDIT_PIN.lock().map(|mut g| g.take()).unwrap_or(None);
    if let Some(raw) = stored {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
        }
    }
}

/// 编辑结果已经落地（新的贴图 / 复制 / 保存）：关掉被藏起来的旧贴图。
pub fn drop_edit_pin() {
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    let stored = EDIT_PIN.lock().map(|mut g| g.take()).unwrap_or(None);
    if let Some(raw) = stored {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM::default(), LPARAM::default());
        }
    }
}

/// Start dragging a borderless window.
///
/// The `WM_NCLBUTTONDOWN` + `HTCAPTION` trick would be one line, but it runs a
/// nested modal loop inside GPUI's event dispatch, which is re-entrant enough to
/// kill the process on longer drags. Instead we poll the real cursor on a
/// dedicated thread and call `SetWindowPos`, which never touches GPUI's loop.
pub fn start_window_drag(window: &gpui::Window) {
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    // HWND is a raw pointer, so pass it across the thread boundary as usize.
    let hwnd_raw = hwnd.0 as usize;
    std::thread::spawn(move || unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetWindowRect, IsWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE,
            SWP_NOZORDER,
        };

        let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);

        let mut origin = RECT::default();
        if GetWindowRect(hwnd, &mut origin).is_err() {
            return;
        }
        let mut cursor0 = POINT::default();
        let _ = GetCursorPos(&mut cursor0);

        loop {
            if !IsWindow(Some(hwnd)).as_bool() {
                return;
            }
            // Stop as soon as the left button is released.
            if (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) == 0 {
                return;
            }
            let mut cur = POINT::default();
            if GetCursorPos(&mut cur).is_ok() {
                let x = origin.left + (cur.x - cursor0.x);
                let y = origin.top + (cur.y - cursor0.y);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    });
}

/// Log every top-level window this process owns: class, title, rect, visibility.
/// Invaluable when a window is created successfully but is nowhere to be seen.
pub fn dump_windows(tag: &str) {
    unsafe extern "system" fn cb(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetWindowRect, GetWindowTextW,
        };
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            let mut cls = [0u16; 128];
            let n = GetClassNameW(hwnd, &mut cls);
            let mut txt = [0u16; 256];
            let m = GetWindowTextW(hwnd, &mut txt);
            let mut r = windows::Win32::Foundation::RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            eprintln!(
                "[pixpin-rs] win {:?} class={:?} title={:?} rect=({},{} {}x{}) visible={}",
                hwnd.0,
                String::from_utf16_lossy(&cls[..n as usize]),
                String::from_utf16_lossy(&txt[..m as usize]),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                IsWindowVisible(hwnd).as_bool()
            );
        }
        BOOL(1)
    }
    eprintln!("[pixpin-rs] --- windows ({tag}) ---");
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(0));
    }
}
