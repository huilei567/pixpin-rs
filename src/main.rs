//! PixPin-RS — a Rust + GPUI take on PixPin's core loop:
//! hotkey -> freeze the screen -> drag a region -> annotate -> copy / save / pin.
//!
//! `main.rs` 只负责声明模块和启动进程；具体流程在 `app`，窗口在 `ui`，
//! 纯逻辑在 `core`，能力在 `services`，Windows 适配在 `platform`。

mod app;
mod core;
mod platform;
mod services;
mod ui;

fn main() {
    app::run();
}
