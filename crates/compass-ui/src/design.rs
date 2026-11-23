//! The launcher's visual vocabulary: one source of truth for both renderers.
//!
//! Two things draw this launcher. Iced draws the real one, on a compositor.
//! The design page under `tools/design/` draws a surrogate in a browser, so a
//! change can be looked at in a second instead of after a twenty-minute VM
//! run. They agree because they read the same numbers from here — the browser
//! page is served this module as JSON rather than having the palette retyped
//! into CSS, which is how a mock starts lying.
//!
//! What the surrogate proves is layout, colour and state. What it cannot prove
//! is that Iced paints the same thing, and the VM tier remains the only judge
//! of that.

use serde::Serialize;

/// What the desktop says it prefers, as `org.freedesktop.appearance`
/// `color-scheme` reports it over the XDG settings portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ColorScheme {
    /// The desktop has no opinion; the application picks.
    NoPreference,
    /// The desktop asks for dark.
    PreferDark,
    /// The desktop asks for light.
    PreferLight,
}

impl ColorScheme {
    /// Reads the portal's integer.
    ///
    /// **1 is dark and 2 is light**, which is the order the freedesktop
    /// specification gives and not the order anyone guesses. Getting it
    /// backwards produces a launcher that is dark for exactly the users who
    /// asked for light, so the mapping has a test of its own.
    ///
    /// An unrecognised value is `NoPreference` rather than an error: the
    /// specification reserves room to grow, and a desktop that reports
    /// something newer should get a readable launcher rather than none.
    #[must_use]
    pub const fn from_portal(value: u32) -> Self {
        match value {
            1 => Self::PreferDark,
            2 => Self::PreferLight,
            _ => Self::NoPreference,
        }
    }

    /// Which appearance to draw.
    ///
    /// No preference means light, because that is Adwaita's default and the
    /// launcher should look like the desktop it is sitting on rather than
    /// announce itself.
    #[must_use]
    pub const fn appearance(self) -> Appearance {
        match self {
            Self::PreferDark => Appearance::Dark,
            Self::NoPreference | Self::PreferLight => Appearance::Light,
        }
    }
}

/// Which of the two palettes is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Adwaita light.
    Light,
    /// Adwaita dark.
    Dark,
}

impl Appearance {
    /// Both, for the design page and for screenshot sweeps.
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    /// The name used in filenames and in the page's toggle.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// One colour, as 8-bit sRGB.
///
/// Stored as bytes rather than floats because these are transcribed from
/// GNOME's published Adwaita values, which are written as hex, and a float
/// round-trip makes them unrecognisable to anyone checking them against the
/// source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// A colour from its hex components.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#rrggbb`, for CSS and for reading in a diff.
    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// As an Iced colour.
    #[must_use]
    pub fn to_iced(self) -> iced::Color {
        iced::Color::from_rgb8(self.r, self.g, self.b)
    }
}

/// The colours one appearance draws with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Palette {
    /// The card's fill.
    pub surface: Rgb,
    /// The search field's fill, one step from the card.
    pub field: Rgb,
    /// Ordinary text.
    pub text: Rgb,
    /// Subtitles and headings.
    pub muted: Rgb,
    /// The selected row's fill.
    pub selection: Rgb,
    /// Text on the selected row.
    pub selection_text: Rgb,
    /// Hairlines: the card's edge, dividers.
    pub border: Rgb,
    /// The accent, for focus rings and shortcut chips.
    pub accent: Rgb,
    /// What is drawn behind the card, over the desktop.
    pub backdrop: Rgb,
    /// How opaque that backdrop is, 0–1.
    pub backdrop_alpha: f32,
}

/// The contrast WCAG AA asks of text (1.4.3).
pub const TEXT_CONTRAST: f64 = 4.5;
/// The contrast WCAG AA asks of a control's boundary and of a focus ring
/// (1.4.11).
pub const UI_CONTRAST: f64 = 3.0;

impl From<Rgb> for compass_core::contrast::Rgb {
    fn from(rgb: Rgb) -> Self {
        Self::new(rgb.r, rgb.g, rgb.b)
    }
}

impl From<compass_core::contrast::Rgb> for Rgb {
    fn from(rgb: compass_core::contrast::Rgb) -> Self {
        Self::new(rgb.r, rgb.g, rgb.b)
    }
}

