//! Resolving a desktop entry's `Icon=` to a file the renderer can draw (#85).
//!
//! Two things live here, and only one of them touches the disk.
//!
//! [`classify`] and [`IconArt`] are the decision: given an icon name and a way
//! to look names up, which file do we draw and with which widget. [`IconCache`]
//! is the memoisation around it, and it caches the *negative* answer too --
//! an application whose icon the theme cannot resolve is the common case on a
//! mixed desktop, and re-walking every theme directory for it on each keystroke
//! is the cost this exists to avoid.
//!
//! The lookup itself is injected rather than called, so the tests here run
//! against a closure and never read the invoking user's icon themes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which widget draws a resolved icon.
///
/// SVG and raster are separate widgets in Iced, so the distinction has to
/// survive out of resolution rather than being rediscovered at draw time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconArt {
    /// A PNG, XPM or other bitmap: `iced::widget::image`.
    Raster(PathBuf),
    /// An SVG: `iced::widget::svg`.
    Vector(PathBuf),
}

impl IconArt {
    /// The file this art came from.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            IconArt::Raster(path) | IconArt::Vector(path) => path,
        }
    }
}

/// Which widget a resolved file needs, by extension.
///
/// **PNG, JPEG and SVG only, and that is a decision rather than an
/// oversight.** PNG and SVG are what icon themes ship, and JPEG is what remote
/// images (avatars, mostly) are; the build enables exactly those decoders
/// (see `Cargo.toml`), so accepting another format here would resolve a file
/// the renderer then cannot draw -- an empty box, which is worse than the
/// initial the row falls back to. `.svgz` is gzipped SVG and Iced's `svg`
/// widget does not decompress it, so it is a miss for the same reason.
#[must_use]
pub fn classify(path: &Path) -> Option<IconArt> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Some(IconArt::Vector(path.to_path_buf())),
        "png" | "jpg" | "jpeg" => Some(IconArt::Raster(path.to_path_buf())),
        _ => None,
    }
}

/// Resolve one `Icon=` value.
///
/// The key may be an absolute path rather than a theme name -- the desktop
/// entry specification allows it, and Flatpak-exported entries written by hand
/// sometimes use it. An absolute path is taken as given and **not** offered to
/// the theme lookup, which would search for a name containing slashes and find
/// nothing.
pub fn resolve(icon: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<IconArt> {
    if icon.is_empty() {
        return None;
    }

    let candidate = Path::new(icon);
    if candidate.is_absolute() {
        return classify(candidate);
    }

    classify(&find(icon)?)
}

/// Resolved icons, by `Icon=` value.
///
/// Holds the misses as well as the hits: see the module docs.
#[derive(Debug, Default)]
pub struct IconCache {
    entries: HashMap<String, Option<IconArt>>,
}

impl IconCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The art for `icon`, resolving and remembering it on first ask.
    pub fn get(&mut self, icon: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<&IconArt> {
        if !self.entries.contains_key(icon) {
            self.entries.insert(icon.to_owned(), resolve(icon, find));
        }
        self.entries.get(icon).and_then(Option::as_ref)
    }

    /// The art for `icon` if it has already been resolved.
    ///
    /// Immutable, because `view` is. Resolution walks theme directories, which
    /// is disk work and does not belong in a draw; [`IconCache::warm`] does it
    /// from `update`, for the rows about to be drawn, and this reads the
    /// result. A name that was never warmed reads as absent and the row falls
    /// back to its initial, which is also what a genuine miss looks like --
    /// the two are indistinguishable on purpose, so a warming bug degrades to
    /// today's appearance rather than to an empty row.
    #[must_use]
    pub fn cached(&self, icon: &str) -> Option<&IconArt> {
        self.entries.get(icon).and_then(Option::as_ref)
    }

    /// Resolve every name in `icons` that is not already known.
    ///
    /// Called with the icons of the rows about to be drawn, so the cost is
    /// bounded by what is on screen rather than by the size of the index.
    pub fn warm<'a>(
        &mut self,
        icons: impl IntoIterator<Item = &'a str>,
        find: &dyn Fn(&str) -> Option<PathBuf>,
    ) {
        for icon in icons {
            self.get(icon, find);
        }
    }

    /// How many names have been resolved, hit or miss. For tests and `doctor`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been resolved yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What a row's icon slot draws, decided in `update` and read by `view`.
///
/// Ports the part of `ImageURL` a builtin list row uses: a themed or
/// application icon keeps its own colours; a builtin icon is single-colour and
/// drawn in `fill` (the row's text colour when `None`), on a rounded `tile`
/// when the C++ sets `setBackgroundTint`, with a `badge` in the corner when it
/// sets `setBadge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    /// A file from an icon theme or an application.
    Art(IconArt),
    /// One of the builtin icons, by name.
    Builtin {
        /// The builtin icon's name, e.g. `copy-clipboard`.
        name: String,
        /// Its colour; `None` is the row's text colour.
        fill: Option<crate::design::Rgb>,
        /// The tile behind it.
        tile: Option<crate::design::Rgb>,
        /// A second builtin drawn small in the bottom-right corner.
        badge: Option<&'static str>,
    },
    /// An emoji or another glyph, drawn as text (`ImageURLType::Emoji` and
    /// `Symbol`).
    Text(String),
}

