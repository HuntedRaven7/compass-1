//! Application search and launch-history services supplied by the composition root.

use std::future::Future;
use std::pin::Pin;

/// An asynchronous backend operation, without a socket dependency in the UI.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The shared application catalog's ranking and successful-launch history.
pub trait ApplicationBackend: std::fmt::Debug + Send + Sync {
    /// Return root-item ENTRYPOINT ids in presentation order.
    ///
    /// `applications:org.mozilla.firefox`, not the launch key
    /// `org.mozilla.firefox.desktop` — this is `QueryHit.id` straight off the
    /// wire, and `AppIndex::position_by_entrypoint` is what resolves it.
    /// `record_launch` below still takes the KEY, because the two ids are
    /// different things and the frecency store is keyed by the launchable.
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>>;

    /// Record an already successful launch; never execute the application again.
    fn record_launch(&self, key: String) -> BackendFuture<'_, ()>;

    /// Keep what the root row's panel changed about the item `id`, and apply
    /// it to the engine's root search. An error is the sentence to show.
    fn edit_root_item(
        &self,
        id: String,
        edit: compass_core::root_items::RootEdit,
    ) -> BackendFuture<'_, ()> {
        let _ = (id, edit);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Run a Power Management command by its id. An error is the sentence to
    /// show.
    fn run_power_command(&self, id: String) -> BackendFuture<'_, ()> {
        let _ = id;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Run a media command by its id, with its optional argument: the player
    /// to fuzzy-match (the default player when `None`), or the volume step.
    /// An error is the sentence to show.
    fn run_media_command(&self, id: String, argument: Option<String>) -> BackendFuture<'_, ()> {
        let _ = (id, argument);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// "Set as Compass font": makes `family` the launcher's font in the
    /// configuration.
    fn set_font(&self, family: String) -> BackendFuture<'_, ()> {
        let _ = family;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Writes one setting of `compass.json` (a key of
    /// `compass_core::settings_catalog`), `null` resetting it. An error is
    /// the sentence to show.
    fn set_setting(&self, key: String, value: serde_json::Value) -> BackendFuture<'_, ()> {
        let _ = (key, value);
        Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) })
    }

    /// The shortcut recorder started or stopped capturing: the engine
    /// releases the global shortcuts meanwhile, so the combination reaches
    /// the recorder. Nothing without an engine, which binds none.
    fn set_shortcut_capture(&self, capturing: bool) -> BackendFuture<'_, ()> {
        let _ = capturing;
        Box::pin(async { Ok(()) })
    }

    /// Whether the desktop would bind `trigger`, which the recorder just
    /// captured (`GlobalShortcutBridge::validate`'s `probeBind`): its
    /// refusal, or `None`. `None` without an engine, which binds nothing.
    fn probe_shortcut(&self, trigger: String) -> BackendFuture<'_, Option<String>> {
        let _ = trigger;
        Box::pin(async { Ok(None) })
    }

    /// Where the launcher hotkey stands with the desktop; with `request`,
    /// the engine asks the desktop for it now and answers after its dialog.
    fn launcher_shortcut(
        &self,
        request: bool,
    ) -> BackendFuture<'_, compass_core::launcher_shortcut::LauncherShortcut> {
        let _ = request;
        Box::pin(async { Err(SHORTCUT_NEEDS_ENGINE.to_owned()) })
    }

    /// Whether Compass starts at login; with `enable`, turns it on or off
    /// first. An error is the sentence to show, and nothing changed.
    fn login_start(
        &self,
        enable: Option<bool>,
    ) -> BackendFuture<'_, compass_core::login_start::Status> {
        let _ = enable;
        Box::pin(async { Err(LOGIN_START_NEEDS_ENGINE.to_owned()) })
    }

    /// Turns a whole provider's items on or off in root search.
    fn set_provider_enabled(&self, provider: String, enabled: bool) -> BackendFuture<'_, ()> {
        let _ = (provider, enabled);
        Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) })
    }

    /// What the user has allowed their own Rhai scripts.
    fn list_script_grants(&self) -> BackendFuture<'_, Vec<ScriptGrant>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Withdraws what a Rhai script was allowed, answering with the list
    /// after the change.
    fn revoke_script_grant(&self, id: String) -> BackendFuture<'_, Vec<ScriptGrant>> {
        let _ = id;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// How many times the engine has rescanned its catalog because an
    /// application or extension directory changed. A window whose last
    /// answer differs scans its own copy again.
    fn catalog_generation(&self) -> BackendFuture<'_, u64> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The applications that open `target` (a path or a URL), the default
    /// first: what "Open with…" lists.
    fn list_openers(&self, target: String) -> BackendFuture<'_, Vec<OpenerRow>> {
        let _ = target;
        Box::pin(async { Err(OPEN_WITH_NEEDS_ENGINE.to_owned()) })
    }

    /// Opens `target` with the application `app`. An error is the sentence
    /// to show.
    fn open_with(&self, app: String, target: String) -> BackendFuture<'_, ()> {
        let _ = (app, target);
        Box::pin(async { Err(OPEN_WITH_NEEDS_ENGINE.to_owned()) })
    }

    /// Inspect Local Storage: the namespaces that hold something.
    fn local_storage_namespaces(&self) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async { Err(STORAGE_NEEDS_ENGINE.to_owned()) })
    }

    /// Inspect Local Storage: one namespace's items, by key.
    fn local_storage_items(&self, namespace: String) -> BackendFuture<'_, Vec<StorageItemRow>> {
        let _ = namespace;
        Box::pin(async { Err(STORAGE_NEEDS_ENGINE.to_owned()) })
    }

    /// Manage OAuth Token Sets: every stored token set.
    fn oauth_token_sets(&self) -> BackendFuture<'_, Vec<TokenSetRow>> {
        Box::pin(async { Err(STORAGE_NEEDS_ENGINE.to_owned()) })
    }

    /// Manage OAuth Token Sets: removes one. An error is the sentence to show.
    fn remove_oauth_token_set(
        &self,
        extension_id: String,
        provider_id: Option<String>,
    ) -> BackendFuture<'_, ()> {
        let _ = (extension_id, provider_id);
        Box::pin(async { Err(STORAGE_NEEDS_ENGINE.to_owned()) })
    }

    /// What a file's action panel depends on.
    fn file_actions(&self, path: String) -> BackendFuture<'_, FileActions> {
        let _ = path;
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// Puts a file on the clipboard as a file, and with `paste` pastes it
    /// into the focused window. An error is the sentence to show.
    fn copy_file(&self, path: String, paste: bool) -> BackendFuture<'_, ()> {
        let _ = (path, paste);
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// Runs a file as a program, making it executable first when asked.
    fn run_executable(&self, path: String, make_executable: bool) -> BackendFuture<'_, ()> {
        let _ = (path, make_executable);
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// Makes an image the wallpaper.
    fn set_wallpaper(&self, path: String) -> BackendFuture<'_, ()> {
        let _ = path;
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// What a default picker offers, the current default first.
    fn list_default_apps(&self, kind: DefaultApp) -> BackendFuture<'_, Vec<DefaultAppRow>> {
        let _ = kind;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Makes `id` the default browser or terminal. An error is the sentence
    /// to show.
    fn set_default_app(&self, kind: DefaultApp, id: String) -> BackendFuture<'_, ()> {
        let _ = (kind, id);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The exchange rates the engine holds, for currency conversions; `None`
    /// when it has none.
    fn exchange_rates(
        &self,
    ) -> BackendFuture<'_, Option<compass_core::exchange_rates::ExchangeRates>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Refresh Exchange Rates: the engine fetches them now. An error is the
    /// sentence to show.
    fn refresh_exchange_rates(
        &self,
    ) -> BackendFuture<'_, compass_core::exchange_rates::ExchangeRates> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The calculator's history matching `query`, in its non-empty groups.
    fn calculator_history(&self, query: String) -> BackendFuture<'_, Vec<CalculatorGroupRow>> {
        let _ = query;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Remembers a calculation whose answer was copied.
    fn add_calculator_record(
        &self,
        question: String,
        answer: String,
        conversion: bool,
    ) -> BackendFuture<'_, ()> {
        let _ = (question, answer, conversion);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Pins, unpins or removes a remembered calculation, or all of them.
    fn edit_calculator_history(&self, change: CalculatorChange) -> BackendFuture<'_, ()> {
        let _ = change;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Other applications' tray icons, for Search Tray.
    fn tray_items(&self) -> BackendFuture<'_, Vec<TrayItemRow>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Activates a tray item, or its secondary activation.
    fn tray_activate(&self, key: String, secondary: bool) -> BackendFuture<'_, ()> {
        let _ = (key, secondary);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// A tray item's menu, flattened.
    fn tray_menu(&self, key: String) -> BackendFuture<'_, Vec<TrayMenuRow>> {
        let _ = key;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Clicks one entry of a tray item's menu.
    fn tray_trigger(&self, key: String, id: i32) -> BackendFuture<'_, ()> {
        let _ = (key, id);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The running media players, for Now Playing.
    fn list_media_players(&self) -> BackendFuture<'_, Vec<MediaPlayerRow>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Play/pause, skip or go back on one player, by its bus name.
    fn control_media_player(&self, player: String, action: MediaAction) -> BackendFuture<'_, ()> {
        let _ = (player, action);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Search Files: what `query` answers within `category` (a filter key,
    /// `None` for all), with the list's heading. An error is the sentence to
    /// show.
    fn search_files(
        &self,
        query: String,
        category: Option<String>,
    ) -> BackendFuture<'_, FileResults> {
        let _ = (query, category);
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// Open a file with its default application, or show it in the file
    /// browser when `reveal`. An error is the sentence to show.
    fn open_file(&self, path: String, reveal: bool) -> BackendFuture<'_, ()> {
        let _ = (path, reveal);
        Box::pin(async { Err(FILES_NEED_ENGINE.to_owned()) })
    }

    /// Every stored shortcut, in the store's order.
    fn list_shortcuts(&self) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async { Err(SHORTCUTS_NEED_ENGINE.to_owned()) })
    }

    /// Creates a shortcut (`id` is `None`) or updates one, answering with the
    /// list after the change. `icon` may be `default`, which the engine
    /// resolves.
    fn save_shortcut(&self, shortcut: ShortcutDraft) -> BackendFuture<'_, Vec<Shortcut>> {
        let _ = shortcut;
        Box::pin(async { Err(SHORTCUTS_NEED_ENGINE.to_owned()) })
    }

    /// Removes a shortcut, answering with the list after the change.
    fn remove_shortcut(&self, id: String) -> BackendFuture<'_, Vec<Shortcut>> {
        let _ = id;
        Box::pin(async { Err(SHORTCUTS_NEED_ENGINE.to_owned()) })
    }

    /// Launches a root item through the engine, as `compass cmd launch`
    /// does: an extension command comes back to the window as a launch, with
    /// `query` as its fallback text.
    fn launch_command(&self, id: String, query: Option<String>) -> BackendFuture<'_, ()> {
        let _ = (id, query);
        Box::pin(async {
            Err(
                "Launching a command needs the Compass engine, and this window is \
                 running without one"
                    .to_owned(),
            )
        })
    }

    /// Puts `text` on a clipboard the engine owns, so it is still there once
    /// the window hides. An error means the engine reaches no clipboard, and
    /// the caller copies through the window's own.
    fn copy_text(&self, text: String) -> BackendFuture<'_, ()> {
        let _ = text;
        Box::pin(async { Err("Copying needs the Compass engine".to_owned()) })
    }

    /// Puts `text` on the clipboard and pastes it where the person was. An
    /// error means the engine cannot paste here, and the caller copies.
    fn paste_text(&self, text: String) -> BackendFuture<'_, ()> {
        let _ = text;
        Box::pin(async { Err("Pasting needs the Compass engine".to_owned()) })
    }

    /// Opens a shortcut with its arguments. An error is the sentence to show.
    fn open_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, ()> {
        let _ = (id, arguments);
        Box::pin(async { Err(SHORTCUTS_NEED_ENGINE.to_owned()) })
    }

    /// A shortcut's link, expanded with its arguments, without opening it.
    fn expand_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, String> {
        let _ = (id, arguments);
        Box::pin(async { Err(SHORTCUTS_NEED_ENGINE.to_owned()) })
    }

    /// Every stored snippet, in the store's order.
    fn list_snippets(&self) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// Creates a snippet (`id` is `None`) or updates one, answering with the
    /// list after the change. An error is the sentence to show.
    fn save_snippet(&self, snippet: SnippetDraft) -> BackendFuture<'_, Vec<Snippet>> {
        let _ = snippet;
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// Removes a snippet, answering with the list after the change.
    fn remove_snippet(&self, id: String) -> BackendFuture<'_, Vec<Snippet>> {
        let _ = id;
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// A snippet expanded with its arguments, to copy.
    fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        let _ = (id, arguments);
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// A snippet expanded for the detail pane, its shell placeholders shown
    /// rather than run.
    fn preview_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        let _ = (id, arguments);
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// Expands a snippet and pastes it into the focused window.
    fn paste_snippet(&self, id: String, arguments: Vec<(String, String)>) -> BackendFuture<'_, ()> {
        let _ = (id, arguments);
        Box::pin(async { Err(SNIPPETS_NEED_ENGINE.to_owned()) })
    }

    /// Generates a new extension's boilerplate: where it was written, or the
    /// sentence saying why not.
    fn create_extension(&self, draft: ExtensionDraft) -> BackendFuture<'_, String> {
        let _ = draft;
        Box::pin(async { Err("Create Extension needs the Compass engine".to_owned()) })
    }

    /// Keeps a theme in the configuration, by its persisted name.
    fn set_theme(&self, theme: String) -> BackendFuture<'_, ()> {
        let _ = theme;
        Box::pin(async { Err("Set Theme needs the Compass engine to keep the theme".to_owned()) })
    }

    /// The installed font families, as Browse Fonts lists them.
    fn list_fonts(&self) -> BackendFuture<'_, FontList> {
        Box::pin(async { Err("Browse Fonts needs the Compass engine".to_owned()) })
    }

    /// A family's specimen, as Markdown.
    fn font_specimen(&self, name: String) -> BackendFuture<'_, String> {
        let _ = name;
        Box::pin(async { Err("Browse Fonts needs the Compass engine".to_owned()) })
    }

    /// A store's rows for `query`: the Vicinae store's list filtered, the
    /// Raycast store's first page or its search results.
    fn store_browse(&self, store: Store, query: String) -> BackendFuture<'_, StoreList> {
        let _ = (store, query);
        Box::pin(async { Err("The extension stores need the Compass engine".to_owned()) })
    }

    /// One store extension's detail page, by the handle the store files it
    /// under ([`StoreRow::owner`]) and its name.
    fn store_extension(
        &self,
        store: Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, StoreDetail> {
        let _ = (store, owner, name);
        Box::pin(async { Err("The extension stores need the Compass engine".to_owned()) })
    }

    /// Downloads and installs a store extension, by [`StoreRow::owner`] and
    /// its name, answering its id and title.
    fn store_install(
        &self,
        store: Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, (String, String)> {
        let _ = (store, owner, name);
        Box::pin(async { Err("Installing extensions needs the Compass engine".to_owned()) })
    }

    /// Uninstalls the extension installed as `id`.
    fn store_uninstall(&self, id: String) -> BackendFuture<'_, ()> {
        let _ = id;
        Box::pin(async { Err("Uninstalling extensions needs the Compass engine".to_owned()) })
    }

    /// Opens an `http(s)` URL in the default browser.
    fn open_url(&self, url: String) -> BackendFuture<'_, ()> {
        let _ = url;
        Box::pin(async { Err("Opening links needs the Compass engine".to_owned()) })
    }

    /// The launch an extension asked for, which the engine holds under
    /// `token` (`launchCommand`, `openCommandPreferences`).
    fn fetch_launch(&self, token: u64) -> BackendFuture<'_, ExtensionLaunch> {
        let _ = token;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The subtitles extensions set for their commands, by command id.
    fn extension_subtitles(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The preferences form of the extension command `id`, without running
    /// it: [`ExtensionStart::NeedsPreferences`].
    fn extension_preferences(&self, id: String) -> BackendFuture<'_, ExtensionStart> {
        let _ = id;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The `compass dmenu` list the engine holds under `token`.
    fn fetch_dmenu(&self, token: u64) -> BackendFuture<'_, DmenuList> {
        let _ = token;
        Box::pin(async { Err("dmenu needs the Compass engine".to_owned()) })
    }

    /// Answers the dmenu list under `token`: what to print, or `None` when it
    /// was dismissed.
    fn choose_dmenu(&self, token: u64, output: Option<String>) -> BackendFuture<'_, ()> {
        let _ = (token, output);
        Box::pin(async { Err("dmenu needs the Compass engine".to_owned()) })
    }

    /// The executables on `PATH`, the terminal they would run in, and the
    /// default action.
    fn list_programs(&self) -> BackendFuture<'_, ProgramList> {
        Box::pin(async { Err(PROGRAMS_NEED_ENGINE.to_owned()) })
    }

    /// Runs a command line, in a terminal (kept open when `hold`) or
    /// directly.
    fn run_program(&self, argv: Vec<String>, terminal: bool, hold: bool) -> BackendFuture<'_, ()> {
        let _ = (argv, terminal, hold);
        Box::pin(async { Err(PROGRAMS_NEED_ENGINE.to_owned()) })
    }

    /// Every script command, scanned afresh.
    fn list_scripts(&self) -> BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
        Box::pin(async { Err(SCRIPTS_NEED_ENGINE.to_owned()) })
    }

    /// Each script command's icon, `(id, icon:// URL)`.
    fn script_icons(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async { Err(SCRIPTS_NEED_ENGINE.to_owned()) })
    }

    /// Every Rhai script the engine has loaded, rescanned.
    fn list_rhai_scripts(
        &self,
    ) -> BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>> {
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Runs a script command with its arguments: the run to follow, or
    /// `None` when there is nothing to follow (silent and terminal modes).
    fn run_script(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, Option<u64>> {
        let _ = (id, arguments);
        Box::pin(async { Err(SCRIPTS_NEED_ENGINE.to_owned()) })
    }

    /// What a script run has printed so far.
    fn script_output(&self, session: u64) -> BackendFuture<'_, ScriptOutputState> {
        let _ = session;
        Box::pin(async { Err(SCRIPTS_NEED_ENGINE.to_owned()) })
    }

    /// Stops a script run.
    fn stop_script(&self, session: u64) -> BackendFuture<'_, ()> {
        let _ = session;
        Box::pin(async { Err(SCRIPTS_NEED_ENGINE.to_owned()) })
    }

    /// Run an installed extension's command by its entrypoint id, with the
    /// argument values entered for it, or `None` when none have been. `Ok`
    /// once the engine has started it; an error is a sentence saying why it
    /// could not, for the launcher to show.
    fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> BackendFuture<'_, ExtensionStart> {
        let _ = (id, arguments);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// A view session's state once its version passes `after` (or a timeout,
    /// with the same version).
    fn extension_view(&self, session: u64, after: u64) -> BackendFuture<'_, ExtensionViewState> {
        let _ = (session, after);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Runs one of a view's callbacks with `args`.
    fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        let _ = (session, handler, args);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Keeps an extension's preference values, by the command's id.
    fn set_extension_preferences(
        &self,
        id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        let _ = (id, values);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The person's answer to the view's [`ExtensionPrompt`].
    fn extension_alert_answer(&self, session: u64, confirmed: bool) -> BackendFuture<'_, ()> {
        let _ = (session, confirmed);
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The person chose the alert's remembered answer (Ctrl+Enter).
    fn extension_alert_remember(&self, session: u64) -> BackendFuture<'_, ()> {
        let _ = session;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Escape on a pushed view: the extension pops it.
    fn extension_pop(&self, session: u64) -> BackendFuture<'_, ()> {
        let _ = session;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// The person left the view: stop the command.
    fn close_extension(&self, session: u64) -> BackendFuture<'_, ()> {
        let _ = session;
        Box::pin(async { Err(NEEDS_ENGINE.to_owned()) })
    }

    /// Asks the desktop's file chooser for what a form's file picker takes:
    /// the absolute paths chosen, empty when the person cancelled. An error
    /// is the sentence to show.
    fn choose_files(
        &self,
        choice: crate::extension_fields::FileChoice,
    ) -> BackendFuture<'_, Vec<String>> {
        let _ = choice;
        Box::pin(async { Err("Choosing files needs the desktop's file chooser".to_owned()) })
    }

    /// Whether a newer Compass release is out (`UpdateService::available`):
    /// `None` when it is not, or checking is off. Nothing without an engine,
    /// which is the only one that asks.
    fn update_status(&self) -> BackendFuture<'_, Option<UpdateOffer>> {
        Box::pin(async { Ok(None) })
    }

    /// Never offer the release `tag` again (`skipAvailableVersion`). An
    /// error is the sentence to show.
    fn skip_update(&self, tag: String) -> BackendFuture<'_, ()> {
        let _ = tag;
        Box::pin(async { Err("Skipping an update needs the Compass engine".to_owned()) })
    }
}

