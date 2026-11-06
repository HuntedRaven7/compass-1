//! The icons beside the launcher's rows: an application's theme icon, a
//! builtin's tinted SVG, an extension's or a shortcut's `ImageURL`, a
//! window's application, and an initial when there is nothing else.
//!
//! Drawing never waits for an icon. A remote or theme icon is asked for
//! ahead of the rows that show it (`warm_icons`), and until it arrives the
//! row draws its fallback.

use super::{
    Alignment, AppItem, Arc, Border, Color, Element, GEOMETRY, LauncherApp, Length, Message,
    Padding, Page, RootRow, Theme, container, image, svg, text,
};

/// The nominal pixel size asked of the icon theme.
///
/// [`design::GEOMETRY`](crate::design::GEOMETRY)'s `icon_size`, which is the square the row reserves.
/// Asking the theme for that size rather than a fixed 32 keeps the two from
/// drifting: a design change that grows the slot asks for a larger icon
/// instead of scaling a small one up.
const ICON_PIXELS: u32 = GEOMETRY.icon_size as u32;

type IconFinder = dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync;

/// Resolves an `Icon=` name to a file. See
/// [`AppFlags::icon_lookup`](super::AppFlags::icon_lookup).
///
/// A named type rather than a bare `Arc<dyn Fn…>` so it can carry a `Debug`
/// impl: [`AppFlags`](super::AppFlags) derives `Debug`, and a closure does not.
#[derive(Clone)]
pub struct IconLookup(Arc<IconFinder>);

impl IconLookup {
    /// Wraps a lookup.
    #[must_use]
    pub fn new(find: impl Fn(&str) -> Option<std::path::PathBuf> + Send + Sync + 'static) -> Self {
        Self(Arc::new(find))
    }

    /// The file for `name`, if the lookup can find one.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<std::path::PathBuf> {
        (self.0)(name)
    }
}

impl std::fmt::Debug for IconLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IconLookup(..)")
    }
}

impl Default for IconLookup {
    /// The XDG icon themes, at the size the row reserves.
    fn default() -> Self {
        Self::new(|name: &str| {
            compass_xdg::find_icon(
                name,
                Some(&compass_xdg::default_theme()),
                Some(ICON_PIXELS),
                1.0,
            )
        })
    }
}

impl LauncherApp {
    /// An application's icon slot: its themed icon when icons are on and it
    /// resolved, its initial otherwise. See [`Self::result_row`].
    pub(super) fn app_icon(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let geometry = self.geometry;
        match self.row_art(item) {
            Some(crate::icons::IconArt::Raster(path)) => {
                container(image(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            Some(crate::icons::IconArt::Vector(path)) => {
                container(svg(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            None => self.initial_badge(item.name(), selected),
        }
    }

    /// An extension row's icon: its art, a builtin tinted to read on the
    /// theme, or a grid cell's colour.
    pub(super) fn extension_icon(
        &self,
        icon: &crate::extension_page::RowIcon,
        selected: bool,
    ) -> Element<'_, Message> {
        self.masked_icon(icon, compass_core::image_url::ImageMask::None, selected)
    }

    /// [`Self::extension_icon`], clipped to `mask` (`Image.mask`): the
    /// image drawn into pixels and clipped as `applyCircleMask` and
    /// `applyRoundedRectMask` clip it.
    pub(super) fn masked_icon(
        &self,
        icon: &crate::extension_page::RowIcon,
        mask: compass_core::image_url::ImageMask,
        selected: bool,
    ) -> Element<'_, Message> {
        self.icon_art(icon, mask, selected, f32::from(self.geometry.icon_size))
    }

    /// [`Self::masked_icon`] in a square of `side`.
    pub(super) fn icon_art(
        &self,
        icon: &crate::extension_page::RowIcon,
        mask: compass_core::image_url::ImageMask,
        selected: bool,
        side: f32,
    ) -> Element<'_, Message> {
        use crate::extension_page::RowIcon;
        let size = Length::Fixed(side);
        let palette = self.palette();
        let text_color = if selected {
            palette.selection_text
        } else {
            palette.text
        }
        .to_iced();
        let masked = match icon {
            RowIcon::Art {
                art,
                monochrome,
                tint,
            } if mask != compass_core::image_url::ImageMask::None => {
                let color = tint.or(monochrome.then_some(text_color)).map(|c| {
                    let [r, g, b, _] = c.into_rgba8();
                    [r, g, b]
                });
                self.view.masked.get(art, mask, color)
            }
            _ => None,
        };
        if let Some(handle) = masked {
            return container(image(handle).width(Length::Fill).height(Length::Fill))
                .width(size)
                .height(size)
                .into();
        }
        let art: Element<Message> = match icon {
            RowIcon::Text(glyph) => container(text(glyph.clone()).size(side * 0.75))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .into(),
            RowIcon::Swatch(color) => {
                let color = *color;
                container(text(""))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    })
                    .into()
            }
            RowIcon::Art {
                art: crate::icons::IconArt::Raster(path),
                ..
            } => image(path).width(Length::Fill).height(Length::Fill).into(),
            RowIcon::Art {
                art: crate::icons::IconArt::Vector(path),
                monochrome,
                tint,
            } => {
                let color = tint.or(monochrome.then_some(text_color));
                svg(path)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_: &Theme, _| iced::widget::svg::Style { color })
                    .into()
            }
        };
        container(art).width(size).height(size).into()
    }

