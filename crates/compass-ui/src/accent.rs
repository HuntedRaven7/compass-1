//! The desktop's accent colour, as libadwaita defines it.
//!
//! `org.gnome.desktop.interface accent-color` names one of libadwaita's nine
//! accents. Compass uses the same base colours and the same "standalone"
//! adjustment (an Oklab lightness clamp) as
//! `libadwaita/src/adw-accent-color.c`, so a user who picks green gets green
//! rather than Compass's hard-coded Adwaita blue.

use crate::design::Rgb;

/// One of libadwaita's system accent colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccentColor {
    /// The default Adwaita blue.
    #[default]
    Blue,
    /// Teal.
    Teal,
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Orange.
    Orange,
    /// Red.
    Red,
    /// Pink.
    Pink,
    /// Purple.
    Purple,
    /// Slate.
    Slate,
}

impl AccentColor {
    /// Parses the `accent-color` GSettings value. Anything unrecognised falls
    /// back to blue, which is what libadwaita itself does.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim().trim_matches('\'').trim_matches('"').trim() {
            "teal" => Self::Teal,
            "green" => Self::Green,
            "yellow" => Self::Yellow,
            "orange" => Self::Orange,
            "red" => Self::Red,
            "pink" => Self::Pink,
            "purple" => Self::Purple,
            "slate" => Self::Slate,
            _ => Self::Blue,
        }
    }

    /// The accent's background colour, from libadwaita's `adw_accent_color_to_rgba`.
    #[must_use]
    pub const fn base(self) -> Rgb {
        match self {
            Self::Blue => Rgb::new(0x35, 0x84, 0xe4),
            Self::Teal => Rgb::new(0x21, 0x90, 0xa4),
            Self::Green => Rgb::new(0x3a, 0x94, 0x4a),
            Self::Yellow => Rgb::new(0xc8, 0x88, 0x00),
            Self::Orange => Rgb::new(0xed, 0x5b, 0x00),
            Self::Red => Rgb::new(0xe6, 0x2d, 0x42),
            Self::Pink => Rgb::new(0xd5, 0x61, 0x99),
            Self::Purple => Rgb::new(0x91, 0x41, 0xac),
            Self::Slate => Rgb::new(0x6f, 0x83, 0x96),
        }
    }

    /// A version of the accent that stands out on `dark` or light surfaces.
    ///
    /// libadwaita clamps the Oklab lightness to at least 0.85 on dark surfaces
    /// (lighter) and at most 0.5 on light ones (darker), keeping the hue and
    /// chroma. We use the light-surface value for filled rows in either
    /// appearance: white text on it stays legible.
    #[must_use]
    pub fn standalone(self, dark: bool) -> Rgb {
        let base = self.base();
        let (l, a, b) = rgb_to_oklab(base.r, base.g, base.b);
        let l = if dark { l.max(0.85) } else { l.min(0.5) };
        oklab_to_rgb(l, a, b)
    }
}

fn ungamma(v: f64) -> f64 {
    if v >= 0.04045 {
        ((v + 0.055) / 1.055).powf(2.4)
    } else {
        v / 12.92
    }
}

fn gamma(v: f64) -> f64 {
    if v > 0.0031308 {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * v
    }
}

fn rgb_to_oklab(red: u8, green: u8, blue: u8) -> (f64, f64, f64) {
    let r = ungamma(f64::from(red) / 255.0);
    let g = ungamma(f64::from(green) / 255.0);
    let b = ungamma(f64::from(blue) / 255.0);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )
}

fn oklab_to_rgb(l: f64, a: f64, b: f64) -> Rgb {
    let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let ss = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let channel = |r: f64, g: f64, b: f64| {
        (gamma((r * ll + g * mm + b * ss).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Rgb::new(
        channel(4.0767416621, -3.3077115913, 0.2309699292),
        channel(-1.2684380046, 2.6097574011, -0.3413193965),
        channel(-0.0041960863, -0.7034186147, 1.7076147010),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gnome_accent_name_is_recognised() {
        for (name, expected) in [
            ("blue", AccentColor::Blue),
            ("teal", AccentColor::Teal),
            ("green", AccentColor::Green),
            ("yellow", AccentColor::Yellow),
            ("orange", AccentColor::Orange),
            ("red", AccentColor::Red),
            ("pink", AccentColor::Pink),
            ("purple", AccentColor::Purple),
            ("slate", AccentColor::Slate),
        ] {
            assert_eq!(AccentColor::from_name(name), expected);
            assert_eq!(AccentColor::from_name(&format!("'{name}'")), expected);
        }
        assert_eq!(AccentColor::from_name("chartreuse"), AccentColor::Blue);
    }

    #[test]
    fn blue_matches_libadwaita_and_the_current_launcher() {
        assert_eq!(AccentColor::Blue.base(), Rgb::new(0x35, 0x84, 0xe4));
        // The darker standalone blue is what a filled row needs for white text.
        let fill = AccentColor::Blue.standalone(false);
        assert!(fill.r < 0x35 && fill.g < 0x84 && fill.b < 0xe4, "{fill:?}");
        // On a dark surface libadwaita lightens it instead.
        let ring = AccentColor::Blue.standalone(true);
        assert!(ring.r > 0x35 && ring.g > 0x84 && ring.b > 0xe4, "{ring:?}");
    }

    #[test]
    fn a_non_blue_accent_really_changes_the_palette() {
        assert_ne!(AccentColor::Green.base(), AccentColor::Blue.base());
        assert_ne!(
            AccentColor::Green.standalone(false),
            AccentColor::Blue.standalone(false)
        );
    }
}
