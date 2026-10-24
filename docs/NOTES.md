# 开发笔记：踩过的坑、验收记录、手工验收清单

> 从 README 拆出来的长尾内容，保留原始记录，方便以后翻旧账。

## 几个踩过的坑

**1. GPUI 的纹理格式是 BGRA，不是 RGBA。**
`directx_atlas.rs` 里写死了 `DXGI_FORMAT_B8G8R8A8_UNORM`，而 GDI 的 32 位 DIB 本来就是 BGRA —— 所以抓屏缓冲可以零拷贝直接喂给 `RenderImage`。反过来，从 `image` crate 拿到的 RGBA 在贴图前必须交换 R/B，否则红色变蓝色。

**2. `Pixels` 的字段是私有的。**
不能写 `bounds.size.width.0`，要用 `f32::from(pixels)`。

**3. 覆盖层必须在抓屏之后创建。**
先抓屏再开窗，冻结画面才是「干净」的桌面；顺序反了就会把自己截进去。

**4. 隐藏窗口后要等 DWM 合成完。**
只调 `ShowWindow(SW_HIDE)` 立刻抓屏，会抓到残留的窗口阴影和标题栏按钮 —— 因为 DWM 还没重新合成。加 `DwmFlush()` + 200ms 才干净。

**5. `cx.spawn` 用 async 闭包。**
`cx.spawn(async move |this, cx| { ... })`，参数是 async 闭包的入参，可以跨 `await` 存活；写成 `move |cx| async move { ... }` 会报生命周期错误。

**6. 坐标系统一用逻辑像素。**
鼠标事件、选区、标注全存逻辑像素，只在导出时乘 `scale` 换成物理像素，避免两套坐标互相污染。

**7. 托盘图标的位图是 BGRA，写反了会变洋红。**
`CreateIcon` 吃的 XOR 位图和 `BitBlt` 的 DIB 一样是 BGRA 序。按 RGBA 顺序写 `[r,g,b,a]`，蓝色 `#0A84FF` 会渲染成紫色。必须写成 `[b,g,r,a]`。

**8. 托盘必须有自己的消息泵线程。**
GPUI 的事件循环不会替你泵 `Shell_NotifyIconW` 的回调消息。所以 `tray.rs` 里单开一个线程：`RegisterClassW` → `CreateWindowExW`（普通隐藏顶层窗口，不是 `HWND_MESSAGE`）→ `NIM_ADD` → `GetMessageW` 循环。菜单项通过 `WM_COMMAND` 回到这个 `wndproc`，再经 `mpsc` 交给 GPUI 线程。

**9. `SetForegroundWindow` 会静默失败。**
从托盘溢出区点开截图时，前台进程是 explorer，我们的进程没有前台权，`SetForegroundWindow` 直接返回失败 —— 覆盖层拿不到键盘焦点，Esc / Enter 全是死的，看起来像「快捷键失灵」。解法是先 `AttachThreadInput` 挂到前台线程的输入队列，再 `SetForegroundWindow` + `SetFocus`，之后解除挂接。

**10. 无边框窗口拖动不能用 `WM_NCLBUTTONDOWN` + `HTCAPTION`。**
一行就能跑起来，但它会在 GPUI 的事件派发里开一个嵌套模态循环，短距离拖没事，长距离拖会把进程直接拖死（无崩溃日志、无 panic）。改成独立线程轮询 `GetAsyncKeyState(VK_LBUTTON)` + `SetWindowPos`，完全不碰 GPUI 的循环。

**11. 贴图窗口要显式置顶。**
`WindowKind::PopUp` + `focus: false` 建出来的窗口虽然 `IsWindowVisible` 为真，但会被普通窗口盖住 —— 现象是「贴图成功了但看不见」。创建时补一个 `SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE)` 才是贴图该有的行为。

