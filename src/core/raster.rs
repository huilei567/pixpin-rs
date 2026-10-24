//! 把标注形状软件栅格化到 RGBA 图像，供复制 / 保存 / 贴图导出使用。
//!
//! 这里只处理像素，不依赖 GPUI；预览仍由 UI 层的矢量绘制负责。

use image::RgbaImage;

use super::annotation::{arrow_head, interp_pts, smooth_pts, Shape};

fn unpack(color: u32) -> (u8, u8, u8) {
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

fn blend(img: &mut RgbaImage, x: i32, y: i32, color: u32, coverage: f32) {
    if coverage <= 0.0 {
        return;
    }
    let (w, h) = (img.width() as i32, img.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let (r, g, b) = unpack(color);
    let a = coverage.clamp(0.0, 1.0);
    let px = img.get_pixel_mut(x as u32, y as u32);
    let dst_a = px[3] as f32 / 255.0;
    let out_a = a + dst_a * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    let mix = |src: u8, dst: u8| -> u8 {
        (((src as f32 * a) + (dst as f32 * dst_a * (1.0 - a))) / out_a).round() as u8
    };
    px[0] = mix(r, px[0]);
    px[1] = mix(g, px[1]);
    px[2] = mix(b, px[2]);
    px[3] = (out_a * 255.0).round() as u8;
}

/// Filled disc with a one pixel soft edge.
pub fn fill_disc(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, color: u32) {
    let r = radius.max(0.5);
    let x0 = (cx - r - 1.0).floor() as i32;
    let x1 = (cx + r + 1.0).ceil() as i32;
    let y0 = (cy - r - 1.0).floor() as i32;
    let y1 = (cy + r + 1.0).ceil() as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let coverage = (r - d + 0.5).clamp(0.0, 1.0);
            blend(img, x, y, color, coverage);
        }
    }
}

pub fn stroke_segment(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32), width: f32, color: u32) {
    stroke_polyline(img, &[a, b], width, color);
}

/// 点到线段的距离。描边用「到折线的最短距离」算覆盖率，
/// 这样圆头、圆角连接和斜线都能得到一像素宽的抗锯齿边缘。
fn segment_distance(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= 1e-6 {
        return ((px - a.0).powi(2) + (py - a.1).powi(2)).sqrt();
    }
    let t = (((px - a.0) * dx + (py - a.1) * dy) / len_sq).clamp(0.0, 1.0);
    let qx = a.0 + dx * t;
    let qy = a.1 + dy * t;
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

pub fn stroke_polyline(img: &mut RgbaImage, pts: &[(f32, f32)], width: f32, color: u32) {
    if pts.is_empty() {
        return;
    }
    let radius = (width / 2.0).max(0.5);
    if pts.len() == 1 {
        fill_disc(img, pts[0].0, pts[0].1, radius, color);
        return;
    }

    // 每条线段先写进同一张覆盖率图，最后每像素只 blend 一次。
    // 旧实现逐段盖圆盘，重叠处 alpha 会累积，斜线和转折处就会发毛、发脏。
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for p in pts {
        min_x = min_x.min(p.0);
        min_y = min_y.min(p.1);
        max_x = max_x.max(p.0);
        max_y = max_y.max(p.1);
    }
    let x0 = ((min_x - radius - 1.0).floor() as i32).max(0);
    let y0 = ((min_y - radius - 1.0).floor() as i32).max(0);
    let x1 = ((max_x + radius + 1.0).ceil() as i32).min(iw - 1);
    let y1 = ((max_y + radius + 1.0).ceil() as i32).min(ih - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let cw = (x1 - x0 + 1) as usize;
    let ch = (y1 - y0 + 1) as usize;
    let mut coverage = vec![0.0f32; cw * ch];

    for pair in pts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let sx0 = ((a.0.min(b.0) - radius - 1.0).floor() as i32).max(x0);
        let sy0 = ((a.1.min(b.1) - radius - 1.0).floor() as i32).max(y0);
        let sx1 = ((a.0.max(b.0) + radius + 1.0).ceil() as i32).min(x1);
        let sy1 = ((a.1.max(b.1) + radius + 1.0).ceil() as i32).min(y1);
        for y in sy0..=sy1 {
            for x in sx0..=sx1 {
                let d = segment_distance(x as f32 + 0.5, y as f32 + 0.5, a, b);
                let c = (radius + 0.5 - d).clamp(0.0, 1.0);
                if c > 0.0 {
                    let idx = (y - y0) as usize * cw + (x - x0) as usize;
                    if c > coverage[idx] {
                        coverage[idx] = c;
                    }
                }
            }
        }
    }

    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = coverage[(y - y0) as usize * cw + (x - x0) as usize];
            if c > 0.0 {
                blend(img, x, y, color, c);
            }
        }
    }
}

