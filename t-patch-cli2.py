# -*- coding: utf-8 -*-
import io, re
p='src/app/cli.rs'; s=io.open(p,encoding='utf-8').read()

ARM_TAIL = "        return true;" + chr(10) + "    }" + chr(10)

def cut(s, start_pat, end_pat):
    """从 s 剪掉 [start, 臂尾)，返回 (新s, 剪下的段含臂尾)"""
    i0 = s.index(start_pat)
    i1 = s.index(ARM_TAIL, i0) + len(ARM_TAIL)
    return s[:i0] + s[i1:], s[i0:i1]

removed = []

# --tray-menu 臂：到 --shot-window 臂开头
s, seg = cut(s, '    if args.len() >= 2 && args[1] == "--tray-menu" {',
             'ARM_TAIL')
removed.append(seg)

# --shot-window 臂：到 --ocr-test 臂开头
s, seg = cut(s, '    if args.len() >= 3 && args[1] == "--shot-window" {',
             'ARM_TAIL')
removed.append(seg)

# --drag 臂：到 --zoom 臂开头
s, seg = cut(s, '    if args.len() >= 6 && args[1] == "--drag" {',
             'ARM_TAIL')
removed.append(seg)

# --move 臂：到 --clicks 臂开头
s, seg = cut(s, '    if args.len() >= 4 && args[1] == "--move" {',
             'ARM_TAIL')
removed.append(seg)

# --clicks 臂：到 --cursor 臂开头
s, seg = cut(s, '    if args.len() >= 4 && args[1] == "--clicks" {',
             'ARM_TAIL')
removed.append(seg)

# --winlist 臂：到 --wheel 臂开头
s, seg = cut(s, '    if args.len() >= 2 && args[1] == "--winlist" {',
             'ARM_TAIL')
removed.append(seg)

# --wheel 臂：到 --key/--send 臂开头
s, seg = cut(s, '    if args.len() >= 5 && args[1] == "--wheel" {',
             'ARM_TAIL')
removed.append(seg)

# --key/--send 臂：到 --where 臂开头
s, seg = cut(s, '    if args.len() >= 3 && (args[1] == "--key" || args[1] == "--send") {',
             'ARM_TAIL')
removed.append(seg)

# 组装 win32_headless（cfg 门控），插在 run_headless 之前
body = ''.join(removed)
win_headless = (
    '/// Windows 专属的无窗口自测开关（输入注入 / Win32 抓屏 / 托盘消息）。\n'
    '#[cfg(windows)]\n'
    'fn win32_headless(args: &[String]) -> Option<bool> {\n'
    + body +
    '    None\n}\n\n'
)
unix_guard = (
    '#[cfg(not(windows))]\n'
    'fn win32_headless(_args: &[String]) -> Option<bool> {\n'
    '    None\n}\n\n'
)

anchor = 'pub fn run_headless(args: &[String]) -> bool {'
s = s.replace(anchor, unix_guard + win_headless + anchor, 1)

# run_headless 开头加 win32 分发
s = s.replace('''pub fn run_headless(args: &[String]) -> bool {
    if args.len() >= 3 && args[1] == "--ocr-test" {''','''pub fn run_headless(args: &[String]) -> bool {
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
    if args.len() >= 3 && args[1] == "--ocr-test" {''')

# 四个 win32 辅助函数加 #[cfg(windows)]
for fn_name in ('simulate_drag', 'send_key', 'list_app_windows', 'shot_window'):
    s = s.replace(f'\nfn {fn_name}(', f'\n#[cfg(windows)]\nfn {fn_name}(')

# 图标 fns 路径
s = s.replace('std::fs::write(&args[2], tray::icon_ico_bytes()).expect("write ico");',
              'std::fs::write(&args[2], crate::platform::icons::icon_ico_bytes()).expect("write ico");')
s = s.replace('''        let s = tray::icon_size();
        if let Some(img) = image::RgbaImage::from_raw(s, s, tray::icon_rgba()) {''',
'''        let s = crate::platform::icons::icon_size();
        if let Some(img) = image::RgbaImage::from_raw(
            s,
            s,
            crate::platform::icons::icon_rgba(),
        ) {''')

io.open(p,'w',encoding='utf-8').write(s)
print('cli restructured')
