//! System tray icon, built directly on `Shell_NotifyIconW` because GPUI has no
//! tray API. Runs on its own thread with a message-only window and a message
//! pump; UI actions are forwarded to the GPUI thread over a channel.

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, PostMessageW, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, TranslateMessage, HICON, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW,
};

#[derive(Clone, Copy, Debug)]
pub enum TrayCommand {
    Capture,
    Settings,
    Quit,
}

static TX: Mutex<Option<Sender<TrayCommand>>> = Mutex::new(None);
/// 托盘消息窗口的 HWND（usize 跨线程），气泡通知要用它回传。
static TRAY_HWND: Mutex<Option<isize>> = Mutex::new(None);

const WM_BALLOON: u32 = WM_APP + 2;

/// 弹一条托盘气泡（Windows 通知）。必须在 `spawn` 成功之后调用。
pub fn balloon(title: &str, msg: &str) {
    let hwnd = TRAY_HWND
        .lock()
        .ok()
        .and_then(|g| *g)
        .map(|raw| windows::Win32::Foundation::HWND(raw as *mut _));
    let Some(hwnd) = hwnd else { return };
    crate::platform::diag::trace("tray: balloon requested");
    let payload = Box::into_raw(Box::new((
        title.encode_utf16().collect::<Vec<u16>>(),
        msg.encode_utf16().collect::<Vec<u16>>(),
    )));
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_BALLOON, WPARAM(0), LPARAM(payload as isize));
    }
}

const WM_TRAY: u32 = WM_APP + 1;
const ID_CAPTURE: usize = 1;
const ID_SETTINGS: usize = 2;
const ID_QUIT: usize = 3;
const ICON_SIZE: usize = 32;

fn send(cmd: TrayCommand) {
    if let Ok(guard) = TX.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(cmd);
        }
    }
}

/// Returns true when the icon is actually in the notification area.
pub fn spawn(tx: Sender<TrayCommand>) -> bool {
    if let Ok(mut guard) = TX.lock() {
        *guard = Some(tx);
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe { run(ready_tx) });
    ready_rx
        .recv_timeout(std::time::Duration::from_millis(1500))
        .unwrap_or(false)
}

