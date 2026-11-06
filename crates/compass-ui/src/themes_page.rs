//! Set Theme: the themes in two sections, and the live preview.
//!
//! `compass_core::theme_picker` decides the sections and the filter, as
//! `ThemeListModel` does; the themes are Compass's curated ones followed by
//! the theme files found in the theme directories ([`crate::theme::Theme`]).

use compass_core::theme_picker::{self, AVAILABLE_SECTION, CURRENT_SECTION};

use crate::theme::Theme;

/// One row: the theme, and whether it starts its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeRow {
    /// The theme.
    pub theme: Theme,
    /// The heading above it, when it is the first of its section.
    pub heading: Option<&'static str>,
}

impl ThemeRow {
    /// The subtitle the row shows: the theme's description, or the stock
    /// one when it has none.
    #[must_use]
    pub fn subtitle(&self) -> &'static str {
        let description = self.theme.description();
        if description.is_empty() {
            theme_picker::DEFAULT_DESCRIPTION
        } else {
            description
        }
    }
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct ThemesPage {
    /// The search text.
    pub query: String,
    /// The theme in use when the view opened: its "current" row, and what
    /// leaving without choosing goes back to.
    pub configured: Theme,
    /// The rows: the current theme, then the others.
    pub rows: Vec<ThemeRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// Why saving did not happen.
    pub notice: Option<String>,
    /// Every theme offered: the curated ones, then the files.
    pub themes: Vec<Theme>,
}

fn picker_theme(theme: Theme) -> theme_picker::Theme {
    theme_picker::Theme {
        id: theme.name().to_owned(),
        name: theme.title().to_owned(),
        description: theme.description().to_owned(),
        icon: None,
        path: theme.path().map(str::to_owned),
    }
}

impl ThemesPage {
    /// The view over every theme, `configured` first; `files` are the
    /// themes read from the theme directories.
    #[must_use]
    pub fn new(configured: Theme, files: Vec<Theme>) -> Self {
        let mut themes = Theme::ALL.to_vec();
        themes.extend(files);
        let mut page = Self {
            query: String::new(),
            configured,
            rows: Vec::new(),
            selected: 0,
            notice: None,
            themes,
        };
        page.refilter();
        page
    }

    /// Recomputes the rows for the current text, back at the top.
    pub fn refilter(&mut self) {
        let themes: Vec<theme_picker::Theme> =
            self.themes.iter().copied().map(picker_theme).collect();
        let list = theme_picker::split(&themes, &self.query, self.configured.name());
        let offered = &self.themes;
        let rows = |section: &[theme_picker::Theme], heading: &'static str| {
            section
                .iter()
                .enumerate()
                .filter_map(|(position, theme)| {
                    offered
                        .iter()
                        .find(|offered| offered.name() == theme.id)
                        .map(|&theme| ThemeRow {
                            theme,
                            heading: (position == 0).then_some(heading),
                        })
                })
                .collect::<Vec<_>>()
        };
        self.rows = rows(&list.current, CURRENT_SECTION);
        self.rows.extend(rows(&list.available, AVAILABLE_SECTION));
        self.selected = 0;
    }

    /// The selected theme, if any.
    #[must_use]
    pub fn selected_theme(&self) -> Option<Theme> {
        self.rows.get(self.selected).map(|row| row.theme)
    }

    /// What the view says in place of the list when no theme matches.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&'static str> {
        self.rows.is_empty().then_some("No themes match")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configured_theme_is_its_own_section_and_the_filter_is_fuzzy() {
        let mut page = ThemesPage::new(Theme::Nord, Vec::new());
        assert_eq!(page.rows[0].theme, Theme::Nord);
        assert_eq!(page.rows[0].heading, Some(CURRENT_SECTION));
        assert_eq!(page.rows[1].heading, Some(AVAILABLE_SECTION));
        assert_eq!(page.rows.len(), Theme::ALL.len());
        page.query = "tokyo nigt".into();
        page.refilter();
        assert_eq!(page.selected_theme(), Some(Theme::TokyoNight));
        assert_eq!(page.rows[0].heading, Some(AVAILABLE_SECTION));
    }

    #[test]
    fn theme_files_are_offered_after_the_curated_themes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("paper-test.toml"),
            "[meta]\nname = \"Paper\"\ndescription = \"Plain\"\nvariant = \"light\"\n",
        )
        .unwrap();
        let files = crate::theme::load_user_themes(&[dir.path().to_path_buf()]);
        let mut page = ThemesPage::new(Theme::System, files.clone());
        assert_eq!(page.rows.len(), Theme::ALL.len() + 1);
        assert_eq!(page.rows.last().map(|row| row.theme), Some(files[0]));
        page.query = "paper".into();
        page.refilter();
        assert_eq!(page.selected_theme(), Some(files[0]));
        let configured = ThemesPage::new(files[0], files.clone());
        assert_eq!(configured.rows[0].theme, files[0], "the current section");
    }
}
