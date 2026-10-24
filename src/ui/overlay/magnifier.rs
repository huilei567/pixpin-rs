use super::*;

impl Overlay {
    /// 放大镜左上角；放不下就翻到光标另一侧。
    pub(super) fn loupe_pos(&self, win_w: f32, win_h: f32) -> (f32, f32) {
        let (cx, cy) = self.cursor;
        let total_w = LOUPE + 8.0 + PANEL_W;
        let total_h = LOUPE.max(64.0);
        let mut x = cx + 22.0;
        let mut y = cy + 22.0;
        if x + total_w > win_w - 6.0 {
            x = cx - 22.0 - total_w;
        }
        if y + total_h > win_h - 6.0 {
            y = cy - 22.0 - total_h;
        }
        (x.max(6.0), y.max(6.0))
    }
}
