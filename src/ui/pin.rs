//! 贴图窗口：一张置顶的无边框小窗，钉在屏幕上。
//!
//! 交互：
//!
//! * 左键拖动移动（交给 Windows 原生拖拽线程，吸附和 DPI 由系统处理）
//! * 滚轮以光标为锚点缩放，窗口跟着变大变小
//! * 双击取消钉图
//! * 右键弹出小菜单：编辑 / 取字 / 复制 / 保存 / 缩放 / 关闭
//!
//! 「编辑」会把贴图重新送回全屏覆盖层（底图就是这张图本身），
//! 所以标注、取字、再贴图、保存全都复用同一套代码，不需要第二套界面。

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, hsla, img, px, rgb, Context, IntoElement, MouseButton, MouseDownEvent, ObjectFit, Render,
    RenderImage, ScrollWheelEvent, Window,
};

use crate::ui::render_image as render_img;

/// 缩放范围：太小看不见、太大 GPU 纹理吃不住。
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 4.0;
/// 重采样纹理的总像素上限（约 64MB RGBA），超过就靠 GPU 拉伸。
const MAX_TEX_PIXELS: f32 = 16_000_000.0;

const MENU_W: f32 = 132.0;
const MENU_ITEM_H: f32 = 30.0;
const MENU_PAD: f32 = 5.0;

pub struct PinView {
    /// 合成图的原始 RGBA 像素。缩放、取字、再编辑都以它为基准 ——
    /// 反复缩放不会累积模糊，因为每次都是从这份原图重采样。
    base: image::RgbaImage,
    /// 100% 的纹理，缩放回到原始大小时直接复用它。
    base_tex: Arc<RenderImage>,
    /// 当前上屏的纹理（按 zoom 重采样过）。
    image: Arc<RenderImage>,
    /// 100% 时窗口的逻辑尺寸。
    base_w: f32,
    base_h: f32,
    /// 显示器 DPI 比例：逻辑像素 ↔ 图片像素的换算。
    dpi: f32,
    zoom: f32,
    /// 右键菜单左上角（窗口本地逻辑坐标）。
    menu: Option<(f32, f32)>,
    /// 临时提示（识别中 / 已复制 / 保存路径…）。
    toast: Option<String>,
}

impl PinView {
    pub fn new(base: image::RgbaImage, dpi: f32) -> Self {
        let dpi = if dpi > 0.0 { dpi } else { 1.0 };
        let (w, h) = base.dimensions();
        let base_tex = render_img::from_rgba(w, h, base.clone().into_raw());
        Self {
            base_w: w as f32 / dpi,
            base_h: h as f32 / dpi,
            base,
            image: base_tex.clone(),
            base_tex,
            dpi,
            zoom: 1.0,
            menu: None,
            toast: None,
        }
    }

    fn size_at(&self, zoom: f32) -> (f32, f32) {
        (self.base_w * zoom, self.base_h * zoom)
    }

    /// 按目标比例重采样纹理。缩小走 CatmullRom 比 GPU 直接拉伸清楚得多；
    /// 放大也只是插值，但至少尺寸对得上，不会被 GPU 再糊一层。
    fn resample(&self, zoom: f32) -> Arc<RenderImage> {
        // 回到 100% 时必须拿原始纹理：否则会把上一次缩小过的图当基准，
        // 反复缩放几次就糊成一片了。
        if (zoom - 1.0).abs() < 1e-4 {
            return self.base_tex.clone();
        }
        let (bw, bh) = self.base.dimensions();
        let w = ((bw as f32 * zoom).round() as u32).max(1);
        let h = ((bh as f32 * zoom).round() as u32).max(1);
        let resized =
            image::imageops::resize(&self.base, w, h, image::imageops::FilterType::CatmullRom);
        render_img::from_rgba(w, h, resized.into_raw())
    }

