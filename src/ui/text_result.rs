//! OCR 结果窗口：显示识别文字，支持完整的自由选择。
//!
//! 交互清单：拖动选中、Shift+点击扩展、双击选词、三击选整行、
//! Ctrl+A 全选、Ctrl+C 复制选中（没有选中时复制全文）、右键菜单、
//! 拖到窗口上下边缘自动滚动、Esc 关闭。
//!
//! 文本不走 GPUI 的 div 排版，而是一次 shape_text 拿到 `WrappedLine`
//! 后自己画：选择高亮和命中测试都建立在 `position_for_index` /
//! `closest_index_for_position` 之上，字节索引贯穿始终。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    canvas, div, fill, point, px, rgb, size, App, Bounds, ContentMask, Context, CursorStyle,
    FocusHandle, Font, FontFeatures, FontStyle, FontWeight, Hsla, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, TextAlign, TextRun, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};

const LINE_HEIGHT_RATIO: f32 = 1.55;
const PAD: f32 = 16.0;
const SEL_BG: Hsla = Hsla {
    h: 0.58,
    s: 1.0,
    l: 0.82,
    a: 0.35,
};
const MENU_W: f32 = 148.0;
const MENU_ITEM_H: f32 = 32.0;

fn text_font() -> Font {
    Font {
        family: "Microsoft YaHei".into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight(400.0),
        style: FontStyle::Normal,
    }
}

fn text_color() -> Hsla {
    rgb(0x24242a).into()
}

fn sel_bg() -> Hsla {
    SEL_BG
}

fn font_size() -> f32 {
    crate::services::config::get()
        .ocr_font_size
        .clamp(10.0, 24.0)
}

/// 一段排版好的文本：每段一个 WrappedLine（段内已按宽度折行）。
#[derive(Default, Clone)]
struct Layout {
    width: Pixels,
    line_height: Pixels,
    lines: Vec<gpui::WrappedLine>,
    /// 每段在原文中的起始字节
    starts: Vec<usize>,
    /// 每段在内容坐标系里的顶部 y
    tops: Vec<Pixels>,
    total: Pixels,
}

impl Layout {
    fn build(window: &mut Window, text: SharedString, width: Pixels) -> Option<Self> {
        let fs = font_size();
        let lh = fs * LINE_HEIGHT_RATIO;
        let font = text_font();
        let color = text_color();
        let font_size = px(fs);
        let line_height = px(lh);
        let run = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let lines: Vec<gpui::WrappedLine> = window
            .text_system()
            .shape_text(text, font_size, &[run], Some(width), None)
            .ok()?
            .into_iter()
            .collect();

        let mut starts = Vec::with_capacity(lines.len());
        let mut tops = Vec::with_capacity(lines.len());
        let mut heights = Vec::with_capacity(lines.len());
        let mut off = px(0.0);
        let mut start = 0usize;
        for line in &lines {
            let h = line.size(line_height).height;
            starts.push(start);
            tops.push(off);
            heights.push(h);
            off += h;
            start += line.len() + 1; // +1 for '\n'
        }
        Some(Self {
            width,
            line_height,
            lines,
            starts,
            tops,
            total: off,
        })
    }

    /// 把窗口坐标点映射到全局字节索引。
    ///
    /// 点击点落在文字外（左 padding、最后一行下方、行尾右侧）时收敛到
    /// 最近的字符边界，而不是返回 None —— 否则点击空白处选区会「卡住」。
    fn hit(&self, pos: Point<Pixels>, origin: Point<Pixels>, scroll: Pixels) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let x = (pos.x - origin.x).max(px(0.0));
        let y = (pos.y - origin.y + scroll).max(px(0.0));

        // 最后一行下方：收在全文末尾。
        if y >= self.total {
            let last = self.lines.len() - 1;
            return Some(self.starts[last] + self.lines[last].len());
        }

        let seg = self
            .tops
            .iter()
            .enumerate()
            .rfind(|(_, &top)| y >= top)
            .map(|(i, _)| i)?;
        let line = &self.lines[seg];
        // y 超出该段（点在段内最后一行的下半行空隙）时压回最后一行，
        // 否则 gpui 的 `_index_for_position` 会返回 Err(0)，索引跳回段首。
        let rel_y = (y - self.tops[seg]).min(line.size(self.line_height).height - px(0.5));
        let local = line
            .closest_index_for_position(point(x, rel_y), self.line_height)
            .unwrap_or_else(|e| e);
        Some((self.starts[seg] + local).min(self.total_len()))
    }

    fn total_len(&self) -> usize {
        self.starts
            .last()
            .map(|s| s + self.lines[self.lines.len() - 1].len())
            .unwrap_or(0)
    }
}

