#![cfg(windows)]

use crate::locator::{FontDataSource, FontLocator, FontOrigin};
use crate::parser::{best_match_by_name, parse_and_collect_font_info, ParsedFont};
use config::{
    FontAttributes, FontStretch as WTFontStretch, FontStyle as WTFontStyle,
    FontWeight as WTFontWeight,
};
use dwrote::{FontStretch, FontStyle, FontWeight};
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use winapi::shared::windef::{HDC, HFONT};
use winapi::um::dwrite::*;
use winapi::um::winbase::MulDiv;
use winapi::um::wingdi::{
    CreateCompatibleDC, CreateFontIndirectW, DeleteDC, DeleteObject, GetDeviceCaps, GetFontData,
    GetTextFaceW, SelectObject, FIXED_PITCH, GDI_ERROR, LF_FACESIZE, LOGFONTW, LOGPIXELSY,
    OUT_TT_ONLY_PRECIS,
};

/// A FontLocator implemented using the system font loading
/// functions provided by the font-loader crate.
pub struct GdiFontLocator {}

// fork: read the face name of the font currently selected into `hdc`.
// The GDI font mapper silently substitutes another font when the requested
// family is missing, so callers use this to tell a real hit from a substitute.
unsafe fn selected_face_name(hdc: HDC) -> String {
    let mut buf = [0u16; LF_FACESIZE];
    let copied = GetTextFaceW(hdc, LF_FACESIZE as i32, buf.as_mut_ptr());
    if copied <= 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

// fork: the GDI fallback only counts as a hit when the face the mapper
// selected matches the requested name. GDI compares family names case
// insensitively, so do the same. A request spelled as a full face name
// ("Foo Bold") is still accepted when the mapper selected family "Foo";
// best_match_by_name then filters by full/postscript name.
fn gdi_face_matches_request(requested: &str, actual: &str) -> bool {
    let requested = requested.trim().to_lowercase();
    let actual = actual.trim().to_lowercase();
    if requested.is_empty() || actual.is_empty() {
        return false;
    }
    requested == actual
        || requested
            .strip_prefix(actual.as_str())
            .map_or(false, |rest| rest.starts_with(' '))
}

fn extract_raw_font_data(
    font: HFONT,
    name: &str,
    require_face_match: bool,
) -> anyhow::Result<FontDataSource> {
    unsafe {
        let hdc = CreateCompatibleDC(std::ptr::null_mut());
        SelectObject(hdc, font as *mut _);

        // fork: bail out when the mapper substituted another family;
        // otherwise we would copy the substitute's whole file into RAM and
        // report it as a hit, hiding the "font not found" condition.
        if require_face_match {
            let actual = selected_face_name(hdc);
            if !gdi_face_matches_request(name, &actual) {
                DeleteDC(hdc);
                anyhow::bail!(
                    "GDI mapped family {:?} to {:?}; not using the substitute font",
                    name,
                    actual
                );
            }
        }

        // GetFontData can retrieve different parts of the font data.
        // We want to fetch the entire font file, but things are made
        // more complicated because the file may be a TTC file.
        // In that case, the full file data isn't full parsable
        // as a TTF so we need to ask specifically for the TTC file,
        // and then try to reverse engineer which element of the TTC
        // is the one we were looking for.

        // See if we can retrieve the ttc data as a first try
        let ttc_table = 0x66637474; // 'ttcf'

        let ttc_size = GetFontData(hdc, ttc_table, 0, std::ptr::null_mut(), 0);

        let data = if ttc_size > 0 && ttc_size != GDI_ERROR {
            let mut data = vec![0u8; ttc_size as usize];
            GetFontData(hdc, ttc_table, 0, data.as_mut_ptr() as *mut _, ttc_size);

            Ok(data)
        } else {
            // Otherwise: presumably a regular ttf

            let size = GetFontData(hdc, 0, 0, std::ptr::null_mut(), 0);
            match size {
                _ if size > 0 && size != GDI_ERROR => {
                    let mut data = vec![0u8; size as usize];
                    GetFontData(hdc, 0, 0, data.as_mut_ptr() as *mut _, size);
                    Ok(data)
                }
                _ => Err(anyhow::anyhow!("Failed to get font data")),
            }
        };
        DeleteDC(hdc);
        let data = data?;

        Ok(FontDataSource::Memory {
            data: Arc::new(data.into_boxed_slice()),
            name: name.to_string(),
        })
    }
}

fn extract_font_data(
    font: HFONT,
    attr: &FontAttributes,
    pixel_size: u16,
) -> anyhow::Result<ParsedFont> {
    let source = extract_raw_font_data(font, &attr.family, true)?;

    let mut font_info = vec![];
    parse_and_collect_font_info(&source, &mut font_info, FontOrigin::Gdi)?;
    let matches = best_match_by_name(attr, pixel_size, font_info);

    match matches {
        Some(m) => Ok(m),
        None => anyhow::bail!("No font matching {:?} in {:?}", attr, source),
    }
}

/// Convert a rust string to a windows wide string
fn wide_string(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// fork: encode codepoints as the UTF-16 code units DirectWrite expects.
// Supplementary plane characters (U+10000 and above) need surrogate pairs
// rather than being truncated to a u16.
fn codepoints_to_utf16(codepoints: &[char]) -> Vec<u16> {
    let mut text = Vec::with_capacity(codepoints.len());
    let mut buf = [0u16; 2];
    for &c in codepoints {
        text.extend_from_slice(c.encode_utf16(&mut buf));
    }
    text
}

fn load_font(font_attr: &FontAttributes, pixel_size: u16) -> anyhow::Result<ParsedFont> {
    let mut log_font = LOGFONTW {
        lfHeight: 0,
        lfWidth: 0,
        lfEscapement: 0,
        lfOrientation: 0,
        lfWeight: font_attr.weight.to_opentype_weight() as _,
        lfItalic: if font_attr.style != WTFontStyle::Normal {
            1
        } else {
            0
        },
        lfUnderline: 0,
        lfStrikeOut: 0,
        lfCharSet: 0,
        lfOutPrecision: OUT_TT_ONLY_PRECIS as u8,
        lfClipPrecision: 0,
        lfQuality: 0,
        lfPitchAndFamily: FIXED_PITCH as u8,
        lfFaceName: [0u16; 32],
    };

    let name = wide_string(&font_attr.family);
    if name.len() > LF_FACESIZE {
        anyhow::bail!(
            "family name {:?} is too large for LOGFONTW",
            font_attr.family
        );
    }
    for (i, &c) in name.iter().enumerate() {
        log_font.lfFaceName[i] = c;
    }

    unsafe {
        let font = CreateFontIndirectW(&log_font);
        let result = extract_font_data(font, font_attr, pixel_size);
        DeleteObject(font as *mut _);
        result
    }
}

pub fn parse_log_font(log_font: &LOGFONTW, hdc: HDC) -> anyhow::Result<(ParsedFont, f64)> {
    let name = String::from_utf16(&log_font.lfFaceName)?;
    unsafe {
        let font = CreateFontIndirectW(log_font);
        let source = extract_raw_font_data(font, &name, false);
        DeleteObject(font as *mut _);
        let source = source?;

        let point_size = MulDiv(-log_font.lfHeight, 72, GetDeviceCaps(hdc, LOGPIXELSY)) as f64;
        let pixel_size = log_font.lfHeight.abs() as u16;

        let mut attr = FontAttributes::new(&name);
        attr.weight = config::FontWeight::from_opentype_weight(log_font.lfWeight as u16);
        if log_font.lfItalic == 1 {
            attr.style = WTFontStyle::Italic;
        }

        let mut font_info = vec![];
        parse_and_collect_font_info(&source, &mut font_info, FontOrigin::Gdi)?;
        let matches = ParsedFont::best_match(&attr, pixel_size, font_info);

        match matches {
            Some(m) => Ok((m, point_size)),
            None => anyhow::bail!("No font matching {:?} in {:?}", attr, source),
        }
    }
}

// fork: collect the on-disk file paths backing every face of a family,
// deduplicated by path: DirectWrite's simulated bold/italic faces share the
// file of the real face, and one TTC is referenced by several faces.
fn family_font_paths(family: &dwrote::FontFamily) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = vec![];
    for idx in 0..family.get_font_count() {
        let font = match family.font(idx) {
            Ok(font) => font,
            Err(hr) => {
                log::warn!(
                    "IDWriteFontFamily::GetFont({idx}) failed: HRESULT {:#x}",
                    hr as u32
                );
                continue;
            }
        };
        let face = font.create_font_face();
        let files = match face.files() {
            Ok(files) => files,
            Err(hr) => {
                log::warn!(
                    "IDWriteFontFace::GetFiles for face {idx} failed: HRESULT {:#x}",
                    hr as u32
                );
                continue;
            }
        };
        for file in files {
            // Fonts served by a non-local loader (e.g. in-memory
            // registrations) have no path; skip them.
            if let Ok(path) = file.font_file_path() {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

// fork: enumerate every face of the family in the DirectWrite system
// collection, parse the backing files and let wezterm's own CSS font
// matching pick the nearest weight/style and flag synthetic bold/italic,
// matching the fontconfig/CoreText locators. The files stay mmap'd
// (OnDisk) so styles and windows share the page cache instead of each
// copying the font into RAM.
//
// The previous `get_font_from_descriptor` path only accepted exact weight
// matches, so bold/dim styles almost always fell through to the GDI
// `GetFontData` whole-file copy.
fn handle_from_family(
    attr: &FontAttributes,
    collection: &dwrote::FontCollection,
    pixel_size: u16,
) -> Option<ParsedFont> {
    let family = match collection.font_family_by_name(&attr.family) {
        Ok(Some(family)) => family,
        Ok(None) => {
            log::debug!("DirectWrite has no family named {:?}", attr.family);
            return None;
        }
        Err(hr) => {
            log::warn!(
                "IDWriteFontCollection::FindFamilyName({:?}) failed: HRESULT {:#x}",
                attr.family,
                hr as u32
            );
            return None;
        }
    };

    let paths = family_font_paths(&family);
    if paths.is_empty() {
        log::debug!(
            "DirectWrite family {:?} has no on-disk font files",
            attr.family
        );
        return None;
    }

    let mut font_info = vec![];
    for path in paths {
        log::debug!("{} -> {}", attr.family, path.display());
        let source = FontDataSource::OnDisk(path);
        if let Err(err) =
            parse_and_collect_font_info(&source, &mut font_info, FontOrigin::DirectWrite)
        {
            log::warn!("While parsing: {:?}: {:#}", source, err);
        }
    }

    best_match_by_name(attr, pixel_size, font_info)
}

impl FontLocator for GdiFontLocator {
    fn load_fonts(
        &self,
        fonts_selection: &[FontAttributes],
        loaded: &mut HashSet<FontAttributes>,
        pixel_size: u16,
    ) -> anyhow::Result<Vec<ParsedFont>> {
        let mut fonts = Vec::new();
        let collection = dwrote::FontCollection::system();

        for font_attr in fonts_selection {
            fn try_handle(
                font_attr: &FontAttributes,
                parsed: ParsedFont,
                fonts: &mut Vec<ParsedFont>,
                loaded: &mut HashSet<FontAttributes>,
            ) -> bool {
                if parsed.matches_name(font_attr) {
                    fonts.push(parsed);
                    loaded.insert(font_attr.clone());
                    true
                } else {
                    log::debug!("parsed {:?} doesn't match {:?}", parsed, font_attr);
                    false
                }
            }

            match handle_from_family(font_attr, &collection, pixel_size) {
                Some(handle) => {
                    log::debug!("Got {:?} from dwrote", handle);
                    if try_handle(font_attr, handle, &mut fonts, loaded) {
                        continue;
                    }
                }
                None => {
                    log::debug!("dwrote couldn't resolve {:?}", font_attr);
                }
            }

            // fork: GDI is the last resort; load_font errors out instead of
            // copying when the mapper substituted another family.
            match load_font(font_attr, pixel_size) {
                Ok(handle) => {
                    log::debug!("Got {:?} from gdi", handle);
                    try_handle(font_attr, handle, &mut fonts, loaded);
                }
                Err(err) => {
                    log::debug!("gdi couldn't resolve {:?} to a path: {:#}", font_attr, err);
                }
            }
        }

        Ok(fonts)
    }

    fn locate_fallback_for_codepoints(
        &self,
        codepoints: &[char],
    ) -> anyhow::Result<Vec<ParsedFont>> {
        // fork: DirectWrite text positions and lengths are in UTF-16 code
        // units, so start/len below advance by text.len(), not by codepoints.
        let text = codepoints_to_utf16(codepoints);
        let text_len = text.len();

        let collection = dwrote::FontCollection::system();
        struct Source {
            locale: String,
            len: u32,
        }
        impl dwrote::TextAnalysisSourceMethods for Source {
            fn get_locale_name<'a>(&'a self, _: u32) -> (Cow<'a, str>, u32) {
                (Cow::Borrowed(&self.locale), self.len)
            }
            fn get_paragraph_reading_direction(&self) -> u32 {
                DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
            }
        }

        let source = dwrote::TextAnalysisSource::from_text(
            Box::new(Source {
                locale: "".to_string(),
                len: text_len as u32,
            }),
            Cow::Borrowed(&text),
        );

        let mut handles = vec![];
        let mut resolved = HashSet::new();

        if let Some(fallback) = dwrote::FontFallback::get_system_fallback() {
            let mut start = 0usize;
            let mut len = text_len;
            loop {
                let result = fallback.map_characters(
                    &source,
                    start as u32,
                    len as u32,
                    &collection,
                    None,
                    FontWeight::Regular,
                    FontStyle::Normal,
                    FontStretch::Normal,
                );

                if let Some(font) = result.mapped_font {
                    log::trace!(
                        "DirectWrite Suggested fallback: {} {}",
                        font.family_name(),
                        font.face_name()
                    );

                    let attr = FontAttributes {
                        weight: WTFontWeight::from_opentype_weight(font.weight().to_u32() as _),
                        stretch: WTFontStretch::from_opentype_stretch(font.stretch().to_u32() as _),
                        style: WTFontStyle::Normal,
                        family: font.family_name(),
                        is_fallback: true,
                        is_synthetic: true,
                        harfbuzz_features: None,
                        freetype_load_target: None,
                        freetype_render_target: None,
                        freetype_load_flags: None,
                        scale: None,
                        assume_emoji_presentation: None,
                    };

                    if !resolved.contains(&attr) {
                        resolved.insert(attr.clone());

                        if let Some(handle) = handle_from_family(
                            &attr,
                            &collection,
                            16, /* pixel_size: irrelevant really as we kinda want a scalable font for fallback */
                        ) {
                            handles.push(handle);
                        }
                    }
                }
                if result.mapped_length > 0 {
                    start += result.mapped_length
                } else {
                    break;
                }
                if start >= text_len {
                    break;
                }
                len = text_len - start;
            }
        } else {
            log::error!("Unable to get system fallback from dwrote");
        }

        Ok(handles)
    }

    fn enumerate_all_fonts(&self) -> anyhow::Result<Vec<ParsedFont>> {
        let collection = dwrote::FontCollection::system();
        let mut fonts = vec![];
        let mut files = HashSet::new();
        for family in collection.families_iter() {
            let count = family.get_font_count();
            for idx in 0..count {
                let font = family.get_font(idx);
                let face = font.create_font_face();
                for file in face.get_files() {
                    if let Some(path) = file.get_font_file_path() {
                        if files.contains(&path) {
                            continue;
                        }
                        files.insert(path.clone());

                        let source = FontDataSource::OnDisk(path);
                        if let Err(err) = parse_and_collect_font_info(
                            &source,
                            &mut fonts,
                            FontOrigin::DirectWrite,
                        ) {
                            log::warn!("While parsing: {:?}: {:#}", source, err);
                        }
                    }
                }
            }
        }
        fonts.sort();

        Ok(fonts)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn utf16_encoding_keeps_surrogate_pairs() {
        // BMP chars take one unit; supplementary plane emoji need a surrogate pair
        assert_eq!(codepoints_to_utf16(&['中', '文']), vec![0x4e2d, 0x6587]);
        assert_eq!(codepoints_to_utf16(&['😀']), vec![0xd83d, 0xde00]);
        assert_eq!(
            codepoints_to_utf16(&['a', '😀', '中']),
            vec![0x61, 0xd83d, 0xde00, 0x4e2d]
        );
    }

    #[test]
    fn gdi_face_match_is_case_insensitive_and_rejects_substitutes() {
        assert!(gdi_face_matches_request(
            "Microsoft YaHei",
            "Microsoft YaHei"
        ));
        assert!(gdi_face_matches_request(
            "microsoft yahei",
            "Microsoft YaHei"
        ));
        assert!(gdi_face_matches_request("微软雅黑", "微软雅黑"));
        // request spelled as a full face name, mapper selected the family
        assert!(gdi_face_matches_request(
            "Microsoft YaHei Bold",
            "Microsoft YaHei"
        ));
        assert!(!gdi_face_matches_request(
            "Microsoft YaHeiBold",
            "Microsoft YaHei"
        ));
        // mapper substituted a different font
        assert!(!gdi_face_matches_request("No Such Font", "Arial"));
        assert!(!gdi_face_matches_request("Segoe UI", "Segoe UI Emoji"));
        assert!(!gdi_face_matches_request("", ""));
        assert!(!gdi_face_matches_request("Arial", ""));
    }
}
