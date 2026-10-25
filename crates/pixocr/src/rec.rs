//! CTC 文字识别：裁切图 → 定高缩放 → 前向 → argmax + 去重 + 去 blank。

use crate::det::oe;
use anyhow::{anyhow, Result};
use image::{imageops, RgbImage};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::path::Path;

/// PP-OCR 系列识别网络固定高度 48。
const REC_H: u32 = 48;
/// 宽度上限，超过就压扁（极长的一行本来就该切开）。
const REC_MAX_W: u32 = 1600;

pub struct Recognizer {
    session: Session,
    dict: Vec<String>,
    /// 模型若声明了固定宽度就按固定宽度走，否则按内容自适应。
    fixed_w: Option<usize>,
}

impl Recognizer {
    pub fn load(model: &Path, dict_path: &Path) -> Result<Self> {
        let session = Session::builder()
            .map_err(oe)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(oe)?
            .with_intra_threads(num_threads())
            .map_err(oe)?
            .commit_from_file(model)
            .map_err(|e| anyhow!("加载识别模型失败 {}: {e}", model.display()))?;

        let fixed_w = probe_fixed_width(&session);

        let raw = std::fs::read_to_string(dict_path)
            .map_err(|e| anyhow!("读取字典失败 {}: {e}", dict_path.display()))?;
        let dict: Vec<String> = raw
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if dict.is_empty() {
            anyhow::bail!("字典为空: {}", dict_path.display());
        }

        Ok(Self {
            session,
            dict,
            fixed_w,
        })
    }

    pub fn dict_len(&self) -> usize {
        self.dict.len()
    }

    /// 返回 (文本, 置信度)。
    pub fn recognize(&mut self, crop: &RgbImage) -> Result<(String, f32)> {
        let (cw, ch) = (crop.width().max(1), crop.height().max(1));
        let ratio = cw as f32 / ch as f32;
        let target_w = match self.fixed_w {
            Some(w) => w as u32,
            None => ((REC_H as f32 * ratio).round() as u32).clamp(16, REC_MAX_W),
        };

        // 等比缩放到高度 48，宽度不足固定宽度时右侧补 0（像素值 0 → 归一化 -1）
        let scaled_w = ((REC_H as f32 * ratio).round() as u32).clamp(1, target_w);
        let scaled = imageops::resize(crop, scaled_w, REC_H, imageops::FilterType::Triangle);

        let plane = (target_w * REC_H) as usize;
        let mut data = vec![0f32; 3 * plane];
        for y in 0..REC_H as usize {
            for x in 0..scaled_w as usize {
                let p = scaled.get_pixel(x as u32, y as u32);
                let idx = y * target_w as usize + x;
                data[idx] = (p[0] as f32 / 255.0 - 0.5) / 0.5;
                data[plane + idx] = (p[1] as f32 / 255.0 - 0.5) / 0.5;
                data[2 * plane + idx] = (p[2] as f32 / 255.0 - 0.5) / 0.5;
            }
        }

        let input = Tensor::from_array((
            [1usize, 3, REC_H as usize, target_w as usize],
            data,
        ))
        .map_err(|e| anyhow!("构造识别输入张量失败: {e}"))?;
        let outputs = self
            .session
            .run(ort::inputs!["x" => input])
            .map_err(|e| anyhow!("识别模型推理失败: {e}"))?;
        let (shape, logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("读取识别输出失败: {e}"))?;

        let dims: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
        // 输出 [1, T, C]
        let (steps, classes) = match dims.len() {
            3 => (dims[1], dims[2]),
            2 => (dims[0], dims[1]),
            _ => anyhow::bail!("识别输出维度异常: {:?}", dims),
        };
        if steps == 0 || classes < 2 {
            anyhow::bail!("识别输出尺寸异常: {:?}", dims);
        }

        let mut text = String::new();
        let mut prev = usize::MAX;
        let mut score_sum = 0f32;

        for t in 0..steps {
            let row = &logits[t * classes..(t + 1) * classes];
            let (idx, prob) = softmax_argmax(row);
            score_sum += prob;
            if idx != 0 && idx != prev {
                match self.dict.get(idx - 1) {
                    Some(ch) => text.push_str(ch),
                    // 字典末尾那个类就是空格
                    None => text.push(' '),
                }
            }
            prev = idx;
        }

        let score = score_sum / steps as f32;
        Ok((text, score))
    }
}

/// 取 argmax 并给出概率。
///
/// PaddleOCR 导出的识别模型末尾已经带了 softmax，输出直接就是概率；
/// 但也有版本导出的是原始 logits，所以这里判断一次再决定要不要重算。
fn softmax_argmax(row: &[f32]) -> (usize, f32) {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, v) in row.iter().enumerate() {
        if *v > best_v {
            best_v = *v;
            best = i;
        }
    }

    let sum: f32 = row.iter().sum();
    let already_prob = row.iter().all(|v| *v >= -1e-6 && *v <= 1.0 + 1e-6)
        && (sum - 1.0).abs() < 0.05;

    let prob = if already_prob {
        best_v.clamp(0.0, 1.0)
    } else {
        let max = best_v;
        let mut denom = 0f32;
        for v in row {
            denom += (v - max).exp();
        }
        if denom > 0.0 { 1.0 / denom } else { 0.0 }
    };
    (best, prob)
}

fn probe_fixed_width(session: &Session) -> Option<usize> {
    let input = session.inputs().first()?;
    if let ort::value::ValueType::Tensor { shape, .. } = input.dtype() {
        let d = shape.as_ref();
        if let Some(w) = d.get(3) {
            if *w > 0 {
                return Some(*w as usize);
            }
        }
    }
    None
}

fn num_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
}
