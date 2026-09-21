//! fork 新增：壁纸管理浮层（批 13）。
//!
//! 列出壁纸目录内的图片（文件名/尺寸/大小/当前标记）；↑↓/j/k/n/p
//! 移动即**实时预览**（只替换窗口背景层栈，不重载 Lua，与批 5 设置页
//! 预览同一口径）；Enter 应用并持久化到 `gui-settings.json` 的
//! `wallpaper` 键（重启后仍生效，Lua 侧 `utils/backdrops.lua` 启动时
//! 读回）；`r` 随机预览、`a` 添加（路径输入，Tab 补全/`~` 展开/粘贴，
//! 校验可解码后复制进壁纸目录）、`d` 删除（`y` 二次确认，只删目录内
//! 条目）；Esc/点外/被顶掉经 `Modal::on_dismissed` 还原未确认预览。

use crate::termwindow::background::{load_background_layer, LoadedBackgroundLayer};
use crate::termwindow::box_model::*;
use crate::termwindow::modal::{Modal, MODAL_CHROME_ROW};
use crate::termwindow::{DimensionContext, TermWindow, TermWindowNotif, UIItemType};
use config::i18n::tr;
use config::keyassignment::KeyAssignment;
use config::{BackgroundLayer, BackgroundSource, Dimension, ImageFileSource, ImageFileSourceWrap};
use std::cell::{Ref, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wezterm_dynamic::Value;
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::{Clipboard, WindowOps};

/// 壁纸图片的扩展名白名单（与 dotfiles `utils/backdrops.lua` 的 glob 一致）
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "ico", "tiff", "pnm", "dds", "tga",
];

/// 目录里的一张壁纸
#[derive(Clone)]
struct WallpaperEntry {
    path: PathBuf,
    /// basename（持久化与「当前」标记都按它）
    name: String,
    size_bytes: u64,
    dimensions: Option<(u32, u32)>,
}

/// 浮层模式：浏览 / 添加路径输入 / 删除二次确认
#[derive(Clone)]
enum Mode {
    Browse,
    Adding(String),
    /// 待确认删除的 entries 索引
    ConfirmDelete(usize),
}

/// 扫描壁纸目录（纯取数，不碰 TermWindow；单测直接驱动它）。
/// 只收白名单扩展名的常规文件，按文件名排序。
fn scan_wallpapers(dir: &Path) -> Vec<WallpaperEntry> {
    let mut entries = vec![];
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        // 只读图片头拿尺寸，不解码整图（大图不进取数路径）
        let dimensions = image::image_dimensions(&path).ok();
        entries.push(WallpaperEntry {
            path,
            name,
            size_bytes,
            dimensions,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// 校验待添加的路径：必须存在、是常规文件、可解码（读头判定）。
/// 返回规范化后的绝对路径或错误文案（纯函数，行内错误不关浮层）。
fn validate_image_path(input: &str) -> anyhow::Result<PathBuf> {
    let expanded = expand_tilde(input.trim());
    let path = PathBuf::from(&expanded);
    if !path.exists() {
        anyhow::bail!(tr("file does not exist").into_owned());
    }
    if !path.is_file() {
        anyhow::bail!(tr("not a regular file").into_owned());
    }
    image::image_dimensions(&path)
        .map_err(|_| anyhow::anyhow!(tr("not a decodable image").into_owned()))?;
    let abs = std::fs::canonicalize(&path).unwrap_or(path);
    Ok(abs)
}

/// `~/...` 展开为家目录
fn expand_tilde(input: &str) -> String {
    if input == "~" || input.starts_with("~/") {
        if let Some(home) = dirs_next_home() {
            return format!("{}{}", home, &input[1..]);
        }
    }
    input.to_string()
}

fn dirs_next_home() -> Option<String> {
    std::env::var_os("HOME").map(|h| h.to_string_lossy().to_string())
}

/// Tab 补全：把输入补到匹配项的最长公共前缀（目录带 `/` 结尾）。
/// 返回（新输入, 匹配数）。纯函数。
fn complete_path(input: &str) -> (String, usize) {
    let expanded = expand_tilde(input);
    let (dir_part, frag) = match expanded.rfind('/') {
        Some(idx) => (&expanded[..idx + 1], &expanded[idx + 1..]),
        None => ("", expanded.as_str()),
    };
    let dir = if dir_part.is_empty() { "." } else { dir_part };
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return (input.to_string(), 0),
    };
    let mut matches: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.starts_with(frag) {
                return None;
            }
            let suffix = if e.path().is_dir() { "/" } else { "" };
            Some(format!("{name}{suffix}"))
        })
        .collect();
    matches.sort();
    if matches.is_empty() {
        return (input.to_string(), 0);
    }
    let mut common = matches[0].clone();
    for m in &matches[1..] {
        while !m.starts_with(&common) {
            common.pop();
        }
    }
    (format!("{dir_part}{common}"), matches.len())
}

