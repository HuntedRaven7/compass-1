//! Adwaita-style controls for Settings (#252), first-run, and launcher
//! forms.
//!
//! Iced's built-in styles are its own: square, bordered buttons filled with
//! the primary colour, a switch with a dark knob, inputs with a hairline frame.
//! Next to GNOME's own preferences windows they look foreign. These follow
//! libadwaita 1.x instead: 6 px corners, flat neutral fills that are the text
//! colour at a low opacity, accent fills only where Adwaita uses them (the
//! suggested action, a switch that is on), a 2 px accent ring on a focused
//! entry, and rows grouped in boxed lists with 12 px corners and separators.
//!
//! Every colour comes from the [`Palette`], so light and dark and the user's
//! theme still apply: `selection` is Adwaita's `accent_bg_color` (fills) and
//! `selection_text` its `accent_fg_color`, while `accent` is `accent_color`,
//! the one for focus rings. The launcher's own rows keep their styles in
//! [`crate::design`].

use iced::widget::button as btn;
use iced::widget::{container, pick_list, rule, text_input, toggler};
use iced::{Background, Border, Color, Shadow};

use crate::design::Palette;

/// A button's and an entry's corner radius.
pub const CONTROL_RADIUS: f32 = 6.0;
/// The padding that makes a 13 px label a 34 px tall button or dropdown.
pub const CONTROL_PADDING: [f32; 2] = [8.5, 14.0];
/// The same height for an entry, with Adwaita's 9 px inset for its text.
pub const ENTRY_PADDING: [f32; 2] = [8.5, 9.0];
/// A switch's height; Iced draws it twice as wide.
pub const SWITCH_SIZE: f32 = 26.0;
/// A boxed list's corner radius.
pub const LIST_RADIUS: f32 = 12.0;
/// The padding inside a boxed list's row: Adwaita's 8 px, less the room a
/// control keeps for its focus ring.
pub const ROW_PADDING: [f32; 2] = [8.0 - FOCUS_RING_ROOM, 12.0];
/// The tallest control in a row with its focus ring's room, which sets the
/// row's least height: a 34 px control, its ring and the row's padding make
/// Adwaita's 50 px action row.
pub const ROW_MIN_CONTENT: f32 = 34.0 + 2.0 * FOCUS_RING_ROOM;
/// The gap between controls, and between a group's title and its list.
pub const SPACING: f32 = 12.0;
/// A focus ring's width.
pub const FOCUS_RING_WIDTH: f32 = 2.0;
/// The gap between a focus ring and its control, so a ring in the accent
/// still shows around a control filled with it (a switch that is on).
pub const FOCUS_RING_GAP: f32 = 2.0;
/// What a ring takes on each side of its control.
pub const FOCUS_RING_ROOM: f32 = FOCUS_RING_WIDTH + FOCUS_RING_GAP;

/// `alpha(currentColor, a)`: the palette's text at opacity `a`.
fn ink(palette: Palette, a: f32) -> Color {
    Color {
        a,
        ..palette.text.to_iced()
    }
}

