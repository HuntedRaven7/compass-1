//! `vicinae.json`: defaults, partial files, error messages, and forward compatibility.

use compass_core::config::{
    DEFAULT_AUTO_UPDATE, DEFAULT_CLOSE_ON_FOCUS_LOSS, DEFAULT_HOTKEY, DEFAULT_MAX_RESULTS,
};
use compass_core::{Config, ConfigError};
use std::path::Path;

fn parse(json: &str) -> Config {
    Config::parse(json, Path::new("/test/vicinae.json")).expect("valid config")
}

fn assert_all_defaults(config: &Config) {
    assert_eq!(config.launcher().hotkey(), DEFAULT_HOTKEY);
    assert_eq!(
        config.launcher().close_on_focus_loss(),
        DEFAULT_CLOSE_ON_FOCUS_LOSS
    );
    assert_eq!(config.launcher().max_results(), DEFAULT_MAX_RESULTS);
    assert_eq!(config.extensions().auto_update(), DEFAULT_AUTO_UPDATE);
    assert!(config.extensions().installed().is_empty());
}

#[test]
fn the_default_config_is_a_working_config() {
    assert_all_defaults(&Config::default());
    // The documented defaults, spelled out so a change to one is a deliberate change to this
    // test rather than a silent behaviour shift.
    assert_eq!(
        (
            DEFAULT_HOTKEY,
            DEFAULT_CLOSE_ON_FOCUS_LOSS,
            DEFAULT_MAX_RESULTS,
            DEFAULT_AUTO_UPDATE,
        ),
        ("super+space", false, 50, true),
    );
}

#[test]
fn an_empty_file_produces_the_defaults() {
    for text in ["", "   ", "\n\n", "{}", "{ }"] {
        assert_all_defaults(&parse(text));
    }
}

#[test]
fn a_missing_file_produces_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::load_from(dir.path().join("vicinae.json")).unwrap();
    assert_all_defaults(&config);
}

#[test]
fn a_full_config_is_read() {
    let config = parse(
        r#"{
          "launcher": {
            "hotkey": "ctrl+space",
            "close_on_focus_loss": true,
            "max_results": 12
          },
          "extensions": {
            "auto_update": false,
            "installed": ["com.example.clock", "com.example.notes"]
          }
        }"#,
    );

    assert_eq!(config.launcher().hotkey(), "ctrl+space");
    assert!(config.launcher().close_on_focus_loss());
    assert_eq!(config.launcher().max_results(), 12);
    assert!(!config.extensions().auto_update());
    assert_eq!(
        config.extensions().installed(),
        ["com.example.clock", "com.example.notes"]
    );
}

#[test]
fn a_partial_config_fills_in_the_rest() {
    let config = parse(r#"{"launcher": {"max_results": 7}}"#);

    assert_eq!(config.launcher().max_results(), 7);
    assert_eq!(config.launcher().hotkey(), DEFAULT_HOTKEY);
    assert_eq!(
        config.launcher().close_on_focus_loss(),
        DEFAULT_CLOSE_ON_FOCUS_LOSS
    );
    assert_eq!(config.extensions().auto_update(), DEFAULT_AUTO_UPDATE);
    assert!(config.extensions().installed().is_empty());
}

#[test]
fn an_explicit_zero_is_not_treated_as_absent() {
    let config = parse(r#"{"launcher": {"max_results": 0}}"#);
    assert_eq!(config.launcher().max_results(), 0);
}

#[test]
fn an_explicitly_empty_installed_list_is_kept() {
    let config = parse(r#"{"extensions": {"installed": []}}"#);
    assert!(config.extensions().installed().is_empty());
    let json = config.to_json_pretty().unwrap();
    assert!(json.contains("\"installed\""), "{json}");
}

// --- Forward compatibility ---------------------------------------------------------------------

/// The whole point: an older build must be able to read and rewrite a config a newer build wrote
/// without silently deleting the settings it does not understand.
#[test]
fn unknown_fields_survive_a_round_trip() {
    let original = r#"{
  "launcher": {
    "hotkey": "ctrl+space",
    "theme": "tokyonight",
    "window": { "width": 780, "corner_radius": 12 }
  },
  "extensions": {
    "installed": ["com.example.clock"],
    "registry": "https://example.invalid/registry"
  },
  "telemetry": { "enabled": false },
  "schema_version": 4
}"#;

    let config = parse(original);

    // The known fields still read normally.
    assert_eq!(config.launcher().hotkey(), "ctrl+space");
    assert_eq!(config.extensions().installed(), ["com.example.clock"]);

    // The unknown ones are visible rather than lost.
    assert_eq!(config.launcher().unknown_fields()["theme"], "tokyonight");
    assert_eq!(config.launcher().unknown_fields()["window"]["width"], 780);
    assert_eq!(
        config.extensions().unknown_fields()["registry"],
        "https://example.invalid/registry"
    );
    assert_eq!(config.unknown_fields()["schema_version"], 4);
    assert_eq!(config.unknown_fields()["telemetry"]["enabled"], false);

    // And they come back out.
    let rewritten = config.to_json_pretty().unwrap();
    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    let after: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert_eq!(
        before, after,
        "round trip changed the document:\n{rewritten}"
    );
}

#[test]
fn unknown_fields_survive_an_edit_by_an_older_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vicinae").join("vicinae.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"launcher": {"max_results": 5, "future_thing": [1, 2, 3]}}"#,
    )
    .unwrap();

    let mut config = Config::load_from(&path).unwrap();
    config.launcher_mut().set_max_results(Some(99));
    config.save_to(&path).unwrap();

    let reloaded = Config::load_from(&path).unwrap();
    assert_eq!(reloaded.launcher().max_results(), 99);
    assert_eq!(
        reloaded.launcher().unknown_fields()["future_thing"],
        serde_json::json!([1, 2, 3])
    );
}

