use super::*;

pub(super) fn handle_points(r: RectF) -> [(Handle, f32, f32); 8] {
    [
        (Handle::TL, r.x, r.y),
        (Handle::T, r.x + r.w / 2.0, r.y),
        (Handle::TR, r.right(), r.y),
        (Handle::R, r.right(), r.y + r.h / 2.0),
        (Handle::BR, r.right(), r.bottom()),
        (Handle::B, r.x + r.w / 2.0, r.bottom()),
        (Handle::BL, r.x, r.bottom()),
        (Handle::L, r.x, r.y + r.h / 2.0),
    ]
}

fn hit_handle(r: RectF, x: f32, y: f32) -> Option<Handle> {
    for (h, hx, hy) in handle_points(r) {
        if (x - hx).abs() <= HANDLE_TOL && (y - hy).abs() <= HANDLE_TOL {
            return Some(h);
        }
    }
    None
}

fn apply_resize(orig: RectF, h: Handle, x: f32, y: f32) -> RectF {
    let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
    match h {
        Handle::TL => {
            x0 = x;
            y0 = y;
        }
        Handle::T => y0 = y,
        Handle::TR => {
            x1 = x;
            y0 = y;
        }
        Handle::R => x1 = x,
        Handle::BR => {
            x1 = x;
            y1 = y;
        }
        Handle::B => y1 = y,
        Handle::BL => {
            x0 = x;
            y1 = y;
        }
        Handle::L => x0 = x,
    }
    RectF::from_corners(x0, y0, x1, y1)
}

impl Overlay {
    /// 光标下的窗口探测，按移动距离节流 —— 走 z 序不算便宜。
    pub(super) fn refresh_hover_win(&mut self) {
        if self.sel.is_some() || !matches!(self.drag, Drag::None) {
            self.hover_win = None;
            return;
        }
        let (x, y) = self.cursor;
        let (px0, py0) = self.probe_at;
        if (x - px0).abs() < 4.0 && (y - py0).abs() < 4.0 {
            return;
        }
        self.probe_at = (x, y);

        let sx = (self.shot.origin_x as f32 + x * self.scale).round() as i32;
        let sy = (self.shot.origin_y as f32 + y * self.scale).round() as i32;
        self.hover_win = winpick::window_at(sx, sy).map(|r: WinRect| {
            RectF::new(
                r.x as f32 / self.scale - self.shot.logical_x(),
                r.y as f32 / self.scale - self.shot.logical_y(),
                r.w as f32 / self.scale,
                r.h as f32 / self.scale,
            )
        });
    }

