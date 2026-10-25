# -*- coding: utf-8 -*-
import io
p='src/app/cli.rs'; s=io.open(p,encoding='utf-8').read()

def rep(old,new):
    global s
    assert old in s, 'NOT FOUND: '+old[:70]
    s=s.replace(old,new,1)

# ---- 1) 图标 fns 移到了 platform::icons
rep('''        std::fs::write(&args[2], tray::icon_ico_bytes()).expect("write ico");''',
    '''        std::fs::write(&args[2], crate::platform::icons::icon_ico_bytes()).expect("write ico");''')
rep('''        let s = tray::icon_size();
        if let Some(img) = image::RgbaImage::from_raw(s, s, tray::icon_rgba()) {''',
    '''        let s = crate::platform::icons::icon_size();
        if let Some(img) =
            image::RgbaImage::from_raw(s, s, crate::platform::icons::icon_rgba())
        {''')

# ---- 2) win32 专属自测开关抽到 win32_headless（cfg 门控）
# 从 run_headless 里剪切这些臂：--tray-menu / --shot-window / --drag /
# --move / --clicks / --winlist / --wheel / --key(--send)
rep('''pub fn run_headless(args: &[String]) -> bool {
    if args.len() >= 2 && args[1] == "--tray-menu" {''','''pub fn run_headless(args: &[String]) -> bool {
    #[cfg(windows)]
    if let Some(handled) = win32_headless(args) {
        return handled;
    }
    #[cfg(not(windows))]
    if args.len() >= 2
        && matches!(
            args[1].as_str(),
            "--tray-menu"
                | "--shot-window"
                | "--drag"
                | "--move"
                | "--clicks"
                | "--winlist"
                | "--wheel"
                | "--key"
                | "--send"
        )
    {
        eprintln!("[pixpin-rs] {} 仅 Windows 支持（输入注入 / Win32 抓屏）", args[1]);
        return true;
    }
    if args.len() >= 2 && args[1] == "--tray-menu" {''')

io.open(p,'w',encoding='utf-8').write(s)
print('cli part1 ok')