impl Glyph {
    /// A plain builtin, in the row's text colour.
    #[must_use]
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin {
            name: name.into(),
            fill: None,
            tile: None,
            badge: None,
        }
    }
}

/// libadwaita's accent colours (`AdwAccentColor`, 1.6), which a command tile
/// is drawn from before [`tile_tone`] evens them out. Cyan is Adwaita's teal;
/// gray is the palette's neutral `dark_1` rather than the bluish slate accent.
const TILE_ACCENTS: [(compass_core::commands::Tile, crate::design::Rgb); 8] = {
    use crate::design::Rgb;
    use compass_core::commands::Tile;
    [
        (Tile::Red, Rgb::new(0xe6, 0x2d, 0x42)),
        (Tile::Orange, Rgb::new(0xed, 0x5b, 0x00)),
        (Tile::Yellow, Rgb::new(0xc8, 0x88, 0x00)),
        (Tile::Green, Rgb::new(0x3a, 0x94, 0x4a)),
        (Tile::Cyan, Rgb::new(0x21, 0x90, 0xa4)),
        (Tile::Blue, Rgb::new(0x35, 0x84, 0xe4)),
        (Tile::Purple, Rgb::new(0x91, 0x41, 0xac)),
        (Tile::Gray, Rgb::new(0x77, 0x76, 0x7b)),
    ]
};

/// The colour a command's tile is filled with, `accent` being the palette's.
#[must_use]
pub fn tile_color(
    tile: compass_core::commands::Tile,
    accent: crate::design::Rgb,
) -> crate::design::Rgb {
    let base = TILE_ACCENTS
        .iter()
        .find(|(candidate, _)| *candidate == tile)
        .map_or(accent, |(_, rgb)| *rgb);
    tile_tone(base)
}

fn to_core(rgb: crate::design::Rgb) -> compass_core::contrast::Rgb {
    compass_core::contrast::Rgb::new(rgb.r, rgb.g, rgb.b)
}

fn from_core(rgb: compass_core::contrast::Rgb) -> crate::design::Rgb {
    crate::design::Rgb::new(rgb.r, rgb.g, rgb.b)
}

/// `clampTileTone`: every tile pulled into one tonal band (saturation
/// 0.55–0.8 unless grey, lightness 0.42–0.52), so a row of them reads as a
/// family and always carries a light glyph.
#[must_use]
pub fn tile_tone(color: crate::design::Rgb) -> crate::design::Rgb {
    use compass_core::contrast::{Hsl, from_hsl, to_hsl};
    let hsl = to_hsl(to_core(color));
    // Qt's HSL is on 0..=255; the C++'s float bounds scaled onto it.
    let saturation = if hsl.saturation > 12 {
        hsl.saturation.clamp(140, 204)
    } else {
        hsl.saturation
    };
    from_core(from_hsl(Hsl {
        hue: Some(hsl.hue.unwrap_or(0)),
        saturation,
        lightness: hsl.lightness.clamp(107, 133),
    }))
}

/// The glyph colour on a tile: `getTonalContrastColor(tile, 5, 0.1)`.
#[must_use]
pub fn on_tile(tile: crate::design::Rgb) -> crate::design::Rgb {
    from_core(compass_core::contrast::tonal_contrast_color_with_amount(
        to_core(tile),
        5.0,
        0.1,
    ))
}

/// The root search's Update row: `download` on a blue tile
/// (`RootUpdateSection::itemIcon`).
#[must_use]
pub fn update_glyph(accent: crate::design::Rgb) -> Glyph {
    let tile = tile_color(compass_core::commands::Tile::Blue, accent);
    Glyph::Builtin {
        name: "download".to_owned(),
        fill: Some(on_tile(tile)),
        tile: Some(tile),
        badge: None,
    }
}

/// A builtin command's glyph: its icon on its C++ tile, with its badge.
#[must_use]
pub fn command_glyph(
    command: &compass_core::commands::BuiltinCommand,
    accent: crate::design::Rgb,
) -> Glyph {
    let tile = tile_color(command.kind.tile(), accent);
    Glyph::Builtin {
        name: command.icon.to_owned(),
        fill: Some(on_tile(tile)),
        tile: Some(tile),
        badge: command.kind.badge(),
    }
}

/// The MIME type a file is shown as, and the two theme names its icon is
/// looked up by: `QMimeType::iconName` and `genericIconName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIconNames {
    /// The MIME type, e.g. `image/png`.
    pub mime: String,
    /// The specific icon, e.g. `image-png`.
    pub icon: String,
    /// The generic one, e.g. `image-x-generic`.
    pub generic: String,
}

/// [`FileIconNames`] for a MIME type, by the shared-mime-info defaults Qt
/// falls back on: the type with `/` made `-`, and the media type with
/// `-x-generic`. A directory's generic icon is `folder`, as shared-mime-info
/// declares for `inode/directory`.
#[must_use]
pub fn names_for_mime(mime: &str) -> FileIconNames {
    let icon = mime.replace('/', "-");
    let generic = if mime == "inode/directory" {
        "folder".to_owned()
    } else {
        let media = mime.split('/').next().unwrap_or(mime);
        format!("{media}-x-generic")
    };
    FileIconNames {
        mime: mime.to_owned(),
        icon,
        generic,
    }
}