/// 把校验过的图片复制进壁纸目录；同名冲突追加短哈希后缀。
/// 返回目标文件名（纯文件操作，可测）。
fn copy_into_wallpaper_dir(src: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid file name"))?
        .to_string();
    std::fs::create_dir_all(dir)?;
    let mut candidate = dir.join(&name);
    if candidate.exists() {
        let same = std::fs::read(&candidate)
            .map(|existing| {
                std::fs::read(src)
                    .map(|new| existing == new)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !same {
            let hash = content_hash(src);
            let stem = src
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("wallpaper");
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("img");
            candidate = dir.join(format!("{stem}-{hash}.{ext}"));
        }
    }
    std::fs::copy(src, &candidate)?;
    Ok(candidate)
}

fn content_hash(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(data) = std::fs::read(path) {
        data.hash(&mut h);
    } else {
        path.hash(&mut h);
    }
    format!("{:08x}", h.finish() as u32)
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / 1024. / 1024.)
    } else {
        format!("{}K", (bytes + 1023) / 1024)
    }
}

/// 全窗重绘（背景层栈替换后走这里；预览不碰任何失效代数——paint 每帧
/// 直接读 `window_background`）
fn invalidate_window(term_window: &TermWindow) {
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}

/// 预览层栈（纯函数）：克隆进入浮层时的层定义，把第一层换成新图，
/// 其余层（遮罩等）原样保留——观感与 backdrops.lua 的 `_create_opts`
/// 一致（壁纸层 + 遮罩层）。进入时无背景层（纯色/无配置）则只放
/// 单层壁纸。
fn preview_defs(layers: &[LoadedBackgroundLayer], path: &Path) -> Vec<BackgroundLayer> {
    let mut defs: Vec<_> = layers.iter().map(|l| l.def.clone()).collect();
    let source = BackgroundSource::File(ImageFileSourceWrap::from(ImageFileSource {
        path: path.display().to_string(),
        speed: 1.0,
    }));
    if defs.is_empty() {
        defs.push(BackgroundLayer {
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
            width: Default::default(),
            height: Default::default(),
        });
    } else {
        defs[0].source = source;
    }
    defs
}

