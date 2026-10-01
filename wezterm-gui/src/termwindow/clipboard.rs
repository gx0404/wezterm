use crate::termwindow::TermWindowNotif;
use crate::TermWindow;
use anyhow::Context as _;
use config::keyassignment::{ClipboardCopyDestination, ClipboardPasteSource};
use config::ClipboardImagePasteMode;
use mux::pane::{Pane, PaneId};
use mux::Mux;
use std::convert::TryInto;
use std::sync::Arc;
use window::{Clipboard, ClipboardImage, ClipboardImageFormat, WindowOps};

impl TermWindow {
    pub fn copy_to_clipboard(&self, clipboard: ClipboardCopyDestination, text: String) {
        let clipboard = match clipboard {
            ClipboardCopyDestination::Clipboard => [Some(Clipboard::Clipboard), None],
            ClipboardCopyDestination::PrimarySelection => [Some(Clipboard::PrimarySelection), None],
            ClipboardCopyDestination::ClipboardAndPrimarySelection => [
                Some(Clipboard::Clipboard),
                Some(Clipboard::PrimarySelection),
            ],
        };
        for &c in &clipboard {
            if let Some(c) = c {
                self.window.as_ref().unwrap().set_clipboard(c, text.clone());
            }
        }
    }

    pub fn paste_from_clipboard(&mut self, pane: &Arc<dyn Pane>, clipboard: ClipboardPasteSource) {
        let pane_id = pane.pane_id();
        log::trace!(
            "paste_from_clipboard in pane {} {:?}",
            pane.pane_id(),
            clipboard
        );
        let window = self.window.as_ref().unwrap().clone();
        let clipboard = match clipboard {
            ClipboardPasteSource::Clipboard => Clipboard::Clipboard,
            ClipboardPasteSource::PrimarySelection => Clipboard::PrimarySelection,
        };
        // fork: when image paste is enabled, prefer the image payload over
        // text; missing image, failed transcode or mode "none" all fall
        // through to the historical text-only behavior below
        let image_mode = self.config.clipboard_image_paste;
        let text_future = window.get_clipboard(clipboard);
        let image_future = if image_mode != ClipboardImagePasteMode::None {
            Some(window.get_clipboard_image(clipboard))
        } else {
            None
        };
        promise::spawn::spawn(async move {
            // fork: transcode on this spawn thread so the GUI thread never
            // blocks on image decode/encode; results go back to the window
            // event loop via Window::notify, like every other cross-thread
            // hop in the GUI
            if let Some(image_future) = image_future {
                if let Ok(Some(image)) = image_future.await {
                    match clipboard_image_to_png(image) {
                        Ok(png) => match image_mode {
                            ClipboardImagePasteMode::Inline => {
                                paste_inline_image(&window, pane_id, png);
                                return;
                            }
                            ClipboardImagePasteMode::Path => {
                                if paste_image_path(&window, pane_id, png) {
                                    return;
                                }
                            }
                            ClipboardImagePasteMode::None => {}
                        },
                        Err(err) => {
                            log::warn!("unable to transcode clipboard image: {err:#}");
                        }
                    }
                }
            }
            if let Ok(clip) = text_future.await {
                window.notify(TermWindowNotif::Apply(Box::new(move |myself| {
                    if let Some(pane) = lookup_pane(myself, pane_id) {
                        pane.send_paste(&clip).ok();
                    }
                })));
            }
        })
        .detach();
        self.maybe_scroll_to_bottom_for_input(&pane);
    }
}

// fork: resolve the paste target the same way for every paste flavor: the
// pane's overlay (when a modal owns it) wins, else the mux pane registry
fn lookup_pane(myself: &TermWindow, pane_id: PaneId) -> Option<Arc<dyn Pane>> {
    myself
        .pane_state(pane_id)
        .overlay
        .as_ref()
        .map(|overlay| overlay.pane.clone())
        .or_else(|| Mux::get().get_pane(pane_id))
}

// fork: image paste. Inserting the image straight into the terminal model
// is a controlled exception to the "pane output only advances on the parse
// thread" invariant: pane.perform_actions is the same main-thread funnel
// the GUI already uses for key_down/mouse_event and the ResetTerminal
// assignment, so this path takes the same lock and mutates the same model
// as user input would, without touching the pty input or relying on the
// shell to echo anything back.
fn paste_inline_image(window: &window::Window, pane_id: PaneId, png: Vec<u8>) {
    let action = termwiz::escape::Action::OperatingSystemCommand(Box::new(
        termwiz::escape::osc::OperatingSystemCommand::ITermProprietary(
            termwiz::escape::osc::ITermProprietary::File(Box::new(
                termwiz::escape::osc::ITermFileData {
                    name: Some("clipboard.png".to_string()),
                    size: Some(png.len()),
                    width: termwiz::escape::osc::ITermDimension::Automatic,
                    height: termwiz::escape::osc::ITermDimension::Automatic,
                    preserve_aspect_ratio: true,
                    inline: true,
                    do_not_move_cursor: false,
                    data: png,
                },
            )),
        ),
    ));
    window.notify(TermWindowNotif::Apply(Box::new(move |myself| {
        if let Some(pane) = lookup_pane(myself, pane_id) {
            pane.perform_actions(vec![action]);
        }
    })));
    window.invalidate();
}

