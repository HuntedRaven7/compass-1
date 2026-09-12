//! Tests against real `.desktop` files, copied verbatim from
//! `/usr/share/applications/` into `tests/fixtures/`.

use std::path::{Path, PathBuf};

use compass_xdg::{DesktopEntry, Locale, ParseOptions};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixtures() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("fixture directory should exist")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
        .collect();

    paths.sort();
    paths
}

fn load(name: &str) -> DesktopEntry {
    load_in(name, "C")
}

fn load_in(name: &str, locale: &str) -> DesktopEntry {
    DesktopEntry::from_file_with(
        fixture_dir().join(name),
        &ParseOptions {
            locale: Some(Locale::parse(locale)),
            ..Default::default()
        },
    )
    .unwrap_or_else(|err| panic!("{name} should parse: {err}"))
}

#[test]
fn every_fixture_parses_and_is_sane() {
    let paths = fixtures();

    assert_eq!(paths.len(), 8, "expected the full fixture set");

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let entry = DesktopEntry::from_file_with(
            &path,
            &ParseOptions {
                locale: Some(Locale::parse("C")),
                ..Default::default()
            },
        )
        .unwrap_or_else(|err| panic!("{name} should parse: {err}"));

        assert!(
            !entry.name().is_empty(),
            "{name} should have a non-empty Name"
        );
        assert!(entry.is_application(), "{name} should be an application");

        let exec = entry
            .exec()
            .unwrap_or_else(|| panic!("{name} should have an Exec"));
        assert!(!exec.is_empty(), "{name} should have a non-empty Exec");

        let argv = entry.expand_exec();
        assert!(
            !argv.is_empty(),
            "{name} should expand to at least a program name"
        );
        assert!(
            !argv[0].starts_with('%'),
            "{name} argv[0] should not be a field code"
        );
        assert!(
            !argv.iter().any(|arg| arg.contains('%')),
            "{name} should have no leftover field code: {argv:?}"
        );

        assert_eq!(entry.path(), Some(path.as_path()));

        for action in entry.actions() {
            assert!(!action.id().is_empty(), "{name} action should have an id");
            assert!(
                !action.name().is_empty(),
                "{name} action should have a name"
            );
            assert!(
                !action.expand_exec().is_empty(),
                "{name} action should have an Exec"
            );
        }
    }
}

#[test]
fn vim_fixture() {
    let entry = load("vim.desktop");

    assert_eq!(entry.name(), "Vim");
    assert_eq!(entry.generic_name(), Some("Text Editor"));
    assert_eq!(entry.icon(), Some("gvim"));
    assert_eq!(entry.exec(), Some("vim %F"));
    assert!(entry.terminal());
    assert!(entry.has_category("TextEditor"));
    assert!(entry.supports_mime("text/plain"));
    assert_eq!(
        entry.expand_exec_with(&["/tmp/a.txt", "/tmp/b.txt"], false, None),
        ["vim", "/tmp/a.txt", "/tmp/b.txt"]
    );
    assert!(entry.should_show(&["GNOME"]));
}

#[test]
fn vim_fixture_is_localized() {
    // The real file carries dozens of Name[..]/GenericName[..] keys.
    let entry = load_in("vim.desktop", "fr_FR.UTF-8");

    assert_eq!(entry.unlocalized_name(), Some("Vim"));
    assert_eq!(entry.generic_name(), Some("Éditeur de texte"));
    assert_eq!(entry.comment(), Some("Éditer des fichiers texte"));

    let german = load_in("vim.desktop", "de_DE.UTF-8");
    assert_eq!(german.generic_name(), Some("Texteditor"));
}

#[test]
fn libreoffice_fixture_has_actions() {
    let entry = load("libreoffice-startcenter.desktop");

    assert_eq!(entry.name(), "LibreOffice");
    assert_eq!(entry.exec(), Some("libreoffice %U"));
    assert!(!entry.no_display());
    assert!(entry.should_show(&["GNOME"]));

    let ids: Vec<&str> = entry
        .actions()
        .iter()
        .map(compass_xdg::DesktopAction::id)
        .collect();

    assert_eq!(ids, ["Writer", "Calc", "Impress", "Draw", "Base", "Math"]);

    let writer = entry.action("Writer").expect("Writer action");

    assert_eq!(writer.name(), "Writer");
    assert_eq!(writer.expand_exec(), ["libreoffice", "--writer"]);
}

#[test]
fn no_display_fixtures_are_not_shown() {
    for name in ["python3.12.desktop", "openjdk-21-java.desktop"] {
        let entry = load(name);

        assert!(entry.no_display(), "{name} sets NoDisplay=true");
        assert!(!entry.should_show(&["GNOME"]), "{name} should not be shown");
        assert!(
            entry.matches_desktop(&["GNOME"]),
            "{name} has no ShowIn restriction"
        );
    }
}

#[test]
fn python_fixture() {
    let entry = load("python3.12.desktop");

    assert_eq!(entry.name(), "Python (v3.12)");
    assert_eq!(entry.expand_exec(), ["/usr/bin/python3.12"]);
    assert!(entry.terminal());
    assert_eq!(entry.categories(), ["Development"]);
}

#[test]
fn a_fixture_read_through_parse_matches_the_one_read_from_disk() {
    let path = fixture_dir().join("vim.desktop");
    let data = std::fs::read_to_string(&path).unwrap();
    let opts = ParseOptions {
        locale: Some(Locale::parse("C")),
        ..Default::default()
    };

    let from_data = DesktopEntry::parse_with(&data, &opts).unwrap();
    let from_file = DesktopEntry::from_file_with(&path, &opts).unwrap();

    assert_eq!(from_data.name(), from_file.name());
    assert_eq!(from_data.exec(), from_file.exec());
    assert_eq!(from_data.path(), None);
    assert_eq!(from_file.path(), Some(path.as_path()));
}

/// A desktop file is supposed to be UTF-8, and real ones are not always.
///
/// `from_file` used to `read_to_string`, so a single Latin-1 byte anywhere in the file -- typically
/// an accented character in a `Comment` -- returned `Error::Io(InvalidData)` and the application
/// vanished from the launcher entirely, with no diagnostic the user could act on. Found while
/// building the application index in `compass-core`, which had to work around it by reading bytes
/// itself.
#[test]
fn a_non_utf8_byte_does_not_lose_the_whole_entry() {
    let dir = std::env::temp_dir().join(format!("compass-xdg-latin1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("latin1.desktop");

    // "Caf\xe9" -- Latin-1, not UTF-8.
    let mut bytes = b"[Desktop Entry]\nType=Application\nName=Cafe App\nComment=Caf".to_vec();
    bytes.push(0xe9);
    bytes.extend_from_slice(b"\nExec=cafe %U\n");
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        String::from_utf8(bytes).is_err(),
        "fixture must not be valid UTF-8"
    );

    let entry = compass_xdg::DesktopEntry::from_file(&path)
        .expect("a bad byte in Comment must not lose the application");

    // The fields that matter are intact; only the undecodable byte is replaced.
    assert_eq!(entry.name(), "Cafe App");
    assert_eq!(entry.exec(), Some("cafe %U"));
    assert!(entry.comment().unwrap().starts_with("Caf"));

    std::fs::remove_dir_all(&dir).ok();
}