/// A newer Compass release, as the root search's Update section offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    /// The release's tag, such as `v1.2.0`.
    pub tag: String,
    /// The tag without its leading `v`.
    pub version: String,
    /// The release page.
    pub release_url: String,
    /// The version running, such as `v0.1.0`.
    pub current: String,
}

const NEEDS_ENGINE: &str = "Running extension commands needs the Compass engine";

/// What the storage views say without an engine.
pub const STORAGE_NEEDS_ENGINE: &str = "Reading extension storage needs the Compass engine";

const FILES_NEED_ENGINE: &str =
    "Search Files needs the Compass engine, and this window is running without one";

const SHORTCUTS_NEED_ENGINE: &str =
    "Shortcuts need the Compass engine, and this window is running without one";

/// Which system default a picker sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultApp {
    /// Set Default Browser.
    Browser,
    /// Set Default Terminal.
    Terminal,
}

/// One application a default picker offers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultAppRow {
    /// The desktop file id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its comment.
    pub description: String,
    /// Whether it is the current default.
    pub is_default: bool,
}

/// One group of the calculator's history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalculatorGroupRow {
    /// The section's name, e.g. `Today`.
    pub name: String,
    /// Its rows.
    pub records: Vec<CalculatorRow>,
}

/// One remembered calculation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalculatorRow {
    /// Its id.
    pub id: String,
    /// What was asked.
    pub question: String,
    /// What came back.
    pub answer: String,
    /// A unit conversion rather than arithmetic.
    pub conversion: bool,
    /// Whether it is pinned.
    pub pinned: bool,
}

