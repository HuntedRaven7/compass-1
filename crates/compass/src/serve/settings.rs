//! The settings view's writes (`GeneralSettingsModel`'s setters, and the
//! extension page's provider switch): one setting of `compass.json` at a
//! time, checked against `compass_core::settings_catalog`, then applied to
//! what the engine already holds where it holds it.

use std::sync::Arc;

use compass_core::Config;
use compass_core::file_search::IndexingSettings;
use compass_core::settings_catalog::{self, Kind};
use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

fn bad_request(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::BadRequest, message))
}

fn internal(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::Internal, message))
}

/// [`Request::SetSetting`], [`Request::SetProviderEnabled`], and the font and
/// theme pickers' [`Request::SetFont`] and [`Request::SetTheme`].
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::SetSetting { key, value_json } => set_setting(state, key, &value_json).await,
        Request::SetProviderEnabled { provider, enabled } => {
            set_provider_enabled(state, provider, enabled).await
        }
        Request::SetFont { family } => {
            let family = family.trim().to_owned();
            if family.is_empty() {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "a font family is needed",
                ));
            }
            let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut config = Config::load()?;
                config.set_font_family(&family);
                config.save_to(compass_core::config::default_config_path()?)?;
                Ok(())
            })
            .await;
            match saved {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(error)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save the font: {error}"),
                )),
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving the font failed: {error}"),
                )),
            }
        }
        Request::SetTheme { theme } => {
            let _ = tokio::task::spawn_blocking(compass_ui::theme::load_default_user_themes).await;
            let Some(parsed) = compass_ui::theme::Theme::from_name(&theme) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("unknown theme {theme:?}"),
                ));
            };
            let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut config = Config::load()?;
                config
                    .launcher_mut()
                    .appearance_mut()
                    .set_theme(Some(parsed.name().to_owned()));
                config.save_to(compass_core::config::default_config_path()?)?;
                Ok(())
            })
            .await;
            match saved {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(error)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save the theme: {error}"),
                )),
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving the theme failed: {error}"),
                )),
            }
        }
        _ => bad_request("not a settings request"),
    }
}

/// Reads the file, changes it with `change` and writes it back. A file that
/// does not parse is left alone rather than replaced by one holding only
/// this change.
async fn rewrite(
    change: impl FnOnce(&mut Config) -> Result<(), String> + Send + 'static,
) -> Result<Config, Response> {
    let saved = tokio::task::spawn_blocking(move || -> Result<Config, String> {
        let mut config =
            Config::load().map_err(|err| format!("could not read the configuration: {err}"))?;
        change(&mut config)?;
        let path = compass_core::config::default_config_path()
            .map_err(|err| format!("could not save the configuration: {err}"))?;
        config
            .save_to(path)
            .map_err(|err| format!("could not save the configuration: {err}"))?;
        Ok(config)
    })
    .await;
    match saved {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(reason)) => Err(internal(reason)),
        Err(err) => Err(internal(format!("the settings task failed: {err}"))),
    }
}

async fn set_setting(state: &Arc<RwLock<EngineState>>, key: String, value_json: &str) -> Response {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(value_json) else {
        return bad_request("the setting's value is not JSON");
    };
    let Some(setting) = settings_catalog::find(&key) else {
        return bad_request(format!("{key:?} is not a setting"));
    };
    let value = match settings_catalog::validate(&setting, value) {
        Ok(value) => value,
        Err(reason) => return bad_request(reason),
    };
    // A theme is kept by the name the theme list knows it by, as Set Theme
    // keeps it; one that is not a theme is refused here rather than written.
    let value = match (&setting.kind, value) {
        (Kind::Theme, Some(serde_json::Value::String(name))) => {
            let _ = tokio::task::spawn_blocking(compass_ui::theme::load_default_user_themes).await;
            match compass_ui::theme::Theme::from_name(&name) {
                Some(theme) => Some(serde_json::Value::String(theme.name().to_owned())),
                None => return bad_request(format!("unknown theme {name:?}")),
            }
        }
        (_, value) => value,
    };
    let path = setting.key.clone();
    let config = match rewrite(move |config| config.set_path(&path, value)).await {
        Ok(config) => config,
        Err(response) => return response,
    };
    apply_live(state, &config, &setting.key).await;
    // The launcher hotkey, or an item's shortcut or switch: the service
    // rebinds only what changed.
    state.read().await.global_shortcuts().reload();
    Response::Ack
}

