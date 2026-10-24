//! 设置窗口：左侧导航 + 右侧卡片式设置页，布局对照 PixPin 的「配置」窗口。
//!
//! 只放真实生效的设置项：
//! * 系统 —— 开机自启（写 HKCU Run）、关闭主窗口缩到托盘
//! * 截图 —— 全局热键、抓屏行为（只读说明）
//! * 保存 —— 保存目录、便携模式状态
//! * 文字识别 —— 自动复制全文、结果窗口字号、模型目录
//! * 关于 —— 版本与依赖信息
//!
//! 控件全部自绘：复选框是 18×18 圆角方块，下拉是自管理的浮层列表
//! （挂在卡片末尾、绝对定位，保证画在后面的行之上），没有任何位图资源。

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rgb, rgba, size, App, Bounds, Context, CursorStyle, Div, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, Render, SharedString, Styled, TitlebarOptions,
    Window, WindowBounds, WindowOptions,
};

use crate::ui::icons::{paint_icon, Icon};

static OPEN: AtomicBool = AtomicBool::new(false);
static HANDLE: std::sync::Mutex<Option<gpui::WindowHandle<SettingsView>>> =
    std::sync::Mutex::new(None);

const BLUE: u32 = 0x2F6BFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    System,
    Capture,
    Save,
    Ocr,
    About,
}

impl Page {
    fn all() -> [(Page, Icon, &'static str); 5] {
        [
            (Page::System, Icon::Gear, "系统"),
            (Page::Capture, Icon::Rect, "截图"),
            (Page::Save, Icon::Save, "保存"),
            (Page::Ocr, Icon::Ocr, "文字识别"),
            (Page::About, Icon::Info, "关于"),
        ]
    }
    fn title(self) -> &'static str {
        match self {
            Page::System => "系统",
            Page::Capture => "截图",
            Page::Save => "保存",
            Page::Ocr => "文字识别",
            Page::About => "关于",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Toggle {
    Autostart,
    RcCancel,
    OcrAutoCopy,
}

pub struct SettingsView {
    page: Page,
    /// 当前展开的下拉（只有字号一个下拉）
    dd_open: bool,
    focus: gpui::FocusHandle,
    /// 一次性提示（如自启写入失败）
    notice: Option<SharedString>,
    /// 截图按钮的动作由 app 层注入，避免 UI 反向依赖 main/commands。
    capture: fn(&mut App),
}

pub fn open(cx: &mut App, capture: fn(&mut App)) {
    if OPEN.swap(true, Ordering::SeqCst) {
        let handle = HANDLE.lock().ok().and_then(|g| *g);
        if let Some(h) = handle {
            // 已开着：翻到前台即可，不置顶 —— present(false) 带抢前台
            // （AttachThreadInput + SetForegroundWindow），前台锁拦不住。
            let _ = h.update(cx, |_this, window, _cx| {
                crate::platform::winutil::present(window, false);
                window.activate_window();
            });
        }
        return;
    }
    let bounds = Bounds::centered(None, size(px(800.0), px(580.0)), cx);
    let res = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("PixPin-RS 配置".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| SettingsView {
                page: Page::System,
                dd_open: false,
                focus: cx.focus_handle(),
                notice: None,
                capture,
            });
            window.focus(&view.read(cx).focus);
            crate::platform::winutil::set_app_icon(window);
            window.on_window_should_close(cx, |_window, _cx| {
                OPEN.store(false, Ordering::SeqCst);
                true
            });
            crate::platform::winutil::present(window, false);
            window.activate_window();
            window.defer(cx, |window, _cx| {
                crate::platform::winutil::present(window, false);
                window.activate_window();
            });
            view
        },
    );
    match res {
        Ok(h) => {
            if let Ok(mut guard) = HANDLE.lock() {
                *guard = Some(h);
            }
        }
        Err(e) => {
            OPEN.store(false, Ordering::SeqCst);
            eprintln!("[pixpin-rs] 打开设置窗口失败: {e}");
        }
    }
}

impl SettingsView {
    fn set_toggle(&mut self, t: Toggle, cx: &mut Context<Self>) {
        let mut cfg = crate::services::config::get();
        match t {
            Toggle::Autostart => {
                let on = !crate::services::config::autostart_enabled();
                match crate::services::config::set_autostart(on) {
                    Ok(()) => self.notice = None,
                    Err(e) => self.notice = Some(e.into()),
                }
            }
            Toggle::RcCancel => {
                cfg.rc_cancel_capture = !cfg.rc_cancel_capture;
                crate::services::config::store(&cfg);
            }
            Toggle::OcrAutoCopy => {
                cfg.ocr_auto_copy = !cfg.ocr_auto_copy;
                crate::services::config::store(&cfg);
            }
        }
        cx.notify();
    }