/// A change to the calculator's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalculatorChange {
    /// Pin a row, by id.
    Pin(String),
    /// Unpin a row, by id.
    Unpin(String),
    /// Remove a row, by id.
    Remove(String),
    /// Remove every row.
    RemoveAll,
}

/// What the user has allowed one Rhai script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptGrant {
    /// The script's id.
    pub id: String,
    /// Its title, or its id when it is no longer installed.
    pub title: String,
    /// The capabilities allowed.
    pub capabilities: Vec<String>,
    /// The same, in the consent prompt's words.
    pub descriptions: Vec<String>,
}

/// One application's tray icon, as Search Tray lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayItemRow {
    /// What the other tray calls name it by.
    pub key: String,
    /// Its title, else its id.
    pub title: String,
    /// Its tooltip.
    pub subtitle: String,
    /// It is asking for attention.
    pub attention: bool,
    /// It has a menu to browse.
    pub has_menu: bool,
    /// The whole item is a menu.
    pub item_is_menu: bool,
    /// Its icon as a file.
    pub icon_path: Option<String>,
    /// Its icon as a theme name.
    pub icon_name: Option<String>,
    /// Its icon as PNG bytes.
    pub icon_png: Option<Vec<u8>>,
}

/// One clickable entry of a tray item's menu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayMenuRow {
    /// Its id, for [`ApplicationBackend::tray_trigger`].
    pub id: i32,
    /// Its label, after its submenus'.
    pub label: String,
    /// For a toggle, whether it is on.
    pub toggled: Option<bool>,
    /// Its icon's theme name.
    pub icon_name: Option<String>,
}