unsafe fn run(ready: Sender<bool>) {
    let hinst: HINSTANCE = match GetModuleHandleW(None) {
        Ok(h) => h.into(),
        Err(_) => {
            let _ = ready.send(false);
            return;
        }
    };

    let class = w!("PixPinRSTrayWindow");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        lpszClassName: class,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        let _ = ready.send(false);
        return;
    }

    let hwnd = match CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        w!("PixPin-RS"),
        // A plain hidden top-level window rather than HWND_MESSAGE: the shell
        // sometimes misbehaves with message-only owners for tray icons.
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        None,
        None,
        Some(hinst),
        None,
    ) {
        Ok(h) => h,
        Err(_) => {
            let _ = ready.send(false);
            return;
        }
    };

    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: build_icon(hinst).unwrap_or_default(),
        ..Default::default()
    };
    let tip: Vec<u16> = "PixPin-RS · Ctrl+Alt+X".encode_utf16().collect();
    let n = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..n].copy_from_slice(&tip[..n]);

    let ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
    if ok {
        if let Ok(mut guard) = TRAY_HWND.lock() {
            *guard = Some(hwnd.0 as isize);
        }
    }
    let _ = ready.send(ok);
    if !ok {
        return;
    }
    eprintln!("[pixpin-rs] tray: icon added, entering message loop");

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    eprintln!("[pixpin-rs] tray: message loop ended");
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            eprintln!("[pixpin-rs] tray event {event}");
            match event {
                WM_LBUTTONUP => send(TrayCommand::Capture),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            eprintln!("[pixpin-rs] tray WM_COMMAND id={id}");
            match id {
                ID_CAPTURE => send(TrayCommand::Capture),
                ID_SETTINGS => send(TrayCommand::Settings),
                ID_QUIT => send(TrayCommand::Quit),
                _ => {}
            }
            LRESULT(0)
        }
        WM_BALLOON => {
            show_balloon(hwnd, lparam);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let _ = AppendMenuW(menu, MF_STRING, ID_CAPTURE, w!("截图  Ctrl+Alt+X"));
    let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("设置…"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("退出"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required so the menu dismisses when the user clicks elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        Some(0),
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
}

fn in_rounded_rect(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> bool {
    if x < x0 || y < y0 || x > x1 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

fn set_px(px: &mut [u8], x: usize, y: usize, c: [u8; 4]) {
    if x >= ICON_SIZE || y >= ICON_SIZE {
        return;
    }
    let i = (y * ICON_SIZE + x) * 4;
    // Icon bitmaps are DIBs: BGRA, not RGBA.
    px[i] = c[2];
    px[i + 1] = c[1];
    px[i + 2] = c[0];
    px[i + 3] = c[3];
}

/// The icon pixels in RGBA, for previewing / tests.
pub fn icon_rgba() -> Vec<u8> {
    let mut bgra = draw_icon_bgra();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    bgra
}

pub fn icon_size() -> u32 {
    ICON_SIZE as u32
}

/// A 32×32 icon drawn by hand: blue rounded square with white capture brackets.
fn draw_icon_bgra() -> Vec<u8> {
    let s = ICON_SIZE as i32;
    let mut px = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
    let blue = [0x0Au8, 0x84, 0xFF, 0xFF];
    let white = [0xFFu8, 0xFF, 0xFF, 0xFF];

    for y in 0..s {
        for x in 0..s {
            if in_rounded_rect(x, y, 1, 1, s - 2, s - 2, 7) {
                set_px(&mut px, x as usize, y as usize, blue);
            }
        }
    }

    // Four L-shaped corner brackets.
    let t = 2usize; // thickness
    let (lo, hi) = (7usize, 24usize); // outer inset / far edge
    let arm = 7usize;
    let mut bar = |x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                set_px(&mut px, x, y, white);
            }
        }
    };
    // top-left
    bar(lo, lo + arm, lo, lo + t - 1);
    bar(lo, lo + t - 1, lo, lo + arm);
    // top-right
    bar(hi - arm, hi, lo, lo + t - 1);
    bar(hi - t + 1, hi, lo, lo + arm);
    // bottom-left
    bar(lo, lo + arm, hi - t + 1, hi);
    bar(lo, lo + t - 1, hi - arm, hi);
    // bottom-right
    bar(hi - arm, hi, hi - t + 1, hi);
    bar(hi - t + 1, hi, hi - arm, hi);

    px
}

unsafe fn build_icon(hinst: HINSTANCE) -> Option<HICON> {
    let s = ICON_SIZE as i32;
    let px = draw_icon_bgra();
    let and_mask = vec![0u8; ICON_SIZE * ICON_SIZE / 8];
    CreateIcon(Some(hinst), s, s, 1, 32, and_mask.as_ptr(), px.as_ptr()).ok()
}

/// 把 (title, msg) 的 UTF-16 内容弹成气泡通知。lparam 是 Box 的裸指针。
unsafe fn show_balloon(hwnd: HWND, lparam: LPARAM) {
    use windows::Win32::UI::Shell::{NIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};
    let payload = Box::from_raw(lparam.0 as *mut (Vec<u16>, Vec<u16>));
    let (title, msg) = *payload;
    crate::platform::diag::trace("tray: balloon shown");

    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_INFO,
        ..Default::default()
    };
    let n = title.len().min(nid.szInfoTitle.len() - 1);
    nid.szInfoTitle[..n].copy_from_slice(&title[..n]);
    let m = msg.len().min(nid.szInfo.len() - 1);
    nid.szInfo[..m].copy_from_slice(&msg[..m]);
    let ok = Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool();
    crate::platform::diag::trace(&format!("tray: NIM_MODIFY -> {ok}"));
}

/// 应用图标（32×32 HICON，带缓存）：窗口标题栏 / 任务栏 / 托盘共用一套像素，
/// 保证三处视觉一致。Windows 的 HICON 是轻量句柄，缓存后随进程存活。
pub fn app_hicon() -> HICON {
    // HICON 内含裸指针（非 Send），缓存里只存 isize，用时重建句柄。
    static ICON: Mutex<Option<isize>> = Mutex::new(None);
    let mut guard = ICON.lock().unwrap();
    if guard.is_none() {
        let h = unsafe { create_icon(32) };
        *guard = Some(h.0 as isize);
    }
    HICON(guard.unwrap() as *mut core::ffi::c_void)
}