// ---- 词 / 行边界 ----

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// 同类字符归为一个「词」：CJK 连续成串，字母数字下划线成词，
/// 空白归空白，标点各自成词。双击选词用。
fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if is_cjk(c) {
        2
    } else if c.is_alphanumeric() || c == '_' {
        3
    } else {
        1
    }
}

/// 把字节索引修正到所在字符的起点。
fn char_start(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn word_around(text: &str, idx: usize) -> (usize, usize) {
    let idx = char_start(text, idx);
    if idx >= text.len() {
        let s = text.char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
        return (s, text.len());
    }
    let cls = text[idx..].chars().next().map(char_class).unwrap_or(0);
    let mut s = idx;
    let mut e = idx + text[idx..].chars().next().map_or(1, |c| c.len_utf8());
    while s > 0 {
        let prev = text[..s].chars().next_back().unwrap();
        if char_class(prev) != cls {
            break;
        }
        s -= prev.len_utf8();
    }
    while e < text.len() {
        let next = text[e..].chars().next().unwrap();
        if char_class(next) != cls {
            break;
        }
        e += next.len_utf8();
    }
    (s, e)
}

/// 三击选整行（一个排版段 = 源文本里的一行）。
fn line_around(text: &str, idx: usize) -> (usize, usize) {
    let idx = char_start(text, idx);
    let s = text[..idx].rfind('\n').map_or(0, |p| p + 1);
    let e = text[idx..].find('\n').map_or(text.len(), |p| idx + p);
    (s, e)
}

pub struct TextView {
    text: String,
    auto_copied: bool,
    focus: FocusHandle,
    layout: Rc<RefCell<Option<Layout>>>,
    canvas_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// 选区锚点与活动端（字节索引）
    anchor: Option<usize>,
    head: Option<usize>,
    scroll: f32,
    dragging: bool,
    /// 连击检测：(时间, x, y, 次数)
    last_click: Option<(Instant, f32, f32, u8)>,
    /// 右键菜单左上角（相对文本区域）
    menu_pos: Option<Point<Pixels>>,
}

impl TextView {
    pub fn open(text: String, auto_copied: bool, cx: &mut App) {
        let bounds = Bounds::centered(None, size(px(560.0), px(460.0)), cx);
        let res = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("提取的文字".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| TextView {
                    text,
                    auto_copied,
                    focus: cx.focus_handle(),
                    layout: Rc::new(RefCell::new(None)),
                    canvas_bounds: Rc::new(RefCell::new(None)),
                    anchor: None,
                    head: None,
                    scroll: 0.0,
                    dragging: false,
                    last_click: None,
                    menu_pos: None,
                });
                window.focus(&view.read(cx).focus);
                crate::platform::winutil::set_app_icon(window);
                // 刚关掉覆盖层时前台还挂在别的进程上，SetForegroundWindow 会被
                // 前台锁直接拒掉（窗口只在任务栏闪一下）。present 里做了
                // TOPMOST + AttachThreadInput，即使激活失败也保证看得见。
                crate::platform::winutil::present(window, true);
                // GPUI 自己的 activate 会模拟一次 Alt 按键，让本进程变成「最后
                // 收到输入的进程」，这是绕过前台锁最稳的一招。
                window.activate_window();
                // 构建闭包执行时窗口往往还没真正 map，这一帧的 SetForegroundWindow
                // 会被系统忽略，下一帧再抢一次才落地。
                window.defer(cx, |window, _cx| {
                    crate::platform::winutil::present(window, true);
                    window.activate_window();
                });
                view
            },
        );
        if let Err(e) = res {
            eprintln!("[pixpin-rs] 打开文字窗口失败: {e}");
        }
    }

    fn sel_min(&self) -> usize {
        match (self.anchor, self.head) {
            (Some(a), Some(b)) => a.min(b),
            _ => 0,
        }
    }
    fn sel_max(&self) -> usize {
        match (self.anchor, self.head) {
            (Some(a), Some(b)) => a.max(b),
            _ => 0,
        }
    }
    fn has_sel(&self) -> bool {
        match (self.anchor, self.head) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }
    fn select_all(&mut self) {
        self.anchor = Some(0);
        self.head = Some(self.text.len());
    }
    fn copy_sel(&self) -> usize {
        let (s, e) = if self.has_sel() {
            (self.sel_min(), self.sel_max())
        } else {
            (0, self.text.len())
        };
        if s >= e {
            return 0;
        }
        let txt = self.text[s..e].to_string();
        let n = txt.chars().count();
        let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(txt));
        n
    }
    fn clamp_scroll(&mut self) {
        if let Ok(lay) = self.layout.try_borrow() {
            if let Some(ref l) = *lay {
                let vh = self
                    .canvas_bounds
                    .borrow()
                    .map_or(0.0, |b| f32::from(b.size.height));
                let max = (f32::from(l.total) - vh).max(0.0);
                self.scroll = self.scroll.clamp(0.0, max);
            }
        }
    }

    fn hit_index(&self, pos: Point<Pixels>) -> Option<usize> {
        let bs = self.canvas_bounds.borrow();
        let bounds = (*bs)?;
        drop(bs);
        let lay = self.layout.borrow();
        let layout = lay.as_ref()?;
        layout.hit(pos, bounds.origin, px(self.scroll))
    }

    fn hint_text(&self) -> String {
        if self.auto_copied {
            "已自动复制全文 · 拖动选中 · 双击选词 · 三击选行 · Ctrl+C 复制选中 · 右键菜单".into()
        } else {
            "拖动选中文字 · 双击选词 · 三击选行 · Ctrl+C 复制 · 右键菜单 · Esc 关闭".into()
        }
    }

    fn close_menu(&mut self) {
        self.menu_pos = None;
    }
}