/// One running media player, as Now Playing lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPlayerRow {
    /// Its bus name.
    pub id: String,
    /// What it calls itself.
    pub identity: String,
    /// Its desktop entry id, when it names one.
    pub app_id: String,
    /// The current track's title.
    pub title: String,
    /// The current track's artists.
    pub artist: String,
    /// Whether it is playing.
    pub playing: bool,
    /// Whether it is paused.
    pub paused: bool,
    /// Whether it has a next track.
    pub can_go_next: bool,
    /// Whether it has a previous track.
    pub can_go_previous: bool,
}

/// What Now Playing asks a player to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    /// Toggle playback.
    PlayPause,
    /// Skip to the next track.
    Next,
    /// Go back to the previous track.
    Previous,
}

/// The Create Extension form's values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionDraft {
    /// Who is writing it.
    pub author: String,
    /// The extension's title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// The directory to create it in.
    pub location: String,
    /// The first command's title.
    pub command_title: String,
    /// The first command's description.
    pub command_description: String,
    /// The command template's resource id.
    pub template: String,
}

pub use compass_core::store_listing::Store;

/// A store's rows and the heading over them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreList {
    /// `Extensions` or `Results`.
    pub heading: String,
    /// The rows, in order.
    pub rows: Vec<StoreRow>,
}

