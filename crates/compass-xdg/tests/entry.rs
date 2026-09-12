//! Ported from `src/lib/xdgpp/tests/entry.cpp`.
//!
//! The C++ suite models failure as `isValid()` / `errorMessage()` on a
//! partially built object; this port models it as `Result`, so the "invalid"
//! cases assert on the error variant instead.
//!
//! PORT-DEFERRED (cases from `entry.cpp` intentionally not ported here):
//!
//! - "Create xdg terminal exec struct if category and main key are present"
//! - "Should not have terminal exec struct if no terminal emulator category"
//! - "Should not have terminal exec struct if no X-TerminalArgExec key"
//! - "Parse all xdg-terminal-exec keys"
//!   The four `X-TerminalArg*` cases cover the xdg-terminal-exec draft
//!   extension, which is outside the scope of this pass (the keys are still
//!   readable through `Reader`, they just have no typed accessor yet).
//! - "should be invalid if file does not exist" is ported, but the `DesktopFile`
//!   / desktop-file-id layer it belongs to (`file.hpp`, `fromId`, `relativeId`)
//!   is not: directory lookup and id computation are a separate pass.
//! - `src/lib/xdgpp/tests/{bookmark,env,file-uri,file,mime,special,
//!   xdg-terminal-exec}.cpp` are whole subsystems outside this crate's current
//!   scope.

use compass_xdg::{DesktopEntry, Error, Locale, ParseOptions, Reader};

fn parse(data: &str) -> DesktopEntry {
    DesktopEntry::parse(data).expect("entry should parse")
}

fn parse_in(data: &str, locale: &str) -> Result<DesktopEntry, Error> {
    DesktopEntry::parse_with(
        data,
        &ParseOptions {
            locale: Some(Locale::parse(locale)),
            ..Default::default()
        },
    )
}

#[test]
fn parse_firefox_desktop_file() {
    let firefox = r"
[Desktop Entry]
Version=1.0
Name=Mozilla Firefox (bin)
GenericName=Web Browser
Comment=Browse the Web
Exec=firefox-bin --name=firefox-bin %u
Icon=firefox-bin
Terminal=false
Type=Application
MimeType=application/pdf;application/vnd.mozilla.xul+xml;application/xhtml+xml;text/html;text/mml;text/xml;x-scheme-handler/http;x-scheme-handler/https;
StartupNotify=true
Categories=Network;WebBrowser;
Keywords=web;browser;internet;
Actions=new-window;new-private-window;
StartupWMClass=firefox
";

    let file = parse(firefox);

    assert!(file.is_application());
    assert_eq!(file.icon(), Some("firefox-bin"));
    assert_eq!(file.name(), "Mozilla Firefox (bin)");
    assert!(file.supports_mime("application/pdf"));
    assert_eq!(file.comment(), Some("Browse the Web"));
    assert_eq!(file.generic_name(), Some("Web Browser"));
    assert_eq!(file.startup_wm_class(), Some("firefox"));
    assert!(!file.hidden());
    assert!(!file.no_display());
    assert_eq!(file.version(), Some("1.0"));

    // web;browser;internet
    assert_eq!(file.keywords(), ["web", "browser", "internet"]);
    assert_eq!(file.categories(), ["Network", "WebBrowser"]);
    // Declared actions with no matching group are dropped.
    assert!(file.actions().is_empty());
}

#[test]
fn should_choose_the_localized_entry_as_name() {
    let data = r"
[Desktop Entry]
Name=Unlocalized Name
Name[en_US]=Name US
Name[en_CA.utf8]=Name CA
Name[en_GB.utf8]=Name GB
Name[fr.utf8]=Name FR
Name[fr_CA.utf8]=Name FR_CA
";

    assert_eq!(parse_in(data, "en_US.utf8").unwrap().name(), "Name US");
    assert_eq!(parse_in(data, "en_CA").unwrap().name(), "Name CA");
    assert_eq!(parse_in(data, "fr").unwrap().name(), "Name FR");
    assert_eq!(parse_in(data, "es_ES").unwrap().name(), "Unlocalized Name");
}

#[test]
fn should_be_invalid_if_entry_has_no_name() {
    let file = DesktopEntry::parse(
        r"
[Desktop Entry]
Icon=Stuff
Exec=some-exec
	",
    );

    assert!(matches!(file, Err(Error::MissingName)));
}

#[test]
fn should_be_invalid_if_file_does_not_exist() {
    let file = DesktopEntry::from_file("/fake/ass/path");

    assert!(matches!(file, Err(Error::Io { .. })));
}

#[test]
fn should_be_invalid_if_data_is_empty() {
    assert!(matches!(
        DesktopEntry::parse(""),
        Err(Error::MissingMainGroup)
    ));
}

