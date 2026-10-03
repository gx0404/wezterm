//! fork 新增：Element 浮层（命令面板 / 设置 / 快捷键速查 / 壁纸 / 右键菜单）
//! 的共享外观。
//!
//! 职责：把「圆角外框 + 1px 描边」「选中行底色 + 左侧强调条」「chrome 行」
//! 「键帽」「分隔线」「分区 tab」收敛到一处。配色每次 `compute()` 从配置现取
//! （`overlay_*` / `command_palette_*`，强调色缺省取配色的 `cursor_bg`），
//! 配置重载经 `config_was_reloaded` → `invalidate_modal` 下一帧生效，这里
//! 不持有任何缓存。
//! 几何常量是行高与外框尺寸推导的唯一真源：`row_px` / `chrome_px` /
//! `separator_px` / `row_inset_px` 与本模块构造的 Element 共用同一批常量并按
//! box model 同样的取整规则求值，可见行数与 `context_menu::menu_box_size`
//! 因此不会与实际布局漂移（WZ-04 教训）。
//! 边界：只产出 Element，不碰 hit map 语义——外框与 chrome 行挂
//! `UIItemType::Modal(MODAL_CHROME_ROW)`（WZ-06），数据行挂调用方给的行号。
//! 文本浮层（termwiz 内存终端）的行样式在 `overlay/style.rs`，两边互不引用。

use crate::termwindow::box_model::*;
use crate::termwindow::modal::MODAL_CHROME_ROW;
use crate::termwindow::render::corners::rounded_corners;
use crate::termwindow::{TermWindow, UIItemType};
use crate::utilsprites::RenderMetrics;
use config::{Config, Dimension, DimensionContext, SrgbaTuple};
use std::rc::Rc;
use wezterm_font::LoadedFont;
use wezterm_term::color::ColorPalette;
use window::color::LinearRgba;
use window::RectF;

/// 外框内边距 / 外边距（单元格）
const CONTAINER_PAD_CELLS: f32 = 0.25;
const CONTAINER_MARGIN_CELLS: f32 = 0.25;
/// 外框描边（像素）
const CONTAINER_BORDER_PX: f32 = 1.;
/// 数据行与 chrome 行的左右 / 上下内边距（单元格）
const ROW_PAD_H_CELLS: f32 = 0.5;
const ROW_PAD_V_CELLS: f32 = 0.1;
/// 选中行左侧强调条（像素）。未选中行与 chrome 行留同宽的透明左边框，
/// 所有行的文字因此左对齐
const ACCENT_BAR_PX: f32 = 2.;
/// 分隔线粗细（像素）与上下留白（单元格）
const SEPARATOR_PX: f32 = 1.;
const SEPARATOR_MARGIN_V_CELLS: f32 = 0.25;
/// 分区 tab 下划线（像素）与左右内边距（单元格）
const TAB_UNDERLINE_PX: f32 = 2.;
const TAB_PAD_H_CELLS: f32 = 0.75;
/// 键帽左右内边距与它左侧的遮罩留白（单元格）
const CHIP_PAD_H_CELLS: f32 = 0.5;
const CHIP_GAP_CELLS: f32 = 1.;
/// 键帽底色：前景在行底色上的混合比例
const CHIP_TINT: f32 = 0.12;
/// 未配置时：描边 = 前景 12% 混在底色上，次要文字 = 前景 60%
const STROKE_TINT: f32 = 0.12;
const SECONDARY_TINT: f32 = 0.6;

/// chrome 行的种类：决定文字颜色与上下留白
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeKind {
    /// 浮层标题
    Title,
    /// 输入行（`> filter_`）
    Input,
    /// 页脚按键提示，用次要文字色
    Hint,
    /// 行内错误 / 状态提示
    Error,
}

/// Element 浮层的配色与几何
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayStyle {
    pub surface: LinearRgba,
    pub text: LinearRgba,
    pub text_dim: LinearRgba,
    pub stroke: LinearRgba,
    pub sel_bg: LinearRgba,
    pub sel_fg: LinearRgba,
    pub accent: LinearRgba,
    pub radius: Dimension,
    pub row_radius: Dimension,
    pub pad: BoxDimension,
    pub row_pad: BoxDimension,
}

