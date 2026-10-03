use crate::customglyph::*;
use crate::termwindow::box_model::*;
use crate::termwindow::overlay_style::blend;
use crate::termwindow::render::corners::*;
use crate::termwindow::{TabBarItem, UIItemType};
use crate::utilsprites::RenderMetrics;
use config::{
    ConfigHandle, Dimension, DimensionContext, IntegratedTitleButtonColor, RgbaColor,
    WindowFrameConfig,
};
use std::rc::Rc;
use wezterm_font::LoadedFont;
use window::color::LinearRgba;
use window::{IntegratedTitleButton, IntegratedTitleButtonStyle as Style};

pub struct WindowButtonColors {
    pub colors: ElementColors,
    pub hover_colors: ElementColors,
}

/// fork: the window state the integrated buttons are drawn for
pub struct WindowButtonState {
    pub is_maximized: bool,
    pub focused: bool,
    /// Pixel height of the tab bar; the Windows style buttons span it
    pub bar_height: f32,
    pub dpi: f32,
}

/// fork: minimum HSL lightness difference between the titlebar background
/// and `window_frame.*_titlebar_fg` for the latter to be used as the Auto
/// button color.
const MIN_FG_LIGHTNESS_CONTRAST: f64 = 0.3;

/// fork: unfocused windows draw their button glyphs at this strength over
/// the titlebar background.
const UNFOCUSED_FG_STRENGTH: f32 = 0.6;

/// fork: the Windows 11 caption close button color (`#c42b1c`), used when
/// `window_frame.close_button_hover_bg` is unset.
fn default_close_hover_bg() -> LinearRgba {
    RgbaColor::from((0xc4u8, 0x2bu8, 0x1cu8)).to_linear()
}

/// fork: the titlebar (background, foreground) for the focus state
fn titlebar_colors(frame: &WindowFrameConfig, focused: bool) -> (RgbaColor, RgbaColor) {
    if focused {
        (frame.active_titlebar_bg, frame.active_titlebar_fg)
    } else {
        (frame.inactive_titlebar_bg, frame.inactive_titlebar_fg)
    }
}

/// fork: the button glyph color. `Auto` prefers the titlebar foreground of
/// the focus state and only falls back to black or white (by background
/// lightness) when that foreground does not contrast with the background.
/// Unfocused windows blend the result toward the background: glyph alpha
/// is replaced by coverage in the shader, so the dimming is pre-mixed.
fn auto_button_color(
    titlebar_bg: RgbaColor,
    titlebar_fg: RgbaColor,
    foreground: &IntegratedTitleButtonColor,
    focused: bool,
) -> LinearRgba {
    let color = match foreground {
        IntegratedTitleButtonColor::Custom(color) => color.to_linear(),
        IntegratedTitleButtonColor::Auto => {
            let (_, _, bg_lightness, _) = titlebar_bg.to_hsla();
            let (_, _, fg_lightness, _) = titlebar_fg.to_hsla();
            if (bg_lightness - fg_lightness).abs() >= MIN_FG_LIGHTNESS_CONTRAST {
                titlebar_fg.to_linear()
            } else if bg_lightness > 0.5 {
                LinearRgba(0.0, 0.0, 0.0, 1.0)
            } else {
                LinearRgba(1.0, 1.0, 1.0, 1.0)
            }
        }
    };
    if focused {
        color
    } else {
        blend(titlebar_bg.to_linear(), color, UNFOCUSED_FG_STRENGTH)
    }
}

/// fork: (top, bottom) padding in whole pixels that makes a button with a
/// `glyph`-pixel tall poly exactly `bar_height` tall
fn vertical_padding_to_fill(bar_height: f32, glyph: f32) -> (f32, f32) {
    let free = (bar_height.floor() - glyph).max(0.);
    let top = (free / 2.).floor();
    (top, free - top)
}

mod windows {
    use super::*;

