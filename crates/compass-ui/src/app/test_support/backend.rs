//! [`TestBackend`]: the engine's application backend as a recording fake.

use std::fs;

/// A snippet the fake expanded or pasted: `(id, arguments, pasted)`.
pub(crate) type SnippetUse = (String, Vec<(String, String)>, bool);

#[derive(Debug, Default)]
pub(crate) struct TestBackend {
    /// Local storage, by namespace.
    pub(crate) storage: Vec<(String, Vec<crate::backend::StorageItemRow>)>,
    /// Stored token sets.
    pub(crate) token_sets: std::sync::Mutex<Vec<crate::backend::TokenSetRow>>,
    /// The root edits the engine was asked for.
    pub(crate) root_edits: std::sync::Mutex<Vec<(String, compass_core::root_items::RootEdit)>>,
    /// Other applications' tray icons.
    pub(crate) tray: Vec<crate::backend::TrayItemRow>,
    /// What the tray was asked to do.
    pub(crate) tray_calls: std::sync::Mutex<Vec<String>>,
    /// What the default pickers offer.
    pub(crate) default_apps: Vec<crate::backend::DefaultAppRow>,
    /// The recorder's capture, as the engine was told it.
    pub(crate) captures: std::sync::Mutex<Vec<bool>>,
    /// What the desktop says of every probed combination, and the
    /// combinations probed.
    pub(crate) probe_refusal: Option<String>,
    pub(crate) probes: std::sync::Mutex<Vec<String>>,
    /// The defaults set: `(kind, id)`.
    pub(crate) defaults_set: std::sync::Mutex<Vec<(crate::backend::DefaultApp, String)>>,
    pub(crate) keys: Vec<String>,
    pub(crate) recorded: std::sync::Mutex<Vec<String>>,
    pub(crate) fail_history: bool,
    pub(crate) ran: std::sync::Mutex<Vec<String>>,
    pub(crate) refuse_runs: Option<String>,
    /// Starts a view session instead of running to completion.
    pub(crate) view: Option<compass_extension_api::View>,
    pub(crate) events: std::sync::Mutex<Vec<(String, Vec<serde_json::Value>)>>,
    pub(crate) closed: std::sync::Mutex<Vec<u64>>,
    /// The view stack depth the fake reports.
    pub(crate) depth: std::sync::Mutex<u32>,
    /// An alert the fake's first view carries.
    pub(crate) alert: Option<crate::backend::ExtensionPrompt>,
    /// A toast the fake's views carry.
    pub(crate) toast: Option<crate::backend::ExtensionToast>,
    /// The power commands asked for.
    pub(crate) powered: std::sync::Mutex<Vec<String>>,
    /// The media commands asked for, each with its argument after a
    /// space.
    pub(crate) played: std::sync::Mutex<Vec<String>>,
    /// The players Now Playing lists.
    pub(crate) players: std::sync::Mutex<Vec<crate::backend::MediaPlayerRow>>,
    /// The families "Set as Compass font" saved.
    pub(crate) fonts_set: std::sync::Mutex<Vec<String>>,
    /// What the user allowed their Rhai scripts.
    pub(crate) grants: std::sync::Mutex<Vec<crate::backend::ScriptGrant>>,
    /// The calculator's history, one group, and what changed it.
    pub(crate) calculations: std::sync::Mutex<Vec<crate::backend::CalculatorRow>>,
    pub(crate) calculator_edits: std::sync::Mutex<Vec<crate::backend::CalculatorChange>>,
    /// What "Open with…" offers, and what it was asked to open.
    pub(crate) openers: Vec<crate::backend::OpenerRow>,
    pub(crate) opener_lookups: std::sync::Mutex<Vec<String>>,
    pub(crate) opened_with: std::sync::Mutex<Vec<(String, String)>>,
    /// What a file's panel depends on, and the file actions asked for.
    pub(crate) file_info: crate::backend::FileActions,
    pub(crate) file_calls: std::sync::Mutex<Vec<(&'static str, String)>>,
    /// What Now Playing asked the players to do.
    pub(crate) controlled: std::sync::Mutex<Vec<(String, crate::backend::MediaAction)>>,
    pub(crate) answers: std::sync::Mutex<Vec<bool>>,
    /// Whether the engine copies text; what it copied.
    pub(crate) copies: bool,
    pub(crate) copied: std::sync::Mutex<Vec<String>>,
    pub(crate) remembered: std::sync::Mutex<Vec<()>>,
    /// Preferences the fake asks for until some are saved.
    pub(crate) needs: Vec<crate::backend::PreferenceInput>,
    pub(crate) saved: std::sync::Mutex<Vec<serde_json::Map<String, serde_json::Value>>>,
    /// Arguments the fake asks for until a run carries some.
    pub(crate) wants: Vec<crate::backend::PreferenceInput>,
    pub(crate) given: std::sync::Mutex<Vec<Option<serde_json::Map<String, serde_json::Value>>>>,
    /// The files Search Files finds, by name.
    pub(crate) files: Vec<crate::backend::FileRow>,
    /// The Search Files queries asked, in order.
    pub(crate) file_queries: std::sync::Mutex<Vec<String>>,
    /// The files opened, and whether each was only revealed.
    pub(crate) opened: std::sync::Mutex<Vec<(String, bool)>>,
    /// The shortcut store.
    pub(crate) shortcuts: std::sync::Mutex<Vec<crate::backend::Shortcut>>,
    /// The shortcuts opened, with their arguments.
    pub(crate) opened_shortcuts: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// The root items launched through the engine, with their text.
    pub(crate) launched: std::sync::Mutex<Vec<(String, Option<String>)>>,
    /// The text pasted.
    pub(crate) pasted: std::sync::Mutex<Vec<String>>,
    /// Whether a paste is refused, as where the engine cannot paste.
    pub(crate) refuse_paste: bool,
    /// The shortcuts saved, as the form sent them.
    pub(crate) drafts: std::sync::Mutex<Vec<crate::backend::ShortcutDraft>>,
    /// The snippet store.
    pub(crate) snippets: std::sync::Mutex<Vec<crate::backend::Snippet>>,
    /// The snippets saved, as the form sent them.
    pub(crate) snippet_drafts: std::sync::Mutex<Vec<crate::backend::SnippetDraft>>,
    /// The snippets expanded or pasted: `(id, arguments, pasted)`.
    pub(crate) snippet_uses: std::sync::Mutex<Vec<SnippetUse>>,
    /// The script commands the fake lists.
    pub(crate) scripts: Vec<compass_core::script_scan::ScriptItem>,
    /// Each script's icon URL, by id.
    pub(crate) script_icons: Vec<(String, String)>,
    /// The scripts run, with their arguments.
    pub(crate) script_runs: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// The programs Run Terminal Program ran: `(argv, terminal, hold)`.
    pub(crate) programs_ran: std::sync::Mutex<Vec<(Vec<String>, bool, bool)>>,
    /// The dmenu answers sent: `(token, output)`.
    pub(crate) dmenu_answers: std::sync::Mutex<Vec<(u64, Option<String>)>>,
    /// The themes kept.
    pub(crate) themes_kept: std::sync::Mutex<Vec<String>>,
    /// The extensions created.
    pub(crate) created: std::sync::Mutex<Vec<crate::backend::ExtensionDraft>>,
    /// The store's rows; an install marks one installed and writes its
    /// manifest into `store_dir`, as the engine would.
    pub(crate) store_rows: std::sync::Mutex<Vec<crate::backend::StoreRow>>,
    /// Each install asked for: the store, the owner and the name.
    pub(crate) installs: std::sync::Mutex<Vec<(crate::backend::Store, String, String)>>,
    /// The handles detail pages and installs were asked by, in order.
    pub(crate) store_owners: std::sync::Mutex<Vec<String>>,
    /// Where installs land.
    pub(crate) store_dir: Option<std::path::PathBuf>,
    /// The store browses asked, in order.
    pub(crate) store_queries: std::sync::Mutex<Vec<(crate::backend::Store, String)>>,
    /// The URLs opened.
    pub(crate) opened_urls: std::sync::Mutex<Vec<String>>,
    /// The newer release the engine reports.
    pub(crate) update: std::sync::Mutex<Option<crate::backend::UpdateOffer>>,
    /// The releases skipped.
    pub(crate) skipped: std::sync::Mutex<Vec<String>>,
    /// The exchange rates the fake engine holds and refreshes to, and
    /// why a refresh fails when it does.
    pub(crate) rates: Option<compass_core::exchange_rates::ExchangeRates>,
    pub(crate) refuse_rates: Option<String>,
    /// The Rhai scripts the fake lists; `None` answers as the trait's
    /// default does.
    pub(crate) rhai_scripts: Option<Vec<compass_core::rhai_scripts::RhaiScriptItem>>,
    /// Whether settings writes are taken and recorded rather than refused
    /// as the trait's default refuses them.
    pub(crate) records_settings: bool,
    /// The settings written: `(key, value)`.
    pub(crate) settings_set: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    /// The providers switched: `(provider, enabled)`.
    pub(crate) providers_set: std::sync::Mutex<Vec<(String, bool)>>,
    /// The commands whose preferences were asked for.
    pub(crate) preferences_asked: std::sync::Mutex<Vec<String>>,
    /// Why opening a file or a link fails, when it does.
    pub(crate) refuse_opens: Option<String>,
    /// Where the fake engine says the launcher hotkey stands; `None`
    /// answers as the trait's default does.
    pub(crate) hotkey: std::sync::Mutex<Option<compass_core::launcher_shortcut::LauncherShortcut>>,
    /// What asking the desktop for the hotkey turns it into.
    pub(crate) hotkey_on_request: Option<compass_core::launcher_shortcut::LauncherShortcut>,
    /// Each hotkey call: `true` for a request, `false` for a read.
    pub(crate) hotkey_calls: std::sync::Mutex<Vec<bool>>,
    /// Whether Compass starts at login, as the fake engine says; `None`
    /// answers as the trait's default does.
    pub(crate) login: std::sync::Mutex<Option<compass_core::login_start::Status>>,
    /// What turning login start on leaves; on otherwise.
    pub(crate) login_enabled_as: Option<compass_core::login_start::State>,
    /// Why a change to login start fails, when it does.
    pub(crate) login_failure: Option<String>,
    /// The changes asked for.
    pub(crate) login_changes: std::sync::Mutex<Vec<bool>>,
}

impl crate::backend::ApplicationBackend for TestBackend {
    fn search(&self, _query: String) -> crate::backend::BackendFuture<'_, Vec<String>> {
        Box::pin(async { Ok(self.keys.clone()) })
    }