/// sRGB 空间按 `t` 把 `over` 混到 `base` 上，得到不透明色。
///
/// 「前景 × α」不能直接交给 GPU：字形与 poly 的片元 alpha 在着色器里被
/// 覆盖率顶替，文字颜色的 alpha 会被丢掉；帧缓冲又是 sRGB，所以按 sRGB
/// 线性插值才与真正的 alpha 混合同一观感。
pub fn blend(base: LinearRgba, over: LinearRgba, t: f32) -> LinearRgba {
    let t = t.clamp(0., 1.);
    let b = base.to_srgb();
    let o = over.to_srgb();
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    SrgbaTuple(
        lerp(b.0, o.0),
        lerp(b.1, o.1),
        lerp(b.2, o.2),
        lerp(b.3, o.3),
    )
    .to_linear()
}

/// 同单位减半：行内圆角取外框圆角的一半，选中行嵌在外框里显得同心
fn half(d: Dimension) -> Dimension {
    match d {
        Dimension::Points(n) => Dimension::Points(n / 2.),
        Dimension::Pixels(n) => Dimension::Pixels(n / 2.),
        Dimension::Percent(n) => Dimension::Percent(n / 2.),
        Dimension::Cells(n) => Dimension::Cells(n / 2.),
    }
}

/// 按 box model 同样的取整规则（`Dimension::evaluate_as_pixels` 向下取整）
/// 把单元格数折成像素
fn cells_px(cells: f32, cell: f32) -> f32 {
    Dimension::Cells(cells).evaluate_as_pixels(DimensionContext {
        dpi: 0.,
        pixel_max: 0.,
        pixel_cell: cell,
    })
}

fn px(n: f32) -> Dimension {
    Dimension::Pixels(n)
}

fn zero() -> Dimension {
    Dimension::Pixels(0.)
}

/// 一行数据行的像素高度：内容一格 + 上下行内边距。`metrics` 必须是浮层
/// 渲染实际用的那份（命令面板字体，已乘 `command_palette_line_height`）
pub fn row_px(metrics: &RenderMetrics) -> f32 {
    row_height_px(metrics.cell_size.height as f32)
}

/// `row_px` 的单元格高度版本（右键菜单按格子尺寸推外框时用）
pub fn row_height_px(cell_h: f32) -> f32 {
    cell_h + 2. * cells_px(ROW_PAD_V_CELLS, cell_h)
}

/// 外框在两个方向上占用的像素（两侧内边距 + 外边距 + 描边之和），
/// 返回 (宽, 高)
pub fn chrome_px(cell_w: f32, cell_h: f32) -> (f32, f32) {
    let ring = |cell: f32| {
        2. * (cells_px(CONTAINER_PAD_CELLS, cell) + cells_px(CONTAINER_MARGIN_CELLS, cell))
            + 2. * CONTAINER_BORDER_PX
    };
    (ring(cell_w), ring(cell_h))
}

/// 一行在水平方向上占用的非内容像素：左右行内边距 + 强调条
pub fn row_inset_px(cell_w: f32) -> f32 {
    2. * cells_px(ROW_PAD_H_CELLS, cell_w) + ACCENT_BAR_PX
}

/// 分隔行的像素高度：线 + 上下留白
pub fn separator_px(cell_h: f32) -> f32 {
    SEPARATOR_PX + 2. * cells_px(SEPARATOR_MARGIN_V_CELLS, cell_h)
}

/// `avail_px` 高度里放得下几行数据行
pub fn rows_that_fit(avail_px: f32, metrics: &RenderMetrics) -> usize {
    (avail_px.max(0.) / row_px(metrics).max(1.)).floor() as usize
}

