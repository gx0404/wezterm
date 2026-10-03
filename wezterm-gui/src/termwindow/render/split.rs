use crate::termwindow::render::TripleLayerQuadAllocator;
use crate::termwindow::{UIItem, UIItemType};
use config::DimensionContext;
use mux::pane::Pane;
use mux::tab::{PositionedSplit, SplitDirection};
use std::sync::Arc;

/// fork: `(offset, thickness)` of the split line. The offset is relative
/// to the cell midpoint where upstream starts the line. Without
/// `split_thickness` the line keeps the underline thickness and position;
/// with it the line is clamped to `[1, cell]` pixels and stays centred
/// on the same centre line.
fn split_line_metrics(configured: Option<f32>, underline: f32, cell: f32) -> (f32, f32) {
    match configured {
        None => (0., underline),
        Some(thickness) => {
            let thickness = thickness.floor().clamp(1., cell.max(1.));
            (((underline - thickness) / 2.).floor(), thickness)
        }
    }
}

impl crate::TermWindow {
    pub fn paint_split(
        &mut self,
        layers: &mut TripleLayerQuadAllocator,
        split: &PositionedSplit,
        pane: &Arc<dyn Pane>,
    ) -> anyhow::Result<()> {
        let palette = self.pane_palette(pane);
        let foreground = palette.split.to_linear();
        let cell_width = self.render_metrics.cell_size.width as f32;
        let cell_height = self.render_metrics.cell_size.height as f32;

        let border = self.get_os_border();
        let first_row_offset = if self.show_tab_bar && !self.config.tab_bar_at_bottom {
            self.tab_bar_pixel_height()?
        } else {
            0.
        } + border.top.get() as f32;

        let (padding_left, padding_top) = self.padding_left_top();

        let pos_y = split.top as f32 * cell_height + first_row_offset + padding_top;
        let pos_x = split.left as f32 * cell_width + padding_left + border.left.get() as f32;

        // fork: the thickness comes from `split_thickness` when set
        let cell_extent = if split.direction == SplitDirection::Horizontal {
            cell_width
        } else {
            cell_height
        };
        let configured = self.config.split_thickness.map(|dim| {
            dim.evaluate_as_pixels(DimensionContext {
                dpi: self.dimensions.dpi as f32,
                pixel_max: cell_extent,
                pixel_cell: cell_extent,
            })
        });
        let (line_offset, line_thickness) = split_line_metrics(
            configured,
            self.render_metrics.underline_height as f32,
            cell_extent,
        );

        if split.direction == SplitDirection::Horizontal {
            self.filled_rectangle(
                layers,
                2,
                euclid::rect(
                    pos_x + (cell_width / 2.0) + line_offset,
                    pos_y - (cell_height / 2.0),
                    line_thickness,
                    (1. + split.size as f32) * cell_height,
                ),
                foreground,
            )?;
            self.ui_items.push(UIItem {
                x: border.left.get() as usize
                    + padding_left as usize
                    + (split.left * cell_width as usize),
                width: cell_width as usize,
                y: padding_top as usize
                    + first_row_offset as usize
                    + split.top * cell_height as usize,
                height: split.size * cell_height as usize,
                item_type: UIItemType::Split(split.clone()),
            });
        } else {
            self.filled_rectangle(
                layers,
                2,
                euclid::rect(
                    pos_x - (cell_width / 2.0),
                    pos_y + (cell_height / 2.0) + line_offset,
                    (1.0 + split.size as f32) * cell_width,
                    line_thickness,
                ),
                foreground,
            )?;
            self.ui_items.push(UIItem {
                x: border.left.get() as usize
                    + padding_left as usize
                    + (split.left * cell_width as usize),
                width: split.size * cell_width as usize,
                y: padding_top as usize
                    + first_row_offset as usize
                    + split.top * cell_height as usize,
                height: cell_height as usize,
                item_type: UIItemType::Split(split.clone()),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::split_line_metrics;

    #[test]
    fn unset_keeps_the_underline_thickness() {
        assert_eq!(split_line_metrics(None, 2., 10.), (0., 2.));
    }

    #[test]
    fn configured_thickness_stays_on_the_same_centre_line() {
        // underline 1px: centre at +0.5; a 3px line starts one pixel earlier
        assert_eq!(split_line_metrics(Some(3.), 1., 10.), (-1., 3.));
        // same thickness as the underline: identical to the default
        assert_eq!(split_line_metrics(Some(2.), 2., 10.), (0., 2.));
    }

    #[test]
    fn thickness_is_clamped_to_the_cell() {
        assert_eq!(split_line_metrics(Some(40.), 1., 9.).1, 9.);
        assert_eq!(split_line_metrics(Some(0.4), 1., 9.).1, 1.);
    }
}