    fn reset_defaults(&mut self, cx: &mut Context<Self>) {
        let _ = crate::services::config::set_autostart(false);
        crate::services::config::store(&crate::services::config::Config::default());
        self.notice = Some("已恢复默认设置".into());
        cx.notify();
    }

    // ---- 控件 ----

    fn nav_item(
        &self,
        cx: &mut Context<Self>,
        page: Page,
        icon: Icon,
        label: &'static str,
    ) -> impl IntoElement {
        let selected = self.page == page;
        let (fg, bg) = if selected {
            (rgb(0xFFFFFF), rgb(BLUE))
        } else {
            (rgb(0x3A3F45), rgba(0x00000000))
        };
        div()
            .id(label)
            .h(px(38.0))
            .px(px(12.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.5))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| if !selected { s.bg(rgb(0xF0F1F4)) } else { s })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _ev, _window, cx| {
                    this.page = page;
                    this.dd_open = false;
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        let c = if selected {
                            rgb(0xFFFFFF)
                        } else {
                            rgb(0x6B7280)
                        };
                        paint_icon(
                            window,
                            icon,
                            f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                            f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
                            16.0,
                            c.into(),
                            1.7,
                        );
                    },
                )
                .size(px(16.0)),
            )
            .child(label)
    }

    /// 一行设置：左标签、右控件；`last` 决定是否画分隔线。
    fn row(&self, label: &str, control: impl IntoElement, last: bool) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(46.0))
            .when(!last, |d| d.border_b_1().border_color(rgb(0xF0F1F3)))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(0x24272E))
                    .child(label.to_string()),
            )
            .child(control)
    }

    fn checkbox(&self, cx: &mut Context<Self>, t: Toggle, checked: bool) -> impl IntoElement {
        let base = div()
            .w(px(18.0))
            .h(px(18.0))
            .rounded(px(5.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::PointingHand);
        let hit = if checked {
            base.bg(rgb(BLUE)).child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xFFFFFF))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("✓"),
            )
        } else {
            base.bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(rgb(0xC9CDD4))
        };
        hit.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                this.set_toggle(t, cx);
            }),
        )
    }

    /// 只读的取值控件
    fn value(&self, s: impl Into<String>) -> impl IntoElement {
        let s: String = s.into();
        div()
            .max_w(px(420.0))
            .text_size(px(12.5))
            .text_color(rgb(0x5A6068))
            .child(s)
    }

    fn action_button(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        action: BtnAction,
    ) -> impl IntoElement {
        let capture = self.capture;
        div()
            .id(label)
            .px(px(12.0))
            .h(px(28.0))
            .rounded(px(6.0))
            .bg(rgb(BLUE))
            .text_color(rgb(0xFFFFFF))
            .text_size(px(12.5))
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(0x2759DB)))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _ev, _window, cx| match action {
                    BtnAction::Capture => {
                        capture(cx);
                    }
                    BtnAction::OpenSaveDir => open_in_explorer(&crate::services::paths::save_dir()),
                    BtnAction::OpenModelDir => {
                        let dir = std::env::current_exe()
                            .ok()
                            .and_then(|e| e.parent().map(|p| p.join("models")))
                            .unwrap_or_default();
                        open_in_explorer(&dir);
                    }
                }),
            )
    }

    fn font_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fs = crate::services::config::get().ocr_font_size;
        let label = match fs {
            v if v < 12.5 => "小",
            v if v < 14.0 => "标准",
            _ => "大",
        };
        let open = self.dd_open;
        let control = div()
            .id("dd-font")
            .w(px(120.0))
            .h(px(30.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(0xD3D7DD))
            .bg(rgb(0xFFFFFF))
            .flex()
            .items_center()
            .justify_between()
            .cursor(CursorStyle::Arrow)
            .text_size(px(12.5))
            .text_color(rgb(0x24272E))
            .child(label)
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(0x8A9099))
                    .child("▾"),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseDownEvent, _window, cx| {
                    this.dd_open = !this.dd_open;
                    cx.notify();
                }),
            );
        if !open {
            return control.into_any_element();
        }
        // 展开的选项列表：挂在卡片末尾统一绘制（见 render 里 Ocr 卡片），
        // 这里只放控制位。选项列表由 dd_list() 提供。
        control.into_any_element()
    }

    fn dd_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fs = crate::services::config::get().ocr_font_size;
        let opts: [(f32, &'static str); 3] = [
            (12.0, "小（12px）"),
            (13.0, "标准（13px）"),
            (15.0, "大（15px）"),
        ];
        let mut list = div()
            .absolute()
            .top(px(92.0))
            .right(px(18.0))
            .w(px(140.0))
            .bg(rgb(0xFFFFFF))
            .border_1()
            .border_color(rgb(0xE0E3E8))
            .rounded(px(6.0))
            .py(px(4.0))
            .shadow_lg()
            .occlude()
            .cursor(CursorStyle::Arrow)
            .on_mouse_down_out(cx.listener(|this, _ev: &MouseDownEvent, _window, cx| {
                this.dd_open = false;
                cx.stop_propagation();
            }));
        for (v, name) in opts {
            let selected = (fs - v).abs() < 0.5;
            list = list.child(
                div()
                    .id(name)
                    .h(px(28.0))
                    .px(px(10.0))
                    .mx(px(4.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .text_color(if selected { rgb(BLUE) } else { rgb(0x24272E) })
                    .bg(if selected {
                        rgb(0xF0F4FF)
                    } else {
                        rgb(0xFFFFFF)
                    })
                    .hover(|s| s.bg(rgb(0xF0F2F5)))
                    .child(name)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev, _window, cx| {
                            let mut cfg = crate::services::config::get();
                            cfg.ocr_font_size = v;
                            crate::services::config::store(&cfg);
                            this.dd_open = false;
                            cx.notify();
                        }),
                    ),
            );
        }
        // 选中后收起
        list
    }
}

