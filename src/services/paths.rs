//! 应用数据目录的判定。
//!
//! `portable.txt` 存在时，截图和配置都跟着 exe 走；否则写用户目录。
//! 这个规则被截图保存、设置窗口和 CLI 自检共用，所以放在 services 层，
//! 不能继续挂在 `main.rs` 上。

use std::path::PathBuf;

/// Where saved screenshots land.
pub fn save_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() {
                return dir.join("screenshots");
            }
        }
    }
    std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures").join("PixPinRS"))
        .unwrap_or_else(|_| std::env::temp_dir())
}