#[test]
fn should_skip_invalid_entries() {
    let data = r"
[Desktop Entry]
Name: MyName
Comment=MyComment
Answer  42
Keywords=test;fun
  ";

    // `Name: MyName` has no '=' separator, so the whole line is skipped and the
    // required Name key ends up missing.
    assert!(matches!(DesktopEntry::parse(data), Err(Error::MissingName)));

    // The surrounding keys are still parsed.
    let reader = Reader::parse(data, Locale::parse("C"));
    let group = reader.group("Desktop Entry").unwrap();

    assert_eq!(group.string("Comment").as_deref(), Some("MyComment"));
    assert_eq!(group.string_list("Keywords"), ["test", "fun"]);
    assert_eq!(group.raw("Name"), None);
    assert_eq!(group.raw("Answer"), None);
}

#[test]
fn should_parse_categories() {
    let file = parse(
        r"
[Desktop Entry]
Name=Test
Categories=1;2;3;4
  ",
    );

    assert_eq!(file.categories(), ["1", "2", "3", "4"]);
}

#[test]
fn handle_copy() {
    let file = parse(
        r"
[Desktop Entry]
Name=Test
Comment=Some comment
  ",
    );
    let copy = file.clone();

    assert_eq!(copy.name(), file.name());
    assert_eq!(copy.comment(), file.comment());
}

#[test]
fn should_ignore_keys_not_within_a_group() {
    let file = DesktopEntry::parse(
        r"
Name=Test
Comment=Some comment
  ",
    );

    assert!(matches!(file, Err(Error::MissingMainGroup)));
}

#[test]
fn should_parse_simple_unquoted_exec() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec=program test --flag true --other-flag false
  ",
    );

    assert!(file.exec().is_some());
    assert_eq!(
        file.expand_exec(),
        ["program", "test", "--flag", "true", "--other-flag", "false"]
    );
}

#[test]
fn should_parse_one_big_quoted_exec() {
    let file = parse(
        r#"
[Desktop Entry]
Name=MyFile
Exec = "program test --flag true --other-flag false"
  "#,
    );

    assert_eq!(
        file.expand_exec(),
        ["program test --flag true --other-flag false"]
    );
}

#[test]
fn should_handle_escaped_quotes_inside_quoted_exec() {
    let file = parse(
        r#"
[Desktop Entry]
Name=MyFile
Exec = program --name "This is \\"quoted\\""
  "#,
    );

    assert_eq!(
        file.expand_exec(),
        ["program", "--name", "This is \"quoted\""]
    );
}

#[test]
fn should_concatenate_consecutive_quoted_strings() {
    let file = parse(
        r#"
[Desktop Entry]
Name=MyFile
Exec = program """this is"" quoted"""
  "#,
    );

    assert_eq!(file.expand_exec(), ["program", "this is quoted"]);
}

#[test]
fn should_handle_backslash_escaping_in_quoted_string() {
    let file = parse(
        r#"
[Desktop Entry]
Name=MyFile
Exec = program "\\\\hello\\\\"
  "#,
    );

    assert_eq!(file.expand_exec(), ["program", r"\hello\"]);
}

#[test]
fn should_ignore_empty_concatenation_of_quoted_strings() {
    let file = parse(
        r#"
[Desktop Entry]
Name=MyFile
Exec = program """""""""""""" true_arg
  "#,
    );

    assert_eq!(file.expand_exec(), ["program", "true_arg"]);
}

#[test]
fn handle_many_arguments() {
    let file = parse(
        "
[Desktop Entry]
Name=MyFile
Exec = a  b   c   d   e    f      g   
  ",
    );

    assert_eq!(file.expand_exec(), ["a", "b", "c", "d", "e", "f", "g"]);
}

#[test]
fn handle_percent_escaping_in_exec_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %%u %%%% 100%%
  ",
    );

    assert_eq!(file.expand_exec(), ["firefox", "%u", "%%", "100%"]);
}

#[test]
fn field_code_should_be_stripped_if_no_uri_is_provided() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %u
  ",
    );

    assert_eq!(file.expand_exec(), ["firefox"]);
}

#[test]
fn u_field_code_should_expand_to_provided_uri() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %u
  ",
    );

    assert_eq!(
        file.expand_exec_with(&["expanded value"], false, None),
        ["firefox", "expanded value"]
    );
}

#[test]
fn i_field_code_should_expand_to_icon() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Icon=/path/to/icon.png
Exec = firefox %i
  ",
    );

    assert_eq!(
        file.expand_exec(),
        ["firefox", "--icon", "/path/to/icon.png"]
    );
}

#[test]
fn i_field_code_should_expand_to_nothing_if_there_is_no_icon() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %i some_arg
  ",
    );

    assert_eq!(file.expand_exec(), ["firefox", "some_arg"]);
}

#[test]
fn c_field_code_should_expand_to_translated_name() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox --name %c
  ",
    );

    assert_eq!(file.expand_exec(), ["firefox", "--name", "MyFile"]);
}