    /// A builtin command's icon slot: its icon on its C++ tile when icons are
    /// on and the builtin set is installed, its initial otherwise.
    pub(super) fn command_icon(
        &self,
        command: &compass_core::commands::BuiltinCommand,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = self
            .icons
            .then(|| crate::icons::command_glyph(command, self.palette().accent));
        self.glyph_or_initial(glyph.as_ref(), command.title, selected)
    }

    /// A window's icon slot: its application's icon, else `AppWindow`
    /// (`SwitchWindowsSection::displayIcon`).
    pub(super) fn window_icon(
        &self,
        window: &crate::backend::WindowRow,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = self.icons.then(|| {
            self.window_app(window)
                .and_then(|item| self.row_art(item))
                .cloned()
                .map_or_else(
                    || crate::icons::Glyph::builtin("app-window"),
                    crate::icons::Glyph::Art,
                )
        });
        self.glyph_or_initial(glyph.as_ref(), &window.app, selected)
    }

    /// The application a window belongs to, when the engine recognised it.
    fn window_app(&self, window: &crate::backend::WindowRow) -> Option<&AppItem> {
        if !window.app_known {
            return None;
        }
        self.app_index
            .items()
            .iter()
            .find(|item| item.name() == window.app)
    }

    /// `glyph` drawn in the row's icon slot, or `title`'s initial when there
    /// is none or it cannot be drawn (a builtin with no installed icon set).
    pub(super) fn glyph_or_initial(
        &self,
        glyph: Option<&crate::icons::Glyph>,
        title: &str,
        selected: bool,
    ) -> Element<'_, Message> {
        glyph
            .and_then(|glyph| self.glyph(glyph, selected, f32::from(self.geometry.icon_size)))
            .unwrap_or_else(|| self.initial_badge(title, selected))
    }

    /// The builtin icon `name` as an SVG of `size`, drawn in `color`: the
    /// icon theme's symbolic stand-in when it has one
    /// ([`compass_core::builtin_icon::symbolic_name`]), else the builtin file.
    pub(super) fn builtin_svg(
        &self,
        name: &str,
        color: Color,
        size: f32,
    ) -> Option<Element<'static, Message>> {
        let file = match self.symbolic_icons.get(name) {
            Some(themed) => themed.clone(),
            None => self
                .builtin_icons
                .as_ref()?
                .join(compass_core::builtin_icon::file_name(name)?),
        };
        Some(
            svg(file)
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .style(move |_: &Theme, _| iced::widget::svg::Style { color: Some(color) })
                .into(),
        )
    }

    /// A [`crate::icons::Glyph`] drawn in a square of `size`.
    ///
    /// A tile is `applyBackdrop`'s rounded square (a quarter of the side),
    /// with the glyph inset by 19% of it as `backdropContentSize`; a badge is
    /// `applyBadge`'s black disc, 44% of the side, 4% in from the corner,
    /// carrying the badge glyph in white.
    pub(super) fn glyph(
        &self,
        glyph: &crate::icons::Glyph,
        selected: bool,
        size: f32,
    ) -> Option<Element<'static, Message>> {
        use crate::icons::Glyph;
        let square = Length::Fixed(size);
        match glyph {
            Glyph::Text(glyph) => Some(
                container(text(glyph.clone()).size(size * 0.75))
                    .width(square)
                    .height(square)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
            ),
            Glyph::Art(art) => {
                let drawn: Element<'static, Message> = match art {
                    crate::icons::IconArt::Raster(path) => image(path.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into(),
                    crate::icons::IconArt::Vector(path) => svg(path.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into(),
                };
                Some(container(drawn).width(square).height(square).into())
            }
            Glyph::Builtin {
                name,
                fill,
                tile,
                badge,
            } => {
                let palette = self.palette();
                let text_color = if selected {
                    palette.selection_text
                } else {
                    palette.text
                };
                let base: Element<'static, Message> = match tile {
                    None => container(self.builtin_svg(
                        name,
                        fill.unwrap_or(text_color).to_iced(),
                        size * 0.8,
                    )?)
                    .width(square)
                    .height(square)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
                    Some(tile) => {
                        let (top, bottom) = crate::icons::tile_gradient(*tile);
                        let gradient = iced::gradient::Linear::new(std::f32::consts::PI)
                            .add_stop(0.0, top.to_iced())
                            .add_stop(1.0, bottom.to_iced());
                        let inner = size * (1.0 - 2.0 * 0.19);
                        let centred = |element: Element<'static, Message>, drop: f32| {
                            container(element)
                                .width(square)
                                .height(square)
                                .align_x(Alignment::Center)
                                .align_y(Alignment::Center)
                                .padding(Padding::ZERO.top(drop * 2.0))
                        };
                        // `applyBackdrop`'s shadow: the glyph's silhouette in
                        // translucent black, a little lower, under it.
                        let shadow = self.builtin_svg(
                            name,
                            Color::from_rgba8(
                                0,
                                0,
                                0,
                                f32::from(crate::icons::TILE_SHADOW_ALPHA) / 255.0,
                            ),
                            inner,
                        )?;
                        let glyph = self.builtin_svg(
                            name,
                            fill.unwrap_or(crate::design::Rgb::new(255, 255, 255))
                                .to_iced(),
                            inner,
                        )?;
                        container(iced::widget::stack![
                            centred(shadow, size * crate::icons::TILE_SHADOW_OFFSET),
                            centred(glyph, 0.0),
                        ])
                        .width(square)
                        .height(square)
                        .style(move |_: &Theme| container::Style {
                            background: Some(iced::Background::Gradient(gradient.into())),
                            border: Border {
                                color: Color::from_rgba8(255, 255, 255, 30.0 / 255.0),
                                width: (size / 32.0).max(1.0),
                                radius: (size * 0.25).into(),
                            },
                            ..container::Style::default()
                        })
                        .into()
                    }
                };
                let Some(badge) = badge else {
                    return Some(base);
                };
                let diameter = size * 0.44;
                let disc = container(self.builtin_svg(badge, Color::WHITE, diameter * 0.56)?)
                    .width(Length::Fixed(diameter))
                    .height(Length::Fixed(diameter))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Color::BLACK.into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: (diameter / 2.0).into(),
                        },
                        ..container::Style::default()
                    });
                let corner = container(disc)
                    .width(square)
                    .height(square)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(size * 0.04);
                Some(iced::widget::stack![base, corner].into())
            }
        }
    }

    /// The first letter of `title` in a tinted square, for a row with no art.
    pub(super) fn initial_badge(&self, title: &str, selected: bool) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let title_color = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let initial = title
            .chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().to_string());
        container(
            text(initial)
                .font(self.font())
                .size(f32::from(geometry.icon_size) / 2.0)
                .color(title_color.to_iced()),
        )
        .width(Length::Fixed(f32::from(geometry.icon_size)))
        .height(Length::Fixed(f32::from(geometry.icon_size)))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_: &Theme| container::Style {
            background: Some(
                Color {
                    a: 0.18,
                    ..palette.accent.to_iced()
                }
                .into(),
            ),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    /// What goes in a row's icon slot: resolved art, or nothing for the initial.
    ///
    /// The three ways to get nothing -- icons off, an entry with no `Icon=`,
    /// and a name the theme could not resolve -- are deliberately one answer.
    /// They all mean the same thing to the row, and collapsing them here is
    /// what keeps `view` from having to know the difference.
    pub(super) fn row_art(&self, item: &AppItem) -> Option<&crate::icons::IconArt> {
        if !self.icons {
            return None;
        }
        item.icon().and_then(|name| self.icon_cache.cached(name))
    }

    /// A root row's `ImageURL` drawn in its icon slot, once warmed, else
    /// `title`'s initial.
    pub(super) fn url_icon(
        &self,
        url: Option<&compass_core::image_url::ImageUrl>,
        title: &str,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = url
            .filter(|_| self.icons)
            .and_then(|url| self.url_glyphs.get(&url.to_url())?.as_ref());
        self.glyph_or_initial(glyph, title, selected)
    }

    /// An extension command's icon URL, once warmed.
    pub(super) fn extension_url(
        &self,
        command: &compass_core::extension_commands::ExtensionCommand,
    ) -> Option<compass_core::image_url::ImageUrl> {
        self.icons
            .then(|| command.icon_url(|path| self.view.known_files.contains(path)))
    }

    /// Resolves `urls` into [`Self::url_glyphs`], asking for each remote
    /// image not yet fetched (when
    /// [`AppFlags::remote_icons`](super::AppFlags::remote_icons) allows).
    fn warm_urls(&mut self, urls: Vec<compass_core::image_url::ImageUrl>) {
        let find = self.icon_lookup.clone();
        let find = |name: &str| find.find(name);
        for url in urls {
            let key = url.to_url();
            if self.url_glyphs.contains_key(&key) {
                continue;
            }
            let remote_files = &self.remote_files;
            let remote = |remote: &str| remote_files.get(remote).cloned();
            let lookup = crate::icons::UrlLookup {
                find: &find,
                remote: &remote,
                favicon: self.favicon_service,
                accent: self.palette().accent,
            };
            let glyph = crate::icons::url_glyph(&url, &lookup);
            let waiting = crate::icons::remote_source(&url, self.favicon_service)
                .filter(|remote| !self.remote_files.contains_key(remote));
            if let Some(remote) = waiting {
                if self.remote_icons && self.remote_requested.insert(remote.clone()) {
                    self.remote_pending.push(remote);
                }
                if glyph.is_none() {
                    // Resolved again when it arrives.
                    continue;
                }
            }
            self.url_glyphs.insert(key, glyph);
        }
    }

    /// A remote root icon arrived in the cache, or could not be fetched.
    pub(super) fn root_icon_arrived(
        &mut self,
        url: &str,
        result: &Result<std::path::PathBuf, String>,
    ) {
        match result {
            Ok(path) => {
                self.remote_files.insert(url.to_owned(), path.clone());
                // Whatever waited on it (and its fallbacks) is resolved again.
                self.url_glyphs.clear();
                self.warm_icons();
                self.warm_clipboard_icons();
            }
            Err(reason) => tracing::debug!(%url, %reason, "a root icon was not fetched"),
        }
    }

    /// Resolve the favicons of clipboard history's link rows.
    pub(super) fn warm_clipboard_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Clipboard(page) = &self.page else {
            return;
        };
        let urls: Vec<_> = page.rows.iter().filter_map(clipboard_url).collect();
        self.warm_urls(urls);
    }

    /// Resolve the icons of the rows now on screen.
    ///
    /// Here rather than in `view` because resolution walks the theme
    /// directories, and a launcher re-ranks on every keystroke: doing disk work
    /// per draw is the cost #85 warns about. Bounded to the current results, so
    /// it is the size of the visible list rather than of the index, and the
    /// cache makes each name at most one walk for the life of the process.
    ///
    /// A no-op when icons are off, so the default configuration does no lookup
    /// at all.
    pub(super) fn warm_icons(&mut self) {
        if !self.icons {
            return;
        }

        let names: Vec<&str> = self
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(index) => self.app_index.items().get(*index),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .filter_map(AppItem::icon)
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache.warm(names, &|name| find.find(name));

        // Extension, script and shortcut rows: their `ImageURL`s.
        let mut urls = Vec::new();
        for row in &self.results {
            match row {
                RootRow::Extension(index) => {
                    if let Some(command) = self.app_index.extensions().get(*index) {
                        for icon in [
                            command.icon.as_deref(),
                            Some(command.extension_icon.as_str()),
                        ]
                        .into_iter()
                        .flatten()
                        .filter(|icon| !icon.is_empty())
                        {
                            let path = command.extension_dir.join("assets").join(icon);
                            if path.is_file() {
                                self.view.known_files.insert(path);
                            }
                        }
                        urls.push(command.icon_url(|path| self.view.known_files.contains(path)));
                    }
                }
                RootRow::Script(index) => {
                    if let Some(url) = self
                        .app_index
                        .scripts()
                        .get(*index)
                        .and_then(|script| self.script_icons.get(&script.id))
                    {
                        urls.push(url.clone());
                    }
                }
                RootRow::Shortcut(index) => {
                    if let Some(shortcut) = self.app_index.shortcuts().get(*index) {
                        urls.push(shortcut_url(&shortcut.icon));
                    }
                }
                _ => {}
            }
        }
        self.warm_urls(urls);
    }

    /// Resolve the file-type icons of Search Files' rows.
    pub(super) fn warm_file_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Files(page) = &self.page else {
            return;
        };
        let find = self.icon_lookup.clone();
        self.file_glyphs
            .warm(page.rows.iter().map(|file| file.path.as_str()), &|name| {
                find.find(name)
            });
    }

    /// Resolve the icons of the windows' applications.
    pub(super) fn warm_window_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Windows(page) = &self.page else {
            return;
        };
        let names: Vec<String> = page
            .all
            .iter()
            .filter_map(|window| self.window_app(window)?.icon().map(str::to_owned))
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache
            .warm(names.iter().map(String::as_str), &|name| find.find(name));
    }
}

