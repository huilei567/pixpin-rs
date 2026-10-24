//! 标注图形的数据模型、命中测试与纯几何工具。
//!
//! 这里不依赖窗口系统，也不做像素写入；导出用的软件栅格化在 `raster.rs`。

pub const PALETTE: [u32; 8] = [
    0xFF3B30, // 红
    0xFF9500, // 橙
    0xFFCC00, // 黄
    0x34C759, // 绿
    0x0A84FF, // 蓝
    0xAF52DE, // 紫
    0x1C1C1E, // 黑
    0xFFFFFF, // 白
];

#[derive(Clone, Debug)]
pub enum Shape {
    Rect {
        a: (f32, f32),
        b: (f32, f32),
        color: u32,
        width: f32,
    },
    Ellipse {
        a: (f32, f32),
        b: (f32, f32),
        color: u32,
        width: f32,
    },
    Arrow {
        a: (f32, f32),
        b: (f32, f32),
        color: u32,
        width: f32,
    },
    Pen {
        pts: Vec<(f32, f32)>,
        color: u32,
        width: f32,
    },
    /// 打码：把矩形区域按 `block` 大小块状化。
    Mosaic {
        a: (f32, f32),
        b: (f32, f32),
        block: f32,
    },
    /// 打码（涂抹模式）：沿笔刷轨迹盖出圆盘状的块状化区域。
    MosaicStroke {
        pts: Vec<(f32, f32)>,
        /// 笔刷半径（逻辑像素）
        radius: f32,
        block: f32,
    },
    /// 文本标注：左上角锚点，雅黑渲染。
    Text {
        pos: (f32, f32),
        text: String,
        color: u32,
        size: f32,
        /// 提交时量好的宽度（逻辑像素），命中测试和光标定位用。
        w: f32,
    },
}

/// 光标点到形状的距离是否在橡皮擦的擦除范围内。
///
/// 描边类形状测「点到轮廓的距离」，填充类测「点是否在内部」。
pub fn shape_hit(shape: &Shape, x: f32, y: f32, tol: f32) -> bool {
    let seg_dist = |a: (f32, f32), b: (f32, f32)| -> f32 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            return ((x - a.0).powi(2) + (y - a.1).powi(2)).sqrt();
        }
        let t = (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0);
        let (px, py) = (a.0 + dx * t, a.1 + dy * t);
        ((x - px).powi(2) + (y - py).powi(2)).sqrt()
    };
    let bbox = |a: (f32, f32), b: (f32, f32)| -> (f32, f32, f32, f32) {
        (
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        )
    };
    match shape {
        Shape::Rect { a, b, width, .. } => {
            let (rx, ry, rw, rh) = bbox(*a, *b);
            // 点到矩形四条边的最小距离
            let dxl = (x - rx).abs();
            let dxr = (x - (rx + rw)).abs();
            let dyt = (y - ry).abs();
            let dyb = (y - (ry + rh)).abs();
            let inside = x >= rx && x <= rx + rw && y >= ry && y <= ry + rh;
            if inside {
                dxl.min(dxr).min(dyt).min(dyb) <= width / 2.0 + tol
            } else {
                // 角落附近也算
                let cx = x.clamp(rx, rx + rw);
                let cy = y.clamp(ry, ry + rh);
                ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() <= tol
            }
        }
        Shape::Ellipse { a, b, width, .. } => {
            let (rx, ry, rw, rh) = bbox(*a, *b);
            if rx < 1.0 || ry < 1.0 {
                return false;
            }
            let (cx, cy) = (rx + rw / 2.0, ry + rh / 2.0);
            let v = (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt();
            (v - 1.0).abs() * rx.min(ry) <= width / 2.0 + tol
        }
        Shape::Arrow { a, b, width, .. } => {
            seg_dist(*a, *b) <= width / 2.0 + tol || {
                let head = arrow_head(*a, *b, *width);
                point_in_triangle(x, y, head)
            }
        }
        Shape::Pen { pts, width, .. } => {
            pts.windows(2)
                .any(|p| seg_dist(p[0], p[1]) <= width / 2.0 + tol)
                || pts.first().map_or(false, |p| {
                    ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt() <= width / 2.0 + tol
                })
        }
        Shape::Mosaic { a, b, .. } => {
            let (rx, ry, rw, rh) = bbox(*a, *b);
            x >= rx - tol && x <= rx + rw + tol && y >= ry - tol && y <= ry + rh + tol
        }
        Shape::MosaicStroke { pts, radius, .. } => {
            pts.windows(2).any(|p| seg_dist(p[0], p[1]) <= radius + tol)
                || pts.first().map_or(false, |p| {
                    ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt() <= radius + tol
                })
        }
        Shape::Text { pos, size, w, .. } => {
            let h = size * 1.25;
            x >= pos.0 - tol && x <= pos.0 + w + tol && y >= pos.1 - tol && y <= pos.1 + h + tol
        }
    }
}