#[test]
fn writing_a_default_config_does_not_invent_settings_the_user_never_chose() {
    let json = Config::default().to_json_pretty().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value, serde_json::json!({}));
}

#[test]
fn clearing_a_field_restores_its_default() {
    let mut config = parse(r#"{"launcher": {"hotkey": "ctrl+space"}}"#);
    config.launcher_mut().set_hotkey(None);
    assert_eq!(config.launcher().hotkey(), DEFAULT_HOTKEY);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&config.to_json_pretty().unwrap()).unwrap(),
        serde_json::json!({})
    );
}

// --- Errors --------------------------------------------------------------------------------------

#[test]
fn malformed_json_names_the_problem_and_where_it_is() {
    let path = Path::new("/test/vicinae.json");
    let err = Config::parse("{\n  \"launcher\": {\n    \"hotkey\": ,\n  }\n}", path).unwrap_err();

    let ConfigError::Parse {
        line,
        column,
        ref message,
        ..
    } = err
    else {
        panic!("expected a parse error, got {err:?}");
    };
    assert_eq!(line, 3);
    assert!(column > 0);
    assert!(
        message.contains("expected value"),
        "unhelpful message: {message}"
    );

    let rendered = err.to_string();
    assert!(rendered.contains("/test/vicinae.json"), "{rendered}");
    assert!(rendered.contains("line 3"), "{rendered}");
    assert!(rendered.contains("expected value"), "{rendered}");
}

#[test]
fn a_wrongly_typed_field_is_a_clear_error() {
    let err = Config::parse(
        r#"{"launcher": {"max_results": "lots"}}"#,
        Path::new("/test/vicinae.json"),
    )
    .unwrap_err();

    let rendered = err.to_string();
    assert!(rendered.contains("invalid configuration"), "{rendered}");
    assert!(
        rendered.contains("invalid type") || rendered.contains("expected"),
        "{rendered}"
    );
}

#[test]
fn a_top_level_non_object_is_a_clear_error() {
    let err = Config::parse("[1, 2, 3]", Path::new("/test/vicinae.json")).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }), "{err:?}");
}

#[test]
fn an_unreadable_path_is_distinguished_from_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where a file is expected: exists, but cannot be read as one.
    let err = Config::load_from(dir.path()).unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }), "{err:?}");
}

#[test]
fn saving_creates_the_parent_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("vicinae.json");

    let mut config = Config::default();
    config.extensions_mut().set_auto_update(Some(false));
    config.save_to(&path).unwrap();

    assert!(path.is_file());
    assert!(!Config::load_from(&path).unwrap().extensions().auto_update());
    assert!(
        std::fs::read_to_string(&path).unwrap().ends_with('\n'),
        "the file should end with a newline"
    );
}

#[test]
fn saving_leaves_no_temporary_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vicinae.json");
    Config::default().save_to(&path).unwrap();

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["vicinae.json"]);
}