/// A shortcut's stored icon as its root row draws it (`RootShortcutItem::iconUrl`):
/// the `ImageURL` it names, a builtin on a purple tile.
pub(super) fn shortcut_url(icon: &str) -> compass_core::image_url::ImageUrl {
    let url = compass_core::image_url::ImageUrl::parse(icon);
    if url.is_builtin() {
        url.with_background_tint(compass_core::image_url::ColorLike::Semantic(
            "Purple".into(),
        ))
    } else {
        url
    }
}

/// A clipboard link row's favicon, with the link builtin as its fallback
/// (`ClipboardHistoryModel`'s icon for a link).
pub(super) fn clipboard_url(
    entry: &crate::backend::ClipboardRow,
) -> Option<compass_core::image_url::ImageUrl> {
    use compass_core::image_url::{ImageUrl, ImageUrlType};
    let host = entry.url_host.as_deref().filter(|host| !host.is_empty())?;
    (entry.kind == crate::backend::ClipboardRowKind::Link).then(|| {
        ImageUrl::new(ImageUrlType::Favicon, host).with_fallback(&ImageUrl::builtin("link"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::icons::icon_tests::{app_with_icons, asked, recording};
    use crate::app::test_support::{
        FakeWindows, TestBackend, index, open_builtin, open_windows, script_item, window_row,
        windows_app,
    };
    use crate::icons::IconArt;
    use compass_core::AppIndex;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn repo_builtin_icons() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extra/builtin-icons")
            .canonicalize()
            .expect("the builtin icon set is in the repository")
    }

    #[test]
    fn a_builtin_command_draws_its_tiled_icon_and_without_the_set_its_initial() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.query = "clipboard history".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Command(_))));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(
                ui.find("C").is_ok(),
                "with no builtin icon set installed the row keeps its initial"
            );
        }
        app.builtin_icons = Some(repo_builtin_icons());
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Clipboard History").is_ok());
        assert!(
            ui.find("C").is_err(),
            "the builtin icon replaces the initial"
        );
    }

    #[test]
    fn search_files_rows_draw_their_file_type_icons() {
        let dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let photo = files.path().join("photo.png");
        fs::write(&photo, b"x").unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        let (log, lookup) = recording(&["image-png"]);
        app.icon_lookup = lookup;
        let page = crate::files_page::FilesPage::default();
        let generation = page.generation;
        app.page = Page::Files(page);
        let row = |path: &Path, category: &str| crate::backend::FileRow {
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            category: category.into(),
        };
        let _ = app.update(Message::FilesLoaded {
            generation,
            result: Ok(crate::backend::FileResults {
                heading: "Files".into(),
                files: vec![row(&photo, "Images"), row(files.path(), "Folders")],
            }),
        });
        assert_eq!(
            app.file_glyphs.cached(&photo.to_string_lossy()),
            Some(&crate::icons::Glyph::Art(IconArt::Vector(PathBuf::from(
                "/i/image-png.svg"
            ))))
        );
        assert_eq!(
            app.file_glyphs.cached(&files.path().to_string_lossy()),
            Some(&crate::icons::Glyph::builtin("folder")),
            "a folder no theme has an icon for draws the builtin folder"
        );
        assert!(asked(&log).contains(&"inode-directory".to_owned()));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("photo.png").is_ok());
        assert!(ui.find("P").is_err(), "no initial on a file row");
    }

    #[test]
    fn a_window_row_draws_its_applications_icon_or_the_app_window_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Firefox", 1),
                crate::backend::WindowRow {
                    app_known: false,
                    ..window_row(9, "xterm", "XTerm", 2)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        let fresh = app_with_icons(dir.path());
        app.app_index = fresh.app_index;
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        open_windows(&mut app);
        assert!(asked(&log).contains(&"firefox".to_owned()));
        let Page::Windows(page) = &app.page else {
            panic!("not the window switcher: {}", app.state_line());
        };
        let firefox = page.all.iter().find(|w| w.id == 7).unwrap();
        let xterm = page.all.iter().find(|w| w.id == 9).unwrap();
        assert_eq!(
            app.window_app(firefox).and_then(|item| app.row_art(item)),
            Some(&IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert!(app.window_app(xterm).is_none());
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("X").is_err(), "the unknown window draws AppWindow");
        assert!(ui.find("F").is_err());
    }

    #[test]
    fn the_default_pickers_mark_is_the_green_check_icon() {
        use crate::backend::DefaultAppRow;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            default_apps: vec![DefaultAppRow {
                id: "firefox.desktop".into(),
                name: "Firefox".into(),
                description: "Browse the web".into(),
                is_default: true,
            }],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        app.builtin_icons = Some(repo_builtin_icons());
        open_builtin(
            &mut app,
            "set default browser",
            "commands:set-default-browser",
        );
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Firefox").is_ok());
        assert!(
            ui.find("✓ Default").is_err(),
            "the check icon replaces the text mark"
        );
    }

    #[test]
    fn clipboard_rows_draw_the_builtin_for_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.builtin_icons = Some(repo_builtin_icons());
        let page = crate::clipboard_page::ClipboardPage {
            rows: vec![crate::backend::ClipboardRow {
                id: "1".into(),
                preview: "hello".into(),
                kind: crate::backend::ClipboardRowKind::Text,
                pinned: false,
                url_host: None,
            }],
            status: crate::clipboard_page::Status::Ready,
            ..crate::clipboard_page::ClipboardPage::default()
        };
        app.page = Page::Clipboard(page);
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("hello").is_ok());
        assert!(ui.find("T").is_err(), "the text builtin, not an initial");
    }

    #[test]
    fn extension_script_and_shortcut_rows_draw_their_icons_in_root_search() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let ext = dir.path().join("extensions/hello");
        fs::create_dir_all(ext.join("assets")).unwrap();
        fs::write(ext.join("assets/hello.png"), b"png").unwrap();
        fs::write(
            ext.join("package.json"),
            r#"{"name": "hello", "title": "Hello", "author": "someone", "icon": "hello.png",
                "commands": [{"name": "write", "title": "Write Greeting", "mode": "no-view"}]}"#,
        )
        .unwrap();
        index(dir.path());
        let mut app = LauncherApp::with_index(
            AppIndex::builder()
                .dir(dir.path())
                .extension_dirs([dir.path().join("extensions")])
                .build(),
        );
        app.backend = Some(backend);
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        app.query = "greeting".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Extension(0)));
        let extension_icon = ext.join("assets/hello.png");
        let key =
            compass_core::image_url::ImageUrl::local(extension_icon.to_string_lossy()).to_url();
        assert_eq!(
            app.url_glyphs.get(&key),
            Some(&Some(crate::icons::Glyph::Art(IconArt::Raster(
                extension_icon
            )))),
            "the extension's own icon, from its assets"
        );
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Write Greeting").is_ok());
            assert!(ui.find("W").is_err(), "the icon replaces the initial");
        }

        // A script whose header names an emoji.
        app.app_index.set_scripts(vec![script_item(
            "party.sh",
            "Party Time",
            compass_core::script_command::OutputMode::Silent,
            0,
        )]);
        let _ = app.update(Message::ScriptIconsLoaded(Ok(vec![(
            "party.sh".into(),
            "icon://emoji/🎉".into(),
        )])));
        app.query = "party time".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Script(_))));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("🎉").is_ok(), "the emoji is drawn as text");
            assert!(ui.find("P").is_err());
        }

        // A shortcut with a builtin icon: on a purple tile.
        app.app_index
            .set_shortcuts(vec![compass_core::shortcut_service::CachedShortcut {
                id: "s1".into(),
                name: "Docs Search".into(),
                icon: "icon://omnicast/link".into(),
                link: compass_core::shortcut::parse_link("https://docs.rs/{query}"),
                app: "default".into(),
                open_count: 0,
                created_at: 0,
                updated_at: 0,
                last_opened_at: None,
            }]);
        app.query = "docs search".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Shortcut(_))));
        let key = "icon://omnicast/link?bg_tint=purple";
        let Some(Some(crate::icons::Glyph::Builtin { name, tile, .. })) = app.url_glyphs.get(key)
        else {
            panic!("no glyph for {key}: {:?}", app.url_glyphs.keys());
        };
        assert_eq!(name, "link");
        assert!(tile.is_some(), "a shortcut's builtin sits on a purple tile");
    }

    #[test]
    fn a_favicon_is_fetched_once_into_the_cache_and_then_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        app.favicon_service = compass_core::favicon::Service::Twenty;
        let row = crate::backend::ClipboardRow {
            id: "1".into(),
            preview: "https://example.com/page".into(),
            kind: crate::backend::ClipboardRowKind::Link,
            pinned: false,
            url_host: Some("example.com".into()),
        };
        let page = crate::clipboard_page::ClipboardPage::default();
        let generation = page.generation;
        app.page = Page::Clipboard(page);
        let load = |app: &mut LauncherApp| {
            let _ = app.update(Message::ClipboardLoaded {
                generation,
                result: Ok(vec![row.clone()]),
            });
        };
        let favicon = clipboard_url(&row).expect("a link has a favicon").to_url();
        let favicon = favicon.as_str();
        assert!(favicon.starts_with("icon://favicon/example.com?fallback="));

        // Without remote icons (every test's default) nothing is fetched and
        // the link builtin stands in.
        load(&mut app);
        assert!(app.remote_requested.is_empty());
        assert_eq!(
            app.url_glyphs.get(favicon),
            Some(&Some(crate::icons::Glyph::builtin("link")))
        );

        app.remote_icons = true;
        app.url_glyphs.clear();
        load(&mut app);
        let wanted = "https://twenty-icons.com/example.com/128";
        assert_eq!(
            app.remote_requested.iter().collect::<Vec<_>>(),
            [wanted],
            "the favicon is asked of the configured service"
        );
        load(&mut app);
        assert_eq!(app.remote_requested.len(), 1, "and asked for once");

        let cached = dir.path().join("favicon.png");
        fs::write(&cached, b"png").unwrap();
        let _ = app.update(Message::ExtensionImageFetched {
            url: wanted.into(),
            result: Ok(cached.clone()),
        });
        assert_eq!(
            app.url_glyphs.get(favicon),
            Some(&Some(crate::icons::Glyph::Art(IconArt::Raster(cached)))),
            "once in the cache, the favicon is drawn"
        );
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use crate::icons::IconArt;
    use compass_core::AppIndex;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// Three entries, two of which name an icon.
    pub(super) fn app_with_icons(dir: &std::path::Path) -> LauncherApp {
        for (file, name, icon) in [
            ("firefox.desktop", "Firefox", Some("firefox")),
            ("files.desktop", "Files", Some("system-file-manager")),
            ("terminal.desktop", "Terminal", None),
        ] {
            let icon_line = icon.map_or_else(String::new, |value| format!("Icon={value}\n"));
            fs::write(
                dir.join(file),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{icon_line}"
                ),
            )
            .expect("write entry");
        }
        LauncherApp::with_index(AppIndex::builder().dir(dir).build())
    }

    /// A lookup that answers for `hits` and records every name it is asked.
    pub(super) fn recording(hits: &[&str]) -> (Arc<Mutex<Vec<String>>>, IconLookup) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let hits: Vec<String> = hits.iter().map(|hit| (*hit).to_owned()).collect();
        let lookup = IconLookup::new(move |name: &str| {
            log.lock().expect("lock").push(name.to_owned());
            hits.iter()
                .any(|hit| hit == name)
                .then(|| PathBuf::from(format!("/i/{name}.svg")))
        });
        (asked, lookup)
    }

    pub(super) fn asked(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().expect("lock").clone()
    }

    fn row<'a>(app: &'a LauncherApp, name: &str) -> &'a AppItem {
        app.results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(index) => app.app_index.items().get(*index),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .find(|item| item.name() == name)
            .unwrap_or_else(|| panic!("{name} is not a row"))
    }

    #[test]
    fn icons_off_looks_nothing_up_at_all() {
        // Explicitly disabling icons must cost nothing: not a directory walk,
        // not even a cached miss.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = false;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("fi".to_owned()));

        assert!(!app.results.is_empty(), "precondition: there are rows");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
        assert!(app.icon_cache.is_empty());
    }

    #[test]
    fn icons_on_warms_only_the_rows_on_screen() {
        // Bounded by the visible list rather than by the index: that is the
        // whole reason resolution happens in `update` and not in `view`.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert_eq!(asked(&log), ["firefox"], "{:?}", asked(&log));
    }

    #[test]
    fn an_entry_with_no_icon_key_is_never_looked_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&[]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));

        assert!(!app.results.is_empty(), "precondition: Terminal is a row");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
    }

    #[test]
    fn the_same_name_is_resolved_once_however_many_keystrokes() {
        // A launcher re-ranks on every keystroke. Without the cache this is
        // one theme walk per character typed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        for query in ["f", "fi", "fir", "fire", "firef"] {
            let _ = app.update(Message::QueryChanged(query.to_owned()));
        }

        let names = asked(&log);
        assert_eq!(
            names.iter().filter(|name| *name == "firefox").count(),
            1,
            "{names:?}"
        );
    }

    #[test]
    fn a_resolved_row_draws_its_icon_and_an_unresolved_one_falls_back() {
        // The ordinary state of a mixed desktop: the theme has one of them.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("f".to_owned()));

        assert_eq!(
            app.row_art(row(&app, "Firefox")),
            Some(&IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert_eq!(
            app.row_art(row(&app, "Files")),
            None,
            "an icon the theme does not have must fall back to the initial"
        );
    }

    #[test]
    fn a_resolved_icon_is_still_not_drawn_once_icons_are_turned_off() {
        // The cache outlives the toggle, so the gate has to be at the draw and
        // not only at the warm.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert!(
            app.row_art(row(&app, "Firefox")).is_some(),
            "precondition: resolved"
        );

        app.icons = false;
        assert_eq!(app.row_art(row(&app, "Firefox")), None);
    }

    #[test]
    fn native_app_icons_are_on_by_default() {
        assert!(
            compass_core::config::LauncherConfig::default()
                .appearance()
                .icons()
        );
        assert!(AppFlags::default().icons);
    }
}
