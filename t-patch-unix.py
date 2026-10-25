# -*- coding: utf-8 -*-
import io
p='src/platform/stub.rs'; s=io.open(p,encoding='utf-8').read()

def rep(old,new):
    global s
    assert old in s, 'NOT FOUND: '+old[:70]
    s=s.replace(old,new,1)

# capture：导出共享类型
rep('''pub mod capture {
    use super::{MonitorGeom, Shot, UNIMPLEMENTED};''','''pub mod capture {
    pub use super::{MonitorGeom, Shot};
    use super::UNIMPLEMENTED;''')

# tray：导出 TrayCommand
rep('''pub mod tray {
    use super::{TrayCommand, UNIMPLEMENTED};''','''pub mod tray {
    pub use super::TrayCommand;
    use super::UNIMPLEMENTED;''')

# winpick：导出 WinRect
rep('''pub mod winpick {
    use super::WinRect;''','''pub mod winpick {
    pub use super::WinRect;
    use super::UNIMPLEMENTED;''')

io.open(p,'w',encoding='utf-8').write(s)
print('stub exports ok')

# ---- diag.rs：module_of 挂 cfg（filter 已有）
p='src/platform/diag.rs'; s=io.open(p,encoding='utf-8').read()
rep('fn module_of(addr: usize) -> String {','#[cfg(windows)]\nfn module_of(addr: usize) -> String {')
io.open(p,'w',encoding='utf-8').write(s)
print('diag ok')

# ---- config.rs：注册表自启函数挂 cfg + unix 桩
p='src/services/config.rs'; s=io.open(p,encoding='utf-8').read()
rep('pub fn autostart_enabled() -> bool {','#[cfg(windows)]\npub fn autostart_enabled() -> bool {')
rep('''/// 写入/删除 `HKCU\\...\\Run\\PixPin-RS`。返回错误文案给设置界面提示。
pub fn set_autostart(on: bool) -> Result<(), String> {''','''/// 写入/删除 `HKCU\\...\\Run\\PixPin-RS`。返回错误文案给设置界面提示。
#[cfg(windows)]
pub fn set_autostart(on: bool) -> Result<(), String> {''')
# unix 桩
s += '''
/// 非 Windows：开机自启暂未实现（注册表是 Win32 能力）。
#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_autostart(_on: bool) -> Result<(), String> {
    Err("开机自启目前仅 Windows 支持".into())
}
'''
io.open(p,'w',encoding='utf-8').write(s)
print('config ok')

# ---- commands.rs / bootstrap.rs：TrayCommand 引用路径改到平台根
p='src/app/commands.rs'; s=io.open(p,encoding='utf-8').read()
s=s.replace('tray::TrayCommand','crate::platform::TrayCommand')
io.open(p,'w',encoding='utf-8').write(s)
p='src/app/bootstrap.rs'; s=io.open(p,encoding='utf-8').read()
s=s.replace('tray::TrayCommand','crate::platform::TrayCommand')
io.open(p,'w',encoding='utf-8').write(s)
print('commands/bootstrap ok')
