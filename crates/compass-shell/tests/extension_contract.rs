//! The in-tree GNOME Shell extension against the contract this crate speaks.
//!
//! The extension cannot run in CI — `gnome-shell` needs logind — so the
//! drift that matters is checked statically: its copies of the interface
//! XML are the contract files, it declares the same version, it implements
//! every method and emits every signal, and `ListWindows` fills every
//! dictionary key the client decodes. A contract change that forgets the
//! extension, or an extension edit that forgets the contract, fails here.

use std::path::PathBuf;

use compass_shell::contract::{CLIPBOARD_XML, WINDOWS_XML, window_key, workspace_key};

fn extension_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/gnome-shell/compass@tunaos.org")
}

fn read(relative: &str) -> String {
    let path = extension_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn script() -> String {
    read("extension.js")
}

/// `name="..."` of every `<tag` element in `xml`.
fn names(xml: &str, tag: &str) -> Vec<String> {
    xml.split(&format!("<{tag} "))
        .skip(1)
        .filter_map(|rest| {
            let start = rest.find("name=\"")? + "name=\"".len();
            let end = rest[start..].find('"')?;
            Some(rest[start..start + end].to_owned())
        })
        .collect()
}

#[test]
fn the_extension_ships_the_contract_files_unchanged() {
    assert_eq!(
        read("dbus/org.tunaos.compass.Shell.Windows.xml"),
        WINDOWS_XML
    );
    assert_eq!(
        read("dbus/org.tunaos.compass.Shell.Clipboard.xml"),
        CLIPBOARD_XML
    );
}

#[test]
fn the_extension_speaks_this_contract_version() {
    let expected = format!(
        "const CONTRACT_VERSION = {};",
        compass_shell::CONTRACT_VERSION
    );
    assert!(
        script().contains(&expected),
        "extension.js must declare `{expected}`"
    );
}

#[test]
fn every_contract_method_is_implemented_and_every_signal_emitted() {
    let script = script();
    for xml in [WINDOWS_XML, CLIPBOARD_XML] {
        let methods = names(xml, "method");
        assert!(!methods.is_empty(), "the parser found no methods");
        for method in methods {
            assert!(
                script.contains(&format!("    {method}("))
                    || script.contains(&format!("    {method}Async(")),
                "extension.js does not implement {method}"
            );
        }
        // Whitespace-insensitive: a long emit wraps its arguments.
        let compact: String = script.chars().filter(|c| !c.is_whitespace()).collect();
        for signal in names(xml, "signal") {
            assert!(
                compact.contains(&format!("emit_signal('{signal}'")),
                "extension.js never emits {signal}"
            );
        }
    }
}

#[test]
fn list_windows_fills_every_key_the_client_decodes() {
    let script = script();
    for key in [
        window_key::ID,
        window_key::TITLE,
        window_key::WM_CLASS,
        window_key::WM_CLASS_INSTANCE,
        window_key::PID,
        window_key::FOCUSED,
        window_key::WORKSPACE,
        window_key::CAN_CLOSE,
        window_key::FULLSCREEN,
        window_key::X,
        window_key::Y,
        window_key::WIDTH,
        window_key::HEIGHT,
    ] {
        assert!(
            script.contains(&format!("{key}: new GLib.Variant("))
                || script.contains(&format!("entry.{key} = new GLib.Variant(")),
            "ListWindows never sets `{key}`"
        );
    }
}

#[test]
fn list_workspaces_fills_every_key_the_client_decodes() {
    let script = script();
    for key in [
        workspace_key::INDEX,
        workspace_key::NAME,
        workspace_key::ACTIVE,
        workspace_key::HAS_FULLSCREEN,
    ] {
        assert!(
            script.contains(&format!("{key}: new GLib.Variant(")),
            "ListWorkspaces never sets `{key}`"
        );
    }
}

#[test]
fn a_workspace_switch_tells_the_client_to_look_again() {
    // Switch Workspaces refreshes on WindowsChanged, so the extension must
    // emit it when the workspaces themselves change, not only the windows.
    let script = script();
    for signal in [
        "'active-workspace-changed'",
        "'workspace-added'",
        "'workspace-removed'",
    ] {
        assert!(
            script.contains(signal),
            "extension.js does not watch {signal}"
        );
    }
}

#[test]
fn the_vm_tier_asks_for_the_workspaces() {
    let checks = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/vmtest/checks.sh"),
    )
    .expect("checks.sh");
    assert!(
        checks.contains("org.tunaos.compass.Shell.Windows.ListWorkspaces"),
        "checks.sh shell-extension must call ListWorkspaces on the real Shell"
    );
}

#[test]
fn the_extension_claims_the_gnome_versions_ci_covers() {
    let metadata = read("metadata.json");
    for version in ["\"50\"", "\"51\""] {
        assert!(
            metadata.contains(version),
            "metadata.json must list GNOME {version}"
        );
    }
}

#[test]
fn the_flatpak_may_talk_to_the_bus_name_the_contract_lives_on() {
    // A talk-name filters bus names. The extension's objects live on the
    // Shell's own connection, so the sandbox needs exactly this one; a grant
    // for the interface name (which no process owns) lets nothing through.
    let manifest = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/flatpak/org.tunaos.compass.yaml"),
    )
    .expect("the Flatpak manifest");
    let grant = format!("- --talk-name={}", compass_shell::SHELL_SERVICE);
    assert!(
        manifest.lines().any(|line| line.trim() == grant),
        "the manifest must grant `{grant}`"
    );
}

#[test]
fn the_vm_tier_waits_for_this_contract_version() {
    let checks = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/vmtest/checks.sh"),
    )
    .expect("checks.sh");
    let wanted = format!("*\"uint32 {}>\"*", compass_shell::CONTRACT_VERSION);
    assert!(
        checks.contains(&wanted),
        "checks.sh shell-extension must wait for `{wanted}`, the version this build speaks"
    );
}

#[test]
fn the_paste_keyboard_does_not_depend_on_the_backend_gnome_51_removed() {
    // `Clutter.get_default_backend()` is gone in GNOME Shell 51. Calling it
    // throws inside a timeout callback after `Paste` has returned, so paste
    // silently does nothing. Only the stage's context may be tried first.
    let script = script();
    let call = "Clutter.get_default_backend()";
    let fallback = "global.stage.get_context\n";
    assert!(
        script.contains("global.stage.get_context().get_backend()"),
        "the virtual keyboard must come from the stage's Clutter context"
    );
    let guarded = script
        .find(fallback)
        .zip(script.find(call))
        .is_some_and(|(guard, old)| guard < old);
    assert!(
        guarded && script.matches(call).count() == 1,
        "the removed `{call}` may only be a fallback behind a `get_context` check"
    );
}
