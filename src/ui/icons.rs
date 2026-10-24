//! 工具栏图标：全部用 `PathBuilder` 现画的线性图形。
//!
//! 不引入 SVG 资源、不依赖图标字体 —— 覆盖层本身就是一块画布，图标在 paint
//! 阶段直接落进场景，缩放到任何 DPI 都是干净的。
//!
//! 每个图标都在一个 [-1, 1] 的设计盒里描述，`y` 轴向下，绘制时按
//! `size / 2` 缩放并平移到给定中心。

use gpui::{point, px, Hsla, PathBuilder, Point, Window};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Rect,
    Ellipse,
    Pen,
    Arrow,
    Mosaic,
    Undo,
    Redo,
    Ocr,
    Pin,
    Save,
    Copy,
    Close,
    Gear,
    Info,
    Text,
    Eraser,
    CaretDown,
}

/// 把设计盒坐标映射到屏幕坐标。
struct Map {
    cx: f32,
    cy: f32,
    s: f32,
}

impl Map {
    fn p(&self, x: f32, y: f32) -> Point<gpui::Pixels> {
        point(px(self.cx + x * self.s), px(self.cy + y * self.s))
    }
}

fn stroke(window: &mut Window, m: &Map, pts: &[(f32, f32)], color: Hsla, width: f32) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(m.p(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(m.p(p.0, p.1));
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn stroke_closed(window: &mut Window, m: &Map, pts: &[(f32, f32)], color: Hsla, width: f32) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(m.p(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(m.p(p.0, p.1));
    }
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn fill(window: &mut Window, m: &Map, pts: &[(f32, f32)], color: Hsla) {
    if pts.len() < 3 {
        return;
    }
    let mut b = PathBuilder::fill();
    b.move_to(m.p(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(m.p(p.0, p.1));
    }
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// 二次贝塞尔近似的一段圆弧（四段拼成整圆）。
fn arc_quad(
    window: &mut Window,
    m: &Map,
    pts: &[(f32, f32)],
    ctrl: (f32, f32),
    color: Hsla,
    width: f32,
) {
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(m.p(pts[0].0, pts[0].1));
    b.curve_to(m.p(pts[1].0, pts[1].1), m.p(ctrl.0, ctrl.1));
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn circle(window: &mut Window, m: &Map, r: f32, color: Hsla, width: f32) {
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(m.p(0.0, -r));
    b.curve_to(m.p(r, 0.0), m.p(r, -r));
    b.curve_to(m.p(0.0, r), m.p(r, r));
    b.curve_to(m.p(-r, 0.0), m.p(-r, r));
    b.curve_to(m.p(0.0, -r), m.p(-r, -r));
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// 在 `(cx, cy)` 处画一个 `size` 见方的图标。
pub fn paint_icon(
    window: &mut Window,
    icon: Icon,
    cx: f32,
    cy: f32,
    size: f32,
    color: Hsla,
    width: f32,
) {
    let m = Map {
        cx,
        cy,
        s: size / 2.0,
    };

    match icon {
        Icon::Rect => stroke_closed(
            window,
            &m,
            &[(-0.82, -0.62), (0.82, -0.62), (0.82, 0.62), (-0.82, 0.62)],
            color,
            width,
        ),

        Icon::Ellipse => circle(window, &m, 0.78, color, width),

        // 铅笔：两条从笔尖出发、略微张开的折线，中间留出笔身。
        Icon::Pen => {
            stroke(
                window,
                &m,
                &[(-0.86, 0.86), (-0.58, 0.22), (0.52, -0.88)],
                color,
                width,
            );
            stroke(
                window,
                &m,
                &[(-0.86, 0.86), (-0.22, 0.56), (0.86, -0.54)],
                color,
                width,
            );
            // 笔尖
            fill(
                window,
                &m,
                &[(-0.88, 0.88), (-0.5, 0.36), (-0.28, 0.58)],
                color,
            );
        }

        Icon::Arrow => {
            stroke(window, &m, &[(-0.78, 0.78), (0.42, -0.42)], color, width);
            fill(
                window,
                &m,
                &[(0.86, -0.86), (0.74, -0.28), (0.28, -0.74)],
                color,
            );
        }

        // 马赛克：2×2 棋盘，对角两块填实。
        // 马赛克：2×2 圆角块，透明度从左上到右下渐变 ——
        // 「图像被打成像素」的观感，比等大实心块柔和。
        Icon::Mosaic => {
            let q = 0.78; // 块边长
            let gp = 0.14; // 间隙
            let o = -(q * 2.0 + gp) / 2.0;
            let rr = 0.14; // 圆角
            let mut sq = |x: f32, y: f32, alpha: f32| {
                let c = Hsla { a: alpha, ..color };
                let mut b = PathBuilder::fill();
                b.move_to(m.p(x + rr, y));
                b.line_to(m.p(x + q - rr, y));
                b.curve_to(m.p(x + q, y + rr), m.p(x + q, y));
                b.line_to(m.p(x + q, y + q - rr));
                b.curve_to(m.p(x + q - rr, y + q), m.p(x + q, y + q));
                b.line_to(m.p(x + rr, y + q));
                b.curve_to(m.p(x, y + q - rr), m.p(x, y + q));
                b.line_to(m.p(x, y + rr));
                b.curve_to(m.p(x + rr, y), m.p(x, y));
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, c);
                }
            };
            sq(o, o, 1.0);
            sq(o + q + gp, o, 0.5);
            sq(o, o + q + gp, 0.5);
            sq(o + q + gp, o + q + gp, 0.85);
        }

        Icon::Undo | Icon::Redo => {
            let s = if icon == Icon::Redo { -1.0 } else { 1.0 };
            // 上半圆
            arc_quad(
                window,
                &m,
                &[(-0.75 * s, 0.12), (0.75 * s, 0.12)],
                (0.0, -1.05),
                color,
                width,
            );
            // 左端的向下箭头
            fill(
                window,
                &m,
                &[(-0.75 * s, 0.62), (-1.02 * s, 0.02), (-0.48 * s, 0.02)],
                color,
            );
        }

        // 取字：四角取景框 + 字母 A
        Icon::Ocr => {
            let c = 0.95;
            let a = 0.5;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                stroke(
                    window,
                    &m,
                    &[
                        (sx * c, sy * c - sy * (c - a)),
                        (sx * c, sy * c),
                        (sx * c - sx * (c - a), sy * c),
                    ],
                    color,
                    width,
                );
            }
            stroke(window, &m, &[(-0.38, 0.42), (0.0, -0.5)], color, width);
            stroke(window, &m, &[(0.0, -0.5), (0.38, 0.42)], color, width);
            stroke(window, &m, &[(-0.2, 0.1), (0.2, 0.1)], color, width);
        }

        // 图钉：帽子和钉身之间留一道缝，否则缩到 19px 就成了漏斗
        // 贴图：描边图钉（Lucide「pin」字形）—— 圆顶头 + 两侧外扩的底座
        // + 竖直长针。曲线用二次贝塞尔近似，线宽与其他图标一致。
        Icon::Pin => {
            let g = |v: f32| v / 12.0 - 1.0; // 24 栅格 → 设计盒

            let mut b = PathBuilder::stroke(px(width));
            // 底座横杆：左 → 右
            b.move_to(m.p(g(5.0), g(17.0)));
            b.line_to(m.p(g(19.0), g(17.0)));
            // 右侧：上收 → 外扩弧 → 颈
            b.line_to(m.p(g(19.0), g(15.24)));
            b.curve_to(m.p(g(16.11), g(13.45)), m.p(g(19.0), g(14.35)));
            b.line_to(m.p(g(14.33), g(12.55)));
            b.curve_to(m.p(g(15.0), g(10.76)), m.p(g(14.5), g(11.6)));
            b.line_to(m.p(g(15.0), g(6.0)));
            b.line_to(m.p(g(16.0), g(6.0)));
            // 圆顶：右侧鼓出 → 顶边 → 左侧鼓出
            b.curve_to(m.p(g(16.0), g(2.0)), m.p(g(18.1), g(4.0)));
            b.line_to(m.p(g(8.0), g(2.0)));
            b.curve_to(m.p(g(8.0), g(6.0)), m.p(g(5.9), g(4.0)));
            b.line_to(m.p(g(9.0), g(6.0)));
            b.line_to(m.p(g(9.0), g(10.76)));
            b.curve_to(m.p(g(7.67), g(12.55)), m.p(g(9.5), g(11.6)));
            b.line_to(m.p(g(5.89), g(13.45)));
            b.curve_to(m.p(g(5.0), g(15.24)), m.p(g(5.45), g(14.35)));
            b.close();
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
            // 针：竖直
            stroke(
                window,
                &m,
                &[(g(12.0), g(17.0)), (g(12.0), g(22.6))],
                color,
                width,
            );
        }

        // 保存：向下的箭头落进托盘
        Icon::Save => {
            stroke(window, &m, &[(0.0, -0.82), (0.0, 0.16)], color, width);
            fill(
                window,
                &m,
                &[(0.0, 0.44), (-0.34, 0.02), (0.34, 0.02)],
                color,
            );
            stroke(
                window,
                &m,
                &[(-0.78, 0.06), (-0.78, 0.78), (0.78, 0.78), (0.78, 0.06)],
                color,
                width,
            );
        }

        // 复制：两张错开的方框
        Icon::Copy => {
            stroke(
                window,
                &m,
                &[(-0.34, -0.86), (0.86, -0.86), (0.86, 0.34)],
                color,
                width,
            );
            stroke_closed(
                window,
                &m,
                &[(-0.86, -0.34), (0.34, -0.34), (0.34, 0.86), (-0.86, 0.86)],
                color,
                width,
            );
        }

        Icon::Close => {
            stroke(window, &m, &[(-0.68, -0.68), (0.68, 0.68)], color, width);
            stroke(window, &m, &[(0.68, -0.68), (-0.68, 0.68)], color, width);
        }

        // 齿轮：外圈 + 八个齿 + 中孔
        Icon::Gear => {
            circle(window, &m, 0.52, color, width);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (c, s) = (a.cos(), a.sin());
                stroke(
                    window,
                    &m,
                    &[(0.72 * c, 0.72 * s), (1.0 * c, 1.0 * s)],
                    color,
                    width,
                );
            }
            circle(window, &m, 0.14, color, width * 0.9);
        }

        // 文本：衬线 T 字 —— 顶部横杠 + 中竖
        Icon::Text => {
            stroke(
                window,
                &m,
                &[(-0.72, -0.78), (0.72, -0.78)],
                color,
                width * 1.15,
            );
            stroke(
                window,
                &m,
                &[(0.0, -0.78), (0.0, 0.86)],
                color,
                width * 1.15,
            );
            // 底部短横增强 T 的识别度（ serif 风格）
            stroke(window, &m, &[(-0.3, 0.86), (0.3, 0.86)], color, width);
        }

        // 橡皮擦：斜 45° 的板擦 —— 圆角宽扁矩形 + 顶部套的分割线。
        Icon::Eraser => {
            const S: f32 = std::f32::consts::FRAC_1_SQRT_2;
            let rot = |x: f32, y: f32| -> (f32, f32) { ((x + y) * S, (y - x) * S) };
            let (x0, x1, y0, y1) = (-0.66, 0.66, -0.34, 0.34);
            let ch = 0.10;
            let body: Vec<(f32, f32)> = [
                (x0 + ch, y0),
                (x1 - ch, y0),
                (x1, y0 + ch),
                (x1, y1 - ch),
                (x1 - ch, y1),
                (x0 + ch, y1),
                (x0, y1 - ch),
                (x0, y0 + ch),
            ]
            .iter()
            .map(|(x, y)| rot(*x, *y))
            .collect();
            stroke_closed(window, &m, &body, color, width);
            let (d0, d1) = (rot(-0.18, y0), rot(-0.18, y1));
            stroke(window, &m, &[d0, d1], color, width * 0.9);
        }

        // 下拉箭头
        Icon::CaretDown => {
            stroke(
                window,
                &m,
                &[(-0.45, -0.3), (0.05, 0.25), (0.55, -0.3)],
                color,
                width * 1.2,
            );
        }

        // 信息：圆圈 + i
        Icon::Info => {
            circle(window, &m, 0.86, color, width);
            stroke(window, &m, &[(0.0, -0.12), (0.0, 0.56)], color, width);
            fill(
                window,
                &m,
                &[(0.0, -0.62), (0.12, -0.5), (0.0, -0.38), (-0.12, -0.5)],
                color,
            );
        }
    }
}
