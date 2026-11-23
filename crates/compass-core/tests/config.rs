//! `compass.json`: defaults, partial files, error messages, and forward compatibility.

use compass_core::config::{
    DEFAULT_AUTO_UPDATE, DEFAULT_CLOSE_ON_FOCUS_LOSS, DEFAULT_COLOR_SCHEME, DEFAULT_HOTKEY,
    DEFAULT_MAX_RESULTS,
};
use compass_core::{Config, ConfigError};
use std::path::Path;

fn parse(json: &str) -> Config {
    Config::parse(json, Path::new("/test/compass.json")).expect("valid config")
}

#[test]
fn root_settings_use_upstream_ids_and_preserve_provider_preferences() {
    let input = serde_json::json!({
        "providers": {
            "applications": {
                "enabled": false,
                "preferences": {"future": [1, 2]},
                "entrypoints": {
                    "org.example.Editor": {
                        "enabled": true, "alias": "write", "shortcut": "ctrl+e",
                        "preferences": {"mode": "custom"}, "future": 42
                    }
                }
            },
            "unknown.extension": {"custom": true}
        },
        "favorites": ["applications:org.example.Editor"],
        "fallbacks": ["files:search"]
    });
    let mut config = parse(&input.to_string());
    let root = config.root_config();
    let provider = &root.providers["applications"];
    assert_eq!(provider.enabled, Some(false));
    let item = &provider.entrypoints["org.example.Editor"];
    assert_eq!(item.enabled, Some(true));
    assert_eq!(item.alias.as_deref(), Some("write"));
    assert_eq!(item.shortcut.as_deref(), Some("ctrl+e"));
    assert_eq!(root.favorites, ["applications:org.example.Editor"]);
    assert_eq!(root.fallbacks, ["files:search"]);
    assert_eq!(serde_json::to_value(&config).unwrap(), input);
    config.launcher_mut().set_max_results(Some(7));
    let output = serde_json::to_value(&config).unwrap();
    assert_eq!(output["providers"], input["providers"]);
}