/// The MIME type of the file at `path`: `inode/directory` for a directory,
/// else by its extension through `mime_guess`, else
/// `application/octet-stream`.
#[must_use]
pub fn mime_for_path(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_owned();
    }
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

/// `renderFileIcon`: the theme's icon for the file's MIME type, else its
/// generic icon, else the builtin `folder` or `blank-document` in the text
/// colour.
pub fn file_glyph(path: &Path, find: &dyn Fn(&str) -> Option<PathBuf>) -> Glyph {
    let names = names_for_mime(&mime_for_path(path));
    if let Some(art) = resolve(&names.icon, find).or_else(|| resolve(&names.generic, find)) {
        return Glyph::Art(art);
    }
    Glyph::builtin(if names.mime == "inode/directory" {
        "folder"
    } else {
        "blank-document"
    })
}

/// A clipboard row's builtin, by what was copied (`iconForEntry`).
#[must_use]
pub fn clipboard_glyph(kind: crate::backend::ClipboardRowKind) -> Glyph {
    use crate::backend::ClipboardRowKind;
    Glyph::builtin(match kind {
        ClipboardRowKind::Image => "image",
        ClipboardRowKind::Link => "link",
        ClipboardRowKind::Text => "text",
        ClipboardRowKind::File => "folder",
        ClipboardRowKind::Unknown => compass_core::builtin_icon::UNKNOWN,
    })
}

/// The pickers' mark on the current default: `CheckCircle` filled green.
#[must_use]
pub fn default_mark() -> Glyph {
    Glyph::Builtin {
        name: "check-circle".to_owned(),
        fill: Some(TILE_ACCENTS[3].1),
        tile: None,
        badge: None,
    }
}

/// The colour a semantic name (`ImageURL`'s `fill` and `bg_tint`) stands
/// for in the Vicinae dark theme, `accent` being the palette's; `None` for
/// the text colours, which follow the row.
#[must_use]
pub fn semantic_color(name: &str, accent: crate::design::Rgb) -> Option<crate::design::Rgb> {
    use compass_core::commands::Tile;
    let tile = match name {
        "Red" => Tile::Red,
        "Orange" => Tile::Orange,
        "Yellow" => Tile::Yellow,
        "Green" => Tile::Green,
        "Cyan" => Tile::Cyan,
        "Blue" => Tile::Blue,
        // `ThemeFile::vicinaeDark` gives magenta and purple one colour.
        "Purple" | "Magenta" => Tile::Purple,
        "Accent" => return Some(accent),
        _ => return None,
    };
    TILE_ACCENTS
        .iter()
        .find(|(candidate, _)| *candidate == tile)
        .map(|(_, rgb)| *rgb)
}