/// One store extension, as a row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreRow {
    /// The id it installs under.
    pub id: String,
    /// Its name in the store.
    pub name: String,
    /// Its author's handle.
    pub author: String,
    /// Its author's display name.
    pub author_name: String,
    /// Its title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// Its icon's URL for a light theme.
    pub icon_light: Option<String>,
    /// Its icon's URL for a dark theme.
    pub icon_dark: Option<String>,
    /// Its download count, formatted.
    pub downloads: String,
    /// Whether it is installed.
    pub installed: bool,
    /// Whether the store serves a newer build than the one installed.
    pub update_available: bool,
    /// Its Raycast compatibility tier, where there is a sheet.
    pub compat: Option<u8>,
    /// Its author's avatar URL.
    pub author_avatar: Option<String>,
    /// The handle the store files it under, which opening and installing it
    /// go by: an organisation's for a Raycast extension one owns (`raycast`
    /// for GitHub, whose author is `thomaslombart`), else the author's.
    pub owner: String,
}

/// One store extension's detail page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreDetail {
    /// Its row.
    pub row: StoreRow,
    /// The page's Markdown.
    pub markdown: String,
    /// Screenshot URLs.
    pub screenshots: Vec<String>,
    /// Where its README is.
    pub readme_url: Option<String>,
    /// Where its source is.
    pub source_url: Option<String>,
    /// Its page on the store's website.
    pub store_url: Option<String>,
}

/// Browse Fonts' families and its filter's categories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontList {
    /// The families, in the browser's order.
    pub fonts: Vec<FontListEntry>,
    /// The category names, in the filter's order.
    pub categories: Vec<String>,
}

/// One family in Browse Fonts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontListEntry {
    /// The typeface's name, its members folded together.
    pub name: String,
    /// The member to draw it with.
    pub family: String,
    /// The glyph its row shows.
    pub glyph: Option<String>,
    /// Whether it is a colour emoji font.
    pub color: bool,
    /// The category it is listed under.
    pub primary: String,
    /// Every category it can be filtered by.
    pub categories: Vec<String>,
}

