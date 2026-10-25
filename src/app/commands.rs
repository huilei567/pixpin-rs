//! 应用用例：把 platform / services / ui 串成用户可见的流程。
//!
//! 这里只做编排，不实现抓屏、不画界面。所有窗口打开顺序和跨模块
//! 的恢复动作都集中在 commands 层，方便以后替换 UI 或加新的入口。

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    point, px, size, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};

use crate::app::cli;
use crate::platform::{capture, diag, tray, winutil};
use crate::ui::{overlay, settings};

/// Ctrl+Alt+X
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+X";

fn capture_window_options(shot: &capture::Shot) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(shot.logical_x()), px(shot.logical_y())),
            size: size(px(shot.logical_w()), px(shot.logical_h())),
        })),
        titlebar: None,
        focus: true,
        show: true,
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
    }
}

pub fn start_capture(cx: &mut gpui::App) {
    diag::trace("capture: begin");
    // Keep the app out of its own screenshot.
    winutil::hide_for_capture();
    std::thread::sleep(Duration::from_millis(220));
    winutil::hide_own_windows();

    // `--demo-bg` 用程序生成的空白画布代替真实抓屏，专门给 README 出图。
    let grabbed = if cli::demo_bg() {
        cli::demo_shot()
    } else {
        capture::capture_cursor_monitor()
    };
    let shot = match grabbed {
        Ok(shot) => Arc::new(shot),
        Err(err) => {
            eprintln!("[pixpin-rs] capture failed: {err}");
            diag::trace(&format!("capture: failed {err}"));
            winutil::restore_after_capture();
            return;
        }
    };

    let selftest = cli::selftest_enabled();
    let result = cx.open_window(capture_window_options(&shot), move |window, cx| {
        cx.new(|cx| overlay::Overlay::new(shot, selftest, window, cx))
    });
    match result {
        Ok(handle) => {
            // The overlay must own the keyboard, otherwise Esc / Enter are dead.
            // `present` makes it topmost as well: when the foreground lock
            // refuses activation the window would otherwise open *behind* the
            // window that had focus, which reads as "the hotkey did nothing".
            let _ = handle.update(cx, |_overlay, window, cx| {
                winutil::present(window, true);
                window.activate_window();
                window.defer(cx, |window, _cx| {
                    winutil::present(window, true);
                    window.activate_window();
                });
            });
            diag::trace("capture: overlay presented");
        }
        Err(_) => {
            diag::trace("capture: open_window failed");
            winutil::restore_after_capture();
        }
    }
}

/// 设置窗口的「立即截图」：设置窗口本身会被抓屏流程隐藏并在结束后恢复。
pub fn start_capture_from_settings(cx: &mut gpui::App) {
    start_capture(cx);
}

/// 把一张已有的图重新送进覆盖层做二次编辑（贴图窗口的「编辑标注」）。
///
/// 覆盖层的输入是「一整屏快照 + 屏幕原点」，所以这里把贴图包装成一份
/// 尺寸等于自己的 `Shot`：窗口就落在贴图原来的位置，画面不跳，
/// 标注 / 取字 / 复制 / 保存 / 再贴图全部复用原有流程。
///
/// 贴图比工具栏还窄时会把画布向右下补成深灰底，否则工具栏按钮会被窗口裁掉。
pub fn edit_image(
    img: image::RgbaImage,
    origin_x: i32,
    origin_y: i32,
    scale: f32,
    cx: &mut gpui::App,
) {
    let (iw, ih) = img.dimensions();
    let min_w = (overlay::MIN_EDIT_W * scale).round().max(1.0) as u32;
    let min_h = (overlay::MIN_EDIT_H * scale).round().max(1.0) as u32;
    let (width, height) = (iw.max(min_w), ih.max(min_h));
    let canvas = if (width, height) == (iw, ih) {
        img
    } else {
        let mut canvas =
            image::RgbaImage::from_pixel(width, height, image::Rgba([0x2A, 0x2A, 0x2E, 0xFF]));
        image::imageops::overlay(&mut canvas, &img, 0, 0);
        canvas
    };

    let mut bgra = canvas.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let shot = Arc::new(capture::Shot {
        width,
        height,
        bgra,
        origin_x,
        origin_y,
        scale,
    });
    // 选区只框住图片本身，多出来的灰底不进导出结果。
    let sel = crate::core::geom::RectF::new(0.0, 0.0, iw as f32 / scale, ih as f32 / scale);
    diag::trace(&format!(
        "edit: reopen overlay {width}x{height} at {origin_x},{origin_y}"
    ));

    let result = cx.open_window(capture_window_options(&shot), move |window, cx| {
        cx.new(|cx| overlay::Overlay::new_editing(shot, sel, window, cx))
    });
    match result {
        Ok(handle) => {
            let _ = handle.update(cx, |_overlay, window, cx| {
                winutil::present(window, true);
                window.activate_window();
                window.defer(cx, |window, _cx| {
                    winutil::present(window, true);
                    window.activate_window();
                });
            });
        }
        Err(_) => diag::trace("edit: open_window failed"),
    }
}

pub fn open_settings(cx: &mut gpui::App) {
    settings::open(cx, start_capture_from_settings);
}

pub fn handle_tray_command(cmd: crate::platform::TrayCommand, cx: &mut gpui::App) {
    eprintln!("[pixpin-rs] tray command: {cmd:?}");
    diag::trace(&format!("tray command: {cmd:?}"));
    match cmd {
        crate::platform::TrayCommand::Capture => start_capture(cx),
        crate::platform::TrayCommand::Settings => open_settings(cx),
        crate::platform::TrayCommand::Quit => cx.quit(),
    }
}