    pub const CLOSE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::LineTo(BlockCoord::One, BlockCoord::One),
            PolyCommand::MoveTo(BlockCoord::One, BlockCoord::Zero),
            PolyCommand::LineTo(BlockCoord::Zero, BlockCoord::One),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::OutlineThin,
    }];

    pub const HIDE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::MoveTo(BlockCoord::Zero, BlockCoord::Frac(6, 10)),
            PolyCommand::LineTo(BlockCoord::One, BlockCoord::Frac(6, 10)),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::OutlineThin,
    }];

    pub const MAXIMIZE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::MoveTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(1, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(9, 10), BlockCoord::Frac(1, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(10, 10), BlockCoord::Frac(2, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(10, 10), BlockCoord::Frac(9, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(9, 10), BlockCoord::Frac(10, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(10, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(1, 10), BlockCoord::Frac(9, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(1, 10), BlockCoord::Frac(2, 10)),
            PolyCommand::LineTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(1, 10)),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::OutlineThin,
    }];

    pub const RESTORE: &[Poly] = &[
        Poly {
            path: &[
                PolyCommand::MoveTo(BlockCoord::Frac(5, 20), BlockCoord::Frac(1, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(8, 10), BlockCoord::Frac(1, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(10, 10), BlockCoord::Frac(3, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(10, 10), BlockCoord::Frac(15, 20)),
            ],
            intensity: BlockAlpha::Full,
            style: PolyStyle::OutlineThin,
        },
        Poly {
            path: &[
                PolyCommand::MoveTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(3, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(7, 10), BlockCoord::Frac(3, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(8, 10), BlockCoord::Frac(4, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(8, 10), BlockCoord::Frac(9, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(7, 10), BlockCoord::Frac(10, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(10, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(1, 10), BlockCoord::Frac(9, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(1, 10), BlockCoord::Frac(4, 10)),
                PolyCommand::LineTo(BlockCoord::Frac(2, 10), BlockCoord::Frac(3, 10)),
            ],
            intensity: BlockAlpha::Full,
            style: PolyStyle::OutlineThin,
        },
    ];

    pub fn sized_poly(poly: &'static [Poly]) -> SizedPoly {
        let scale = 72.0 / 96.0;
        let size = Dimension::Points(10. * scale);
        SizedPoly {
            poly,
            width: size,
            height: size,
        }
    }

    pub fn window_button_colors(
        foreground: LinearRgba,
        close_hover_bg: LinearRgba,
        window_button: IntegratedTitleButton,
    ) -> WindowButtonColors {
        let colors = ElementColors {
            border: BorderColor::new(LinearRgba::TRANSPARENT),
            bg: LinearRgba::TRANSPARENT.into(),
            text: foreground.into(),
        };

        // fork: the close hover color comes from
        // window_frame.close_button_hover_bg (Windows 11 red by default)
        let hover_colors = if window_button == IntegratedTitleButton::Close {
            ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: close_hover_bg.into(),
                text: LinearRgba(1.0, 1.0, 1.0, 1.0).into(),
            }
        } else {
            ElementColors {
                border: BorderColor::new(LinearRgba::TRANSPARENT),
                bg: foreground.mul_alpha(0.1).into(),
                text: foreground.into(),
            }
        };

        WindowButtonColors {
            colors,
            hover_colors,
        }
    }
}

mod gnome {
    use super::*;

    pub const CLOSE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::LineTo(BlockCoord::One, BlockCoord::One),
            PolyCommand::MoveTo(BlockCoord::One, BlockCoord::Zero),
            PolyCommand::LineTo(BlockCoord::Zero, BlockCoord::One),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::Outline,
    }];

    pub const HIDE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::MoveTo(BlockCoord::Zero, BlockCoord::Frac(15, 16)),
            PolyCommand::LineTo(BlockCoord::One, BlockCoord::Frac(15, 16)),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::Outline,
    }];

    pub const MAXIMIZE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::LineTo(BlockCoord::Frac(1, 16), BlockCoord::Frac(15, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(15, 16), BlockCoord::Frac(15, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(15, 16), BlockCoord::Frac(1, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(1, 16), BlockCoord::Frac(1, 16)),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::Outline,
    }];

    pub const RESTORE: &[Poly] = &[Poly {
        path: &[
            PolyCommand::MoveTo(BlockCoord::Frac(3, 16), BlockCoord::Frac(3, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(3, 16), BlockCoord::Frac(13, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(13, 16), BlockCoord::Frac(13, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(13, 16), BlockCoord::Frac(3, 16)),
            PolyCommand::LineTo(BlockCoord::Frac(3, 16), BlockCoord::Frac(3, 16)),
        ],
        intensity: BlockAlpha::Full,
        style: PolyStyle::Outline,
    }];

    pub fn sized_poly(poly: &'static [Poly]) -> SizedPoly {
        let size = Dimension::Pixels(8.);
        SizedPoly {
            poly,
            width: size,
            height: size,
        }
    }

    pub fn window_button_colors(
        foreground: LinearRgba,
        _close_hover_bg: LinearRgba,
        _window_button: IntegratedTitleButton,
    ) -> WindowButtonColors {
        WindowButtonColors {
            colors: ElementColors {
                border: BorderColor::new(foreground.mul_alpha(0.1)),
                bg: foreground.mul_alpha(0.1).into(),
                text: foreground.into(),
            },
            hover_colors: ElementColors {
                border: BorderColor::new(foreground.mul_alpha(0.15)),
                bg: foreground.mul_alpha(0.15).into(),
                text: foreground.into(),
            },
        }
    }
}

pub fn window_button_element(
    window_button: IntegratedTitleButton,
    state: &WindowButtonState,
    font: &Rc<LoadedFont>,
    metrics: &RenderMetrics,
    config: &ConfigHandle,
) -> Element {
    let style = config.integrated_title_button_style;

    if style == Style::MacOsNative {
        return Element::new(font, ElementContent::Text(String::new()));
    }

    let poly = {
        let (close, hide, maximize, restore) = match style {
            Style::Windows => {
                use self::windows::{CLOSE, HIDE, MAXIMIZE, RESTORE};
                (CLOSE, HIDE, MAXIMIZE, RESTORE)
            }
            Style::Gnome => {
                use self::gnome::{CLOSE, HIDE, MAXIMIZE, RESTORE};
                (CLOSE, HIDE, MAXIMIZE, RESTORE)
            }
            Style::MacOsNative => unreachable!(),
        };
        let poly = match window_button {
            IntegratedTitleButton::Hide => hide,
            IntegratedTitleButton::Maximize => {
                if state.is_maximized {
                    restore
                } else {
                    maximize
                }
            }
            IntegratedTitleButton::Close => close,
        };

        match style {
            Style::Windows => self::windows::sized_poly(poly),
            Style::Gnome => self::gnome::sized_poly(poly),
            Style::MacOsNative => unreachable!(),
        }
    };

    let element = Element::new(
        &font,
        ElementContent::Poly {
            line_width: metrics.underline_height.max(2),
            poly,
        },
    );

    let element = match style {
        Style::Windows => {
            let left_padding = match window_button {
                IntegratedTitleButton::Hide => 17.0,
                _ => 18.0,
            };
            let scale = 72.0 / 96.0;
            // fork: span the whole tab bar height (Windows 11 caption
            // buttons) instead of a fixed 30px box
            let glyph = poly.height.evaluate_as_pixels(DimensionContext {
                dpi: state.dpi,
                pixel_max: state.bar_height,
                pixel_cell: state.bar_height,
            });
            let (top, bottom) = vertical_padding_to_fill(state.bar_height, glyph);

            element
                .zindex(1)
                .vertical_align(VerticalAlign::Middle)
                .padding(BoxDimension {
                    left: Dimension::Points(left_padding * scale),
                    right: Dimension::Points(18. * scale),
                    top: Dimension::Pixels(top),
                    bottom: Dimension::Pixels(bottom),
                })
        }
        Style::Gnome => {
            let dim = Dimension::Pixels(7.);
            let border_corners_size = Dimension::Pixels(12.);
            element
                .zindex(1)
                .vertical_align(VerticalAlign::Middle)
                .padding(BoxDimension {
                    left: dim,
                    right: dim,
                    top: dim,
                    bottom: dim,
                })
                .border(BoxDimension::new(Dimension::Pixels(1.)))
                .border_corners(Some(Corners {
                    top_left: SizedPoly {
                        width: border_corners_size,
                        height: border_corners_size,
                        poly: TOP_LEFT_ROUNDED_CORNER,
                    },
                    top_right: SizedPoly {
                        width: border_corners_size,
                        height: border_corners_size,
                        poly: TOP_RIGHT_ROUNDED_CORNER,
                    },
                    bottom_left: SizedPoly {
                        width: border_corners_size,
                        height: border_corners_size,
                        poly: BOTTOM_LEFT_ROUNDED_CORNER,
                    },
                    bottom_right: SizedPoly {
                        width: border_corners_size,
                        height: border_corners_size,
                        poly: BOTTOM_RIGHT_ROUNDED_CORNER,
                    },
                }))
                .margin(BoxDimension {
                    left: dim,
                    right: dim,
                    top: dim,
                    bottom: dim,
                })
        }
        Style::MacOsNative => unreachable!(),
    };

    // fork: the glyph color follows the focus state and prefers the
    // titlebar foreground (see auto_button_color)
    let (titlebar_bg, titlebar_fg) = titlebar_colors(&config.window_frame, state.focused);
    let foreground = auto_button_color(
        titlebar_bg,
        titlebar_fg,
        &config.integrated_title_button_color,
        state.focused,
    );
    let close_hover_bg = config
        .window_frame
        .close_button_hover_bg
        .map(|c| c.to_linear())
        .unwrap_or_else(default_close_hover_bg);

    let window_button_colors_fn = match style {
        Style::Windows => self::windows::window_button_colors,
        Style::Gnome => self::gnome::window_button_colors,
        Style::MacOsNative => unreachable!(),
    };

    let colors = window_button_colors_fn(foreground, close_hover_bg, window_button);

    let element = element
        .item_type(UIItemType::TabBar(TabBarItem::WindowButton(window_button)))
        .colors(colors.colors)
        .hover_colors(Some(colors.hover_colors));

    element
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> RgbaColor {
        (r, g, b).into()
    }

    #[test]
    fn auto_prefers_the_titlebar_foreground_when_it_contrasts() {
        let bg = rgb(0x1e, 0x1e, 0x2e);
        let fg = rgb(0xcd, 0xd6, 0xf4);
        let auto = IntegratedTitleButtonColor::Auto;
        assert_eq!(auto_button_color(bg, fg, &auto, true), fg.to_linear());
        // 前景与底色太接近：按底色亮度回落纯白 / 纯黑
        let dark_fg = rgb(0x31, 0x32, 0x44);
        assert_eq!(
            auto_button_color(bg, dark_fg, &auto, true),
            LinearRgba(1.0, 1.0, 1.0, 1.0)
        );
        let light_bg = rgb(0xef, 0xf1, 0xf5);
        let light_fg = rgb(0xdc, 0xe0, 0xe8);
        assert_eq!(
            auto_button_color(light_bg, light_fg, &auto, true),
            LinearRgba(0.0, 0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn unfocused_buttons_are_dimmed_toward_the_background() {
        let bg = rgb(0x1e, 0x1e, 0x2e);
        let fg = rgb(0xcd, 0xd6, 0xf4);
        for color in [
            IntegratedTitleButtonColor::Auto,
            IntegratedTitleButtonColor::Custom(fg),
        ] {
            let dimmed = auto_button_color(bg, fg, &color, false);
            assert_eq!(dimmed, blend(bg.to_linear(), fg.to_linear(), 0.6));
            assert_ne!(dimmed, fg.to_linear());
            assert_eq!(dimmed.3, 1.0);
        }
    }

    #[test]
    fn titlebar_colors_follow_focus() {
        let frame = WindowFrameConfig::default();
        assert_eq!(
            titlebar_colors(&frame, true),
            (frame.active_titlebar_bg, frame.active_titlebar_fg)
        );
        assert_eq!(
            titlebar_colors(&frame, false),
            (frame.inactive_titlebar_bg, frame.inactive_titlebar_fg)
        );
    }

    #[test]
    fn close_hover_defaults_to_windows_11_red() {
        let red = default_close_hover_bg();
        assert_eq!(red, rgb(0xc4, 0x2b, 0x1c).to_linear());
        let colors = windows::window_button_colors(
            LinearRgba(1.0, 1.0, 1.0, 1.0),
            red,
            IntegratedTitleButton::Close,
        );
        assert_eq!(colors.hover_colors.bg, InheritableColor::Color(red));
    }

    #[test]
    fn button_padding_fills_the_bar_exactly() {
        for (bar, glyph) in [(35., 10.), (36., 10.), (28., 15.), (30.4, 10.)] {
            let (top, bottom) = vertical_padding_to_fill(bar, glyph);
            assert_eq!(top + glyph + bottom, f32::floor(bar), "bar={bar}");
            assert_eq!(top, top.floor());
            assert_eq!(bottom, bottom.floor());
            assert!((top - bottom).abs() <= 1.);
        }
        // 栏比字形还矮：不留负内边距
        assert_eq!(vertical_padding_to_fill(8., 10.), (0., 0.));
    }
}
