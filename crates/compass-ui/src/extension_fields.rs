//! The form fields that are not a line of text: dates, tags and files.
//!
//! What each sends back is what `@vicinae/api`'s `Form` hands its `onChange`
//! and `onSubmit`:
//!
//! - a date picker, a string `new Date()` parses — `YYYY-MM-DDTHH:MM:00`,
//!   with no zone, so JavaScript reads it as local time, as a person typing a
//!   date means it;
//! - a tag picker, the array of chosen values, in the options' order;
//! - a file picker, the array of chosen absolute paths, from the desktop's
//!   own file chooser (the XDG portal), which is also what lets a Flatpak
//!   hand an extension a file outside its sandbox.
//!
//! The date is typed rather than picked from a calendar, which Iced does not
//! have; the placeholder says the format, and an entry that does not parse
//! is kept as typed and not sent until it does.

use compass_extension_api::view::{DatePrecision, DropdownOption};
use iced::Element;
use iced::widget::{button, column, row, text, text_input};

use crate::message::Message;

/// The placeholder, and the format, for a precision.
#[must_use]
pub const fn date_format(precision: DatePrecision) -> &'static str {
    match precision {
        DatePrecision::Day => "YYYY-MM-DD",
        DatePrecision::Minute => "YYYY-MM-DD HH:MM",
    }
}

/// A value from the extension (RFC 3339, or anything with a date in front)
/// as the field shows it.
#[must_use]
pub fn show_date(value: &str, precision: DatePrecision) -> String {
    let date = value.get(..10).unwrap_or(value);
    match (precision, value.get(11..16)) {
        (DatePrecision::Minute, Some(time)) => format!("{date} {time}"),
        _ => date.to_owned(),
    }
}

/// What the person typed, as the value to send, or `None` while it is not a
/// date. `YYYY-MM-DD` always parses; `HH:MM` after it is taken at minute
/// precision and ignored at day precision.
#[must_use]
pub fn parse_date(typed: &str, precision: DatePrecision) -> Option<String> {
    let typed = typed.trim();
    let (date, time) = match typed.split_once([' ', 'T']) {
        Some((date, time)) => (date, Some(time.trim())),
        None => (typed, None),
    };
    let mut parts = date.split('-');
    let year: u32 = digits(parts.next()?, 4)?;
    let month: u32 = digits(parts.next()?, 2)?;
    let day: u32 = digits(parts.next()?, 2)?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in(year, month) {
        return None;
    }
    let (hour, minute) = match (precision, time) {
        (DatePrecision::Minute, Some(time)) => {
            let (hour, minute) = time.split_once(':')?;
            let (hour, minute): (u32, u32) = (digits(hour, 2)?, digits(minute, 2)?);
            if hour > 23 || minute > 59 {
                return None;
            }
            (hour, minute)
        }
        (DatePrecision::Day, _) | (DatePrecision::Minute, None) => (0, 0),
    };
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:00"
    ))
}

fn digits(part: &str, len: usize) -> Option<u32> {
    (part.len() == len && part.bytes().all(|b| b.is_ascii_digit()))
        .then(|| part.parse().ok())
        .flatten()
}

const fn days_in(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The values chosen after toggling `value` in `chosen`, in the options'
/// order.
#[must_use]
pub fn toggle_tag(options: &[DropdownOption], chosen: &[String], value: &str) -> Vec<String> {
    let on = !chosen.iter().any(|c| c == value);
    options
        .iter()
        .map(|option| option.value.as_str())
        .filter(|candidate| {
            if *candidate == value {
                on
            } else {
                chosen.iter().any(|c| c == candidate)
            }
        })
        .map(str::to_owned)
        .collect()
}

/// The strings in a JSON array value; empty for anything else.
#[must_use]
pub fn strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// A date field: typed, and sent once it parses.
pub fn date_field<'a>(
    name: &str,
    shown: String,
    precision: DatePrecision,
    font: iced::Font,
) -> Element<'a, Message> {
    let name = name.to_owned();
    text_input(date_format(precision), &shown)
        .font(font)
        .style(crate::adwaita::form_entry)
        .on_input(move |typed| Message::ExtensionDateEdited(name.clone(), typed))
        .padding(8)
        .into()
}