**12. 「便携」不等于「单文件」。**
一开始想用 `RUSTFLAGS="-C target-feature=+crt-static"` 把 VC++ 运行库链进去，
但 onnxruntime 的静态库是按 `/MD` 编的，`+crt-static` 下 67 个 `__imp_*` 符号全部无法解析。
只能接受动态 CRT，把 `vcruntime140.dll` / `msvcp140.dll` 四个文件随包带上 —— 对用户仍然是
「解压即用」，只是目录里多了几个 dll。另外 exe 同级存在 `portable.txt` 才把截图写进
`.\screenshots\`，拔掉 U 盘就走，不留痕迹。

**13. 点工具栏按钮点不准，是因为 DPI 不是 150%。**
自动化点击时按 1.5 倍算坐标，结果整个工具栏偏了一个按钮。别用「大概 150%」推，
要么按 `GetDpiForMonitor` 算，要么让程序把实际点中的按钮编号打出来（`[pixpin-rs] toolbar button 8`）。
这台机器实测工具栏左边缘在 505 物理像素，按 1.5 算正好一次点中。

**14. `ort` 的版本必须精确锁死。**
`ort = "2.0.0-rc.12"` 是范围匹配，会解析到 rc.15，而 rc.15 把
`Session::builder()` 改成了需要 `&Environment` 参数，报 37 个编译错误。
写 `ort = "=2.0.0-rc.12"` 才对。

**15. `ort::Error` 不是 `Send + Sync`。**
rc.12 起错误类型变成泛型 `Error<R = ()>`，里面可能带着 `SessionBuilder` 这类不可回滚的资源，
直接丢进 `anyhow` 会编译不过。得先降级成字符串：

```rust
pub(crate) fn oe<E: std::fmt::Display>(e: E) -> anyhow::Error { anyhow!("{e}") }
```

**16. `AsyncApp` 不实现 `VisualContext`。**
在后台任务里更新 UI，`cx.spawn(...)` 会报 `AsyncApp: gpui::VisualContext is not satisfied`。
要用 `cx.spawn_in(window, ...)` 拿到 `AsyncWindowContext`，再 `this.update_in(cx, |this, window, cx| ...)`。

**17. 结果窗口被覆盖层盖住，且行距被拉成 70px。**
两个独立问题。前者是开窗顺序：必须 `this.close(window)` 先收掉覆盖层，再开结果窗，
并在结果窗里 `force_foreground`。后者是布局：把每一行文字塞成独立的 `div` 放进
flex column，会被拉伸成等高 —— 改成整段文本交给单个文本元素 + `overflow_y_scroll()` 才正常。

**18. 窗口识别永远命中桌面 —— 元凶是 `ShellHandwritingCanvas`。**
覆盖层自己盖住全屏，所以不能用 `WindowFromPoint`（只会返回自己），改成按 z 序遍历顶层窗口。
但第一版跑出来整个屏幕都不变暗：`winpick` 报回来一个 `ShellHandwritingCanvas {GUID}`
的窗口，`rect=(0,0 2560x1600)` —— Windows Ink 的手写画布，全屏、可见、不属于本进程，
于是遮罩给它「挖了个洞」，看起来就像遮罩没生效。把 shell 窗口按**类名前缀**排掉
（`Progman` / `WorkerW` / `Shell_TrayWnd` / `ShellHandwritingCanvas` …）才正常。
用前缀而不是全等，是因为这个类名后面还跟一个 GUID。

**19. 单击选窗口要记「按下时」的窗口，不能记「松开时」的。**
`on_down` 里为了开始新框选会清空 `hover_win`，等 `on_up` 再读就已经是 `None` 了。
加一个 `press_win` 字段，在 `on_down` 的第一时间快照下来。

**20. 工具栏翻到选区上方时会和尺寸徽标打架。**
工具栏默认贴在选区下方，下方放不下就翻到上方 —— 但徽标是固定在选区左上角外侧的，
翻上去之后两者重叠，徽标被工具栏盖住。判断一下工具栏在上还是在下，在上就把徽标收进选区内部。

**21. 双击启动后热键「不生效」，其实是覆盖层开在了控制台窗口后面。**
exe 是 console 子系统，双击时 Windows 会给它配一个黑色控制台窗口，标题就是 exe 路径。
这个窗口停在最上层、还占着前台，于是：热键正常触发 → 抓屏正常 → 覆盖层也建出来了，
但它开在控制台后面，用户看到的就是「按了没反应」。而点托盘图标那一刻，输入事件让本进程
拿到了前台权，覆盖层才终于露出来 —— 于是表现为「点托盘之后才生效」。
修法两条：启动时用 `GetConsoleProcessList` 判断控制台是不是自己独有的（返回 1 就是双击
启动的），是就 `FreeConsole()` 收掉；同时覆盖层改用「`SetWindowPos(HWND_TOPMOST)` +
`AttachThreadInput` + `SetForegroundWindow`」三重保险，抢不到焦点也至少看得见。

**22. 用 `keybd_event` 注入的组合键永远触发不了 `RegisterHotKey`。**
排查 21 的时候一度以为热键注册坏了：`RegisterHotKey` 返回 true，但消息泵一行日志都没有。
原因是测试脚本走的是 `keybd_event`，它注入的按键不会进到全局热键的匹配里。
换成 `SendInput` 立刻就能收到 `WM_HOTKEY`。所以自测留了两个开关：`--send`（SendInput，
能触发热键）和 `--key`（keybd_event，只能验证普通按键）。
顺带一个坑：`GetMessageW` 返回 0（WM_QUIT）和 -1（出错）都要退泵，
原来的 `while GetMessageW(...).as_bool()` 会把 -1 也当成「有消息」继续转。

**23. 窗口刚 `open_window` 出来时抢前台一定失败。**
构建闭包执行的那一刻窗口还没真正 map，`SetForegroundWindow` 打上去会被系统忽略。
要 `window.defer(cx, |window, _cx| ...)` 推到下一帧再抢一次；另外 GPUI 自带的
`window.activate_window()` 会模拟一次 Alt 按键，让本进程变成「最后收到输入的进程」，
这是绕前台锁最稳的一招，和 `AttachThreadInput` 一起用基本没有抢不到的情况。

**24. 用 PowerShell 查窗口坐标做自动化点击，拿到的是 DPI 虚拟化坐标。**
`GetWindowRect` / `GetClientRect` 返回的是调用进程坐标空间里的值——PowerShell 默认
不是 DPI-aware，在 150% 屏上拿到的宽高和位置都是物理值 ÷ 1.5，按它注入鼠标点击全部
偏到窗口外，现象是「事件处理函数根本不触发」。要么给 PS 进程声明 DPI aware，
要么把查到的坐标 × 缩放比再用。验证坐标是否对得上：在 mouse handler 里 trace
`ev.position`（GPUI 逻辑像素，= 物理客户区偏移 ÷ 缩放），反推窗口真实位置。

**25. 气泡通知要在托盘就位之后再发。**
`Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)` 借的是已注册的托盘图标；启动时先弹气泡再建托盘，
`TRAY_HWND` 还是空，函数静默早退 —— trace 里一条不落，看起来像「代码没执行」。
顺序固定为：建托盘（阻塞等 ready）→ 再发气泡。另外通知是否在屏幕上冒出来由系统
通知策略决定（勿扰模式直送通知中心），验证时用 Win+N 打开通知中心看。

**26. 取字结果的选择交互是自己画的，别想着用 div 文本元素凑合。**
`shape_text` 一次拿到全部 `WrappedLine`，命中测试走
`closest_index_for_position`、高亮走 `position_for_index`，字节索引贯穿始终。
三个坑：点击点在最后一行下方时 gpui 的 `_index_for_position` 会返回 `Err(0)`
把索引跳回段首，必须先把 y 压回该段的最后一行；`on_mouse_up` 挂在文本区域上，
鼠标在区域外松开收不到，要用 `on_mouse_up_out` 兜底（`on_mouse_move` 里检查
`pressed_button` 也行）；右键菜单用普通 div 浮层时，后面的行会画在浮层之上，
要么把浮层挂到容器末尾、要么 `.occlude()` 挡住底层 hitbox。

## 验收记录

- `assets/screenshots/overlay.png` —— 覆盖层实拍：遮罩、白框 + 八手柄、尺寸徽标、四种标注、图标工具栏、放大镜
- `assets/screenshots/toolbar.png` —— 工具栏 1:1 实拍：五个标注工具 + 分隔线 + 七个动作，撤销/重做置灰
- `assets/screenshots/export-demo.png` —— 导出的成品：裁剪正确、红框/黄箭头/绿画笔/马赛克栅格化正确、颜色无 R/B 反转
- `assets/icons/icon.png` —— 托盘图标 8 倍放大：蓝色圆角方块 + 白色取景角标，配色正确
- （托盘溢出区实拍 `tray-flyout.png` 已归档，未随仓库保留）
- `assets/screenshots/tray-menu.png` —— 右键菜单：截图 / 显示主窗口 / 退出
- `assets/screenshots/pin-window.png` —— 贴图窗口（蓝框是窗口边界）；内容是自测用的渐变色块，专门验证置顶、拖动和滚轮缩放
- `assets/screenshots/ocr-window.png` —— 取字结果窗口：三段正文按段落还原，页脚提示已复制到剪贴板
- （设置窗口截图 `settings-window.png` 当时截错了内容，已删除，待重拍）

## 手工验收清单

1. 启动 → 没有任何窗口弹出，托盘出现蓝色取景框图标（托盘即主入口）
2. 左键点图标 → 全屏冻结覆盖层，直接按 `Esc` 能取消（不用先点一下）
3. 右键点图标 → 菜单三项（截图 / 设置… / 退出）都能用；`设置…` 重复点只置顶同一窗口；`退出` 后进程干净结束
4. 设置窗口开着时发起截图 → 设置窗自动隐藏，Esc 收起覆盖层后设置窗自动回来
5. 悬停某个窗口 → 该窗口高亮、其余变暗；单击 → 整窗被选中，徽标显示窗口坐标和尺寸
6. 框选后拖任一 corner 手柄 → 尺寸实时变化；按住选区内部拖 → 整体平移
7. 光标移到彩色区域 → 放大镜里能看到像素网格，读数面板跟着变
8. 选「马赛克」→ 拖一块 → 该区域变成色块；`Ctrl+Z` 撤回
9. 截图后按贴图 → 图浮在所有窗口之上，按住左键能拖，右键关掉
10. 截图后点「取字」→ 0.2 秒内弹结果窗，内容正确且已进剪贴板，`Esc` 关窗
11. 便携版：`dist\PixPin-RS\` 拷到别处 → 双击 → 截图 → `Ctrl+S` → 文件出现在同级 `screenshots\`
12. 便携版：删掉 `models\` → 点「取字」应给出「找不到 OCR 模型」的提示，而不是崩溃
13. **双击启动**（不是在终端里跑）→ 不应出现黑色控制台窗口，主面板直接在最前面
14. **双击启动后立刻按 `Ctrl+Alt+X`**（不要先点托盘）→ 覆盖层应该马上铺满全屏
15. 取字结果窗口弹出时，它应该压在工作窗口之上；不点它直接按 `Esc` 能关掉（说明拿到了焦点）

### 便携版实测记录

- `PixPin-RS.exe --where` → `D:\dev\pixpin-rs\dist\PixPin-RS\screenshots`，`portable: true`
- `PixPin-RS.exe --ocr-test test-render.png` → 216 字，模型从 `dist\PixPin-RS\models` 加载，178 ms
- 热键截图 → 框选 800×550 → 点「保存」→ `screenshots\PixPinRS_1792586892993.png` 落盘，内容与选区一致
- 热键截图 → 框选正文 → 点「取字」→ 303 字，弹窗正常，剪贴板可粘贴
- 单击 WorkBuddy 窗口 → 徽标读出 `378,135 1815 × 1215 px`，与 `DwmGetWindowAttribute` 一致
- 拖右下角手柄 2200,1450 → 2350,1520 → 徽标从 `700 × 450` 变成 `850 × 520`，工具栏跟着翻到上方

OCR 实测耗时（Release，8 线程）：

| 输入 | 检测 | 识别 | 合计 |
|---|---|---|---|
| 900×520 截图选区 | 83 ms | 126 ms | 210 ms |
| 2560×1600 全屏 | 133 ms | 1037 ms | 1170 ms |

模型加载 143~178 ms，只在第一次点「取字」时发生。

**版面还原是规则版，不是模型。** 段落判断靠行距、左边界、字号三个阈值，
所以在「一列短标签」这种场景会过度合并 —— 比如把一串工具调用记录
（`已读取 xxx.png` / `深度思考` 各占一行）当成同一段拼成一长句。
正文类文档表现正常，UI 截图取字时建议一次框小一点。