/// A `compass dmenu` list and its options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DmenuList {
    /// The entries, one per line.
    pub content: String,
    /// `--navigation-title`.
    pub navigation_title: Option<String>,
    /// `--section-title`, with `{count}`.
    pub section_title: Option<String>,
    /// `--format index`.
    pub output_index: bool,
    /// `--placeholder`.
    pub placeholder: Option<String>,
    /// `--query`.
    pub query: Option<String>,
    /// `--no-section`.
    pub no_section: bool,
    /// `--no-quick-look`, or a width under 500.
    pub no_quick_look: bool,
    /// `--width`.
    pub width: Option<u32>,
    /// `--height`.
    pub height: Option<u32>,
    /// `--no-metadata`.
    pub no_metadata: bool,
    /// `--no-footer`, or a width under 500.
    pub no_footer: bool,
}

const PROGRAMS_NEED_ENGINE: &str =
    "Run Terminal Program needs the Compass engine, and this window is running without one";

/// What Run Terminal Program offers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramList {
    /// Every executable, by path.
    pub programs: Vec<String>,
    /// The terminal's name, when one is installed.
    pub terminal: Option<String>,
    /// The `default-action` preference.
    pub default_action: String,
}

const SCRIPTS_NEED_ENGINE: &str =
    "Script commands need the Compass engine, and this window is running without one";

/// What a script run has printed, and whether it has ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptOutputState {
    /// Everything read so far.
    pub output: String,
    /// Whether it has ended.
    pub finished: bool,
    /// Its exit code, when it exited normally.
    pub exit_code: Option<i32>,
    /// How long it has run, or ran.
    pub elapsed_ms: u64,
}

const SNIPPETS_NEED_ENGINE: &str =
    "Snippets need the Compass engine, and this window is running without one";

/// A stored snippet, as the engine lists it.
pub type Snippet = compass_core::snippet_store::SerializedSnippet;

/// A text snippet to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetDraft {
    /// The snippet to update, or `None` for a new one.
    pub id: Option<String>,
    /// Its name.
    pub name: String,
    /// Its text.
    pub text: String,
    /// Its keyword, if any.
    pub keyword: Option<String>,
    /// Whether the keyword waits for a word boundary.
    pub word: bool,
    /// The applications the keyword is limited to.
    pub apps: Vec<String>,
}

/// A stored shortcut, as the engine lists it.
pub type Shortcut = compass_core::shortcut_store::SerializedShortcut;

/// A shortcut to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutDraft {
    /// The shortcut to update, or `None` for a new one.
    pub id: Option<String>,
    /// Its name; may be empty.
    pub name: String,
    /// Its icon URL, or `default`.
    pub icon: String,
    /// The link.
    pub url: String,
    /// The application id, or `default`.
    pub app: String,
}

/// One file in Search Files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    /// Absolute path.
    pub path: String,
    /// The last path component.
    pub name: String,
    /// Its category's filter key, e.g. `Documents`.
    pub category: String,
}

/// What a Search Files query answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileResults {
    /// What the list is, e.g. "Recently Accessed".
    pub heading: String,
    /// The files, in presentation order.
    pub files: Vec<FileRow>,
}

/// A launch an extension asked the launcher to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLaunch {
    /// The command's root id.
    pub id: String,
    /// Its arguments, when the extension passed any.
    pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
    /// Open its preferences form rather than run it.
    pub preferences: bool,
    /// What its search starts with (`compass cmd launch --query`).
    pub fallback_text: Option<String>,
}

/// How an extension command began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionStart {
    /// A no-view command, running on its own.
    Ran,
    /// A view command: follow this session.
    View(u64),
    /// It did not start: a required preference has no value. The form.
    NeedsPreferences {
        /// The command's title.
        title: String,
        /// Every preference it reads.
        fields: Vec<PreferenceInput>,
    },
    /// It did not start: it takes arguments, and has not been given them
    /// (or a required one is empty). The form.
    NeedsArguments {
        /// The command's title.
        title: String,
        /// Every argument, with what was already entered.
        fields: Vec<PreferenceInput>,
    },
}

/// One preference in the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceInput {
    /// The name the extension reads it by.
    pub name: String,
    /// The label.
    pub title: String,
    /// Help text; may be empty.
    pub description: String,
    /// Placeholder; may be empty.
    pub placeholder: String,
    /// Whether the command cannot run without it.
    pub required: bool,
    /// What it takes.
    pub kind: PreferenceInputKind,
    /// Its current value.
    pub value: Option<serde_json::Value>,
}

/// What a [`PreferenceInput`] takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferenceInputKind {
    /// Text.
    Text,
    /// Text, hidden.
    Password,
    /// A tick box.
    Checkbox {
        /// Its label.
        label: String,
    },
    /// One of a list, as `(title, value)`.
    Dropdown {
        /// The options.
        options: Vec<(String, String)>,
    },
    /// Several lines of text.
    TextArea,
    /// A kind the form cannot edit yet.
    Unsupported {
        /// What the manifest calls it.
        declared: String,
    },
}

/// What an extension view shows now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtensionViewState {
    /// Bumped on every change.
    pub version: u64,
    /// The view, once rendered.
    pub view: Option<Box<compass_extension_api::View>>,
    /// Why it cannot be drawn, or why it ended.
    pub problem: Option<String>,
    /// Whether the command has ended.
    pub ended: bool,
    /// How many views the extension has pushed, the root one included.
    pub depth: u32,
    /// A confirmation the extension waits on.
    pub alert: Option<ExtensionPrompt>,
    /// The toast it shows, as `(style, title, message)`.
    pub toast: Option<ExtensionToast>,
}

