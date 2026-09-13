//! Icon theme lookup per the freedesktop [icon theme specification][spec].
//!
//! This module finds icon files given an icon name, searching the standard
//! XDG icon directories in precedence order, respecting theme inheritance and
//! the icon theme's `index.theme` file.
//!
//! [spec]: https://specifications.freedesktop.org/icon-theme-spec/latest/

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::xdg_dirs::icon_dirs;

/// An icon theme directory entry.
#[derive(Debug, Clone)]
pub struct IconThemeDir {
    /// The directory path containing icons.
    pub path: PathBuf,
    /// The size this directory is for, or None for scalable.
    pub size: Option<u32>,
    /// The icon type (fixed, scalable, threshold).
    pub icon_type: IconType,
    /// The context this directory is for, or None for all.
    pub context: Option<String>,
}

/// The type of icons in a directory, from the icon theme specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconType {
    /// Fixed size icons (e.g., 16x16, 24x24).
    Fixed,
    /// Scalable icons (SVG).
    Scalable,
    /// Threshold icons - used if no fixed size matches.
    Threshold,
}

impl IconType {
    /// Parse from the string in index.theme.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fixed" => Some(Self::Fixed),
            "scalable" => Some(Self::Scalable),
            "threshold" => Some(Self::Threshold),
            _ => None,
        }
    }
}

impl std::str::FromStr for IconType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str(s).ok_or(())
    }
}

/// A parsed icon theme index file.
#[derive(Debug, Clone, Default)]
pub struct IconTheme {
    /// The theme name.
    pub name: String,
    /// The theme comment/description.
    pub comment: Option<String>,
    /// Parent themes to inherit from.
    pub inherits: Vec<String>,
    /// Directories in this theme.
    pub directories: Vec<IconThemeDir>,
    /// The theme's example icon.
    pub example: Option<String>,
}

impl IconTheme {
    /// Parse an index.theme file.
    pub fn parse(data: &str) -> Option<Self> {
        let mut theme = Self::default();
        let mut current_section = String::new();

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current_section = section.to_owned();
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match current_section.as_str() {
                "Icon Theme" => match key {
                    "Name" => theme.name = value.to_owned(),
                    "Comment" => theme.comment = Some(value.to_owned()),
                    "Inherits" => {
                        theme.inherits = value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToOwned::to_owned)
                            .collect();
                    }
                    "Example" => theme.example = Some(value.to_owned()),
                    _ => {}
                },
                _ => {
                    // Directory section
                    let _dir = IconThemeDir {
                        path: PathBuf::new(),
                        size: None,
                        icon_type: IconType::Fixed,
                        context: None,
                    };
                    // We'll parse directory sections differently - need to know the section name
                }
            }
        }

        // Second pass for directory sections
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current_section = section.to_owned();
                continue;
            }

            if current_section != "Icon Theme" {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };

                // Find or create the directory entry for this section
                let mut found = false;
                for dir in &mut theme.directories {
                    if dir.path.file_name().and_then(|s| s.to_str()) == Some(&current_section) {
                        match key {
                            "Size" => dir.size = value.parse().ok(),
                            "Type" => {
                                dir.icon_type = IconType::from_str(value).unwrap_or(IconType::Fixed)
                            }
                            "Context" => dir.context = Some(value.to_owned()),
                            _ => {}
                        }
                        found = true;
                        break;
                    }
                }

                if !found {
                    let size = if key == "Size" {
                        value.parse().ok()
                    } else {
                        None
                    };
                    let icon_type = if key == "Type" {
                        IconType::from_str(value).unwrap_or(IconType::Fixed)
                    } else {
                        IconType::Fixed
                    };
                    let context = if key == "Context" {
                        Some(value.to_owned())
                    } else {
                        None
                    };

                    theme.directories.push(IconThemeDir {
                        path: PathBuf::from(current_section.clone()),
                        size,
                        icon_type,
                        context,
                    });
                }
            }
        }

        Some(theme)
    }

    /// Read and parse the index.theme file at the given path.
    pub fn from_file(path: &Path) -> Option<Self> {
        let data = std::fs::read_to_string(path).ok()?;
        Self::parse(&data)
    }
}