pub fn stroke_rect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, width: f32, color: u32) {
    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)];
    stroke_polyline(img, &corners, width, color);
}

pub fn stroke_ellipse(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, width: f32, color: u32) {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (rx, ry) = (w / 2.0, h / 2.0);
    let steps = (((w + h) / 2.0) * 1.6).clamp(48.0, 512.0) as usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32 * std::f32::consts::TAU;
        pts.push((cx + rx * t.cos(), cy + ry * t.sin()));
    }
    stroke_polyline(img, &pts, width, color);
}

pub fn fill_triangle(img: &mut RgbaImage, p: [(f32, f32); 3], color: u32) {
    let min_x = p.iter().map(|v| v.0).fold(f32::MAX, f32::min).floor() as i32;
    let max_x = p.iter().map(|v| v.0).fold(f32::MIN, f32::max).ceil() as i32;
    let min_y = p.iter().map(|v| v.1).fold(f32::MAX, f32::min).floor() as i32;
    let max_y = p.iter().map(|v| v.1).fold(f32::MIN, f32::max).ceil() as i32;

    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    if area.abs() < 1e-4 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (sx, sy) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - sx) * (p[2].1 - sy) - (p[2].0 - sx) * (p[1].1 - sy)) / area;
            let w1 = ((p[2].0 - sx) * (p[0].1 - sy) - (p[0].0 - sx) * (p[2].1 - sy)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                blend(img, x, y, color, 1.0);
            }
        }
    }
}

/// 把矩形区域块状化：每块取块内平均色再回填。
///
/// 这是真正的像素化而不是叠加模糊 —— 打码要的是「信息真的没了」。
pub fn mosaic(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, block: f32) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = x.floor().max(0.0) as i32;
    let y0 = y.floor().max(0.0) as i32;
    let x1 = ((x + w).ceil() as i32).min(iw);
    let y1 = ((y + h).ceil() as i32).min(ih);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let bs = block.round().max(2.0) as i32;
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        while bx < x1 {
            let ex = (bx + bs).min(x1);
            let ey = (by + bs).min(y1);
            let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
            for py in by..ey {
                for pxx in bx..ex {
                    let p = img.get_pixel(pxx as u32, py as u32);
                    sr += p[0] as u64;
                    sg += p[1] as u64;
                    sb += p[2] as u64;
                    n += 1;
                }
            }
            if n > 0 {
                let c = image::Rgba([
                    (sr / n) as u8,
                    (sg / n) as u8,
                    (sb / n) as u8,
                    img.get_pixel(bx as u32, by as u32)[3],
                ]);
                for py in by..ey {
                    for pxx in bx..ex {
                        img.put_pixel(pxx as u32, py as u32, c);
                    }
                }
            }
            bx += bs;
        }
        by += bs;
    }
}

/// Draw every shape into `img`. `scale` converts logical coords to image pixels
/// and `offset` is the crop origin in image pixels.
pub fn rasterize(img: &mut RgbaImage, shapes: &[Shape], scale: f32, offset: (f32, f32)) {
    let map = |p: (f32, f32)| -> (f32, f32) { (p.0 * scale - offset.0, p.1 * scale - offset.1) };
    for shape in shapes {
        match shape {
            Shape::Rect { a, b, color, width } => {
                let (a, b) = (map(*a), map(*b));
                let (x, y) = (a.0.min(b.0), a.1.min(b.1));
                let (w, h) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
                stroke_rect(img, x, y, w, h, width * scale, *color);
            }
            Shape::Ellipse { a, b, color, width } => {
                let (a, b) = (map(*a), map(*b));
                let (x, y) = (a.0.min(b.0), a.1.min(b.1));
                let (w, h) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
                stroke_ellipse(img, x, y, w, h, width * scale, *color);
            }
            Shape::Arrow { a, b, color, width } => {
                let (a, b) = (map(*a), map(*b));
                let w = width * scale;
                let head = arrow_head(a, b, w);
                let dx = b.0 - a.0;
                let dy = b.1 - a.1;
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let shrink = (w * 3.0).min(len * 0.6);
                let shaft_end = (b.0 - dx / len * shrink, b.1 - dy / len * shrink);
                stroke_segment(img, a, shaft_end, w, *color);
                fill_triangle(img, head, *color);
            }
            Shape::Pen { pts, color, width } => {
                // 先平滑再映射：预览用的是同一份平滑点，导出和肉眼看到的一致。
                let smooth = smooth_pts(pts);
                let mapped: Vec<(f32, f32)> = smooth.iter().map(|p| map(*p)).collect();
                stroke_polyline(img, &mapped, width * scale, *color);
            }
            Shape::Mosaic { a, b, block } => {
                let (a, b) = (map(*a), map(*b));
                let (x, y) = (a.0.min(b.0), a.1.min(b.1));
                let (w, h) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
                mosaic(img, x, y, w, h, block * scale);
            }
            Shape::MosaicStroke { pts, radius, block } => {
                let mapped: Vec<(f32, f32)> = pts.iter().map(|p| map(*p)).collect();
                let step = (radius * 0.5 * scale).max(2.0);
                for (px, py) in interp_pts(&mapped, step) {
                    mosaic_disc(img, px, py, radius * scale, block * scale);
                }
            }
            Shape::Text {
                pos,
                text,
                color,
                size,
                ..
            } => {
                let (px_, py) = map(*pos);
                draw_text_left(img, text, px_, py, size * scale, *color);
            }
        }
    }
}