/// `color`, moved as little as it takes to reach `ratio` against every one
/// of `backgrounds`.
fn readable(color: Rgb, backgrounds: &[Rgb], ratio: f64) -> Rgb {
    let backgrounds: Vec<compass_core::contrast::Rgb> =
        backgrounds.iter().map(|&rgb| rgb.into()).collect();
    compass_core::contrast::ensure_contrast(color.into(), &backgrounds, ratio).into()
}

impl Palette {
    /// A boxed list's fill: Adwaita's `card_bg_color`.
    ///
    /// White over the light window, and a lift of the text colour over the
    /// dark one. A palette's `field` is the first, but in a dark palette it is
    /// the sunken search field, darker than the card it sits on, which would
    /// read as a hole rather than a card.
    #[must_use]
    pub fn card(&self) -> Rgb {
        let lightness = |c: Rgb| u16::from(c.r) + u16::from(c.g) + u16::from(c.b);
        if lightness(self.field) >= lightness(self.surface) {
            self.field
        } else {
            compass_core::contrast::mix(self.surface.into(), self.text.into(), 0.08).into()
        }
    }

    /// The backgrounds text and controls are drawn on: the card, the search
    /// field and the boxed lists.
    #[must_use]
    pub fn backgrounds(&self) -> [Rgb; 3] {
        [self.surface, self.field, self.card()]
    }

    /// What marks out a control that is not filled with the accent: an
    /// entry's frame, a switch's track when off, the page dots.
    ///
    /// The hairline `border` is decoration and may be faint; this is the
    /// same colour pushed to 3:1 against every background, which is what
    /// WCAG asks of a control's boundary.
    #[must_use]
    pub fn control(&self) -> Rgb {
        readable(self.border, &self.backgrounds(), UI_CONTRAST)
    }

    /// The palette with every pair the launcher draws brought up to WCAG AA,
    /// moving each colour as little as it takes.
    ///
    /// Text, secondary text and the accent (used for notices and focus
    /// rings) reach 4.5:1 against the card, the field and the boxed lists;
    /// the selection's fill reaches 4.5:1 against the text drawn on it. A
    /// palette that already reads comes back unchanged.
    #[must_use]
    pub fn accessible(self) -> Self {
        let backgrounds = self.backgrounds();
        Self {
            text: readable(self.text, &backgrounds, TEXT_CONTRAST),
            muted: readable(self.muted, &backgrounds, TEXT_CONTRAST),
            accent: readable(self.accent, &backgrounds, TEXT_CONTRAST),
            selection: readable(self.selection, &[self.selection_text], TEXT_CONTRAST),
            ..self
        }
    }

    /// Text that reports a failure: Adwaita's `@error_color`, #c01c28 on a
    /// light card and #ff7b63 on a dark one, chosen by the card's lightness
    /// so a curated theme gets the variant it can be read on.
    #[must_use]
    pub fn error(&self) -> Rgb {
        let Rgb { r, g, b } = self.surface;
        let light = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 > 128_000;
        if light {
            Rgb::new(0xc0, 0x1c, 0x28)
        } else {
            Rgb::new(0xff, 0x7b, 0x63)
        }
    }
}

/// Adwaita light.
///
/// Transcribed from GNOME's named palette: `@window_bg_color` #fafafa,
/// `@view_bg_color` #ffffff. The selection and the accent are
/// `@accent_color` #1c71d8 rather than `@accent_bg_color` #3584e4: white on
/// #3584e4 is 3.77:1, short of the 4.5:1 a selected row's text needs.
pub const LIGHT: Palette = Palette {
    surface: Rgb::new(0xfa, 0xfa, 0xfa),
    field: Rgb::new(0xff, 0xff, 0xff),
    text: Rgb::new(0x1e, 0x1e, 0x1e),
    muted: Rgb::new(0x5e, 0x5c, 0x64),
    selection: Rgb::new(0x1c, 0x71, 0xd8),
    selection_text: Rgb::new(0xff, 0xff, 0xff),
    border: Rgb::new(0xd8, 0xd8, 0xd4),
    accent: Rgb::new(0x1c, 0x71, 0xd8),
    backdrop: Rgb::new(0x00, 0x00, 0x00),
    backdrop_alpha: 0.25,
};

