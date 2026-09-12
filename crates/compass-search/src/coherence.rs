//! Coherence: does an alignment look like a match a human would recognize?
//!
//! Port of the classification the C++ backtracker performs in
//! `fzf::Matcher::match_ascii` (`src/lib/fuzzy/include/fuzzy/fzf.hpp`, phase 4),
//! whose result lands in `fzf::Result::coherent`.
//!
//! # What the C++ does
//!
//! While walking the DP matrix back from the best cell it maintains two flags
//! over the aligned region:
//!
//! * `boundary_inside` — set when any position *after* the first matched
//!   character and up to the last one carries a non-zero fzf position bonus
//!   `B[j]`. A non-zero bonus means "this position starts something": the start
//!   of the string, a character after whitespace / a delimiter / a non-word
//!   character, a camelCase hump, or a letter-to-digit transition. So the flag
//!   says *the match reaches across a word boundary*.
//! * `mid_word_run_start` — set when a run of consecutive matched characters
//!   begins at a position whose bonus is zero, i.e. in the middle of a word.
//!   The first matched character always begins a run, so it is checked too.
//!
//! and then
//!
//! ```text
//! coherent = !boundary_inside || !mid_word_run_start
//! ```
//!
//! Read positively: a match is incoherent exactly when it spans more than one
//! word *and* at least one of its runs starts mid-word. That admits substrings
//! (`"time"` in `"Run[time] Settings"` — one word, so no boundary inside),
//! in-word abbreviations (`"kbd"` in `"[K]ey[b]oar[d]"` — likewise), and
//! acronyms (`"sim"` in `"[S]tart [I]nput [M]ethod"` — several words, but every
//! run starts on a boundary), and rejects the launcher's classic false
//! positive, `"time"` in `"S[t]art [I]nput [Me]thod"`.
//!
//! # What this module does
//!
//! Exactly the same thing. `B[j]` depends only on the character classes of
//! `haystack[j - 1]` and `haystack[j]`, and the run structure depends only on
//! the matched indices — neither needs the DP matrix, so the flag is fully
//! reconstructible from `(haystack, indices)`, which is precisely what nucleo
//! hands back. This is a port, not a heuristic: it was validated against the
//! real C++ matcher (built from `src/lib/fuzzy`) over the whole ported test
//! corpus plus 3537 randomly generated haystack/needle pairs, feeding the C++
//! matcher's own positions in and comparing flags. Zero mismatches.
//!
//! Two deliberate deviations, both documented on the items below:
//!
//! * The bonus *values* are not reproduced, only their sign. The C++ code only
//!   ever tests `B[j] > 0` and `B[j] == 0` here, so a boolean
//!   ("is this position a boundary?") carries the same information.
//! * Character classes are Unicode-aware. The C++ classifies the *bytes* of its
//!   ASCII-folded haystack, so every byte of an unfolded non-Latin character
//!   (Cyrillic, CJK, …) classifies as `NonWord` and therefore reads as a
//!   boundary, making every such match trivially coherent. Classifying by
//!   `char` instead treats Cyrillic as letters, which is what the C++ would do
//!   if it could. See [`CharClass`].
//!
//! The residual divergence from C++ is not in this classifier but upstream of
//! it: nucleo and fzf-v2 do not always pick the *same* alignment, and coherence
//! is a property of an alignment.

/// The fzf character classes, in the C++ enum's order.
///
/// `Letter` covers alphabetic characters that are neither upper- nor lowercase
/// (scripts without case, such as CJK or Hebrew). Unlike the C++ this
/// classifies whole `char`s rather than bytes, so non-Latin text is classified
/// as letters instead of falling through to `NonWord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CharClass {
    White,
    NonWord,
    Delimiter,
    Lower,
    Upper,
    Letter,
    Number,
}

/// Characters the default fzf scheme treats as path/list delimiters.
///
/// `Matcher::m_delimiter_chars` for `Scheme::Default`. This crate only ever
/// uses the default scheme, so the path and history variants are not ported.
const DELIMITERS: &str = "/,:;|";

impl CharClass {
    fn of(c: char) -> Self {
        if c.is_ascii_lowercase() {
            Self::Lower
        } else if c.is_ascii_uppercase() {
            Self::Upper
        } else if c.is_ascii_digit() {
            Self::Number
        } else if c.is_whitespace() {
            Self::White
        } else if c.is_ascii() {
            if DELIMITERS.contains(c) {
                Self::Delimiter
            } else {
                Self::NonWord
            }
        } else if c.is_numeric() {
            Self::Number
        } else if c.is_lowercase() {
            Self::Lower
        } else if c.is_uppercase() {
            Self::Upper
        } else if c.is_alphabetic() {
            Self::Letter
        } else {
            Self::NonWord
        }
    }
}

