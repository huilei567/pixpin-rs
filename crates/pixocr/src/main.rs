//! 命令行工具：`pixocr <图片> [--models <目录>] [--json] [--probe]`

use anyhow::{Context, Result};
use pixocr::Ocr;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print_help();
        return Ok(());
    }

    let mut img_path: Option<PathBuf> = None;
    let mut model_dir = Ocr::default_model_dir();
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--models" => {
                i += 1;
                if i < args.len() {
                    model_dir = PathBuf::from(&args[i]);
                }
            }
            "--json" => json = true,
            other => img_path = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    let img_path = img_path.context("没有指定图片路径")?;
    let img = image::open(&img_path)
        .with_context(|| format!("打开图片失败: {}", img_path.display()))?
        .to_rgb8();

    let t0 = Instant::now();
    let mut ocr = Ocr::load(&model_dir)?;
    let load_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let (blocks, stats) = ocr.recognize_profiled(&img)?;
    let total_ms = t1.elapsed().as_millis();

    if json {
        print_json(&blocks, stats);
    } else {
        println!(
            "图片 {}x{}  ·  加载 {}ms  ·  检测 {}ms  ·  识别 {}ms  ·  合计 {}ms",
            img.width(),
            img.height(),
            load_ms,
            stats.det_ms,
            stats.rec_ms,
            total_ms
        );
        println!("检出 {} 个文本框，{} 个段落\n", stats.boxes, blocks.len());
        println!("{}", "─".repeat(60));
        for (i, b) in blocks.iter().enumerate() {
            println!("[段落 {}] {}", i + 1, b.text());
            for line in &b.lines {
                println!(
                    "    ({:>4},{:>4},{:>4},{:>4})  {:.2}  {}",
                    line.rect.x, line.rect.y, line.rect.w, line.rect.h, line.score, line.text
                );
            }
        }
        println!("{}", "─".repeat(60));
        println!("纯文本：\n");
        for b in &blocks {
            println!("{}\n", b.text());
        }
    }
    Ok(())
}

fn print_json(blocks: &[pixocr::Block], stats: pixocr::Stats) {
    let mut out = String::from("{\"blocks\":[");
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"text\":{},\"rect\":[{},{},{},{}],\"lines\":[",
            jstr(&b.text()),
            b.rect.x,
            b.rect.y,
            b.rect.w,
            b.rect.h
        ));
        for (j, l) in b.lines.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"text\":{},\"score\":{:.4},\"rect\":[{},{},{},{}]}}",
                jstr(&l.text),
                l.score,
                l.rect.x,
                l.rect.y,
                l.rect.w,
                l.rect.h
            ));
        }
        out.push_str("]}");
    }
    out.push_str(&format!(
        "],\"stats\":{{\"det_ms\":{},\"rec_ms\":{},\"boxes\":{}}}}}",
        stats.det_ms, stats.rec_ms, stats.boxes
    ));
    println!("{out}");
}

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn print_help() {
    println!(
        "pixocr —— 本地 OCR\n\
         \n\
         用法: pixocr <图片> [选项]\n\
         \n\
         选项:\n\
           --models <目录>   模型目录（默认找 exe 同级 models/）\n\
           --json            输出 JSON\n\
         \n\
         模型目录需要: det.onnx  rec.onnx  ppocrv5_dict.txt"
    );
}
