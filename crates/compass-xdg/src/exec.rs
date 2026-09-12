//! Expansion of the `Exec` key into an argument vector.
//!
//! Ported from `src/lib/xdgpp/xdgpp/desktop-entry/exec.cpp`.

/// Expands an `Exec` value into an argument vector, applying the quoting rules
/// and the field codes of the desktop entry specification.
#[derive(Debug, Clone, Default)]
pub struct ExecParser<'a> {
    name: &'a str,
    icon: Option<&'a str>,
    entry_path: Option<&'a str>,
    force_append: bool,
}

enum State {
    Reset,
    FieldCode,
    Escaped,
    Quote,
    QuotedEscaped,
}

fn is_quote_char(c: char) -> bool {
    c == '"' || c == '\''
}

impl<'a> ExecParser<'a> {
    /// Creates a parser expanding `%c` to `name`.
    #[must_use]
    pub fn new(name: &'a str) -> ExecParser<'a> {
        ExecParser {
            name,
            icon: None,
            entry_path: None,
            force_append: false,
        }
    }

    /// Sets the value `%i` expands to. Without it, `%i` expands to nothing.
    #[must_use]
    pub fn with_icon(mut self, icon: Option<&'a str>) -> ExecParser<'a> {
        self.icon = icon;
        self
    }

    /// Sets the value `%k` expands to (the location of the desktop file).
    #[must_use]
    pub fn with_entry_path(mut self, path: Option<&'a str>) -> ExecParser<'a> {
        self.entry_path = path;
        self
    }

    /// If no field code expanded to the set of provided URIs, append all the
    /// URIs at the end of the command line, one argument per URI.
    ///
    /// Note that if a single URI field code such as `%f` or `%u` was expanded
    /// this won't append the remaining URIs: those are lost.
    #[must_use]
    pub fn with_force_append(mut self, force_append: bool) -> ExecParser<'a> {
        self.force_append = force_append;
        self
    }

    /// Expands `exec`, substituting the URI field codes with `uris`.
    #[must_use]
    pub fn parse(&self, exec: &str, uris: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        let mut part = String::new();
        let mut state = State::Reset;
        let mut quote_char = '\0';
        let mut uri_expanded = false;

        // Flushes the pending word so a multi valued field code does not get
        // glued to it.
        macro_rules! flush {
            () => {
                if !part.is_empty() {
                    args.push(std::mem::take(&mut part));
                }
            };
        }

        for ch in exec.chars() {
            match state {
                State::Reset => {
                    if is_quote_char(ch) {
                        state = State::Quote;
                        quote_char = ch;
                    } else if ch == '%' {
                        state = State::FieldCode;
                    } else if ch == '\\' {
                        state = State::Escaped;
                    } else if ch.is_whitespace() {
                        flush!();
                    } else {
                        part.push(ch);
                    }
                }
                State::FieldCode => {
                    match ch {
                        '%' => part.push('%'),
                        // Single URI field codes.
                        'f' | 'u' => {
                            uri_expanded = true;
                            if let Some(uri) = uris.first() {
                                part.push_str(uri);
                            }
                        }
                        // Multiple URI field codes.
                        'F' | 'U' => {
                            uri_expanded = true;
                            flush!();
                            args.extend(uris.iter().map(|uri| (*uri).to_owned()));
                        }
                        'i' => {
                            if let Some(icon) = self.icon {
                                flush!();
                                args.push("--icon".to_owned());
                                args.push(icon.to_owned());
                            }
                        }
                        'c' => part.push_str(self.name),
                        'k' => {
                            if let Some(path) = self.entry_path {
                                part.push_str(path);
                            }
                        }
                        // Deprecated (%d %D %n %N %v %m) and unknown field
                        // codes expand to nothing.
                        _ => {}
                    }
                    state = State::Reset;
                }
                State::Escaped => {
                    part.push(ch);
                    state = State::Reset;
                }
                State::Quote => {
                    if ch == '\\' {
                        state = State::QuotedEscaped;
                    } else if ch == quote_char {
                        state = State::Reset;
                    } else {
                        part.push(ch);
                    }
                }
                State::QuotedEscaped => {
                    part.push(ch);
                    state = State::Quote;
                }
            }
        }

        flush!();

        if !uri_expanded && self.force_append {
            args.extend(uris.iter().map(|uri| (*uri).to_owned()));
        }

        args
    }
}
