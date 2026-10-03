use crate::color::LinearRgba;
use crate::glyphcache::LoadState;
use crate::quad::{QuadAllocator, QuadTrait};
use crate::termwindow::RenderState;
use crate::utilsprites::RenderMetrics;
use crate::Dimensions;
use ::window::bitmaps::TextureRect;
use anyhow::Context;
use config::{
    BackgroundHorizontalAlignment, BackgroundLayer, BackgroundRepeat, BackgroundSize,
    BackgroundSource, BackgroundVerticalAlignment, ConfigHandle, DimensionContext, Gradient,
    GradientOrientation,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use termwiz::image::{ImageData, ImageDataType};
use wezterm_term::StableRowIndex;

lazy_static::lazy_static! {
    static ref IMAGE_CACHE: Mutex<HashMap<String, CachedImage>> = Mutex::new(HashMap::new());
    static ref GRADIENT_CACHE: Mutex<Vec<CachedGradient>> = Mutex::new(vec![]);
    // fork: solid color layers are drawn as plain colored quads (see
    // `solid_layer_color`) and never touch the texture atlas. They used to
    // upload a square texture about the size of the window (~1728^2 at
    // 2560x1440), which together with a wallpaper pushed the atlas to
    // 4096^2 with several full re-renders on the first frame.
    // `LoadedBackgroundLayer` still wants an image, so they all share this
    // 1x1 placeholder, which is never uploaded.
    static ref COLOR_LAYER_PLACEHOLDER: Arc<ImageData> = Arc::new(ImageData::with_data(
        ImageDataType::new_single_frame(1, 1, vec![0, 0, 0, 0]),
    ));
}

// fork: monotonically increasing access ticks backing LRU eviction for
// the image cache above
static IMAGE_CACHE_TICK: AtomicU64 = AtomicU64::new(1);

struct CachedGradient {
    g: Gradient,
    width: u32,
    height: u32,
    image: Arc<ImageData>,
    marked: bool,
}

impl CachedGradient {
    fn compute(g: &Gradient, width: u32, height: u32) -> anyhow::Result<Arc<ImageData>> {
        let grad = g
            .build()
            .with_context(|| format!("building gradient {:?}", g))?;

        let mut imgbuf = image::RgbaImage::new(width, height);
        let fw = width as f64;
        let fh = height as f64;

        fn to_pixel(c: colorgrad::Color) -> image::Rgba<u8> {
            image::Rgba(c.to_rgba8())
        }

        // Map t which is in range [a, b] to range [c, d]
        fn remap(t: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
            (t - a) * ((d - c) / (b - a)) + c
        }

        let (dmin, dmax) = grad.domain();

        let mut rng = fastrand::Rng::new();

        // We add some randomness to the position that we use to
        // index into the color gradient, so that we can avoid
        // visible color banding.  The default 64 was selected
        // because it it was the smallest value on my mac where
        // the banding wasn't obvious.
        let noise_amount = g.noise.unwrap_or_else(|| {
            if matches!(g.orientation, GradientOrientation::Radial { .. }) {
                16
            } else {
                64
            }
        });

        fn noise(rng: &mut fastrand::Rng, noise_amount: usize) -> f64 {
            if noise_amount == 0 {
                0.
            } else {
                rng.usize(0..noise_amount) as f64 * -1.
            }
        }

        match g.orientation {
            GradientOrientation::Horizontal => {
                for (x, _, pixel) in imgbuf.enumerate_pixels_mut() {
                    *pixel = to_pixel(grad.at(remap(
                        x as f64 + noise(&mut rng, noise_amount),
                        0.0,
                        fw,
                        dmin,
                        dmax,
                    )));
                }
            }
            GradientOrientation::Vertical => {
                for (_, y, pixel) in imgbuf.enumerate_pixels_mut() {
                    *pixel = to_pixel(grad.at(remap(
                        y as f64 + noise(&mut rng, noise_amount),
                        0.0,
                        fh,
                        dmin,
                        dmax,
                    )));
                }
            }
            GradientOrientation::Linear { angle } => {
                let angle = angle.unwrap_or(0.0).to_radians();
                for (x, y, pixel) in imgbuf.enumerate_pixels_mut() {
                    let (x, y) = (x as f64, y as f64);
                    let (x, y) = (x - fw / 2., y - fh / 2.);
                    let t = x * f64::cos(angle) - y * f64::sin(angle);
                    *pixel = to_pixel(grad.at(remap(
                        t + noise(&mut rng, noise_amount),
                        -fw / 2.,
                        fw / 2.,
                        dmin,
                        dmax,
                    )));
                }
            }
            GradientOrientation::Radial { radius, cx, cy } => {
                let radius = fw * radius.unwrap_or(0.5);
                let cx = fw * cx.unwrap_or(0.5);
                let cy = fh * cy.unwrap_or(0.5);

                for (x, y, pixel) in imgbuf.enumerate_pixels_mut() {
                    let x = x as f64;
                    let y = y as f64;

                    // If we are close to the center, stop applying noise,
                    // as the noise can wrap around and start using the
                    // color from the other end of the gradient and look weird
                    let nx = if ((cx - x).abs() as usize) < noise_amount {
                        0.
                    } else {
                        noise(&mut rng, noise_amount)
                    };
                    let ny = if ((cy - y).abs() as usize) < noise_amount {
                        0.
                    } else {
                        noise(&mut rng, noise_amount)
                    };

                    let t = (nx + (x - cx).powi(2) + (ny + y - cy).powi(2)).sqrt() / radius;
                    *pixel = to_pixel(grad.at(t));
                }
            }
        }

        let data = imgbuf.into_vec();
        let image = Arc::new(ImageData::with_data(ImageDataType::new_single_frame(
            width, height, data,
        )));

        Ok(image)
    }

    fn load(g: &Gradient, width: u32, height: u32) -> anyhow::Result<Arc<ImageData>> {
        let mut cache = GRADIENT_CACHE.lock().unwrap();

        if let Some(entry) = cache
            .iter_mut()
            .find(|entry| entry.g == *g && entry.width == width && entry.height == height)
        {
            entry.marked = false;
            return Ok(Arc::clone(&entry.image));
        }

        let image = Self::compute(g, width, height)?;

        cache.push(Self {
            g: g.clone(),
            width,
            height,
            image: Arc::clone(&image),
            marked: false,
        });
        Ok(image)
    }

    fn mark() {
        let mut cache = GRADIENT_CACHE.lock().unwrap();
        for entry in cache.iter_mut() {
            entry.marked = true;
        }
    }

    fn sweep() {
        let mut cache = GRADIENT_CACHE.lock().unwrap();
        cache.retain(|entry| !entry.marked);
    }
}

/// fork: the color a solid `BackgroundSource::Color` layer is painted
/// with: the layer color, linearized, with its alpha multiplied by the
/// layer opacity. This matches what the shader produced for the old
/// uniform texture (texel alpha times the `opacity` carried in the quad's
/// foreground alpha). None for image and gradient layers.
pub(crate) fn solid_layer_color(def: &BackgroundLayer) -> Option<LinearRgba> {
    match &def.source {
        BackgroundSource::Color(color) => Some((**color).to_linear().mul_alpha(def.opacity)),
        BackgroundSource::Gradient(_) | BackgroundSource::File(_) => None,
    }
}

struct CachedImage {
    modified: SystemTime,
    image: Arc<ImageData>,
    marked: bool,
    speed: f32,
    // fork: LRU tick, bumped on every access; drives prune_image_cache_lru
    last_used: u64,
}

impl CachedImage {
    fn load(path: &str, speed: f32) -> anyhow::Result<Arc<ImageData>> {
        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .with_context(|| format!("getting metadata for {}", path))?;
        let mut cache = IMAGE_CACHE.lock().unwrap();
        if let Some(cached) = cache.get_mut(path) {
            if cached.modified == modified && cached.speed == speed {
                cached.marked = false;
                cached.last_used = IMAGE_CACHE_TICK.fetch_add(1, Ordering::Relaxed);
                return Ok(Arc::clone(&cached.image));
            }
        }

        let data = std::fs::read(path)
            .with_context(|| format!("Failed to load window_background_image {}", path))?;
        log::trace!("loaded {}", path);
        let mut data = ImageDataType::EncodedFile(data);
        data.adjust_speed(speed);
        let image = Arc::new(ImageData::with_data(data));

        cache.insert(
            path.to_string(),
            Self {
                modified,
                image: Arc::clone(&image),
                marked: false,
                speed,
                last_used: IMAGE_CACHE_TICK.fetch_add(1, Ordering::Relaxed),
            },
        );

        Ok(image)
    }

    fn mark() {
        let mut cache = IMAGE_CACHE.lock().unwrap();
        for entry in cache.values_mut() {
            entry.marked = true;
        }
    }

    fn sweep() {
        let mut cache = IMAGE_CACHE.lock().unwrap();
        cache.retain(|k, entry| {
            if entry.marked {
                log::trace!("Unloading {} from cache", k);
            }
            !entry.marked
        });
    }
}

// fork: the wallpaper preview overlay can pull dozens of full-size files
// through IMAGE_CACHE in a single session; bound it by evicting the
// least-recently-used entries. Evicting an entry only drops the cached
// bytes; live windows keep their Arc<ImageData> and the next config
// reload re-reads the file.
pub(crate) fn prune_image_cache_lru(keep: usize) {
    let mut cache = IMAGE_CACHE.lock().unwrap();
    if cache.len() <= keep {
        return;
    }
    let mut usage: Vec<(u64, String)> = cache
        .iter()
        .map(|(path, entry)| (entry.last_used, path.clone()))
        .collect();
    usage.sort_unstable();
    let evict = usage.len() - keep;
    for (_, path) in &usage[..evict] {
        log::trace!("Pruning {} from image cache", path);
        cache.remove(path);
    }
}

// fork: called when a wallpaper preview session ends; keeps only entries
// still referenced by the resulting layer stack, dropping the images the
// user merely scrolled past.
pub(crate) fn prune_image_cache_except(layers: &[LoadedBackgroundLayer]) {
    let mut cache = IMAGE_CACHE.lock().unwrap();
    cache.retain(|path, _| {
        layers.iter().any(|layer| match &layer.def.source {
            BackgroundSource::File(source) => source.path == *path,
            _ => false,
        })
    });
}

#[derive(Clone)]
pub struct LoadedBackgroundLayer {
    pub source: Arc<ImageData>,
    pub def: BackgroundLayer,
}

// fork: crate-visible so the wallpaper overlay can build preview layers
// without touching the config (batch 13)
pub(crate) fn load_background_layer(
    layer: &BackgroundLayer,
    dimensions: &Dimensions,
    render_metrics: &RenderMetrics,
) -> anyhow::Result<LoadedBackgroundLayer> {
    let h_context = DimensionContext {
        dpi: dimensions.dpi as f32,
        pixel_max: dimensions.pixel_width as f32,
        pixel_cell: render_metrics.cell_size.width as f32,
    };
    let v_context = DimensionContext {
        dpi: dimensions.dpi as f32,
        pixel_max: dimensions.pixel_height as f32,
        pixel_cell: render_metrics.cell_size.height as f32,
    };

    let data = match &layer.source {
        BackgroundSource::Gradient(g) => {
            let mut width = match layer.width {
                BackgroundSize::Dimension(d) => d.evaluate_as_pixels(h_context),
                unsup => anyhow::bail!(
                    "{unsup:?} is not implemented for background gradients. \
                     Use e.g. `width = '100%'` instead"
                ),
            } as u32;
            let mut height = match layer.height {
                BackgroundSize::Dimension(d) => d.evaluate_as_pixels(v_context),
                unsup => anyhow::bail!(
                    "{unsup:?} is not implemented for background gradients. \
                     Use e.g. `height = '100%'` instead"
                ),
            } as u32;

            if matches!(g.orientation, GradientOrientation::Radial { .. }) {
                // To simplify the math, we compute a perfect circle
                // for the radial gradient, and let the texture sampler
                // perturb it to fill the window
                width = width.min(height);
                height = height.min(width);
            }

            CachedGradient::load(g, width, height)?
        }
        BackgroundSource::Color(_) => {
            // fork: the layer is painted as a solid quad by
            // render_background (see `solid_layer_color`), so there is no
            // image to build; only the size validation remains.
            match layer.width {
                BackgroundSize::Dimension(_) => {}
                unsup => anyhow::bail!(
                    "{unsup:?} is not implemented for background color. \
                     Use e.g. `width = '100%'` instead"
                ),
            }
            match layer.height {
                BackgroundSize::Dimension(_) => {}
                unsup => anyhow::bail!(
                    "{unsup:?} is not implemented for background color. \
                     Use e.g. `height = '100%'` instead"
                ),
            }
            Arc::clone(&COLOR_LAYER_PLACEHOLDER)
        }
        BackgroundSource::File(source) => CachedImage::load(&source.path, source.speed)?,
    };

    Ok(LoadedBackgroundLayer {
        source: data,
        def: layer.clone(),
    })
}

pub fn load_background_image(
    config: &ConfigHandle,
    dimensions: &Dimensions,
    render_metrics: &RenderMetrics,
) -> Vec<LoadedBackgroundLayer> {
    let mut layers = vec![];
    for layer in &config.background {
        let load_start = std::time::Instant::now();
        match load_background_layer(layer, dimensions, render_metrics) {
            Ok(layer) => {
                log::trace!("loaded layer in {:?}", load_start.elapsed());
                layers.push(layer);
            }
            Err(err) => {
                log::error!("Failed to load background: {:#}", err);
            }
        }
    }
    layers
}

pub fn reload_background_image(
    config: &ConfigHandle,
    existing: &[LoadedBackgroundLayer],
    dimensions: &Dimensions,
    render_metrics: &RenderMetrics,
) -> Vec<LoadedBackgroundLayer> {
    // We want to reuse the existing version of the image where possible
    // so that the textures we may have cached can be re-used and so that
    // animation state can be preserved across the reload.
    let map: HashMap<_, _> = existing
        .iter()
        .map(|layer| (layer.source.hash(), &layer.source))
        .collect();

    CachedImage::mark();
    CachedGradient::mark();

    let result = load_background_image(config, dimensions, render_metrics)
        .into_iter()
        .map(|mut layer| {
            let hash = layer.source.hash();

            if let Some(existing) = map.get(&hash) {
                layer.source = Arc::clone(existing);
            }

            layer
        })
        .collect();

    CachedImage::sweep();
    CachedGradient::sweep();

    result
}

impl crate::TermWindow {
    pub fn render_backgrounds(
        &self,
        bg_color: LinearRgba,
        top: StableRowIndex,
    ) -> anyhow::Result<bool> {
        let gl_state = self.render_state.as_ref().unwrap();
        let mut layer_idx = -127;
        let mut loaded_any = false;
        for layer in self.window_background.iter() {
            if self.render_background(gl_state, bg_color, layer, layer_idx, top)? {
                loaded_any = true;
                layer_idx = layer_idx.saturating_add(1);
            }
        }
        Ok(loaded_any)
    }

    fn render_background(
        &self,
        gl_state: &RenderState,
        bg_color: LinearRgba,
        layer: &LoadedBackgroundLayer,
        layer_index: i8,
        top: StableRowIndex,
    ) -> anyhow::Result<bool> {
        let render_layer = gl_state.layer_for_zindex(layer_index)?;
        let vbs = render_layer.vb.borrow();
        let mut layer0 = vbs[0].map();

        let pixel_width = self.dimensions.pixel_width as f32;
        let pixel_height = self.dimensions.pixel_height as f32;

        // fork: a Color layer is a solid quad and needs no atlas space.
        // Its size can only be a Dimension (load_background_layer rejects
        // Contain/Cover), so the texture size below only feeds the unused
        // aspect computations; the window size keeps them finite.
        let (fill, tex_width, tex_height) = match solid_layer_color(&layer.def) {
            Some(color) => (
                LayerFill::Solid {
                    color,
                    coords: gl_state.util_sprites.filled_box.texture_coords(),
                },
                pixel_width,
                pixel_height,
            ),
            None => {
                let (sprite, next_due, load_state) = gl_state
                    .glyph_cache
                    .borrow_mut()
                    .cached_image(&layer.source, None, self.allow_images)?;
                self.update_next_frame_time(next_due);

                if load_state == LoadState::Loading {
                    return Ok(false);
                }

                (
                    LayerFill::Image {
                        color: bg_color.mul_alpha(layer.def.opacity),
                        coords: sprite.texture_coords(),
                    },
                    sprite.coords.width() as f32,
                    sprite.coords.height() as f32,
                )
            }
        };

        let scale_width = pixel_width / tex_width as f32;
        let scale_height = pixel_height / tex_height as f32;

        let h_context = DimensionContext {
            dpi: self.dimensions.dpi as f32,
            pixel_max: pixel_width,
            pixel_cell: self.render_metrics.cell_size.width as f32,
        };
        let v_context = DimensionContext {
            dpi: self.dimensions.dpi as f32,
            pixel_max: pixel_height,
            pixel_cell: self.render_metrics.cell_size.height as f32,
        };

        // log::info!("tex {tex_width}x{tex_height} aspect={aspect}");

        // Compute the smallest aspect-preserved size that will fit the space
        let (min_aspect_width, min_aspect_height) = {
            let scale = scale_width.min(scale_height);
            (tex_width * scale, tex_height * scale)
        };
        // Compute the largest aspect-preserved size that will fill the space
        let (max_aspect_width, max_aspect_height) = {
            let scale = scale_width.max(scale_height);
            (tex_width * scale, tex_height * scale)
        };

        let width = match layer.def.width {
            BackgroundSize::Contain => min_aspect_width as f32,
            BackgroundSize::Cover => max_aspect_width as f32,
            BackgroundSize::Dimension(n) => n.evaluate_as_pixels(h_context),
        };

        let height = match layer.def.height {
            BackgroundSize::Contain => min_aspect_height as f32,
            BackgroundSize::Cover => max_aspect_height as f32,
            BackgroundSize::Dimension(n) => n.evaluate_as_pixels(v_context),
        };

        let mut origin_x = pixel_width / -2.;
        let top_pixel = pixel_height / -2.;
        let mut origin_y = top_pixel;

        match layer.def.vertical_align {
            BackgroundVerticalAlignment::Top => {}
            BackgroundVerticalAlignment::Bottom => {
                origin_y += pixel_height - height;
            }
            BackgroundVerticalAlignment::Middle => {
                origin_y += (pixel_height - height) / 2.;
            }
        }
        match layer.def.horizontal_align {
            BackgroundHorizontalAlignment::Left => {}
            BackgroundHorizontalAlignment::Right => {
                origin_x += pixel_width - width;
            }
            BackgroundHorizontalAlignment::Center => {
                origin_x += (pixel_width - width) / 2.;
            }
        }

        let vertical_offset = layer
            .def
            .vertical_offset
            .map(|d| d.evaluate_as_pixels(v_context))
            .unwrap_or(0.);
        origin_y += vertical_offset;

        let horizontal_offset = layer
            .def
            .horizontal_offset
            .map(|d| d.evaluate_as_pixels(h_context))
            .unwrap_or(0.);
        origin_x += horizontal_offset;

        let repeat_x = layer
            .def
            .repeat_x_size
            .map(|size| size.evaluate_as_pixels(h_context))
            .unwrap_or(width);
        let repeat_y = layer
            .def
            .repeat_y_size
            .map(|size| size.evaluate_as_pixels(v_context))
            .unwrap_or(height);

        // log::info!("computed {width}x{height}");

        let mut start_tile = 0;
        if let Some(factor) = layer.def.attachment.scroll_factor() {
            let distance = top as f32 * self.render_metrics.cell_size.height as f32 * factor;
            let num_tiles = distance / repeat_y;
            origin_y -= (num_tiles.fract() * repeat_y).floor();
            start_tile = num_tiles.floor() as usize;
        }

        let limit_y = top_pixel + pixel_height;

        let mut emitted = false;

        for y_step in start_tile.. {
            let offset_y = (y_step - start_tile) as f32 * repeat_y;
            let origin_y = origin_y + offset_y;
            if origin_y >= limit_y
                || (y_step > start_tile && layer.def.repeat_y == BackgroundRepeat::NoRepeat)
            {
                break;
            }

            for x_step in 0.. {
                let offset_x = x_step as f32 * repeat_x;
                if offset_x >= pixel_width
                    || (x_step > 0 && layer.def.repeat_x == BackgroundRepeat::NoRepeat)
                {
                    break;
                }
                let origin_x = origin_x + offset_x;
                let mut quad = layer0.allocate()?;
                emitted = true;
                // log::info!("quad {origin_x},{origin_y} {width}x{height}");
                quad.set_position(origin_x, origin_y, origin_x + width, origin_y + height);

                match &fill {
                    LayerFill::Solid { color, coords } => {
                        // Mirroring a uniform color is a no-op
                        quad.set_texture(*coords);
                        quad.set_is_background();
                        quad.set_hsv(Some(layer.def.hsb));
                        quad.set_fg_color(*color);
                    }
                    LayerFill::Image { color, coords } => {
                        let mut x1 = coords.min_x();
                        let mut x2 = coords.max_x();
                        let mut y1 = coords.min_y();
                        let mut y2 = coords.max_y();
                        if layer.def.repeat_x == BackgroundRepeat::Mirror && x_step % 2 == 1 {
                            std::mem::swap(&mut x1, &mut x2);
                        }
                        if layer.def.repeat_y == BackgroundRepeat::Mirror && y_step % 2 == 1 {
                            std::mem::swap(&mut y1, &mut y2);
                        }

                        quad.set_texture_discrete(x1, x2, y1, y2);
                        quad.set_is_background_image();
                        quad.set_hsv(Some(layer.def.hsb));
                        quad.set_fg_color(*color);
                    }
                }
            }
        }

        Ok(emitted)
    }
}

/// fork: how render_background fills the quads of one layer
enum LayerFill {
    /// Color layer: `IS_SOLID_COLOR` quad, no atlas sprite
    Solid {
        color: LinearRgba,
        coords: TextureRect,
    },
    /// Image or gradient layer: atlas sprite tinted by the layer opacity
    Image {
        color: LinearRgba,
        coords: TextureRect,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Dimension, ImageFileSource, ImageFileSourceWrap};
    use termwiz::color::SrgbaTuple;
    use wezterm_font::units::PixelLength;

    fn layer(source: BackgroundSource, size: BackgroundSize) -> BackgroundLayer {
        BackgroundLayer {
            source,
            origin: Default::default(),
            attachment: Default::default(),
            repeat_x: Default::default(),
            repeat_x_size: None,
            repeat_y: Default::default(),
            repeat_y_size: None,
            vertical_align: Default::default(),
            vertical_offset: None,
            horizontal_align: Default::default(),
            horizontal_offset: None,
            opacity: 1.0,
            hsb: Default::default(),
            width: size,
            height: size,
        }
    }

    fn color_layer(color: SrgbaTuple, opacity: f32) -> BackgroundLayer {
        BackgroundLayer {
            opacity,
            ..layer(
                BackgroundSource::Color(color.into()),
                BackgroundSize::Dimension(Dimension::Percent(1.0)),
            )
        }
    }

    fn dims() -> (Dimensions, RenderMetrics) {
        let dimensions = Dimensions {
            pixel_width: 2560,
            pixel_height: 1440,
            dpi: 96,
        };
        let metrics = RenderMetrics {
            descender: PixelLength::new(0.),
            descender_row: 0,
            descender_plus_two: 0,
            underline_height: 1,
            strike_row: 0,
            cell_size: ::window::Size::new(10, 20),
        };
        (dimensions, metrics)
    }

    #[test]
    fn color_layers_do_not_build_window_sized_images() {
        let (dimensions, metrics) = dims();
        let mask = load_background_layer(
            &color_layer(SrgbaTuple(0.16, 0.16, 0.16, 1.0), 0.92),
            &dimensions,
            &metrics,
        )
        .unwrap();
        let other = load_background_layer(
            &color_layer(SrgbaTuple(1.0, 0.0, 0.0, 1.0), 1.0),
            &dimensions,
            &metrics,
        )
        .unwrap();
        // every color layer shares the 1x1 placeholder: nothing is
        // allocated per color or per window size, and nothing reaches
        // the atlas
        assert!(Arc::ptr_eq(&mask.source, &COLOR_LAYER_PLACEHOLDER));
        assert!(Arc::ptr_eq(&other.source, &COLOR_LAYER_PLACEHOLDER));
        match &*COLOR_LAYER_PLACEHOLDER.data() {
            ImageDataType::Rgba8 { width, height, .. } => assert_eq!((*width, *height), (1, 1)),
            other => panic!("unexpected placeholder {:?}", other),
        }
    }

    #[test]
    fn color_layers_still_require_explicit_dimensions() {
        let (dimensions, metrics) = dims();
        for size in [BackgroundSize::Cover, BackgroundSize::Contain] {
            let def = layer(
                BackgroundSource::Color(SrgbaTuple(0., 0., 0., 1.).into()),
                size,
            );
            assert!(load_background_layer(&def, &dimensions, &metrics).is_err());
        }
    }

    #[test]
    fn solid_color_is_linearized_and_multiplied_by_opacity() {
        let def = color_layer(SrgbaTuple(1.0, 0.0, 0.0, 0.5), 0.5);
        let color = solid_layer_color(&def).expect("color layer paints a solid quad");
        assert_eq!(color, LinearRgba(1.0, 0.0, 0.0, 0.25));

        // sRGB -> linear on the color channels, alpha stays linear
        let def = color_layer(SrgbaTuple(0.5, 0.5, 0.5, 1.0), 1.0);
        let LinearRgba(r, g, b, a) = solid_layer_color(&def).unwrap();
        assert!((r - 0.21404).abs() < 1e-4, "{}", r);
        assert_eq!((r, r, a), (g, b, 1.0));
    }

    #[test]
    fn image_layers_keep_the_texture_path() {
        let def = layer(
            BackgroundSource::File(ImageFileSourceWrap::from(ImageFileSource {
                path: "/nonexistent.png".to_string(),
                speed: 1.0,
            })),
            BackgroundSize::Cover,
        );
        assert!(solid_layer_color(&def).is_none());
    }
}