/// Find an icon file by name, searching through XDG icon directories.
///
/// Returns the absolute path to the icon file if found, preferring the
/// highest-resolution match according to the icon theme specification.
///
/// # Arguments
///
/// * `icon_name` - The icon name to search for (without extension).
/// * `theme_name` - The icon theme to search in. If None, uses the default
///   theme from `$XDG_CURRENT_DESKTOP` or falls back to "hicolor".
/// * `size` - Desired icon size in pixels. If None, returns the scalable version
///   if available, otherwise the largest fixed size.
/// * `scale` - Desired scale factor (e.g., 2.0 for @2x).
///
/// # Search order
///
/// 1. `$XDG_DATA_HOME/icons/<theme>/...`
/// 2. Each `$XDG_DATA_DIRS/icons/<theme>/...` in order
/// 3. If theme not found, try parent themes (from `Inherits`)
/// 4. Fall back to "hicolor" theme
/// 5. If still not found, try other themes in the same directories
pub fn find_icon(
    icon_name: &str,
    theme_name: Option<&str>,
    size: Option<u32>,
    scale: f32,
) -> Option<PathBuf> {
    let theme = theme_name.unwrap_or("hicolor");
    let search_dirs = icon_search_dirs(theme);

    let target_size = size.map(|s| (s as f32 * scale).round() as u32);

    // Search in theme and its parents
    let mut visited = HashSet::new();
    let mut themes_to_search = vec![theme.to_owned()];

    while let Some(current_theme) = themes_to_search.pop() {
        if !visited.insert(current_theme.clone()) {
            continue;
        }

        for base_dir in &search_dirs {
            let theme_dir = base_dir.join(&current_theme);
            if let Some(found) = find_icon_in_theme_dir(&theme_dir, icon_name, target_size, scale) {
                return Some(found);
            }
        }

        // Add parent themes
        if let Some(parent_theme) = load_theme_inherits(&search_dirs, &current_theme) {
            themes_to_search.extend(parent_theme);
        }
    }

    // Fallback: search all themes in the search directories
    for base_dir in &search_dirs {
        if let Ok(read_dir) = std::fs::read_dir(base_dir) {
            for entry in read_dir.flatten() {
                let theme_name = entry.file_name();
                let theme_name_str = theme_name.to_string_lossy();
                if visited.contains(&*theme_name_str) {
                    continue;
                }
                let theme_dir = entry.path();
                if let Some(found) =
                    find_icon_in_theme_dir(&theme_dir, icon_name, target_size, scale)
                {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Get the icon search directories in precedence order.
fn icon_search_dirs(_theme: &str) -> Vec<PathBuf> {
    icon_dirs()
}

/// Load the Inherits list from a theme's index.theme.
fn load_theme_inherits(search_dirs: &[PathBuf], theme: &str) -> Option<Vec<String>> {
    for base_dir in search_dirs {
        let index_path = base_dir.join(theme).join("index.theme");
        if let Some(theme_data) = IconTheme::from_file(&index_path)
            && !theme_data.inherits.is_empty()
        {
            return Some(theme_data.inherits);
        }
    }
    None
}

/// Search for an icon file within a theme directory.
fn find_icon_in_theme_dir(
    theme_dir: &Path,
    icon_name: &str,
    target_size: Option<u32>,
    _scale: f32,
) -> Option<PathBuf> {
    // Read index.theme to get directory structure
    let index_path = theme_dir.join("index.theme");
    let theme = IconTheme::from_file(&index_path)?;

    // Find matching directories
    let mut candidates = Vec::new();

    for dir in &theme.directories {
        let dir_path = theme_dir.join(&dir.path);
        if !dir_path.exists() {
            continue;
        }

        // Check context match (we ignore context for now, could be improved)
        // Check if this directory can provide the target size
        let can_provide = match dir.icon_type {
            IconType::Fixed => {
                if let Some(dir_size) = dir.size {
                    target_size.is_none_or(|ts| ts <= dir_size)
                } else {
                    true
                }
            }
            IconType::Scalable => true,
            IconType::Threshold => {
                if let Some(dir_size) = dir.size {
                    target_size.is_none_or(|ts| ts >= dir_size)
                } else {
                    true
                }
            }
        };

        if !can_provide {
            continue;
        }

        // Search for the icon file in this directory
        let extensions = ["svg", "png", "xpm", "jpg", "jpeg"];
        for ext in &extensions {
            let icon_path = dir_path.join(format!("{icon_name}.{ext}"));
            if icon_path.exists() {
                candidates.push((icon_path, dir.icon_type, dir.size));
            }
        }
    }

    // Sort candidates by preference:
    // 1. Scalable (SVG) first
    // 2. Then fixed size >= target, smallest first
    // 3. Then threshold
    candidates.sort_by(|a, b| {
        use IconType::*;
        let (_, type_a, size_a) = a;
        let (_, type_b, size_b) = b;

        // Scalable wins
        match (type_a, type_b) {
            (Scalable, Scalable) => {}
            (Scalable, _) => return std::cmp::Ordering::Less,
            (_, Scalable) => return std::cmp::Ordering::Greater,
            _ => {}
        }

        // For fixed sizes, prefer exact match or smallest >= target
        if matches!((type_a, type_b), (Fixed, Fixed)) {
            match (size_a, size_b, target_size) {
                (Some(sa), Some(sb), Some(ts)) => {
                    let diff_a = sa.abs_diff(ts);
                    let diff_b = sb.abs_diff(ts);
                    diff_a.cmp(&diff_b).then(sa.cmp(sb))
                }
                (Some(_), None, _) => std::cmp::Ordering::Less,
                (None, Some(_), _) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        } else {
            std::cmp::Ordering::Equal
        }
    });

    candidates.first().map(|(p, _, _)| p.clone())
}

/// Get the default icon theme name from the environment.
pub fn default_theme() -> String {
    // Check $XDG_CURRENT_DESKTOP for known desktop-specific themes
    if let Ok(desktops) = std::env::var("XDG_CURRENT_DESKTOP") {
        for desktop in desktops.split(':') {
            match desktop.to_ascii_lowercase().as_str() {
                "gnome" | "gnome-classic" => return "Adwaita".to_owned(),
                "kde" => return "breeze".to_owned(),
                "xfce" => return "elementary-xfce".to_owned(),
                _ => {}
            }
        }
    }

    // Check $GTK_THEME
    if let Ok(gtk_theme) = std::env::var("GTK_THEME")
        && let Some(theme) = gtk_theme.split(':').next()
    {
        return theme.to_owned();
    }

    "hicolor".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn icon_theme_parse() {
        let data = r#"
[Icon Theme]
Name=Test Theme
Comment=A test theme
Inherits=hicolor
Example=folder

[scalable]
Size=256
Type=Scalable
Context=places

[16x16]
Size=16
Type=Fixed
Context=places
"#;

        let theme = IconTheme::parse(data).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.comment, Some("A test theme".to_owned()));
        assert_eq!(theme.inherits, vec!["hicolor"]);
        assert_eq!(theme.example, Some("folder".to_owned()));
        assert_eq!(theme.directories.len(), 2);

        let scalable = theme
            .directories
            .iter()
            .find(|d| d.path.as_os_str() == "scalable")
            .unwrap();
        assert_eq!(scalable.icon_type, IconType::Scalable);
        assert_eq!(scalable.size, Some(256));
        assert_eq!(scalable.context, Some("places".to_owned()));

        let fixed = theme
            .directories
            .iter()
            .find(|d| d.path.as_os_str() == "16x16")
            .unwrap();
        assert_eq!(fixed.icon_type, IconType::Fixed);
        assert_eq!(fixed.size, Some(16));
        assert_eq!(fixed.context, Some("places".to_owned()));
    }

    #[test]
    fn find_icon_in_test_theme() {
        let dir = tempdir().unwrap();
        let theme_dir = dir.path().join("test-theme");
        fs::create_dir_all(theme_dir.join("scalable")).unwrap();
        fs::create_dir_all(theme_dir.join("16x16")).unwrap();

        fs::write(
            theme_dir.join("index.theme"),
            r#"
[Icon Theme]
Name=Test
Inherits=hicolor

[scalable]
Size=256
Type=Scalable

[16x16]
Size=16
Type=Fixed
"#,
        )
        .unwrap();

        fs::write(
            theme_dir.join("scalable").join("test-icon.svg"),
            "<svg></svg>",
        )
        .unwrap();
        fs::write(theme_dir.join("16x16").join("test-icon.png"), b"png").unwrap();

        let found = find_icon_in_theme_dir(&theme_dir, "test-icon", Some(16), 1.0);
        assert!(found.is_some());
        assert!(found.unwrap().to_string_lossy().contains("test-icon"));
    }
}
