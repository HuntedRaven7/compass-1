//! Curated color themes for #153, and the user's theme files.
//!
//! Each curated entry is a full [`Palette`] so the browser surrogate can
//! render it from `states.json` without retyping colours. `system` is not a
//! palette — it means "follow the portal appearance with Adwaita colours".
//!
//! A theme file (`compass_core::theme_file`) becomes a [`Theme::User`]: it is
//! read once per scan, resolved to a palette, and kept for the life of the
//! process so the theme stays a `Copy` value the whole launcher can pass
//! around. A file read again unchanged reuses its entry; an edited one adds a
//! new entry, so what is kept grows only with edits.

use crate::design::{Appearance, Palette, Rgb};

/// A named theme from the #153 assortment.
///
/// The string forms are what `launcher.appearance.theme` persists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Follow the desktop (Adwaita light/dark).
    #[default]
    System,
    /// Catppuccin Mocha (dark) / Latte (light).
    Catppuccin,
    /// Dracula — dark only, light falls back to a light variant.
    Dracula,
    /// Nord — dark and light.
    Nord,
    /// Gruvbox — dark and light.
    Gruvbox,
    /// Tokyo Night — dark (Storm) / light.
    TokyoNight,
    /// Solarized — dark / light.
    Solarized,
    /// A theme file from the user's or the system's theme directory.
    #[serde(skip)]
    User(&'static UserTheme),
}

/// A theme file, resolved to what the launcher draws with.
#[derive(Debug, Clone, PartialEq)]
pub struct UserTheme {
    /// The file's id, which the configuration stores.
    pub id: String,
    /// `meta.name`.
    pub name: String,
    /// `meta.description`.
    pub description: String,
    /// The file it was read from.
    pub path: String,
    /// Its palette, whatever the desktop's appearance: a theme file is dark
    /// or light by its own `variant`.
    pub palette: Palette,
    /// The eight swatches a row shows.
    pub swatches: [Rgb; 8],
}

// The palette's only float is its backdrop alpha, which is never NaN.
impl Eq for UserTheme {}

static USER_THEMES: std::sync::Mutex<Vec<&'static UserTheme>> = std::sync::Mutex::new(Vec::new());

fn rgb(color: compass_core::theme_file::Rgba) -> Rgb {
    Rgb::new(color.r, color.g, color.b)
}

/// Reads the theme files in `dirs` and returns them as themes, keeping each
/// for [`Theme::from_name`] to find.
pub fn load_user_themes(dirs: &[std::path::PathBuf]) -> Vec<Theme> {
    use compass_core::theme_file;
    let files = theme_file::scan(dirs);
    let mut kept = USER_THEMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    files
        .iter()
        .map(|file| {
            let resolved = theme_file::resolve(file, &files);
            let theme = UserTheme {
                id: file.id.clone(),
                name: file.name.clone(),
                description: file.description.clone(),
                path: file.path.to_string_lossy().into_owned(),
                palette: Palette {
                    surface: rgb(resolved.background),
                    field: rgb(resolved.input_background),
                    text: rgb(resolved.text),
                    muted: rgb(resolved.muted),
                    selection: rgb(resolved.selection),
                    selection_text: rgb(resolved.selection_text),
                    border: rgb(resolved.border),
                    accent: rgb(resolved.accent),
                    backdrop: Rgb::new(0, 0, 0),
                    backdrop_alpha: if resolved.dark { 0.35 } else { 0.20 },
                },
                swatches: resolved.swatches.map(rgb),
            };
            let entry = match kept.iter().find(|entry| ***entry == theme) {
                Some(entry) => *entry,
                None => {
                    let entry: &'static UserTheme = Box::leak(Box::new(theme));
                    // The newest read of an id is the one `from_name` finds.
                    kept.insert(0, entry);
                    entry
                }
            };
            Theme::User(entry)
        })
        .collect()
}

/// [`load_user_themes`] over this session's theme directories.
pub fn load_default_user_themes() -> Vec<Theme> {
    load_user_themes(&compass_core::theme_file::default_search_dirs())
}

impl Theme {
    /// All curated names, for the picker and for `states.json`.
    pub const ALL: [Self; 7] = [
        Self::System,
        Self::Catppuccin,
        Self::Dracula,
        Self::Nord,
        Self::Gruvbox,
        Self::TokyoNight,
        Self::Solarized,
    ];