    pub(super) fn on_move(
        &mut self,
        pos: Point<Pixels>,
        pressed: Option<MouseButton>,
        window: &Window,
    ) {
        self.cursor = (f32::from(pos.x), f32::from(pos.y));

        // 菜单控件拖动中：滑杆 / 色相条 / 取色面板跟手。
        if self.menu_drag.is_some() && self.menu_btn.is_some() {
            let (ww2, wh2) = Self::win_size(window);
            if let Some((_, picker, _, sliders)) = self.menu_layout(ww2, wh2) {
                match self.menu_drag {
                    Some(MenuDrag::Slider(want)) => {
                        for (sl, track) in sliders {
                            if sl == want {
                                let v = self.slider_value(sl, track, self.cursor.0);
                                self.set_slider(sl, v);
                                return;
                            }
                        }
                    }
                    Some(MenuDrag::Hue) => {
                        if let Some((_, hue, _)) = picker {
                            let h =
                                ((self.cursor.0 - hue.x) / hue.w.max(1.0)).clamp(0.0, 1.0) * 360.0;
                            self.set_hsv(h, self.sat, self.val);
                            return;
                        }
                    }
                    Some(MenuDrag::SatVal) => {
                        if let Some((sv, _, _)) = picker {
                            let (sa, v) = (
                                ((self.cursor.0 - sv.x) / sv.w.max(1.0)).clamp(0.0, 1.0),
                                1.0 - ((self.cursor.1 - sv.y) / sv.h.max(1.0)).clamp(0.0, 1.0),
                            );
                            self.set_hsv(self.hue, sa, v);
                            return;
                        }
                    }
                    None => {}
                }
            }
        }

        // 右键拖拽：位移超过阈值即进入擦除（任何工具下可用）。
        if pressed == Some(MouseButton::Right) {
            if let Some((px0, py0)) = self.right_press {
                let (x, y) = self.cursor;
                if !self.right_erasing && ((x - px0).abs() > 4.0 || (y - py0).abs() > 4.0) {
                    self.right_erasing = true;
                    self.erasing = true;
                }
                if self.right_erasing {
                    self.erase_at(x, y);
                    return;
                }
            }
        }
        let (ww, wh) = Self::win_size(window);
        self.hover_btn = self.hit_button(ww, wh, self.cursor.0, self.cursor.1);

        let (x, y) = self.cursor;
        match self.drag {
            Drag::New => {
                let (px0, py0) = self.press;
                if (x - px0).abs() > 2.0 || (y - py0).abs() > 2.0 {
                    self.sel = Some(RectF::from_corners(
                        px0.clamp(0.0, ww),
                        py0.clamp(0.0, wh),
                        x.clamp(0.0, ww),
                        y.clamp(0.0, wh),
                    ));
                }
            }
            Drag::Move { grab, orig } => {
                let nx = (orig.x + (x - grab.0)).clamp(0.0, (ww - orig.w).max(0.0));
                let ny = (orig.y + (y - grab.1)).clamp(0.0, (wh - orig.h).max(0.0));
                self.sel = Some(RectF::new(nx, ny, orig.w, orig.h));
            }
            Drag::Resize { h, orig } => {
                let r = apply_resize(orig, h, x.clamp(0.0, ww), y.clamp(0.0, wh));
                if r.w >= 2.0 && r.h >= 2.0 {
                    self.sel = Some(r);
                }
            }
            Drag::None => {}
        }

        if self.erasing {
            self.erase_at(x, y);
            return;
        }

        if self.drawing {
            if self.tool == Tool::Pen {
                // 事件点之间可能隔几十像素：先按 3px 步长补齐，再交给 Chaikin。
                // 补点不改变轨迹，但能让平滑只削掉真正的转角，不会把整段拉弯。
                let from = self.pen.last().copied().unwrap_or(self.cursor);
                let (dx, dy) = (self.cursor.0 - from.0, self.cursor.1 - from.1);
                let dist = (dx * dx + dy * dy).sqrt();
                let steps = (dist / 3.0).floor() as usize;
                for i in 1..=steps {
                    let t = i as f32 * 3.0 / dist;
                    self.pen.push((from.0 + dx * t, from.1 + dy * t));
                }
                self.pen.push(self.cursor);
            } else if let Some(shape) = self.draft.as_mut() {
                match shape {
                    Shape::Rect { b, .. }
                    | Shape::Ellipse { b, .. }
                    | Shape::Arrow { b, .. }
                    | Shape::Mosaic { b, .. } => {
                        *b = self.cursor;
                    }
                    Shape::MosaicStroke { pts, .. } => {
                        pts.push(self.cursor);
                    }
                    _ => {}
                }
            }
        }

        self.refresh_hover_win();
    }

