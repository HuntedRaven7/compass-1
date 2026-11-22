//! Native host manifests, generated from one source for both browsers.
//!
//! The manifest is the security boundary: it names the one extension allowed
//! to spawn the host. A wildcard here would let any page's extension talk to
//! the launcher, which is why the generator refuses origins that are not
//! explicit. Install paths are recorded as data so #316 can generate them
//! instead of hand-editing per browser.

use serde_json::{Value, json};

/// The native host's registered name, shared by both browsers.
pub const HOST_NAME: &str = "org.tunaos.compass.browser";

/// Chrome/Chromium look here, relative to the user's home.
pub const CHROMIUM_PATHS: &[&str] = &[
    ".config/google-chrome/NativeMessagingHosts",
    ".config/chromium/NativeMessagingHosts",
    ".config/chromium-browser/NativeMessagingHosts",
];

/// Firefox looks here, and Flatpak Firefox under its own data directory.
pub const FIREFOX_PATHS: &[&str] = &[
    ".mozilla/native-messaging-hosts",
    ".var/app/org.mozilla.firefox/.mozilla/native-messaging-hosts",
];

/// Whether an origin is specific enough to name one extension.
fn specific_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://")
        && origin.ends_with('/')
        && !origin.contains('*')
        && origin.len() > "chrome-extension:///".len()
}

/// Whether a Firefox id names one extension rather than a wildcard.
fn specific_extension(id: &str) -> bool {
    !id.is_empty() && !id.contains('*') && !id.contains('/')
}

/// The Chrome manifest. Refuses to produce one with a wildcard origin.
///
/// # Errors
///
/// A sentence when no specific origin was supplied.
pub fn chrome_manifest(host_path: &str, allowed_origins: &[String]) -> Result<Value, String> {
    if allowed_origins.is_empty() || !allowed_origins.iter().all(|origin| specific_origin(origin)) {
        return Err("Chrome native hosts need at least one explicit chrome-extension:// origin and no wildcards".to_owned());
    }
    Ok(json!({
        "name": HOST_NAME,
        "description": "Compass browser tab bridge",
        "path": host_path,
        "type": "stdio",
        "allowed_origins": allowed_origins,
    }))
}

/// The Firefox manifest. Refuses to produce one with a wildcard id.
///
/// # Errors
///
/// A sentence when no specific extension id was supplied.
pub fn firefox_manifest(host_path: &str, allowed_extensions: &[String]) -> Result<Value, String> {
    if allowed_extensions.is_empty() || !allowed_extensions.iter().all(|id| specific_extension(id))
    {
        return Err(
            "Firefox native hosts need at least one explicit extension id and no wildcards"
                .to_owned(),
        );
    }
    Ok(json!({
        "name": HOST_NAME,
        "description": "Compass browser tab bridge",
        "path": host_path,
        "type": "stdio",
        "allowed_extensions": allowed_extensions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_names_one_origin_and_the_host_binary() {
        let manifest = chrome_manifest(
            "/app/bin/compass-browser-host",
            &["chrome-extension://abcdefghijklmnop/".to_owned()],
        )
        .unwrap();
        assert_eq!(manifest["name"], HOST_NAME);
        assert_eq!(manifest["path"], "/app/bin/compass-browser-host");
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(
            manifest["allowed_origins"][0],
            "chrome-extension://abcdefghijklmnop/"
        );
    }

    #[test]
    fn a_wildcard_origin_is_refused() {
        for origin in [
            "chrome://*".to_owned(),
            "*".to_owned(),
            "chrome-extension://*/".to_owned(),
            String::new(),
        ] {
            assert!(
                chrome_manifest("/bin/host", std::slice::from_ref(&origin)).is_err(),
                "{origin} should not be allowed to spawn the host"
            );
        }
    }

    #[test]
    fn firefox_manifest_names_one_extension() {
        let manifest = firefox_manifest(
            "/app/bin/compass-browser-host",
            &["tabs@tunaos.org".to_owned()],
        )
        .unwrap();
        assert_eq!(manifest["allowed_extensions"][0], "tabs@tunaos.org");
        assert!(
            firefox_manifest("/bin/host", std::slice::from_ref(&"*".to_owned())).is_err(),
            "a wildcard extension id must be refused"
        );
    }
}
