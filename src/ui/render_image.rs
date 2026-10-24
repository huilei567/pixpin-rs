//! Conversions from raw pixel buffers into GPUI textures.
//!
//! GPUI's DirectX atlas uses `DXGI_FORMAT_B8G8R8A8_UNORM`, so the byte order
//! handed to `RenderImage` must be BGRA regardless of how we think about colors.

use std::sync::Arc;

use gpui::RenderImage;

/// Already BGRA (straight from GDI) — no channel work needed.
pub fn from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Arc<RenderImage> {
    let buffer =
        image::RgbaImage::from_raw(width, height, bgra).expect("bgra buffer size mismatch");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// RGBA (e.g. from `image` crate) — swap red/blue on the way in.
pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Arc<RenderImage> {
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    from_bgra(width, height, rgba)
}