    pub(super) fn on_down(
        &mut self,
        pos: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (ww, wh) = Self::win_size(window);
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        if self.editing.is_some() {
            self.commit_editing(window);
        }

        // 二级菜单开着：先看滑杆（按下进入拖动、菜单不关），
        // 再看色板 / 模式条目（点选即关），点外面只收菜单。
        if self.menu_btn.is_some() {
            if let Some((_, picker, _, sliders)) = self.menu_layout(ww, wh) {
                for (sl, track) in sliders {
                    let fat =
                        RectF::new(track.x - 6.0, track.y - 6.0, track.w + 12.0, track.h + 12.0);
                    if fat.contains(x, y) {
                        self.menu_drag = Some(MenuDrag::Slider(sl));
                        let v = self.slider_value(sl, track, x);
                        self.set_slider(sl, v);
                        return;
                    }
                }
                if let Some((sv, hue, _hex)) = picker {
                    if sv.contains(x, y) {
                        self.menu_drag = Some(MenuDrag::SatVal);
                        let (sa, v) = (
                            ((x - sv.x) / sv.w.max(1.0)).clamp(0.0, 1.0),
                            1.0 - ((y - sv.y) / sv.h.max(1.0)).clamp(0.0, 1.0),
                        );
                        self.set_hsv(self.hue, sa, v);
                        return;
                    }
                    if hue.contains(x, y) {
                        self.menu_drag = Some(MenuDrag::Hue);
                        let h = ((x - hue.x) / hue.w.max(1.0)).clamp(0.0, 1.0) * 360.0;
                        self.set_hsv(h, self.sat, self.val);
                        return;
                    }
                }
            }
            if !self.on_menu_click(ww, wh, x, y) {
                self.menu_btn = None;
            }
            cx.notify();
            return;
        }

        // 工具栏是画在 canvas 上的，命中判断也在这里做 —— 不给按钮挂独立的
        // 事件元素，否则「子元素先关窗、父元素再处理」会踩到已移除的窗口。
        if let Some((i, r)) = self
            .tb_buttons(ww, wh)
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
        {
            // 点在 ⌄ 区：开/关该工具的二级菜单，不切换工具。
            if MENU_BTNS.contains(&i) && x >= r.x + r.w - 13.0 {
                self.menu_btn = if self.menu_btn == Some(i) {
                    None
                } else {
                    Some(i)
                };
                cx.notify();
                return;
            }
            self.on_toolbar(i, window, cx);
            return;
        }

        if let Some(sel) = self.sel {
            if let Some(h) = hit_handle(sel, x, y) {
                self.drag = Drag::Resize { h, orig: sel };
                return;
            }
            if sel.contains(x, y) {
                if self.tool != Tool::None {
                    match self.tool {
                        // 文本：单击定位输入点，进入键入状态（Enter 提交）。
                        Tool::Text => {
                            self.commit_editing(window);
                            self.editing = Some(((x, y), String::new()));
                        }
                        // 橡皮擦：按住拖动连续擦除。
                        Tool::Eraser => {
                            self.erasing = true;
                            self.erase_at(x, y);
                        }
                        _ => self.start_draw(x, y),
                    }
                    cx.notify();
                    return;
                }
                self.drag = Drag::Move {
                    grab: (x, y),
                    orig: sel,
                };
                return;
            }
        }

        // 空白处：重新框选。
        self.press_win = self.hover_win;
        self.sel = None;
        self.record();
        self.shapes.clear();
        self.menu_btn = None;
        self.draft = None;
        self.drag = Drag::New;
        self.press = (x, y);
    }

    pub(super) fn start_draw(&mut self, x: f32, y: f32) {
        self.drawing = true;
        let (color, stroke) = (self.color, self.stroke);
        self.draft = Some(match self.tool {
            Tool::Ellipse => Shape::Ellipse {
                a: (x, y),
                b: (x, y),
                color,
                width: stroke,
            },
            Tool::Arrow => Shape::Arrow {
                a: (x, y),
                b: (x, y),
                color,
                width: stroke,
            },
            Tool::Pen => Shape::Pen {
                pts: vec![(x, y)],
                color,
                width: stroke,
            },
            Tool::Mosaic if self.mosaic_brush => Shape::MosaicStroke {
                pts: vec![(x, y)],
                radius: self.brush_radius,
                block: self.block,
            },
            Tool::Mosaic => Shape::Mosaic {
                a: (x, y),
                b: (x, y),
                block: self.block,
            },
            _ => Shape::Rect {
                a: (x, y),
                b: (x, y),
                color,
                width: stroke,
            },
        });
        self.pen = vec![(x, y)];
    }

