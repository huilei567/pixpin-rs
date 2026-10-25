//! DBNet 文字检测：前处理 → 推理 → 概率图后处理（连通域 + unclip）。

use crate::geom::Rect;
use anyhow::{anyhow, Result};
use image::{imageops, RgbImage};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::path::Path;

/// 送入网络前把长边压到这个尺寸以内。PP-OCR 训练时用的是 736，
/// 截图里字号偏小，放到 960 召回明显更好，代价是慢约 40%。
const LIMIT_SIDE: u32 = 960;
/// 概率图二值化阈值。
const BIN_THRESH: f32 = 0.3;
/// 整块区域的平均置信度低于这个值就丢掉。
const BOX_THRESH: f32 = 0.6;
/// unclip 扩张系数。DBNet 输出的是「收缩后的文字核」，要按周长比例还原。
const UNCLIP_RATIO: f32 = 1.5;
/// 网络要求 32 的整数倍。
const ALIGN: u32 = 32;

#[derive(Clone, Copy, Debug)]
pub struct TextBox {
    pub rect: Rect,
    pub score: f32,
}

pub struct Detector {
    session: Session,
}

impl Detector {
    pub fn load(model: &Path) -> Result<Self> {
        // ort 的 Error<R> 会带回滚用的资源（SessionBuilder 之类），
        // 不是 Send + Sync，所以不能直接 `?` 进 anyhow，统一转成字符串。
        let session = Session::builder()
            .map_err(oe)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(oe)?
            .with_intra_threads(num_threads())
            .map_err(oe)?
            .commit_from_file(model)
            .map_err(|e| anyhow!("加载检测模型失败 {}: {e}", model.display()))?;
        Ok(Self { session })
    }

    pub fn detect(&mut self, img: &RgbImage) -> Result<Vec<TextBox>> {
        let (iw, ih) = (img.width(), img.height());
        if iw == 0 || ih == 0 {
            return Ok(Vec::new());
        }

        // 1) 等比缩小 + 32 对齐（补 0 而不是拉伸，避免文字变形）
        let longest = iw.max(ih);
        let ratio = if longest > LIMIT_SIDE {
            LIMIT_SIDE as f32 / longest as f32
        } else {
            1.0
        };
        let rw = ((iw as f32 * ratio).round() as u32).max(ALIGN);
        let rh = ((ih as f32 * ratio).round() as u32).max(ALIGN);
        let pw = rw.div_ceil(ALIGN) * ALIGN;
        let ph = rh.div_ceil(ALIGN) * ALIGN;

        let resized = if rw != iw || rh != ih {
            imageops::resize(img, rw, rh, imageops::FilterType::Triangle)
        } else {
            img.clone()
        };

        // 2) RGB → CHW f32，ImageNet 均值方差（DBNet 就是这么训的）
        let plane = (pw * ph) as usize;
        let mut data = vec![0f32; 3 * plane];
        for y in 0..rh as usize {
            for x in 0..rw as usize {
                let p = resized.get_pixel(x as u32, y as u32);
                let idx = y * pw as usize + x;
                data[idx] = (p[0] as f32 / 255.0 - 0.485) / 0.229;
                data[plane + idx] = (p[1] as f32 / 255.0 - 0.456) / 0.224;
                data[2 * plane + idx] = (p[2] as f32 / 255.0 - 0.406) / 0.225;
            }
        }

        let input = Tensor::from_array(([1usize, 3, ph as usize, pw as usize], data))
            .map_err(|e| anyhow!("构造检测输入张量失败: {e}"))?;
        let outputs = self
            .session
            .run(ort::inputs!["x" => input])
            .map_err(|e| anyhow!("检测模型推理失败: {e}"))?;
        let (shape, probs) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("读取检测输出失败: {e}"))?;

        // 输出形状 [1, 1, H, W]，H/W 可能因为 pad 变成 32 的倍数
        let dims: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
        let (mh, mw) = match dims.len() {
            4 => (dims[2], dims[3]),
            3 => (dims[1], dims[2]),
            _ => anyhow::bail!("检测输出维度异常: {:?}", dims),
        };
        if mh == 0 || mw == 0 || probs.len() < mh * mw {
            anyhow::bail!("检测输出尺寸异常: {:?}", dims);
        }

        // 3) 二值化 + 连通域。DBNet 的 region map 一行文字是一块，
        //    所以直接做连通域就是「一行一个框」，不需要额外的行合并。
        let mask: Vec<bool> = probs[..mh * mw].iter().map(|v| *v > BIN_THRESH).collect();
        let comps = connected_components(&mask, mw, mh, probs);

        // 4) 过滤 + 拆分 + unclip + 坐标还原
        let bounds = Rect::new(0, 0, iw as i32, ih as i32);
        let mask_bounds = Rect::new(0, 0, mw as i32, mh as i32);
        let inv = 1.0 / ratio;
        let mut boxes = Vec::with_capacity(comps.len());