unsafe fn create_icon(size: u32) -> HICON {
    let hinst = match GetModuleHandleW(None) {
        Ok(h) => h.into(),
        Err(_) => HINSTANCE::default(),
    };
    let rgba = draw_icon_rgba(size);
    let s = size as usize;
    // CreateIcon 吃 BGRA
    let mut bgra = rgba.clone();
    for c in bgra.chunks_exact_mut(4) {
        c.swap(0, 2);
    }
    let and_mask = vec![0u8; s * s / 8];
    CreateIcon(
        Some(hinst),
        s as i32,
        s as i32,
        1,
        32,
        and_mask.as_ptr(),
        bgra.as_ptr(),
    )
    .unwrap_or_default()
}

/// 任意尺寸绘制应用图标（RGBA，自上而下）。32px 设计稿等比缩放。
pub fn draw_icon_rgba(size: u32) -> Vec<u8> {
    let s = size as usize;
    let mut px = vec![0u8; s * s * 4];
    let u = size as f32 / 32.0; // 设计稿是 32px
    let blue = [0x0Au8, 0x84, 0xFF, 0xFF];
    let white = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let mut set = |x: i32, y: i32, c: [u8; 4]| {
        if x < 0 || y < 0 || x >= s as i32 || y >= s as i32 {
            return;
        }
        let i = ((y as usize) * s + x as usize) * 4;
        px[i] = c[0];
        px[i + 1] = c[1];
        px[i + 2] = c[2];
        px[i + 3] = c[3];
    };
    let f = |v: f32| (v * u).round() as i32;
    for y in 0..s as i32 {
        for x in 0..s as i32 {
            if in_rounded_rect(
                x,
                y,
                f(1.0),
                f(1.0),
                s as i32 - 1 - f(1.0),
                s as i32 - 1 - f(1.0),
                f(7.0),
            ) {
                set(x, y, blue);
            }
        }
    }
    let t = f(2.0) as usize;
    let lo = f(7.0) as usize;
    let hi = f(24.0) as usize;
    let arm = f(7.0) as usize;
    let mut bar = |x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..=y1.min(s - 1) {
            for x in x0..=x1.min(s - 1) {
                set(x as i32, y as i32, white);
            }
        }
    };
    bar(lo, lo + arm, lo, lo + t - 1);
    bar(lo, lo + t - 1, lo, lo + arm);
    bar(hi - arm, hi, lo, lo + t - 1);
    bar(hi - t + 1, hi, lo, lo + arm);
    bar(lo, lo + arm, hi - t + 1, hi);
    bar(lo, lo + t - 1, hi - arm, hi);
    bar(hi - arm, hi, hi - t + 1, hi);
    bar(hi - t + 1, hi, hi - arm, hi);
    px
}

/// 生成多尺寸 .ico 文件内容（16–256），供 build.rs 嵌入 exe 资源。
pub fn icon_ico_bytes() -> Vec<u8> {
    let sizes: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    for &sz in &sizes {
        let rgba = draw_icon_rgba(sz);
        let s = sz as usize;
        // ICO 里的 BMP：BGRA、行序自下而上
        let mut px = vec![0u8; s * s * 4];
        for y in 0..s {
            for x in 0..s {
                let src = ((y * s + x) * 4) as usize;
                let dst = (((s - 1 - y) * s + x) * 4) as usize;
                px[dst] = rgba[src + 2];
                px[dst + 1] = rgba[src + 1];
                px[dst + 2] = rgba[src];
                px[dst + 3] = rgba[src + 3];
            }
        }
        let mut bmp = Vec::new();
        bmp.extend_from_slice(&40u32.to_le_bytes()); // biSize
        bmp.extend_from_slice(&(sz as i32).to_le_bytes()); // 宽
        bmp.extend_from_slice(&((sz as i32) * 2).to_le_bytes()); // 高（XOR+AND）
        bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
        bmp.extend_from_slice(&32u16.to_le_bytes()); // bpp
        bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        bmp.extend_from_slice(&((s * s * 4) as u32).to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&px);
        let mask_row = ((sz + 31) / 32) * 4; // AND 掩码行按 32bit 对齐
        bmp.extend(std::iter::repeat(0u8).take(mask_row as usize * s));
        images.push((sz, bmp));
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6u32 + 16u32 * images.len() as u32;
    for (sz, data) in &images {
        out.push(if *sz >= 256 { 0 } else { *sz as u8 });
        out.push(if *sz >= 256 { 0 } else { *sz as u8 });
        out.push(0); // 调色板色数
        out.push(0); // 保留
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}