    fn set_shortcut_capture(&self, capturing: bool) -> crate::backend::BackendFuture<'_, ()> {
        self.captures.lock().unwrap().push(capturing);
        Box::pin(async { Ok(()) })
    }

    fn probe_shortcut(&self, trigger: String) -> crate::backend::BackendFuture<'_, Option<String>> {
        self.probes.lock().unwrap().push(trigger);
        let refusal = self.probe_refusal.clone();
        Box::pin(async move { Ok(refusal) })
    }

    fn launcher_shortcut(
        &self,
        request: bool,
    ) -> crate::backend::BackendFuture<'_, compass_core::launcher_shortcut::LauncherShortcut> {
        self.hotkey_calls.lock().unwrap().push(request);
        let mut hotkey = self.hotkey.lock().unwrap();
        if request && let Some(after) = &self.hotkey_on_request {
            *hotkey = Some(after.clone());
        }
        let answer = hotkey
            .clone()
            .ok_or_else(|| crate::backend::SHORTCUT_NEEDS_ENGINE.to_owned());
        Box::pin(async move { answer })
    }

    fn login_start(
        &self,
        enable: Option<bool>,
    ) -> crate::backend::BackendFuture<'_, compass_core::login_start::Status> {
        use compass_core::login_start::{Mechanism, State};
        let mut login = self.login.lock().unwrap();
        let answer = match (enable, &self.login_failure) {
            (Some(_), Some(failure)) => Err(failure.clone()),
            (Some(on), None) => {
                self.login_changes.lock().unwrap().push(on);
                let state = if on {
                    self.login_enabled_as.clone().unwrap_or(State::On {
                        mechanism: Mechanism::Portal,
                    })
                } else {
                    State::Off
                };
                let status = compass_core::login_start::Status {
                    state,
                    ..login.clone().unwrap_or_default()
                };
                *login = Some(status.clone());
                Ok(status)
            }
            (None, _) => login
                .clone()
                .ok_or_else(|| crate::backend::LOGIN_START_NEEDS_ENGINE.to_owned()),
        };
        Box::pin(async move { answer })
    }

    fn edit_root_item(
        &self,
        id: String,
        edit: compass_core::root_items::RootEdit,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.root_edits.lock().unwrap().push((id, edit));
            Ok(())
        })
    }

    fn local_storage_namespaces(&self) -> crate::backend::BackendFuture<'_, Vec<String>> {
        Box::pin(async move { Ok(self.storage.iter().map(|(ns, _)| ns.clone()).collect()) })
    }

    fn local_storage_items(
        &self,
        namespace: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::StorageItemRow>> {
        Box::pin(async move {
            Ok(self
                .storage
                .iter()
                .find(|(ns, _)| *ns == namespace)
                .map(|(_, items)| items.clone())
                .unwrap_or_default())
        })
    }

    fn oauth_token_sets(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TokenSetRow>> {
        Box::pin(async move { Ok(self.token_sets.lock().unwrap().clone()) })
    }

    fn remove_oauth_token_set(
        &self,
        extension_id: String,
        provider_id: Option<String>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.token_sets
                .lock()
                .unwrap()
                .retain(|set| set.extension_id != extension_id || set.provider_id != provider_id);
            Ok(())
        })
    }

    fn list_default_apps(
        &self,
        _kind: crate::backend::DefaultApp,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::DefaultAppRow>> {
        Box::pin(async move { Ok(self.default_apps.clone()) })
    }

    fn set_default_app(
        &self,
        kind: crate::backend::DefaultApp,
        id: String,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if id == "broken.desktop" {
                return Err(compass_core::default_app::BROWSER_FAILURE.to_owned());
            }
            self.defaults_set.lock().unwrap().push((kind, id));
            Ok(())
        })
    }

    fn record_launch(&self, key: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.recorded.lock().unwrap().push(key);
            if self.fail_history {
                Err("disk full".to_owned())
            } else {
                Ok(())
            }
        })
    }

    fn run_power_command(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.powered.lock().unwrap().push(id);
            Ok(())
        })
    }

    fn run_media_command(
        &self,
        id: String,
        argument: Option<String>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.played.lock().unwrap().push(match argument {
                Some(argument) => format!("{id} {argument}"),
                None => id,
            });
            Err("No media player is running".to_owned())
        })
    }

    fn list_media_players(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::MediaPlayerRow>> {
        Box::pin(async move { Ok(self.players.lock().unwrap().clone()) })
    }

    fn control_media_player(
        &self,
        player: String,
        action: crate::backend::MediaAction,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            let mut players = self.players.lock().unwrap();
            if action == crate::backend::MediaAction::PlayPause
                && let Some(row) = players.iter_mut().find(|row| row.id == player)
            {
                row.playing = !row.playing;
                row.paused = !row.playing;
            }
            self.controlled.lock().unwrap().push((player, action));
            Ok(())
        })
    }

    fn file_actions(
        &self,
        _path: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::FileActions> {
        Box::pin(async move { Ok(self.file_info.clone()) })
    }

    fn copy_file(&self, path: String, paste: bool) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            let what = if paste { "paste" } else { "copy" };
            self.file_calls.lock().unwrap().push((what, path));
            Ok(())
        })
    }

    fn run_executable(
        &self,
        path: String,
        make_executable: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            assert!(make_executable);
            self.file_calls.lock().unwrap().push(("run", path));
            Ok(())
        })
    }

    fn set_wallpaper(&self, path: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.file_calls.lock().unwrap().push(("wallpaper", path));
            Err("Failed to set wallpaper: no backend".to_owned())
        })
    }

    fn search_files(
        &self,
        query: String,
        category: Option<String>,
    ) -> crate::backend::BackendFuture<'_, crate::backend::FileResults> {
        Box::pin(async move {
            self.file_queries.lock().unwrap().push(match &category {
                Some(category) => format!("{query} [{category}]"),
                None => query.clone(),
            });
            Ok(crate::backend::FileResults {
                heading: if query.is_empty() {
                    "Recently Accessed".to_owned()
                } else {
                    "Results".to_owned()
                },
                files: self
                    .files
                    .iter()
                    .filter(|file| file.name.contains(&query))
                    .filter(|file| category.as_ref().is_none_or(|c| &file.category == c))
                    .cloned()
                    .collect(),
            })
        })
    }

    fn open_file(&self, path: String, reveal: bool) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.opened.lock().unwrap().push((path, reveal));
            self.refuse_opens.clone().map_or(Ok(()), Err)
        })
    }

    fn fetch_dmenu(
        &self,
        token: u64,
    ) -> crate::backend::BackendFuture<'_, crate::backend::DmenuList> {
        Box::pin(async move {
            assert_eq!(token, 5);
            Ok(crate::backend::DmenuList {
                content: "alpha\nbeta\n\ngamma\n".into(),
                section_title: Some("Pick ({count})".into()),
                ..crate::backend::DmenuList::default()
            })
        })
    }

    fn create_extension(
        &self,
        draft: crate::backend::ExtensionDraft,
    ) -> crate::backend::BackendFuture<'_, String> {
        Box::pin(async move {
            let path = format!("{}/{}", draft.location, draft.title.to_lowercase());
            self.created.lock().unwrap().push(draft);
            Ok(path)
        })
    }

    fn set_theme(&self, theme: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.themes_kept.lock().unwrap().push(theme);
            Ok(())
        })
    }

    fn list_fonts(&self) -> crate::backend::BackendFuture<'_, crate::backend::FontList> {
        Box::pin(async {
            let font =
                |name: &str, primary: &str, categories: &[&str]| crate::backend::FontListEntry {
                    name: name.into(),
                    family: name.into(),
                    glyph: Some("Aa".into()),
                    color: false,
                    primary: primary.into(),
                    categories: categories.iter().map(|c| (*c).to_owned()).collect(),
                };
            Ok(crate::backend::FontList {
                fonts: vec![
                    font("Inter", "Latin", &["Latin"]),
                    font("JetBrains Mono", "Monospace", &["Latin", "Monospace"]),
                ],
                categories: vec!["Latin".into(), "Monospace".into()],
            })
        })
    }

    fn list_script_grants(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ScriptGrant>> {
        Box::pin(async move { Ok(self.grants.lock().unwrap().clone()) })
    }

    fn tray_items(&self) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TrayItemRow>> {
        Box::pin(async move { Ok(self.tray.clone()) })
    }

    fn tray_activate(&self, key: String, secondary: bool) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.tray_calls
                .lock()
                .unwrap()
                .push(format!("activate {key} {secondary}"));
            Ok(())
        })
    }

    fn tray_menu(
        &self,
        key: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TrayMenuRow>> {
        Box::pin(async move {
            self.tray_calls.lock().unwrap().push(format!("menu {key}"));
            Ok(vec![
                crate::backend::TrayMenuRow {
                    id: 1,
                    label: "Open Chat".into(),
                    ..crate::backend::TrayMenuRow::default()
                },
                crate::backend::TrayMenuRow {
                    id: 3,
                    label: "Status › Away".into(),
                    toggled: Some(true),
                    ..crate::backend::TrayMenuRow::default()
                },
            ])
        })
    }

    fn tray_trigger(&self, key: String, id: i32) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.tray_calls
                .lock()
                .unwrap()
                .push(format!("trigger {key} {id}"));
            Ok(())
        })
    }

    fn exchange_rates(
        &self,
    ) -> crate::backend::BackendFuture<'_, Option<compass_core::exchange_rates::ExchangeRates>>
    {
        Box::pin(async move { Ok(self.rates.clone()) })
    }

    fn refresh_exchange_rates(
        &self,
    ) -> crate::backend::BackendFuture<'_, compass_core::exchange_rates::ExchangeRates> {
        Box::pin(async move {
            match (&self.refuse_rates, &self.rates) {
                (Some(reason), _) => Err(reason.clone()),
                (None, Some(rates)) => Ok(rates.clone()),
                (None, None) => Err("there is no exchange rate source".to_owned()),
            }
        })
    }

    fn calculator_history(
        &self,
        query: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::CalculatorGroupRow>> {
        Box::pin(async move {
            let records: Vec<_> = self
                .calculations
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.question.contains(&query) || r.answer.contains(&query))
                .cloned()
                .collect();
            Ok(if records.is_empty() {
                Vec::new()
            } else {
                vec![crate::backend::CalculatorGroupRow {
                    name: "Today".into(),
                    records,
                }]
            })
        })
    }

    fn add_calculator_record(
        &self,
        question: String,
        answer: String,
        conversion: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            let mut rows = self.calculations.lock().unwrap();
            let id = format!("r{}", rows.len());
            rows.insert(
                0,
                crate::backend::CalculatorRow {
                    id,
                    question,
                    answer,
                    conversion,
                    pinned: false,
                },
            );
            Ok(())
        })
    }

    fn edit_calculator_history(
        &self,
        change: crate::backend::CalculatorChange,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if let crate::backend::CalculatorChange::Remove(id) = &change {
                self.calculations.lock().unwrap().retain(|r| r.id != *id);
            }
            self.calculator_edits.lock().unwrap().push(change);
            Ok(())
        })
    }

    fn revoke_script_grant(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ScriptGrant>> {
        Box::pin(async move {
            let mut grants = self.grants.lock().unwrap();
            grants.retain(|grant| grant.id != id);
            Ok(grants.clone())
        })
    }

    fn set_font(&self, family: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.fonts_set.lock().unwrap().push(family);
            Ok(())
        })
    }

    fn font_specimen(&self, name: String) -> crate::backend::BackendFuture<'_, String> {
        Box::pin(async move { Ok(format!("# {name}\n\nThe quick brown fox\n\n---\n")) })
    }

    fn store_browse(
        &self,
        store: crate::backend::Store,
        query: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::StoreList> {
        Box::pin(async move {
            self.store_queries
                .lock()
                .unwrap()
                .push((store, query.clone()));
            let rows = self
                .store_rows
                .lock()
                .unwrap()
                .iter()
                .filter(|row| row.title.to_lowercase().contains(&query.to_lowercase()))
                .cloned()
                .collect();
            Ok(crate::backend::StoreList {
                heading: if query.is_empty() {
                    "Extensions"
                } else {
                    "Results"
                }
                .into(),
                rows,
            })
        })
    }

    fn store_extension(
        &self,
        _store: crate::backend::Store,
        owner: String,
        name: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::StoreDetail> {
        Box::pin(async move {
            // Only a named owner: the action audit watches this
            // backend's fields, and its rows name none.
            if !owner.is_empty() {
                self.store_owners.lock().unwrap().push(owner);
            }
            let row = self
                .store_rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.name == name)
                .cloned()
                .ok_or("Extension not found")?;
            Ok(crate::backend::StoreDetail {
                markdown: format!("# {}\n\n{}", row.title, row.description),
                row,
                readme_url: Some("https://example.com/README.md".into()),
                ..crate::backend::StoreDetail::default()
            })
        })
    }

    fn store_install(
        &self,
        store: crate::backend::Store,
        owner: String,
        name: String,
    ) -> crate::backend::BackendFuture<'_, (String, String)> {
        Box::pin(async move {
            self.installs
                .lock()
                .unwrap()
                .push((store, owner.clone(), name.clone()));
            // Only a named owner: the action audit watches this
            // backend's fields, and its rows name none.
            if !owner.is_empty() {
                self.store_owners.lock().unwrap().push(owner);
            }
            let mut rows = self.store_rows.lock().unwrap();
            let row = rows
                .iter_mut()
                .find(|row| row.name == name)
                .ok_or("Extension not found")?;
            row.installed = true;
            if let Some(dir) = &self.store_dir {
                let target = dir.join(&row.id);
                fs::create_dir_all(&target).unwrap();
                fs::write(
                    target.join("package.json"),
                    format!(
                        r#"{{"name": "{name}", "title": "{}", "author": "zoe",
                                "commands": [{{"name": "show", "title": "Show The Clock", "mode": "view"}}]}}"#,
                        row.title
                    ),
                )
                .unwrap();
            }
            Ok((row.id.clone(), row.title.clone()))
        })
    }

    fn store_uninstall(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            for row in self.store_rows.lock().unwrap().iter_mut() {
                if row.id == id {
                    row.installed = false;
                }
            }
            if let Some(dir) = &self.store_dir {
                let _ = fs::remove_dir_all(dir.join(&id));
            }
            Ok(())
        })
    }

    fn open_url(&self, url: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.opened_urls.lock().unwrap().push(url);
            self.refuse_opens.clone().map_or(Ok(()), Err)
        })
    }

    fn update_status(
        &self,
    ) -> crate::backend::BackendFuture<'_, Option<crate::backend::UpdateOffer>> {
        Box::pin(async move { Ok(self.update.lock().unwrap().clone()) })
    }

    fn skip_update(&self, tag: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            let mut update = self.update.lock().unwrap();
            if update.as_ref().is_some_and(|offer| offer.tag == tag) {
                *update = None;
            }
            self.skipped.lock().unwrap().push(tag);
            Ok(())
        })
    }

    fn choose_dmenu(
        &self,
        token: u64,
        output: Option<String>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.dmenu_answers.lock().unwrap().push((token, output));
            Ok(())
        })
    }

    fn list_programs(&self) -> crate::backend::BackendFuture<'_, crate::backend::ProgramList> {
        Box::pin(async {
            Ok(crate::backend::ProgramList {
                programs: vec!["/usr/bin/htop".into(), "/usr/bin/top".into()],
                terminal: Some("Ptyxis".into()),
                default_action: "run-in-terminal".into(),
            })
        })
    }

    fn run_program(
        &self,
        argv: Vec<String>,
        terminal: bool,
        hold: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.programs_ran
                .lock()
                .unwrap()
                .push((argv, terminal, hold));
            Ok(())
        })
    }

    fn list_scripts(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
        Box::pin(async move { Ok(self.scripts.clone()) })
    }

    fn run_script(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> crate::backend::BackendFuture<'_, Option<u64>> {
        Box::pin(async move {
            let mode = self
                .scripts
                .iter()
                .find(|script| script.id == id)
                .map(|script| script.mode)
                .ok_or("No script command has that id")?;
            self.script_runs.lock().unwrap().push((id, arguments));
            Ok(match mode {
                compass_core::script_command::OutputMode::Silent
                | compass_core::script_command::OutputMode::Terminal => None,
                _ => Some(7),
            })
        })
    }

    fn script_output(
        &self,
        session: u64,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ScriptOutputState> {
        Box::pin(async move {
            let (id, arguments) = self.script_runs.lock().unwrap().last().cloned().unwrap();
            assert_eq!(session, 7);
            Ok(crate::backend::ScriptOutputState {
                output: format!(
                    "\u{1b}[31m{id}\u{1b}[0m {}\nsecond line",
                    arguments.join(" ")
                ),
                finished: true,
                exit_code: Some(0),
                elapsed_ms: 1200,
            })
        })
    }

    fn list_snippets(&self) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
        Box::pin(async move { Ok(self.snippets.lock().unwrap().clone()) })
    }

    fn save_snippet(
        &self,
        draft: crate::backend::SnippetDraft,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
        Box::pin(async move {
            self.snippet_drafts.lock().unwrap().push(draft.clone());
            let mut snippets = self.snippets.lock().unwrap();
            let id = draft
                .id
                .clone()
                .unwrap_or_else(|| format!("snp-{}", snippets.len()));
            let stored = crate::backend::Snippet {
                id: id.clone(),
                name: draft.name,
                data: compass_core::snippet_store::SnippetData::Text { text: draft.text },
                expansion: draft.keyword.map(|keyword| {
                    compass_core::snippet_store::StoredExpansion {
                        keyword,
                        apps: draft.apps,
                        word: draft.word,
                    }
                }),
                ..crate::backend::Snippet::default()
            };
            match snippets.iter_mut().find(|s| s.id == id) {
                Some(existing) => *existing = stored,
                None => snippets.push(stored),
            }
            Ok(snippets.clone())
        })
    }

    fn remove_snippet(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
        Box::pin(async move {
            let mut snippets = self.snippets.lock().unwrap();
            snippets.retain(|s| s.id != id);
            Ok(snippets.clone())
        })
    }

    fn preview_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> crate::backend::BackendFuture<'_, String> {
        Box::pin(async move {
            let values: Vec<String> = arguments.into_iter().map(|(_, v)| v).collect();
            Ok(format!("preview {id}:{}", values.join(",")))
        })
    }

    fn script_icons(&self) -> crate::backend::BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async move { Ok(self.script_icons.clone()) })
    }

    fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> crate::backend::BackendFuture<'_, String> {
        Box::pin(async move {
            self.snippet_uses
                .lock()
                .unwrap()
                .push((id.clone(), arguments.clone(), false));
            let values: Vec<String> = arguments.into_iter().map(|(_, v)| v).collect();
            Ok(format!("{id}:{}", values.join(",")))
        })
    }

    fn paste_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.snippet_uses
                .lock()
                .unwrap()
                .push((id, arguments, true));
            Ok(())
        })
    }

    fn list_shortcuts(&self) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
        Box::pin(async move { Ok(self.shortcuts.lock().unwrap().clone()) })
    }

    fn save_shortcut(
        &self,
        draft: crate::backend::ShortcutDraft,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
        Box::pin(async move {
            self.drafts.lock().unwrap().push(draft.clone());
            let mut shortcuts = self.shortcuts.lock().unwrap();
            let id = draft
                .id
                .clone()
                .unwrap_or_else(|| format!("sct-{}", shortcuts.len()));
            let stored = crate::backend::Shortcut {
                id: id.clone(),
                name: draft.name,
                icon: draft.icon,
                url: draft.url,
                app: draft.app,
                ..crate::backend::Shortcut::default()
            };
            match shortcuts.iter_mut().find(|s| s.id == id) {
                Some(existing) => *existing = stored,
                None => shortcuts.push(stored),
            }
            Ok(shortcuts.clone())
        })
    }

    fn remove_shortcut(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
        Box::pin(async move {
            let mut shortcuts = self.shortcuts.lock().unwrap();
            shortcuts.retain(|s| s.id != id);
            Ok(shortcuts.clone())
        })
    }

    fn list_openers(
        &self,
        target: String,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::OpenerRow>> {
        Box::pin(async move {
            self.opener_lookups.lock().unwrap().push(target);
            Ok(self.openers.clone())
        })
    }

    fn open_with(&self, app: String, target: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.opened_with.lock().unwrap().push((app, target));
            Ok(())
        })
    }

    fn open_shortcut(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.opened_shortcuts.lock().unwrap().push((id, arguments));
            Ok(())
        })
    }

    fn copy_text(&self, text: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if !self.copies {
                return Err("Copying needs a clipboard".to_owned());
            }
            self.copied.lock().unwrap().push(text);
            Ok(())
        })
    }

    fn paste_text(&self, text: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if self.refuse_paste {
                return Err("Pasting needs the GNOME Shell extension".to_owned());
            }
            self.pasted.lock().unwrap().push(text);
            Ok(())
        })
    }

    fn launch_command(
        &self,
        id: String,
        query: Option<String>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.launched.lock().unwrap().push((id, query));
            Ok(())
        })
    }

    fn expand_shortcut(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> crate::backend::BackendFuture<'_, String> {
        Box::pin(async move {
            let shortcuts = self.shortcuts.lock().unwrap();
            let shortcut = shortcuts.iter().find(|s| s.id == id).ok_or("gone")?;
            Ok(format!("{}|{}", shortcut.url, arguments.join(",")))
        })
    }

    fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionStart> {
        Box::pin(async move {
            self.ran.lock().unwrap().push(id);
            let asked = !self.wants.is_empty() && arguments.is_none();
            self.given.lock().unwrap().push(arguments);
            if let Some(reason) = self.refuse_runs.clone() {
                return Err(reason);
            }
            if !self.needs.is_empty() && self.saved.lock().unwrap().is_empty() {
                return Ok(crate::backend::ExtensionStart::NeedsPreferences {
                    title: "Write Greeting".into(),
                    fields: self.needs.clone(),
                });
            }
            if asked {
                return Ok(crate::backend::ExtensionStart::NeedsArguments {
                    title: "Write Greeting".into(),
                    fields: self.wants.clone(),
                });
            }
            Ok(if self.view.is_some() {
                crate::backend::ExtensionStart::View(7)
            } else {
                crate::backend::ExtensionStart::Ran
            })
        })
    }

    // The first poll gets the view; the next says the command ended, so a
    // test's task loop stops rather than polling for ever.
    fn extension_view(
        &self,
        _session: u64,
        after: u64,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionViewState> {
        Box::pin(async move {
            Ok(crate::backend::ExtensionViewState {
                version: after + 1,
                view: (after == 0)
                    .then(|| self.view.clone().map(Box::new))
                    .flatten(),
                problem: None,
                ended: after > 0,
                depth: *self.depth.lock().unwrap(),
                alert: self.alert.clone(),
                toast: self.toast.clone(),
            })
        })
    }

    fn extension_event(
        &self,
        _session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push((handler, args));
            Ok(())
        })
    }

    fn set_extension_preferences(
        &self,
        _id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.saved.lock().unwrap().push(values);
            Ok(())
        })
    }

    fn extension_alert_answer(
        &self,
        _session: u64,
        confirmed: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.answers.lock().unwrap().push(confirmed);
            Ok(())
        })
    }

    fn extension_alert_remember(&self, _session: u64) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.remembered.lock().unwrap().push(());
            Ok(())
        })
    }

    fn extension_pop(&self, _session: u64) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push(("pop".to_owned(), vec![]));
            Ok(())
        })
    }

    fn close_extension(&self, session: u64) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.closed.lock().unwrap().push(session);
            Ok(())
        })
    }

    fn list_rhai_scripts(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>> {
        let scripts = self.rhai_scripts.clone();
        Box::pin(async move { scripts.ok_or_else(|| "no Rhai scripts here".to_owned()) })
    }

    fn set_setting(
        &self,
        key: String,
        value: serde_json::Value,
    ) -> crate::backend::BackendFuture<'_, ()> {
        if !self.records_settings {
            return Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) });
        }
        self.settings_set.lock().unwrap().push((key, value));
        Box::pin(async { Ok(()) })
    }

    fn set_provider_enabled(
        &self,
        provider: String,
        enabled: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        if !self.records_settings {
            return Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) });
        }
        self.providers_set.lock().unwrap().push((provider, enabled));
        Box::pin(async { Ok(()) })
    }

    fn extension_preferences(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionStart> {
        if !self.records_settings {
            return Box::pin(async {
                Err("Running extension commands needs the Compass engine".to_owned())
            });
        }
        self.preferences_asked.lock().unwrap().push(id.clone());
        Box::pin(async move {
            Ok(crate::backend::ExtensionStart::NeedsPreferences {
                title: id,
                fields: Vec::new(),
            })
        })
    }
}
