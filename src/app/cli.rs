//! CLI 自测开关与无窗口工具。
//!
//! 这些代码只在开发、打包验收时使用，不属于正常截图流程，所以从
//! bootstrap 里独立出来。`--capture` / `--ocr-test` 也会随发布版保留，
//! 用来验证便携包完整性。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    point, px, size, App, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions,
};

use crate::app::commands;
use crate::platform::{capture, tray, winutil};
use crate::services::{ocr, paths};
use crate::ui::{pin, text_result};

pub static SELFTEST: AtomicBool = AtomicBool::new(false);
pub static PIN_TEST: AtomicBool = AtomicBool::new(false);
pub static NO_TRAY: AtomicBool = AtomicBool::new(false);
pub static SETTINGS_TEST: AtomicBool = AtomicBool::new(false);
pub static TEXT_TEST: AtomicBool = AtomicBool::new(false);
pub static DEMO_BG: AtomicBool = AtomicBool::new(false);
pub static TEXT_TEST_FILE: Mutex<Option<String>> = Mutex::new(None);
pub static PIN_TEST_FILE: Mutex<Option<String>> = Mutex::new(None);

pub fn parse_flags(args: &[String]) {
    if args.iter().any(|a| a == "--selftest") {
        SELFTEST.store(true, Ordering::SeqCst);
    }
    if let Some(pos) = args.iter().position(|a| a == "--pin-test") {
        PIN_TEST.store(true, Ordering::SeqCst);
        if let Some(f) = args.get(pos + 1).filter(|f| !f.starts_with("--")) {
            if let Ok(mut guard) = PIN_TEST_FILE.lock() {
                *guard = Some(f.clone());
            }
        }
    }
    if args.iter().any(|a| a == "--no-tray") {
        NO_TRAY.store(true, Ordering::SeqCst);
    }
    if args.iter().any(|a| a == "--settings") {
        SETTINGS_TEST.store(true, Ordering::SeqCst);
    }
    if args.iter().any(|a| a == "--demo-bg") {
        DEMO_BG.store(true, Ordering::SeqCst);
    }
    if let Some(pos) = args.iter().position(|a| a == "--text-test") {
        TEXT_TEST.store(true, Ordering::SeqCst);
        if let Some(f) = args.get(pos + 1).filter(|f| !f.starts_with("--")) {
            if let Ok(mut guard) = TEXT_TEST_FILE.lock() {
                *guard = Some(f.clone());
            }
        }
    }
}

pub fn selftest_enabled() -> bool {
    SELFTEST.load(Ordering::SeqCst)
}

/// 演示背景：用程序生成的空白画布代替真实抓屏。
///
/// README 的截图不能带出桌面上的私人内容，而且换台机器就拍不出一样的图。
/// 打开这个开关后，覆盖层的底图和正常截图完全一样（同样尺寸、原点、DPI），
/// 只是像素由这里画出来：纯白 + 极淡网格，一眼能看出是空白背景，
/// 马赛克打上去也有东西可打。
pub fn demo_bg() -> bool {
    DEMO_BG.load(Ordering::SeqCst)
}

/// 生成演示用的空白底图。
pub fn demo_shot() -> Result<capture::Shot, String> {
    let geom = capture::cursor_monitor_geometry()?;
    let (w, h) = (geom.width as usize, geom.height as usize);
    let mut bgra = vec![0u8; w * h * 4];
    // 纯白
    for px in bgra.chunks_exact_mut(4) {
        px[0] = 0xFF;
        px[1] = 0xFF;
        px[2] = 0xFF;
        px[3] = 0xFF;
    }
    let mut put = |x: usize, y: usize, v: u8| {
        if x < w && y < h {
            let i = (y * w + x) * 4;
            bgra[i] = v;
            bgra[i + 1] = v;
            bgra[i + 2] = v;
        }
    };
    // 淡网格：每 40px 一条 0xEF，每 200px 一条 0xE2，给画面一点尺度感
    for x in (0..w).step_by(40) {
        let v = if x % 200 == 0 { 0xE2 } else { 0xEF };
        for y in 0..h {
            put(x, y, v);
        }
    }
    for y in (0..h).step_by(40) {
        let v = if y % 200 == 0 { 0xE2 } else { 0xEF };
        for x in 0..w {
            put(x, y, v);
        }
    }
    Ok(capture::Shot {
        width: geom.width,
        height: geom.height,
        bgra,
        origin_x: geom.origin_x,
        origin_y: geom.origin_y,
        scale: geom.scale,
    })
}