    pub(super) fn on_up(&mut self, pos: Point<Pixels>, window: &Window) {
        let _ = pos;
        let _ = window;

        if self.menu_drag.is_some() {
            self.menu_drag = None;
            return;
        }

        if self.erasing {
            self.erasing = false;
            self.drag = Drag::None;
            return;
        }

        if self.drawing {
            self.drawing = false;
            if self.tool == Tool::Pen {
                let pts = std::mem::take(&mut self.pen);
                if pts.len() > 1 {
                    self.record();
                    self.shapes.push(Shape::Pen {
                        pts,
                        color: self.color,
                        width: self.stroke,
                    });
                }
            } else if let Some(shape) = self.draft.take() {
                let meaningful = match &shape {
                    Shape::Rect { a, b, .. }
                    | Shape::Ellipse { a, b, .. }
                    | Shape::Arrow { a, b, .. }
                    | Shape::Mosaic { a, b, .. } => {
                        (a.0 - b.0).abs() > 2.0 || (a.1 - b.1).abs() > 2.0
                    }
                    Shape::MosaicStroke { pts, .. } => !pts.is_empty(),
                    _ => true,
                };
                if meaningful {
                    self.record();
                    self.shapes.push(shape);
                }
            }
            self.draft = None;
            self.drag = Drag::None;
            return;
        }

        if matches!(self.drag, Drag::New) {
            let tiny = self.sel.map(|s| s.w < 6.0 || s.h < 6.0).unwrap_or(true);
            if tiny {
                // 没拖动 = 单击：选中按下时光标下的窗口。
                self.sel = self.press_win.take();
            }
        }
        self.press_win = None;
        self.drag = Drag::None;
    }

    /// 提交正在编辑的文本（量宽后落成 Shape::Text）。
    pub(super) fn commit_editing(&mut self, window: &mut Window) {
        let Some((pos, buf)) = self.editing.take() else {
            return;
        };
        if buf.trim().is_empty() {
            return;
        }
        let (color, size) = (self.color, self.text_size);
        let w = Self::measure_text(window, &buf, size);
        self.record();
        self.shapes.push(Shape::Text {
            pos,
            text: buf,
            color,
            size,
            w,
        });
    }

    pub(super) fn measure_text(window: &mut Window, text: &str, size: f32) -> f32 {
        let run = TextRun {
            len: text.len(),
            font: Font {
                family: "Microsoft YaHei".into(),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: FontWeight(400.0),
                style: FontStyle::Normal,
            },
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_text(
                SharedString::from(text.to_string()),
                px(size),
                &[run],
                None,
                None,
            )
            .ok()
            .and_then(|l| l.into_iter().next())
            .map(|line| f32::from(line.size(px(size * 1.25)).width))
            .unwrap_or(size * text.chars().count() as f32 * 0.6)
    }

    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control;
        let shift = ev.keystroke.modifiers.shift;

        // 文本输入状态：按键先喂给输入框。GPUI 0.2.2 无 IME，
        // 英文直接键入，中文 Ctrl+V 粘贴，Enter 提交，Esc 只取消输入。
        if let Some((_, buf)) = self.editing.as_mut() {
            match key {
                "escape" => {
                    self.editing = None;
                }
                "enter" => self.commit_editing(window),
                "backspace" => {
                    buf.pop();
                }
                "v" if ctrl => {
                    if let Ok(txt) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        buf.push_str(&txt);
                    }
                }
                _ if !ctrl && !ev.keystroke.modifiers.alt && key.len() == 1 => {
                    let mut c = key.chars().next().unwrap();
                    if shift && c.is_ascii_lowercase() {
                        c = c.to_ascii_uppercase();
                    } else if !shift && c.is_ascii_uppercase() {
                        c = c.to_ascii_lowercase();
                    }
                    buf.push(c);
                }
                _ => {}
            }
            cx.notify();
            return;
        }

        match key {
            "escape" => self.close(window),
            "enter" => self.copy_to_clipboard(window),
            "z" if ctrl && shift => {
                self.redo();
                cx.notify();
            }
            "z" if ctrl => {
                self.undo();
                cx.notify();
            }
            "y" if ctrl => {
                self.redo();
                cx.notify();
            }
            "c" if ctrl => self.copy_to_clipboard(window),
            "s" if ctrl => self.save_to_file(window),
            "a" if ctrl => {
                self.sel = Some(RectF::new(
                    0.0,
                    0.0,
                    self.shot.logical_w(),
                    self.shot.logical_h(),
                ));
                cx.notify();
            }
            _ => {}
        }
    }
}
