//! 几何基元。全部用整数像素坐标，原点在图像左上角。

/// 轴对齐矩形。截图场景里文字基本都是正的，所以这里不引入旋转框，
/// 代价是略微倾斜的文字会带一点背景，但换来的是整条链路没有透视变换。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn center_y(&self) -> f32 {
        self.y as f32 + self.h as f32 * 0.5
    }

    pub fn area(&self) -> f32 {
        (self.w.max(0) * self.h.max(0)) as f32
    }

    pub fn perimeter(&self) -> f32 {
        2.0 * (self.w.max(0) + self.h.max(0)) as f32
    }

    /// 四条边同时外扩 `d` 像素，再夹回 `bounds` 之内。
    pub fn expand(&self, d: f32, bounds: Rect) -> Rect {
        let x = (self.x as f32 - d).round() as i32;
        let y = (self.y as f32 - d).round() as i32;
        let r = (self.right() as f32 + d).round() as i32;
        let b = (self.bottom() as f32 + d).round() as i32;
        let x = x.clamp(bounds.x, bounds.right());
        let y = y.clamp(bounds.y, bounds.bottom());
        let r = r.clamp(bounds.x, bounds.right());
        let b = b.clamp(bounds.y, bounds.bottom());
        Rect::new(x, y, (r - x).max(0), (b - y).max(0))
    }

    /// 两个矩形在垂直方向的重叠比例（相对于较矮的那个）。
    pub fn vertical_overlap(&self, other: &Rect) -> f32 {
        let top = self.y.max(other.y);
        let bottom = self.bottom().min(other.bottom());
        let overlap = (bottom - top).max(0) as f32;
        let smaller = self.h.min(other.h).max(1) as f32;
        overlap / smaller
    }

    /// 把本矩形并进来，得到外接矩形。
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        Rect::new(x, y, r - x, b - y)
    }
}