/// 浮层统一的布局上下文：宽高按窗口像素封顶，单元格取浮层字体度量
pub fn overlay_layout_context<'a>(
    tw: &'a TermWindow,
    metrics: &'a RenderMetrics,
    bounds: RectF,
) -> LayoutContext<'a> {
    LayoutContext {
        height: DimensionContext {
            dpi: tw.dimensions.dpi as f32,
            pixel_max: tw.dimensions.pixel_height as f32,
            pixel_cell: metrics.cell_size.height as f32,
        },
        width: DimensionContext {
            dpi: tw.dimensions.dpi as f32,
            pixel_max: tw.dimensions.pixel_width as f32,
            pixel_cell: metrics.cell_size.width as f32,
        },
        bounds,
        metrics,
        gl_state: tw.render_state.as_ref().unwrap(),
        zindex: 100,
    }
}

impl OverlayStyle {
    /// 从配置现取配色；未配置的项按文档推导：选中行沿用反色、描边与次要
    /// 文字由前景混在底色上得到、强调色取配色方案的 `cursor_bg`
    pub fn from_config(config: &Config, palette: &ColorPalette) -> Self {
        let surface = config.command_palette_bg_color.to_linear();
        let text = config.command_palette_fg_color.to_linear();
        let pick = |c: Option<config::RgbaColor>, fallback: LinearRgba| {
            c.map(|c| c.to_linear()).unwrap_or(fallback)
        };
        Self {
            surface,
            text,
            text_dim: pick(
                config.command_palette_secondary_fg_color,
                blend(surface, text, SECONDARY_TINT),
            ),
            stroke: pick(
                config.overlay_border_color,
                blend(surface, text, STROKE_TINT),
            ),
            sel_bg: pick(config.command_palette_selection_bg_color, text),
            sel_fg: pick(config.command_palette_selection_fg_color, surface),
            accent: pick(
                config.command_palette_accent_color,
                palette.cursor_bg.to_linear(),
            ),
            radius: config.overlay_corner_radius,
            row_radius: half(config.overlay_corner_radius),
            pad: BoxDimension::new(Dimension::Cells(CONTAINER_PAD_CELLS)),
            row_pad: BoxDimension {
                left: Dimension::Cells(ROW_PAD_H_CELLS),
                right: Dimension::Cells(ROW_PAD_H_CELLS),
                top: Dimension::Cells(ROW_PAD_V_CELLS),
                bottom: Dimension::Cells(ROW_PAD_V_CELLS),
            },
        }
    }