/// Hands what the engine holds in memory the new value, where it holds it;
/// the rest is read where it is used, or when the launcher next starts.
async fn apply_live(state: &Arc<RwLock<EngineState>>, config: &Config, key: &str) {
    let expander = {
        let mut state = state.write().await;
        if key == "launcher.max_results" {
            state.search.max_results = config.launcher().max_results();
        }
        if key.starts_with("providers.clipboard.") {
            state
                .clipboard
                .control
                .apply(&crate::clipboard_service::Settings::from_preferences(
                    config.provider_preferences(crate::clipboard_service::PROVIDER_ID),
                ));
        }
        if key.contains(&format!(".{}.", crate::programs::ENTRYPOINT)) {
            state.scripts.run_program_default =
                crate::programs::default_action(config.entrypoint_preferences(
                    compass_core::commands::COMMANDS_PROVIDER_ID,
                    crate::programs::ENTRYPOINT,
                ));
        }
        state.snippets.expander.clone()
    };
    if key.starts_with("providers.files.") {
        let files = Arc::clone(&state.read().await.search.files);
        let applied = tokio::task::spawn_blocking(move || {
            files.reload_settings(|| {
                let latest = Config::load().map_err(|error| error.to_string())?;
                file_indexing_settings(&latest)
            })
        })
        .await;
        match applied {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => {
                tracing::warn!(%reason, "keeping the file indexing settings in use");
            }
            Err(error) => {
                tracing::warn!(%error, "could not apply file indexing settings");
            }
        }
    }
    if key == "tray.enabled" {
        state
            .read()
            .await
            .tray_icon()
            .set_enabled(config.tray().enabled());
    }
    if key == "input_server.enabled"
        && let Some(expander) = expander
    {
        expander
            .server()
            .set_enabled(config.input_server().enabled());
    }
}

/// A malformed hand edit must not turn a restricted index into the default
/// home-wide index. Validate the raw values, which Config preserves even when
/// it cannot deserialize an ancestor, before taking a new live snapshot.
pub(super) fn file_indexing_settings(config: &Config) -> Result<IndexingSettings, String> {
    for key in [
        "providers",
        "providers.files",
        "providers.files.preferences",
    ] {
        if config
            .get_path(key)
            .is_some_and(|value| !value.is_null() && !value.is_object())
        {
            return Err(format!("{key} must be an object"));
        }
    }
    let mut preferences = serde_json::Map::new();
    for name in ["autoIndexing", "indexingPaths", "excludedIndexingPaths"] {
        let key = format!("providers.files.preferences.{name}");
        if let Some(value) = config.get_path(&key) {
            let setting = settings_catalog::find(&key).expect("a file indexing setting");
            if let Some(value) = settings_catalog::validate(&setting, value)? {
                preferences.insert(name.to_owned(), value);
            }
        }
    }
    Ok(IndexingSettings::from_preferences(
        Some(&preferences),
        compass_core::xdg_dirs::home_dir().as_deref(),
    ))
}

/// `compass.json` changed on disk (a hand edit, `compass theme set`, another
/// window's settings): everything the engine holds of it is brought in line,
/// as [`apply_live`] does for one key.
pub async fn apply_all(state: &Arc<RwLock<EngineState>>, config: &Config) {
    for key in [
        "launcher.max_results",
        "providers.clipboard.",
        "providers.files.",
        &format!(".{}.", crate::programs::ENTRYPOINT),
        "tray.enabled",
        "input_server.enabled",
    ] {
        apply_live(state, config, key).await;
    }
    state.read().await.global_shortcuts().reload();
    state.read().await.stores.sync.request();
    state
        .write()
        .await
        .search
        .index
        .apply_root_config(&config.root_config());
}

async fn set_provider_enabled(
    state: &Arc<RwLock<EngineState>>,
    provider: String,
    enabled: bool,
) -> Response {
    if provider.is_empty() || provider.contains(':') {
        return bad_request(format!("{provider:?} is not a provider id"));
    }
    let config = match rewrite(move |config| {
        config.set_provider_enabled(&provider, enabled);
        Ok(())
    })
    .await
    {
        Ok(config) => config,
        Err(response) => return response,
    };
    state
        .write()
        .await
        .search
        .index
        .apply_root_config(&config.root_config());
    Response::Ack
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(value: serde_json::Value) -> Config {
        Config::parse_layered(
            &value.to_string(),
            std::path::Path::new("compass.json"),
            None,
        )
        .unwrap()
        .0
    }

    #[test]
    fn malformed_file_preferences_are_not_a_reset_to_defaults() {
        for value in [
            serde_json::json!({"providers": false}),
            serde_json::json!({"providers": {"files": []}}),
            serde_json::json!({"providers": {"files": {"preferences": false}}}),
            serde_json::json!({"providers": {"files": {"preferences": {
                "autoIndexing": "yes"
            }}}}),
            serde_json::json!({"providers": {"files": {"preferences": {
                "indexingPaths": "/data"
            }}}}),
            serde_json::json!({"providers": {"files": {"preferences": {
                "excludedIndexingPaths": ["/data/private", 7]
            }}}}),
        ] {
            assert!(
                file_indexing_settings(&config(value.clone())).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn missing_and_null_file_preferences_still_reset_to_defaults() {
        let defaults = file_indexing_settings(&Config::default()).unwrap();
        for value in [
            serde_json::json!({}),
            serde_json::json!({"providers": {"files": {"preferences": null}}}),
            serde_json::json!({"providers": {"files": {"preferences": {
                "autoIndexing": null, "indexingPaths": null, "excludedIndexingPaths": null
            }}}}),
        ] {
            assert_eq!(file_indexing_settings(&config(value)).unwrap(), defaults);
        }
        let empty = config(serde_json::json!({"providers": {"files": {"preferences": {
            "autoIndexing": false, "indexingPaths": [], "excludedIndexingPaths": []
        }}}}));
        let settings = file_indexing_settings(&empty).unwrap();
        assert!(!settings.enabled);
        assert!(
            settings.paths.is_empty(),
            "an explicit empty list stays empty"
        );
    }
}
