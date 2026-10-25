//! 全屏冻结覆盖层：框选 → 标注 → 复制 / 保存 / 贴图 / 取字。
//!
//! 交互和视觉参照 PixPin：
//!
//! * 光标旁一个放大镜，实时显示像素网格、坐标和色值
//! * 选区八个方向手柄，可以拖拽改尺寸，按住在内部拖可以整体移动
//! * 没有选区时悬停高亮光标下的窗口，单击直接选中它
//! * 工具栏是白色圆角条 + 矢量图标，左半是标注工具、右半是动作，中间一道分隔线
//! * 工具栏贴着选区右下角，下方放不下就翻到上方

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, hsla, img, point, px, quad, rgb, size, App, Bounds, Context, ElementId,
    FocusHandle, Font, FontFeatures, FontStyle, FontWeight, Hsla, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PathBuilder, Pixels,
    Point, Render, RenderImage, SharedString, TextAlign, TextRun, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};

use crate::core::annotation::{self as paint, Shape};
use crate::core::geom::RectF;
use crate::platform::capture::Shot;
use crate::platform::winpick::{self, WinRect};
use crate::ui::icons::{paint_icon, Icon};
use crate::ui::render_image as render_img;

mod actions;
mod input;
mod magnifier;
mod render;
mod toolbar;

// ---------- 工具栏尺寸（逻辑像素） ----------

const TB_H: f32 = 44.0;
const TB_PAD: f32 = 5.0;
const BTN: f32 = 34.0;
const SEP_W: f32 = 9.0;
const TB_RADIUS: f32 = 10.0;
const TB_GAP: f32 = 8.0;

const HANDLE: f32 = 7.0;
const HANDLE_TOL: f32 = 7.0;

const LOUPE: f32 = 99.0;
const LOUPE_CELLS: i32 = 11;
const PANEL_W: f32 = 150.0;

/// 编辑贴图时画布的最小逻辑尺寸。
///
/// 工具栏本身有 504 逻辑像素宽，贴图比它窄的话按钮会被窗口裁掉、点不到，
/// 所以二次编辑时把画布补到至少这么宽，多出来的部分画成深灰底。
pub const MIN_EDIT_W: f32 = 540.0;
pub const MIN_EDIT_H: f32 = 120.0;

/// 标注工具在前，动作在后，中间一道分隔线 —— 和 PixPin 的分区一致。
const BUTTONS: [(Icon, &str); 14] = [
    (Icon::Rect, "矩形"),
    (Icon::Ellipse, "椭圆"),
    (Icon::Pen, "画笔"),
    (Icon::Arrow, "箭头"),
    (Icon::Text, "文本"),
    (Icon::Mosaic, "马赛克"),
    (Icon::Eraser, "橡皮擦"),
    (Icon::Undo, "撤销"),
    (Icon::Redo, "重做"),
    (Icon::Ocr, "取字"),
    (Icon::Pin, "贴图"),
    (Icon::Save, "保存"),
    (Icon::Copy, "复制"),
    (Icon::Close, "关闭"),
];

/// 带二级菜单（色板 / 线宽 / 块大小）的工具按钮。
const MENU_BTNS: [usize; 6] = [0, 1, 2, 3, 4, 5];
const SV_H: f32 = 96.0;
const HUE_H: f32 = 14.0;
const HEX_H: f32 = 18.0;
const MENU_ROW_H: f32 = 30.0;
const MENU_PAD: f32 = 9.0;
/// 滑杆可调范围（逻辑像素）：线宽 / 马赛克块 / 涂抹笔刷 / 序号圆半径。
const WIDTH_RANGE: (f32, f32) = (1.0, 20.0);
const BLOCK_RANGE: (f32, f32) = (4.0, 40.0);
const BRUSH_RANGE: (f32, f32) = (6.0, 60.0);
const TEXTSZ_RANGE: (f32, f32) = (12.0, 64.0);

/// 菜单里可拖动的控件。
#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuDrag {
    Slider(Slider),
    Hue,
    SatVal,
}

/// 菜单里的滑杆种类。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Slider {
    Width,
    Block,
    Brush,
    TextSize,
}