        for c in comps {
            if c.count < 4 {
                continue;
            }
            let score = c.sum / c.count as f32;
            if score < BOX_THRESH {
                continue;
            }
            let w = (c.max_x - c.min_x + 1) as f32;
            let h = (c.max_y - c.min_y + 1) as f32;
            // 细到一两个像素的多半是图标里的噪点
            if h < 2.0 || w < 2.0 {
                continue;
            }

            let base = Rect::new(c.min_x as i32, c.min_y as i32, w as i32, h as i32);
            // 特别宽的一行（菜单栏、状态栏）会在 rec 阶段被压扁，
            // 先按空白列切开，保证每段的宽高比在识别网络舒适区内。
            let parts = if w > 8.0 * h {
                split_by_columns(probs, mw, base)
            } else {
                vec![base]
            };

            for part in parts {
                let pw = part.w.max(1) as f32;
                let ph = part.h.max(1) as f32;
                // unclip：按面积/周长比外扩
                let d = (pw * ph * UNCLIP_RATIO) / (2.0 * (pw + ph));
                let r = part.expand(d, mask_bounds);

                // 网络坐标 → 原图坐标
                let out = Rect::new(
                    (r.x as f32 * inv).floor() as i32,
                    (r.y as f32 * inv).floor() as i32,
                    (r.w as f32 * inv).ceil() as i32,
                    (r.h as f32 * inv).ceil() as i32,
                )
                .expand(0.0, bounds);

                if out.w < 4 || out.h < 4 {
                    continue;
                }
                boxes.push(TextBox { rect: out, score });
            }
        }

        sort_reading_order(&mut boxes);
        Ok(boxes)
    }
}

struct Component {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    count: usize,
    sum: f32,
}

/// 在一块很宽的文本框里按「空白列」切开。
/// 判定用的不是二值图而是概率峰值，避免文字边缘的抗锯齿像素把空隙填满。
fn split_by_columns(prob: &[f32], mw: usize, r: Rect) -> Vec<Rect> {
    let h = r.h.max(1) as usize;
    let n = r.w.max(1) as usize;
    // 空隙阈值跟着行高走：行越高，字间空隙也越大
    let gap_thresh = ((r.h as f32 * 0.5).max(8.0)) as usize;

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut run = 0usize;

    for i in 0..n {
        let x = r.x as usize + i;
        if x >= mw {
            break;
        }
        let mut peak = 0f32;
        for y in r.y as usize..(r.y as usize + h) {
            let v = prob[y * mw + x];
            if v > peak {
                peak = v;
            }
        }
        if peak > BIN_THRESH {
            run = 0;
        } else {
            run += 1;
            if run >= gap_thresh {
                let end = i + 1 - run;
                if end > start {
                    out.push(Rect::new(
                        r.x + start as i32,
                        r.y,
                        (end - start) as i32,
                        r.h,
                    ));
                }
                start = i + 1;
            }
        }
    }
    if start < n {
        out.push(Rect::new(r.x + start as i32, r.y, (n - start) as i32, r.h));
    }

    out.retain(|b| b.w >= 6);
    if out.is_empty() {
        vec![r]
    } else {
        out
    }
}

/// 8 邻接连通域。用显式栈而不是递归，避免大图上爆栈。
fn connected_components(mask: &[bool], w: usize, h: usize, prob: &[f32]) -> Vec<Component> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::with_capacity(1024);

    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.clear();
        stack.push(start);

        let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0usize, 0usize);
        let mut count = 0usize;
        let mut sum = 0f32;

        while let Some(idx) = stack.pop() {
            let x = idx % w;
            let y = idx / w;
            count += 1;
            sum += prob[idx];
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);

            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(h - 1);
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            for ny in y0..=y1 {
                for nx in x0..=x1 {
                    let ni = ny * w + nx;
                    if mask[ni] && !seen[ni] {
                        seen[ni] = true;
                        stack.push(ni);
                    }
                }
            }
        }

        out.push(Component {
            min_x,
            min_y,
            max_x,
            max_y,
            count,
            sum,
        });
    }
    out
}

/// 阅读顺序：先按行分带，行内再按 x 排。单纯按 (y, x) 排会让
/// 同一行里高一点点的框跑到上一行去。
pub fn sort_reading_order(boxes: &mut Vec<TextBox>) {
    if boxes.is_empty() {
        return;
    }
    boxes.sort_by(|a, b| {
        a.rect
            .center_y()
            .partial_cmp(&b.rect.center_y())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut rows: Vec<Vec<TextBox>> = Vec::new();
    for b in boxes.drain(..) {
        let mut placed = false;
        if let Some(row) = rows.last_mut() {
            let anchor = row[0].rect;
            // 和当前行任意一个框垂直重叠过半，就算同一行
            if row.iter().any(|r| r.rect.vertical_overlap(&b.rect) > 0.5)
                || anchor.vertical_overlap(&b.rect) > 0.5
            {
                row.push(b);
                placed = true;
            }
        }
        if !placed {
            rows.push(vec![b]);
        }
    }

    for row in rows.iter_mut() {
        row.sort_by_key(|b| b.rect.x);
        boxes.extend(row.iter().copied());
    }
}

/// 按矩形裁切，越界部分用白色补齐（比黑色对识别更友好）。
pub fn crop_rect(img: &RgbImage, r: Rect) -> RgbImage {
    let w = r.w.max(1) as u32;
    let h = r.h.max(1) as u32;
    let mut out = RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            let sx = r.x + x as i32;
            let sy = r.y + y as i32;
            if sx >= 0 && sy >= 0 && sx < img.width() as i32 && sy < img.height() as i32 {
                out.put_pixel(x, y, *img.get_pixel(sx as u32, sy as u32));
            }
        }
    }
    out
}

fn num_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
}

/// ort 的错误类型带不可回滚资源，先降级成字符串再交给 anyhow。
pub(crate) fn oe<E: std::fmt::Display>(e: E) -> anyhow::Error {
    anyhow!("{e}")
}
