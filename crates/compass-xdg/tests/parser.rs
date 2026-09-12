//! Coverage for the file format itself (groups, comments, escapes, value
//! types) and for the predicates that have no direct C++ test case.

use compass_xdg::value;
use compass_xdg::{DesktopEntry, Locale, ParseOptions, Reader};

fn parse(data: &str) -> DesktopEntry {
    DesktopEntry::parse_with(
        data,
        &ParseOptions {
            locale: Some(Locale::parse("C")),
            ..Default::default()
        },
    )
    .expect("entry should parse")
}

fn reader(data: &str) -> Reader {
    Reader::parse(data, Locale::parse("C"))
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let entry = parse(
        "# a leading comment\n\
         \n\
         [Desktop Entry]\n\
         # Name=Commented Out\n\
         Name=Real Name\n\
         \n\
         #Exec=nope\n\
         Exec=real\n",
    );

    assert_eq!(entry.name(), "Real Name");
    assert_eq!(entry.exec(), Some("real"));
}

#[test]
fn a_hash_is_only_a_comment_at_the_start_of_a_line() {
    let entry = parse("[Desktop Entry]\nName=colour #ff0000\n");

    assert_eq!(entry.name(), "colour #ff0000");
}

#[test]
fn blanks_around_the_separator_are_not_part_of_the_value() {
    let reader = reader("[Desktop Entry]\nName   =  \tspaced out  \t\nOther=x\n");
    let group = reader.group("Desktop Entry").unwrap();

    assert_eq!(group.raw("Name"), Some("spaced out"));
    assert_eq!(group.raw("Other"), Some("x"));
}

#[test]
fn crlf_line_endings_are_tolerated() {
    let entry = parse("[Desktop Entry]\r\nName=Windows\r\nExec=cmd\r\n");

    assert_eq!(entry.name(), "Windows");
    assert_eq!(entry.exec(), Some("cmd"));
}

#[test]
fn string_escape_sequences_are_applied() {
    let entry = parse("[Desktop Entry]\nName=a\\nb\\tc\\rd\\\\e\\sf\n");

    assert_eq!(entry.name(), "a\nb\tc\rd\\e f");
}