    /// 外框：圆角 + 1px 描边 + 底色。外框自己进 hit map，内边距 / 描边 /
    /// 外边距那一圈不属于任何行，点在那里不能被当成点浮层外（WZ-06）。
    /// 宽度等由调用方再加（`min_width` 等）
    pub fn container(&self, font: &Rc<LoadedFont>, rows: Vec<Element>) -> Element {
        Element::new(font, ElementContent::Children(rows))
            .colors(ElementColors {
                border: BorderColor::new(self.stroke),
                bg: self.surface.into(),
                text: self.text.into(),
            })
            .padding(self.pad)
            .border(BoxDimension::new(px(CONTAINER_BORDER_PX)))
            .margin(BoxDimension::new(Dimension::Cells(CONTAINER_MARGIN_CELLS)))
            .border_corners(Some(rounded_corners(self.radius)))
            .display(DisplayType::Block)
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW))
    }

    /// 数据行的配色：选中 = 选中底色 + 左侧强调条；未选中透明底、强调条
    /// 位置留同宽透明边
    fn row_colors(&self, selected: bool) -> ElementColors {
        if selected {
            ElementColors {
                border: BorderColor {
                    left: self.accent,
                    top: LinearRgba::TRANSPARENT,
                    right: LinearRgba::TRANSPARENT,
                    bottom: LinearRgba::TRANSPARENT,
                },
                bg: self.sel_bg.into(),
                text: self.sel_fg.into(),
            }
        } else {
            ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: LinearRgba::TRANSPARENT.into(),
                text: self.text.into(),
            }
        }
    }

    /// 一行数据行，挂 `UIItemType::Modal(idx)` 进 hit map。选中行圆角取
    /// `row_radius`；未选中行没有底色，不画圆角省掉角片
    pub fn row(
        &self,
        font: &Rc<LoadedFont>,
        content: ElementContent,
        selected: bool,
        idx: usize,
    ) -> Element {
        Element::new(font, content)
            .colors(self.row_colors(selected))
            .border_corners(selected.then(|| rounded_corners(self.row_radius)))
            .border(BoxDimension {
                left: px(ACCENT_BAR_PX),
                top: zero(),
                right: zero(),
                bottom: zero(),
            })
            .padding(self.row_pad)
            .min_width(Some(Dimension::Percent(1.)))
            .display(DisplayType::Block)
            .item_type(UIItemType::Modal(idx))
    }

    /// chrome 行（标题 / 输入 / 页脚 / 错误）：吞点击不关浮层
    pub fn chrome(&self, font: &Rc<LoadedFont>, text: String, kind: ChromeKind) -> Element {
        let (color, top, bottom) = match kind {
            ChromeKind::Title => (self.text, ROW_PAD_V_CELLS, ROW_PAD_V_CELLS),
            ChromeKind::Input => (self.text, 0., ROW_PAD_V_CELLS),
            ChromeKind::Hint => (self.text_dim, ROW_PAD_V_CELLS, ROW_PAD_V_CELLS),
            ChromeKind::Error => (self.text, 0., 0.),
        };
        Element::new(font, ElementContent::Text(text))
            .colors(ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: LinearRgba::TRANSPARENT.into(),
                text: color.into(),
            })
            .border(BoxDimension {
                left: px(ACCENT_BAR_PX),
                top: zero(),
                right: zero(),
                bottom: zero(),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(ROW_PAD_H_CELLS),
                right: Dimension::Cells(ROW_PAD_H_CELLS),
                top: Dimension::Cells(top),
                bottom: Dimension::Cells(bottom),
            })
            .min_width(Some(Dimension::Percent(1.)))
            .display(DisplayType::Block)
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW))
    }

    /// 键帽的 (遮罩底色, 键帽底色, 文字色)：遮罩取行的不透明底色
    fn chip_colors(&self, selected: bool) -> (LinearRgba, LinearRgba, LinearRgba) {
        if selected {
            (
                self.sel_bg,
                blend(self.sel_bg, self.sel_fg, CHIP_TINT),
                self.sel_fg,
            )
        } else {
            (
                self.surface,
                blend(self.surface, self.text, CHIP_TINT),
                self.text_dim,
            )
        }
    }

    /// 行尾右浮的键位标签。外层用行的不透明底色做遮罩（zindex 10）：标签
    /// 过长时盖住被它压住的正文，与上游反色块的遮挡语义一致
    pub fn key_chip(&self, font: &Rc<LoadedFont>, label: String, selected: bool) -> Element {
        let (mask, chip_bg, fg) = self.chip_colors(selected);
        let chip = Element::new(font, ElementContent::Text(label))
            .colors(ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: chip_bg.into(),
                text: fg.into(),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(CHIP_PAD_H_CELLS),
                right: Dimension::Cells(CHIP_PAD_H_CELLS),
                top: zero(),
                bottom: zero(),
            })
            .border_corners(Some(rounded_corners(self.row_radius)));
        Element::new(font, ElementContent::Children(vec![chip]))
            .float(Float::Right)
            .zindex(10)
            .padding(BoxDimension {
                left: Dimension::Cells(CHIP_GAP_CELLS),
                right: zero(),
                top: zero(),
                bottom: zero(),
            })
            .colors(ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: mask.into(),
                text: fg.into(),
            })
    }

    /// 满宽 1px 分隔线（描边色），上下各留 `SEPARATOR_MARGIN_V_CELLS`
    pub fn separator(&self, font: &Rc<LoadedFont>) -> Element {
        Element::new(font, ElementContent::Children(vec![]))
            .colors(ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: self.stroke.into(),
                text: self.text.into(),
            })
            .min_width(Some(Dimension::Percent(1.)))
            .min_height(Some(px(SEPARATOR_PX)))
            .margin(BoxDimension {
                left: zero(),
                right: zero(),
                top: Dimension::Cells(SEPARATOR_MARGIN_V_CELLS),
                bottom: Dimension::Cells(SEPARATOR_MARGIN_V_CELLS),
            })
            .display(DisplayType::Block)
    }

    /// 分区 tab：当前分区为正文色 + 底部强调线，其余为次要文字色；
    /// 底边两种状态等宽，切换时不跳动。hit map 项由调用方挂
    pub fn section_tab(&self, font: &Rc<LoadedFont>, label: String, active: bool) -> Element {
        let (underline, color) = if active {
            (self.accent, self.text)
        } else {
            (LinearRgba::TRANSPARENT, self.text_dim)
        };
        Element::new(font, ElementContent::Text(label))
            .colors(ElementColors {
                border: BorderColor {
                    left: LinearRgba::TRANSPARENT,
                    top: LinearRgba::TRANSPARENT,
                    right: LinearRgba::TRANSPARENT,
                    bottom: underline,
                },
                bg: LinearRgba::TRANSPARENT.into(),
                text: color.into(),
            })
            .border(BoxDimension {
                left: zero(),
                top: zero(),
                right: zero(),
                bottom: px(TAB_UNDERLINE_PX),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(TAB_PAD_H_CELLS),
                right: Dimension::Cells(TAB_PAD_H_CELLS),
                top: zero(),
                bottom: Dimension::Cells(ROW_PAD_V_CELLS),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wezterm_font::units::PixelLength;

    fn close(a: LinearRgba, b: LinearRgba) -> bool {
        let d = |x: f32, y: f32| (x - y).abs() < 1e-3;
        d(a.0, b.0) && d(a.1, b.1) && d(a.2, b.2) && d(a.3, b.3)
    }

    fn metrics(cell_w: isize, cell_h: isize) -> RenderMetrics {
        RenderMetrics {
            descender: PixelLength::new(0.),
            descender_row: 0,
            descender_plus_two: 0,
            underline_height: 1,
            strike_row: 0,
            cell_size: ::window::Size::new(cell_w, cell_h),
        }
    }

    fn rgb(r: u8, g: u8, b: u8) -> config::RgbaColor {
        (r, g, b).into()
    }

    #[test]
    fn blend_endpoints_and_midpoint() {
        let black = LinearRgba::with_components(0., 0., 0., 1.);
        let white = LinearRgba::with_components(1., 1., 1., 1.);
        assert!(close(blend(black, white, 0.), black));
        assert!(close(blend(black, white, 1.), white));
        // sRGB 空间的中点：50% 灰，线性值约 0.214
        let mid = blend(black, white, 0.5);
        assert!((mid.0 - 0.214).abs() < 0.01, "{mid:?}");
        assert_eq!(mid.3, 1.);
        // 越界的比例被夹住
        assert!(close(blend(black, white, 2.), white));
    }

    #[test]
    fn unset_colors_follow_the_palette_colors() {
        let config = Config::default_config();
        let palette = ColorPalette::default();
        let style = OverlayStyle::from_config(&config, &palette);
        let surface = config.command_palette_bg_color.to_linear();
        let text = config.command_palette_fg_color.to_linear();

        assert_eq!(style.surface, surface);
        assert_eq!(style.text, text);
        // 选中行未配置时沿用反色
        assert_eq!(style.sel_bg, text);
        assert_eq!(style.sel_fg, surface);
        // 描边 / 次要文字由前景混在底色上得到
        assert!(close(style.stroke, blend(surface, text, STROKE_TINT)));
        assert!(close(style.text_dim, blend(surface, text, SECONDARY_TINT)));
        assert_ne!(style.stroke, surface);
        assert_eq!(style.accent, palette.cursor_bg.to_linear());
        assert_eq!(style.radius, Dimension::Cells(0.25));
        assert_eq!(style.row_radius, Dimension::Cells(0.125));
    }

    #[test]
    fn configured_colors_win() {
        let mut config = Config::default_config();
        config.overlay_border_color = Some(rgb(0x45, 0x47, 0x5a));
        config.command_palette_selection_bg_color = Some(rgb(0x31, 0x32, 0x44));
        config.command_palette_selection_fg_color = Some(rgb(0xcd, 0xd6, 0xf4));
        config.command_palette_secondary_fg_color = Some(rgb(0xa6, 0xad, 0xc8));
        config.command_palette_accent_color = Some(rgb(0x89, 0xb4, 0xfa));
        config.overlay_corner_radius = Dimension::Pixels(8.);
        let style = OverlayStyle::from_config(&config, &ColorPalette::default());

        assert_eq!(style.stroke, rgb(0x45, 0x47, 0x5a).to_linear());
        assert_eq!(style.sel_bg, rgb(0x31, 0x32, 0x44).to_linear());
        assert_eq!(style.sel_fg, rgb(0xcd, 0xd6, 0xf4).to_linear());
        assert_eq!(style.text_dim, rgb(0xa6, 0xad, 0xc8).to_linear());
        assert_eq!(style.accent, rgb(0x89, 0xb4, 0xfa).to_linear());
        assert_eq!(style.radius, Dimension::Pixels(8.));
        assert_eq!(style.row_radius, Dimension::Pixels(4.));
    }

    #[test]
    fn selected_row_gets_background_and_accent_bar_only() {
        let style = OverlayStyle::from_config(&Config::default_config(), &ColorPalette::default());
        let selected = style.row_colors(true);
        assert_eq!(selected.bg, InheritableColor::Color(style.sel_bg));
        assert_eq!(selected.text, InheritableColor::Color(style.sel_fg));
        assert_eq!(selected.border.left, style.accent);
        assert_eq!(selected.border.top, LinearRgba::TRANSPARENT);
        assert_eq!(selected.border.right, LinearRgba::TRANSPARENT);
        assert_eq!(selected.border.bottom, LinearRgba::TRANSPARENT);

        let plain = style.row_colors(false);
        assert_eq!(plain.bg, InheritableColor::Color(LinearRgba::TRANSPARENT));
        assert_eq!(plain.text, InheritableColor::Color(style.text));
        assert_eq!(plain.border.left, LinearRgba::TRANSPARENT);
    }

    #[test]
    fn key_chip_masks_with_the_opaque_row_background() {
        let style = OverlayStyle::from_config(&Config::default_config(), &ColorPalette::default());
        let (mask, chip, fg) = style.chip_colors(false);
        assert_eq!(mask, style.surface);
        assert_eq!(fg, style.text_dim);
        assert_eq!(chip.3, 1.);
        assert_ne!(chip, style.surface);

        let (mask, chip, fg) = style.chip_colors(true);
        assert_eq!(mask, style.sel_bg);
        assert_eq!(fg, style.sel_fg);
        assert_ne!(chip, style.sel_bg);
    }

    #[test]
    fn geometry_matches_the_box_model_rounding() {
        // 9×20 的格子：行内边距 floor(0.1×20)=2，上下各一
        assert_eq!(row_px(&metrics(9, 20)), 24.);
        // 0.1×17 = 1.7 向下取整为 1
        assert_eq!(row_px(&metrics(8, 17)), 19.);
        assert_eq!(rows_that_fit(240., &metrics(9, 20)), 10);
        assert_eq!(rows_that_fit(-5., &metrics(9, 20)), 0);

        // 外框：两侧 floor(0.25×cell) 的内边距与外边距 + 两条 1px 描边
        assert_eq!(
            chrome_px(9., 20.),
            (2. * (2. + 2.) + 2., 2. * (5. + 5.) + 2.)
        );
        // 行的水平占用：左右 floor(0.5×9)=4 + 2px 强调条
        assert_eq!(row_inset_px(9.), 10.);
        // 分隔行：1px + 上下 floor(0.25×20)=5
        assert_eq!(separator_px(20.), 11.);
    }
}