enum BtnAction {
    OpenSaveDir,
    OpenModelDir,
    Capture,
}

fn open_in_explorer(dir: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

/// 卡片容器：白底圆角，内部行有分隔线。
fn card(children: impl IntoIterator<Item = impl IntoElement>) -> Div {
    let mut c = div()
        .bg(rgb(0xFFFFFF))
        .rounded(px(10.0))
        .px(px(18.0))
        .py(px(4.0))
        .flex()
        .flex_col()
        .relative();
    for ch in children {
        c = c.child(ch);
    }
    c
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = crate::services::config::get();
        let autostart = crate::services::config::autostart_enabled();

        let sidebar = div()
            .w(px(180.0))
            .h_full()
            .bg(rgb(0xFBFBFC))
            .border_r_1()
            .border_color(rgb(0xEBEDF0))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(Page::all().map(|(p, icon, label)| self.nav_item(cx, p, icon, label)))
            .child(div().flex_1())
            .child(
                div()
                    .id("reset")
                    .h(px(34.0))
                    .rounded(px(8.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.5))
                    .text_color(rgb(0xD94848))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(0xFBEFEF)))
                    .child("恢复默认设置")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _ev, _window, cx| this.reset_defaults(cx)),
                    ),
            );

        let content = match self.page {
            Page::System => self.render_system(cx, autostart),
            Page::Capture => self.render_capture(cx),
            Page::Save => self.render_save(cx),
            Page::Ocr => self.render_ocr(cx, cfg),
            Page::About => self.render_about(),
        };

        let hotkey_ok = crate::platform::hotkey::REGISTERED.load(Ordering::SeqCst);
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.to_lowercase() == "escape" {
                    OPEN.store(false, Ordering::SeqCst);
                    this.dd_open = false;
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .font_family("Microsoft YaHei")
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xF4F5F7))
            .when(!hotkey_ok, |root| {
                root.child(
                    div()
                        .id("hotkey-warn")
                        .w_full()
                        .h(px(34.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .bg(rgb(0xFDE8E8))
                        .border_b_1()
                        .border_color(rgb(0xF5B5B5))
                        .text_size(px(12.5))
                        .text_color(rgb(0xC42B1C))
                        .child("全局热键 Ctrl+Alt+X 被其他程序占用，注册失败 —— 可用左键点托盘图标截图"),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p(px(24.0))
                            .flex()
                            .flex_col()
                            .gap(px(14.0))
                            .child(
                                div()
                                    .text_size(px(19.0))
                                    .font_weight(gpui::FontWeight(600.0))
                                    .text_color(rgb(0x1F2329))
                                    .child(self.page.title()),
                            )
                            .child(content),
                    ),
            )
    }
}

impl SettingsView {
    fn render_system(&mut self, cx: &mut Context<Self>, autostart: bool) -> Div {
        let hotkey_ok = crate::platform::hotkey::REGISTERED.load(Ordering::SeqCst);
        card([
            self.row(
                "开机自动启动",
                self.checkbox(cx, Toggle::Autostart, autostart),
                false,
            )
            .into_any_element(),
            self.row(
                "全局热键",
                self.value(if hotkey_ok {
                    "Ctrl+Alt+X · 已注册"
                } else {
                    "Ctrl+Alt+X · 被占用，请用托盘左键截图"
                }),
                true,
            )
            .into_any_element(),
        ])
    }

    fn render_capture(&self, cx: &mut Context<Self>) -> Div {
        let rc_cancel = crate::services::config::get().rc_cancel_capture;
        card([
            self.row(
                "手动截图",
                self.action_button(cx, "立即截图", BtnAction::Capture),
                false,
            )
            .into_any_element(),
            self.row(
                "右键取消截图",
                self.checkbox(cx, Toggle::RcCancel, rc_cancel),
                false,
            )
            .into_any_element(),
            self.row("抓取范围", self.value("鼠标所在显示器"), false)
                .into_any_element(),
            self.row("截图时隐藏本程序窗口", self.value("始终开启"), false)
                .into_any_element(),
            self.row("截图前置提示", self.value("无（热键后直接冻结屏幕）"), true)
                .into_any_element(),
        ])
    }

    fn render_save(&self, cx: &mut Context<Self>) -> Div {
        let portable = crate::services::paths::save_dir().ends_with("screenshots");
        card([
            self.row(
                "保存目录",
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(self.value(crate::services::paths::save_dir().display().to_string()))
                    .child(self.action_button(cx, "打开目录", BtnAction::OpenSaveDir)),
                false,
            )
            .into_any_element(),
            self.row(
                "便携模式",
                self.value(if portable {
                    "已启用（配置与截图跟随程序目录）"
                } else {
                    "未启用"
                }),
                true,
            )
            .into_any_element(),
        ])
    }

    fn render_ocr(&mut self, cx: &mut Context<Self>, cfg: crate::services::config::Config) -> Div {
        let models_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join("models")))
            .unwrap_or_default();
        let has_models = models_dir.join("det.onnx").exists();
        let mut c = card([
            self.row(
                "识别后自动复制全文",
                self.checkbox(cx, Toggle::OcrAutoCopy, cfg.ocr_auto_copy),
                false,
            )
            .into_any_element(),
            self.row("结果窗口字号", self.font_dropdown(cx), false)
                .into_any_element(),
            self.row("识别语言", self.value("中英混合（横排）"), false)
                .into_any_element(),
            self.row(
                "模型状态",
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(self.value(if has_models {
                        "已加载（DBNet + PP-OCRv5）"
                    } else {
                        "缺少模型文件，取字不可用"
                    }))
                    .child(self.action_button(cx, "打开目录", BtnAction::OpenModelDir)),
                true,
            )
            .into_any_element(),
        ]);
        if self.dd_open {
            c = c.child(self.dd_list(cx));
        }
        c
    }

    fn render_about(&mut self) -> Div {
        card([
            self.row("版本", self.value(env!("CARGO_PKG_VERSION")), false)
                .into_any_element(),
            self.row("界面框架", self.value("Rust + GPUI 0.2.2"), false)
                .into_any_element(),
            self.row(
                "OCR 引擎",
                self.value("pixocr（ONNX Runtime 2.0 · DBNet + PP-OCRv5）"),
                false,
            )
            .into_any_element(),
            self.row(
                "模型来源",
                self.value("ModelScope · RapidAI/RapidOCR"),
                true,
            )
            .into_any_element(),
        ])
    }
}