// ---------- 文字栅格化（序号数字用） ----------

/// 系统字体懒加载：微软雅黑 → 雅黑旧版 → Arial，总有一个在。
fn ui_font() -> Option<&'static ab_glyph::FontVec> {
    static FONT: std::sync::OnceLock<Option<ab_glyph::FontVec>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        for path in [
            format!(r"{}\Fonts\msyh.ttc", windir),
            format!(r"{}\Fonts\msyh.ttf", windir),
            format!(r"{}\Fonts\arial.ttf", windir),
        ] {
            if let Ok(data) = std::fs::read(&path) {
                if let Ok(f) = ab_glyph::FontVec::try_from_vec_and_index(data, 0) {
                    eprintln!("[pixpin-rs] export font: {path}");
                    return Some(f);
                }
            }
        }
        eprintln!("[pixpin-rs] no system font found, number badges will be blank in exports");
        None
    })
    .as_ref()
}

/// 圆盘区域的块状化：与 [`mosaic`] 共用同一张块网格（锚点在原点），
/// 块内平均色对整块计算（不裁到圆盘内），重叠盖章结果稳定。
pub fn mosaic_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, block: f32) {
    use std::collections::HashMap;
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let bs = block.round().max(2.0) as i32;
    let x0 = ((cx - r).floor().max(0.0) as i32).min(iw);
    let x1 = ((cx + r).ceil() as i32).clamp(0, iw);
    let y0 = ((cy - r).floor().max(0.0) as i32).min(ih);
    let y1 = ((cy + r).ceil() as i32).clamp(0, ih);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // 先把涉及的块的平均色算出来
    let mut avgs: HashMap<(i32, i32), image::Rgba<u8>> = HashMap::new();
    let bx0 = x0 / bs;
    let bx1 = (x1 - 1) / bs;
    let by0 = y0 / bs;
    let by1 = (y1 - 1) / bs;
    for by in by0..=by1 {
        for bx in bx0..=bx1 {
            let ex = ((bx + 1) * bs).min(iw);
            let ey = ((by + 1) * bs).min(ih);
            let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
            for py in (by * bs)..ey {
                for pxx in (bx * bs)..ex {
                    let p = img.get_pixel(pxx as u32, py as u32);
                    sr += p[0] as u64;
                    sg += p[1] as u64;
                    sb += p[2] as u64;
                    n += 1;
                }
            }
            if n > 0 {
                avgs.insert(
                    (bx, by),
                    image::Rgba([(sr / n) as u8, (sg / n) as u8, (sb / n) as u8, 255]),
                );
            }
        }
    }
    let r2 = r * r;
    for py in y0..y1 {
        for pxx in x0..x1 {
            let dx = pxx as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r2 {
                if let Some(c) = avgs.get(&(pxx / bs, py / bs)) {
                    let mut px = *img.get_pixel(pxx as u32, py as u32);
                    px[0] = c[0];
                    px[1] = c[1];
                    px[2] = c[2];
                    img.put_pixel(pxx as u32, py as u32, px);
                }
            }
        }
    }
}

/// 左上角锚点画一行文字（文本标注导出用）；居中版见 [`draw_text`]。
pub fn draw_text_left(img: &mut RgbaImage, text: &str, x: f32, y: f32, size_px: f32, color: u32) {
    use ab_glyph::{Font as _, ScaleFont};
    let Some(font) = ui_font() else { return };
    let font = font.into_scaled(size_px);
    let baseline = y + font.ascent();
    let mut pen_x = x;
    for ch in text.chars() {
        let mut g = font.scaled_glyph(ch);
        g.position = ab_glyph::point(pen_x, baseline);
        if let Some(outlined) = font.outline_glyph(g) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                blend(
                    img,
                    bounds.min.x as i32 + gx as i32,
                    bounds.min.y as i32 + gy as i32,
                    color,
                    coverage as f32,
                );
            });
        }
        pen_x += font.h_advance(font.glyph_id(ch));
    }
}