/// A toast an extension shows over its view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionToast {
    /// Whether it reports a failure, which the footer draws in the danger
    /// colour.
    pub failure: bool,
    /// Still working: drawn with an ellipsis.
    pub animated: bool,
    /// The heading.
    pub title: String,
    /// More text; may be empty.
    pub message: String,
}

/// A confirmation an extension asked for, as the launcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtensionPrompt {
    /// The heading.
    pub title: String,
    /// The body; may be empty.
    pub message: String,
    /// What Enter does.
    pub confirm_text: String,
    /// What Escape does.
    pub cancel_text: String,
    /// What Ctrl+Enter does, when the alert has a third, remembered answer
    /// ("Always Allow").
    pub remember_text: Option<String>,
}

/// One clipboard history row, as the UI draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardRow {
    /// Stable id, used to fetch the content.
    pub id: String,
    /// What the row says: the start of copied text, or a label.
    pub preview: String,
    /// What kind of thing was copied.
    pub kind: ClipboardRowKind,
    /// Whether it is pinned to the top.
    pub pinned: bool,
    /// For links, the host.
    pub url_host: Option<String>,
}

/// What kind of thing a [`ClipboardRow`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRowKind {
    /// Plain text.
    Text,
    /// A URL.
    Link,
    /// An image.
    Image,
    /// Files.
    File,
    /// Anything else.
    Unknown,
}

/// One entry's full content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardContent {
    /// MIME type of `data`.
    pub mime_type: String,
    /// The content as copied.
    pub data: Vec<u8>,
}

/// Clipboard history, which only the engine holds.
///
/// Separate from [`ApplicationBackend`] because a window can search
/// applications on its own and cannot read clipboard history on its own: the
/// store is the engine's, behind its keyring.
pub trait ClipboardBackend: std::fmt::Debug + Send + Sync {
    /// Entries matching `query`, pinned first then newest; empty lists all.
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>>;

    /// One entry's full content, for copying it back.
    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent>;

    /// Put one entry on the clipboard and paste it into the window focus
    /// moves to next. Ask while the launcher is focused, then hide it. A
    /// refusal (no GNOME Shell extension) means the caller copies instead.
    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()>;

    /// Pin or unpin one entry.
    fn clipboard_set_pinned(&self, id: String, pinned: bool) -> BackendFuture<'_, ()>;

    /// Remove one entry and its stored content.
    fn clipboard_remove(&self, id: String) -> BackendFuture<'_, ()>;

    /// [`Self::clipboard_history`] restricted to one kind, the view's filter;
    /// `None` is every kind.
    fn clipboard_history_of_kind(
        &self,
        query: String,
        limit: u32,
        kind: Option<ClipboardRowKind>,
    ) -> BackendFuture<'_, Vec<ClipboardRow>> {
        match kind {
            None => self.clipboard_history(query, limit),
            Some(_) => Box::pin(async { Err(CLIPBOARD_NEEDS_ENGINE.to_owned()) }),
        }
    }

    /// What the detail pane shows about one entry besides its content.
    fn clipboard_detail(&self, id: String) -> BackendFuture<'_, ClipboardDetail> {
        let _ = id;
        Box::pin(async { Err(CLIPBOARD_NEEDS_ENGINE.to_owned()) })
    }

    /// Set the words an entry is also found by; empty clears them.
    fn clipboard_set_keywords(&self, id: String, keywords: String) -> BackendFuture<'_, ()> {
        let _ = (id, keywords);
        Box::pin(async { Err(CLIPBOARD_NEEDS_ENGINE.to_owned()) })
    }

    /// Remove every entry (sparing tagged ones when the preference says so).
    fn clipboard_remove_all(&self) -> BackendFuture<'_, ()> {
        Box::pin(async { Err(CLIPBOARD_NEEDS_ENGINE.to_owned()) })
    }

    /// Whether copies are being recorded, after turning recording on or off
    /// when `enabled` is given.
    fn clipboard_monitoring(
        &self,
        enabled: Option<bool>,
    ) -> BackendFuture<'_, ClipboardMonitoring> {
        let _ = enabled;
        Box::pin(async { Err(CLIPBOARD_NEEDS_ENGINE.to_owned()) })
    }
}

const CLIPBOARD_NEEDS_ENGINE: &str = "Clipboard history needs the Compass engine";

/// What the detail pane shows about one entry besides its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardDetail {
    /// The entry.
    pub id: String,
    /// The MIME type of what was copied.
    pub mime_type: String,
    /// What kind of thing it is.
    pub kind: ClipboardRowKind,
    /// Its size in bytes.
    pub size: i64,
    /// Its MD5.
    pub md5: String,
    /// When it was last copied, in milliseconds since the epoch.
    pub updated_at: i64,
    /// Whether it is encrypted at rest.
    pub encrypted: bool,
    /// The words it is also found by.
    pub keywords: String,
}

/// Whether copies are being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardMonitoring {
    /// Whether the engine can record copies on this desktop at all.
    pub supported: bool,
    /// Whether it is recording them.
    pub enabled: bool,
}