/// Adwaita dark.
///
/// `@window_bg_color` #242424, `@view_bg_color` #1e1e1e, the selection in the
/// same #1c71d8 as light (white on it is 4.77:1), and the dark
/// `@accent_color` #78aeed for rings and notices. The secondary text is
/// lighter than Adwaita's #9a9996, which is 4.24:1 on a boxed list.
pub const DARK: Palette = Palette {
    surface: Rgb::new(0x24, 0x24, 0x24),
    field: Rgb::new(0x1e, 0x1e, 0x1e),
    text: Rgb::new(0xff, 0xff, 0xff),
    muted: Rgb::new(0xa3, 0xa2, 0x9f),
    selection: Rgb::new(0x1c, 0x71, 0xd8),
    selection_text: Rgb::new(0xff, 0xff, 0xff),
    border: Rgb::new(0x3d, 0x3d, 0x3d),
    accent: Rgb::new(0x78, 0xae, 0xed),
    backdrop: Rgb::new(0x00, 0x00, 0x00),
    backdrop_alpha: 0.35,
};

/// The palette for an appearance.
#[must_use]
pub const fn palette(appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Light => LIGHT,
        Appearance::Dark => DARK,
    }
}

/// Sizes and spacing, in logical pixels.
///
/// A Search Light-shaped card: a fixed width rather than a fraction of the
/// screen, because a launcher that grows with the monitor is unreadable on a
/// wide one, and centred horizontally but held high vertically, because the
/// eye starts above the middle and the results grow downward.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Geometry {
    /// The card's width.
    pub card_width: u16,
    /// The tallest the card grows before the list scrolls.
    pub card_max_height: u16,
    /// The card's corner radius.
    pub card_radius: u16,
    /// How far down the screen the card's top sits, as a fraction of height.
    pub card_top_fraction: f32,
    /// Padding inside the card.
    pub card_padding: u16,
    /// The search field's height.
    pub field_height: u16,
    /// The search field's corner radius.
    pub field_radius: u16,
    /// A result row's height.
    pub row_height: u16,
    /// A result row's corner radius when selected.
    pub row_radius: u16,
    /// The gap between rows.
    pub row_spacing: u16,
    /// The icon's edge length.
    pub icon_size: u16,
    /// Title text size.
    pub title_size: u16,
    /// Subtitle text size.
    pub subtitle_size: u16,
    /// Section heading text size.
    pub heading_size: u16,
    /// Search field text size.
    pub query_size: u16,
}

fn scale_size(size: u16, scale: f32) -> u16 {
    ((f64::from(size) * f64::from(scale)).round() as i64).clamp(1, i64::from(u16::MAX)) as u16
}

impl Geometry {
    /// Scales only the text sizes by the desktop interface font's scale,
    /// leaving the card's geometry alone. An unset scale returns `self`.
    #[must_use]
    pub fn scaled_text(self, scale: f32) -> Self {
        if scale == 1.0 {
            return self;
        }
        Self {
            title_size: scale_size(self.title_size, scale),
            subtitle_size: scale_size(self.subtitle_size, scale),
            heading_size: scale_size(self.heading_size, scale),
            query_size: scale_size(self.query_size, scale),
            ..self
        }
    }
}

/// The shipped geometry.
pub const GEOMETRY: Geometry = Geometry {
    card_width: 720,
    card_max_height: 560,
    card_radius: 16,
    card_top_fraction: 0.18,
    card_padding: 8,
    field_height: 56,
    field_radius: 12,
    row_height: 48,
    row_radius: 10,
    row_spacing: 2,
    icon_size: 32,
    title_size: 15,
    subtitle_size: 12,
    heading_size: 11,
    query_size: 20,
};

/// Action-menu dimensions in logical pixels, shared with the design preview.
pub const PANEL_METRICS: [(&str, u16); 9] = [
    ("width", 300),
    ("padding", 6),
    ("inset", 10),
    ("row-height", 34),
    ("row-radius", 8),
    ("gap", 2),
    ("header-height", 24),
    ("divider-gap", 5),
    ("filter-height", 36),
];

/// Look up a shared action-menu dimension.
#[must_use]
pub fn panel_metric(name: &str) -> f32 {
    f32::from(
        PANEL_METRICS
            .iter()
            .find(|(key, _)| *key == name)
            .expect("known panel metric")
            .1,
    )
}

/// The font stack, most preferred first.
///
/// Cantarell is GNOME's interface font; the rest are what a non-GNOME desktop
/// running this is likely to have. The browser surrogate is given the same
/// list so that text metrics are close enough for layout decisions to carry.
pub const FONT_STACK: &[&str] = &["Cantarell", "Adwaita Sans", "Inter", "Cantarell Regular"];