/// 当前生效壁纸的 basename（窗口背景第一层是 File 时）
fn current_wallpaper_name(layers: &[LoadedBackgroundLayer]) -> Option<String> {
    let first = layers.first()?;
    match &first.def.source {
        BackgroundSource::File(f) => Path::new(&f.path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

pub struct WallpaperOverlay {
    dir: PathBuf,
    entries: RefCell<Vec<WallpaperEntry>>,
    selected: RefCell<usize>,
    top_row: RefCell<usize>,
    max_rows_on_screen: RefCell<usize>,
    mode: RefCell<Mode>,
    /// 行内错误（非法路径/非图片/删除失败等），不关浮层
    error: RefCell<Option<String>>,
    /// 进入浮层时的背景层栈快照（Esc/点外还原用它）
    original: Vec<LoadedBackgroundLayer>,
    /// 正在预览的壁纸名；None = 无未确认预览
    previewing: RefCell<Option<String>>,
    /// 进入浮层时的当前壁纸（Enter 之外的还原目标）
    current_on_open: RefCell<Option<String>>,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

impl WallpaperOverlay {
    pub fn new(term_window: &TermWindow) -> Self {
        let dir = wallpaper_dir();
        let entries = scan_wallpapers(&dir);
        let current = current_wallpaper_name(&term_window.window_background);
        let selected = current
            .as_ref()
            .and_then(|name| entries.iter().position(|e| &e.name == name))
            .unwrap_or(0);
        Self {
            dir,
            entries: RefCell::new(entries),
            selected: RefCell::new(selected),
            top_row: RefCell::new(0),
            max_rows_on_screen: RefCell::new(0),
            mode: RefCell::new(Mode::Browse),
            error: RefCell::new(None),
            original: term_window.window_background.clone(),
            previewing: RefCell::new(None),
            current_on_open: RefCell::new(current),
            element: RefCell::new(None),
        }
    }

    fn move_selection(&self, delta: isize) -> bool {
        let entries = self.entries.borrow();
        let len = entries.len();
        if len == 0 {
            return false;
        }
        let mut selected = self.selected.borrow_mut();
        let next = (*selected as isize + delta).rem_euclid(len as isize) as usize;
        if next == *selected {
            return false;
        }
        *selected = next;
        drop(selected);
        drop(entries);
        let max_rows = *self.max_rows_on_screen.borrow();
        let mut top_row = self.top_row.borrow_mut();
        let sel = *self.selected.borrow();
        if sel < *top_row {
            *top_row = sel;
        } else if max_rows > 0 && sel >= *top_row + max_rows {
            *top_row = sel.saturating_sub(max_rows - 1);
        }
        true
    }

    /// WZ-11：滚轮只滚视口，不改键盘选中
    fn scroll_rows(&self, delta: isize) {
        let len = self.entries.borrow().len();
        let max_rows = (*self.max_rows_on_screen.borrow()).max(1);
        let max_top = len.saturating_sub(max_rows);
        let mut top_row = self.top_row.borrow_mut();
        *top_row = (*top_row as isize + delta).clamp(0, max_top as isize) as usize;
    }

    /// 实时预览选中行（只换背景层，不重载 Lua）
    fn preview_selected(&self, term_window: &mut TermWindow) {
        let entries = self.entries.borrow();
        let selected = *self.selected.borrow();
        let Some(entry) = entries.get(selected) else {
            return;
        };
        if self.previewing.borrow().as_deref() == Some(entry.name.as_str()) {
            return;
        }
        let defs = preview_defs(&self.original, &entry.path);
        let mut layers = vec![];
        for def in &defs {
            match load_background_layer(def, &term_window.dimensions, &term_window.render_metrics) {
                Ok(layer) => layers.push(layer),
                Err(err) => {
                    self.error
                        .replace(Some(format!("{}: {err:#}", tr("preview failed"))));
                    return;
                }
            }
        }
        self.previewing.replace(Some(entry.name.clone()));
        term_window.window_background = layers;
        invalidate_window(term_window);
    }

    /// Enter：应用并持久化（写 sidecar + reload；reload 后 Lua 侧
    /// backdrops 从 sidecar 读回同一壁纸，视觉连续）。
    fn apply_selected(&self, term_window: &mut TermWindow) {
        let entries = self.entries.borrow();
        let selected = *self.selected.borrow();
        let Some(entry) = entries.get(selected) else {
            return;
        };
        let name = entry.name.clone();
        drop(entries);
        if let Err(err) = config::gui_settings::store_key("wallpaper", &Value::String(name.clone()))
        {
            self.error
                .replace(Some(format!("{}: {err:#}", tr("failed to write settings"))));
            return;
        }
        // 预览态转正：清除标记使 on_dismissed 不再还原
        self.previewing.replace(None);
        self.current_on_open.replace(Some(name));
        config::reload();
        term_window.cancel_modal();
    }

    fn rescan(&self) {
        let entries = scan_wallpapers(&self.dir);
        let keep = {
            let old = self.entries.borrow();
            let selected = *self.selected.borrow();
            old.get(selected).map(|e| e.name.clone())
        };
        let pos = keep
            .as_ref()
            .and_then(|name| entries.iter().position(|e| &e.name == name))
            .unwrap_or(0);
        self.entries.replace(entries);
        self.selected.replace(pos);
        self.top_row.replace(0);
    }

    /// 添加路径输入的 Enter：校验 → 复制进目录 → 重扫并选中它。
    fn finish_add(&self, input: &str, term_window: &mut TermWindow) {
        match validate_image_path(input).and_then(|src| copy_into_wallpaper_dir(&src, &self.dir)) {
            Ok(dest) => {
                let name = dest
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                self.rescan();
                if let Some(idx) = self.entries.borrow().iter().position(|e| e.name == name) {
                    self.selected.replace(idx);
                }
                self.error.replace(None);
                *self.mode.borrow_mut() = Mode::Browse;
                // 添加后立即预览新条目
                self.preview_selected(term_window);
            }
            Err(err) => {
                self.error.replace(Some(format!("{err:#}")));
                // 留在 Adding 模式让用户改路径
            }
        }
    }

    fn confirm_delete(&self, idx: usize, term_window: &mut TermWindow) {
        let entry = self.entries.borrow().get(idx).cloned();
        *self.mode.borrow_mut() = Mode::Browse;
        let Some(entry) = entry else {
            return;
        };
        match std::fs::remove_file(&entry.path) {
            Ok(()) => {
                let was_current = self.current_on_open.borrow().as_deref()
                    == Some(entry.name.as_str())
                    || self.previewing.borrow().as_deref() == Some(entry.name.as_str());
                self.rescan();
                if was_current {
                    // 删的是正在用的：sidecar 清键，有剩余则预览第一张，
                    // 空目录则还原进入时的层栈
                    if let Err(err) = config::gui_settings::delete_key("wallpaper") {
                        log::warn!("wallpaper: cannot clear sidecar key: {err:#}");
                    }
                    if self.entries.borrow().is_empty() {
                        term_window.window_background = self.original.clone();
                        self.previewing.replace(None);
                    } else {
                        self.selected.replace(0);
                        self.previewing.replace(None);
                        self.preview_selected(term_window);
                    }
                }
                self.error.replace(None);
            }
            Err(err) => {
                self.error
                    .replace(Some(format!("{}: {err:#}", tr("delete failed"))));
            }
        }
    }

    fn row_label(entry: &WallpaperEntry, is_current: bool) -> String {
        let marker = if is_current { "✓ " } else { "  " };
        let dims = entry
            .dimensions
            .map(|(w, h)| format!("{w}×{h}"))
            .unwrap_or_else(|| "?".to_string());
        format!(
            "{marker}{}  {}  {}",
            entry.name,
            dims,
            format_size(entry.size_bytes)
        )
    }

    fn compute(&self, term_window: &mut TermWindow) -> anyhow::Result<Vec<ComputedElement>> {
        let font = term_window
            .fonts
            .command_palette_font()
            .expect("to resolve command palette font");
        let metrics = crate::utilsprites::RenderMetrics::with_font_metrics(&font.metrics())
            .scale_line_height(term_window.config.command_palette_line_height);

        let bg: InheritableColor = term_window
            .config
            .command_palette_bg_color
            .to_linear()
            .into();
        let fg: InheritableColor = term_window
            .config
            .command_palette_fg_color
            .to_linear()
            .into();

        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height_lossy()
        } else {
            0.
        };
        let (padding_left, padding_top) = term_window.padding_left_top();
        let border = term_window.get_os_border();
        let top_pixel_y = top_bar_height + padding_top + border.top.get() as f32;

        let mut max_rows_on_screen = ((term_window.dimensions.pixel_height * 8 / 10)
            / metrics.cell_size.height as usize)
            .saturating_sub(6);
        if let Some(size) = term_window.config.command_palette_rows {
            max_rows_on_screen = max_rows_on_screen.min(size);
        }
        *self.max_rows_on_screen.borrow_mut() = max_rows_on_screen;

        let mut rows =
            vec![
                Element::new(&font, ElementContent::Text(tr("Wallpapers").into_owned()))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.1),
                        bottom: Dimension::Cells(0.1),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
            ];

        let mode = self.mode.borrow().clone();
        let selected = *self.selected.borrow();
        let top_row = *self.top_row.borrow();
        let entries = self.entries.borrow();
        let current = self.current_on_open.borrow().clone();

        match &mode {
            Mode::Browse => {
                if entries.is_empty() {
                    // 空态（批 13 spec）：引导按 a 添加
                    rows.push(
                        Element::new(
                            &font,
                            ElementContent::Text(
                                tr("(empty — press 'a' to add a wallpaper)").into_owned(),
                            ),
                        )
                        .colors(ElementColors {
                            border: BorderColor::default(),
                            bg: LinearRgba::TRANSPARENT.into(),
                            text: fg.clone(),
                        })
                        .padding(BoxDimension {
                            left: Dimension::Cells(0.5),
                            right: Dimension::Cells(0.5),
                            top: Dimension::Cells(0.),
                            bottom: Dimension::Cells(0.),
                        })
                        .min_width(Some(Dimension::Percent(1.)))
                        .display(DisplayType::Block)
                        .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
                    );
                }
                for (display_idx, entry) in entries
                    .iter()
                    .enumerate()
                    .skip(top_row)
                    .take(max_rows_on_screen)
                {
                    let is_current = current.as_deref() == Some(entry.name.as_str());
                    let label = Self::row_label(entry, is_current);
                    let (row_bg, row_fg) = if display_idx == selected {
                        (fg.clone(), bg.clone())
                    } else {
                        (LinearRgba::TRANSPARENT.into(), fg.clone())
                    };
                    rows.push(
                        Element::new(&font, ElementContent::Text(label))
                            .colors(ElementColors {
                                border: BorderColor::default(),
                                bg: row_bg,
                                text: row_fg,
                            })
                            .padding(BoxDimension {
                                left: Dimension::Cells(0.5),
                                right: Dimension::Cells(0.5),
                                top: Dimension::Cells(0.),
                                bottom: Dimension::Cells(0.),
                            })
                            .min_width(Some(Dimension::Percent(1.)))
                            .display(DisplayType::Block)
                            .item_type(UIItemType::Modal(display_idx)),
                    );
                }
            }
            Mode::Adding(input) => {
                rows.push(
                    Element::new(
                        &font,
                        ElementContent::Text(format!("{}: {input}_", tr("Add path"))),
                    )
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
                );
            }
            Mode::ConfirmDelete(idx) => {
                let name = entries.get(*idx).map(|e| e.name.as_str()).unwrap_or("?");
                rows.push(
                    Element::new(
                        &font,
                        ElementContent::Text(tr("Delete").into_owned() + " " + name + "? [y]"),
                    )
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
                );
            }
        }

        // 行内错误（不关浮层）
        if let Some(err) = self.error.borrow().as_ref() {
            rows.push(
                Element::new(&font, ElementContent::Text(format!("! {err}")))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
            );
        }

        rows.push(
            Element::new(
                &font,
                ElementContent::Text(
                    tr("↑↓ preview  Enter apply  a add  d del  r random  Esc cancel").into_owned(),
                ),
            )
            .colors(ElementColors {
                border: BorderColor::default(),
                bg: LinearRgba::TRANSPARENT.into(),
                text: fg.clone(),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(0.5),
                right: Dimension::Cells(0.5),
                top: Dimension::Cells(0.1),
                bottom: Dimension::Cells(0.1),
            })
            .min_width(Some(Dimension::Percent(1.)))
            .display(DisplayType::Block)
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
        );

        let dimensions = term_window.dimensions;
        let size = term_window.terminal_size;
        let desired_width = (size.cols / 2).max(72).min(size.cols);
        let avail_pixel_width =
            size.cols as f32 * term_window.render_metrics.cell_size.width as f32;
        let desired_pixel_width =
            desired_width as f32 * term_window.render_metrics.cell_size.width as f32;

        let element = Element::new(&font, ElementContent::Children(rows))
            .colors(ElementColors {
                border: BorderColor::new(
                    term_window
                        .config
                        .command_palette_bg_color
                        .to_linear()
                        .into(),
                ),
                bg: term_window
                    .config
                    .command_palette_bg_color
                    .to_linear()
                    .into(),
                text: term_window
                    .config
                    .command_palette_fg_color
                    .to_linear()
                    .into(),
            })
            .margin(BoxDimension {
                left: Dimension::Cells(0.25),
                right: Dimension::Cells(0.25),
                top: Dimension::Cells(0.25),
                bottom: Dimension::Cells(0.25),
            })
            .padding(BoxDimension::new(Dimension::Cells(0.25)))
            .border(BoxDimension::new(Dimension::Pixels(1.)))
            .min_width(Some(Dimension::Pixels(desired_pixel_width)))
            // 外框自己也进 hit map：点 padding 一圈不会被当成点外面（WZ-06）
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW));

        let x_adjust = ((avail_pixel_width - padding_left) - desired_pixel_width) / 2.;

        let computed = term_window.compute_element(
            &LayoutContext {
                height: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_height as f32,
                    pixel_cell: metrics.cell_size.height as f32,
                },
                width: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_width as f32,
                    pixel_cell: metrics.cell_size.width as f32,
                },
                bounds: euclid::rect(
                    padding_left + x_adjust,
                    top_pixel_y,
                    desired_pixel_width,
                    size.rows as f32 * term_window.render_metrics.cell_size.height as f32,
                ),
                metrics: &metrics,
                gl_state: term_window.render_state.as_ref().unwrap(),
                zindex: 100,
            },
            &element,
        )?;

        Ok(vec![computed])
    }
}