pub fn no_tray() -> bool {
    NO_TRAY.load(Ordering::SeqCst)
}

fn open_test_pin(cx: &mut App, file: Option<String>) {
    let (w, h) = (400u32, 240u32);
    let image = match file.as_deref().and_then(|p| image::open(p).ok()) {
        // 给 README 出图用：把导出的标注样张直接贴上来，比渐变色块真实
        Some(img) => img.to_rgba8(),
        None => {
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i] = (x * 255 / w) as u8;
                    rgba[i + 1] = (y * 255 / h) as u8;
                    rgba[i + 2] = 200;
                    rgba[i + 3] = 255;
                }
            }
            image::RgbaImage::from_raw(w, h, rgba).expect("test pin buffer size mismatch")
        }
    };
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    // 100% 缩放时 1 图像像素 = 1 逻辑像素，高 DPI 下会被放大 1.5 倍而变糊。
    // 测试窗口按 DPI 反算一下，让图严格 1:1 落在屏幕上。
    let scale = capture::cursor_monitor_geometry()
        .map(|g| g.scale)
        .unwrap_or(1.0)
        .max(1.0);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(300.0), px(300.0)),
            size: size(px(iw / scale), px(ih / scale)),
        })),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        display_id: None,
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: None,
        window_min_size: None,
        window_decorations: None,
        tabbing_identifier: None,
    };
    let res = cx.open_window(opts, move |window, cx| {
        winutil::make_topmost(window);
        winutil::set_app_icon(window);
        cx.new(|_| pin::PinView::new(image, 1.0 / scale))
    });
    match res {
        Ok(_) => {
            eprintln!("[pixpin-rs] test pin window created");
            winutil::dump_windows("after pin");
        }
        Err(e) => eprintln!("[pixpin-rs] test pin failed: {e}"),
    }
}

/// 启动后的延迟自测。和主消息泵分开跑，避免测试逻辑污染托盘轮询。
pub fn schedule_startup_tests(cx: &mut App) {
    if !SELFTEST.load(Ordering::SeqCst)
        && !PIN_TEST.load(Ordering::SeqCst)
        && !TEXT_TEST.load(Ordering::SeqCst)
        && !SETTINGS_TEST.load(Ordering::SeqCst)
    {
        return;
    }

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        if SELFTEST.load(Ordering::SeqCst) {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = cx.update(|cx| commands::start_capture(cx));
        }
        if PIN_TEST.load(Ordering::SeqCst) {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            let file = PIN_TEST_FILE.lock().ok().and_then(|mut g| g.take());
            let _ = cx.update(|cx| open_test_pin(cx, file));
        }
        if TEXT_TEST.load(Ordering::SeqCst) {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            let _ = cx.update(|cx| {
                let text = TEXT_TEST_FILE
                    .lock()
                    .ok()
                    .and_then(|mut g| g.take())
                    .and_then(|p| std::fs::read_to_string(&p).ok())
                    .unwrap_or_else(|| {
                        "PIXPIN-RS 文字选择自测。

第一段：拖动鼠标应该能看到蓝色选区，双击选中一个词，三击选中整段，右键弹出菜单。

第二段 The quick brown fox jumps over the lazy dog. 0123456789，中英混排 ABCdef 记录 2026-10-24。"
                            .to_string()
                    });
                text_result::TextView::open(text, true, cx);
            });
        }
        if SETTINGS_TEST.load(Ordering::SeqCst) {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            let _ = cx.update(|cx| commands::open_settings(cx));
        }
    })
    .detach();
}

