use crate::customglyph::*;
use crate::termwindow::box_model::{Corners, SizedPoly};
use config::Dimension;

pub const TOP_LEFT_ROUNDED_CORNER: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::One, BlockCoord::One),
        radiuses: (BlockCoord::One, BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

pub const BOTTOM_LEFT_ROUNDED_CORNER: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::One, BlockCoord::Zero),
        radiuses: (BlockCoord::One, BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

pub const TOP_RIGHT_ROUNDED_CORNER: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::Zero, BlockCoord::One),
        radiuses: (BlockCoord::One, BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

pub const BOTTOM_RIGHT_ROUNDED_CORNER: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (BlockCoord::Zero, BlockCoord::Zero),
        radiuses: (BlockCoord::One, BlockCoord::One),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Fill,
}];

// fork: border arcs for the rounded corners above. Each arc is a
// `PolyStyle::Outline` stroke whose width is the `underline_height`
// handed to `poly_quad` (the border width of the edge the corner belongs
// to), and whose radius is pulled in by half that width so the whole
// stroke sits inside the corner, flush with the straight border runs.
//
// `hint()` in `BlockCoord::to_pixel` nudges integral offsets by -0.5, so
// the geometry is exact for odd border widths (1px being the common
// case) and off by half a pixel inward for even widths.

/// `max - width/2`: the arc radius.
const ARC_RADIUS: BlockCoord = BlockCoord::FracWithOffset(1, 1, LineScale::Div(-2));
/// `max + width/2`: the oval center on the `One` side (see below).
const ARC_CENTER_ONE: BlockCoord = BlockCoord::FracWithOffset(1, 1, LineScale::Div(2));
/// `0 + width/2`: the oval center on the `Zero` side (see below).
const ARC_CENTER_ZERO: BlockCoord = BlockCoord::FracWithOffset(0, 1, LineScale::Div(2));