impl Modal for WallpaperOverlay {
    fn perform_assignment(&self, assignment: &KeyAssignment, term_window: &mut TermWindow) -> bool {
        // 添加路径输入里接受粘贴（剪贴板异步取回后塞进输入行）
        if !matches!(assignment, KeyAssignment::PasteFrom(_)) {
            return false;
        }
        if !matches!(&*self.mode.borrow(), Mode::Adding(_)) {
            return false;
        }
        if let Some(window) = term_window.window.as_ref() {
            let window = window.clone();
            let future = window.get_clipboard(Clipboard::Clipboard);
            promise::spawn::spawn(async move {
                if let Ok(clip) = future.await {
                    window.notify(TermWindowNotif::Apply(Box::new(move |tw| {
                        // 剪贴板文本回到主线程后塞进当前浮层的输入行；
                        // RefCell 内部可变性，downcast_ref 就够
                        if let Some(m) = tw.get_modal() {
                            if let Some(overlay) = m.downcast_ref::<WallpaperOverlay>() {
                                let cleaned: String =
                                    clip.chars().filter(|c| !c.is_control()).collect();
                                if let Mode::Adding(input) = &mut *overlay.mode.borrow_mut() {
                                    input.push_str(cleaned.trim());
                                }
                                tw.invalidate_modal();
                            }
                        }
                    })));
                }
            })
            .detach();
        }
        true
    }