/// Whether a character of class `class_` preceded by one of class `prev` sits
/// at a boundary, i.e. whether the C++ `bonus_for(prev, class_)` is non-zero.
///
/// The C++ returns the actual bonus (8, 9, 10 or 7 depending on which rule
/// fires); the coherence pass only ever compares it against zero, so this
/// returns the sign. Reading the C++ `bonus_for` top to bottom, the *only* way
/// to reach `return 0` is a word character (`Lower`/`Upper`/`Letter`/`Number`)
/// preceded by another word character, with neither the camelCase nor the
/// letter-to-digit rule firing.
fn is_boundary(prev: CharClass, class_: CharClass) -> bool {
    // Word character after whitespace, a delimiter, or a non-word character.
    if class_ > CharClass::NonWord
        && matches!(
            prev,
            CharClass::White | CharClass::Delimiter | CharClass::NonWord
        )
    {
        return true;
    }
    // camelCase hump, or a digit starting a number run.
    if (prev == CharClass::Lower && class_ == CharClass::Upper)
        || (prev != CharClass::Number && class_ == CharClass::Number)
    {
        return true;
    }
    // Any non-word or whitespace character is itself a boundary.
    matches!(
        class_,
        CharClass::NonWord | CharClass::Delimiter | CharClass::White
    )
}

/// Per-character boundary flags for `haystack`, indexed by char position.
///
/// The character before position 0 is treated as whitespace, mirroring
/// `Matcher::m_initial_char_class` for the default scheme. (The C++ starts its
/// bonus array one character *before* the first occurrence of the needle's
/// first character rather than at the string start, so every position it
/// actually inspects sees its real predecessor; position 0 is the only place
/// the synthetic predecessor is used, and there it is correct.)
fn boundary_flags(haystack: &str, out: &mut Vec<bool>) {
    out.clear();
    let mut prev = CharClass::White;
    for c in haystack.chars() {
        let class_ = CharClass::of(c);
        out.push(is_boundary(prev, class_));
        prev = class_;
    }
}

/// Whether the alignment of `indices` (ascending char indices into `haystack`)
/// is coherent, in the sense of `fzf::Result::coherent`.
///
/// Matches of zero or one character are always coherent: the C++ returns early
/// for a single-character needle, before the backtracking pass runs, leaving
/// `Result::coherent` at its `true` default.
pub fn is_coherent(haystack: &str, indices: &[u32]) -> bool {
    let mut flags = Vec::new();
    is_coherent_with(haystack, indices, &mut flags)
}

