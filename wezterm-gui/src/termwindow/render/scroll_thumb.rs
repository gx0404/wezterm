//! fork(zh)：滚动条 thumb 细线化。
//!
//! 上游把 thumb 画成铺满右侧 padding 的实心矩形；这里改为宽度由
//! `scroll_bar_thumb_width` 决定（默认 3pt≈96dpi 下 4px）、靠右、两端
//! 半圆的细条，悬停或拖动时提亮。只改「画什么」：鼠标命中区（pane.rs
//! 登记的 Above/ScrollThumb/Below 三个 `UIItem`）仍覆盖整个 padding 宽，
//! 拖动/点击手感不变。几何计算是纯函数，单测锁定。

use crate::customglyph::*;
use crate::quad::TripleLayerQuadAllocator;
use crate::termwindow::{UIItem, UIItemType};
use config::DimensionContext;
use window::color::LinearRgba;

/// thumb 顶端半圆：椭圆中心在底边中点，半径为半宽 × 整高
const TOP_CAP: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::Frac(1, 2), BlockCoord::One),
        radiuses: (BlockCoord::Frac(1, 2), BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

/// thumb 底端半圆：椭圆中心在顶边中点
const BOTTOM_CAP: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::Frac(1, 2), BlockCoord::Zero),
        radiuses: (BlockCoord::Frac(1, 2), BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

/// 悬停/拖动时向最大亮度靠拢的比例
const HOVER_LIGHTEN: f64 = 0.3;

/// thumb 在水平方向的落点
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbGeometry {
    /// 左缘（窗口像素坐标）
    pub x: f32,
    pub width: f32,
    /// 两端半圆的高度；thumb 太矮放不下两端时为 0（退化为矩形）
    pub cap: f32,
}

/// 由右侧 padding 的位置/宽度与期望宽度算出 thumb 几何：宽度夹在
/// `[1, padding]`，靠右并与右缘留出至多半个 thumb 宽的间隙。
pub fn thumb_geometry(
    padding_x: f32,
    padding: f32,
    desired: f32,
    thumb_height: f32,
) -> Option<ThumbGeometry> {
    let padding = padding.floor();
    if padding < 1. {
        return None;
    }
    let width = desired.floor().clamp(1., padding);
    let gap = ((padding - width) / 2.).min(width / 2.).floor().max(0.);
    let cap = (width / 2.).floor();
    let cap = if cap >= 1. && thumb_height >= 2. * cap {
        cap
    } else {
        0.
    };
    Some(ThumbGeometry {
        x: padding_x + padding - width - gap,
        width,
        cap,
    })
}

impl crate::TermWindow {
    /// 悬停在 thumb 上或正在拖动 thumb
    fn scroll_thumb_is_hot(&self) -> bool {
        let is_thumb = |item: &UIItem| matches!(item.item_type, UIItemType::ScrollThumb);
        let hovered = self.current_mouse_event.is_some()
            && self.last_ui_item.as_ref().map_or(false, is_thumb);
        let dragging = self
            .dragging
            .as_ref()
            .map_or(false, |(item, _)| is_thumb(item));
        hovered || dragging
    }

    /// 画细线化的 thumb；`padding_x/padding` 是右侧 padding 的左缘与宽度
    pub fn paint_scroll_thumb(
        &self,
        layers: &mut TripleLayerQuadAllocator,
        padding_x: f32,
        padding: f32,
        top: f32,
        height: f32,
        color: config::SrgbaTuple,
    ) -> anyhow::Result<()> {
        let desired = self
            .config
            .scroll_bar_thumb_width
            .evaluate_as_pixels(DimensionContext {
                dpi: self.dimensions.dpi as f32,
                pixel_max: padding,
                pixel_cell: self.render_metrics.cell_size.width as f32,
            });
        let Some(geom) = thumb_geometry(padding_x, padding, desired, height) else {
            return Ok(());
        };
        let color: LinearRgba = if self.scroll_thumb_is_hot() {
            color.lighten(HOVER_LIGHTEN).to_linear()
        } else {
            color.to_linear()
        };

        if geom.cap > 0. {
            let cap_size = euclid::size2(geom.width, geom.cap);
            self.poly_quad(
                layers,
                2,
                euclid::point2(geom.x, top),
                TOP_CAP,
                self.render_metrics.underline_height,
                cap_size,
                color,
            )?;
            self.poly_quad(
                layers,
                2,
                euclid::point2(geom.x, top + height - geom.cap),
                BOTTOM_CAP,
                self.render_metrics.underline_height,
                cap_size,
                color,
            )?;
        }
        self.filled_rectangle(
            layers,
            2,
            euclid::rect(geom.x, top + geom.cap, geom.width, height - 2. * geom.cap),
            color,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slim_thumb_hugs_the_right_edge() {
        // 10px padding、4px thumb：间隙取半个 thumb 宽（2px）
        let g = thumb_geometry(100., 10., 4., 50.).unwrap();
        assert_eq!(
            g,
            ThumbGeometry {
                x: 104.,
                width: 4.,
                cap: 2.,
            }
        );
        assert!(g.x + g.width <= 110.);
    }

    #[test]
    fn full_width_matches_upstream_rectangle() {
        // "100%" 的效果：铺满 padding、无间隙
        let g = thumb_geometry(100., 12., 12., 50.).unwrap();
        assert_eq!((g.x, g.width), (100., 12.));
    }

    #[test]
    fn width_is_clamped_to_the_padding() {
        let g = thumb_geometry(0., 8., 40., 50.).unwrap();
        assert_eq!((g.x, g.width), (0., 8.));
        let g = thumb_geometry(0., 8., 0.2, 50.).unwrap();
        assert_eq!(g.width, 1.);
        assert!(thumb_geometry(0., 0.5, 4., 50.).is_none());
    }

    #[test]
    fn short_thumb_drops_the_round_caps() {
        assert_eq!(thumb_geometry(0., 10., 4., 3.).unwrap().cap, 0.);
        assert_eq!(thumb_geometry(0., 10., 4., 4.).unwrap().cap, 2.);
        // 1px 宽时半圆不足 1px，同样退化为矩形
        assert_eq!(thumb_geometry(0., 10., 1., 50.).unwrap().cap, 0.);
    }
}