/// An Iced theme for an appearance.
#[must_use]
pub fn theme(appearance: Appearance) -> iced::Theme {
    let p = palette(appearance);

    iced::Theme::custom(
        format!("Compass {}", appearance.name()),
        iced::theme::Palette {
            background: p.surface.to_iced(),
            text: p.text.to_iced(),
            primary: p.accent.to_iced(),
            success: p.accent.to_iced(),
            warning: p.accent.to_iced(),
            danger: iced::Color::from_rgb8(0xe0, 0x1b, 0x24),
        },
    )
}

/// A dropdown that reads as part of the card (#239).
///
/// Iced's default pick-list chrome — near-square corners, a full-contrast
/// border, and colors off the generic ramp — sits visibly apart from the
/// launcher's rows. A filter instead takes the search field's fill and the
/// rows' corner radius, with no border, in every status: the launcher's rows
/// do not glow on hover either. The font is set at the call site, because a
/// pick list carries its own.
#[must_use]
pub fn dropdown(
    palette: Palette,
    _status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    iced::widget::pick_list::Style {
        text_color: palette.text.to_iced(),
        placeholder_color: palette.muted.to_iced(),
        handle_color: palette.muted.to_iced(),
        background: iced::Background::Color(palette.field.to_iced()),
        border: iced::Border {
            radius: f32::from(GEOMETRY.row_radius).into(),
            ..iced::Border::default()
        },
    }
}

/// An open dropdown's menu: the card's fill, row-radius corners, and the
/// selection colors the results list uses, so an open filter looks like the
/// list it filters. The hairline and soft shadow are the popover's own, the
/// way GNOME draws one over a window.
#[must_use]
pub fn dropdown_menu(palette: Palette) -> iced::widget::overlay::menu::Style {
    iced::widget::overlay::menu::Style {
        background: iced::Background::Color(palette.surface.to_iced()),
        border: iced::Border {
            color: palette.border.to_iced(),
            width: 1.0,
            radius: f32::from(GEOMETRY.row_radius).into(),
        },
        text_color: palette.text.to_iced(),
        selected_text_color: palette.selection_text.to_iced(),
        selected_background: iced::Background::Color(palette.selection.to_iced()),
        shadow: iced::Shadow {
            color: iced::Color {
                a: 0.25,
                ..iced::Color::BLACK
            },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
    }
}

/// The room around the card for its shadow, in logical pixels, on every
/// side. The window is the card plus this, so the shadow has to end inside it.
pub const SHADOW_PADDING: u16 = 24;
/// How far the card's drop shadow falls below it.
pub const SHADOW_OFFSET_Y: f32 = 6.0;
/// The blur radius for the card's drop shadow.
///
/// Iced fades a shadow out over `blur_radius` past the shape it is cast by, so
/// the shadow reaches `SHADOW_OFFSET_Y + SHADOW_BLUR` below the card and
/// `SHADOW_BLUR` to each side. It used to be a 16 px offset and a 32 px blur:
/// 48 px of shadow in 24 px of window, cut off by the surface's bottom edge in
/// a hard band wherever the card grew to its full height (#251).
pub const SHADOW_BLUR: f32 = 18.0;
/// How dark the card's drop shadow is where it meets the card.
pub const SHADOW_ALPHA: f32 = 0.35;

const _: () = assert!(
    SHADOW_OFFSET_Y + SHADOW_BLUR <= SHADOW_PADDING as f32,
    "the card's shadow must fade out inside the window"
);

/// The card's drop shadow: soft, falling a little below it, and wholly inside
/// [`SHADOW_PADDING`].
#[must_use]
pub fn card_shadow() -> iced::Shadow {
    iced::Shadow {
        color: iced::Color {
            a: SHADOW_ALPHA,
            ..iced::Color::BLACK
        },
        offset: iced::Vector::new(0.0, SHADOW_OFFSET_Y),
        blur_radius: SHADOW_BLUR,
    }
}

/// How opaque the card is when `tint` is on (#86).
///
/// 0.82 rather than something lower: the launcher is text over an arbitrary
/// wallpaper, and legibility is not negotiable for the thing bound to
/// Super+Space. Enough to read as translucent over a busy background, opaque
/// enough that body text keeps its contrast over a bright one.
pub const TINT_ALPHA: f32 = 0.82;
