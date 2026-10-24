use std::path::PathBuf;

/// 让 `winresource` 不去查注册表：显式找到 Windows SDK 的 rc.exe。
///
/// 在受限终端 / 沙箱里 `reg.exe` 可能被禁用，`winresource` 查 SDK 根目录
/// 会直接失败。优先尊重外部传入的 `RC_PATH`，否则扫描已安装的 SDK。
fn ensure_rc_path() {
    if std::env::var_os("RC_PATH").is_some() {
        return;
    }
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("WindowsSdkVerBinPath") {
        roots.push(PathBuf::from(path));
    }
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(key) {
            roots.push(
                PathBuf::from(base)
                    .join("Windows Kits")
                    .join("10")
                    .join("bin"),
            );
        }
    }
    for root in roots {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        versions.sort();
        for version in versions.into_iter().rev() {
            let rc = version.join("x64").join("rc.exe");
            if rc.is_file() {
                std::env::set_var("RC_PATH", &rc);
                return;
            }
        }
    }
}

fn main() {
    // exe 内嵌应用图标：资源管理器 / 任务栏固定项 / 快捷方式都用它，
    // 和窗口标题栏、托盘的像素同源（tray.rs 现画）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        ensure_rc_path();
        winresource::WindowsResource::new()
            .set_icon("assets/icons/icon.ico")
            .compile()
            .expect("embed icon resource");
    }
}