/// One open window, as the switcher draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    /// Handle for activate and close.
    pub id: u32,
    /// The window's title.
    pub title: String,
    /// The application's name when recognised, else its `WM_CLASS`.
    pub app: String,
    /// Its `WM_CLASS`, searched at a low weight.
    pub wm_class: String,
    /// The owning process, so the launcher can leave out its own window.
    pub pid: Option<u32>,
    /// Whether it can be closed.
    pub can_close: bool,
    /// Whether the engine recognised its application, which is what Quit
    /// and Force Quit act on.
    pub app_known: bool,
}

/// Whether an application is running, as its root row's panel asks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppRuntimeInfo {
    /// It has a window.
    pub running: bool,
    /// One of its windows has focus.
    pub frontmost: bool,
    /// Its windows, the first the one Focus Window raises.
    pub windows: Vec<WindowRow>,
}

/// Window switching, which only the engine can do (through the Shell
/// extension).
pub trait WindowBackend: std::fmt::Debug + Send + Sync {
    /// The open windows, the one worth switching to first.
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>>;

    /// Focus and raise a window.
    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()>;

    /// Ask a window to close.
    fn close_window(&self, id: u32) -> BackendFuture<'_, ()>;

    /// Whether the application with this desktop id is running.
    fn app_runtime(&self, id: String) -> BackendFuture<'_, AppRuntimeInfo> {
        let _ = id;
        Box::pin(async { Err("Quitting applications needs the engine".to_owned()) })
    }

    /// Quit (or with `force`, Force Quit) the application with this
    /// desktop id.
    fn quit_app(&self, id: String, force: bool) -> BackendFuture<'_, ()> {
        let _ = (id, force);
        Box::pin(async { Err("Quitting applications needs the engine".to_owned()) })
    }

    /// Quit (or Force Quit) the application a window belongs to.
    fn quit_window_app(&self, window: u32, force: bool) -> BackendFuture<'_, ()> {
        let _ = (window, force);
        Box::pin(async { Err("Quitting applications needs the engine".to_owned()) })
    }

    /// What the compositor's window manager can do, which decides the
    /// window-management commands root search offers.
    fn window_manager_capabilities(
        &self,
    ) -> BackendFuture<'_, compass_core::window_switcher::Capabilities> {
        Box::pin(async { Err(WORKSPACES_NEED_ENGINE.to_owned()) })
    }

    /// The workspaces, for Switch Workspaces.
    fn list_workspaces(&self) -> BackendFuture<'_, Vec<WorkspaceRow>> {
        Box::pin(async { Err(WORKSPACES_NEED_ENGINE.to_owned()) })
    }

    /// Switch to a workspace, by its id.
    fn focus_workspace(&self, id: String) -> BackendFuture<'_, ()> {
        let _ = id;
        Box::pin(async { Err(WORKSPACES_NEED_ENGINE.to_owned()) })
    }

    /// Toggle fullscreen or floating on the window the person was in, or
    /// the overview. An error is the sentence to show.
    fn toggle_window_state(&self, toggle: WindowToggle) -> BackendFuture<'_, ()> {
        let _ = toggle;
        Box::pin(async { Err(WORKSPACES_NEED_ENGINE.to_owned()) })
    }
}

/// One item of a local storage namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageItemRow {
    /// Its key.
    pub key: String,
    /// Its value as text.
    pub value: String,
}

/// One stored OAuth token set (`OAuth::TokenSet`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenSetRow {
    /// The extension it belongs to.
    pub extension_id: String,
    /// The provider, or `None` for the unnamed one.
    pub provider_id: Option<String>,
    /// The bearer token.
    pub access_token: String,
    /// The refresh token.
    pub refresh_token: Option<String>,
    /// The id token.
    pub id_token: Option<String>,
    /// The granted scope.
    pub scope: Option<String>,
    /// When it expires, in seconds since the epoch.
    pub expires_at: Option<i64>,
    /// Whether it has expired.
    pub expired: bool,
}

/// What a file's action panel depends on (`FileActions::actionPanel`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileActions {
    /// Its MIME type.
    pub mime: Option<String>,
    /// Whether an application opens it.
    pub has_opener: bool,
    /// Whether the wallpaper can be set on this desktop.
    pub can_set_wallpaper: bool,
    /// Whether the engine can paste into the focused window.
    pub can_paste: bool,
}

/// What "Open with…" says without an engine.
pub const OPEN_WITH_NEEDS_ENGINE: &str =
    "Open with needs the Compass engine, and this window is running without one";

/// What the launcher hotkey's row says without an engine, which binds it.
pub const SHORTCUT_NEEDS_ENGINE: &str = "Setting up the launcher hotkey needs the Compass engine.";

/// What the login start switch says without an engine.
pub const LOGIN_START_NEEDS_ENGINE: &str = "Starting at login needs the Compass engine.";

/// An application "Open with…" offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenerRow {
    /// Its desktop id.
    pub id: String,
    /// Its display name.
    pub name: String,
    /// Its icon name.
    pub icon: Option<String>,
    /// Whether it is the default for the target's type.
    pub default: bool,
}

/// What a window-management request says without an engine.
pub const WORKSPACES_NEED_ENGINE: &str =
    "Window management needs the Compass engine, and this window is running without one";

/// What a window-management toggle acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowToggle {
    /// The window in and out of fullscreen.
    Fullscreen,
    /// The window between floating and tiled.
    Floating,
    /// The compositor's overview.
    Overview,
}

/// One workspace, as Switch Workspaces draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// The compositor's id, to switch to it.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// The monitor it is on.
    pub monitor: Option<String>,
    /// How many windows are on it.
    pub window_count: usize,
    /// The applications with a window on it, as (name, icon).
    pub apps: Vec<(String, Option<String>)>,
    /// Whether it is the active one.
    pub active: bool,
}
