//! 生成一张已知内容的测试图，用来量识别正确率。
//! 运行：`cargo run --example make_test_image`

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::{Rgb, RgbImage};

const FONTS: &[(&str, u32)] = &[
    (r"C:\Windows\Fonts\msyh.ttc", 0),
    (r"C:\Windows\Fonts\Deng.ttf", 0),
    (r"C:\Windows\Fonts\simhei.ttf", 0),
];

fn main() -> anyhow::Result<()> {
    let (font, idx) = FONTS
        .iter()
        .find(|(p, _)| std::path::Path::new(p).exists())
        .ok_or_else(|| anyhow::anyhow!("找不到中文字体"))?;
    let data = std::fs::read(font)?;
    let font = FontVec::try_from_vec_and_index(data, *idx)?;

    let mut img = RgbImage::from_pixel(900, 520, Rgb([255, 255, 255]));

    let lines: &[(f32, f32, &str, [u8; 3])] = &[
        (34.0, 46.0, "本地 OCR 引擎验收", [20, 20, 20]),
        (22.0, 110.0, "这一行用来验证中文段落的换行合并是否正确，", [70, 70, 70]),
        (22.0, 148.0, "第二行应该和上一行接在同一段里，中间不加空格。", [70, 70, 70]),
        (22.0, 200.0, "Mixed English and 中文 in one line, 1234567890.", [70, 70, 70]),
        (20.0, 248.0, "fn main() { println!(\"hello, world\"); }", [20, 20, 20]),
        (20.0, 284.0, "检测阈值 0.3 / 框阈值 0.6 / unclip 1.5", [70, 70, 70]),
        (11.0, 330.0, "这是一行很小的字，用来测试小字号召回能力。", [90, 90, 90]),
        (18.0, 470.0, "右下角签名 PixOCR 2026", [70, 70, 70]),
    ];

    for (size, baseline, text, color) in lines {
        draw_line(&mut img, &font, *size, 40.0, *baseline, text, *color);
    }

    img.save("test-render.png")?;
    println!("已生成 test-render.png");
    println!("--- 标准答案 ---");
    for (_, _, text, _) in lines {
        println!("{text}");
    }
    Ok(())
}

fn draw_line(
    img: &mut RgbImage,
    font: &FontVec,
    size: f32,
    x: f32,
    baseline: f32,
    text: &str,
    color: [u8; 3],
) {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let mut caret = x;

    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bb = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let px = bb.min.x as i32 + gx as i32;
                let py = bb.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px as u32 >= img.width() || py as u32 >= img.height() {
                    return;
                }
                let p = img.get_pixel_mut(px as u32, py as u32);
                for i in 0..3 {
                    let bg = p[i] as f32;
                    let fg = color[i] as f32;
                    p[i] = (bg * (1.0 - cov) + fg * cov).round().clamp(0.0, 255.0) as u8;
                }
            });
        }
        caret += scaled.h_advance(gid);
    }
}