fn simulate_drag(x0: i32, y0: i32, x1: i32, y1: i32, right: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP,
    };
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
    let (down, up) = if right {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    };
    unsafe {
        let _ = SetCursorPos(x0, y0);
        std::thread::sleep(Duration::from_millis(150));
        mouse_event(down, 0, 0, 0, 0);
        let steps = 24;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let x = x0 + ((x1 - x0) as f32 * t) as i32;
            let y = y0 + ((y1 - y0) as f32 * t) as i32;
            let _ = SetCursorPos(x, y);
            std::thread::sleep(Duration::from_millis(12));
        }
        mouse_event(up, 0, 0, 0, 0);
    }
}

/// Injects `vk` with the given modifiers.
///
/// `modern == true` goes through `SendInput`, which is the API the system
/// feeds into the same pipeline as hardware input — that is what
/// `RegisterHotKey` matches against. The legacy `keybd_event` path is kept for
/// comparison, because whether injected keys trip a global hotkey at all is
/// exactly what we need to pin down when debugging "the hotkey does nothing".
fn send_key(vk: u16, mods: &[String], modern: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP,
    };
    // VK_CONTROL / VK_MENU (Alt) / VK_SHIFT
    let wanted: Vec<u8> = mods
        .iter()
        .filter_map(|m| match m.as_str() {
            "ctrl" => Some(0x11u8),
            "alt" => Some(0x12),
            "shift" => Some(0x10),
            _ => None,
        })
        .collect();

    // Order: modifiers down, key down, key up, modifiers up (reverse).
    let mut seq: Vec<(u8, bool)> = Vec::new();
    for &m in &wanted {
        seq.push((m, false));
    }
    seq.push((vk as u8, false));
    seq.push((vk as u8, true));
    for &m in wanted.iter().rev() {
        seq.push((m, true));
    }

    unsafe {
        if modern {
            let mut inputs: Vec<INPUT> = Vec::with_capacity(seq.len());
            for (code, up) in seq {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(
                                code as u16,
                            ),
                            dwFlags: if up {
                                KEYEVENTF_KEYUP
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                            ..Default::default()
                        },
                    },
                });
            }
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            println!("SendInput -> {sent}/{} events", inputs.len());
        } else {
            for (code, up) in seq {
                keybd_event(
                    code,
                    0,
                    if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    0,
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// 处理所有“执行完就退出”的 CLI 命令。返回 true 表示进程不应进入事件循环。
/// 列出所有 GPUI 顶层窗口的矩形（跨进程），自动化验证贴图缩放 / 移动用。
fn list_app_windows() {    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };

    unsafe extern "system" fn cb(hwnd: HWND, _l: LPARAM) -> BOOL {
        let mut cls = [0u16; 128];
        let n = unsafe { GetClassNameW(hwnd, &mut cls) };
        if String::from_utf16_lossy(&cls[..n as usize]) == "Zed::Window" {
            let mut pid = 0u32;
            let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(hwnd, &mut r) };
            println!(
                "pid={} rect={} {} {}x{} visible={}",
                pid,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                unsafe { IsWindowVisible(hwnd).as_bool() }
            );
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(0));
    }
}

// ---------------------------------------------------------------------------
// 窗口截图：给 README 生成界面截图用
//
// GPUI 在 Windows 上走 DirectComposition，屏幕 BitBlt 抓不到自己的窗口，
// 所以这里对目标 HWND 调 PrintWindow(PW_RENDERFULLCONTENT)。只用于开发期
// 出图，不在截图主流程里。
// ---------------------------------------------------------------------------

// user32!PrintWindow。windows crate 把它挂在 Win32_Storage_Xps feature 下，
// 为了一个开发期函数给发布版多开一个 feature 不划算，这里直接声明。
#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: isize, hdc: isize, flags: u32) -> i32;
}

static SHOT_CANDIDATES: Mutex<Vec<(isize, i32, i32)>> = Mutex::new(Vec::new());