fn point_in_triangle(x: f32, y: f32, p: [(f32, f32); 3]) -> bool {
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    if area.abs() < 1e-6 {
        return false;
    }
    let w0 = ((p[1].0 - x) * (p[2].1 - y) - (p[2].0 - x) * (p[1].1 - y)) / area;
    let w1 = ((p[2].0 - x) * (p[0].1 - y) - (p[0].0 - x) * (p[2].1 - y)) / area;
    let w2 = 1.0 - w0 - w1;
    w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
}

/// HSV(h 0-360, s/v 0-1) → RGB u32。
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0) as u32) << 16 | (((g + m) * 255.0) as u32) << 8 | ((b + m) * 255.0) as u32
}

/// RGB u32 → HSV(h 0-360, s/v 0-1)。
pub fn rgb_to_hsv(color: u32) -> (f32, f32, f32) {
    let (r, g, b) = (
        ((color >> 16) & 0xFF) as f32 / 255.0,
        ((color >> 8) & 0xFF) as f32 / 255.0,
        (color & 0xFF) as f32 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-6 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max < 1e-6 { 0.0 } else { d / max };
    (h, s, max)
}

/// Arrow head triangle for a shaft going from `a` to `b`.
pub fn arrow_head(a: (f32, f32), b: (f32, f32), width: f32) -> [(f32, f32); 3] {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.0).clamp(10.0, 40.0);
    let base = (b.0 - ux * head, b.1 - uy * head);
    let spread = head * 0.45;
    let nx = -uy * spread;
    let ny = ux * spread;
    [b, (base.0 + nx, base.1 + ny), (base.0 - nx, base.1 - ny)]
}

/// 画笔轨迹平滑：去重 → Chaikin 两次细分。
///
/// 鼠标事件是离散的，快速拖动时相邻点能差几十像素，直接连折线就会看到
/// 一段段直线和硬角 —— 这就是「像素感重」的来源。Chaikin 把每个角切掉，
/// 收敛到一条二次 B 样条：预览和导出都走这里，两边长得一样。
pub fn smooth_pts(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    // 同一像素上的连续事件对曲线没有贡献，只会让切线方向乱跳。
    let mut src: Vec<(f32, f32)> = Vec::with_capacity(pts.len());
    for &p in pts {
        match src.last() {
            Some(&last) if (p.0 - last.0).powi(2) + (p.1 - last.1).powi(2) < 0.36 => {}
            _ => src.push(p),
        }
    }
    if src.len() < 3 {
        return src;
    }
    // 点越多越不需要多轮细分：两轮就够顺，点太多反而拖慢每帧重建路径。
    let rounds = if src.len() > 400 { 1 } else { 2 };
    let mut cur = src;
    for _ in 0..rounds {
        let mut next = Vec::with_capacity(cur.len() * 2);
        next.push(cur[0]);
        for pair in cur.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            next.push((a.0 * 0.75 + b.0 * 0.25, a.1 * 0.75 + b.1 * 0.25));
            next.push((a.0 * 0.25 + b.0 * 0.75, a.1 * 0.25 + b.1 * 0.75));
        }
        next.push(cur[cur.len() - 1]);
        cur = next;
    }
    cur
}

/// 沿折线按 `step` 间距插值，涂抹类笔刷用（事件点之间可能隔得很远）。
pub fn interp_pts(pts: &[(f32, f32)], step: f32) -> Vec<(f32, f32)> {
    let step = step.max(1.0);
    let mut out = Vec::new();
    if let Some(&first) = pts.first() {
        out.push(first);
    }
    for pair in pts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let n = (d / step).ceil() as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}
