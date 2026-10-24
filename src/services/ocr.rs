//! OCR 引擎的宿主：全局单例 + 懒加载。
//!
//! 模型加载要 ~170ms，识别本身只要 ~200ms，所以引擎必须复用，
//! 否则每次点「文字」都要重新建 ONNX session。

use std::sync::{Mutex, OnceLock};

static ENGINE: OnceLock<Mutex<Option<pixocr::Ocr>>> = OnceLock::new();

/// 同步执行一次识别。调用方负责放到后台线程上。
pub fn run(img: image::RgbImage) -> Result<String, String> {
    let cell = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .map_err(|_| "OCR 引擎状态异常（锁被污染）".to_string())?;

    if guard.is_none() {
        let dir = pixocr::Ocr::default_model_dir();
        if !dir.join("det.onnx").exists() {
            return Err(format!(
                "找不到 OCR 模型，请把 models 目录放到 exe 同级：{}",
                dir.display()
            ));
        }
        eprintln!("[pixpin-rs] loading OCR models from {}", dir.display());
        let t0 = std::time::Instant::now();
        let engine = pixocr::Ocr::load(&dir).map_err(|e| format!("加载 OCR 模型失败: {e}"))?;
        eprintln!(
            "[pixpin-rs] OCR models ready in {}ms",
            t0.elapsed().as_millis()
        );
        *guard = Some(engine);
    }

    let engine = guard.as_mut().unwrap();
    engine.text(&img).map_err(|e| format!("识别失败: {e}"))
}
