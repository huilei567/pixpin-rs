use super::input::handle_points;
use super::*;

// ---------- 绘制辅助 ----------

fn fill_rect(
    window: &mut Window,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: impl Into<gpui::Background>,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    window.paint_quad(quad(
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        },
        px(0.0),
        color,
        px(0.0),
        hsla(0.0, 0.0, 0.0, 0.0),
        Default::default(),
    ));
}

/// 形状的外接框（橡皮擦悬停高亮用）。
fn shape_bbox(shape: &Shape) -> (f32, f32, f32, f32) {
    let bbox = |a: (f32, f32), b: (f32, f32)| {
        (
            a.0.min(b.0) - 4.0,
            a.1.min(b.1) - 4.0,
            (a.0 - b.0).abs() + 8.0,
            (a.1 - b.1).abs() + 8.0,
        )
    };
    match shape {
        Shape::Rect { a, b, .. }
        | Shape::Ellipse { a, b, .. }
        | Shape::Arrow { a, b, .. }
        | Shape::Mosaic { a, b, .. } => bbox(*a, *b),
        Shape::MosaicStroke { pts, radius, .. } => {
            let (x0, y0, x1, y1) =
                pts.iter()
                    .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |acc, p| {
                        (
                            acc.0.min(p.0),
                            acc.1.min(p.1),
                            acc.2.max(p.0),
                            acc.3.max(p.1),
                        )
                    });
            (
                x0 - radius - 3.0,
                y0 - radius - 3.0,
                x1 - x0 + radius * 2.0 + 6.0,
                y1 - y0 + radius * 2.0 + 6.0,
            )
        }
        Shape::Pen { pts, .. } => {
            let (x0, y0, x1, y1) =
                pts.iter()
                    .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |acc, p| {
                        (
                            acc.0.min(p.0),
                            acc.1.min(p.1),
                            acc.2.max(p.0),
                            acc.3.max(p.1),
                        )
                    });
            (x0 - 4.0, y0 - 4.0, x1 - x0 + 8.0, y1 - y0 + 8.0)
        }
        Shape::Text { pos, size, w, .. } => (pos.0 - 3.0, pos.1 - 3.0, w + 6.0, size * 1.25 + 6.0),
    }
}

