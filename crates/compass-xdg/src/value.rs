//! Value-type conversions for raw desktop entry values.
//!
//! Ported from `src/lib/xdgpp/xdgpp/desktop-entry/value.cpp`.

/// Maps an escape sequence letter to the character it stands for.
///
/// Unknown sequences yield `None`, in which case both the backslash and the
/// escaped character are dropped (this is what the C++ implementation does).
fn escape_char(c: char) -> Option<char> {
    match c {
        's' => Some(' '),
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        '\\' => Some('\\'),
        _ => None,
    }
}

/// Applies the `string` value-type escape rules: `\s \n \t \r \\`.
#[must_use]
pub fn as_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut escaped = false;

    for c in raw.chars() {
        if escaped {
            if let Some(c) = escape_char(c) {
                out.push(c);
            }
            escaped = false;
            continue;
        }

        if c == '\\' {
            escaped = true;
            continue;
        }

        out.push(c);
    }

    out
}

/// The `boolean` value type. Only the exact string `true` is true.
#[must_use]
pub fn as_bool(raw: &str) -> bool {
    raw == "true"
}

/// The `numeric` value type.
#[must_use]
pub fn as_number(raw: &str) -> Option<f64> {
    raw.trim().parse().ok()
}

/// Applies the `string(s)` value-type rules: values are separated by `;`, a
/// literal semicolon is written `\;`, and the remaining escape sequences are
/// applied per element.
///
/// A trailing empty element (the usual case, since well formed lists end with
/// a `;`) is dropped.
#[must_use]
pub fn as_string_list(raw: &str) -> Vec<String> {
    let mut list = Vec::new();
    let mut part = String::new();
    let mut escaped = false;

    for c in raw.chars() {
        if escaped {
            if c == ';' {
                part.push(';');
            } else {
                part.push('\\');
                part.push(c);
            }
            escaped = false;
            continue;
        }

        match c {
            '\\' => escaped = true,
            ';' => {
                list.push(as_string(&part));
                part.clear();
            }
            _ => part.push(c),
        }
    }

    if !part.is_empty() {
        list.push(as_string(&part));
    }

    list
}