#[test]
fn absent_root_settings_stay_absent_and_malformed_settings_are_rejected() {
    // The default file's favourites, Clipboard History by its C++ id, read
    // as the Compass command.
    assert_eq!(
        parse("{}").root_config(),
        compass_core::root_items::RootConfig {
            favorites: vec!["commands:clipboard-history".to_owned()],
            ..Default::default()
        }
    );
    assert!(
        parse(r#"{"favorites": []}"#)
            .root_config()
            .favorites
            .is_empty(),
        "an empty list the user wrote stays empty"
    );
    assert_eq!(
        parse("{}").fallback_ids(),
        ["files:search"],
        "the default file's"
    );
    assert!(parse(r#"{"fallbacks": []}"#).fallback_ids().is_empty());
    assert_eq!(
        serde_json::to_value(parse("{}")).unwrap(),
        serde_json::json!({})
    );
    for value in [
        serde_json::json!({"providers": {}}),
        serde_json::json!({"providers": {"applications": {}}}),
        serde_json::json!({"providers": {"applications": {"entrypoints": {}}}}),
        serde_json::json!({"favorites": []}),
        serde_json::json!({"fallbacks": []}),
    ] {
        assert_eq!(
            serde_json::to_value(parse(&value.to_string())).unwrap(),
            value
        );
    }
    for value in [
        r#"{"providers": []}"#,
        r#"{"providers": {"applications": {"enabled": "false"}}}"#,
        r#"{"providers": {"applications": {"entrypoints": {"x": {"alias": 3}}}}}"#,
        r#"{"favorites": [42]}"#,
        r#"{"fallbacks": false}"#,
    ] {
        // A malformed setting no longer costs the file: it is reported, its
        // default used, and the value kept as written.
        let (config, issues) = Config::parse_checked(value, Path::new("config.json")).unwrap();
        assert!(
            issues.iter().any(|issue| issue.is_invalid_value()),
            "{value}"
        );
        assert!(
            config
                .root_config()
                .favorites
                .iter()
                .all(|id| !id.is_empty())
        );
        let written = serde_json::to_value(&config).unwrap();
        let original: serde_json::Value = serde_json::from_str(value).unwrap();
        assert_eq!(written, original, "kept as written: {value}");
    }
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
    assert_eq!(
        config.launcher().appearance().color_scheme(),
        DEFAULT_COLOR_SCHEME
    );
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
    let config = Config::load_from(dir.path().join("compass.json")).unwrap();
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

/// Every *known* key must survive a round trip on its own, too.
///
/// `unknown_fields_survive_a_round_trip` above cannot catch this: its document
/// carries unknown keys in every section, so no section is ever empty and
/// `skip_serializing_if` never fires. A config holding only `keybinding`
/// serialised back out as `{}` -- the section's `is_empty` had not been
/// updated when the key was added, so a read-modify-write deleted the user's
/// setting. Two more keys had the same hole.
///
/// Driven from a list that is itself checked for completeness below, so a key
/// added without a case here fails rather than going untested.
#[test]
fn every_known_key_survives_a_round_trip_on_its_own() {
    let cases = [
        (r#"{"launcher":{"hotkey":"ctrl+space"}}"#, "hotkey"),
        (
            r#"{"launcher":{"close_on_focus_loss":true}}"#,
            "close_on_focus_loss",
        ),
        (r#"{"launcher":{"max_results":12}}"#, "max_results"),
        (r#"{"launcher":{"keybinding":"vim"}}"#, "keybinding"),
        (
            r#"{"launcher":{"wrap_navigation":true}}"#,
            "wrap_navigation",
        ),
        (r#"{"launcher":{"quick_launch":false}}"#, "quick_launch"),
        (r#"{"extensions":{"auto_update":false}}"#, "auto_update"),
        (
            r#"{"extensions":{"installed":["com.example.clock"]}}"#,
            "installed",
        ),
        (
            r#"{"extensions":{"deny":[{"store":"raycast","author":"spam-*"}]}}"#,
            "deny",
        ),
        (
            r#"{"launcher":{"appearance":{"icons":true}}}"#,
            "appearance.icons",
        ),
        (
            r#"{"launcher":{"appearance":{"color_scheme":"dark"}}}"#,
            "appearance.color_scheme",
        ),
        (
            r#"{"launcher":{"appearance":{"preset":"rofi"}}}"#,
            "appearance.preset",
        ),
        (
            r#"{"launcher":{"appearance":{"tint":true}}}"#,
            "appearance.tint",
        ),
    ];

    for (original, key) in cases {
        let rewritten = parse(original).to_json_pretty().unwrap();
        let before: serde_json::Value = serde_json::from_str(original).unwrap();
        let after: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(
            before, after,
            "a config holding only `{key}` did not survive:\n{rewritten}"
        );
    }

    // Completeness: every key a fully-populated config writes must have a case
    // above. Without this the list rots the same way `is_empty` did.
    let everything = r#"{
      "launcher": {
        "hotkey": "ctrl+space",
        "close_on_focus_loss": true,
        "max_results": 12,
        "keybinding": "vim",
        "wrap_navigation": true,
        "quick_launch": false,
        "appearance": { "color_scheme": "dark", "preset": "rofi", "icons": true, "tint": true }
      },
      "extensions": { "auto_update": false, "installed": ["com.example.clock"] }
    }"#;
    let written: serde_json::Value =
        serde_json::from_str(&parse(everything).to_json_pretty().unwrap()).unwrap();
    let covered: std::collections::BTreeSet<&str> = cases.iter().map(|(_, k)| *k).collect();

    // Descends into nested sections, so `appearance.icons` is checked rather
    // than just `appearance`.
    fn leaves(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
        match value.as_object() {
            Some(map) => {
                for (key, child) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    leaves(child, &path, out);
                }
            }
            None => out.push(prefix.to_owned()),
        }
    }

    for section in ["launcher", "extensions"] {
        let mut found = Vec::new();
        leaves(&written[section], "", &mut found);
        assert!(!found.is_empty(), "`{section}` wrote nothing");
        for key in found {
            assert!(
                covered.contains(key.as_str()),
                "`{section}.{key}` has no round-trip case above"
            );
        }
    }
}

#[test]
fn unknown_fields_survive_an_edit_by_an_older_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compass").join("compass.json");
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
    let path = Path::new("/test/compass.json");
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
    assert!(rendered.contains("/test/compass.json"), "{rendered}");
    assert!(rendered.contains("line 3"), "{rendered}");
    assert!(rendered.contains("expected value"), "{rendered}");
}

#[test]
fn a_wrongly_typed_field_is_reported_and_the_rest_is_kept() {
    let (config, issues) = Config::parse_checked(
        r#"{"launcher": {"max_results": "lots", "hotkey": "alt+space"}}"#,
        Path::new("/test/compass.json"),
    )
    .unwrap();
    assert_eq!(config.launcher().hotkey(), "alt+space");
    assert_eq!(config.launcher().max_results(), DEFAULT_MAX_RESULTS);
    assert_eq!(issues.len(), 1);
    let rendered = issues[0].to_string();
    assert!(
        rendered.starts_with("launcher.max_results has a value of the wrong type"),
        "{rendered}"
    );
    assert!(rendered.contains("invalid type"), "{rendered}");
}

#[test]
fn a_top_level_non_object_is_a_clear_error() {
    let err = Config::parse("[1, 2, 3]", Path::new("/test/compass.json")).unwrap_err();
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
    let path = dir.path().join("a").join("b").join("compass.json");

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
    let path = dir.path().join("compass.json");
    Config::default().save_to(&path).unwrap();

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["compass.json"]);
}

/// `tint` is a *known* key, not an unknown one that survives by accident.
///
/// The round-trip test above cannot tell these apart, and I assumed it could.
/// Marking the field `#[serde(skip)]` — which should break a known key
/// completely — left that test green, because the key then falls into the
/// `#[serde(flatten)]` unknown map and is written back out verbatim.
///
/// Forward-compatibility preserving unknown keys is a feature. It also means
/// round-tripping proves nothing about whether *this build* understands a key.
/// Reading it back through the typed accessor is what does.
#[test]
fn tint_is_understood_and_not_merely_preserved() {
    let config = parse(r#"{"launcher":{"appearance":{"tint":true}}}"#);
    let appearance = config.launcher().appearance();

    assert_eq!(
        appearance.tint_override(),
        Some(true),
        "`tint` did not reach the typed accessor; it is being carried as an unknown key, so \
         this build does not actually understand it"
    );
    assert!(appearance.tint());
    assert!(
        !appearance.unknown_fields().contains_key("tint"),
        "`tint` is being held as an unknown key as well as a known one"
    );

    // And the default when nothing is written.
    let empty = parse("{}");
    assert_eq!(empty.launcher().appearance().tint_override(), None);
    assert!(
        !empty.launcher().appearance().tint(),
        "tint must default off: the Spotlight-simple default is what the VM tier's pixel \
         gates are calibrated against"
    );
}

#[test]
fn color_scheme_is_understood_and_system_is_the_default() {
    let config = parse(r#"{"launcher":{"appearance":{"color_scheme":"dark"}}}"#);
    let appearance = config.launcher().appearance();

    assert_eq!(appearance.color_scheme_override(), Some("dark"));
    assert_eq!(appearance.color_scheme(), "dark");
    assert!(!appearance.unknown_fields().contains_key("color_scheme"));

    let mut restored = config.clone();
    restored
        .launcher_mut()
        .appearance_mut()
        .set_color_scheme(None);
    assert_eq!(
        restored.launcher().appearance().color_scheme(),
        DEFAULT_COLOR_SCHEME
    );
}

#[test]
fn theme_is_understood_and_system_is_the_default() {
    let config = parse(r#"{"launcher":{"appearance":{"theme":"dracula"}}}"#);
    let appearance = config.launcher().appearance();

    assert_eq!(appearance.theme_override(), Some("dracula"));
    assert_eq!(appearance.theme(), "dracula");
    assert!(!appearance.unknown_fields().contains_key("theme"));

    // Unknown theme is treated as curated variant still persisted — parsing happens in compass-ui.
    // Config layer only ensures round-trip and default.
    let mut restored = config.clone();
    restored.launcher_mut().appearance_mut().set_theme(None);
    assert_eq!(
        restored.launcher().appearance().theme(),
        compass_core::config::DEFAULT_THEME
    );
    assert_eq!(restored.launcher().appearance().theme_override(), None);

    // Theme and preset are independently selectable (#153).
    let both = parse(
        r#"{"launcher":{"appearance":{"theme":"nord","preset":"raycast","color_scheme":"system"}}}"#,
    );
    assert_eq!(both.launcher().appearance().theme(), "nord");
    assert_eq!(both.launcher().appearance().preset(), "raycast");
    assert_eq!(both.launcher().appearance().color_scheme(), "system");
}

#[test]
fn provider_preferences_are_read_from_the_provider_object() {
    let config = parse(
        r#"{"providers": {"files": {"preferences": {"autoIndexing": false}},
            "broken": {"preferences": [1]}}}"#,
    );
    assert_eq!(
        config
            .provider_preferences("files")
            .and_then(|p| p.get("autoIndexing")),
        Some(&serde_json::Value::Bool(false))
    );
    assert!(config.provider_preferences("broken").is_none());
    assert!(config.provider_preferences("missing").is_none());
    assert!(parse("{}").provider_preferences("files").is_none());
}

#[test]
fn set_as_vicinae_font_writes_the_family_and_keeps_the_rest_of_font() {
    let mut config =
        parse(r#"{"font": {"rendering": "qt", "normal": {"family": "auto", "size": 10.5}}}"#);
    assert_eq!(config.font_family(), None, "auto is not a family");
    config.set_font_family("Fira Sans");
    assert_eq!(config.font_family(), Some("Fira Sans"));
    let written = config.to_json_pretty().unwrap();
    assert_eq!(parse(&written).font_family(), Some("Fira Sans"));
    assert!(written.contains("\"rendering\": \"qt\""), "{written}");
    assert!(written.contains("10.5"), "{written}");

    let mut empty = Config::default();
    empty.set_font_family("Noto Serif");
    assert_eq!(empty.font_family(), Some("Noto Serif"));
}

#[test]
fn the_root_panel_writes_favorites_whole_and_an_items_alias_and_switch() {
    use compass_core::root_items::RootEdit;
    let mut config = parse(r#"{"launcher": {"max_results": 9}}"#);

    // Favouriting writes the list out whole, default included, the new one
    // first (`setItemAsFavorite` takes the merged list).
    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Favorite(true)));
    let written: serde_json::Value = serde_json::to_value(&config).unwrap();
    assert_eq!(
        written["favorites"],
        serde_json::json!(["applications:firefox", "commands:clipboard-history"])
    );
    assert!(!config.apply_root_edit("applications:firefox", &RootEdit::Favorite(true)));
    assert!(config.apply_root_edit(
        "applications:firefox",
        &RootEdit::MoveFavorite { down: true }
    ));
    assert_eq!(
        config.root_config().favorites,
        ["commands:clipboard-history", "applications:firefox"]
    );
    assert!(
        !config.apply_root_edit(
            "applications:firefox",
            &RootEdit::MoveFavorite { down: true }
        ),
        "the last cannot move down"
    );
    assert!(config.apply_root_edit("commands:clipboard-history", &RootEdit::Favorite(false)));
    assert_eq!(config.root_config().favorites, ["applications:firefox"]);

    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Alias("ff".into())));
    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Disable));
    let item = &config.root_config().providers["applications"].entrypoints["firefox"];
    assert_eq!(item.alias.as_deref(), Some("ff"));
    assert_eq!(item.enabled, Some(false));
    assert!(!config.apply_root_edit("applications:firefox", &RootEdit::ResetRanking));
    assert_eq!(config.launcher().max_results(), 9, "the rest is kept");
}

#[test]
fn the_settings_switches_turn_an_item_and_a_provider_back_on() {
    use compass_core::root_items::RootEdit;
    let mut config = parse(r#"{"providers": {"applications": {"enabled": false}}}"#);
    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Disable));
    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Enabled(true)));
    config.set_provider_enabled("applications", true);
    let root = config.root_config();
    assert_eq!(root.providers["applications"].enabled, Some(true));
    assert_eq!(
        root.providers["applications"].entrypoints["firefox"].enabled,
        Some(true)
    );

    let mut local = compass_core::root_items::RootConfig::default();
    assert!(compass_core::root_items::apply_edit(
        &mut local,
        "applications:firefox",
        &RootEdit::Enabled(false)
    ));
    assert_eq!(
        local.providers["applications"].entrypoints["firefox"].enabled,
        Some(false)
    );
}