fn stroke_rect_outline(
    window: &mut Window,
    r: RectF,
    color: impl Into<gpui::Background>,
    width: f32,
) {
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(point(px(r.x), px(r.y)));
    b.line_to(point(px(r.right()), px(r.y)));
    b.line_to(point(px(r.right()), px(r.bottom())));
    b.line_to(point(px(r.x), px(r.bottom())));
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// 带圆角和描边的面板 —— 工具栏和浮层都用它。
fn panel(window: &mut Window, r: RectF, radius: f32, bg: Hsla, border: Hsla) {
    window.paint_quad(quad(
        Bounds {
            origin: point(px(r.x), px(r.y)),
            size: size(px(r.w), px(r.h)),
        },
        px(radius),
        bg,
        px(1.0),
        border,
        Default::default(),
    ));
}

fn paint_shape(window: &mut Window, cx: &mut App, shape: &Shape, shot: &Shot) {
    match shape {
        Shape::Rect { a, b, color, width } => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = (a.0.max(b.0), a.1.max(b.1));
            let mut builder = PathBuilder::stroke(px(*width));
            builder.move_to(point(px(x0), px(y0)));
            builder.line_to(point(px(x1), px(y0)));
            builder.line_to(point(px(x1), px(y1)));
            builder.line_to(point(px(x0), px(y1)));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(*color));
            }
        }
        Shape::Ellipse { a, b, color, width } => {
            let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            let (rx, ry) = ((a.0 - b.0).abs() / 2.0, (a.1 - b.1).abs() / 2.0);
            let mut builder = PathBuilder::stroke(px(*width));
            let steps = 72;
            for i in 0..=steps {
                let t = i as f32 / steps as f32 * std::f32::consts::TAU;
                let p = point(px(cx + rx * t.cos()), px(cy + ry * t.sin()));
                if i == 0 {
                    builder.move_to(p);
                } else {
                    builder.line_to(p);
                }
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(*color));
            }
        }
        Shape::Arrow { a, b, color, width } => {
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let len = (dx * dx + dy * dy).sqrt().max(1e-3);
            let head = (*width * 4.0).clamp(10.0, 40.0);
            let shrink = head * 0.8;
            let end = (b.0 - dx / len * shrink, b.1 - dy / len * shrink);
            let mut builder = PathBuilder::stroke(px(*width));
            builder.move_to(point(px(a.0), px(a.1)));
            builder.line_to(point(px(end.0), px(end.1)));
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(*color));
            }
            let tri = paint::arrow_head(*a, *b, *width);
            let mut fill = PathBuilder::fill();
            fill.move_to(point(px(tri[0].0), px(tri[0].1)));
            fill.line_to(point(px(tri[1].0), px(tri[1].1)));
            fill.line_to(point(px(tri[2].0), px(tri[2].1)));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, rgb(*color));
            }
        }
        Shape::Pen { pts, color, width } => {
            if pts.len() < 2 {
                return;
            }
            // 和导出一致：Chaikin 平滑后再画，避免离散事件点连出折线硬角。
            let smooth = paint::smooth_pts(pts);
            if smooth.len() < 2 {
                return;
            }
            let mut builder = PathBuilder::stroke(px(*width));
            for (i, p) in smooth.iter().enumerate() {
                let pt = point(px(p.0), px(p.1));
                if i == 0 {
                    builder.move_to(pt);
                } else {
                    builder.line_to(pt);
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(*color));
            }
        }
        Shape::MosaicStroke { pts, radius, block } => {
            // 涂抹预览：沿插值点盖块网格圆盘，块与导出网格同锚点。
            let bs = block.max(3.0);
            let step = (radius * 0.5).max(3.0);
            for (px, py) in paint::interp_pts(pts, step) {
                let mut by = ((py - radius) / bs).floor() * bs;
                let y1 = py + radius;
                while by < y1 {
                    let mut bx = ((px - radius) / bs).floor() * bs;
                    let x1 = px + radius;
                    while bx < x1 {
                        let cxb = bx + bs / 2.0;
                        let cyb = by + bs / 2.0;
                        let (dx, dy) = (cxb - px, cyb - py);
                        if dx * dx + dy * dy <= radius * radius {
                            let sx = (cxb * shot.scale) as i32;
                            let sy = (cyb * shot.scale) as i32;
                            if let Some((r, g, b)) = sample(shot, sx, sy) {
                                fill_rect(
                                    window,
                                    bx,
                                    by,
                                    bs,
                                    bs,
                                    rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
                                );
                            }
                        }
                        bx += bs;
                    }
                    by += bs;
                }
            }
        }
        Shape::Mosaic { a, b, block } => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = (a.0.max(b.0), a.1.max(b.1));
            let bs = block.max(3.0);
            let mut y = y0;
            while y < y1 {
                let bh = bs.min(y1 - y);
                let mut x = x0;
                while x < x1 {
                    let bw = bs.min(x1 - x);
                    // 预览取块中心像素，导出时再做块内平均（见 paint::mosaic）。
                    let sx = (x + bw / 2.0) * shot.scale;
                    let sy = (y + bh / 2.0) * shot.scale;
                    if let Some((r, g, b)) = sample(shot, sx as i32, sy as i32) {
                        fill_rect(
                            window,
                            x,
                            y,
                            bw,
                            bh,
                            rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
                        );
                    }
                    x += bs;
                }
                y += bs;
            }
        }
        Shape::Text {
            pos,
            text,
            color,
            size,
            ..
        } => {
            let txt: SharedString = text.clone().into();
            let run = TextRun {
                len: txt.len(),
                font: Font {
                    family: "Microsoft YaHei".into(),
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: FontWeight(400.0),
                    style: FontStyle::Normal,
                },
                color: rgb(*color).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            if let Some(line) = window
                .text_system()
                .shape_text(txt, px(*size), &[run], None, None)
                .ok()
                .and_then(|l| l.into_iter().next())
            {
                let lh = px(size * 1.25);
                let _ = line.paint(
                    point(px(pos.0), px(pos.1)),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
    }
}

/// 从冻结帧里取一个物理像素的 RGB。
fn sample(shot: &Shot, x: i32, y: i32) -> Option<(u8, u8, u8)> {
    if x < 0 || y < 0 || x >= shot.width as i32 || y >= shot.height as i32 {
        return None;
    }
    let i = ((y as usize * shot.width as usize) + x as usize) * 4;
    let bgra = &shot.bgra;
    if i + 2 >= bgra.len() {
        return None;
    }
    Some((bgra[i + 2], bgra[i + 1], bgra[i]))
}

// ---------- 渲染 ----------

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (win_w, win_h) = Self::win_size(window);
        let sel = self.sel;
        let hover_win = self.hover_win;
        let shapes = self.shapes.clone();
        let draft = self.draft.clone();
        let pen = self.pen.clone();
        let cursor = self.cursor;
        let (color, stroke) = (self.color, self.stroke);
        let tool = self.tool;
        let scale = self.scale;
        let status = self.status.clone();
        let image = self.image.clone();
        let shot = self.shot.clone();
        let dragging = !matches!(self.drag, Drag::None);
        let drawing = self.drawing;
        let erasing = self.erasing;
        let hover_btn = self.hover_btn;
        // 悬停提示牌：仅在空闲（未拖拽/绘制/擦除/菜单）时显示
        let tooltip_btn = if !drawing && !self.erasing && !dragging && self.menu_btn.is_none() {
            hover_btn
        } else {
            None
        };
        let can_undo = !self.undo_stack.is_empty();
        let can_redo = !self.redo_stack.is_empty();
        let menu_btn = self.menu_btn;
        let menu = self.menu_layout(win_w, win_h);
        let hue = self.hue;
        let sat = self.sat;
        let val = self.val;
        let editing_txt = self.editing.clone();
        let editing_size = self.text_size;
        let block = self.block;
        let mosaic_brush = self.mosaic_brush;
        let brush_radius = self.brush_radius;
        let text_size = self.text_size;
        let menu_drag = self.menu_drag;

        let tb_origin = self.tb_origin(win_w, win_h);
        let tb_rect = tb_origin.map(|(x, y)| {
            let (w, h) = Self::tb_size();
            RectF::new(x, y, w, h)
        });
        let tb_btns = self.tb_buttons(win_w, win_h);
        let (loupe_x, loupe_y) = self.loupe_pos(win_w, win_h);
        // 菜单开着、悬停工具栏按钮或正在擦除时收起放大镜。
        let show_loupe = !drawing && !erasing && self.menu_btn.is_none() && hover_btn.is_none();
        let active_btn = tool.button();

        // 光标处的色值（给放大镜面板用）
        let (cx_px, cy_px) = (
            (cursor.0 * scale).round() as i32,
            (cursor.1 * scale).round() as i32,
        );
        let cursor_rgb = sample(&shot, cx_px, cy_px).unwrap_or((0, 0, 0));
        let hex = format!(
            "#{:02X}{:02X}{:02X}",
            cursor_rgb.0, cursor_rgb.1, cursor_rgb.2
        );
        let rgb_txt = format!("rgb({}, {}, {})", cursor_rgb.0, cursor_rgb.1, cursor_rgb.2);

        let mut root = div()
            .id(ElementId::Name("pixpin-overlay".into()))
            .relative()
            .size_full()
            .bg(gpui::black())
            .font_family("Microsoft YaHei")
            .track_focus(&self.focus)
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                this.on_move(ev.position, ev.pressed_button, window);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.on_down(ev.position, window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.on_up(ev.position, window);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, _window, _cx| {
                    // 右键按下不立即动作：拖动了 = 擦除标注，原地松开 = 取消
                    // 截图（设置里可关）。Esc 永远可用。
                    this.right_press = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, _ev: &MouseUpEvent, window, cx| {
                    if this.right_erasing {
                        this.right_erasing = false;
                        this.erasing = false;
                    } else if crate::services::config::get().rc_cancel_capture {
                        this.close(window);
                    }
                    this.right_press = None;
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.on_key(ev, window, cx);
            }))
            .child(
                img(image)
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w_full()
                    .h_full()
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |_bounds, _state, window, cx| {
                                let win = window.bounds().size;
                                let (ww, wh) = (f32::from(win.width), f32::from(win.height));
                                let dim = hsla(0.0, 0.0, 0.0, 0.45);
                                let accent = hsla(0.6, 1.0, 0.5, 1.0);
                                let white = hsla(0.0, 0.0, 1.0, 1.0);

                                // ---- 遮罩 ----
                                // 悬停窗口如果本来就铺满全屏，留洞等于没有遮罩，
                                // 看不出「已在截图模式」，这种情况按无洞处理。
                                let hole = sel
                                    .filter(|s| s.w > 0.0 && s.h > 0.0)
                                    .or(hover_win.filter(|h| h.w * h.h < ww * wh * 0.97));
                                match hole {
                                    Some(s) => {
                                        fill_rect(window, 0.0, 0.0, ww, s.y, dim);
                                        fill_rect(
                                            window,
                                            0.0,
                                            s.bottom(),
                                            ww,
                                            (wh - s.bottom()).max(0.0),
                                            dim,
                                        );
                                        fill_rect(window, 0.0, s.y, s.x, s.h, dim);
                                        fill_rect(
                                            window,
                                            s.right(),
                                            s.y,
                                            (ww - s.right()).max(0.0),
                                            s.h,
                                            dim,
                                        );
                                    }
                                    None => {
                                        fill_rect(window, 0.0, 0.0, ww, wh, dim);
                                    }
                                }

                                // 悬停窗口：细边框 + 四角标记，和「已选中」区分开
                                if sel.is_none() {
                                    if let Some(hw) = hover_win {
                                        stroke_rect_outline(
                                            window,
                                            hw,
                                            hsla(0.55, 1.0, 0.6, 0.85),
                                            1.5,
                                        );
                                    }
                                }

                                // ---- 标注 ----
                                for shape in shapes.iter() {
                                    paint_shape(window, cx, shape, &shot);
                                }
                                if let Some(d) = draft.as_ref() {
                                    paint_shape(window, cx, d, &shot);
                                }
                                if pen.len() > 1 {
                                    paint_shape(
                                        window,
                                        cx,
                                        &Shape::Pen {
                                            pts: pen.clone(),
                                            color,
                                            width: stroke,
                                        },
                                        &shot,
                                    );
                                }

                                // ---- 选区边框与手柄 ----
                                if let Some(s) = sel {
                                    if s.w > 0.0 && s.h > 0.0 {
                                        stroke_rect_outline(window, s, white, 1.5);
                                        for (_, hx, hy) in handle_points(s) {
                                            let r = RectF::new(
                                                hx - HANDLE / 2.0,
                                                hy - HANDLE / 2.0,
                                                HANDLE,
                                                HANDLE,
                                            );
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(px(r.x), px(r.y)),
                                                    size: size(px(r.w), px(r.h)),
                                                },
                                                px(1.0),
                                                white,
                                                px(1.0),
                                                hsla(0.0, 0.0, 0.0, 0.45),
                                                Default::default(),
                                            ));
                                        }
                                    }
                                } else if !dragging {
                                    // 十字准线
                                    fill_rect(
                                        window,
                                        0.0,
                                        cursor.1,
                                        ww,
                                        1.0,
                                        hsla(0.0, 0.0, 1.0, 0.35),
                                    );
                                    fill_rect(
                                        window,
                                        cursor.0,
                                        0.0,
                                        1.0,
                                        wh,
                                        hsla(0.0, 0.0, 1.0, 0.35),
                                    );
                                }

                                // ---- 工具栏 ----
                                if let Some(tb) = tb_rect {
                                    panel(
                                        window,
                                        tb,
                                        TB_RADIUS,
                                        hsla(0.0, 0.0, 1.0, 0.97),
                                        hsla(0.0, 0.0, 0.0, 0.12),
                                    );
                                    // 分隔线
                                    let mut sx = tb.x + 5.0;
                                    for it in ITEMS {
                                        match it {
                                            Item::Sep => {
                                                let cx0 = sx + SEP_W / 2.0;
                                                fill_rect(
                                                    window,
                                                    cx0,
                                                    tb.y + 10.0,
                                                    1.0,
                                                    tb.h - 20.0,
                                                    hsla(0.0, 0.0, 0.0, 0.12),
                                                );
                                                sx += SEP_W;
                                            }
                                            Item::Btn(_) => sx += BTN,
                                        }
                                    }

                                    for (i, r) in tb_btns.iter() {
                                        let is_active = active_btn == Some(*i);
                                        let enabled = match *i {
                                            5 => can_undo,
                                            6 => can_redo,
                                            _ => true,
                                        };
                                        if is_active {
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(px(r.x), px(r.y)),
                                                    size: size(px(r.w), px(r.h)),
                                                },
                                                px(6.0),
                                                hsla(0.6, 1.0, 0.45, 0.16),
                                                px(0.0),
                                                hsla(0.0, 0.0, 0.0, 0.0),
                                                Default::default(),
                                            ));
                                        } else if hover_btn == Some(*i) && enabled {
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(px(r.x), px(r.y)),
                                                    size: size(px(r.w), px(r.h)),
                                                },
                                                px(6.0),
                                                hsla(0.0, 0.0, 0.0, 0.12),
                                                px(0.0),
                                                hsla(0.0, 0.0, 0.0, 0.0),
                                                Default::default(),
                                            ));
                                        }

                                        let icon_color = if !enabled {
                                            hsla(0.0, 0.0, 0.0, 0.22)
                                        } else if is_active {
                                            accent
                                        } else {
                                            hsla(0.0, 0.0, 0.11, 1.0)
                                        };
                                        let center_x = if MENU_BTNS.contains(i) {
                                            r.x + (r.w - 8.0) / 2.0
                                        } else {
                                            r.x + r.w / 2.0
                                        };
                                        paint_icon(
                                            window,
                                            BUTTONS[*i].0,
                                            center_x,
                                            r.y + r.h / 2.0,
                                            19.0,
                                            icon_color,
                                            1.5,
                                        );
                                        // ⌄ 小箭头提示二级菜单
                                        if MENU_BTNS.contains(i) {
                                            paint_icon(
                                                window,
                                                Icon::CaretDown,
                                                r.x + r.w - 7.0,
                                                r.y + r.h / 2.0 - 4.0,
                                                8.0,
                                                if menu_btn == Some(*i) || hover_btn == Some(*i) {
                                                    hsla(0.0, 0.0, 0.3, 1.0)
                                                } else {
                                                    hsla(0.0, 0.0, 0.55, 1.0)
                                                },
                                                1.3,
                                            );
                                        }
                                    }
                                }

                                // ---- 编辑中的文本 + 光标 ----
                                if let Some(((ex, ey), buf)) = editing_txt.clone() {
                                    let w = Self::measure_text(window, &buf, editing_size);
                                    let _ = w;
                                    paint_shape(
                                        window,
                                        cx,
                                        &Shape::Text {
                                            pos: (ex, ey),
                                            text: buf,
                                            color,
                                            size: editing_size,
                                            w,
                                        },
                                        &shot,
                                    );
                                    // 光标竖线
                                    fill_rect(
                                        window,
                                        ex + w + 1.0,
                                        ey + 1.0,
                                        1.5,
                                        editing_size * 1.1,
                                        accent,
                                    );
                                }

                                // ---- 橡皮擦悬停高亮（工具态或任何擦除中） ----
                                if tool == Tool::Eraser || erasing {
                                    if let Some(sel) = sel {
                                        if sel.contains(cursor.0, cursor.1) {
                                            if let Some(hs) = shapes.iter().rev().find(|s| {
                                                paint::shape_hit(s, cursor.0, cursor.1, 8.0)
                                            }) {
                                                let (bx, by, bw, bh) = shape_bbox(hs);
                                                window.paint_quad(quad(
                                                    Bounds {
                                                        origin: point(px(bx), px(by)),
                                                        size: size(px(bw), px(bh)),
                                                    },
                                                    px(4.0),
                                                    hsla(0.58, 1.0, 0.5, 0.12),
                                                    px(1.0),
                                                    hsla(0.58, 1.0, 0.5, 0.8),
                                                    Default::default(),
                                                ));
                                            }
                                        }
                                    }
                                }

                                // ---- 二级菜单 ----
                                if let (Some(_mb), Some((panel, picker, modes, sliders))) =
                                    (menu_btn, menu)
                                {
                                    window.paint_quad(quad(
                                        Bounds {
                                            origin: point(px(panel.x), px(panel.y)),
                                            size: size(px(panel.w), px(panel.h)),
                                        },
                                        px(8.0),
                                        hsla(0.0, 0.0, 1.0, 0.98),
                                        px(1.0),
                                        hsla(0.0, 0.0, 0.0, 0.14),
                                        Default::default(),
                                    ));
                                    if let Some((sv, huer, hexr)) = picker {
                                        // SV 面板：18x12 网格插值
                                        let (nc, nr) = (18, 12);
                                        let cw = sv.w / nc as f32;
                                        let chh = sv.h / nr as f32;
                                        for iy in 0..nr {
                                            for ix in 0..nc {
                                                let sa = (ix as f32 + 0.5) / nc as f32;
                                                let v = 1.0 - (iy as f32 + 0.5) / nr as f32;
                                                let c = paint::hsv_to_rgb(hue, sa, v);
                                                window.paint_quad(quad(
                                                    Bounds {
                                                        origin: point(
                                                            px(sv.x + ix as f32 * cw),
                                                            px(sv.y + iy as f32 * chh),
                                                        ),
                                                        size: size(px(cw + 0.7), px(chh + 0.7)),
                                                    },
                                                    px(0.0),
                                                    rgb(c),
                                                    px(0.0),
                                                    hsla(0.0, 0.0, 0.0, 0.0),
                                                    Default::default(),
                                                ));
                                            }
                                        }
                                        // SV 旋钮
                                        let kx = sv.x + sat * sv.w;
                                        let ky = sv.y + (1.0 - val) * sv.h;
                                        let mut kb = PathBuilder::fill();
                                        let st = 24;
                                        for i in 0..=st {
                                            let a = i as f32 / st as f32 * std::f32::consts::TAU;
                                            let pt = point(
                                                px(kx + 5.0 * a.cos()),
                                                px(ky + 5.0 * a.sin()),
                                            );
                                            if i == 0 {
                                                kb.move_to(pt);
                                            } else {
                                                kb.line_to(pt);
                                            }
                                        }
                                        kb.close();
                                        if let Ok(path) = kb.build() {
                                            window.paint_path(path, hsla(0.0, 0.0, 1.0, 1.0));
                                        }
                                        let mut ko = PathBuilder::stroke(px(1.4));
                                        ko.move_to(point(px(kx - 5.0), px(ky)));
                                        ko.curve_to(
                                            point(px(kx), px(ky - 5.0)),
                                            point(px(kx - 5.0), px(ky - 5.0)),
                                        );
                                        ko.curve_to(
                                            point(px(kx + 5.0), px(ky)),
                                            point(px(kx + 5.0), px(ky - 5.0)),
                                        );
                                        ko.curve_to(
                                            point(px(kx), px(ky + 5.0)),
                                            point(px(kx + 5.0), px(ky + 5.0)),
                                        );
                                        ko.curve_to(
                                            point(px(kx - 5.0), px(ky)),
                                            point(px(kx - 5.0), px(ky + 5.0)),
                                        );
                                        ko.close();
                                        if let Ok(path) = ko.build() {
                                            window.paint_path(path, hsla(0.0, 0.0, 0.25, 1.0));
                                        }

                                        // 色相条：24 段
                                        let seg = huer.w / 24.0;
                                        for i in 0..24 {
                                            let h0 = i as f32 / 24.0 * 360.0;
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(
                                                        px(huer.x + i as f32 * seg),
                                                        px(huer.y),
                                                    ),
                                                    size: size(px(seg + 0.7), px(huer.h)),
                                                },
                                                px(0.0),
                                                rgb(paint::hsv_to_rgb(h0, 1.0, 1.0)),
                                                px(0.0),
                                                hsla(0.0, 0.0, 0.0, 0.0),
                                                Default::default(),
                                            ));
                                        }
                                        // 色相指示三角
                                        let hx = huer.x + hue / 360.0 * huer.w;
                                        let mut tri = PathBuilder::fill();
                                        tri.move_to(point(px(hx - 4.0), px(huer.y + huer.h + 1.0)));
                                        tri.line_to(point(px(hx + 4.0), px(huer.y + huer.h + 1.0)));
                                        tri.line_to(point(px(hx), px(huer.y + huer.h + 6.0)));
                                        tri.close();
                                        if let Ok(path) = tri.build() {
                                            window.paint_path(path, hsla(0.0, 0.0, 0.35, 1.0));
                                        }

                                        // hex 行：色块 + #RRGGBB
                                        window.paint_quad(quad(
                                            Bounds {
                                                origin: point(px(hexr.x), px(hexr.y + 2.0)),
                                                size: size(px(14.0), px(14.0)),
                                            },
                                            px(3.0),
                                            rgb(color),
                                            px(1.0),
                                            hsla(0.0, 0.0, 0.0, 0.25),
                                            Default::default(),
                                        ));
                                        let hextxt: SharedString =
                                            format!("#{:06X}", color & 0xFFFFFF).into();
                                        let hrun = TextRun {
                                            len: hextxt.len(),
                                            font: Font {
                                                family: "Consolas".into(),
                                                features: FontFeatures::default(),
                                                fallbacks: None,
                                                weight: FontWeight(400.0),
                                                style: FontStyle::Normal,
                                            },
                                            color: rgb(0x6B7280).into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        if let Some(line) = window
                                            .text_system()
                                            .shape_text(hextxt, px(11.0), &[hrun], None, None)
                                            .ok()
                                            .and_then(|l| l.into_iter().next())
                                        {
                                            let lh = px(14.0);
                                            let _ = line.paint(
                                                point(px(hexr.x + 20.0), px(hexr.y + 2.0)),
                                                lh,
                                                TextAlign::Left,
                                                None,
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                    for (is_brush, r) in modes {
                                        let selected = is_brush == mosaic_brush;
                                        let hovered = r.contains(cursor.0, cursor.1);
                                        if hovered || selected {
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(px(r.x), px(r.y)),
                                                    size: size(px(r.w), px(r.h)),
                                                },
                                                px(5.0),
                                                if selected {
                                                    hsla(0.6, 1.0, 0.95, 1.0)
                                                } else {
                                                    hsla(0.0, 0.0, 0.0, 0.06)
                                                },
                                                px(if selected { 1.2 } else { 0.0 }),
                                                if selected {
                                                    accent
                                                } else {
                                                    hsla(0.0, 0.0, 0.0, 0.0)
                                                },
                                                Default::default(),
                                            ));
                                        }
                                        let label: SharedString = if is_brush {
                                            "涂抹".into()
                                        } else {
                                            "框选".into()
                                        };
                                        let run = TextRun {
                                            len: label.len(),
                                            font: Font {
                                                family: "Microsoft YaHei".into(),
                                                features: FontFeatures::default(),
                                                fallbacks: None,
                                                weight: FontWeight(400.0),
                                                style: FontStyle::Normal,
                                            },
                                            color: if selected {
                                                rgb(0x0A84FF).into()
                                            } else {
                                                rgb(0x3A3F45).into()
                                            },
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        if let Some(line) = window
                                            .text_system()
                                            .shape_text(label, px(12.0), &[run], None, None)
                                            .ok()
                                            .and_then(|l| l.into_iter().next())
                                        {
                                            let lh = px(16.0);
                                            let w = f32::from(line.size(lh).width);
                                            let _ = line.paint(
                                                point(
                                                    px(r.x + r.w / 2.0 - w / 2.0),
                                                    px(r.y + r.h / 2.0 - f32::from(lh) / 2.0),
                                                ),
                                                lh,
                                                TextAlign::Left,
                                                None,
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                    for (sl, track) in sliders {
                                        let cur = match sl {
                                            Slider::Width => stroke,
                                            Slider::Block => block,
                                            Slider::Brush => brush_radius,
                                            Slider::TextSize => text_size,
                                        };
                                        let (vmin, vmax) = sl.range();
                                        let t = ((cur - vmin) / (vmax - vmin)).clamp(0.0, 1.0);
                                        let cy = track.y + track.h / 2.0;

                                        // 标签
                                        let label: SharedString = sl.label().into();
                                        let lrun = TextRun {
                                            len: label.len(),
                                            font: Font {
                                                family: "Microsoft YaHei".into(),
                                                features: FontFeatures::default(),
                                                fallbacks: None,
                                                weight: FontWeight(400.0),
                                                style: FontStyle::Normal,
                                            },
                                            color: rgb(0x6B7280).into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        if let Some(line) = window
                                            .text_system()
                                            .shape_text(label, px(11.0), &[lrun], None, None)
                                            .ok()
                                            .and_then(|l| l.into_iter().next())
                                        {
                                            let lh = px(14.0);
                                            let lw = f32::from(line.size(lh).width);
                                            let _ = line.paint(
                                                point(
                                                    px(track.x - 6.0 - lw),
                                                    px(cy - f32::from(lh) / 2.0),
                                                ),
                                                lh,
                                                TextAlign::Left,
                                                None,
                                                window,
                                                cx,
                                            );
                                        }

                                        // 轨道：底 + 已填充
                                        let tk_h = 4.0;
                                        window.paint_quad(quad(
                                            Bounds {
                                                origin: point(px(track.x), px(cy - tk_h / 2.0)),
                                                size: size(px(track.w), px(tk_h)),
                                            },
                                            px(2.0),
                                            hsla(0.0, 0.0, 0.0, 0.14),
                                            px(0.0),
                                            hsla(0.0, 0.0, 0.0, 0.0),
                                            Default::default(),
                                        ));
                                        window.paint_quad(quad(
                                            Bounds {
                                                origin: point(px(track.x), px(cy - tk_h / 2.0)),
                                                size: size(px(track.w * t), px(tk_h)),
                                            },
                                            px(2.0),
                                            accent,
                                            px(0.0),
                                            hsla(0.0, 0.0, 0.0, 0.0),
                                            Default::default(),
                                        ));

                                        // 旋钮
                                        let kx = track.x + track.w * t;
                                        let kr = if menu_drag == Some(MenuDrag::Slider(sl)) {
                                            6.5
                                        } else {
                                            5.5
                                        };
                                        let mut kb = PathBuilder::fill();
                                        let steps_c = 24;
                                        for i in 0..=steps_c {
                                            let a =
                                                i as f32 / steps_c as f32 * std::f32::consts::TAU;
                                            let pt =
                                                point(px(kx + kr * a.cos()), px(cy + kr * a.sin()));
                                            if i == 0 {
                                                kb.move_to(pt);
                                            } else {
                                                kb.line_to(pt);
                                            }
                                        }
                                        kb.close();
                                        if let Ok(path) = kb.build() {
                                            window.paint_path(path, hsla(0.0, 0.0, 1.0, 1.0));
                                        }
                                        let mut ko = PathBuilder::stroke(px(1.5));
                                        ko.move_to(point(px(kx - kr), px(cy)));
                                        ko.curve_to(
                                            point(px(kx), px(cy - kr)),
                                            point(px(kx - kr), px(cy - kr)),
                                        );
                                        ko.curve_to(
                                            point(px(kx + kr), px(cy)),
                                            point(px(kx + kr), px(cy - kr)),
                                        );
                                        ko.curve_to(
                                            point(px(kx), px(cy + kr)),
                                            point(px(kx + kr), px(cy + kr)),
                                        );
                                        ko.curve_to(
                                            point(px(kx - kr), px(cy)),
                                            point(px(kx - kr), px(cy + kr)),
                                        );
                                        ko.close();
                                        if let Ok(path) = ko.build() {
                                            window.paint_path(path, accent);
                                        }

                                        // 数值
                                        let valtxt: SharedString = format!("{cur:.1}").into();
                                        let vrun = TextRun {
                                            len: valtxt.len(),
                                            font: Font {
                                                family: "Microsoft YaHei".into(),
                                                features: FontFeatures::default(),
                                                fallbacks: None,
                                                weight: FontWeight(500.0),
                                                style: FontStyle::Normal,
                                            },
                                            color: rgb(0x2A2F36).into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        if let Some(line) = window
                                            .text_system()
                                            .shape_text(valtxt, px(11.0), &[vrun], None, None)
                                            .ok()
                                            .and_then(|l| l.into_iter().next())
                                        {
                                            let lh = px(14.0);
                                            let lw = f32::from(line.size(lh).width);
                                            let _ = line.paint(
                                                point(
                                                    px(track.x + track.w + 8.0),
                                                    px(cy - f32::from(lh) / 2.0),
                                                ),
                                                lh,
                                                TextAlign::Left,
                                                None,
                                                window,
                                                cx,
                                            );
                                            let _ = lw;
                                        }
                                    }
                                }

                                // ---- 笔刷 / 橡皮预览圈 ----
                                let brush_r = if erasing {
                                    Some(8.0)
                                } else if tool == Tool::Eraser && !drawing && !dragging {
                                    Some(8.0)
                                } else if tool == Tool::Mosaic
                                    && mosaic_brush
                                    && !drawing
                                    && !dragging
                                {
                                    Some(brush_radius)
                                } else {
                                    None
                                };
                                if let Some(r) = brush_r {
                                    if erasing
                                        || sel.map_or(false, |sl| sl.contains(cursor.0, cursor.1))
                                    {
                                        let mut b = PathBuilder::stroke(px(1.2));
                                        let steps = 36;
                                        for i in 0..=steps {
                                            let t = i as f32 / steps as f32 * std::f32::consts::TAU;
                                            let pt = point(
                                                px(cursor.0 + r * t.cos()),
                                                px(cursor.1 + r * t.sin()),
                                            );
                                            if i == 0 {
                                                b.move_to(pt);
                                            } else {
                                                b.line_to(pt);
                                            }
                                        }
                                        b.close();
                                        if let Ok(path) = b.build() {
                                            window.paint_path(path, hsla(0.0, 0.0, 1.0, 0.9));
                                        }
                                    }
                                }

                                // ---- 悬停提示牌（按钮名） ----
                                if let Some((bi, br)) = tooltip_btn.and_then(|hb| {
                                    tb_btns
                                        .iter()
                                        .find(|(i, _)| *i == hb)
                                        .map(|(i, r)| (*i, *r))
                                }) {
                                    if let (Some(tb), Some(sel)) = (tb_rect, sel) {
                                        let name: SharedString = BUTTONS[bi].1.into();
                                        let run = TextRun {
                                            len: name.len(),
                                            font: Font {
                                                family: "Microsoft YaHei".into(),
                                                features: FontFeatures::default(),
                                                fallbacks: None,
                                                weight: FontWeight(400.0),
                                                style: FontStyle::Normal,
                                            },
                                            color: rgb(0xFFFFFF).into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        if let Some(line) = window
                                            .text_system()
                                            .shape_text(name, px(11.5), &[run], None, None)
                                            .ok()
                                            .and_then(|l| l.into_iter().next())
                                        {
                                            let lh = px(15.0);
                                            let tw = f32::from(line.size(lh).width) + 14.0;
                                            let th = 20.0;
                                            // 工具栏在选区下方 → 提示牌放工具栏下面；反之上面
                                            let below = tb.y > sel.y;
                                            let ty = if below {
                                                (tb.y + tb.h + 5.0).min(wh - th - 4.0)
                                            } else {
                                                (tb.y - th - 5.0).max(4.0)
                                            };
                                            let tx = (br.x + br.w / 2.0 - tw / 2.0)
                                                .clamp(4.0, (ww - tw - 4.0).max(4.0));
                                            window.paint_quad(quad(
                                                Bounds {
                                                    origin: point(px(tx), px(ty)),
                                                    size: size(px(tw), px(th)),
                                                },
                                                px(5.0),
                                                hsla(0.0, 0.0, 0.12, 0.92),
                                                px(0.0),
                                                hsla(0.0, 0.0, 0.0, 0.0),
                                                Default::default(),
                                            ));
                                            let _ = line.paint(
                                                point(
                                                    px(tx + 7.0),
                                                    px(ty + (th - f32::from(lh)) / 2.0),
                                                ),
                                                lh,
                                                TextAlign::Left,
                                                None,
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                }

                                // ---- 放大镜 ----
                                if show_loupe {
                                    let lx = loupe_x;
                                    let ly = loupe_y;
                                    let cell = LOUPE / LOUPE_CELLS as f32;
                                    let half = LOUPE_CELLS / 2;
                                    // 先铺底色，避免边界外取样留空
                                    fill_rect(
                                        window,
                                        lx,
                                        ly,
                                        LOUPE,
                                        LOUPE,
                                        hsla(0.0, 0.0, 0.1, 1.0),
                                    );
                                    for j in 0..LOUPE_CELLS {
                                        for i in 0..LOUPE_CELLS {
                                            let sx = cx_px + i - half;
                                            let sy = cy_px + j - half;
                                            if let Some((r, g, b)) = sample(&shot, sx, sy) {
                                                fill_rect(
                                                    window,
                                                    lx + i as f32 * cell,
                                                    ly + j as f32 * cell,
                                                    cell,
                                                    cell,
                                                    rgb(((r as u32) << 16)
                                                        | ((g as u32) << 8)
                                                        | b as u32),
                                                );
                                            }
                                        }
                                    }
                                    // 像素网格：中灰半透明，浅底和深底上都看得见
                                    let grid = hsla(0.0, 0.0, 0.5, 0.32);
                                    for i in 1..LOUPE_CELLS {
                                        let gx = lx + i as f32 * cell;
                                        fill_rect(window, gx, ly, 1.0, LOUPE, grid);
                                        let gy = ly + i as f32 * cell;
                                        fill_rect(window, lx, gy, LOUPE, 1.0, grid);
                                    }
                                    // 中心像素：先黑后白，任何底色上都看得见
                                    let ch = half as f32 * cell;
                                    let center = RectF::new(lx + ch, ly + ch, cell, cell);
                                    stroke_rect_outline(
                                        window,
                                        RectF::new(
                                            center.x - 1.0,
                                            center.y - 1.0,
                                            center.w + 2.0,
                                            center.h + 2.0,
                                        ),
                                        hsla(0.0, 0.0, 0.0, 0.75),
                                        2.0,
                                    );
                                    stroke_rect_outline(window, center, white, 1.5);
                                    // 外框
                                    stroke_rect_outline(
                                        window,
                                        RectF::new(lx - 0.5, ly - 0.5, LOUPE + 1.0, LOUPE + 1.0),
                                        hsla(0.0, 0.0, 0.0, 0.6),
                                        1.0,
                                    );
                                }
                            },
                        )
                        .size_full(),
                    ),
            );

        // ---- 尺寸徽标 ----
        // 工具栏翻到选区上方时，徽标会跟它打架 —— 这种情况把徽标收进选区里面。
        let tb_above = match (tb_origin, sel) {
            (Some((_, ty)), Some(s)) => ty < s.y,
            _ => false,
        };
        let badge_inside = tb_above || sel.map(|s| s.y < 40.0).unwrap_or(false);
        if let Some(s) = sel {
            let gx = self.global_px(s.x);
            let gy = self.global_px(s.y);
            let pw = (s.w * scale).round() as i32;
            let ph = (s.h * scale).round() as i32;
            let (bx, by) = if badge_inside {
                (s.x + 8.0, s.y + 8.0)
            } else {
                (s.x, s.y - 34.0)
            };
            root = root.child(
                div()
                    .absolute()
                    .left(px(bx))
                    .top(px(by))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(9.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(hsla(0.0, 0.0, 0.09, 0.92))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(hsla(0.0, 0.0, 0.65, 1.0))
                            .child(format!("{gx},{gy}")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .gap(px(3.0))
                            .text_size(px(12.5))
                            .text_color(rgb(0xFFFFFF))
                            .child(format!("{pw} × {ph}"))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(hsla(0.0, 0.0, 0.6, 1.0))
                                    .child("px"),
                            ),
                    ),
            );
        }

        // ---- 放大镜读数面板 ----
        if show_loupe {
            root = root.child(
                div()
                    .absolute()
                    .left(px(loupe_x + LOUPE + 8.0))
                    .top(px(loupe_y))
                    .w(px(PANEL_W))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .px(px(9.0))
                    .py(px(7.0))
                    .rounded(px(7.0))
                    .bg(hsla(0.0, 0.0, 0.09, 0.86))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(0.0, 0.0, 0.72, 1.0))
                            .child(format!(
                                "{},{}",
                                self.global_px(cursor.0),
                                self.global_px(cursor.1)
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().w(px(13.0)).h(px(13.0)).rounded(px(3.0)).bg(rgb(
                                ((cursor_rgb.0 as u32) << 16)
                                    | ((cursor_rgb.1 as u32) << 8)
                                    | cursor_rgb.2 as u32,
                            )))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(0xFFFFFF))
                                    .child(hex),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(hsla(0.0, 0.0, 0.55, 1.0))
                            .child(rgb_txt),
                    ),
            );
        }

        // ---- 快捷键提示（只在还没框选时显示）----
        if sel.is_none() && status.is_none() {
            let rows: [(&str, &str); 6] = [
                ("拖动", "框选区域"),
                ("单击", "选中窗口"),
                ("Enter", "复制到剪贴板"),
                ("Ctrl+S", "保存 PNG"),
                ("Ctrl+Z", "撤销标注"),
                ("Esc", "取消"),
            ];
            let mut panel = div()
                .absolute()
                .left(px(16.0))
                .bottom(px(16.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .px(px(10.0))
                .py(px(9.0))
                .rounded(px(8.0))
                .bg(hsla(0.0, 0.0, 0.08, 0.82));
            for (k, v) in rows {
                panel = panel.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(9.0))
                        .child(
                            div().w(px(52.0)).flex().flex_row().child(
                                div()
                                    .px(px(5.0))
                                    .py(px(1.5))
                                    .rounded(px(3.0))
                                    .bg(hsla(0.0, 0.0, 1.0, 0.14))
                                    .text_size(px(10.0))
                                    .text_color(hsla(0.0, 0.0, 0.98, 1.0))
                                    .child(k),
                            ),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(hsla(0.0, 0.0, 0.62, 1.0))
                                .child(v),
                        ),
                );
            }
            root = root.child(panel);
        }

        // ---- 状态 / 操作提示 ----
        let hint = match &status {
            Some(s) => Some(s.clone()),
            None => {
                if sel.is_some() && tb_origin.is_none() {
                    Some("选区太小".to_string())
                } else {
                    None
                }
            }
        };
        if let Some(hint) = hint {
            root = root.child(
                div()
                    .absolute()
                    .left(px((win_w - 320.0) / 2.0))
                    .top(px(14.0))
                    .w(px(320.0))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(10.0))
                            .py(px(5.0))
                            .rounded(px(6.0))
                            .bg(hsla(0.0, 0.0, 0.0, 0.62))
                            .text_color(rgb(0xFFFFFF))
                            .text_size(px(11.0))
                            .child(hint),
                    ),
            );
        }

        // ---- 按钮命中区不再单独建元素：见 on_down 里的 hit_button ----

        root
    }
}