/// A literal colour as `QColor(name)` reads the common forms: `#rgb`,
/// `#rrggbb` and `#aarrggbb` (the alpha dropped).
fn literal_color(text: &str) -> Option<crate::design::Rgb> {
    let hex = text.strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    let nibble = |at: usize| {
        u8::from_str_radix(hex.get(at..=at)?, 16)
            .ok()
            .map(|n| n * 17)
    };
    match hex.len() {
        3 => Some(crate::design::Rgb::new(nibble(0)?, nibble(1)?, nibble(2)?)),
        6 => Some(crate::design::Rgb::new(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(crate::design::Rgb::new(byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

fn color_like(
    color: &compass_core::image_url::ColorLike,
    accent: crate::design::Rgb,
) -> Option<crate::design::Rgb> {
    match color {
        compass_core::image_url::ColorLike::Semantic(name) => semantic_color(name, accent),
        compass_core::image_url::ColorLike::Literal(text) => literal_color(text),
    }
}

/// What [`url_glyph`] resolves an `ImageURL` against.
pub struct UrlLookup<'a> {
    /// An icon theme name to a file.
    pub find: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// A remote image's cached file, once it has been fetched.
    pub remote: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// Where favicons come from.
    pub favicon: compass_core::favicon::Service,
    /// The palette's accent, for `Accent` tints.
    pub accent: crate::design::Rgb,
}

impl std::fmt::Debug for UrlLookup<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlLookup")
            .field("favicon", &self.favicon)
            .field("accent", &self.accent)
            .finish_non_exhaustive()
    }
}

/// The remote URL an `ImageURL` is drawn from, when it is one: an `http`
/// image, or a favicon through the configured service.
#[must_use]
pub fn remote_source(
    url: &compass_core::image_url::ImageUrl,
    favicon: compass_core::favicon::Service,
) -> Option<String> {
    use compass_core::image_url::ImageUrlType;
    match url.kind {
        ImageUrlType::Http => Some(url.name.clone()),
        ImageUrlType::Favicon => favicon.url(&url.name),
        _ => None,
    }
}

/// An `ImageURL` as a row draws it, as `ImageRenderer` renders each type: a
/// builtin in its fill on its tint's tile, a file, an emoji or symbol as
/// text, a theme icon, a fetched image or favicon, a file's type icon;
/// else its fallback. `None` when nothing can be drawn (yet: a remote image
/// that has not arrived).
#[must_use]
pub fn url_glyph(url: &compass_core::image_url::ImageUrl, lookup: &UrlLookup<'_>) -> Option<Glyph> {
    use compass_core::image_url::ImageUrlType;
    let drawn = match url.kind {
        ImageUrlType::Builtin => {
            let tile = url
                .background_tint
                .as_ref()
                .and_then(|tint| color_like(tint, lookup.accent))
                .map(tile_tone);
            let fill = match tile {
                Some(tile) => Some(on_tile(tile)),
                None => url
                    .fill
                    .as_ref()
                    .and_then(|fill| color_like(fill, lookup.accent)),
            };
            Some(Glyph::Builtin {
                name: url.name.clone(),
                fill,
                tile,
                badge: None,
            })
        }
        ImageUrlType::Local => classify(Path::new(&url.name)).map(Glyph::Art),
        ImageUrlType::Emoji | ImageUrlType::Symbol => {
            (!url.name.is_empty()).then(|| Glyph::Text(url.name.clone()))
        }
        ImageUrlType::System => resolve(&url.name, lookup.find).map(Glyph::Art),
        ImageUrlType::Http | ImageUrlType::Favicon => remote_source(url, lookup.favicon)
            .and_then(|remote| (lookup.remote)(&remote))
            .and_then(|path| classify(&path))
            .map(Glyph::Art),
        ImageUrlType::FileIcon => Some(file_glyph(Path::new(&url.name), lookup.find)),
        _ => None,
    };
    drawn.or_else(|| {
        let fallback = compass_core::image_url::ImageUrl::parse(url.fallback.as_deref()?);
        fallback.is_valid().then(|| url_glyph(&fallback, lookup))?
    })
}

/// The shift `applyBackdrop` gives a tile's colour for each end of its
/// gradient: hue, saturation and lightness in HSL's 0–1 units.
fn shifted(color: crate::design::Rgb, dh: f32, ds: f32, dl: f32) -> crate::design::Rgb {
    let (r, g, b) = (
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = f32::midpoint(max, min);
    let d = max - min;
    let (mut h, s) = if d <= f32::EPSILON {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if (max - r).abs() <= f32::EPSILON {
            ((g - b) / d).rem_euclid(6.0)
        } else if (max - g).abs() <= f32::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    h = (h + dh + 1.0).rem_euclid(1.0);
    let s = (s + ds).clamp(0.0, 1.0);
    let l = (l + dl).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let channel = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    crate::design::Rgb::new(channel(r), channel(g), channel(b))
}

/// `applyBackdrop`'s tile gradient, top then bottom: the tile a little
/// lighter and warmer at the top, a little deeper at the bottom.
#[must_use]
pub fn tile_gradient(tile: crate::design::Rgb) -> (crate::design::Rgb, crate::design::Rgb) {
    (
        shifted(tile, 0.025, -0.03, 0.10),
        shifted(tile, -0.015, 0.06, -0.05),
    )
}

/// The drop shadow under a tile's glyph: its silhouette in black at
/// `QColor(0, 0, 0, 70)`, moved down by 3.5% of the side.
pub const TILE_SHADOW_ALPHA: u8 = 70;
/// How far the shadow sits under the glyph, as a share of the tile's side.
pub const TILE_SHADOW_OFFSET: f32 = 0.035;

/// The side masked images are drawn at, in pixels: enough for a grid cell
/// on a doubled display.
pub const MASK_SIDE: u32 = 128;

/// Clips straight-alpha RGBA `pixels` of `width` by `height` to `mask`, as
/// `applyCircleMask` and `applyRoundedRectMask` do: the ellipse inscribed in
/// the image, or a rectangle whose corners are a quarter of the shorter side
/// round; antialiased over one pixel.
pub fn apply_mask(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    mask: compass_core::image_url::ImageMask,
) {
    use compass_core::image_url::ImageMask;
    let (w, h) = (width as f32, height as f32);
    let coverage: Box<dyn Fn(f32, f32) -> f32> = match mask {
        ImageMask::None => return,
        ImageMask::Circle => {
            let (rx, ry) = (w / 2.0, h / 2.0);
            Box::new(move |x, y| {
                // The distance to the ellipse's edge, in pixels, near it.
                let (dx, dy) = ((x - rx) / rx, (y - ry) / ry);
                let norm = (dx * dx + dy * dy).sqrt();
                ((1.0 - norm) * rx.min(ry) + 0.5).clamp(0.0, 1.0)
            })
        }
        ImageMask::RoundedRectangle => {
            let radius = w.min(h) * 0.25;
            Box::new(move |x, y| {
                let cx = x.clamp(radius, w - radius);
                let cy = y.clamp(radius, h - radius);
                let (dx, dy) = (x - cx, y - cy);
                (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
            })
        }
    };
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let index = index as u32;
        let (x, y) = ((index % width) as f32 + 0.5, (index / width) as f32 + 0.5);
        let cover = coverage(x, y);
        pixel[3] = (f32::from(pixel[3]) * cover).round() as u8;
    }
}

/// `art` drawn into at most `side` pixels square, aspect kept, as
/// straight-alpha RGBA with its width and height; an SVG drawn in `tint`
/// when given. `None` when the file cannot be read or decoded.
#[must_use]
pub fn rasterize(art: &IconArt, side: u32, tint: Option<[u8; 3]>) -> Option<(u32, u32, Vec<u8>)> {
    match art {
        IconArt::Raster(path) => {
            let decoded = image::open(path).ok()?;
            let fitted = if decoded.width() > side || decoded.height() > side {
                decoded.resize(side, side, image::imageops::FilterType::Triangle)
            } else {
                decoded
            };
            let rgba = fitted.to_rgba8();
            Some((rgba.width(), rgba.height(), rgba.into_raw()))
        }
        IconArt::Vector(path) => {
            use resvg::{tiny_skia, usvg};
            let data = std::fs::read(path).ok()?;
            let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
            let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
            let size = tree.size();
            let full = side as f32;
            let scale = (full / size.width()).min(full / size.height());
            let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
                (full - size.width() * scale) / 2.0,
                (full - size.height() * scale) / 2.0,
            );
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            let mut rgba = Vec::with_capacity((side * side * 4) as usize);
            for pixel in pixmap.pixels() {
                let straight = pixel.demultiply();
                let [r, g, b] = tint.unwrap_or([straight.red(), straight.green(), straight.blue()]);
                rgba.extend_from_slice(&[r, g, b, straight.alpha()]);
            }
            Some((side, side, rgba))
        }
    }
}

/// A masked image's file, mask and tint.
type MaskKey = (PathBuf, compass_core::image_url::ImageMask, Option<[u8; 3]>);

/// `art` clipped to `mask`, ready for `iced::widget::image`, drawn once per
/// file, mask and tint and kept: `view` asks for it on every frame.
#[derive(Debug, Default)]
pub struct MaskedCache {
    entries: std::sync::Mutex<HashMap<MaskKey, Option<iced::widget::image::Handle>>>,
}

impl MaskedCache {
    /// The masked image, drawing it on first ask; `None` when the file
    /// cannot be drawn.
    pub fn get(
        &self,
        art: &IconArt,
        mask: compass_core::image_url::ImageMask,
        tint: Option<[u8; 3]>,
    ) -> Option<iced::widget::image::Handle> {
        let key = (art.path().to_path_buf(), mask, tint);
        let mut entries = self.entries.lock().ok()?;
        entries
            .entry(key)
            .or_insert_with(|| {
                let (width, height, mut pixels) = rasterize(art, MASK_SIDE, tint)?;
                apply_mask(&mut pixels, width, height, mask);
                Some(iced::widget::image::Handle::from_rgba(
                    width, height, pixels,
                ))
            })
            .clone()
    }
}

/// File glyphs by path, warmed from `update` as [`IconCache`] is.
#[derive(Debug, Default)]
pub struct FileGlyphCache {
    entries: HashMap<String, Glyph>,
}

impl FileGlyphCache {
    /// Resolve every path in `paths` that is not already known.
    pub fn warm<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a str>,
        find: &dyn Fn(&str) -> Option<PathBuf>,
    ) {
        for path in paths {
            if !self.entries.contains_key(path) {
                self.entries
                    .insert(path.to_owned(), file_glyph(Path::new(path), find));
            }
        }
    }

    /// The glyph for `path`, if it has been warmed.
    #[must_use]
    pub fn cached(&self, path: &str) -> Option<&Glyph> {
        self.entries.get(path)
    }

    /// How many paths have been resolved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been resolved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A theme lookup with a fixed set of answers.
    fn theme(answers: &[(&str, &str)]) -> impl Fn(&str) -> Option<PathBuf> {
        let answers: Vec<(String, PathBuf)> = answers
            .iter()
            .map(|(name, path)| ((*name).to_owned(), PathBuf::from(*path)))
            .collect();
        move |name: &str| {
            answers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, path)| path.clone())
        }
    }

    #[test]
    fn an_svg_and_a_png_need_different_widgets() {
        assert_eq!(
            classify(Path::new("/i/firefox.svg")),
            Some(IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert_eq!(
            classify(Path::new("/i/firefox.png")),
            Some(IconArt::Raster(PathBuf::from("/i/firefox.png")))
        );
    }

    #[test]
    fn the_extension_is_matched_without_regard_to_case() {
        assert_eq!(
            classify(Path::new("/i/App.SVG")),
            Some(IconArt::Vector(PathBuf::from("/i/App.SVG")))
        );
        assert_eq!(
            classify(Path::new("/i/App.PNG")),
            Some(IconArt::Raster(PathBuf::from("/i/App.PNG")))
        );
    }

    #[test]
    fn a_format_neither_widget_can_draw_resolves_to_nothing() {
        // `.svgz` is gzipped SVG: the svg widget does not decompress it and the
        // image widget cannot read it, so the row falls back to the initial
        // rather than drawing an empty box.
        assert_eq!(classify(Path::new("/i/app.svgz")), None);
        assert_eq!(classify(Path::new("/i/app.icns")), None);
        assert_eq!(classify(Path::new("/i/app")), None);
        // Formats the `image` crate can decode but this build ships no decoder
        // for. Accepting one would draw an empty box.
        assert_eq!(classify(Path::new("/i/app.webp")), None);
        assert_eq!(classify(Path::new("/i/app.xpm")), None);
    }

    #[test]
    fn an_absolute_icon_value_is_taken_as_given() {
        // The desktop entry specification allows `Icon=` to be a path. Handing
        // it to the theme lookup would search for a name containing slashes.
        let find = theme(&[]);
        assert_eq!(
            resolve("/opt/thing/icon.png", &find),
            Some(IconArt::Raster(PathBuf::from("/opt/thing/icon.png")))
        );
    }

    #[test]
    fn a_name_goes_through_the_theme_lookup() {
        let find = theme(&[("firefox", "/usr/share/icons/h/firefox.svg")]);
        assert_eq!(
            resolve("firefox", &find),
            Some(IconArt::Vector(PathBuf::from(
                "/usr/share/icons/h/firefox.svg"
            )))
        );
    }

    #[test]
    fn a_name_the_theme_does_not_have_resolves_to_nothing() {
        let find = theme(&[]);
        assert_eq!(resolve("no-such-app", &find), None);
    }

    #[test]
    fn an_empty_icon_value_is_not_a_lookup() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };
        assert_eq!(resolve("", &find), None);
        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    #[test]
    fn the_cache_resolves_a_name_once() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            Some(PathBuf::from("/i/firefox.png"))
        };

        let mut cache = IconCache::new();
        for _ in 0..5 {
            assert!(cache.get("firefox", &find).is_some());
        }
        assert_eq!(*asked.borrow(), ["firefox"]);
    }

    #[test]
    fn the_cache_remembers_a_miss_too() {
        // The point of the cache. An application whose icon the theme cannot
        // resolve is the common case on a mixed desktop, and re-walking every
        // theme directory for it on each keystroke is what this avoids.
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };

        let mut cache = IconCache::new();
        for _ in 0..5 {
            assert!(cache.get("no-such-app", &find).is_none());
        }
        assert_eq!(*asked.borrow(), ["no-such-app"]);
        assert_eq!(cache.len(), 1, "the miss was not remembered");
    }

    #[test]
    fn warming_resolves_the_names_it_is_given_and_nothing_else() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            Some(PathBuf::from(format!("/i/{name}.png")))
        };

        let mut cache = IconCache::new();
        cache.warm(["a", "b"], &find);
        assert_eq!(*asked.borrow(), ["a", "b"]);

        // The cost is bounded by what is on screen: warming the same rows
        // again asks nothing.
        cache.warm(["a", "b"], &find);
        assert_eq!(*asked.borrow(), ["a", "b"]);
    }

    #[test]
    fn a_name_that_was_never_warmed_reads_as_absent() {
        // Not an error and not a panic: the row falls back to its initial, the
        // same as for a name the theme genuinely does not have.
        let find = theme(&[("firefox", "/i/firefox.png")]);
        let mut cache = IconCache::new();
        cache.warm(["firefox"], &find);
        assert!(cache.cached("firefox").is_some());
        assert!(cache.cached("thunderbird").is_none());
    }

    #[test]
    fn different_names_are_cached_separately() {
        let find = theme(&[("a", "/i/a.png"), ("b", "/i/b.svg")]);
        let mut cache = IconCache::new();
        assert_eq!(
            cache.get("a", &find).map(IconArt::path),
            Some(Path::new("/i/a.png"))
        );
        assert_eq!(
            cache.get("b", &find).map(IconArt::path),
            Some(Path::new("/i/b.svg"))
        );
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_file_takes_its_mime_icon_then_the_generic_one_then_a_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("photo.png");
        let notes = dir.path().join("notes.txt");
        let blob = dir.path().join("blob.qqqq");
        for file in [&png, &notes, &blob] {
            std::fs::write(file, b"x").unwrap();
        }
        let find = theme(&[
            ("image-png", "/i/image-png.svg"),
            ("text-x-generic", "/i/text-x-generic.png"),
            ("folder", "/i/folder.svg"),
        ]);
        assert_eq!(
            file_glyph(&png, &find),
            Glyph::Art(IconArt::Vector(PathBuf::from("/i/image-png.svg")))
        );
        assert_eq!(
            file_glyph(&notes, &find),
            Glyph::Art(IconArt::Raster(PathBuf::from("/i/text-x-generic.png"))),
            "text/plain has no text-plain icon here, so the generic one"
        );
        assert_eq!(
            file_glyph(dir.path(), &find),
            Glyph::Art(IconArt::Vector(PathBuf::from("/i/folder.svg"))),
            "a directory's generic icon is folder"
        );
        assert_eq!(file_glyph(&blob, &find), Glyph::builtin("blank-document"));
        assert_eq!(
            file_glyph(dir.path(), &theme(&[])),
            Glyph::builtin("folder")
        );
    }

    #[test]
    fn mime_icon_names_follow_the_shared_mime_info_defaults() {
        let names = names_for_mime("application/pdf");
        assert_eq!(names.icon, "application-pdf");
        assert_eq!(names.generic, "application-x-generic");
        assert_eq!(names_for_mime("inode/directory").generic, "folder");
    }

    #[test]
    fn file_glyphs_are_resolved_once_per_path() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };
        let mut cache = FileGlyphCache::default();
        cache.warm(["/nowhere/a.png", "/nowhere/a.png"], &find);
        cache.warm(["/nowhere/a.png"], &find);
        assert_eq!(*asked.borrow(), ["image-png", "image-x-generic"]);
        assert_eq!(
            cache.cached("/nowhere/a.png"),
            Some(&Glyph::builtin("blank-document"))
        );
        assert!(cache.cached("/nowhere/b.png").is_none());
    }

    fn lookup_with<'a>(
        find: &'a dyn Fn(&str) -> Option<PathBuf>,
        remote: &'a dyn Fn(&str) -> Option<PathBuf>,
    ) -> UrlLookup<'a> {
        UrlLookup {
            find,
            remote,
            favicon: compass_core::favicon::Service::Google,
            accent: crate::design::Rgb::new(0x35, 0x84, 0xe4),
        }
    }

    #[test]
    fn an_image_url_is_drawn_as_its_type_says() {
        use compass_core::image_url::{ColorLike, ImageUrl, ImageUrlType};
        let find = theme(&[("firefox", "/i/firefox.svg")]);
        let remote = |url: &str| {
            (url == "https://www.google.com/s2/favicons?domain=example.com&sz=128")
                .then(|| PathBuf::from("/cache/favicon.png"))
        };
        let lookup = lookup_with(&find, &remote);

        let Some(Glyph::Builtin {
            name, tile, fill, ..
        }) = url_glyph(
            &ImageUrl::builtin("link").with_background_tint(ColorLike::Semantic("Purple".into())),
            &lookup,
        )
        else {
            panic!("a builtin draws a builtin");
        };
        assert_eq!(name, "link");
        let tile = tile.expect("a tinted builtin sits on a tile");
        assert!(tile.b > tile.g, "purple: {tile:?}");
        assert_eq!(fill, Some(on_tile(tile)));

        assert_eq!(
            url_glyph(&ImageUrl::new(ImageUrlType::Emoji, "🎉"), &lookup),
            Some(Glyph::Text("🎉".into()))
        );
        assert_eq!(
            url_glyph(&ImageUrl::local("/opt/x/logo.png"), &lookup),
            Some(Glyph::Art(IconArt::Raster("/opt/x/logo.png".into())))
        );
        assert_eq!(
            url_glyph(&ImageUrl::new(ImageUrlType::System, "firefox"), &lookup),
            Some(Glyph::Art(IconArt::Vector("/i/firefox.svg".into())))
        );
        assert_eq!(
            url_glyph(
                &ImageUrl::new(ImageUrlType::Favicon, "example.com"),
                &lookup
            ),
            Some(Glyph::Art(IconArt::Raster("/cache/favicon.png".into()))),
            "a favicon is its service's image, once fetched"
        );
        assert_eq!(
            url_glyph(
                &ImageUrl::new(ImageUrlType::Favicon, "elsewhere.org")
                    .with_fallback(&ImageUrl::builtin("link")),
                &lookup
            ),
            Some(Glyph::builtin("link")),
            "until then, its fallback"
        );
        assert_eq!(
            url_glyph(&ImageUrl::http("https://a.example/b.png"), &lookup),
            None
        );
        assert_eq!(
            remote_source(
                &ImageUrl::new(ImageUrlType::Favicon, "example.com"),
                compass_core::favicon::Service::None
            ),
            None,
            "with favicons off nothing is fetched"
        );
    }

    #[test]
    fn a_tile_is_a_gradient_lighter_at_the_top_and_deeper_at_the_bottom() {
        use crate::design::Rgb;
        let lightness = |c: Rgb| {
            let hsl = compass_core::contrast::to_hsl(to_core(c));
            i32::from(hsl.lightness)
        };
        for tile in [
            Rgb::new(0x3a, 0x9c, 0x61),
            Rgb::new(0xb9, 0x54, 0x3b),
            Rgb::new(128, 132, 138),
        ] {
            let tile = tile_tone(tile);
            let (top, bottom) = tile_gradient(tile);
            // `+0.10` and `-0.05` of lightness, on Qt's 0..=255 scale.
            assert!(
                (lightness(top) - lightness(tile) - 26).abs() <= 2,
                "{top:?} over {tile:?}"
            );
            assert!(
                (lightness(tile) - lightness(bottom) - 13).abs() <= 2,
                "{bottom:?} under {tile:?}"
            );
        }
        let white = Rgb::new(255, 255, 255);
        assert_eq!(tile_gradient(white).0, white, "lightness is clamped");
    }

    fn opaque(side: u32) -> Vec<u8> {
        vec![255; (side * side * 4) as usize]
    }

    fn alpha_at(pixels: &[u8], width: u32, x: u32, y: u32) -> u8 {
        pixels[((y * width + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_circle_mask_clears_the_corners_and_keeps_the_middle() {
        use compass_core::image_url::ImageMask;
        let mut pixels = opaque(64);
        apply_mask(&mut pixels, 64, 64, ImageMask::Circle);
        assert_eq!(alpha_at(&pixels, 64, 0, 0), 0);
        assert_eq!(alpha_at(&pixels, 64, 63, 63), 0);
        assert_eq!(alpha_at(&pixels, 64, 32, 32), 255);
        assert!(alpha_at(&pixels, 64, 32, 0) >= 250, "the top edge's middle");
        // Where a circle has cut and a rounded rectangle has not.
        assert_eq!(alpha_at(&pixels, 64, 6, 6), 0);
        let edge = alpha_at(&pixels, 64, 9, 9);
        assert!(edge > 0 && edge < 255, "the edge is antialiased: {edge}");
    }

    #[test]
    fn a_rounded_mask_rounds_a_quarter_of_the_side() {
        use compass_core::image_url::ImageMask;
        let mut pixels = opaque(64);
        apply_mask(&mut pixels, 64, 64, ImageMask::RoundedRectangle);
        assert_eq!(alpha_at(&pixels, 64, 0, 0), 0, "a corner is cut");
        assert_eq!(alpha_at(&pixels, 64, 6, 6), 255, "a circle would cut here");
        assert_eq!(
            alpha_at(&pixels, 64, 16, 0),
            255,
            "the straight edge is kept"
        );
        let mut untouched = opaque(8);
        apply_mask(&mut untouched, 8, 8, ImageMask::None);
        assert_eq!(untouched, opaque(8));
    }

    #[test]
    fn a_masked_image_is_drawn_once_from_a_png_or_an_svg() {
        use compass_core::image_url::ImageMask;
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("avatar.png");
        image::RgbaImage::from_pixel(256, 128, image::Rgba([10, 20, 30, 255]))
            .save(&png)
            .unwrap();
        let (width, height, _) =
            rasterize(&IconArt::Raster(png.clone()), MASK_SIDE, None).expect("a png");
        assert_eq!(
            (width, height),
            (MASK_SIDE, MASK_SIDE / 2),
            "fitted, aspect kept"
        );
        let svg = dir.path().join("dot.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#,
        )
        .unwrap();
        let (_, _, pixels) =
            rasterize(&IconArt::Vector(svg.clone()), 16, Some([1, 2, 3])).expect("an svg");
        assert_eq!(&pixels[..4], &[1, 2, 3, 255], "tinted, straight alpha");

        let cache = MaskedCache::default();
        assert!(
            cache
                .get(&IconArt::Raster(png), ImageMask::Circle, None)
                .is_some()
        );
        assert!(
            cache
                .get(&IconArt::Vector(svg), ImageMask::RoundedRectangle, None)
                .is_some()
        );
        assert!(
            cache
                .get(
                    &IconArt::Raster(dir.path().join("gone.png")),
                    ImageMask::Circle,
                    None
                )
                .is_none(),
            "a file that cannot be drawn is none, and the row keeps its initial"
        );
    }

    #[test]
    fn a_command_is_drawn_on_its_tile_with_a_light_glyph() {
        use crate::design::Rgb;
        let accent = Rgb::new(0x35, 0x84, 0xe4);
        let clipboard = compass_core::commands::by_id("commands:clipboard-history").unwrap();
        let Glyph::Builtin {
            name,
            fill,
            tile,
            badge,
        } = command_glyph(clipboard, accent)
        else {
            panic!("a builtin command draws a builtin");
        };
        assert_eq!(name, "copy-clipboard");
        assert_eq!(badge, None);
        let tile = tile.expect("clipboard history has a red tile");
        assert!(tile.r > tile.g && tile.r > tile.b, "red: {tile:?}");
        let hsl = compass_core::contrast::to_hsl(to_core(tile));
        assert!((107..=133).contains(&hsl.lightness), "{hsl:?}");
        let fill = fill.expect("a tile carries a contrasting glyph");
        assert!(
            compass_core::contrast::contrast_ratio(to_core(tile), to_core(fill)) >= 4.5,
            "{fill:?} on {tile:?}"
        );
        let create = compass_core::commands::by_id("commands:create-snippet").unwrap();
        assert!(matches!(
            command_glyph(create, accent),
            Glyph::Builtin {
                badge: Some("plus"),
                ..
            }
        ));
    }

    #[test]
    fn a_grey_tile_stays_grey_and_accent_follows_the_palette() {
        use crate::design::Rgb;
        let grey = tile_color(compass_core::commands::Tile::Gray, Rgb::new(0, 0, 0));
        assert!(grey.r.abs_diff(grey.b) < 16, "{grey:?}");
        let accent = Rgb::new(0x20, 0xa0, 0x20);
        let tile = tile_color(compass_core::commands::Tile::Accent, accent);
        assert!(tile.g > tile.r && tile.g > tile.b, "{tile:?}");
    }

    #[test]
    fn clipboard_rows_and_the_default_mark_use_the_cpps_builtins() {
        use crate::backend::ClipboardRowKind;
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Image),
            Glyph::builtin("image")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Link),
            Glyph::builtin("link")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Text),
            Glyph::builtin("text")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::File),
            Glyph::builtin("folder")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Unknown),
            Glyph::builtin("question-mark-circle")
        );
        let Glyph::Builtin { name, fill, .. } = default_mark() else {
            panic!("the mark is a builtin");
        };
        assert_eq!(name, "check-circle");
        let fill = fill.unwrap();
        assert!(fill.g > fill.r && fill.g > fill.b, "green: {fill:?}");
    }
}