/// Square corners stroke a `Circle`, whose center is rasterized exactly
/// where it is written.
pub const TOP_LEFT_ROUNDED_CORNER_ARC: &[Poly] = &[Poly {
    path: &[PolyCommand::Circle {
        center: (BlockCoord::One, BlockCoord::One),
        radius: ARC_RADIUS,
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

pub const BOTTOM_LEFT_ROUNDED_CORNER_ARC: &[Poly] = &[Poly {
    path: &[PolyCommand::Circle {
        center: (BlockCoord::One, BlockCoord::Zero),
        radius: ARC_RADIUS,
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

pub const TOP_RIGHT_ROUNDED_CORNER_ARC: &[Poly] = &[Poly {
    path: &[PolyCommand::Circle {
        center: (BlockCoord::Zero, BlockCoord::One),
        radius: ARC_RADIUS,
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

pub const BOTTOM_RIGHT_ROUNDED_CORNER_ARC: &[Poly] = &[Poly {
    path: &[PolyCommand::Circle {
        center: (BlockCoord::Zero, BlockCoord::Zero),
        radius: ARC_RADIUS,
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

// Non-square corners need an elliptical arc. `PolyCommand::Oval` is
// rasterized with its bounding box anchored at `center - cell size`
// rather than `center - radius` (it was only ever used with a radius of
// `One`), so the written center is pushed outward by the same half width
// that the radius is pulled in, which lands the ellipse back on the true
// corner center.
const TOP_LEFT_ROUNDED_CORNER_ARC_OVAL: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (ARC_CENTER_ONE, ARC_CENTER_ONE),
        radiuses: (ARC_RADIUS, ARC_RADIUS),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

const BOTTOM_LEFT_ROUNDED_CORNER_ARC_OVAL: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (ARC_CENTER_ONE, ARC_CENTER_ZERO),
        radiuses: (ARC_RADIUS, ARC_RADIUS),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

const TOP_RIGHT_ROUNDED_CORNER_ARC_OVAL: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (ARC_CENTER_ZERO, ARC_CENTER_ONE),
        radiuses: (ARC_RADIUS, ARC_RADIUS),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

const BOTTOM_RIGHT_ROUNDED_CORNER_ARC_OVAL: &[Poly] = &[Poly {
    path: &[PolyCommand::Oval {
        center: (ARC_CENTER_ZERO, ARC_CENTER_ZERO),
        radiuses: (ARC_RADIUS, ARC_RADIUS),
    }],
    intensity: BlockAlpha::Full,
    style: PolyStyle::Outline,
}];

/// fork: the border arc that matches a rounded corner fill poly, or
/// `None` when `fill` is not one of the four `*_ROUNDED_CORNER` shapes
/// (a custom corner shape then gets no arc rather than a wrong one).
/// `width`/`height` are the corner's pixel size: square corners get the
/// circular arc, anything else the elliptical one.
pub fn rounded_corner_arc(
    fill: &'static [Poly],
    width: f32,
    height: f32,
) -> Option<&'static [Poly]> {
    let square = width == height;
    let pick =
        |circle: &'static [Poly], oval: &'static [Poly]| Some(if square { circle } else { oval });
    if fill == TOP_LEFT_ROUNDED_CORNER {
        pick(
            TOP_LEFT_ROUNDED_CORNER_ARC,
            TOP_LEFT_ROUNDED_CORNER_ARC_OVAL,
        )
    } else if fill == TOP_RIGHT_ROUNDED_CORNER {
        pick(
            TOP_RIGHT_ROUNDED_CORNER_ARC,
            TOP_RIGHT_ROUNDED_CORNER_ARC_OVAL,
        )
    } else if fill == BOTTOM_LEFT_ROUNDED_CORNER {
        pick(
            BOTTOM_LEFT_ROUNDED_CORNER_ARC,
            BOTTOM_LEFT_ROUNDED_CORNER_ARC_OVAL,
        )
    } else if fill == BOTTOM_RIGHT_ROUNDED_CORNER {
        pick(
            BOTTOM_RIGHT_ROUNDED_CORNER_ARC,
            BOTTOM_RIGHT_ROUNDED_CORNER_ARC_OVAL,
        )
    } else {
        None
    }
}

/// fork: all four corners rounded with the same `radius`.
///
/// `Dimension::Pixels`/`Points` evaluate to the same pixel count on both
/// axes and give a true circle; `Cells`/`Percent` follow the cell width
/// on one axis and the cell height on the other and give an ellipse.
pub fn rounded_corners(radius: Dimension) -> Corners {
    Corners {
        top_left: SizedPoly {
            width: radius,
            height: radius,
            poly: TOP_LEFT_ROUNDED_CORNER,
        },
        top_right: SizedPoly {
            width: radius,
            height: radius,
            poly: TOP_RIGHT_ROUNDED_CORNER,
        },
        bottom_left: SizedPoly {
            width: radius,
            height: radius,
            poly: BOTTOM_LEFT_ROUNDED_CORNER,
        },
        bottom_right: SizedPoly {
            width: radius,
            height: radius,
            poly: BOTTOM_RIGHT_ROUNDED_CORNER,
        },
    }
}

/// fork: only the top two corners rounded. The bottom corners are
/// `SizedPoly::none()`, so the side borders run all the way down to the
/// bottom edge; the fancy tab bar keeps its own zero-width, 0.33-cell
/// placeholders there on purpose to stop its sides short.
pub fn top_rounded_corners(radius: Dimension) -> Corners {
    Corners {
        top_left: SizedPoly {
            width: radius,
            height: radius,
            poly: TOP_LEFT_ROUNDED_CORNER,
        },
        top_right: SizedPoly {
            width: radius,
            height: radius,
            poly: TOP_RIGHT_ROUNDED_CORNER,
        },
        bottom_left: SizedPoly::none(),
        bottom_right: SizedPoly::none(),
    }
}

/// fork: the mirror image of `top_rounded_corners`, for a tab bar placed
/// at the bottom of the window: only the bottom two corners are rounded.
pub fn bottom_rounded_corners(radius: Dimension) -> Corners {
    Corners {
        top_left: SizedPoly::none(),
        top_right: SizedPoly::none(),
        bottom_left: SizedPoly {
            width: radius,
            height: radius,
            poly: BOTTOM_LEFT_ROUNDED_CORNER,
        },
        bottom_right: SizedPoly {
            width: radius,
            height: radius,
            poly: BOTTOM_RIGHT_ROUNDED_CORNER,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILLS: [&[Poly]; 4] = [
        TOP_LEFT_ROUNDED_CORNER,
        TOP_RIGHT_ROUNDED_CORNER,
        BOTTOM_LEFT_ROUNDED_CORNER,
        BOTTOM_RIGHT_ROUNDED_CORNER,
    ];

    #[test]
    fn arc_lookup_follows_the_fill_poly() {
        assert_eq!(
            rounded_corner_arc(TOP_LEFT_ROUNDED_CORNER, 8., 8.),
            Some(TOP_LEFT_ROUNDED_CORNER_ARC)
        );
        assert_eq!(
            rounded_corner_arc(TOP_RIGHT_ROUNDED_CORNER, 8., 8.),
            Some(TOP_RIGHT_ROUNDED_CORNER_ARC)
        );
        assert_eq!(
            rounded_corner_arc(BOTTOM_LEFT_ROUNDED_CORNER, 8., 8.),
            Some(BOTTOM_LEFT_ROUNDED_CORNER_ARC)
        );
        assert_eq!(
            rounded_corner_arc(BOTTOM_RIGHT_ROUNDED_CORNER, 8., 8.),
            Some(BOTTOM_RIGHT_ROUNDED_CORNER_ARC)
        );
        // 非圆角形状（占位空 poly）不画弧
        assert_eq!(rounded_corner_arc(&[], 8., 8.), None);
    }

    #[test]
    fn square_corners_stroke_a_circle_and_others_an_oval() {
        for fill in FILLS {
            let square = rounded_corner_arc(fill, 8., 8.).unwrap();
            assert_eq!(square.len(), 1);
            assert_eq!(square[0].style, PolyStyle::Outline);
            assert!(matches!(square[0].path[0], PolyCommand::Circle { .. }));

            let oblong = rounded_corner_arc(fill, 5., 10.).unwrap();
            assert_eq!(oblong.len(), 1);
            assert_eq!(oblong[0].style, PolyStyle::Outline);
            assert!(matches!(oblong[0].path[0], PolyCommand::Oval { .. }));
        }
    }

    /// 1px 边框、8×8 角：圆心 (8,8)、半径 7.5，描边覆盖半径 [7,8]，
    /// 与直边的 [0,1] 像素带齐平。
    #[test]
    fn circle_arc_sits_inside_the_corner_for_one_pixel_border() {
        let (w, h, line) = (8usize, 8usize, 1.0f32);
        let square = w.min(h);
        let PolyCommand::Circle {
            center: (cx, cy),
            radius,
        } = TOP_LEFT_ROUNDED_CORNER_ARC[0].path[0]
        else {
            panic!("expected a circle");
        };
        assert_eq!(cx.to_pixel(w, line, square), 8.);
        assert_eq!(cy.to_pixel(h, line, square), 8.);
        assert_eq!(radius.to_pixel(square, line, square), 7.5);
    }

    /// 椭圆弧按 `PolyCommand::to_skia` 的 Oval 规则（包围盒左上 =
    /// center - 格子尺寸）折算后，圆心必须落回角的真实圆心，半径内缩半线宽。
    #[test]
    fn oval_arc_lands_on_the_true_center_for_one_pixel_border() {
        let (w, h, line) = (5usize, 10usize, 1.0f32);
        let square = w.min(h);
        let cases = [
            (TOP_LEFT_ROUNDED_CORNER_ARC_OVAL, (5., 10.)),
            (TOP_RIGHT_ROUNDED_CORNER_ARC_OVAL, (0., 10.)),
            (BOTTOM_LEFT_ROUNDED_CORNER_ARC_OVAL, (5., 0.)),
            (BOTTOM_RIGHT_ROUNDED_CORNER_ARC_OVAL, (0., 0.)),
        ];
        for (arc, expected_center) in cases {
            let PolyCommand::Oval {
                center: (cx, cy),
                radiuses: (rx, ry),
            } = arc[0].path[0]
            else {
                panic!("expected an oval");
            };
            let left = cx.to_pixel(w, line, square) - w as f32;
            let top = cy.to_pixel(h, line, square) - h as f32;
            let radius_x = rx.to_pixel(w, line, square);
            let radius_y = ry.to_pixel(h, line, square);
            assert_eq!((left + radius_x, top + radius_y), expected_center);
            assert_eq!((radius_x, radius_y), (4.5, 9.5));
        }
    }

    #[test]
    fn rounded_corners_use_one_radius_on_both_axes() {
        let radius = Dimension::Pixels(8.);
        let corners = rounded_corners(radius);
        let pieces = [
            (corners.top_left, TOP_LEFT_ROUNDED_CORNER),
            (corners.top_right, TOP_RIGHT_ROUNDED_CORNER),
            (corners.bottom_left, BOTTOM_LEFT_ROUNDED_CORNER),
            (corners.bottom_right, BOTTOM_RIGHT_ROUNDED_CORNER),
        ];
        for (piece, fill) in pieces {
            assert_eq!(piece.width, radius);
            assert_eq!(piece.height, radius);
            assert_eq!(piece.poly, fill);
        }
    }

    #[test]
    fn top_rounded_corners_leave_the_bottom_square() {
        let radius = Dimension::Points(6.);
        let corners = top_rounded_corners(radius);
        assert_eq!(corners.top_left.poly, TOP_LEFT_ROUNDED_CORNER);
        assert_eq!(corners.top_right.poly, TOP_RIGHT_ROUNDED_CORNER);
        assert_eq!(
            (corners.top_left.width, corners.top_left.height),
            (radius, radius)
        );
        // 下两角无占位：侧边直边一直画到底边
        assert_eq!(corners.bottom_left, SizedPoly::none());
        assert_eq!(corners.bottom_right, SizedPoly::none());
    }

    #[test]
    fn bottom_rounded_corners_mirror_the_top_ones() {
        let radius = Dimension::Pixels(8.);
        let corners = bottom_rounded_corners(radius);
        assert_eq!(corners.bottom_left.poly, BOTTOM_LEFT_ROUNDED_CORNER);
        assert_eq!(corners.bottom_right.poly, BOTTOM_RIGHT_ROUNDED_CORNER);
        assert_eq!(
            (corners.bottom_right.width, corners.bottom_right.height),
            (radius, radius)
        );
        assert_eq!(corners.top_left, SizedPoly::none());
        assert_eq!(corners.top_right, SizedPoly::none());
    }
}
