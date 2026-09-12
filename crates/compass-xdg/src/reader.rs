//! Low level reader for the desktop entry file format.
//!
//! Ported from `src/lib/xdgpp/xdgpp/desktop-entry/reader.cpp` and `group.cpp`.
//!
//! This level of abstraction only understands groups, keys and values. Use
//! [`crate::DesktopEntry`] for the typed, high level view.

use std::collections::HashMap;

use crate::locale::Locale;
use crate::value;

const LF: char = '\n';

/// A localized value together with the locale it was tagged with.
#[derive(Debug, Clone)]
struct Localized {
    value: String,
    locale: Locale,
}

#[derive(Debug, Clone, Default)]
struct Entry {
    /// The value of the unsuffixed key, if it was ever set.
    value: Option<String>,
    /// The best scoring localized value seen so far.
    localized: Option<Localized>,
}

/// A `[Group Name]` section of a desktop entry file.
#[derive(Debug, Clone)]
pub struct Group {
    name: String,
    entries: HashMap<String, Entry>,
}

impl Group {
    fn new(name: String) -> Group {
        Group {
            name,
            entries: HashMap::new(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The raw value for `key`, without any escape rule applied. Leading and
    /// trailing blanks are never part of a value, even in raw form.
    ///
    /// The localized value is preferred when one was retained for this locale.
    #[must_use]
    pub fn raw(&self, key: &str) -> Option<&str> {
        let entry = self.entries.get(key)?;

        if let Some(localized) = &entry.localized {
            return Some(&localized.value);
        }

        entry.value.as_deref()
    }

    /// The raw value of the unsuffixed `key`, ignoring any localized variant.
    #[must_use]
    pub fn raw_unlocalized(&self, key: &str) -> Option<&str> {
        self.entries.get(key)?.value.as_deref()
    }

    /// The value of `key` as a `string`, localized when possible.
    #[must_use]
    pub fn string(&self, key: &str) -> Option<String> {
        self.raw(key).map(value::as_string)
    }

    /// The value of the unsuffixed `key` as a `string`.
    #[must_use]
    pub fn unlocalized_string(&self, key: &str) -> Option<String> {
        self.raw_unlocalized(key).map(value::as_string)
    }

    /// The value of `key` as a `boolean`. Absent keys are false.
    #[must_use]
    pub fn bool(&self, key: &str) -> bool {
        self.raw(key).is_some_and(value::as_bool)
    }

    /// The value of `key` as a `numeric`.
    #[must_use]
    pub fn number(&self, key: &str) -> Option<f64> {
        self.raw(key).and_then(value::as_number)
    }

    /// The value of `key` as a `string(s)` list. Absent keys yield an empty
    /// list.
    #[must_use]
    pub fn string_list(&self, key: &str) -> Vec<String> {
        self.raw(key).map(value::as_string_list).unwrap_or_default()
    }

    /// Every key name present in this group, in unspecified order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// A parsed desktop entry file, as a sequence of groups.
#[derive(Debug, Clone)]
pub struct Reader {
    groups: Vec<Group>,
    index: HashMap<String, usize>,
    locale: Locale,
}

impl Reader {
    /// Parses `data` resolving localized keys against `locale`.
    #[must_use]
    pub fn parse(data: &str, locale: Locale) -> Reader {
        let mut parser = Parser {
            data: data.chars().collect(),
            cursor: 0,
            reader: Reader {
                groups: Vec::new(),
                index: HashMap::new(),
                locale,
            },
            current: None,
        };

        parser.run();
        parser.reader
    }

    /// The group named `name`, if any.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.index.get(name).map(|i| &self.groups[*i])
    }

    /// Every group, in file order.
    #[must_use]
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The locale localized keys were resolved against.
    #[must_use]
    pub fn locale(&self) -> &Locale {
        &self.locale
    }
}

struct Parser {
    data: Vec<char>,
    cursor: usize,
    reader: Reader,
    current: Option<usize>,
}

enum State {
    Reset,
    Comment,
}

fn is_group_header_char(c: char) -> bool {
    c != '[' && c != ']' && c != LF
}

fn is_inline_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{b}' | '\r' | '\u{c}')
}

fn is_key_char(c: char) -> bool {
    c != '=' && c != LF && c != '[' && !c.is_whitespace()
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.data.get(self.cursor).copied()
    }

    fn consume(&mut self) -> Option<char> {
        let c = self.peek();
        self.cursor += 1;
        c
    }

    fn consume_if(&mut self, c: char) {
        if self.peek() == Some(c) {
            self.cursor += 1;
        }
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(is_inline_space) {
            self.cursor += 1;
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            if c == LF {
                break;
            }
            self.cursor += 1;
        }
    }

    fn run(&mut self) {
        let mut state = State::Reset;

        while let Some(c) = self.peek() {
            match state {
                State::Reset => {
                    if c == '#' {
                        state = State::Comment;
                        self.cursor += 1;
                    } else if c == '[' {
                        self.parse_group_header();
                    } else if is_key_char(c) {
                        self.parse_entry();
                    } else {
                        self.cursor += 1;
                    }
                }
                State::Comment => {
                    if c == LF {
                        state = State::Reset;
                    }
                    self.cursor += 1;
                }
            }
        }
    }

    // [Desktop Entry]
    fn parse_group_header(&mut self) {
        let mut name = String::new();

        self.cursor += 1;

        while let Some(c) = self.peek() {
            if !is_group_header_char(c) {
                break;
            }
            name.push(c);
            self.cursor += 1;
        }

        // A missing ']' is tolerated, as in the C++ implementation.
        self.consume_if(']');

        // A redeclared group replaces the previous one entirely.
        let index = match self.reader.index.get(&name) {
            Some(index) => {
                self.reader.groups[*index] = Group::new(name);
                *index
            }
            None => {
                let index = self.reader.groups.len();
                self.reader.index.insert(name.clone(), index);
                self.reader.groups.push(Group::new(name));
                index
            }
        };

        self.current = Some(index);
    }

    fn parse_key(&mut self) -> String {
        let mut key = String::new();

        while let Some(c) = self.peek() {
            if !is_key_char(c) {
                break;
            }
            key.push(c);
            self.cursor += 1;
        }

        key
    }

    // lang_COUNTRY.ENCODING@MODIFIER
    fn parse_raw_locale(&mut self) -> String {
        let mut locale = String::new();

        self.consume_if('[');
        while let Some(c) = self.peek() {
            if c == ']' || c == LF {
                break;
            }
            locale.push(c);
            self.cursor += 1;
        }
        self.consume_if(']');

        locale
    }

    fn parse_raw_value(&mut self) -> String {
        let mut value = String::new();

        while let Some(c) = self.peek() {
            if c == LF {
                break;
            }
            value.push(c);
            self.cursor += 1;
        }

        value.trim_end().to_string()
    }

    fn parse_entry(&mut self) {
        let key = self.parse_key();

        self.skip_space();

        let locale = (self.peek() == Some('[')).then(|| Locale::parse(&self.parse_raw_locale()));

        // If we don't get the expected '=' separator we just skip the current
        // line. When the separator we got *is* the end of the line there is
        // nothing left to skip: the C++ reader unconditionally skips ahead here
        // and eats the following line.
        let separator = self.consume();

        if separator != Some('=') {
            if matches!(separator, Some(c) if c != LF) {
                self.skip_line();
            }
            return;
        }

        self.skip_space();

        let value = self.parse_raw_value();

        let Some(index) = self.current else {
            return;
        };

        let Some(locale) = locale else {
            self.reader.groups[index]
                .entries
                .entry(key)
                .or_default()
                .value = Some(value);
            return;
        };

        let score = self.reader.locale.score(&locale);

        if score == 0 {
            return;
        }

        let current_score = self.reader.groups[index]
            .entries
            .get(&key)
            .and_then(|entry| entry.localized.as_ref())
            .map(|localized| self.reader.locale.score(&localized.locale));

        // On a tie the last declaration wins, as in the C++ implementation.
        if current_score.is_some_and(|current| current > score) {
            return;
        }

        self.reader.groups[index]
            .entries
            .entry(key)
            .or_default()
            .localized = Some(Localized { value, locale });
    }
}
