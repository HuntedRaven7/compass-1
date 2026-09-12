//! Transliteration of non-Latin scripts to ASCII.
//!
//! Ported from `src/lib/fuzzy/include/fuzzy/normalize.hpp` (the `TRANSLIT_TABLE`
//! half of it). The diacritic-folding half of that header is *not* ported: the
//! Rust matcher delegates diacritic folding to [`nucleo_matcher::chars::normalize`],
//! which is derived from the same fzf table.
//!
//! The matcher is a subsequence matcher, so a spelling longer than the Latin one it
//! has to match rules the item out entirely, while a shorter one still matches at a
//! gap penalty: prefer under-expanding (щ -> "sh", not "shch"; х -> "h", not "kh").
//! `alt` covers sounds with two common Latin spellings, notably /k/ written "c" in
//! Calculator or Discord and "k" in Konsole or Kitty.

/// Which of the two Latin spellings to use for sounds that have more than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslitScheme {
    /// The default spelling (к -> "k").
    Primary,
    /// The alternate spelling where one exists (к -> "c"), else the primary.
    Alternate,
}

impl TranslitScheme {
    /// Both schemes, in the order the matcher should try them.
    pub const ALL: [TranslitScheme; 2] = [TranslitScheme::Primary, TranslitScheme::Alternate];
}

struct Entry {
    cp: char,
    ascii: &'static str,
    alt: &'static str,
}

const fn e(cp: char, ascii: &'static str) -> Entry {
    Entry { cp, ascii, alt: "" }
}

const fn ea(cp: char, ascii: &'static str, alt: &'static str) -> Entry {
    Entry { cp, ascii, alt }
}

/// Keyed by lowercase codepoint, except the Greek accented capitals whose
/// lowercase partners are not a fixed offset away (see [`fold_script_case`]).
/// Sorted by codepoint; [`transliterate_char`] binary-searches it.
static TRANSLIT_TABLE: &[Entry] = &[
    // Greek
    e('\u{0386}', "a"),
    e('\u{0388}', "e"),
    e('\u{0389}', "i"),
    e('\u{038A}', "i"),
    e('\u{038C}', "o"),
    e('\u{038E}', "u"),
    e('\u{038F}', "o"),
    e('\u{0390}', "i"),
    e('\u{03AA}', "i"),
    e('\u{03AB}', "u"),
    e('\u{03AC}', "a"),
    e('\u{03AD}', "e"),
    e('\u{03AE}', "i"),
    e('\u{03AF}', "i"),
    e('\u{03B0}', "u"),
    e('\u{03B1}', "a"),
    e('\u{03B2}', "b"),
    e('\u{03B3}', "g"),
    e('\u{03B4}', "d"),
    e('\u{03B5}', "e"),
    e('\u{03B6}', "z"),
    e('\u{03B7}', "i"),
    e('\u{03B8}', "th"),
    e('\u{03B9}', "i"),
    ea('\u{03BA}', "k", "c"),
    e('\u{03BB}', "l"),
    e('\u{03BC}', "m"),
    e('\u{03BD}', "n"),
    e('\u{03BE}', "x"),
    e('\u{03BF}', "o"),
    e('\u{03C0}', "p"),
    e('\u{03C1}', "r"),
    e('\u{03C2}', "s"),
    e('\u{03C3}', "s"),
    e('\u{03C4}', "t"),
    e('\u{03C5}', "u"),
    e('\u{03C6}', "f"),
    e('\u{03C7}', "h"),
    e('\u{03C8}', "ps"),
    e('\u{03C9}', "o"),
    e('\u{03CA}', "i"),
    e('\u{03CB}', "u"),
    e('\u{03CC}', "o"),
    e('\u{03CD}', "u"),
    e('\u{03CE}', "o"),
    // Cyrillic
    e('\u{0430}', "a"),
    e('\u{0431}', "b"),
    e('\u{0432}', "v"),
    e('\u{0433}', "g"),
    e('\u{0434}', "d"),
    e('\u{0435}', "e"),
    e('\u{0436}', "zh"),
    e('\u{0437}', "z"),
    e('\u{0438}', "i"),
    e('\u{0439}', "i"),
    ea('\u{043A}', "k", "c"),
    e('\u{043B}', "l"),
    e('\u{043C}', "m"),
    e('\u{043D}', "n"),
    e('\u{043E}', "o"),
    e('\u{043F}', "p"),
    e('\u{0440}', "r"),
    e('\u{0441}', "s"),
    e('\u{0442}', "t"),
    e('\u{0443}', "u"),
    e('\u{0444}', "f"),
    e('\u{0445}', "h"),
    e('\u{0446}', "c"),
    e('\u{0447}', "ch"),
    e('\u{0448}', "sh"),
    e('\u{0449}', "sh"),
    e('\u{044A}', ""),
    e('\u{044B}', "y"),
    e('\u{044C}', ""),
    e('\u{044D}', "e"),
    e('\u{044E}', "u"),
    e('\u{044F}', "a"),
    e('\u{0451}', "e"),
    e('\u{0452}', "d"),
    e('\u{0453}', "g"),
    e('\u{0454}', "e"),
    e('\u{0455}', "z"),
    e('\u{0456}', "i"),
    e('\u{0457}', "i"),
    e('\u{0458}', "i"),
    e('\u{0459}', "l"),
    e('\u{045A}', "n"),
    e('\u{045B}', "c"),
    e('\u{045C}', "k"),
    e('\u{045E}', "u"),
    e('\u{045F}', "d"),
    e('\u{0491}', "g"),
];