    fn mouse_event(
        &self,
        event: ::window::MouseEvent,
        row: usize,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<()> {
        use ::window::MouseEventKind as WMEK;
        if row == MODAL_CHROME_ROW {
            return Ok(());
        }
        if !matches!(&*self.mode.borrow(), Mode::Browse) {
            return Ok(());
        }
        match event.kind {
            // WZ-11：滚轮滚视口
            WMEK::VertWheel(amount) => {
                self.scroll_rows(-(amount as isize));
                term_window.invalidate_modal();
            }
            WMEK::Move => {
                let len = self.entries.borrow().len();
                if row < len && *self.selected.borrow() != row {
                    self.selected.replace(row);
                    self.preview_selected(term_window);
                    term_window.invalidate_modal();
                }
            }
            WMEK::Press(::window::MousePress::Left) => {
                self.selected.replace(row);
                self.apply_selected(term_window);
            }
            _ => {}
        }
        Ok(())
    }

    fn key_down(
        &self,
        key: KeyCode,
        mods: KeyModifiers,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<bool> {
        let mode = self.mode.borrow().clone();
        match mode {
            Mode::Browse => {
                match (key, mods) {
                    (KeyCode::Escape, KeyModifiers::NONE)
                    | (KeyCode::Char('g'), KeyModifiers::CTRL)
                    | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                        term_window.cancel_modal();
                    }
                    (KeyCode::UpArrow, KeyModifiers::NONE)
                    | (KeyCode::Char('p'), KeyModifiers::CTRL)
                    | (KeyCode::Char('k'), KeyModifiers::NONE)
                    | (KeyCode::Char('p'), KeyModifiers::NONE) => {
                        if self.move_selection(-1) {
                            self.preview_selected(term_window);
                        }
                    }
                    (KeyCode::DownArrow, KeyModifiers::NONE)
                    | (KeyCode::Char('n'), KeyModifiers::CTRL)
                    | (KeyCode::Char('j'), KeyModifiers::NONE)
                    | (KeyCode::Char('n'), KeyModifiers::NONE) => {
                        if self.move_selection(1) {
                            self.preview_selected(term_window);
                        }
                    }
                    (KeyCode::Enter, _) => {
                        self.apply_selected(term_window);
                        return Ok(true);
                    }
                    (KeyCode::Char('r'), KeyModifiers::NONE) => {
                        // r 随机：预览级动作（Enter 才持久化）
                        let len = self.entries.borrow().len();
                        if len > 0 {
                            let target = random_below(len);
                            self.selected.replace(target);
                            self.preview_selected(term_window);
                        }
                    }
                    (KeyCode::Char('a'), KeyModifiers::NONE) => {
                        *self.mode.borrow_mut() = Mode::Adding(String::new());
                        self.error.replace(None);
                    }
                    (KeyCode::Char('d'), KeyModifiers::NONE) => {
                        let selected = *self.selected.borrow();
                        if self.entries.borrow().get(selected).is_some() {
                            *self.mode.borrow_mut() = Mode::ConfirmDelete(selected);
                        }
                    }
                    _ => return Ok(false),
                }
            }
            Mode::Adding(input) => match (key, mods) {
                (KeyCode::Escape, KeyModifiers::NONE) => {
                    *self.mode.borrow_mut() = Mode::Browse;
                    self.error.replace(None);
                }
                (KeyCode::Enter, KeyModifiers::NONE) => {
                    self.finish_add(&input, term_window);
                }
                (KeyCode::Backspace, KeyModifiers::NONE) => {
                    let mut input = input;
                    input.pop();
                    *self.mode.borrow_mut() = Mode::Adding(input);
                }
                (KeyCode::Tab, KeyModifiers::NONE) => {
                    let (completed, _) = complete_path(&input);
                    *self.mode.borrow_mut() = Mode::Adding(completed);
                }
                (KeyCode::Char(c), KeyModifiers::NONE) => {
                    let mut input = input;
                    input.push(c);
                    *self.mode.borrow_mut() = Mode::Adding(input);
                }
                _ => return Ok(false),
            },
            Mode::ConfirmDelete(idx) => {
                match (key, mods) {
                    (KeyCode::Char('y'), KeyModifiers::NONE) => {
                        self.confirm_delete(idx, term_window);
                    }
                    _ => {
                        // 任意其它键取消删除（与 herdr 强删确认同构）
                        *self.mode.borrow_mut() = Mode::Browse;
                    }
                }
            }
        }
        term_window.invalidate_modal();
        Ok(true)
    }

    fn computed_element(
        &self,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<Ref<'_, [ComputedElement]>> {
        if self.element.borrow().is_none() {
            let element = self.compute(term_window)?;
            self.element.borrow_mut().replace(element);
        }
        Ok(Ref::map(self.element.borrow(), |v| {
            v.as_ref().unwrap().as_slice()
        }))
    }

    fn reconfigure(&self, _term_window: &mut TermWindow) {
        self.element.borrow_mut().take();
    }

    /// 批 13：关闭时还原未确认的预览。Enter 已把 previewing 清空，
    /// 只有 Esc/点外/被顶掉且预览仍在时才回到进入时的层栈。
    fn on_dismissed(&self, term_window: &mut TermWindow) {
        if self.previewing.borrow().is_some() {
            term_window.window_background = self.original.clone();
            invalidate_window(term_window);
        }
    }
}

/// 壁纸目录：wezterm.config_dir/backdrops（与 backdrops.lua 同一真源）；
/// GUI 进程的 WEZTERM_CONFIG_DIR 已由加载链设为生效配置目录。
fn wallpaper_dir() -> PathBuf {
    std::env::var_os("WEZTERM_CONFIG_DIR")
        .map(PathBuf::from)
        .map(|d| d.join("backdrops"))
        .unwrap_or_else(|| config::HOME_DIR.join(".config/wezterm/backdrops"))
}

/// 均匀随机数（不引 rand 依赖）：纳秒时间戳线性同余
fn random_below(bound: usize) -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    (nanos % bound.max(1) as u64) as usize
}