impl Slider {
    fn range(self) -> (f32, f32) {
        match self {
            Slider::Width => WIDTH_RANGE,
            Slider::Block => BLOCK_RANGE,
            Slider::Brush => BRUSH_RANGE,
            Slider::TextSize => TEXTSZ_RANGE,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Slider::Width => "线宽",
            Slider::Block => "块大小",
            Slider::Brush => "笔刷",
            Slider::TextSize => "字号",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Btn(usize),
    Sep,
}

const ITEMS: [Item; 16] = [
    Item::Btn(0),
    Item::Btn(1),
    Item::Btn(2),
    Item::Btn(3),
    Item::Btn(4),
    Item::Btn(5),
    Item::Btn(6),
    Item::Sep,
    Item::Btn(7),
    Item::Btn(8),
    Item::Sep,
    Item::Btn(9),
    Item::Btn(10),
    Item::Btn(11),
    Item::Btn(12),
    Item::Btn(13),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    None,
    Rect,
    Ellipse,
    Pen,
    Arrow,
    Text,
    Mosaic,
    Eraser,
}

impl Tool {
    fn from_button(i: usize) -> Option<Tool> {
        match i {
            0 => Some(Tool::Rect),
            1 => Some(Tool::Ellipse),
            2 => Some(Tool::Pen),
            3 => Some(Tool::Arrow),
            4 => Some(Tool::Text),
            5 => Some(Tool::Mosaic),
            6 => Some(Tool::Eraser),
            _ => None,
        }
    }

    fn button(&self) -> Option<usize> {
        match self {
            Tool::Rect => Some(0),
            Tool::Ellipse => Some(1),
            Tool::Pen => Some(2),
            Tool::Arrow => Some(3),
            Tool::Text => Some(4),
            Tool::Mosaic => Some(5),
            Tool::Eraser => Some(6),
            Tool::None => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Handle {
    TL,
    T,
    TR,
    R,
    BR,
    B,
    BL,
    L,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    /// 拖出一块新选区。
    New,
    Move {
        grab: (f32, f32),
        orig: RectF,
    },
    Resize {
        h: Handle,
        orig: RectF,
    },
}

pub struct Overlay {
    shot: Arc<Shot>,
    image: Arc<RenderImage>,
    scale: f32,
    focus: FocusHandle,

    tool: Tool,
    sel: Option<RectF>,
    /// 光标下的窗口（覆盖层本地逻辑坐标），仅在没有选区时更新。
    hover_win: Option<RectF>,

    drag: Drag,
    press: (f32, f32),
    /// 按下瞬间光标下的窗口 —— 单击选中窗口要用它，而不是松开时的 hover_win
    /// （拖拽过程中 hover_win 会被清掉）。
    press_win: Option<RectF>,
    /// 上一次探测窗口时的光标位置，用来节流。
    probe_at: (f32, f32),

    drawing: bool,
    erasing: bool,
    /// 右键拖拽擦除的判定状态：按下点 + 是否已进入擦除。
    right_press: Option<(f32, f32)>,
    right_erasing: bool,
    shapes: Vec<Shape>,
    /// 撤销/重做存的是整份形状快照 —— 形状最多几十个，快照比差分索引稳，
    /// 橡皮擦这种「删中间一个」的操作也不用操心索引漂移。
    undo_stack: Vec<Vec<Shape>>,
    redo_stack: Vec<Vec<Shape>>,
    /// 当前展开二级菜单的工具按钮（None = 关闭）。
    menu_btn: Option<usize>,
    draft: Option<Shape>,
    pen: Vec<(f32, f32)>,

    cursor: (f32, f32),
    color: u32,
    /// 取色盘状态（color 的 HSV 源，拖动取色时保持同步）。
    hue: f32,
    sat: f32,
    val: f32,
    stroke: f32,
    /// 马赛克块大小（逻辑像素），马赛克工具的二级菜单控制它。
    block: f32,
    /// 马赛克涂抹模式：true = 笔刷沿轨迹盖马赛克，false = 框选矩形。
    mosaic_brush: bool,
    /// 涂抹笔刷半径（逻辑像素），独立于块大小可调。
    brush_radius: f32,
    /// 文本字号（逻辑像素）。
    text_size: f32,
    /// 正在拖动的菜单控件（滑杆 / 色相条 / 取色面板）。
    menu_drag: Option<MenuDrag>,
    /// 文本工具的输入状态：位置 + 已敲字符。GPUI 0.2.2 无 IME，
    /// 中文走 Ctrl+V 粘贴，英文直接键入。
    editing: Option<((f32, f32), String)>,
    hover_btn: Option<usize>,
    status: Option<String>,
}

impl Overlay {
    pub fn new(
        shot: Arc<Shot>,
        selftest: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scale = shot.scale;
        let image = render_img::from_bgra(shot.width, shot.height, shot.bgra.clone());
        let focus = cx.focus_handle();
        window.focus(&focus);
        let (mx, my) = crate::platform::capture::cursor_pos();
        let origin_x = shot.logical_x();
        let origin_y = shot.logical_y();
        let (lw, lh) = (shot.logical_w(), shot.logical_h());
        let cursor = (mx as f32 / scale - origin_x, my as f32 / scale - origin_y);
        let mut this = Self {
            shot,
            image,
            scale,
            focus,
            tool: Tool::None,
            sel: None,
            hover_win: None,
            drag: Drag::None,
            press: cursor,
            press_win: None,
            probe_at: (-9999.0, -9999.0),
            drawing: false,
            erasing: false,
            right_press: None,
            right_erasing: false,
            shapes: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            menu_btn: None,
            draft: None,
            pen: Vec::new(),
            cursor,
            color: paint::PALETTE[0],
            hue: paint::rgb_to_hsv(paint::PALETTE[0]).0,
            sat: paint::rgb_to_hsv(paint::PALETTE[0]).1,
            val: paint::rgb_to_hsv(paint::PALETTE[0]).2,
            stroke: 3.0,
            block: 12.0,
            mosaic_brush: false,
            brush_radius: 22.0,
            text_size: 16.0,
            menu_drag: None,
            editing: None,
            hover_btn: None,
            status: None,
        };

        if selftest {
            // 演示样张：一张居中的大选区 + 六种标注各来一个，位置按比例摆，
            // 换分辨率也不会跑偏。README 的截图就是这套形状导出来的。
            let sel = RectF::new(lw * 0.13, lh * 0.15, lw * 0.56, lh * 0.54);
            let (w, h) = (sel.w, sel.h);
            let at = |fx: f32, fy: f32| (sel.x + w * fx, sel.y + h * fy);
            this.sel = Some(sel);
            this.tool = Tool::Rect;
            this.shapes = vec![
                Shape::Text {
                    pos: at(0.03, 0.02),
                    text: "PIXPIN-RS 文本".into(),
                    color: 0xFF3B30,
                    size: 20.0,
                    w: 170.0,
                },
                Shape::Rect {
                    a: at(0.03, 0.11),
                    b: at(0.40, 0.43),
                    color: 0xFF3B30,
                    width: 3.0,
                },
                Shape::Ellipse {
                    a: at(0.48, 0.11),
                    b: at(0.92, 0.43),
                    color: 0x0A84FF,
                    width: 3.0,
                },
                Shape::Pen {
                    pts: (0..30)
                        .map(|i| {
                            let t = i as f32 / 29.0;
                            (
                                sel.x + w * (0.04 + t * 0.46),
                                sel.y + h * 0.62 + (t * 7.5).sin() * h * 0.055,
                            )
                        })
                        .collect(),
                    color: 0x34C759,
                    width: 5.0,
                },
                // 深色文字：导出后能看出字形渲染，也说明文字是矢量落定的
                Shape::Text {
                    pos: at(0.55, 0.60),
                    text: "标注所见即所得".into(),
                    color: 0x1C1C1E,
                    size: 22.0,
                    w: 200.0,
                },
                Shape::Arrow {
                    a: at(0.58, 0.90),
                    b: at(0.90, 0.68),
                    color: 0xFFCC00,
                    width: 4.0,
                },
            ];
            this.record();
            // Exercise the export pipeline (crop + rasterize) without needing input.
            if let Some(img) = this.composite() {
                let out = std::env::temp_dir().join("pixpin_selftest_export.png");
                if img.save(&out).is_ok() {
                    eprintln!("[pixpin-rs] selftest export -> {}", out.display());
                }
            }
        }

        this
    }

    /// 用一张已有图片重新打开覆盖层（贴图窗口的「编辑标注」）。
    ///
    /// 和截图流程唯一的差别是底图来源：这里直接把图当成整屏快照，
    /// `sel` 预置成图片本身的范围（画布可能被补过，见 [`MIN_EDIT_W`]）。
    pub fn new_editing(
        shot: Arc<Shot>,
        sel: RectF,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(shot, false, window, cx);
        let (lw, lh) = (this.shot.logical_w(), this.shot.logical_h());
        this.sel = Some(sel);
        // 光标可能落在图外（贴图窗口刚被销毁），夹回画面内，工具栏才不会跑出去。
        this.cursor = (this.cursor.0.clamp(0.0, lw), this.cursor.1.clamp(0.0, lh));
        this
    }

    fn win_size(window: &Window) -> (f32, f32) {
        let b = window.bounds().size;
        (f32::from(b.width), f32::from(b.height))
    }

    /// 选区左上角换算成屏幕物理像素，和 PixPin 的坐标口径一致。
    fn global_px(&self, v: f32) -> i32 {
        (self.shot.origin_x as f32 + v * self.scale).round() as i32
    }
}
