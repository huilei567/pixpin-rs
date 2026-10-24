use super::*;

impl Overlay {
    pub(super) fn tb_size() -> (f32, f32) {
        let n_btn = BUTTONS.len() as f32;
        let n_sep = ITEMS.iter().filter(|i| matches!(i, Item::Sep)).count() as f32;
        (n_btn * BTN + n_sep * SEP_W + TB_PAD * 2.0, TB_H)
    }

    pub(super) fn tb_origin(&self, win_w: f32, win_h: f32) -> Option<(f32, f32)> {
        let sel = self.sel?;
        if sel.is_empty() {
            return None;
        }
        let (w, h) = Self::tb_size();
        // 右对齐选区的右边缘。
        let x = (sel.right() - w).clamp(4.0, (win_w - w - 4.0).max(4.0));
        let below = sel.bottom() + TB_GAP;
        let y = if below + h <= win_h - 4.0 {
            below
        } else {
            (sel.y - h - TB_GAP).max(4.0)
        };
        Some((x, y))
    }

    pub(super) fn tb_buttons(&self, win_w: f32, win_h: f32) -> Vec<(usize, RectF)> {
        let Some((ox, oy)) = self.tb_origin(win_w, win_h) else {
            return Vec::new();
        };
        let mut x = ox + TB_PAD;
        let mut out = Vec::new();
        for it in ITEMS {
            match it {
                Item::Sep => x += SEP_W,
                Item::Btn(i) => {
                    out.push((i, RectF::new(x, oy + TB_PAD, BTN, TB_H - TB_PAD * 2.0)));
                    x += BTN;
                }
            }
        }
        out
    }

    pub(super) fn hit_button(&self, win_w: f32, win_h: f32, x: f32, y: f32) -> Option<usize> {
        self.tb_buttons(win_w, win_h)
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(i, _)| i)
    }

    /// 二级菜单几何：面板 + 色块矩形 + 档位矩形。返回 None = 无菜单可显示。
    #[allow(clippy::type_complexity)]
    pub(super) fn menu_layout(
        &self,
        win_w: f32,
        win_h: f32,
    ) -> Option<(
        RectF,
        Option<(RectF, RectF, RectF)>, // (SV 面板, 色相条, hex 行)
        Vec<(bool, RectF)>,
        Vec<(Slider, RectF)>,
    )> {
        let btn = self.menu_btn?;
        let (_, br) = self
            .tb_buttons(win_w, win_h)
            .into_iter()
            .find(|(i, _)| *i == btn)?;
        let sel = self.sel?;

        let has_picker = btn != 5; // 马赛克不吃颜色
        let has_mode = btn == 5; // 马赛克：框选 / 涂抹
        let sliders: Vec<Slider> = match btn {
            0 | 1 | 2 | 3 => vec![Slider::Width],
            4 => vec![Slider::TextSize],
            5 => vec![Slider::Block, Slider::Brush],
            _ => vec![],
        };
        if !has_picker && !has_mode && sliders.is_empty() {
            return None;
        }
        let w = 200.0;
        let picker_h = if has_picker {
            SV_H + HUE_H + HEX_H + 8.0
        } else {
            0.0
        };
        let rows = has_mode as usize + sliders.len();
        let h = MENU_PAD * 2.0 + picker_h + rows as f32 * MENU_ROW_H - 4.0;

        // 工具栏在选区下方 → 菜单在工具栏下方；反之在上方。
        let toolbar_below = br.y > sel.bottom() - 1.0;
        let mx = br.x.clamp(4.0, (win_w - w - 4.0).max(4.0));
        let my = if toolbar_below {
            (br.y + br.h + 6.0).min(win_h - h - 4.0)
        } else {
            (br.y - h - 6.0).max(4.0)
        };
        let panel = RectF::new(mx, my, w, h);

        let mut picker = None;
        let mut modes = Vec::new();
        let mut slider_rows = Vec::new();
        let mut ry = my + MENU_PAD;
        if has_picker {
            let sv = RectF::new(mx + MENU_PAD, ry, w - MENU_PAD * 2.0, SV_H);
            let hue = RectF::new(mx + MENU_PAD, ry + SV_H + 4.0, w - MENU_PAD * 2.0, HUE_H);
            let hex = RectF::new(
                mx + MENU_PAD,
                ry + SV_H + HUE_H + 8.0,
                w - MENU_PAD * 2.0,
                HEX_H,
            );
            picker = Some((sv, hue, hex));
            ry += picker_h;
        }
        if has_mode {
            let cw = (w - MENU_PAD * 2.0 - 6.0) / 2.0;
            modes.push((
                false,
                RectF::new(mx + MENU_PAD, ry + 3.0, cw, MENU_ROW_H - 8.0),
            ));
            modes.push((
                true,
                RectF::new(mx + MENU_PAD + cw + 6.0, ry + 3.0, cw, MENU_ROW_H - 8.0),
            ));
            ry += MENU_ROW_H;
        }
        for sl in sliders {
            // 行内布局：[标签 30px] [轨道 ...flex] [数值 30px]
            let label_w = 34.0;
            let val_w = 30.0;
            let tx = mx + MENU_PAD + label_w;
            let tw = w - MENU_PAD * 2.0 - label_w - val_w;
            slider_rows.push((sl, RectF::new(tx, ry, tw, MENU_ROW_H - 4.0)));
            ry += MENU_ROW_H;
        }
        Some((panel, picker, modes, slider_rows))
    }

    /// 从滑杆行的 x 坐标反算数值（吸附 0.5 步进）。
    pub(super) fn slider_value(&self, sl: Slider, track: RectF, x: f32) -> f32 {
        let (min, max) = sl.range();
        let t = ((x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0);
        ((min + (max - min) * t) * 2.0).round() / 2.0
    }

    pub(super) fn set_slider(&mut self, sl: Slider, v: f32) {
        let (min, max) = sl.range();
        let v = v.clamp(min, max);
        match sl {
            Slider::Width => self.stroke = v,
            Slider::Block => self.block = v,
            Slider::Brush => self.brush_radius = v,
            Slider::TextSize => self.text_size = v,
        }
    }

    /// 菜单点选：命中条目则应用并关闭，返回是否处理了这次点击。
    pub(super) fn on_menu_click(&mut self, win_w: f32, win_h: f32, x: f32, y: f32) -> bool {
        let Some((_, _, modes, _sliders)) = self.menu_layout(win_w, win_h) else {
            return false;
        };
        for (is_brush, r) in modes {
            if r.contains(x, y) {
                self.mosaic_brush = is_brush;
                self.menu_btn = None;
                return true;
            }
        }
        false
    }

    pub(super) fn set_hsv(&mut self, h: f32, sa: f32, v: f32) {
        self.hue = h.rem_euclid(360.0);
        self.sat = sa.clamp(0.0, 1.0);
        self.val = v.clamp(0.0, 1.0);
        self.color = paint::hsv_to_rgb(self.hue, self.sat, self.val);
    }
}