#[test]
fn unknown_escape_sequences_are_dropped() {
    // The C++ implementation drops both the backslash and the escaped char.
    assert_eq!(value::as_string(r"a\qb"), "ab");
    assert_eq!(value::as_string(r"trailing\"), "trailing");
}

#[test]
fn list_values_split_on_semicolons() {
    assert_eq!(value::as_string_list("a;b;c;"), ["a", "b", "c"]);
    assert_eq!(value::as_string_list("a;b;c"), ["a", "b", "c"]);
    assert_eq!(value::as_string_list(""), Vec::<String>::new());
    assert_eq!(value::as_string_list("a;;b;"), ["a", "", "b"]);
}

#[test]
fn escaped_semicolons_stay_inside_an_element() {
    assert_eq!(value::as_string_list(r"a\;b;c;"), ["a;b", "c"]);
    // A literal backslash right before a separator.
    assert_eq!(value::as_string_list(r"a\\;b;"), [r"a\", "b"]);
}

#[test]
fn escapes_apply_to_an_unterminated_last_element() {
    // Diverges from the C++ implementation, which forgets to unescape the
    // trailing element when the list has no terminating ';'.
    assert_eq!(value::as_string_list(r"a;b\sc"), ["a", "b c"]);
}

#[test]
fn boolean_values_are_strict() {
    assert!(value::as_bool("true"));
    assert!(!value::as_bool("True"));
    assert!(!value::as_bool("1"));
    assert!(!value::as_bool("yes"));
    assert!(!value::as_bool(""));

    let entry = parse("[Desktop Entry]\nName=n\nTerminal=True\nNoDisplay=1\nHidden=true\n");

    assert!(!entry.terminal());
    assert!(!entry.no_display());
    assert!(entry.hidden());
}

#[test]
fn numeric_values() {
    assert_eq!(value::as_number("1.5"), Some(1.5));
    assert_eq!(value::as_number("nonsense"), None);
}

#[test]
fn groups_are_exposed_in_file_order() {
    let reader =
        reader("[Desktop Entry]\nName=n\n[Desktop Action b]\nName=B\n[Desktop Action a]\nName=A\n");

    let names: Vec<&str> = reader
        .groups()
        .iter()
        .map(compass_xdg::Group::name)
        .collect();

    assert_eq!(
        names,
        ["Desktop Entry", "Desktop Action b", "Desktop Action a"]
    );
}

#[test]
fn a_redeclared_group_replaces_the_previous_one() {
    let reader = reader("[Desktop Entry]\nName=first\nIcon=icon\n[Desktop Entry]\nName=second\n");
    let group = reader.group("Desktop Entry").unwrap();

    assert_eq!(group.raw("Name"), Some("second"));
    assert_eq!(group.raw("Icon"), None);
    assert_eq!(reader.groups().len(), 1);
}

#[test]
fn a_duplicate_key_keeps_the_last_value() {
    let entry = parse("[Desktop Entry]\nName=first\nName=second\n");

    assert_eq!(entry.name(), "second");
}

#[test]
fn keys_before_any_group_are_dropped() {
    let reader = reader("Name=orphan\n[Desktop Entry]\nName=kept\n");

    assert_eq!(reader.groups().len(), 1);
    assert_eq!(
        reader.group("Desktop Entry").unwrap().raw("Name"),
        Some("kept")
    );
}

#[test]
fn an_unterminated_group_header_is_tolerated() {
    let reader = reader("[Desktop Entry\nName=n\n");

    assert!(reader.group("Desktop Entry").is_some());
}

#[test]
fn a_truncated_locale_suffix_does_not_swallow_the_next_line() {
    // The C++ reader loops forever here; we stop at the newline instead.
    let reader = reader("[Desktop Entry]\nName[fr\nIcon=kept\n");
    let group = reader.group("Desktop Entry").unwrap();

    assert_eq!(group.raw("Icon"), Some("kept"));
}

#[test]
fn a_key_with_no_separator_is_skipped() {
    let reader = reader("[Desktop Entry]\nName\nIcon=kept\n");
    let group = reader.group("Desktop Entry").unwrap();

    assert_eq!(group.raw("Name"), None);
    assert_eq!(group.raw("Icon"), Some("kept"));
}

#[test]
fn locale_fallback_chain() {
    let data = "[Desktop Entry]\n\
                Name=Base\n\
                Name[de]=de\n\
                Name[de_DE]=de_DE\n\
                Name[de@euro]=de_euro\n\
                Name[de_DE@euro]=de_DE_euro\n";

    let name = |locale: &str| {
        DesktopEntry::parse_with(
            data,
            &ParseOptions {
                locale: Some(Locale::parse(locale)),
                ..Default::default()
            },
        )
        .unwrap()
        .name()
        .to_owned()
    };

    assert_eq!(name("de_DE@euro"), "de_DE_euro");
    assert_eq!(name("de_DE"), "de_DE");
    assert_eq!(name("de@euro"), "de_euro");
    assert_eq!(name("de"), "de");
    assert_eq!(name("fr"), "Base");
}

#[test]
fn locale_fallback_uses_the_best_available_match() {
    let data = "[Desktop Entry]\nName=Base\nName[de]=de\n";

    let name = |locale: &str| {
        DesktopEntry::parse_with(
            data,
            &ParseOptions {
                locale: Some(Locale::parse(locale)),
                ..Default::default()
            },
        )
        .unwrap()
        .name()
        .to_owned()
    };

    // de_DE@euro, de_DE and de@euro all fall back to the plain `de` value.
    assert_eq!(name("de_DE@euro"), "de");
    assert_eq!(name("de_DE"), "de");
    assert_eq!(name("de@euro"), "de");
}

#[test]
fn the_declaration_order_of_localized_keys_does_not_matter() {
    let ascending = "[Desktop Entry]\nName=Base\nName[de]=de\nName[de_DE]=de_DE\n";
    let descending = "[Desktop Entry]\nName=Base\nName[de_DE]=de_DE\nName[de]=de\n";

    let opts = ParseOptions {
        locale: Some(Locale::parse("de_DE")),
        ..Default::default()
    };

    assert_eq!(
        DesktopEntry::parse_with(ascending, &opts).unwrap().name(),
        "de_DE"
    );
    assert_eq!(
        DesktopEntry::parse_with(descending, &opts).unwrap().name(),
        "de_DE"
    );
}

#[test]
fn the_locale_encoding_is_ignored_when_matching() {
    let data = "[Desktop Entry]\nName=Base\nName[en_US.UTF-8]=localized\n";
    let opts = ParseOptions {
        locale: Some(Locale::parse("en_US")),
        ..Default::default()
    };

    assert_eq!(
        DesktopEntry::parse_with(data, &opts).unwrap().name(),
        "localized"
    );
}

#[test]
fn localized_keys_other_than_name() {
    let data = "[Desktop Entry]\n\
                Name=n\n\
                GenericName=Browser\n\
                GenericName[fr]=Navigateur\n\
                Comment=Browse\n\
                Comment[fr]=Naviguer\n\
                Keywords=web;\n\
                Keywords[fr]=toile;\n\
                Icon=generic\n\
                Icon[fr]=francais\n";

    let entry = DesktopEntry::parse_with(
        data,
        &ParseOptions {
            locale: Some(Locale::parse("fr")),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(entry.generic_name(), Some("Navigateur"));
    assert_eq!(entry.comment(), Some("Naviguer"));
    assert_eq!(entry.keywords(), ["toile"]);
    assert_eq!(entry.icon(), Some("francais"));
}

#[test]
fn an_unknown_type_is_neither_application_nor_link() {
    let entry = parse("[Desktop Entry]\nName=n\nType=ServiceType\n");

    assert!(!entry.is_application());
    assert!(!entry.is_link());
    assert!(!entry.is_directory());
    assert_eq!(
        entry.entry_type(),
        &compass_xdg::EntryType::Other("ServiceType".to_owned())
    );
}

#[test]
fn a_missing_type_defaults_to_application() {
    assert!(parse("[Desktop Entry]\nName=n\n").is_application());
}

#[test]
fn should_show_honours_no_display_and_hidden() {
    assert!(parse("[Desktop Entry]\nName=n\n").should_show(&["GNOME"]));
    assert!(!parse("[Desktop Entry]\nName=n\nNoDisplay=true\n").should_show(&["GNOME"]));
    assert!(!parse("[Desktop Entry]\nName=n\nHidden=true\n").should_show(&["GNOME"]));
}

#[test]
fn should_show_honours_only_show_in() {
    let entry = parse("[Desktop Entry]\nName=n\nOnlyShowIn=KDE;GNOME;\n");

    assert!(entry.should_show(&["GNOME"]));
    assert!(entry.should_show(&["X-Foo", "KDE"]));
    assert!(!entry.should_show(&["XFCE"]));
    assert!(!entry.should_show(&[]));
}

#[test]
fn should_show_honours_not_show_in() {
    let entry = parse("[Desktop Entry]\nName=n\nNotShowIn=KDE;\n");

    assert!(entry.should_show(&["GNOME"]));
    assert!(entry.should_show(&[]));
    assert!(!entry.should_show(&["KDE"]));
    assert!(!entry.should_show(&["GNOME", "KDE"]));
}

#[test]
fn not_show_in_wins_over_only_show_in() {
    let entry = parse("[Desktop Entry]\nName=n\nOnlyShowIn=GNOME;\nNotShowIn=GNOME;\n");

    assert!(!entry.should_show(&["GNOME"]));
}

#[test]
fn desktop_matching_is_case_sensitive() {
    let entry = parse("[Desktop Entry]\nName=n\nOnlyShowIn=GNOME;\n");

    assert!(!entry.should_show(&["gnome"]));
}

#[test]
fn matches_desktop_ignores_no_display() {
    let entry = parse("[Desktop Entry]\nName=n\nNoDisplay=true\n");

    assert!(entry.matches_desktop(&["GNOME"]));
    assert!(!entry.should_show(&["GNOME"]));
}

#[test]
fn k_field_code_expands_to_the_entry_location() {
    let entry = DesktopEntry::parse_with(
        "[Desktop Entry]\nName=n\nExec=viewer %k\n",
        &ParseOptions {
            locale: Some(Locale::parse("C")),
            path: Some("/usr/share/applications/n.desktop".into()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(
        entry.expand_exec(),
        ["viewer", "/usr/share/applications/n.desktop"]
    );
}

#[test]
fn k_field_code_expands_to_nothing_without_a_location() {
    assert_eq!(
        parse("[Desktop Entry]\nName=n\nExec=viewer %k\n").expand_exec(),
        ["viewer"]
    );
}

#[test]
fn a_field_code_may_be_glued_to_the_rest_of_a_word() {
    // The C++ parser pushes the expansion as its own argument, reordering the
    // command line; we substitute in place instead.
    let entry = parse("[Desktop Entry]\nName=MyFile\nExec=prog --name=%c --file=%f\n");

    assert_eq!(
        entry.expand_exec_with(&["/tmp/x"], false, None),
        ["prog", "--name=MyFile", "--file=/tmp/x"]
    );
}

#[test]
fn a_launch_prefix_is_prepended_to_the_command_line() {
    let entry = parse("[Desktop Entry]\nName=n\nExec=firefox %u\n");

    assert_eq!(
        entry.expand_exec_with(&["https://x"], false, Some("systemd-run --user")),
        ["systemd-run", "--user", "firefox", "https://x"]
    );
}

#[test]
fn an_entry_without_exec_expands_to_nothing() {
    let entry = parse("[Desktop Entry]\nName=n\n");

    assert!(entry.expand_exec().is_empty());
    assert!(entry.expand_exec_with(&["a"], true, None).is_empty());
}

#[test]
fn exec_escapes_are_applied_before_field_codes() {
    // `\\s` in the file is a space in the value, which then splits the word.
    let entry = parse(
        r"[Desktop Entry]
Name=n
Exec=prog a\sb
",
    );

    assert_eq!(entry.exec(), Some("prog a b"));
    assert_eq!(entry.expand_exec(), ["prog", "a", "b"]);
}

#[test]
fn actions_expose_their_own_icon_and_name() {
    let entry = parse(
        "[Desktop Entry]\n\
         Name=Parent\n\
         Icon=parent-icon\n\
         Actions=one;two;missing;\n\
         \n\
         [Desktop Action one]\n\
         Name=One\n\
         Icon=one-icon\n\
         Exec=prog --one %c %i\n\
         \n\
         [Desktop Action two]\n\
         Name=Two\n\
         Exec=prog --two\n",
    );

    assert_eq!(entry.actions().len(), 2);
    assert_eq!(
        entry.actions()[0].expand_exec(),
        ["prog", "--one", "One", "--icon", "one-icon"]
    );
    assert_eq!(entry.actions()[1].icon(), None);
    assert_eq!(entry.actions()[1].expand_exec(), ["prog", "--two"]);
}

#[test]
fn action_names_are_localized() {
    let entry = DesktopEntry::parse_with(
        "[Desktop Entry]\n\
         Name=Parent\n\
         Actions=one;\n\
         [Desktop Action one]\n\
         Name=One\n\
         Name[fr]=Un\n\
         Exec=prog\n",
        &ParseOptions {
            locale: Some(Locale::parse("fr")),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(entry.actions()[0].name(), "Un");
}

#[test]
fn unlocalized_name_is_absent_when_only_a_localized_name_exists() {
    let entry = DesktopEntry::parse_with(
        "[Desktop Entry]\nName[fr]=Bonjour\n",
        &ParseOptions {
            locale: Some(Locale::parse("fr")),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(entry.name(), "Bonjour");
    assert_eq!(entry.unlocalized_name(), None);
}
