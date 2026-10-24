//! Minimal rectangle math shared by the overlay and the exporter.
//! All values are in *logical* pixels unless a function name says otherwise.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Build a normalized rect from two arbitrary corners.
    pub fn from_corners(ax: f32, ay: f32, bx: f32, by: f32) -> Self {
        let x = ax.min(bx);
        let y = ay.min(by);
        Self {
            x,
            y,
            w: (ax - bx).abs(),
            h: (ay - by).abs(),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom()
    }

    pub fn is_empty(&self) -> bool {
        self.w < 1.0 || self.h < 1.0
    }

    /// Clamp to `0..=max` and round to whole pixels.
    pub fn clamped_rounded(&self, max_w: f32, max_h: f32) -> RectF {
        let x = self.x.max(0.0).min(max_w);
        let y = self.y.max(0.0).min(max_h);
        let w = self.w.min(max_w - x).max(0.0);
        let h = self.h.min(max_h - y).max(0.0);
        RectF {
            x: x.round(),
            y: y.round(),
            w: w.round().max(1.0),
            h: h.round().max(1.0),
        }
    }
}