/// Convenience: open the wallpaper manager modally
pub fn open_wallpaper_overlay(term_window: &mut TermWindow) {
    let modal = Rc::new(WallpaperOverlay::new(term_window));
    term_window.set_modal(modal);
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wezterm-wallpaper-test-{}-{}-{}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 一张合法的最小 PNG（1x1）
    const PNG_1PX: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 252, 255, 255, 191, 0,
        5, 254, 2, 254, 167, 53, 129, 132, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn wallpaper_scan_sorts_and_filters_by_extension() {
        let dir = temp_dir("scan");
        std::fs::write(dir.join("b.png"), PNG_1PX).unwrap();
        std::fs::write(dir.join("a.jpg"), b"not really a jpeg but named one").unwrap();
        std::fs::write(dir.join("readme.txt"), b"no").unwrap();
        std::fs::create_dir_all(dir.join("nested.png")).unwrap(); // 目录不收
        let entries = scan_wallpapers(&dir);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.jpg", "b.png"]);
        // PNG 读头拿到 1x1；假 jpeg 没有尺寸
        assert_eq!(entries[1].dimensions, Some((1, 1)));
        assert_eq!(entries[0].dimensions, None);
        assert!(entries[0].size_bytes > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wallpaper_scan_empty_dir_is_empty() {
        let dir = temp_dir("empty");
        assert!(scan_wallpapers(&dir).is_empty());
        assert!(scan_wallpapers(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_image_path_checks_existence_and_decodability() {
        let dir = temp_dir("validate");
        let good = dir.join("ok.png");
        std::fs::write(&good, PNG_1PX).unwrap();
        assert!(validate_image_path(good.to_str().unwrap()).is_ok());

        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"this is not an image").unwrap();
        assert!(validate_image_path(bad.to_str().unwrap()).is_err());
        assert!(validate_image_path(dir.join("missing.png").to_str().unwrap()).is_err());
        assert!(validate_image_path(dir.to_str().unwrap()).is_err()); // 目录
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_path_fills_common_prefix() {
        let dir = temp_dir("complete");
        std::fs::create_dir_all(dir.join("wallpapers")).unwrap();
        std::fs::write(dir.join("wallpaper-alpha.png"), PNG_1PX).unwrap();
        std::fs::write(dir.join("wallpaper-beta.png"), PNG_1PX).unwrap();
        let prefix = format!("{}/wall", dir.display());
        let (completed, matches) = complete_path(&prefix);
        assert_eq!(matches, 3); // wallpapers/ + 两个文件
        assert!(completed.ends_with("wallpaper"));
        let (completed2, matches2) = complete_path(&format!("{}/wallpaper-alpha", dir.display()));
        assert_eq!(matches2, 1);
        assert!(completed2.ends_with("wallpaper-alpha.png"));
        // 无匹配保持原输入
        let (same, zero) = complete_path(&format!("{}/zzz", dir.display()));
        assert_eq!(zero, 0);
        assert_eq!(same, format!("{}/zzz", dir.display()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_into_wallpaper_dir_dedupes_by_content() {
        let dir = temp_dir("copy");
        let src_dir = temp_dir("src");
        let src = src_dir.join("photo.png");
        std::fs::write(&src, PNG_1PX).unwrap();

        let first = copy_into_wallpaper_dir(&src, &dir).unwrap();
        assert_eq!(first.file_name().unwrap(), "photo.png");
        // 同名同内容：复用不加后缀
        let again = copy_into_wallpaper_dir(&src, &dir).unwrap();
        assert_eq!(again.file_name().unwrap(), "photo.png");
        // 同名不同内容：哈希后缀
        std::fs::write(&src, b"different bytes").unwrap();
        // 不同内容须可解码校验发生在调用前；copy 本身不校验
        let third = copy_into_wallpaper_dir(&src, &dir).unwrap();
        let third_name = third.file_name().unwrap().to_str().unwrap().to_string();
        assert!(third_name.starts_with("photo-"));
        assert!(third_name.ends_with(".png"));
        assert_ne!(third_name, "photo.png");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn preview_defs_swap_first_layer_and_keep_the_rest() {
        // 预览只换第一层（壁纸），遮罩层原样保留；无背景层时放单层
        let mk_layer = |path: &str| LoadedBackgroundLayer {
            source: std::sync::Arc::new(termwiz::image::ImageData::with_data(
                termwiz::image::ImageDataType::new_single_frame(1, 1, vec![0, 0, 0, 255]),
            )),
            def: BackgroundLayer {
                source: BackgroundSource::File(ImageFileSourceWrap::from(ImageFileSource {
                    path: path.to_string(),
                    speed: 1.0,
                })),
                ..preview_layer_fixture()
            },
        };
        let mask = BackgroundLayer {
            source: BackgroundSource::Color(
                termwiz::color::SrgbaTuple(0.16, 0.16, 0.16, 1.0).into(),
            ),
            opacity: 0.92,
            ..preview_layer_fixture()
        };
        let layers = vec![
            mk_layer("/pics/old.png"),
            LoadedBackgroundLayer {
                source: std::sync::Arc::new(termwiz::image::ImageData::with_data(
                    termwiz::image::ImageDataType::new_single_frame(1, 1, vec![0, 0, 0, 255]),
                )),
                def: mask.clone(),
            },
        ];
        let defs = preview_defs(&layers, Path::new("/pics/new.png"));
        assert_eq!(defs.len(), 2);
        match &defs[0].source {
            BackgroundSource::File(f) => assert_eq!(f.path, "/pics/new.png"),
            other => panic!("expected file layer, got {other:?}"),
        }
        // 遮罩层原样（颜色与 opacity 不动）
        assert_eq!(defs[1].opacity, 0.92);
        assert!(matches!(defs[1].source, BackgroundSource::Color(_)));

        let empty = preview_defs(&[], Path::new("/pics/only.png"));
        assert_eq!(empty.len(), 1);
    }

    /// BackgroundLayer 无 Default；预览路径的字段默认集中在这里
    fn preview_layer_fixture() -> BackgroundLayer {
        BackgroundLayer {
            source: BackgroundSource::Color(termwiz::color::SrgbaTuple(0.0, 0.0, 0.0, 1.0).into()),
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
            width: Default::default(),
            height: Default::default(),
        }
    }
}
