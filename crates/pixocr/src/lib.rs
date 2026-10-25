//! pixocr —— 纯 Rust 的本地 OCR 引擎。
//!
//! 三段式，和 PixPin 的 `PixOCR2.dll` 同构：
//! 1. [`det`]  DBNet 检测，输出一行一个框
//! 2. [`rec`]  CTC 识别，输出每行文本
//! 3. [`layout`] 规则版版面还原，输出段落
//!
//! 推理后端是 ONNX Runtime，模型用 PP-OCRv5 的 mobile 版本，
//! 所以模型本身是「可替换件」——换成 server 版或别的语言版只需要换文件。

pub mod det;
pub mod geom;
pub mod layout;
pub mod rec;

use anyhow::{Context, Result};
use image::RgbImage;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub use geom::Rect;
pub use layout::{Block, Line};

/// 一次识别的统计信息，方便在 UI 上显示耗时。
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub det_ms: u128,
    pub rec_ms: u128,
    pub boxes: usize,
}

pub struct Ocr {
    det: det::Detector,
    rec: rec::Recognizer,
}

impl Ocr {
    /// `dir` 下需要有 `det.onnx`、`rec.onnx`、`ppocrv5_dict.txt`。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let det_path = dir.join("det.onnx");
        let rec_path = dir.join("rec.onnx");
        let dict_path = dir.join("ppocrv5_dict.txt");
        for p in [&det_path, &rec_path, &dict_path] {
            if !p.exists() {
                anyhow::bail!("缺少模型文件: {}", p.display());
            }
        }
        Ok(Self {
            det: det::Detector::load(&det_path)?,
            rec: rec::Recognizer::load(&rec_path, &dict_path)?,
        })
    }

    /// 模型目录的默认查找顺序：exe 同级 `models/` → 当前目录 `models/`。
    pub fn default_model_dir() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let p = parent.join("models");
                if p.join("det.onnx").exists() {
                    return p;
                }
            }
        }
        PathBuf::from("models")
    }

    pub fn dict_len(&self) -> usize {
        self.rec.dict_len()
    }

    /// 完整链路：检测 → 逐框识别 → 版面还原。
    pub fn recognize(&mut self, img: &RgbImage) -> Result<Vec<Block>> {
        self.recognize_profiled(img).map(|(b, _)| b)
    }

    pub fn recognize_profiled(&mut self, img: &RgbImage) -> Result<(Vec<Block>, Stats)> {
        let mut stats = Stats::default();

        let t0 = Instant::now();
        let boxes = self.det.detect(img).context("检测阶段失败")?;
        stats.det_ms = t0.elapsed().as_millis();
        stats.boxes = boxes.len();

        let t1 = Instant::now();
        let mut lines = Vec::with_capacity(boxes.len());
        for b in &boxes {
            let crop = det::crop_rect(img, b.rect);
            if crop.width() < 2 || crop.height() < 2 {
                continue;
            }
            match self.rec.recognize(&crop) {
                Ok((text, score)) => {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        lines.push(Line {
                            text,
                            score,
                            rect: b.rect,
                        });
                    }
                }
                // 单行失败不该毁掉整次识别
                Err(e) => eprintln!("[pixocr] 识别失败 @ {:?}: {e}", b.rect),
            }
        }
        stats.rec_ms = t1.elapsed().as_millis();

        Ok((layout::group(lines), stats))
    }

    /// 只要纯文本，段落之间空一行。
    pub fn text(&mut self, img: &RgbImage) -> Result<String> {
        let blocks = self.recognize(img)?;
        Ok(blocks
            .iter()
            .map(|b| b.text())
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}