#[test]
fn big_f_field_code_should_expand_to_a_list_of_files() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %F --private-window
  ",
    );

    assert_eq!(
        file.expand_exec_with(&["file1.png", "file2.png", "file3.png"], false, None),
        [
            "firefox",
            "file1.png",
            "file2.png",
            "file3.png",
            "--private-window"
        ]
    );
}

#[test]
fn big_u_field_code_should_expand_to_a_list_of_uris() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %U --private-window
  ",
    );

    assert_eq!(
        file.expand_exec_with(
            &["file://file1.png", "file://file2.png", "file://file3.png"],
            false,
            None
        ),
        [
            "firefox",
            "file://file1.png",
            "file://file2.png",
            "file://file3.png",
            "--private-window"
        ]
    );
}

#[test]
fn deprecated_field_codes_should_be_removed() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %d %D %n --private-window %v %m
  ",
    );

    assert_eq!(
        file.expand_exec_with(&["file://file1.png"], false, None),
        ["firefox", "--private-window"]
    );
}

#[test]
fn parse_desktop_actions() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
Actions = open-private;

[Desktop Action open-private]
Name=Open In Private Window
Exec=firefox %U
Icon=firefox
  ",
    );

    assert_eq!(file.actions().len(), 1);

    let action = &file.actions()[0];

    assert_eq!(action.id(), "open-private");
    assert_eq!(action.name(), "Open In Private Window");
    assert_eq!(action.exec(), Some("firefox %U"));
    assert_eq!(action.icon(), Some("firefox"));
    assert_eq!(
        action.expand_exec_with(&["file://test"], false, None)[1],
        "file://test"
    );
    assert!(file.action("open-private").is_some());
    assert!(file.action("nope").is_none());
}

#[test]
fn should_not_parse_action_not_listed_in_actions_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox

[Desktop Action open-private]
Name=Open In Private Window
Exec=firefox %U
Icon=firefox
  ",
    );

    assert!(file.actions().is_empty());
}

#[test]
fn should_support_single_main_window_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
SingleMainWindow = true

  ",
    );

    assert!(file.single_main_window());
}

#[test]
fn should_support_try_exec_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
TryExec = try-firefox

  ",
    );

    assert_eq!(file.try_exec(), Some("try-firefox"));
}

#[test]
fn should_support_startup_wm_class_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
StartupWMClass = firefox-class

  ",
    );

    assert_eq!(file.startup_wm_class(), Some("firefox-class"));
}

#[test]
fn should_support_terminal_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=Neovim
Exec = nvim
Terminal = true

  ",
    );

    assert!(file.terminal());
}

#[test]
fn should_support_path_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
Path = /opt/firefox-wd

  ",
    );

    assert_eq!(
        file.working_directory(),
        Some(std::path::Path::new("/opt/firefox-wd"))
    );
}

#[test]
fn should_support_only_show_in_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
OnlyShowIn = Gnome;KDE;
",
    );

    assert_eq!(file.only_show_in(), ["Gnome", "KDE"]);
}

#[test]
fn should_support_not_show_in_key() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
NotShowIn  = Gnome;KDE;
",
    );

    assert_eq!(file.not_show_in(), ["Gnome", "KDE"]);
}

#[test]
fn should_force_append_one_uri_if_no_field_code_was_expanded() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
",
    );

    assert_eq!(
        file.expand_exec_with(&["https://example.com"], true, None),
        ["firefox", "https://example.com"]
    );
}

#[test]
fn should_force_append_many_uris_if_no_field_code_was_expanded() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox
",
    );

    assert_eq!(
        file.expand_exec_with(&["1", "2", "3"], true, None),
        ["firefox", "1", "2", "3"]
    );
}

#[test]
fn should_not_force_append_many_uris_if_a_field_code_was_expanded() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Exec = firefox %f
",
    );

    assert_eq!(
        file.expand_exec_with(&["1", "2", "3"], true, None),
        ["firefox", "1"]
    );
}

#[test]
fn should_parse_url_for_link_entries() {
    let file = parse(
        r"
[Desktop Entry]
Name=MyFile
Type=Link
URL=https://vicinae.com
",
    );

    assert!(file.is_link());
    assert_eq!(file.url(), Some("https://vicinae.com"));
}

#[test]
fn should_flag_a_link_entry_without_an_url_as_invalid() {
    let file = DesktopEntry::parse(
        r"
[Desktop Entry]
Name=MyFile
Type=Link
",
    );

    assert!(matches!(file, Err(Error::MissingUrl)));
}

#[test]
fn unlocalized_name() {
    let file = parse_in(
        r"
[Desktop Entry]
Name=wechat
Name[zh_CN]=微信
Exec=/usr/bin/wechat %U
Icon=/usr/share/icons/hicolor/256x256/apps/wechat.png
Type=Application
Comment=Wechat Desktop
Comment[zh_CN]=微信桌面版
",
        "zh_CN",
    )
    .unwrap();

    assert_eq!(file.unlocalized_name(), Some("wechat"));
    assert_eq!(file.name(), "微信");
    assert_eq!(file.comment(), Some("微信桌面版"));
}