    /// 缩放到 `target`，并保持光标下的那个像素不动。
    fn zoom_to(
        &mut self,
        target: f32,
        anchor: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 重采样后的纹理就是窗口的物理像素数，放大到 4 倍时一张全屏图能到
        // 上亿像素，直接把显存吃光。按总面积封顶。
        let area = (self.base.width() * self.base.height()) as f32;
        let cap = (MAX_TEX_PIXELS / area.max(1.0)).sqrt().max(1.0);
        let target = target.clamp(ZOOM_MIN, ZOOM_MAX.min(cap));
        if (target - self.zoom).abs() < 1e-4 {
            return;
        }
        let (old_w, old_h) = self.size_at(self.zoom);
        let (new_w, new_h) = self.size_at(target);
        // 锚点在窗口里的相对位置（0..1）。
        let fx = if old_w > 0.0 {
            (anchor.0 / old_w).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fy = if old_h > 0.0 {
            (anchor.1 / old_h).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let origin = window.bounds().origin;
        let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));

        self.zoom = target;
        self.image = self.resample(target);
        self.menu = None;
        eprintln!(
            "[pixpin-rs] pin zoom {:.2}x ({new_w:.0}x{new_h:.0} logical)",
            self.zoom
        );

        window.resize(gpui::size(px(new_w), px(new_h)));
        // resize 只改尺寸，位置要自己补：锚点比例不变 ⇒ 光标下的像素不动。
        crate::platform::winutil::set_window_pos(
            window,
            ox + (old_w - new_w) * fx,
            oy + (old_h - new_h) * fy,
        );
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = ev.delta.pixel_delta(px(20.0));
        let step = f32::from(delta.y) / 20.0;
        if step.abs() < 0.01 {
            return;
        }
        // 每格（Windows 默认 3 行 = 60px）放大 10%。
        let target = self.zoom * 1.1_f32.powf(step);
        let anchor = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.zoom_to(target, anchor, window, cx);
    }