/// `--shot-window <out.png> [class] [index] [window|client] [printwindow-flags]`
///
/// class 默认 `Zed::Window`（GPUI 的窗口类）；按面积从大到小排，index 选第几个。
/// 倒数第二个参数默认 `client`：裁到客户区，去掉 DWM 那圈不可见边框；
/// 传 `window` 则保留整窗（含标题栏）。最后一个是 PrintWindow 的 flags，
/// 默认 2（PW_RENDERFULLCONTENT，抓 DirectComposition 窗口必需）；
/// 原生菜单要传 0，否则只画出第一项。
fn shot_window(out: &str, class: &str, index: usize, whole: bool, pw_flags: u32) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, IsWindowVisible,
    };

    struct Hit {
        hwnd: isize,
        w: i32,
        h: i32,
    }

    // 收集目标类名的窗口。
    let mut found: Vec<Hit> = Vec::new();
    unsafe extern "system" fn cb(hwnd: HWND, l: LPARAM) -> windows::core::BOOL {
        use windows::core::BOOL;
        let want = &*(l.0 as *const String);
        let mut cls = [0u16; 128];
        let n = unsafe { GetClassNameW(hwnd, &mut cls) };
        if String::from_utf16_lossy(&cls[..n as usize]) == *want {
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(hwnd, &mut r) };
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            // 只要看得见的：GPUI 会留一个隐藏的常驻窗口，抓它会得到全黑。
            if w > 0 && h > 0 && unsafe { IsWindowVisible(hwnd).as_bool() } {
                if let Ok(mut g) = SHOT_CANDIDATES.lock() {
                    g.push((hwnd.0 as isize, w, h));
                }
            }
        }
        BOOL(1)
    }
    if let Ok(mut g) = SHOT_CANDIDATES.lock() {
        g.clear();
    }
    let want = class.to_string();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&want as *const String as isize));
    }
    if let Ok(g) = SHOT_CANDIDATES.lock() {
        for &(h, w, hh) in g.iter() {
            found.push(Hit { hwnd: h, w, h: hh });
        }
    }
    found.sort_by_key(|b| -(b.w as i64 * b.h as i64));
    let Some(target) = found.get(index.min(found.len().saturating_sub(1))) else {
        eprintln!("[pixpin-rs] shot-window: 没找到类名 {class} 的窗口");
        return true;
    };
    let (w, h) = (target.w, target.h);

    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0 as u32;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            eprintln!("[pixpin-rs] shot-window: CreateDIBSection 失败");
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            return true;
        };
        let old = SelectObject(mem, dib.into());
        let ok = PrintWindow(
            target.hwnd,
            mem.0 as isize,
            pw_flags,
        );

        let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
        std::ptr::copy_nonoverlapping(bits as *const u8, bgra.as_mut_ptr(), bgra.len());

        SelectObject(mem, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        let non_black = bgra
            .chunks_exact(4)
            .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
            .count();
        // DWM 给每个顶层窗口都留了一圈不可见的缩放边框，PrintWindow 会把它
        // 拍成黑边。用客户区相对窗口的偏移把左右两侧的黑边切掉；
        // `window` 模式再把标题栏（窗口顶到客户区顶之间的部分）保留下来。
        let (mut x0, mut y0) = (0usize, 0usize);
        let (mut tw, mut th) = (w as usize, h as usize);
        {
            use windows::Win32::Foundation::RECT as R;
            use windows::Win32::Graphics::Gdi::ClientToScreen;
            use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
            let mut wr = R::default();
            let mut cr = R::default();
            let mut origin = windows::Win32::Foundation::POINT { x: 0, y: 0 };
            if GetWindowRect(HWND(target.hwnd as *mut _), &mut wr).is_ok()
                && GetClientRect(HWND(target.hwnd as *mut _), &mut cr).is_ok()
                && ClientToScreen(HWND(target.hwnd as *mut _), &mut origin).as_bool()
            {
                let inset_l = (origin.x - wr.left).max(0) as usize;
                let inset_t = (origin.y - wr.top).max(0) as usize;
                let cw = (cr.right - cr.left).max(1) as usize;
                let ch = (cr.bottom - cr.top).max(1) as usize;
                x0 = inset_l.min(w as usize - 1);
                tw = cw.min(w as usize - x0);
                y0 = if whole { 0 } else { inset_t.min(h as usize - 1) };
                th = if whole {
                    (inset_t + ch).min(h as usize)
                } else {
                    ch.min(h as usize - y0)
                };
            }
        }
        let mut rgba = vec![0u8; tw * th * 4];
        for y in 0..th {
            let src = ((y + y0) * w as usize + x0) * 4;
            let dst = y * tw * 4;
            rgba[dst..dst + tw * 4].copy_from_slice(&bgra[src..src + tw * 4]);
        }
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        match image::RgbaImage::from_raw(tw as u32, th as u32, rgba) {
            Some(img) => match img.save(out) {
                Ok(()) => println!(
                    "shot-window -> {out} ({w}x{h} -> {tw}x{th}, PrintWindow={ok}, 非黑像素 {non_black}/{})",
                    w as i64 * h as i64
                ),
                Err(e) => eprintln!("[pixpin-rs] 保存失败: {e}"),
            },
            None => eprintln!("[pixpin-rs] 缓冲区尺寸不符"),
        }
    }
    true
}