impl Render for TextView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.layout.clone();
        let bounds_store = self.canvas_bounds.clone();
        let sel_min = self.sel_min();
        let sel_max = self.sel_max();
        let show_sel = self.has_sel();
        let scroll = px(self.scroll);
        let text: SharedString = self.text.clone().into();
        let hint = self.hint_text();
        let menu_pos = self.menu_pos;

        div()
            .id("ocr-result")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(0xfbfbfa))
            .p(px(PAD))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, _cx| {
                let ctrl = ev.keystroke.modifiers.control;
                let k = ev.keystroke.key.to_lowercase();
                crate::platform::diag::trace(&format!(
                    "textwin key k={k} ctrl={ctrl} sel={:?}/{:?}",
                    this.anchor, this.head
                ));
                match k.as_str() {
                    "escape" if !ctrl => {
                        if this.menu_pos.is_some() {
                            this.close_menu();
                            _cx.notify();
                        } else {
                            window.remove_window();
                        }
                    }
                    "a" if ctrl => {
                        this.select_all();
                        _cx.notify();
                    }
                    "c" if ctrl => {
                        let n = this.copy_sel();
                        crate::platform::diag::trace(&format!(
                            "textwin ctrl+c copied {n} chars (sel={}..{})",
                            this.sel_min(),
                            this.sel_max()
                        ));
                    }
                    _ => {}
                }
            }))
            .child(
                // 文本区域：canvas 自绘 + 滚动 + 选择
                div()
                    .id("ocr-canvas-wrap")
                    .flex_1()
                    .relative()
                    .cursor(CursorStyle::IBeam)
                    .child(
                        canvas(
                            {
                                let layout = layout.clone();
                                let bounds_store = bounds_store.clone();
                                let text = text.clone();
                                move |bounds: Bounds<Pixels>, window: &mut Window, _cx: &mut App| {
                                    *bounds_store.borrow_mut() = Some(bounds);
                                    let width = bounds.size.width;
                                    let mut lay = layout.borrow_mut();
                                    let needs_rebuild = lay.as_ref().map_or(true, |l| {
                                        (f32::from(l.width) - f32::from(width)).abs() > 0.5
                                    });
                                    if needs_rebuild {
                                        *lay = Layout::build(window, text.clone(), width);
                                    }
                                }
                            },
                            {
                                let layout = layout.clone();
                                move |_bounds: Bounds<Pixels>,
                                      _state: (),
                                      window: &mut Window,
                                      cx: &mut App| {
                                    let lay_opt = layout.borrow();
                                    let Some(lay) = lay_opt.as_ref() else {
                                        return;
                                    };
                                    let origin = _bounds.origin;
                                    let clip = Some(Bounds {
                                        origin,
                                        size: _bounds.size,
                                    });
                                    window.with_content_mask(
                                        clip.map(|b| ContentMask { bounds: b }),
                                        |window| {
                                            // ---- 绘制选择高亮 ----
                                            if show_sel && sel_min != sel_max {
                                                for (i, line) in lay.lines.iter().enumerate() {
                                                    let seg_start = lay.starts[i];
                                                    let seg_end = seg_start + line.len();
                                                    if sel_max <= seg_start || sel_min >= seg_end {
                                                        continue;
                                                    }
                                                    let a = sel_min.max(seg_start) - seg_start;
                                                    let b = sel_max.min(seg_end) - seg_start;
                                                    let pa =
                                                        line.position_for_index(a, lay.line_height);
                                                    let pb =
                                                        line.position_for_index(b, lay.line_height);
                                                    let (Some(pa), Some(pb)) = (pa, pb) else {
                                                        continue;
                                                    };
                                                    let top = origin.y + lay.tops[i] - scroll;
                                                    let w = lay.width;
                                                    let lh = lay.line_height;
                                                    let bg = sel_bg();

                                                    if pa.y == pb.y {
                                                        let x0 = origin.x + pa.x;
                                                        let x1 = origin.x + pb.x;
                                                        let y = top + pa.y;
                                                        window.paint_quad(fill(
                                                            Bounds {
                                                                origin: point(x0, y),
                                                                size: size(x1 - x0, lh),
                                                            },
                                                            bg,
                                                        ));
                                                    } else {
                                                        // 首行
                                                        let y0 = top + pa.y;
                                                        let x0 = origin.x + pa.x;
                                                        window.paint_quad(fill(
                                                            Bounds {
                                                                origin: point(x0, y0),
                                                                size: size(w - pa.x, lh),
                                                            },
                                                            bg,
                                                        ));
                                                        // 中间整行
                                                        let mut row_y = y0 + lh;
                                                        while row_y + px(0.1) < top + pb.y {
                                                            window.paint_quad(fill(
                                                                Bounds {
                                                                    origin: point(origin.x, row_y),
                                                                    size: size(w, lh),
                                                                },
                                                                bg,
                                                            ));
                                                            row_y += lh;
                                                        }
                                                        // 末行
                                                        let y_last = top + pb.y;
                                                        window.paint_quad(fill(
                                                            Bounds {
                                                                origin: point(origin.x, y_last),
                                                                size: size(pb.x, lh),
                                                            },
                                                            bg,
                                                        ));
                                                    }
                                                }
                                            }

                                            // ---- 绘制文字 ----
                                            for (i, line) in lay.lines.iter().enumerate() {
                                                let y = origin.y + lay.tops[i] - scroll;
                                                let _ = line.paint(
                                                    point(origin.x, y),
                                                    lay.line_height,
                                                    TextAlign::Left,
                                                    None,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        },
                                    );
                                }
                            },
                        )
                        .size_full(),
                    )
                    // ---- 右键菜单 ----
                    .children(menu_pos.map(|mp| {
                        let has_sel = self.has_sel();
                        let panel = div()
                            .absolute()
                            .left(mp.x)
                            .top(mp.y)
                            .w(px(MENU_W))
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0xe3e5e8))
                            .rounded(px(8.0))
                            .py(px(4.0))
                            .shadow_lg()
                            .occlude()
                            .cursor(CursorStyle::Arrow)
                            .on_mouse_down_out(cx.listener(
                                |this, ev: &MouseDownEvent, _window, cx| {
                                    // 点在菜单外：只收菜单，不让这次点击再去动选区。
                                    crate::platform::diag::trace(&format!(
                                        "menu out: pos=({},{})",
                                        f32::from(ev.position.x),
                                        f32::from(ev.position.y)
                                    ));
                                    this.close_menu();
                                    cx.stop_propagation();
                                },
                            ));
                        let mk_item = |label: &'static str, action: MenuAction| {
                            div()
                                .id(label)
                                .h(px(MENU_ITEM_H))
                                .px(px(12.0))
                                .flex()
                                .items_center()
                                .text_size(px(12.5))
                                .text_color(rgb(0x2a2a30))
                                .rounded(px(5.0))
                                .mx(px(4.0))
                                .hover(|s| s.bg(rgb(0xf0f2f5)))
                                .child(label)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _ev: &MouseDownEvent, window, cx| {
                                        this.close_menu();
                                        crate::platform::diag::trace("menu item fired");
                                        match action {
                                            MenuAction::Copy => {
                                                let n = this.copy_sel();
                                                crate::platform::diag::trace(&format!(
                                                    "menu copy {n} chars"
                                                ));
                                            }
                                            MenuAction::SelectAll => this.select_all(),
                                            MenuAction::Close => window.remove_window(),
                                        }
                                        cx.notify();
                                    }),
                                )
                        };
                        panel
                            .child(mk_item(
                                if has_sel {
                                    "复制选中"
                                } else {
                                    "复制全文"
                                },
                                MenuAction::Copy,
                            ))
                            .child(mk_item("全选", MenuAction::SelectAll))
                            .child(mk_item("关闭窗口", MenuAction::Close))
                            .into_any_element()
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                            let bs = this.canvas_bounds.borrow();
                            let Some(bounds) = *bs else {
                                return;
                            };
                            drop(bs);
                            // 菜单贴着光标弹出，且整块都留在文本区域内。
                            let x = f32::from(ev.position.x - bounds.origin.x)
                                .clamp(0.0, (f32::from(bounds.size.width) - MENU_W - 4.0).max(0.0));
                            let y = f32::from(ev.position.y - bounds.origin.y).clamp(
                                0.0,
                                (f32::from(bounds.size.height) - MENU_ITEM_H * 3.0 - 12.0).max(0.0),
                            );
                            this.menu_pos = Some(point(px(x), px(y)));
                            crate::platform::diag::trace(&format!("menu open at local=({x},{y})"));
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                            let pos = ev.position;
                            let Some(idx) = this.hit_index(pos) else {
                                return;
                            };
                            // 连击判定：350ms 内、位移 4px 内算同一次多击。
                            let now = Instant::now();
                            let (lx, ly) = (f32::from(pos.x), f32::from(pos.y));
                            let count = match this.last_click {
                                Some((t, x, y, c))
                                    if now.duration_since(t).as_millis() < 350
                                        && (x - lx).abs() < 4.5
                                        && (y - ly).abs() < 4.5 =>
                                {
                                    if c >= 3 {
                                        1
                                    } else {
                                        c + 1
                                    }
                                }
                                _ => 1,
                            };
                            this.last_click = Some((now, lx, ly, count));

                            if ev.modifiers.shift {
                                // Shift+点击：从现有锚点扩展到新位置。
                                if this.anchor.is_some() {
                                    this.head = Some(idx.clamp(0, this.text.len()));
                                    cx.notify();
                                    return;
                                }
                            }
                            let (s, e) = match count {
                                2 => word_around(&this.text, idx),
                                3 => line_around(&this.text, idx),
                                _ => (idx, idx),
                            };
                            crate::platform::diag::trace(&format!(
                                "textwin down cnt={count} idx={idx} sel={s}..{e}"
                            ));
                            this.anchor = Some(s.clamp(0, this.text.len()));
                            this.head = Some(e.clamp(0, this.text.len()));
                            this.dragging = true;
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                        if ev.pressed_button != Some(MouseButton::Left) {
                            // 在窗口外松开了按键的话，dragging 状态会残留，这里兜底。
                            this.dragging = false;
                            return;
                        }
                        if !this.dragging {
                            return;
                        }
                        let bs = this.canvas_bounds.borrow();
                        let Some(bounds) = *bs else {
                            return;
                        };
                        drop(bs);

                        // 拖到上下边缘时朝那个方向滚动。
                        let vh = f32::from(bounds.size.height);
                        let ly = f32::from(ev.position.y - bounds.origin.y);
                        let edge = 26.0;
                        let mut scrolled = false;
                        if ly < edge {
                            this.scroll = (this.scroll - (edge - ly) * 0.35).max(0.0);
                            scrolled = true;
                        } else if ly > vh - edge {
                            if let Ok(lay) = this.layout.try_borrow() {
                                if let Some(ref l) = *lay {
                                    let max = (f32::from(l.total) - vh).max(0.0);
                                    this.scroll =
                                        (this.scroll + (ly - (vh - edge)) * 0.35).min(max);
                                    scrolled = true;
                                }
                            }
                        }
                        let _ = scrolled;

                        if let Some(idx) = this.hit_index(ev.position) {
                            this.head = Some(idx.clamp(0, this.text.len()));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _ev: &MouseUpEvent, _window, _cx| {
                            this.dragging = false;
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _ev: &MouseUpEvent, _window, _cx| {
                            this.dragging = false;
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _window, cx| {
                        let delta_y = match ev.delta {
                            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                            gpui::ScrollDelta::Lines(l) => l.y * 20.0,
                        };
                        this.scroll -= delta_y;
                        this.clamp_scroll();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x8a8a85))
                    .child(hint),
            )
    }
}

enum MenuAction {
    Copy,
    SelectAll,
    Close,
}