/// Case-folds Cyrillic and Greek codepoints to the key used by [`TRANSLIT_TABLE`].
fn fold_script_case(cp: char) -> char {
    const GREEK_SPECIAL_CASES: [(char, char); 9] = [
        ('\u{0386}', '\u{03AC}'),
        ('\u{0388}', '\u{03AD}'),
        ('\u{0389}', '\u{03AE}'),
        ('\u{038A}', '\u{03AF}'),
        ('\u{038C}', '\u{03CC}'),
        ('\u{038E}', '\u{03CD}'),
        ('\u{038F}', '\u{03CE}'),
        ('\u{03AA}', '\u{03CA}'),
        ('\u{03AB}', '\u{03CB}'),
    ];

    let shifted = |by: u32| char::from_u32(cp as u32 + by).unwrap_or(cp);
    match cp {
        '\u{0410}'..='\u{042F}' => return shifted(0x20),
        '\u{0400}'..='\u{040F}' => return shifted(0x50),
        '\u{0391}'..='\u{03A9}' => return shifted(0x20),
        '\u{03C2}' => return '\u{03C3}',
        '\u{0490}' => return '\u{0491}',
        _ => {}
    }

    for (upper, lower) in GREEK_SPECIAL_CASES {
        if cp == upper {
            return lower;
        }
    }
    cp
}

/// The ASCII spelling of a single non-Latin codepoint, or `None` if the
/// codepoint is not transliterable.
pub fn transliterate_char(cp: char, scheme: TranslitScheme) -> Option<&'static str> {
    let folded = fold_script_case(cp);
    let idx = TRANSLIT_TABLE
        .binary_search_by(|entry| entry.cp.cmp(&folded))
        .ok()?;
    let entry = &TRANSLIT_TABLE[idx];
    if scheme == TranslitScheme::Alternate && !entry.alt.is_empty() {
        Some(entry.alt)
    } else {
        Some(entry.ascii)
    }
}

/// Whether `text` contains at least one codepoint this module can transliterate.
pub fn needs_transliteration(text: &str) -> bool {
    text.chars()
        .any(|c| transliterate_char(c, TranslitScheme::Primary).is_some())
}

/// Transliterates `text` under `scheme`.
///
/// Returns `None` when nothing was transliterated, or when the result is empty
/// (an all-soft-sign input such as "ьъ") — mirroring the C++ `transliterate`,
/// whose callers use `nullopt` to mean "no extra variant to try".
pub fn transliterate(text: &str, scheme: TranslitScheme) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut changed = false;

    for c in text.chars() {
        match transliterate_char(c, scheme) {
            Some(ascii) => {
                out.push_str(ascii);
                changed = true;
            }
            None => out.push(c),
        }
    }

    if !changed || out.is_empty() {
        return None;
    }
    Some(out)
}