/// `a` moved `t` of the way to `b`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// A flat button: no border, a neutral fill that darkens as it is hovered
/// and pressed (`button` in libadwaita).
#[must_use]
pub fn button(palette: Palette, status: btn::Status) -> btn::Style {
    let (fill, text) = match status {
        btn::Status::Active => (0.10, 1.0),
        btn::Status::Hovered => (0.15, 1.0),
        btn::Status::Pressed => (0.30, 1.0),
        btn::Status::Disabled => (0.05, 0.0),
    };
    btn::Style {
        background: Some(Background::Color(ink(palette, fill))),
        // A disabled button says why in its label ("Installed"), so its
        // label stays readable: the secondary text colour, not a faded one.
        text_color: if text > 0.0 {
            ink(palette, text)
        } else {
            palette.muted.to_iced()
        },
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A `.flat` button: no fill until it is hovered or pressed. Disabled, it is
/// only its faded label, which is how a done state (Installed) reads.
#[must_use]
pub fn flat_button(palette: Palette, status: btn::Status) -> btn::Style {
    let (fill, text) = match status {
        btn::Status::Active => (0.0, 1.0),
        btn::Status::Hovered => (0.07, 1.0),
        btn::Status::Pressed => (0.16, 1.0),
        btn::Status::Disabled => (0.0, 0.5),
    };
    btn::Style {
        background: (fill > 0.0).then(|| Background::Color(ink(palette, fill))),
        text_color: ink(palette, text),
        ..button(palette, btn::Status::Active)
    }
}

/// A small pill label beside a title (a store's name, say): the neutral
/// fill with fully rounded ends.
#[must_use]
pub fn badge(palette: Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(ink(palette, 0.08))),
        border: Border {
            radius: 9.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The one button a page asks the user to press (`.suggested-action`): the
/// accent fill, lighter on hover and darker when pressed.
#[must_use]
pub fn suggested_button(palette: Palette, status: btn::Status) -> btn::Style {
    let accent = palette.selection.to_iced();
    let on_accent = palette.selection_text.to_iced();
    let (background, text) = match status {
        btn::Status::Active => (accent, on_accent),
        btn::Status::Hovered => (mix(accent, on_accent, 0.1), on_accent),
        btn::Status::Pressed => (mix(accent, Color::BLACK, 0.2), on_accent),
        btn::Status::Disabled => (
            Color { a: 0.5, ..accent },
            Color {
                a: 0.5,
                ..on_accent
            },
        ),
    };
    btn::Style {
        background: Some(Background::Color(background)),
        text_color: text,
        ..button(palette, btn::Status::Active)
    }
}

/// A switch: a pill-shaped track, the neutral fill when off and the accent
/// when on, with a round knob in the colour drawn on the accent.
#[must_use]
pub fn switch(palette: Palette, status: toggler::Status) -> toggler::Style {
    let (on, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let accent = palette.selection.to_iced();
    let knob = palette.selection_text.to_iced();
    let track = match (on, hovered) {
        (true, false) => accent,
        (true, true) => mix(accent, knob, 0.1),
        (false, false) => palette.control().to_iced(),
        (false, true) => mix(palette.control().to_iced(), palette.text.to_iced(), 0.15),
    };
    let fade = |colour: Color| {
        if disabled {
            Color {
                a: colour.a * 0.5,
                ..colour
            }
        } else {
            colour
        }
    };
    toggler::Style {
        background: Background::Color(fade(track)),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Background::Color(fade(knob)),
        foreground_border_width: 1.0,
        foreground_border_color: fade(Color {
            a: 0.12,
            ..Color::BLACK
        }),
        text_color: Some(palette.text.to_iced()),
        border_radius: None,
        padding_ratio: 3.0 / SWITCH_SIZE,
    }
}

/// A launcher form entry with the shared control radius and Iced's existing
/// colours, border, and interaction states.
#[must_use]
pub fn form_entry(theme: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.border.radius = CONTROL_RADIUS.into();
    style
}

/// An entry: the neutral fill, a 1 px frame in the control colour so the
/// field can be found without it (WCAG 1.4.11), and a 2 px accent ring while
/// it has the focus.
#[must_use]
pub fn entry(palette: Palette, status: text_input::Status) -> text_input::Style {
    let (fill, focused) = match status {
        text_input::Status::Active | text_input::Status::Disabled => (0.10, false),
        text_input::Status::Hovered => (0.13, false),
        text_input::Status::Focused { .. } => (0.10, true),
    };
    let accent = palette.accent.to_iced();
    text_input::Style {
        background: Background::Color(ink(palette, fill)),
        border: Border {
            color: if focused {
                accent
            } else {
                palette.control().to_iced()
            },
            width: if focused { FOCUS_RING_WIDTH } else { 1.0 },
            radius: CONTROL_RADIUS.into(),
        },
        icon: palette.muted.to_iced(),
        placeholder: palette.muted.to_iced(),
        value: palette.text.to_iced(),
        selection: Color { a: 0.3, ..accent },
    }
}

/// A dropdown: drawn as a button, with its chevron in the text colour.
#[must_use]
pub fn dropdown(palette: Palette, status: pick_list::Status) -> pick_list::Style {
    let fill = match status {
        pick_list::Status::Active => 0.10,
        pick_list::Status::Hovered => 0.15,
        pick_list::Status::Opened { .. } => 0.30,
    };
    pick_list::Style {
        text_color: palette.text.to_iced(),
        placeholder_color: palette.muted.to_iced(),
        handle_color: palette.text.to_iced(),
        background: Background::Color(ink(palette, fill)),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
    }
}

/// A launcher form dropdown with the shared control radius and Iced's
/// existing colours, border, and interaction states.
#[must_use]
pub fn form_dropdown(theme: &iced::Theme, status: pick_list::Status) -> pick_list::Style {
    let mut style = pick_list::default(theme, status);
    style.border.radius = CONTROL_RADIUS.into();
    style
}

/// The focus ring (`outline: 2px solid accent`, offset outward): a 2 px
/// line in the accent, which the palette holds at 4.5:1 against every
/// background, around a control with the keyboard, and nothing around one
/// without. `radius` is the control's own, which the ring follows.
#[must_use]
pub fn focus_ring(palette: Palette, focused: bool, radius: f32) -> container::Style {
    container::Style {
        border: Border {
            color: if focused {
                palette.accent.to_iced()
            } else {
                Color::TRANSPARENT
            },
            width: FOCUS_RING_WIDTH,
            radius: (radius + FOCUS_RING_ROOM).into(),
        },
        ..container::Style::default()
    }
}

/// `control` with room for its focus ring, drawn when `focused`. The room
/// is kept either way, so a control does not move when it takes the focus.
pub fn ringed<'a, Message: 'a>(
    control: impl Into<iced::Element<'a, Message>>,
    palette: Palette,
    focused: bool,
    radius: f32,
) -> iced::widget::Container<'a, Message> {
    container(control)
        .padding(FOCUS_RING_ROOM)
        .style(move |_: &iced::Theme| focus_ring(palette, focused, radius))
}

/// A boxed list's fill: Adwaita's `card_bg_color` ([`Palette::card`]).
#[must_use]
pub fn card(palette: Palette) -> Color {
    palette.card().to_iced()
}

/// A boxed list (`.boxed-list`, an `AdwPreferencesGroup`'s rows): the card
/// fill, a hairline, and 12 px corners.
#[must_use]
pub fn boxed_list(palette: Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(card(palette))),
        border: Border {
            color: palette.border.to_iced(),
            width: 1.0,
            radius: LIST_RADIUS.into(),
        },
        ..container::Style::default()
    }
}

/// The separator between a boxed list's rows.
#[must_use]
pub fn separator(palette: Palette) -> rule::Style {
    rule::Style {
        color: palette.border.to_iced(),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

/// A sidebar row (`.navigation-sidebar`): the selected one takes the neutral
/// fill, as in GNOME Settings, and the rest are bare.
#[must_use]
pub fn sidebar_row(palette: Palette, selected: bool) -> container::Style {
    container::Style {
        background: selected.then(|| Background::Color(ink(palette, 0.10))),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{DARK, LIGHT};

    #[test]
    fn the_card_is_white_over_light_and_lifted_over_dark() {
        assert_eq!(card(LIGHT), LIGHT.field.to_iced());
        let dark = card(DARK);
        assert!(dark.r > DARK.surface.to_iced().r, "{dark:?}");
        assert!(dark.r < 0.25, "a lift, not a light card: {dark:?}");
    }

    #[test]
    fn only_the_suggested_action_and_an_on_switch_take_the_accent() {
        let accent = Some(Background::Color(LIGHT.selection.to_iced()));
        assert_ne!(button(LIGHT, btn::Status::Active).background, accent);
        assert_eq!(
            suggested_button(LIGHT, btn::Status::Active).background,
            accent
        );
        let on = switch(LIGHT, toggler::Status::Active { is_toggled: true });
        let off = switch(LIGHT, toggler::Status::Active { is_toggled: false });
        assert_eq!(Some(on.background), accent);
        assert_ne!(Some(off.background), accent);
    }

    #[test]
    fn a_flat_button_is_bare_until_hovered() {
        assert_eq!(flat_button(DARK, btn::Status::Active).background, None);
        assert_eq!(flat_button(DARK, btn::Status::Disabled).background, None);
        assert!(flat_button(DARK, btn::Status::Hovered).background.is_some());
        assert!(button(DARK, btn::Status::Active).background.is_some());
    }

    #[test]
    fn a_focused_control_has_a_two_pixel_ring_that_reads_on_every_background() {
        for palette in [LIGHT, DARK] {
            let ring = focus_ring(palette, true, CONTROL_RADIUS);
            assert!((ring.border.width - 2.0).abs() < f32::EPSILON);
            assert_eq!(ring.border.color, palette.accent.to_iced());
            for background in palette.backgrounds() {
                let ratio = compass_core::contrast::contrast_ratio(
                    palette.accent.into(),
                    background.into(),
                );
                assert!(
                    ratio >= 3.0,
                    "{palette:?}: ring {ratio:.2} on {background:?}"
                );
            }
            let idle = focus_ring(palette, false, CONTROL_RADIUS);
            assert_eq!(idle.border.color, Color::TRANSPARENT);
        }
    }

    #[test]
    fn an_entry_is_framed_and_a_focused_one_has_an_accent_ring() {
        let idle = entry(DARK, text_input::Status::Active);
        assert!((idle.border.width - 1.0).abs() < f32::EPSILON);
        assert_eq!(idle.border.color, DARK.control().to_iced());
        let focused = entry(DARK, text_input::Status::Focused { is_hovered: false });
        assert!((focused.border.width - FOCUS_RING_WIDTH).abs() < f32::EPSILON);
        assert_eq!(focused.border.color, DARK.accent.to_iced());
    }

    #[test]
    fn launcher_form_controls_share_the_adwaita_radius() {
        let theme = iced::Theme::Dark;
        assert_eq!(
            form_entry(&theme, text_input::Status::Active).border.radius,
            CONTROL_RADIUS.into()
        );
        assert_eq!(
            form_dropdown(&theme, pick_list::Status::Active)
                .border
                .radius,
            CONTROL_RADIUS.into()
        );
    }
}
