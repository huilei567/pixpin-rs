//! 应用设置的持久化。
//!
//! 设置存成 JSON：便携模式（exe 旁有 `portable.txt`）放在 exe 同级，
//! 否则放 `%APPDATA%\PixPinRS\config.json`。文件缺失或损坏一律回落默认值，
//! 绝不让设置问题挡住主流程。
//!
//! 开机自启不进 JSON —— 注册表 `HKCU\...\Run` 才是权威，这样用户在
//! 任务管理器里手动删掉启动项后，设置界面读到的状态也是对的。

use serde::{Deserialize, Serialize};

const VALUE_NAME: &str = "PixPin-RS";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// 截图状态下右键取消（Esc 永远可用）
    pub rc_cancel_capture: bool,
    /// OCR 完成后自动把全文写进剪贴板
    pub ocr_auto_copy: bool,
    /// OCR 结果窗口的字号（逻辑像素）
    pub ocr_font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rc_cancel_capture: true,
            ocr_auto_copy: true,
            ocr_font_size: 13.0,
        }
    }
}

/// 便携模式跟随截图目录的判定：exe 旁有 `portable.txt` 就把配置也放在 exe 旁。
fn path() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() {
                return dir.join("pixpin-rs.json");
            }
        }
    }
    std::env::var("APPDATA")
        .map(|p| {
            std::path::PathBuf::from(p)
                .join("PixPinRS")
                .join("config.json")
        })
        .unwrap_or_else(|_| std::env::temp_dir().join("pixpin-rs-config.json"))
}

pub fn load() -> Config {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn store(config: &Config) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&p, s);
    }
}

/// 读取当前设置（总是从磁盘读，量小且调用频率低，不值得再做一层缓存）。
pub fn get() -> Config {
    load()
}

// ---- 开机自启（HKCU Run） ----

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };
    unsafe {
        let mut hk = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            None,
            KEY_READ,
            &mut hk,
        ) != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return false;
        }
        let ok = RegQueryValueExW(hk, &HSTRING::from(VALUE_NAME), None, None, None, None)
            == windows::Win32::Foundation::ERROR_SUCCESS;
        let _ = RegCloseKey(hk);
        ok
    }
}

/// 写入/删除 `HKCU\...\Run\PixPin-RS`。返回错误文案给设置界面提示。
#[cfg(windows)]
pub fn set_autostart(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };
    unsafe {
        let key = windows::core::HSTRING::from(RUN_KEY);
        let name = windows::core::HSTRING::from(VALUE_NAME);
        let mut hk = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &key, None, KEY_SET_VALUE, &mut hk)
            != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return Err("打开注册表 Run 键失败".into());
        }
        let result = if on {
            let exe = std::env::current_exe().map_err(|e| format!("取 exe 路径失败: {e}"))?;
            let cmd = format!("\"{}\"", exe.display());
            let mut bytes: Vec<u8> = cmd.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            bytes.extend_from_slice(&[0, 0]); // UTF-16 结尾
            RegSetValueExW(hk, &name, None, REG_SZ, Some(&bytes))
                == windows::Win32::Foundation::ERROR_SUCCESS
        } else {
            let err = RegDeleteValueW(hk, &name);
            err == windows::Win32::Foundation::ERROR_SUCCESS
                || err == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        };
        let _ = RegCloseKey(hk);
        if result {
            Ok(())
        } else {
            Err("写入注册表失败".into())
        }
    }
}

/// 非 Windows：开机自启暂未实现（注册表是 Win32 能力）。
#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_autostart(_on: bool) -> Result<(), String> {
    Err("开机自启目前仅 Windows 支持".into())
}