    fn on_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match index {
            // 这两条会让当前窗口离开屏幕（隐藏 / 销毁），之后不能再碰 cx。
            0 => return self.edit(window, cx),
            7 => return window.remove_window(),
            1 => self.extract_text(window, cx),
            2 => self.copy(window, cx),
            3 => self.save(window, cx),
            4 => self.zoom_to(self.zoom / 1.25, (0.0, 0.0), window, cx),
            5 => self.zoom_to(1.0, (0.0, 0.0), window, cx),
            6 => self.zoom_to(self.zoom * 1.25, (0.0, 0.0), window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// 顶部小提示，1.6 秒后自己消失。
    fn toast(&mut self, text: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.toast = Some(text.into());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1600))
                .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                this.toast = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// 二次编辑：把这张图当成底图，重新打开截图用的覆盖层。
    ///
    /// 覆盖层要独占画面，所以先把贴图窗口藏起来（不是销毁）—— 窗口位置
    /// 决定了覆盖层的落点，编辑时画面才不会跳；编辑取消时原样放回来。
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dpi = self.dpi;
        let origin = window.bounds().origin;
        let ox = (f32::from(origin.x) * dpi).round() as i32;
        let oy = (f32::from(origin.y) * dpi).round() as i32;
        let base = self.base.clone();
        // 先把自己藏起来而不是销毁：编辑被取消时还能原样放回来。
        crate::platform::winutil::stash_edit_pin(window);
        crate::app::commands::edit_image(base, ox, oy, dpi, cx);
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (w, h) = self.base.dimensions();
        let ok = match arboard::Clipboard::new() {
            Ok(mut clipboard) => clipboard
                .set_image(arboard::ImageData {
                    width: w as usize,
                    height: h as usize,
                    bytes: std::borrow::Cow::Owned(self.base.clone().into_raw()),
                })
                .is_ok(),
            Err(_) => false,
        };
        let msg = if ok { "已复制" } else { "复制失败" };
        self.toast(msg, window, cx);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = crate::services::paths::save_dir();
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("PixPinRS_{stamp}.png"));
        match self.base.save(&path) {
            Ok(()) => {
                eprintln!("[pixpin-rs] pin saved -> {}", path.display());
                self.toast("已保存", window, cx);
            }
            Err(e) => {
                eprintln!("[pixpin-rs] pin save failed: {e}");
                self.toast(format!("保存失败: {e}"), window, cx);
            }
        }
    }

    /// 直接对整张贴图取字（不需要再框选）。
    fn extract_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rgb = image::DynamicImage::ImageRgba8(self.base.clone()).to_rgb8();
        self.toast = Some("识别中…".into());
        cx.notify();

        let task = cx
            .background_executor()
            .spawn(async move { crate::services::ocr::run(rgb) });

        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _window, cx| match result {
                Ok(text) if !text.trim().is_empty() => {
                    let chars = text.chars().count();
                    let auto_copy = crate::services::config::get().ocr_auto_copy;
                    let copied = if auto_copy {
                        arboard::Clipboard::new()
                            .and_then(|mut c| c.set_text(text.clone()))
                            .is_ok()
                    } else {
                        false
                    };
                    eprintln!("[pixpin-rs] pin ocr ok: {chars} chars, copied={copied}");
                    this.toast = None;
                    cx.notify();
                    crate::ui::text_result::TextView::open(text, copied, cx);
                }
                Ok(_) => {
                    this.toast = Some("没识别到文字".into());
                    cx.notify();
                }
                Err(e) => {
                    eprintln!("[pixpin-rs] pin ocr failed: {e}");
                    this.toast = Some(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn menu_height() -> f32 {
        MENU_PAD * 2.0 + MENU_ITEM_H * 8.0
    }

    /// 光标是否落在右键菜单上。落在菜单上的左键不能当成「拖窗口」，
    /// 否则菜单项还没等到 mouse-up 就被收掉，点击直接丢失。
    fn in_menu(&self, x: f32, y: f32) -> bool {
        match self.menu {
            Some((mx, my)) => {
                x >= mx && x <= mx + MENU_W && y >= my && y <= my + Self::menu_height()
            }
            None => false,
        }
    }

    fn menu_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const ITEMS: [(&str, usize); 8] = [
            ("编辑标注", 0),
            ("提取文字", 1),
            ("复制", 2),
            ("保存", 3),
            ("缩小", 4),
            ("原始大小", 5),
            ("放大", 6),
            ("关闭", 7),
        ];
        let mut col = div()
            .flex()
            .flex_col()
            .w(px(MENU_W))
            .p(px(MENU_PAD))
            .rounded(px(8.0))
            .bg(rgb(0xFFFFFF))
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.12));
        for (i, (label, index)) in ITEMS.iter().enumerate() {
            let last = i + 1 == ITEMS.len();
            col = col.child(
                div()
                    .id(("pin-menu", *index))
                    .h(px(MENU_ITEM_H))
                    .px(px(9.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .text_size(px(12.5))
                    .text_color(if last { rgb(0xFF3B30) } else { rgb(0x1C1C1E) })
                    .hover(|s| s.bg(rgb(0x0A84FF)).text_color(rgb(0xFFFFFF)))
                    .on_click(
                        cx.listener(move |this, _ev, window, cx| this.on_menu(*index, window, cx)),
                    )
                    .child(*label),
            );
        }
        col
    }
}

impl Render for PinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root =
            div()
                .id("pixpin-pin")
                .relative()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
                        // 点在菜单上：留给菜单项的 click，别收菜单、也别开始拖 ——
                        // click 是在 mouse-up 时才派发的，中途把菜单收掉就丢了。
                        if this.in_menu(x, y) {
                            return;
                        }
                        let had_menu = this.menu.take().is_some();
                        // 双击 = 取消钉图。第一击会启动拖动线程，按键一松它自己退出，
                        // 不会挡住第二击的 click_count。
                        if ev.click_count >= 2 {
                            window.remove_window();
                            return;
                        }
                        if had_menu {
                            cx.notify();
                            return;
                        }
                        // 点一下就把焦点拿过来，否则 Windows 会把滚轮发给别的窗口。
                        crate::platform::winutil::present(window, false);
                        crate::platform::winutil::start_window_drag(window);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
                        if this.in_menu(x, y) {
                            return;
                        }
                        let (w, h) = this.size_at(this.zoom);
                        let mx = x.min(w - MENU_W - 4.0).max(4.0);
                        let my = y.min(h - Self::menu_height() - 4.0).max(4.0);
                        this.menu = if this.menu.is_some() {
                            None
                        } else {
                            Some((mx, my))
                        };
                        cx.notify();
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                    this.on_scroll(ev, window, cx)
                }))
                // 图片绝对定位铺满：不加内边距/边框，纹理像素和屏幕像素严格 1:1，
                // 100% 时不会因为被挤小 2px 而重新插值糊掉。
                .child(
                    img(self.image.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                )
                // 蓝框画在最上层，不参与布局，所以不会挤到图。
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .border_1()
                        .border_color(rgb(0x0A84FF)),
                );

        if let Some((mx, my)) = self.menu {
            root = root.child(
                div()
                    .absolute()
                    .left(px(mx))
                    .top(px(my))
                    .child(self.menu_panel(cx)),
            );
        }

        if let Some(text) = self.toast.clone() {
            root = root.child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .top(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .bg(hsla(0.0, 0.0, 0.0, 0.72))
                    .text_size(px(12.0))
                    .text_color(rgb(0xFFFFFF))
                    .child(text),
            );
        }

        root
    }
}