// fork: write the PNG to a temp file and paste its path as text through
// the regular send_paste funnel; false lets the caller fall back to text
fn paste_image_path(window: &window::Window, pane_id: PaneId, png: Vec<u8>) -> bool {
    let path = unique_temp_png_path();
    match std::fs::write(&path, &png) {
        Ok(()) => {
            let text = path.display().to_string();
            window.notify(TermWindowNotif::Apply(Box::new(move |myself| {
                if let Some(pane) = lookup_pane(myself, pane_id) {
                    pane.send_paste(&text).ok();
                }
            })));
            true
        }
        Err(err) => {
            log::warn!(
                "unable to write clipboard image to {}: {err:#}",
                path.display()
            );
            false
        }
    }
}

// fork: unique temp file name from a timestamp + process-wide counter;
// deliberately dependency-free
fn unique_temp_png_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("wezterm-clipboard-{ts}-{seq}.png"))
}

// fork: longest edge (in pixels) a pasted clipboard image may keep; larger
// images are downsampled before being encoded as PNG
const CLIPBOARD_PASTE_MAX_DIMENSION: u32 = 2048;

// fork: normalize a clipboard image payload to PNG bytes for both paste
// modes. PNG passes through untouched; the DIB flavors are wrapped with a
// synthesized BITMAPFILEHEADER and decoded with the image crate. Only
// ever called on the spawn thread.
fn clipboard_image_to_png(image: ClipboardImage) -> anyhow::Result<Vec<u8>> {
    let mut rgba = match image.format {
        ClipboardImageFormat::Png => return Ok(image.data),
        ClipboardImageFormat::Dib | ClipboardImageFormat::DibV5 => decode_dib(&image.data)?,
    };
    if let Some((width, height)) =
        downscale_dims(rgba.width(), rgba.height(), CLIPBOARD_PASTE_MAX_DIMENSION)
    {
        rgba = image::DynamicImage::ImageRgba8(rgba)
            .resize_exact(width, height, image::imageops::FilterType::Triangle)
            .to_rgba8();
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .context("encoding clipboard image as PNG")?;
    Ok(png)
}

// fork: clipboard DIBs (CF_DIB/CF_DIBV5) start at the BITMAPINFOHEADER or
// BITMAPV5HEADER and lack the 14-byte BITMAPFILEHEADER the BMP decoder
// expects; synthesize one, computing the pixel data offset from the
// header size, the color table and any legacy bitmask DWORDs that follow
// a BITMAPINFOHEADER with BI_BITFIELDS/BI_ALPHABITFIELDS compression.
fn dib_to_bmp_bytes(dib: &[u8]) -> anyhow::Result<Vec<u8>> {
    const FILE_HEADER_LEN: usize = 14;
    const BITMAPINFOHEADER_LEN: usize = 40;
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;

    anyhow::ensure!(
        dib.len() >= BITMAPINFOHEADER_LEN,
        "clipboard DIB too short: {} bytes",
        dib.len()
    );
    let header_len = u32::from_le_bytes(dib[0..4].try_into().unwrap()) as usize;
    anyhow::ensure!(
        header_len >= BITMAPINFOHEADER_LEN,
        "unexpected DIB header size {header_len}"
    );
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(dib[16..20].try_into().unwrap());
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().unwrap());
    let palette_len = if colors_used != 0 {
        colors_used
    } else if bit_count <= 8 {
        1u32 << bit_count
    } else {
        0
    };
    let mask_len = if header_len == BITMAPINFOHEADER_LEN {
        match compression {
            BI_BITFIELDS => 3 * 4,
            BI_ALPHABITFIELDS => 4 * 4,
            _ => 0,
        }
    } else {
        0
    };

    let pixel_offset = FILE_HEADER_LEN + header_len + mask_len + palette_len as usize * 4;
    let mut bmp = Vec::with_capacity(FILE_HEADER_LEN + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((FILE_HEADER_LEN + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

// fork: decode DIB bytes into RGBA; see dib_to_bmp_bytes for the header
// synthesis
fn decode_dib(dib: &[u8]) -> anyhow::Result<image::RgbaImage> {
    let bmp = dib_to_bmp_bytes(dib)?;
    let decoded = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
        .context("decoding clipboard DIB image")?;
    let mut rgba = decoded.to_rgba8();
    treat_all_zero_alpha_as_opaque(&mut rgba);
    Ok(rgba)
}

// fork: 32bpp BI_RGB DIBs from careless writers carry an all-zero alpha
// plane; a fully transparent clipboard image is never what the user meant
// to paste, so promote an all-zero alpha plane to opaque and leave any
// image with at least one non-zero alpha byte untouched
fn treat_all_zero_alpha_as_opaque(image: &mut image::RgbaImage) -> bool {
    if image.pixels().any(|p| p[3] != 0) {
        return false;
    }
    for p in image.pixels_mut() {
        p[3] = 255;
    }
    true
}

// fork: fit within `max` on the longest edge while preserving the aspect
// ratio; None when no rescale is needed, and never below 1 pixel
fn downscale_dims(width: u32, height: u32, max: u32) -> Option<(u32, u32)> {
    let longest = width.max(height);
    if longest <= max || longest == 0 {
        return None;
    }
    let scale = max as f64 / longest as f64;
    let scaled = |n: u32| (((n as f64) * scale).round() as u32).max(1);
    Some((scaled(width), scaled(height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal 1x1 24bpp CF_DIB: BITMAPINFOHEADER followed by one padded
    // BGR pixel
    fn dib_1x1_24bpp() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&1i32.to_le_bytes()); // biWidth
        dib.extend_from_slice(&1i32.to_le_bytes()); // biHeight
        dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        dib.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
        dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
        dib.extend_from_slice(&4u32.to_le_bytes()); // biSizeImage
        dib.extend_from_slice(&[0u8; 16]); // pels per meter + clrUsed + clrImportant
        dib.extend_from_slice(&[1, 2, 3, 0]); // bottom-up BGRx pixel row
        dib
    }

    #[test]
    fn dib_header_synthesis() {
        let dib = dib_1x1_24bpp();
        let bmp = dib_to_bmp_bytes(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()),
            (14 + dib.len()) as u32
        );
        assert_eq!(u32::from_le_bytes(bmp[6..10].try_into().unwrap()), 0);
        // no palette, BI_RGB: pixels start right after the two headers
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(&bmp[14..], &dib[..]);
    }

    #[test]
    fn dib_header_offset_counts_palette() {
        // 8bpp with biClrUsed=0 implies the full 256-entry color table
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&8u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]);
        let bmp = dib_to_bmp_bytes(&dib).unwrap();
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            (14 + 40 + 256 * 4) as u32
        );
    }

    #[test]
    fn dib_header_offset_counts_bitfield_masks() {
        // legacy BITMAPINFOHEADER + BI_BITFIELDS carries 3 mask DWORDs
        // before the pixel data
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]);
        let bmp = dib_to_bmp_bytes(&dib).unwrap();
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            14 + 40 + 12
        );
    }

    #[test]
    fn dib_rejects_short_payloads() {
        assert!(dib_to_bmp_bytes(&[0u8; 8]).is_err());
        // a header claiming less than BITMAPINFOHEADER is malformed
        let mut dib = dib_1x1_24bpp();
        dib[0..4].copy_from_slice(&12u32.to_le_bytes());
        assert!(dib_to_bmp_bytes(&dib).is_err());
    }

    #[test]
    fn dib_roundtrip_through_bmp_decoder() {
        // encode an image as BMP, strip the file header to emulate CF_DIB,
        // then decode it back through the same funnel used for pasting
        let src = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([(x * 40) as u8, (y * 80) as u8, 128, 255])
        });
        let mut bmp = Vec::new();
        image::DynamicImage::ImageRgba8(src)
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .unwrap();
        let decoded = decode_dib(&bmp[14..]).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
    }

    #[test]
    fn all_zero_alpha_is_promoted_to_opaque() {
        let mut img = image::RgbaImage::from_raw(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 0]).unwrap();
        assert!(treat_all_zero_alpha_as_opaque(&mut img));
        assert!(img.pixels().all(|p| p[3] == 255));

        let mut img = image::RgbaImage::from_raw(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 8]).unwrap();
        assert!(!treat_all_zero_alpha_as_opaque(&mut img));
        assert_eq!(img.pixels().map(|p| p[3]).collect::<Vec<_>>(), vec![0, 8]);
    }

    #[test]
    fn downscale_fits_longest_edge() {
        assert_eq!(downscale_dims(100, 50, 2048), None);
        assert_eq!(downscale_dims(2048, 1024, 2048), None);
        assert_eq!(downscale_dims(4096, 2048, 2048), Some((2048, 1024)));
        assert_eq!(downscale_dims(1000, 4000, 2048), Some((512, 2048)));
        // extreme aspect ratios never collapse to zero
        assert_eq!(downscale_dims(100_000, 1, 2048), Some((2048, 1)));
        assert_eq!(downscale_dims(0, 0, 2048), None);
    }

    #[test]
    fn png_payload_passes_through_untouched() {
        let data = vec![1, 2, 3, 4];
        let png = clipboard_image_to_png(ClipboardImage {
            data: data.clone(),
            format: ClipboardImageFormat::Png,
        })
        .unwrap();
        assert_eq!(png, data);
    }
}
