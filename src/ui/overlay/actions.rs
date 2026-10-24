use super::*;

impl Overlay {
    /// Rasterize the selection plus annotations into an RGBA image.
    pub(super) fn composite(&self) -> Option<image::RgbaImage> {
        let sel = self.sel?;
        let (win_w, win_h) = (self.shot.logical_w(), self.shot.logical_h());
        let sel = sel.clamped_rounded(win_w, win_h);
        if sel.is_empty() {
            return None;
        }
        let s = self.scale;
        let x = (sel.x * s).round().max(0.0) as u32;
        let y = (sel.y * s).round().max(0.0) as u32;
        let w = ((sel.w * s).round().max(1.0) as u32).min(self.shot.width.saturating_sub(x));
        let h = ((sel.h * s).round().max(1.0) as u32).min(self.shot.height.saturating_sub(y));
        if w == 0 || h == 0 {
            return None;
        }

        let mut full =
            image::RgbaImage::from_raw(self.shot.width, self.shot.height, self.shot.to_rgba())?;
        let mut crop = image::imageops::crop(&mut full, x, y, w, h).to_image();
        crate::core::raster::rasterize(&mut crop, &self.shapes, s, (x as f32, y as f32));
        Some(crop)
    }

    pub(super) fn copy_to_clipboard(&mut self, window: &mut Window) {
        self.commit_editing(window);
        let Some(img) = self.composite() else {
            return;
        };
        let ok = match arboard::Clipboard::new() {
            Ok(mut clipboard) => clipboard
                .set_image(arboard::ImageData {
                    width: img.width() as usize,
                    height: img.height() as usize,
                    bytes: std::borrow::Cow::Owned(img.into_raw()),
                })
                .is_ok(),
            Err(_) => false,
        };
        if ok {
            crate::platform::winutil::drop_edit_pin();
            self.close(window);
        } else {
            self.status = Some("复制失败".into());
        }
    }

    pub(super) fn save_to_file(&mut self, window: &mut Window) {
        self.commit_editing(window);
        let Some(img) = self.composite() else {
            return;
        };
        let dir = crate::services::paths::save_dir();
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("PixPinRS_{stamp}.png"));
        match img.save(&path) {
            Ok(()) => {
                eprintln!("[pixpin-rs] saved -> {}", path.display());
                crate::platform::winutil::drop_edit_pin();
                self.close(window)
            }
            Err(e) => {
                eprintln!("[pixpin-rs] save failed: {e}");
                self.status = Some(format!("保存失败: {e}"));
            }
        }
    }

    pub(super) fn pin_to_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_editing(window);
        let Some(sel) = self.sel else {
            return;
        };
        let Some(img) = self.composite() else {
            return;
        };
        let dpi = self.scale;
        let origin = window.bounds().origin;
        let (sx, sy) = (f32::from(origin.x) + sel.x, f32::from(origin.y) + sel.y);

        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(sx), px(sy)),
                size: size(px(sel.w), px(sel.h)),
            })),
            titlebar: None,
            // 贴图窗口要拿键盘焦点：Windows 只把滚轮事件发给焦点窗口，
            // 不 focus 的话「滚轮缩放」在别的窗口抢着焦点时就是死的。
            focus: true,
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
        let _ = cx.open_window(opts, move |window, cx| {
            crate::platform::winutil::make_topmost(window);
            crate::platform::winutil::set_app_icon(window);
            cx.new(|_| crate::ui::pin::PinView::new(img, dpi))
        });
        crate::platform::winutil::drop_edit_pin();
        self.close(window);
    }

    pub(super) fn close(&mut self, window: &mut Window) {
        window.remove_window();
        crate::platform::winutil::restore_after_capture();
        // 二次编辑被取消时，把之前藏起来的贴图放回屏幕。
        crate::platform::winutil::restore_edit_pin();
    }

    /// 提取选区里的文字：后台跑 OCR，完成后写剪贴板并弹结果窗口。
    ///
    /// 识别要 200ms 左右，直接在主线程做会把覆盖层的重绘卡住，
    /// 所以扔到 background executor，再用 `update_in` 回到 UI 线程。
    pub(super) fn extract_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(img) = self.composite() else {
            self.status = Some("先框选一块区域".into());
            cx.notify();
            return;
        };
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();

        self.status = Some("识别中…".into());
        cx.notify();

        let task = cx
            .background_executor()
            .spawn(async move { crate::services::ocr::run(rgb) });

        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
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
                    eprintln!("[pixpin-rs] ocr ok: {chars} chars, copied={copied}");
                    // 先把覆盖层收掉、主窗口复位，再开结果窗口，
                    // 否则结果窗口会被 restore 时的前台切换压到后面去。
                    this.close(window);
                    crate::ui::text_result::TextView::open(text, auto_copy, cx);
                }
                Ok(_) => {
                    eprintln!("[pixpin-rs] ocr returned no text");
                    this.status = Some("没识别到文字".into());
                    cx.notify();
                }
                Err(e) => {
                    eprintln!("[pixpin-rs] ocr failed: {e}");
                    this.status = Some(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 任何改动 shapes 的操作前先调用：把当前状态压进撤销栈。
    pub(super) fn record(&mut self) {
        self.undo_stack.push(self.shapes.clone());
        self.redo_stack.clear();
    }

    pub(super) fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(std::mem::take(&mut self.shapes));
            self.shapes = prev;
        }
    }

    pub(super) fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(std::mem::take(&mut self.shapes));
            self.shapes = next;
        }
    }

    /// 橡皮擦：删掉光标下最上层的命中形状（可撤销）。
    pub(super) fn erase_at(&mut self, x: f32, y: f32) {
        if let Some(idx) = self
            .shapes
            .iter()
            .rposition(|s| paint::shape_hit(s, x, y, 8.0))
        {
            self.record();
            self.shapes.remove(idx);
        }
    }

    pub(super) fn on_toolbar(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        eprintln!("[pixpin-rs] toolbar button {index}");
        self.commit_editing(window);
        match index {
            0..=6 => {
                if let Some(t) = Tool::from_button(index) {
                    self.tool = if self.tool == t { Tool::None } else { t };
                    self.erasing = false;
                }
            }
            7 => self.undo(),
            8 => self.redo(),
            9 => self.extract_text(window, cx),
            10 => self.pin_to_screen(window, cx),
            11 => self.save_to_file(window),
            12 => self.copy_to_clipboard(window),
            _ => self.close(window),
        }
        cx.notify();
    }
}
