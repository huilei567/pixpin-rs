//! 应用图标像素绘制（全平台共享）：托盘 / 窗口图标 / .ico 资源同源。

fn in_rounded_rect(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> bool {
    if x < x0 || y < y0 || x > x1 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// 任意尺寸绘制应用图标（RGBA，自上而下）。32px 设计稿等比缩放。
pub fn draw_icon_rgba(size: u32) -> Vec<u8> {
    let s = size as usize;
    let mut px = vec![0u8; s * s * 4];
    let u = size as f32 / 32.0; // 设计稿是 32px
    let blue = [0x0Au8, 0x84, 0xFF, 0xFF];
    let white = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let mut set = |x: i32, y: i32, c: [u8; 4]| {
        if x < 0 || y < 0 || x >= s as i32 || y >= s as i32 {
            return;
        }
        let i = ((y as usize) * s + x as usize) * 4;
        px[i] = c[0];
        px[i + 1] = c[1];
        px[i + 2] = c[2];
        px[i + 3] = c[3];
    };
    let f = |v: f32| (v * u).round() as i32;
    for y in 0..s as i32 {
        for x in 0..s as i32 {
            if in_rounded_rect(
                x,
                y,
                f(1.0),
                f(1.0),
                s as i32 - 1 - f(1.0),
                s as i32 - 1 - f(1.0),
                f(7.0),
            ) {
                set(x, y, blue);
            }
        }
    }
    let t = f(2.0) as usize;
    let lo = f(7.0) as usize;
    let hi = f(24.0) as usize;
    let arm = f(7.0) as usize;
    let mut bar = |x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..=y1.min(s - 1) {
            for x in x0..=x1.min(s - 1) {
                set(x as i32, y as i32, white);
            }
        }
    };
    bar(lo, lo + arm, lo, lo + t - 1);
    bar(lo, lo + t - 1, lo, lo + arm);
    bar(hi - arm, hi, lo, lo + t - 1);
    bar(hi - t + 1, hi, lo, lo + arm);
    bar(lo, lo + arm, hi - t + 1, hi);
    bar(lo, lo + t - 1, hi - arm, hi);
    bar(hi - arm, hi, hi - t + 1, hi);
    bar(hi - t + 1, hi, hi - arm, hi);
    px
}

/// 生成多尺寸 .ico 文件内容（16–256），供 build.rs 嵌入 exe 资源。
pub fn icon_ico_bytes() -> Vec<u8> {
    let sizes: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    for &sz in &sizes {
        let rgba = draw_icon_rgba(sz);
        let s = sz as usize;
        // ICO 里的 BMP：BGRA、行序自下而上
        let mut px = vec![0u8; s * s * 4];
        for y in 0..s {
            for x in 0..s {
                let src = ((y * s + x) * 4) as usize;
                let dst = (((s - 1 - y) * s + x) * 4) as usize;
                px[dst] = rgba[src + 2];
                px[dst + 1] = rgba[src + 1];
                px[dst + 2] = rgba[src];
                px[dst + 3] = rgba[src + 3];
            }
        }
        let mut bmp = Vec::new();
        bmp.extend_from_slice(&40u32.to_le_bytes()); // biSize
        bmp.extend_from_slice(&(sz as i32).to_le_bytes()); // 宽
        bmp.extend_from_slice(&((sz as i32) * 2).to_le_bytes()); // 高（XOR+AND）
        bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
        bmp.extend_from_slice(&32u16.to_le_bytes()); // bpp
        bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        bmp.extend_from_slice(&((s * s * 4) as u32).to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&px);
        let mask_row = ((sz + 31) / 32) * 4; // AND 掩码行按 32bit 对齐
        bmp.extend(std::iter::repeat(0u8).take(mask_row as usize * s));
        images.push((sz, bmp));
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6u32 + 16u32 * images.len() as u32;
    for (sz, data) in &images {
        out.push(if *sz >= 256 { 0 } else { *sz as u8 });
        out.push(if *sz >= 256 { 0 } else { *sz as u8 });
        out.push(0); // 调色板色数
        out.push(0); // 保留
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// The icon pixels in RGBA, for previewing / tests.
pub fn icon_rgba() -> Vec<u8> {
    draw_icon_rgba(32)
}

pub fn icon_size() -> u32 {
    32
}