pub fn run_headless(args: &[String]) -> bool {
    if args.len() >= 2 && args[1] == "--tray-menu" {
        // 给 README 拍托盘菜单：往托盘窗口投一条 WM_TRAY + WM_RBUTTONUP，
        // 等价于右键点了托盘图标。菜单是 TrackPopupMenu 弹的原生菜单，
        // 会阻塞托盘线程，所以这里发完就退出。
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
        const WM_TRAY: u32 = 0x8000 + 1; // WM_APP + 1
        const WM_RBUTTONUP: usize = 0x0205;
        let hwnd: HWND =
            unsafe { FindWindowW(windows::core::w!("PixPinRSTrayWindow"), None) }.unwrap_or_default();
        if hwnd.is_invalid() {
            eprintln!("[pixpin-rs] tray-menu: 没找到托盘窗口（程序没在跑？）");
        } else {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_TRAY, WPARAM(0), LPARAM(WM_RBUTTONUP as isize)) };
            println!("tray-menu posted");
        }
        return true;
    }
    if args.len() >= 3 && args[1] == "--shot-window" {
        let class = args.get(3).map(|s| s.as_str()).unwrap_or("Zed::Window");
        let index: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        let whole = args.get(5).map(|s| s == "window").unwrap_or(false);
        // 原生菜单要 0（走 WM_PRINT），GPUI 的 DirectComposition 窗口要 2
        let pw_flags: u32 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(2);
        return shot_window(&args[2], class, index, whole, pw_flags);
    }
    if args.len() >= 3 && args[1] == "--ocr-test" {
        match image::open(&args[2]) {
            Ok(img) => match ocr::run(img.to_rgb8()) {
                Ok(text) => {
                    println!("{text}");
                    eprintln!("[pixpin-rs] ocr-test ok: {} chars", text.chars().count());
                }
                Err(e) => {
                    eprintln!("[pixpin-rs] ocr-test failed: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("[pixpin-rs] cannot open {}: {e}", args[2]);
                std::process::exit(1);
            }
        }
        return true;
    }
    if args.len() >= 6 && args[1] == "--drag" {
        let v: Vec<i32> = args[2..6].iter().filter_map(|s| s.parse().ok()).collect();
        if v.len() == 4 {
            let right = args.get(6).map(|s| s == "right").unwrap_or(false);
            simulate_drag(v[0], v[1], v[2], v[3], right);
        }
        return true;
    }
    if args.len() >= 8 && args[1] == "--zoom" {
        let n = |i: usize| args[i].parse::<u32>().unwrap_or(0);
        let (x, y, w, h) = (n(4), n(5), n(6), n(7));
        match image::open(&args[2]) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let sub = image::imageops::crop_imm(&rgba, x, y, w, h).to_image();
                let big = image::imageops::resize(
                    &sub,
                    (w * 6).max(1),
                    (h * 6).max(1),
                    image::imageops::FilterType::Nearest,
                );
                let _ = big.save(&args[3]);
                println!("zoom -> {}", args[3]);
            }
            Err(e) => eprintln!("open failed: {e}"),
        }
        return true;
    }
    if args.len() >= 4 && args[1] == "--move" {
        let (x, y) = (
            args[2].parse::<i32>().unwrap_or(0),
            args[3].parse::<i32>().unwrap_or(0),
        );
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
        }
        return true;
    }
    if args.len() >= 4 && args[1] == "--clicks" {
        let x: i32 = args[2].parse().unwrap_or(0);
        let y: i32 = args[3].parse().unwrap_or(0);
        let n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(2);
        let gap: u64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(90);
        // 第 7 个参数写 "right" 就是右键点击，用来驱动贴图窗口的右键菜单。
        let right = args.get(6).map(|s| s == "right").unwrap_or(false);
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN,
                MOUSEEVENTF_RIGHTUP,
            };
            use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
            let (down, up) = if right {
                (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
            } else {
                (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
            };
            let _ = SetCursorPos(x, y);
            std::thread::sleep(Duration::from_millis(120));
            for _ in 0..n.max(1) {
                mouse_event(down, 0, 0, 0, 0);
                std::thread::sleep(Duration::from_millis(30));
                mouse_event(up, 0, 0, 0, 0);
                std::thread::sleep(Duration::from_millis(gap));
            }
        }
        return true;
    }
    if args.len() >= 2 && args[1] == "--cursor" {
        let (x, y) = capture::cursor_pos();
        println!("{x},{y}");
        return true;
    }
    if args.len() >= 2 && args[1] == "--winlist" {
        list_app_windows();
        return true;
    }
    if args.len() >= 5 && args[1] == "--wheel" {
        let x: i32 = args[2].parse().unwrap_or(0);
        let y: i32 = args[3].parse().unwrap_or(0);
        let n: i32 = args[4].parse().unwrap_or(1);
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{mouse_event, MOUSEEVENTF_WHEEL};
            use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
            let _ = SetCursorPos(x, y);
            std::thread::sleep(Duration::from_millis(80));
            for _ in 0..n.abs().max(1) {
                // 正数向上滚 = 放大，和真实滚轮方向一致。
                mouse_event(MOUSEEVENTF_WHEEL, 0, 0, if n > 0 { 120 } else { -120 }, 0);
                std::thread::sleep(Duration::from_millis(70));
            }
        }
        return true;
    }
    if args.len() >= 3 && (args[1] == "--key" || args[1] == "--send") {
        if let Ok(vk) = args[2].parse::<u16>() {
            send_key(vk, &args[3..], args[1] == "--send");
        }
        return true;
    }
    if args.len() >= 2 && args[1] == "--where" {
        let dir = paths::save_dir();
        println!("{}", dir.display());
        println!("portable: {}", dir.ends_with("screenshots"));
        return true;
    }
    if args.len() >= 8 && args[1] == "--crop" {
        let (x, y, w, h) = (
            args[4].parse::<u32>().unwrap_or(0),
            args[5].parse::<u32>().unwrap_or(0),
            args[6].parse::<u32>().unwrap_or(100),
            args[7].parse::<u32>().unwrap_or(100),
        );
        match image::open(&args[2]) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let sub = image::imageops::crop_imm(&rgba, x, y, w, h).to_image();
                let _ = sub.save(&args[3]);
                println!("cropped -> {}", args[3]);
            }
            Err(e) => eprintln!("open failed: {e}"),
        }
        return true;
    }
    if args.len() >= 3 && args[1] == "--icon-export" {
        std::fs::write(&args[2], tray::icon_ico_bytes()).expect("write ico");
        println!("ico -> {}", args[2]);
        return true;
    }
    if args.len() >= 3 && args[1] == "--icon-test" {
        let s = tray::icon_size();
        if let Some(img) = image::RgbaImage::from_raw(s, s, tray::icon_rgba()) {
            let big =
                image::imageops::resize(&img, s * 8, s * 8, image::imageops::FilterType::Nearest);
            let _ = big.save(&args[2]);
            println!("icon -> {}", args[2]);
        }
        return true;
    }
    if args.len() >= 3 && args[1] == "--capture" {
        match capture::capture_cursor_monitor() {
            Ok(shot) => {
                let (w, h) = (shot.width, shot.height);
                match image::RgbaImage::from_raw(w, h, shot.to_rgba()) {
                    Some(img) => match img.save(&args[2]) {
                        Ok(()) => println!("saved {}x{} -> {}", w, h, args[2]),
                        Err(e) => eprintln!("save failed: {e}"),
                    },
                    None => eprintln!("buffer size mismatch"),
                }
            }
            Err(e) => eprintln!("capture failed: {e}"),
        }
        return true;
    }
    false
}