/// [`is_coherent`], reusing a caller-owned scratch buffer.
///
/// Scoring a query walks many haystacks in a row; the buffer keeps that
/// allocation-free, for the same reason [`Matcher`](crate::Matcher) holds its
/// nucleo scratch space.
pub(crate) fn is_coherent_with(haystack: &str, indices: &[u32], flags: &mut Vec<bool>) -> bool {
    if indices.len() <= 1 {
        return true;
    }

    boundary_flags(haystack, flags);
    let at = |i: u32| flags.get(i as usize).copied().unwrap_or(false);

    let first = indices[0];
    let last = indices[indices.len() - 1];

    // `boundary_inside`: any boundary strictly after the first matched
    // character, up to and including the last. This is the C++ walk's range: it
    // tests `B[j] > 0` at every column it steps through, and breaks out before
    // that test on the step that consumes the needle's first character.
    let boundary_inside = ((first + 1)..=last).any(at);
    if !boundary_inside {
        // Confined to a single word: any in-word run start is fine.
        return true;
    }

    // `mid_word_run_start`: some run of consecutive matched characters begins
    // off a boundary. The first matched character always begins a run.
    let mid_word_run_start =
        !at(first) || indices.windows(2).any(|w| w[1] != w[0] + 1 && !at(w[1]));

    !mid_word_run_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn check(haystack: &str, indices: &[u32], expected: bool) {
        assert_eq!(
            is_coherent(haystack, indices),
            expected,
            "{haystack:?} @ {indices:?}"
        );
    }

    #[test]
    fn short_matches_are_always_coherent() {
        check("Start Input Method", &[], true);
        check("Start Input Method", &[7], true);
        // Even a single character in the middle of a word.
        check("Start Input Method", &[15], true);
    }

    /// The exact alignments the C++ matcher produces for the coherent half of
    /// `TEST_CASE("match: coherence separates ...")`, read out of a harness
    /// built against `src/lib/fuzzy`.
    #[test]
    fn cpp_coherent_alignments() {
        check("Runtime Settings", &[3, 4, 5, 6], true); // "time", one word
        check("Keyboard", &[0, 3, 7], true); // "kbd", in-word abbreviation
        check("Start Input Method", &[0, 6, 12], true); // "sim", acronym
        check("Event Log", &[0, 1, 6, 7, 8], true); // "evlog"
        check("Firefox Developer Edition", &[0, 8, 9, 10], true); // "fdev"
        check("Café Bar", &[0, 1, 2, 5, 6], true); // "cafba"
        check("Play this game on Steam", &[18, 19, 20, 21, 22], true); // "steam"
    }

    /// Likewise for the incoherent half, plus the other alignments the C++
    /// harness reports as incoherent across the ported corpus.
    #[test]
    fn cpp_incoherent_alignments() {
        check("Play this game on Steam", &[5, 7, 12, 13], false); // "time"
        check("Start Input Method", &[4, 6, 12, 13], false); // "time"
        check(
            "An intelligent spaced-repetition memory training program",
            &[28, 29, 33, 34],
            false, // "time"
        );
        check("profile editor", &[3, 4, 5, 8], false); // "file"
        check(
            "OpenJDK Java 17 Console",
            &[6, 17, 18, 19, 20, 21, 22],
            false,
        ); // "konsole"
        check("Donate to vicinae", &[3, 10, 11], false); // "avi"
        check("Rofi.code-workspace", &[8, 14, 15], false); // "Esp"
        check("Reload Script Directories", &[7, 11, 20], false); // "Spo"
    }

    /// A match confined to one word never crosses a boundary, so nothing
    /// inside it can make it incoherent however scattered it is.
    #[test]
    fn matches_inside_one_word_are_coherent() {
        check("Minecraft", &[0, 4, 7, 8], true); // "mcft"
        check("Thunderbird", &[7, 8, 9, 10], true); // "bird"
        check("Firefox", &[0, 5, 6], true); // "fox"
        check("Packages", &[0, 3, 5], true); // "pkg"
        // The extreme case: first and last letter of a long word.
        check(
            "An intelligent spaced-repetition memory training program",
            &[3, 13],
            true,
        );
    }

    /// Every run pinned to a word boundary: acronyms and initialisms.
    #[test]
    fn boundary_anchored_runs_are_coherent() {
        check("Visual Studio Code", &[0, 7, 14], true); // "vsc"
        check("System Info Event Log", &[0, 7, 8, 9, 10], true); // "sinfo"
        check(
            "Browse the World Wide Web",
            &[11, 12, 13, 14, 15, 17, 18],
            true,
        ); // "worldwi"
        // A boundary-anchored run may be many characters long.
        check(
            "Access and organize files",
            &[11, 12, 13, 20, 21, 22, 23, 24],
            true,
        );
    }

    /// One mid-word run start is enough, even when every other run is anchored.
    #[test]
    fn one_mid_word_run_start_spoils_the_match() {
        // "Access and organize files": [o]rganize + fil[es] -- the second run
        // starts at 23, mid-word.
        check("Access and organize files", &[11, 23, 24], false);
        // ... and it is the *run start* that matters, not the run's length.
        check("Web Browser", &[0, 1, 2, 6, 7], false); // "webow"
    }

    /// Separators other than whitespace open a boundary too.
    #[test]
    fn delimiters_and_punctuation_open_boundaries() {
        // '-' is NonWord, '/' is a Delimiter: the letter after each is a
        // boundary, so these initialisms stay coherent.
        check("eos-update", &[0, 4], true); // "eu"
        check("usr/local/bin", &[0, 4, 10], true); // "ulb"
        // A run that starts mid-segment is not.
        check("eos-update", &[0, 5], false); // "ep"
        check("usr/local/bin", &[0, 6], false); // "uc"
    }

    /// camelCase humps and digit runs are boundaries, as in fzf.
    #[test]
    fn camel_case_and_digits_open_boundaries() {
        check("openJDK", &[0, 4], true); // "oj", camel hump
        check("Java17", &[0, 4], true); // "j1", letter -> digit
        check("openJDK", &[0, 5], false); // "od", mid-hump
        // Inside a digit run there is no boundary, so this is one "word".
        check("Java 17 Console", &[5, 6], true);
    }

    /// The false positive a launcher actually suffers from: a short query
    /// scattered across a long description.
    #[test]
    fn short_query_scattered_across_a_description_is_incoherent() {
        // "seal" over "Send and receive mail": S-e ... a ... l, last run at 20.
        check("Send and receive mail", &[0, 1, 5, 20], false);
        // "solid" over "Browse the World Wide Web".
        check("Browse the World Wide Web", &[4, 12, 14, 18, 19], false);
        // "fort" over "Perform arithmetic": "for" inside "Perform", then a
        // lone "t" from "arithmetic".
        check("Perform arithmetic", &[3, 4, 5, 11], false);
    }

    /// Unicode-aware classification: the deviation documented at the top of the
    /// module. Cyrillic is letters, so it behaves like Latin text rather than
    /// like the C++'s all-`NonWord` bytes.
    #[test]
    fn non_latin_scripts_are_classified_as_letters() {
        check("Привет мир", &[7, 8, 9], true); // "мир", one word
        // Scattered across two Cyrillic words with the second run mid-word:
        // incoherent, where the byte-classifying C++ would say coherent.
        check("Привет мир", &[1, 2, 8], false);
    }

    /// Diacritics do not change a character's class, so folding cannot change
    /// the verdict.
    #[test]
    fn diacritics_do_not_change_the_verdict() {
        check("Café Bar", &[0, 1, 2, 5, 6], true); // "cafba"
        check("Zürich Öffnen", &[0, 1, 7], true); // "zuo", both runs anchored
        check("Zürich Öffnen", &[0, 8, 9], false); // "zff", second run mid-word
    }
}