/// A tag picker: every option as a toggle, the chosen ones marked.
pub fn tag_field<'a>(
    name: &str,
    options: &[DropdownOption],
    chosen: &[String],
    font: iced::Font,
) -> Element<'a, Message> {
    let mut tags = row![].spacing(6);
    for option in options {
        let on = chosen.contains(&option.value);
        let label = if on {
            format!("✓ {}", option.title)
        } else {
            option.title.clone()
        };
        let next = toggle_tag(options, chosen, &option.value);
        let name = name.to_owned();
        let tag = button(text(label).font(font).size(12))
            .padding([4, 8])
            .style(if on {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Message::ExtensionFieldEdited(
                name,
                serde_json::Value::from(next),
            ));
        tags = tags.push(tag);
    }
    tags.wrap().into()
}

/// A file picker: what is chosen, and a button that asks the desktop.
pub fn file_field<'a>(
    name: &str,
    chosen: &[String],
    choice: FileChoice,
    font: iced::Font,
) -> Element<'a, Message> {
    let label = match (choice.directories, choice.files) {
        (true, false) => "Choose Folder…",
        _ => "Choose File…",
    };
    let mut entry = column![
        button(text(label).font(font).size(12))
            .padding([4, 8])
            .style(button::secondary)
            .on_press(Message::ExtensionChooseFiles {
                name: name.to_owned(),
                choice,
            })
    ]
    .spacing(4);
    for path in chosen {
        entry = entry.push(text(path.clone()).font(font).size(12));
    }
    entry.into()
}

/// What a file picker lets the person choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileChoice {
    /// More than one.
    pub multiple: bool,
    /// Directories.
    pub directories: bool,
    /// Files.
    pub files: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(value: &str) -> DropdownOption {
        DropdownOption {
            title: value.to_uppercase(),
            value: value.to_owned(),
            icon: None,
            keywords: Vec::new(),
        }
    }

    #[test]
    fn a_typed_date_becomes_a_local_timestamp_javascript_parses() {
        assert_eq!(
            parse_date("2026-09-24", DatePrecision::Day).as_deref(),
            Some("2026-09-24T00:00:00")
        );
        assert_eq!(
            parse_date(" 2026-09-24 14:05 ", DatePrecision::Minute).as_deref(),
            Some("2026-09-24T14:05:00")
        );
        assert_eq!(
            parse_date("2026-09-24 14:05", DatePrecision::Day).as_deref(),
            Some("2026-09-24T00:00:00"),
            "a day picker ignores a time"
        );
        assert_eq!(
            parse_date("2024-02-29", DatePrecision::Day).as_deref(),
            Some("2024-02-29T00:00:00")
        );
    }

    #[test]
    fn half_a_date_or_an_impossible_one_is_not_sent() {
        for typed in [
            "",
            "2026",
            "2026-09",
            "2026-9-24",
            "2026-13-01",
            "2025-02-29",
            "2026-04-31",
            "2026-09-24 25:00",
            "2026-09-24 12:5",
            "tomorrow",
        ] {
            assert_eq!(parse_date(typed, DatePrecision::Minute), None, "{typed:?}");
        }
    }

    #[test]
    fn an_extensions_value_is_shown_in_the_typed_format() {
        assert_eq!(
            show_date("2026-09-24T14:05:00.000Z", DatePrecision::Minute),
            "2026-09-24 14:05"
        );
        assert_eq!(
            show_date("2026-09-24T14:05:00.000Z", DatePrecision::Day),
            "2026-09-24"
        );
    }

    #[test]
    fn toggling_a_tag_keeps_the_options_order() {
        let options = [option("a"), option("b"), option("c")];
        let chosen = vec!["c".to_owned()];
        assert_eq!(toggle_tag(&options, &chosen, "a"), ["a", "c"]);
        assert_eq!(toggle_tag(&options, &["a".into(), "c".into()], "a"), ["c"]);
        assert!(toggle_tag(&options, &chosen, "c").is_empty());
    }
}