#[test]
fn the_clock_is_on_every_minute_in_hh_mm_unless_set() {
    let clock = parse("{}");
    let clock = clock.launcher().clock();
    assert!(clock.enabled());
    assert_eq!(clock.format(), "hh:mm");
    assert_eq!(clock.interval(), 60);

    let set = parse(
        r#"{"launcher": {"clock": {"enabled": false, "format": "hh:mm:ss", "interval": 0}}}"#,
    );
    let clock = set.launcher().clock();
    assert!(!clock.enabled());
    assert_eq!(clock.format(), "hh:mm:ss");
    assert_eq!(clock.interval(), 1, "never zero");
    assert_eq!(
        serde_json::to_value(&set).unwrap()["launcher"]["clock"]["format"],
        "hh:mm:ss"
    );
}

#[test]
fn a_fallback_is_enabled_first_and_disabled_as_the_cpp_writes_them() {
    use compass_core::root_items::RootEdit;
    let mut config = parse(r#"{"launcher": {"max_results": 9}}"#);
    assert_eq!(config.fallback_ids(), ["files:search"], "the default");
    assert!(config.apply_root_edit("@a/notes:new", &RootEdit::Fallback(true)));
    assert_eq!(config.fallback_ids(), ["@a/notes:new", "files:search"]);
    assert!(
        !config.apply_root_edit("@a/notes:new", &RootEdit::Fallback(true)),
        "already one"
    );
    assert!(config.apply_root_edit("files:search", &RootEdit::Fallback(false)));
    assert!(
        !config.apply_root_edit("files:search", &RootEdit::Fallback(false)),
        "not one any more"
    );
    let written: serde_json::Value = serde_json::to_value(&config).unwrap();
    assert_eq!(written["fallbacks"], serde_json::json!(["@a/notes:new"]));
    assert!(config.apply_root_edit("@a/notes:new", &RootEdit::Fallback(false)));
    assert!(
        config.fallback_ids().is_empty(),
        "an emptied list stays empty"
    );
}

#[test]
fn a_root_items_shortcut_is_written_in_the_cpps_spelling_and_cleared() {
    use compass_core::root_items::{RootEdit, RootItem, RootItemMeta};
    let mut config = parse(r#"{"launcher": {"max_results": 9}}"#);
    assert!(config.apply_root_edit(
        "applications:firefox",
        &RootEdit::Shortcut("control+shift+F".into())
    ));
    let written: serde_json::Value = serde_json::to_value(&config).unwrap();
    assert_eq!(
        written["providers"]["applications"]["entrypoints"]["firefox"]["shortcut"],
        "control+shift+F"
    );
    let mut firefox = RootItem {
        id: "applications:firefox".into(),
        title: "Firefox".into(),
        unlocalized_title: None,
        subtitle: String::new(),
        keywords: Vec::new(),
        meta: RootItemMeta::default(),
    };
    firefox.merge_config(&config.root_config(), false);
    assert_eq!(firefox.meta.shortcut.as_deref(), Some("control+shift+F"));

    // Cleared, it is gone from the file and from the item at the next merge.
    assert!(config.apply_root_edit("applications:firefox", &RootEdit::Shortcut(String::new())));
    let written: serde_json::Value = serde_json::to_value(&config).unwrap();
    assert!(
        written["providers"]["applications"]["entrypoints"]["firefox"]
            .get("shortcut")
            .is_none_or(serde_json::Value::is_null),
        "{written}"
    );
    firefox.merge_config(&config.root_config(), false);
    assert_eq!(firefox.meta.shortcut, None);
}

// --- system-wide defaults --------------------------------------------------

/// Writes `json` as `compass/compass.json` under `dir`, as a distribution puts
/// it under `/etc/xdg`.
fn system_file(dir: &Path, json: &str) {
    std::fs::create_dir_all(dir.join("compass")).unwrap();
    std::fs::write(dir.join("compass/compass.json"), json).unwrap();
}

#[test]
fn the_system_file_sets_defaults_the_users_file_overrides() {
    let etc = tempfile::tempdir().unwrap();
    system_file(
        etc.path(),
        r#"{
            // Bluefin binds the launcher through GNOME's own keyboard settings.
            "launcher": {"hotkey": "", "max_results": 20}
        }"#,
    );
    let system = compass_core::config::system_defaults_in(&[etc.path().to_path_buf()]);
    let path = Path::new("/test/compass.json");

    let (config, _) = Config::parse_layered("", path, system.as_ref()).unwrap();
    assert_eq!(config.launcher().hotkey(), "");
    assert_eq!(config.launcher().max_results(), 20);

    let user = r#"{"launcher": {"hotkey": "ctrl+space"}}"#;
    let (config, issues) = Config::parse_layered(user, path, system.as_ref()).unwrap();
    assert_eq!(
        config.launcher().hotkey(),
        "ctrl+space",
        "the user's own key wins"
    );
    assert_eq!(
        config.launcher().max_results(),
        20,
        "the rest of the section is inherited"
    );
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn the_first_system_directory_wins_and_a_broken_file_is_skipped() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let broken = tempfile::tempdir().unwrap();
    system_file(first.path(), r#"{"launcher": {"hotkey": "super+k"}}"#);
    system_file(
        second.path(),
        r#"{"launcher": {"hotkey": "super+j", "max_results": 9}}"#,
    );
    system_file(broken.path(), "{ not json");

    let system = compass_core::config::system_defaults_in(&[
        broken.path().to_path_buf(),
        first.path().to_path_buf(),
        second.path().to_path_buf(),
    ]);
    let (config, _) =
        Config::parse_layered("", Path::new("/test/compass.json"), system.as_ref()).unwrap();
    assert_eq!(config.launcher().hotkey(), "super+k");
    assert_eq!(config.launcher().max_results(), 9);
}

#[test]
fn saving_leaves_out_what_only_the_system_file_set() {
    let etc = tempfile::tempdir().unwrap();
    system_file(
        etc.path(),
        r#"{"launcher": {"hotkey": "", "max_results": 20}}"#,
    );
    let system = compass_core::config::system_defaults_in(&[etc.path().to_path_buf()]);
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("compass.json");
    std::fs::write(&path, "{\n  \"launcher\": {\"max_results\": 20}\n}\n").unwrap();

    let read = std::fs::read_to_string(&path).unwrap();
    let (mut config, _) = Config::parse_layered(&read, &path, system.as_ref()).unwrap();
    config.launcher_mut().set_max_results(Some(30));
    config.save_layered(&path, system.as_ref()).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["launcher"]["max_results"], 30);
    assert!(
        saved["launcher"].get("hotkey").is_none(),
        "the system's hotkey must not be copied into the user's file: {saved}"
    );

    // A fresh file gets only what the user changed.
    let fresh = home.path().join("fresh.json");
    let (mut config, _) = Config::parse_layered("", &fresh, system.as_ref()).unwrap();
    config.launcher_mut().set_max_results(Some(5));
    config.save_layered(&fresh, system.as_ref()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
    assert_eq!(
        saved["launcher"],
        serde_json::json!({"max_results": 5}),
        "{saved}"
    );
}

#[test]
fn a_value_the_user_wrote_stays_even_when_it_equals_the_systems() {
    let etc = tempfile::tempdir().unwrap();
    system_file(etc.path(), r#"{"launcher": {"hotkey": ""}}"#);
    let system = compass_core::config::system_defaults_in(&[etc.path().to_path_buf()]);
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("compass.json");
    std::fs::write(&path, r#"{"launcher": {"hotkey": ""}}"#).unwrap();

    let read = std::fs::read_to_string(&path).unwrap();
    let (mut config, _) = Config::parse_layered(&read, &path, system.as_ref()).unwrap();
    config.launcher_mut().set_max_results(Some(5));
    config.save_layered(&path, system.as_ref()).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["launcher"]["hotkey"], "", "{saved}");
}

#[test]
fn no_system_directory_means_no_system_defaults() {
    let empty = tempfile::tempdir().unwrap();
    assert!(compass_core::config::system_defaults_in(&[empty.path().to_path_buf()]).is_none());
    assert!(compass_core::config::system_defaults_in(&[]).is_none());
}

#[test]
fn an_admin_deny_list_cannot_be_lifted_or_written_by_the_user() {
    let etc = tempfile::tempdir().unwrap();
    system_file(
        etc.path(),
        r#"{"extensions": {
            "admin_deny": [{"store": "raycast"}],
            "deny": [{"id": "store.vicinae.clock"}]
        }}"#,
    );
    let system = compass_core::config::system_defaults_in(&[etc.path().to_path_buf()]);
    let path = Path::new("/test/compass.json");

    // A user who tries to set or clear the admin key cannot: it is removed
    // from their document before the system layer is inherited.
    let user = r#"{"extensions": {
        "admin_deny": [],
        "deny": [{"author": "spam-*"}]
    }}"#;
    let (config, issues) = Config::parse_layered(user, path, system.as_ref()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    let rules = config.extensions().deny_rules();
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert_eq!(rules[0].store.as_deref(), Some("raycast"), "admin first");
    assert_eq!(
        rules[1].author.as_deref(),
        Some("spam-*"),
        "then the user's"
    );

    // Saving never copies the admin list into the user's file.
    let home = tempfile::tempdir().unwrap();
    let saved_path = home.path().join("compass.json");
    config.save_layered(&saved_path, system.as_ref()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved_path).unwrap()).unwrap();
    assert!(
        saved["extensions"].get("admin_deny").is_none(),
        "the admin key must not be written to the user's file: {saved}"
    );
}
