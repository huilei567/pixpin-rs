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
pub static TEXT_TEST_FILE: Mutex<Option<String>> = Mutex::new(None);

pub fn parse_flags(args: &[String]) {
    if args.iter().any(|a| a == "--selftest") {
        SELFTEST.store(true, Ordering::SeqCst);
    }
    if args.iter().any(|a| a == "--pin-test") {
        PIN_TEST.store(true, Ordering::SeqCst);
    }
    if args.iter().any(|a| a == "--no-tray") {
        NO_TRAY.store(true, Ordering::SeqCst);
    }
    if args.iter().any(|a| a == "--settings") {
        SETTINGS_TEST.store(true, Ordering::SeqCst);
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

pub fn no_tray() -> bool {
    NO_TRAY.load(Ordering::SeqCst)
}

fn open_test_pin(cx: &mut App) {
    let (w, h) = (400u32, 240u32);
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
    let image = image::RgbaImage::from_raw(w, h, rgba).expect("test pin buffer size mismatch");
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(300.0), px(300.0)),
            size: size(px(400.0), px(240.0)),
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
        cx.new(|_| pin::PinView::new(image, 1.0))
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
            let _ = cx.update(|cx| open_test_pin(cx));
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
fn list_app_windows() {
    use windows::core::BOOL;
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

pub fn run_headless(args: &[String]) -> bool {
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