    /// The persisted spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::User(theme) => &theme.id,
            Self::System => "system",
            Self::Catppuccin => "catppuccin",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
            Self::TokyoNight => "tokyo-night",
            Self::Solarized => "solarized",
        }
    }

    /// What the picker calls it.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::User(theme) => &theme.name,
            Self::System => "System",
            Self::Catppuccin => "Catppuccin",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::Gruvbox => "Gruvbox",
            Self::TokyoNight => "Tokyo Night",
            Self::Solarized => "Solarized",
        }
    }

    /// A line saying what it is, for the picker and `compass theme list`.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::User(theme) => &theme.description,
            Self::System => "Follow OS (Adwaita)",
            Self::Catppuccin => "Catppuccin (Mocha/Latte)",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::Gruvbox => "Gruvbox",
            Self::TokyoNight => "Tokyo Night",
            Self::Solarized => "Solarized",
        }
    }

    /// Parse the persisted spelling, case-insensitive.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "catppuccin" | "catppuccin-mocha" | "catppuccin-latte" => Some(Self::Catppuccin),
            "dracula" => Some(Self::Dracula),
            "nord" => Some(Self::Nord),
            "gruvbox" => Some(Self::Gruvbox),
            "tokyo-night" | "tokyonight" | "tokyo_night" => Some(Self::TokyoNight),
            "solarized" => Some(Self::Solarized),
            _ => USER_THEMES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .find(|theme| theme.id == name.trim())
                .map(|theme| Self::User(theme)),
        }
    }

    /// The file a user theme was read from.
    #[must_use]
    pub fn path(self) -> Option<&'static str> {
        match self {
            Self::User(theme) => Some(&theme.path),
            _ => None,
        }
    }

    /// The eight swatches a user theme's row shows.
    #[must_use]
    pub fn swatches(self) -> Option<&'static [Rgb; 8]> {
        match self {
            Self::User(theme) => Some(&theme.swatches),
            _ => None,
        }
    }

    /// Palette for this theme at the given appearance, brought up to WCAG
    /// AA ([`Palette::accessible`]).
    ///
    /// System returns the Adwaita palette; the others return curated
    /// palettes, and a theme file its own. Each is drawn as its author wrote
    /// it except where a pair falls short: a selection white text cannot be
    /// read on, a secondary text too faint for its background. Doing it here
    /// rather than in each palette means a theme file someone writes
    /// tomorrow is held to the same line.
    #[must_use]
    pub fn palette(self, appearance: Appearance) -> Palette {
        self.declared_palette(appearance).accessible()
    }

    /// The palette as the theme declares it.
    // Over the 150-line limit when it was added (#379): split it, don't grow it.
    #[allow(clippy::too_many_lines)]
    fn declared_palette(self, appearance: Appearance) -> Palette {
        match self {
            Self::User(theme) => theme.palette,
            Self::System => crate::design::palette(appearance),
            Self::Catppuccin => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x1e, 0x1e, 0x2e), // Mocha base
                    field: Rgb::new(0x31, 0x32, 0x44),   // surface0
                    text: Rgb::new(0xcd, 0xd6, 0xf4),
                    muted: Rgb::new(0xa6, 0xad, 0xc8), // subtext0
                    selection: Rgb::new(0x89, 0xb4, 0xfa), // blue
                    selection_text: Rgb::new(0x1e, 0x1e, 0x2e),
                    border: Rgb::new(0x45, 0x47, 0x5a), // surface1
                    accent: Rgb::new(0x89, 0xb4, 0xfa),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xef, 0xf1, 0xf5), // Latte base
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x4c, 0x4f, 0x69),
                    muted: Rgb::new(0x8c, 0x8f, 0xa1),
                    selection: Rgb::new(0x1e, 0x66, 0xf5),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xcc, 0xcd, 0xd6),
                    accent: Rgb::new(0x1e, 0x66, 0xf5),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Dracula => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x28, 0x2a, 0x36),
                    field: Rgb::new(0x44, 0x47, 0x5a),
                    text: Rgb::new(0xf8, 0xf8, 0xf2),
                    muted: Rgb::new(0xbd, 0x93, 0xf9), // purple muted
                    selection: Rgb::new(0xbd, 0x93, 0xf9),
                    selection_text: Rgb::new(0x28, 0x2a, 0x36),
                    border: Rgb::new(0x62, 0x72, 0xa4), // comment
                    accent: Rgb::new(0xff, 0x79, 0xc6), // pink
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xf8, 0xf8, 0xf2),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x28, 0x2a, 0x36),
                    muted: Rgb::new(0x62, 0x72, 0xa4),
                    selection: Rgb::new(0xbd, 0x93, 0xf9),
                    selection_text: Rgb::new(0x28, 0x2a, 0x36),
                    border: Rgb::new(0xbd, 0x93, 0xf9),
                    accent: Rgb::new(0xff, 0x79, 0xc6),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Nord => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x2e, 0x34, 0x40), // polar night 0
                    field: Rgb::new(0x3b, 0x42, 0x52),   // polar night 1
                    text: Rgb::new(0xec, 0xef, 0xf4),    // snow storm 2
                    muted: Rgb::new(0x81, 0xa1, 0xc1),   // frost
                    selection: Rgb::new(0x88, 0xc0, 0xd0),
                    selection_text: Rgb::new(0x2e, 0x34, 0x40),
                    border: Rgb::new(0x4c, 0x56, 0x6a),
                    accent: Rgb::new(0x81, 0xa1, 0xc1),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xec, 0xef, 0xf4),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x2e, 0x34, 0x40),
                    muted: Rgb::new(0x4c, 0x56, 0x6a),
                    selection: Rgb::new(0x5e, 0x81, 0xac),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xd8, 0xde, 0xe9),
                    accent: Rgb::new(0x5e, 0x81, 0xac),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Gruvbox => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x28, 0x28, 0x28),   // bg0
                    field: Rgb::new(0x3c, 0x38, 0x36),     // bg1
                    text: Rgb::new(0xeb, 0xdb, 0xb2),      // fg
                    muted: Rgb::new(0xa8, 0x99, 0x84),     // gray
                    selection: Rgb::new(0xfb, 0x49, 0x34), // red
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0x50, 0x49, 0x45), // bg2
                    accent: Rgb::new(0xfe, 0x80, 0x19), // orange
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xfb, 0xf1, 0xc7), // bg0 light
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x3c, 0x38, 0x36),
                    muted: Rgb::new(0x7c, 0x6f, 0x64),
                    selection: Rgb::new(0xcc, 0x24, 0x1d),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xd5, 0xc4, 0xa1),
                    accent: Rgb::new(0xd6, 0x5d, 0x0e),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::TokyoNight => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x1a, 0x1b, 0x26), // Storm
                    field: Rgb::new(0x24, 0x28, 0x3b),
                    text: Rgb::new(0xc0, 0xca, 0xf5),
                    muted: Rgb::new(0x56, 0x5f, 0x89),
                    selection: Rgb::new(0x7a, 0xa2, 0xf7), // blue
                    selection_text: Rgb::new(0x1a, 0x1b, 0x26),
                    border: Rgb::new(0x29, 0x2e, 0x42),
                    accent: Rgb::new(0x7a, 0xa2, 0xf7),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xd5, 0xd6, 0xdb),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x1a, 0x1b, 0x26),
                    muted: Rgb::new(0x56, 0x5f, 0x89),
                    selection: Rgb::new(0x34, 0x5c, 0xb2),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xcb, 0xcc, 0xd1),
                    accent: Rgb::new(0x34, 0x5c, 0xb2),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Solarized => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x00, 0x2b, 0x36),   // base03
                    field: Rgb::new(0x07, 0x36, 0x42),     // base02
                    text: Rgb::new(0x83, 0x94, 0x96),      // base0
                    muted: Rgb::new(0x58, 0x6e, 0x75),     // base01
                    selection: Rgb::new(0x26, 0x8b, 0xd2), // blue
                    selection_text: Rgb::new(0xfd, 0xf6, 0xe3),
                    border: Rgb::new(0x07, 0x36, 0x42),
                    accent: Rgb::new(0x2a, 0xa1, 0x98), // cyan
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xfd, 0xf6, 0xe3), // base3
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x58, 0x6e, 0x75), // base01 — darker for 4.5:1
                    muted: Rgb::new(0x93, 0xa1, 0xa1), // base1
                    selection: Rgb::new(0x26, 0x8b, 0xd2),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xee, 0xe8, 0xd5), // base2
                    accent: Rgb::new(0x2a, 0xa1, 0x98),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::Appearance;

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        compass_core::contrast::contrast_ratio(a.into(), b.into())
    }

    /// Every bundled theme: the curated ones in both appearances, and each
    /// file in `extra/themes`.
    fn bundled() -> Vec<(String, Palette)> {
        let mut palettes = Vec::new();
        for theme in Theme::ALL {
            for appearance in Appearance::ALL {
                palettes.push((
                    format!("{} {}", theme.name(), appearance.name()),
                    theme.palette(appearance),
                ));
            }
        }
        let files = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extra/themes");
        let loaded = load_user_themes(&[files]);
        assert!(loaded.len() > 20, "the bundled theme files are read");
        for theme in loaded {
            palettes.push((theme.name().to_owned(), theme.palette(Appearance::Light)));
        }
        palettes
    }

    #[test]
    fn every_bundled_theme_meets_wcag_aa() {
        use crate::design::{TEXT_CONTRAST, UI_CONTRAST};
        let mut failures = Vec::new();
        for (name, p) in bundled() {
            let mut check = |what: &str, ratio: f64, needed: f64| {
                if ratio < needed {
                    failures.push(format!("{name}: {what} {ratio:.2} < {needed}"));
                }
            };
            for (place, background) in [("card", p.surface), ("field", p.field), ("list", p.card())]
            {
                check(
                    &format!("text/{place}"),
                    contrast(p.text, background),
                    TEXT_CONTRAST,
                );
                check(
                    &format!("muted/{place}"),
                    contrast(p.muted, background),
                    TEXT_CONTRAST,
                );
                check(
                    &format!("accent/{place}"),
                    contrast(p.accent, background),
                    UI_CONTRAST,
                );
                check(
                    &format!("control/{place}"),
                    contrast(p.control(), background),
                    UI_CONTRAST,
                );
            }
            check(
                "selection text",
                contrast(p.selection_text, p.selection),
                TEXT_CONTRAST,
            );
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn adwaita_is_already_accessible_and_selects_in_its_accent_colour() {
        use crate::design::{DARK, LIGHT};
        assert_eq!(LIGHT.accessible(), LIGHT);
        assert_eq!(DARK.accessible(), DARK);
        assert_eq!(LIGHT.selection.hex(), "#1c71d8");
    }

    #[test]
    fn a_theme_that_falls_short_is_raised_and_keeps_its_hue() {
        // White on Gruvbox's red is 3.3:1 as declared.
        let declared = Theme::Gruvbox.declared_palette(Appearance::Dark);
        let drawn = Theme::Gruvbox.palette(Appearance::Dark);
        assert!(contrast(declared.selection_text, declared.selection) < 4.5);
        assert_ne!(drawn.selection, declared.selection);
        assert!(
            drawn.selection.r > drawn.selection.g,
            "still red: {drawn:?}"
        );
    }

    #[test]
    fn curated_themes_round_trip_through_config() {
        // #153: custom/user choices preserved across updates — theme string round-trips
        for theme in Theme::ALL {
            let name = theme.name();
            let parsed = Theme::from_name(name).expect("round-trip");
            assert_eq!(parsed, theme, "theme {name} did not round-trip");
            // case-insensitive and alias forms
            assert_eq!(Theme::from_name(&name.to_ascii_uppercase()), Some(theme));
        }
        // System is the default fallback
        assert_eq!(Theme::from_name("unknown"), None);
        assert_eq!(Theme::default(), Theme::System);
    }

    #[test]
    fn a_theme_file_becomes_a_theme_found_by_its_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ocean-test.toml"),
            "[meta]\nname = \"Ocean\"\ndescription = \"Deep\"\nvariant = \"dark\"\n\
             [colors.core]\nbackground = \"#001122\"\nforeground = \"#eeeeee\"\n",
        )
        .unwrap();
        let themes = load_user_themes(&[dir.path().to_path_buf()]);
        assert_eq!(themes.len(), 1);
        let theme = themes[0];
        assert_eq!(theme.name(), "ocean-test");
        assert_eq!(theme.title(), "Ocean");
        assert_eq!(Theme::from_name("ocean-test"), Some(theme));
        let palette = theme.palette(Appearance::Light);
        assert_eq!(palette.surface, Rgb::new(0x00, 0x11, 0x22));
        assert_eq!(palette.text, Rgb::new(0xee, 0xee, 0xee));
        let again = load_user_themes(&[dir.path().to_path_buf()]);
        assert_eq!(again, themes, "an unchanged file reuses its entry");
    }
}
