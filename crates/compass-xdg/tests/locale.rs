//! Ported from `src/lib/xdgpp/tests/locale.cpp`.

use compass_xdg::locale::{Components, Locale};

#[test]
fn should_parse_lang() {
    assert_eq!(Locale::parse("en").to_string(), "en");
}

#[test]
fn should_parse_lang_country() {
    assert_eq!(Locale::parse("en_US").to_string(), "en_US");
}

#[test]
fn should_parse_lang_country_encoding() {
    assert_eq!(Locale::parse("en_US.utf8").to_string(), "en_US.utf8");
}

#[test]
fn should_parse_lang_country_encoding_modifier() {
    assert_eq!(
        Locale::parse("en_US.utf8@latin").to_string(),
        "en_US.utf8@latin"
    );
}

#[test]
fn should_parse_lang_encoding() {
    assert_eq!(Locale::parse("en.utf8").to_string(), "en.utf8");
}

#[test]
fn should_parse_lang_modifier() {
    assert_eq!(Locale::parse("en@latin").to_string(), "en@latin");
}

#[test]
fn lang_country_encoding_should_match_lang_country() {
    assert!(Locale::parse("en_US.utf8").matches_only(
        &Locale::parse("en_US"),
        Components::COUNTRY | Components::LANG
    ));
}

// Additional coverage beyond the C++ suite.

#[test]
fn should_expose_every_component() {
    let locale = Locale::parse("de_DE.UTF-8@euro");

    assert_eq!(locale.lang(), "de");
    assert_eq!(locale.country(), Some("DE"));
    // '-' is not alphanumeric, so it is dropped by the C++ state machine too.
    assert_eq!(locale.encoding(), Some("UTF8"));
    assert_eq!(locale.modifier(), Some("euro"));
}

#[test]
fn parsing_stops_at_closing_bracket() {
    assert_eq!(Locale::parse("fr_FR]garbage").to_string(), "fr_FR");
}

#[test]
fn flags_ignore_the_encoding() {
    assert_eq!(
        Locale::parse("en_US.utf8").flags(),
        Components::LANG | Components::COUNTRY
    );
    assert_eq!(Locale::parse("en").flags(), Components::LANG);
    assert_eq!(
        Locale::parse("en@latin").flags(),
        Components::LANG | Components::MODIFIER
    );
}

#[test]
fn matches_only_requires_exactly_the_requested_components() {
    let want = Locale::parse("de_DE@euro");

    // rhs carries a modifier we did not ask about.
    assert!(!want.matches_only(&Locale::parse("de_DE@euro"), Components::LANG));
    assert!(want.matches_only(&Locale::parse("de"), Components::LANG));
}

#[test]
fn scoring_follows_the_specification_order() {
    use Components as C;

    let want = Locale::parse("de_DE@euro");

    assert_eq!(want.flags(), C::LANG | C::COUNTRY | C::MODIFIER);
    assert_eq!(want.score(&Locale::parse("de_DE@euro")), 4);
    assert_eq!(want.score(&Locale::parse("de_DE")), 3);
    assert_eq!(want.score(&Locale::parse("de@euro")), 2);
    assert_eq!(want.score(&Locale::parse("de")), 1);
    assert_eq!(want.score(&Locale::parse("fr")), 0);
    assert_eq!(want.score(&Locale::parse("de_AT")), 0);
}

#[test]
fn scoring_never_widens_the_request() {
    let want = Locale::parse("de");

    assert_eq!(want.score(&Locale::parse("de")), 1);
    // A more specific candidate never matches a less specific request.
    assert_eq!(want.score(&Locale::parse("de_DE")), 0);
    assert_eq!(want.score(&Locale::parse("de@euro")), 0);
}
