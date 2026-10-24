//! 进程启动与常驻服务。
//!
//! `main.rs` 只保留 `app::run()`；单实例、控制台隐藏、常驻窗口、
//! 热键/托盘消息泵和 40ms 轮询都在这里。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, point, px, size, App, Application, Bounds, Context, IntoElement, Render, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};

use crate::app::{cli, commands};
use crate::platform::{diag, hotkey, tray};

/// 常驻的 1×1 隐藏窗口。GPUI 在最后一个窗口关闭时会整个退出
/// （platform.rs: close_one_window → PostQuitMessage），而托盘应用必须
/// 活得比窗口久 —— 贴图双击取消、覆盖层 Esc 都不能连带杀掉进程。
struct Resident;

impl Render for Resident {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn keep_resident_window(cx: &mut App) {
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(1.0), px(1.0)),
            })),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Opaque,
            app_id: None,
            window_min_size: None,
            window_decorations: None,
            tabbing_identifier: None,
        },
        |_window, cx| cx.new(|_| Resident),
    )
    .expect("failed to create resident window");
}

fn start_background_services(cx: &mut gpui::App) {
    let flag = Arc::new(AtomicBool::new(false));
    let hotkey_ok = hotkey::spawn(flag.clone());
    hotkey::REGISTERED.store(hotkey_ok, Ordering::SeqCst);
    if !hotkey_ok {
        eprintln!(
            "[pixpin-rs] global hotkey {} is taken; use the tray icon instead",
            commands::HOTKEY_LABEL
        );
    }

    // 托盘要先就位：热键失败的气泡通知要借托盘图标发，建在它前面会静默丢掉。
    let (tray_tx, tray_rx) = std::sync::mpsc::channel();
    let no_tray = cli::no_tray();
    let tray_ok = if no_tray { false } else { tray::spawn(tray_tx) };
    if !tray_ok && !no_tray {
        eprintln!("[pixpin-rs] failed to create the tray icon");
    }

    if !hotkey_ok && tray_ok {
        // 此刻进程没有任何窗口可说话：气泡是唯一能让用户知道「热键没生效」
        // 的通道。设置窗口打开时顶部还有红字横幅兜底。
        tray::balloon(
            "PixPin-RS 热键注册失败",
            &format!(
                "全局热键 {} 被其他程序占用，截图请左键点托盘图标。详情见 设置 → 系统。",
                commands::HOTKEY_LABEL
            ),
        );
    }

    cli::schedule_startup_tests(cx);

    cx.spawn(async move |cx: &mut gpui::AsyncApp| loop {
        cx.background_executor()
            .timer(Duration::from_millis(40))
            .await;

        let mut queued: Vec<tray::TrayCommand> = Vec::new();
        while let Ok(cmd) = tray_rx.try_recv() {
            queued.push(cmd);
        }
        if flag.swap(false, Ordering::SeqCst) {
            diag::trace("poller: hotkey flag picked up");
            queued.push(tray::TrayCommand::Capture);
        }
        if queued.is_empty() {
            continue;
        }
        let _ = cx.update(|cx| {
            for cmd in queued {
                commands::handle_tray_command(cmd, cx);
            }
        });
    })
    .detach();
}

fn already_running() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        match CreateMutexW(None, true, w!("Global\\PixPinRS.SingleInstance")) {
            Ok(handle) => {
                // `HANDLE` is `Copy` with no destructor, so the handle stays
                // open for the whole process — exactly what the guard needs.
                let _ = handle;
                GetLastError() == ERROR_ALREADY_EXISTS
            }
            Err(_) => false,
        }
    }
}

/// Get rid of the console window Windows hands to a console-subsystem exe.
///
/// Double-clicking the exe creates a console owned by *us*: a black box that
/// parks itself on top of the screen and takes the foreground. That is the
/// direct cause of "the hotkey does nothing until I click the tray icon" — the
/// overlay did open, it just opened behind the console.
fn hide_own_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    unsafe {
        let mut pids = [0u32; 8];
        let count = GetConsoleProcessList(&mut pids);
        if count == 1 {
            let _ = FreeConsole();
        }
    }
}

pub fn run() {
    diag::install();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--trace") || std::env::var_os("PIXPIN_TRACE").is_some() {
        diag::enable_trace();
        diag::trace(&format!("start: args={args:?}"));
    }
    cli::parse_flags(&args);
    if cli::run_headless(&args) {
        return;
    }

    if already_running() {
        eprintln!("[pixpin-rs] already running — look for the tray icon");
        diag::trace("start: another instance is already running");
        return;
    }

    // Do this before the window shows up, so the console never covers the UI.
    hide_own_console();
    diag::trace("start: entering event loop");

    Application::new().run(|cx: &mut App| {
        cx.activate(true);
        // 没有主窗口了：常驻托盘 + 全局热键是唯二入口，设置窗口按需打开。
        keep_resident_window(cx);
        start_background_services(cx);
    });
    eprintln!("[pixpin-rs] event loop exited");
}
