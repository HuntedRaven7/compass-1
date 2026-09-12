//! POSIX locale strings as used by the desktop entry specification.
//!
//! Ported from `src/lib/xdgpp/xdgpp/locale/locale.hpp`.

use std::fmt;

/// The individual components a [`Locale`] may carry.
///
/// The encoding is deliberately *not* a component: the specification says the
/// encoding of a `LOCALE` key suffix must be ignored when matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Components(u8);

impl Components {
    /// The `lang` part. Always present (possibly empty).
    pub const LANG: Components = Components(1);
    /// The `_COUNTRY` part.
    pub const COUNTRY: Components = Components(1 << 1);
    /// The `@MODIFIER` part.
    pub const MODIFIER: Components = Components(1 << 2);

    /// Returns true if every component of `other` is also set in `self`.
    #[must_use]
    pub const fn contains(self, other: Components) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Components {
    type Output = Components;

    fn bitor(self, rhs: Components) -> Components {
        Components(self.0 | rhs.0)
    }
}

/// A parsed `lang_COUNTRY.ENCODING@MODIFIER` locale string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Locale {
    lang: String,
    country: Option<String>,
    encoding: Option<String>,
    modifier: Option<String>,
}

/// Parser state for [`Locale::parse`].
enum State {
    Lang,
    Country,
    Encoding,
    Modifier,
}

impl Locale {
    /// Parses a locale string. Parsing never fails: unexpected characters are
    /// simply dropped, mirroring the permissive C++ implementation.
    ///
    /// Parsing stops at the first `]`, so a raw `LOCALE` suffix may be passed
    /// directly.
    #[must_use]
    pub fn parse(data: &str) -> Locale {
        let mut state = State::Lang;
        let (mut lang, mut country, mut encoding, mut modifier) =
            (String::new(), String::new(), String::new(), String::new());

        for c in data.chars() {
            if c == ']' {
                break;
            }

            match state {
                State::Lang => match c {
                    '_' => state = State::Country,
                    '.' => state = State::Encoding,
                    '@' => state = State::Modifier,
                    _ if c.is_alphabetic() => lang.push(c),
                    _ => {}
                },
                State::Country => match c {
                    '.' => state = State::Encoding,
                    '@' => state = State::Modifier,
                    _ if c.is_alphabetic() => country.push(c),
                    _ => {}
                },
                State::Encoding => match c {
                    '@' => state = State::Modifier,
                    _ if c.is_alphanumeric() => encoding.push(c),
                    _ => {}
                },
                State::Modifier => {
                    if c.is_alphabetic() {
                        modifier.push(c);
                    }
                }
            }
        }

        Locale {
            lang,
            country: (!country.is_empty()).then_some(country),
            encoding: (!encoding.is_empty()).then_some(encoding),
            modifier: (!modifier.is_empty()).then_some(modifier),
        }
    }

    /// A suitable locale for the current process, derived from `LC_ALL`,
    /// `LC_MESSAGES` then `LANG`, as `setlocale(LC_MESSAGES, "")` would.
    #[must_use]
    pub fn system() -> Locale {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            match std::env::var(var) {
                Ok(value) if !value.is_empty() => return Locale::parse(&value),
                _ => {}
            }
        }

        Locale::parse("C")
    }

    #[must_use]
    pub fn lang(&self) -> &str {
        &self.lang
    }

    #[must_use]
    pub fn country(&self) -> Option<&str> {
        self.country.as_deref()
    }

    #[must_use]
    pub fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    #[must_use]
    pub fn modifier(&self) -> Option<&str> {
        self.modifier.as_deref()
    }

    /// The set of components this locale is made of. `LANG` is always set.
    #[must_use]
    pub fn flags(&self) -> Components {
        let mut flags = Components::LANG;

        if self.country.is_some() {
            flags = flags | Components::COUNTRY;
        }
        if self.modifier.is_some() {
            flags = flags | Components::MODIFIER;
        }

        flags
    }

    /// Matches only if `rhs` is made of exactly `components` and all of those
    /// components are equal to the ones of `self`.
    #[must_use]
    pub fn matches_only(&self, rhs: &Locale, components: Components) -> bool {
        if rhs.flags() != components {
            return false;
        }
        if components.contains(Components::LANG) && self.lang != rhs.lang {
            return false;
        }
        if components.contains(Components::COUNTRY) && self.country != rhs.country {
            return false;
        }
        if components.contains(Components::MODIFIER) && self.modifier != rhs.modifier {
            return false;
        }

        true
    }

    /// Scores `candidate` (a `LOCALE` key suffix) against this locale.
    ///
    /// Implements the matching order of the specification: a score of zero
    /// means "no match, ignore the value", higher is better.
    #[must_use]
    pub fn score(&self, candidate: &Locale) -> u8 {
        use Components as C;

        let flags = self.flags();

        if flags == C::LANG | C::COUNTRY | C::MODIFIER {
            if self.matches_only(candidate, C::LANG | C::COUNTRY | C::MODIFIER) {
                return 4;
            }
            if self.matches_only(candidate, C::LANG | C::COUNTRY) {
                return 3;
            }
            if self.matches_only(candidate, C::LANG | C::MODIFIER) {
                return 2;
            }
            if self.matches_only(candidate, C::LANG) {
                return 1;
            }
            return 0;
        }

        if flags == C::LANG | C::COUNTRY || flags == C::LANG | C::MODIFIER {
            if self.matches_only(candidate, flags) {
                return 2;
            }
            if self.matches_only(candidate, C::LANG) {
                return 1;
            }
            return 0;
        }

        if flags == C::LANG && self.matches_only(candidate, C::LANG) {
            return 1;
        }

        0
    }
}

impl From<&str> for Locale {
    fn from(value: &str) -> Locale {
        Locale::parse(value)
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lang)?;

        if let Some(country) = &self.country {
            write!(f, "_{country}")?;
        }
        if let Some(encoding) = &self.encoding {
            write!(f, ".{encoding}")?;
        }
        if let Some(modifier) = &self.modifier {
            write!(f, "@{modifier}")?;
        }

        Ok(())
    }
}
