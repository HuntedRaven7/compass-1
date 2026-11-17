//! What an extension reaches through `Command/*`: launching a sibling
//! command, relabelling itself in root search, and its preferences.
//!
//! The C++ `ExtCommandService` finds the command among the root manager's
//! extensions and hands it to the navigation controller, which pushes it in
//! the launcher. Here the launcher is another process, so a launch is kept
//! under a token and the window is told to take it
//! (`WindowCommand::Launch`, IPC v15), the way `compass dmenu` hands the
//! window a list: the window fetches it (`ExtensionLaunchFetch`) and runs the
//! command as if it had been picked in root search, arguments form,
//! preferences form and all. The launch context and fallback text ride along
//! beside it, kept here for the command's next run.
//!
//! `updateCommandMetadata` keeps a subtitle override for the command, in
//! memory as the C++ keeps it, which root search shows in place of the
//! extension's title (`ExtensionSubtitles`, IPC v15). `openCommandPreferences`
//! and `openExtensionPreferences` open the command's preferences form in the
//! launcher, by the same token. PARITY, "The extension host API".

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use compass_worker_host::command_service::{Commands, LaunchProps};

/// How long a launch context waits for its command to run: long enough for
/// a person to fill in the arguments form, short enough that a form they
/// walked away from does not hand its context to a later, unrelated run.
pub const CONTEXT_LIFETIME: Duration = Duration::from_secs(300);

/// A launch the window is to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The command's root id.
    pub id: String,
    /// Its arguments, as a JSON object, when the extension passed any.
    pub arguments_json: Option<String>,
    /// Open its preferences rather than run it.
    pub preferences: bool,
    /// What its search starts with: `compass cmd launch --query`.
    pub fallback_text: Option<String>,
}

/// What an extension's launch carries beyond its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    /// `options.context`, handed to the launched command untouched.
    pub launch_context: serde_json::Value,
    /// `options.fallbackText`.
    pub fallback_text: Option<String>,
}

/// The launches, contexts and subtitle overrides the engine holds.
#[derive(Debug, Default)]
pub struct Launches {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Launch>>,
    contexts: Mutex<HashMap<String, (Context, Instant)>>,
    subtitles: Mutex<HashMap<String, String>>,
}

impl Launches {
    /// Keeps `launch` under a new token, with `context` for its next run.
    pub fn open(&self, launch: Launch, context: Option<Context>) -> u64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(context) = context {
            self.contexts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(launch.id.clone(), (context, Instant::now()));
        }
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(token, launch);
        token
    }

    /// The launch behind `token`, once.
    pub fn take(&self, token: u64) -> Option<Launch> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&token)
    }

    /// The context the next run of `id` carries, once, if it is fresh.
    pub fn take_context(&self, id: &str) -> Option<Context> {
        let (context, kept) = self
            .contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)?;
        (kept.elapsed() < CONTEXT_LIFETIME).then_some(context)
    }

    /// Sets (or with `None` clears) the subtitle `id` shows in root search.
    pub fn set_subtitle(&self, id: &str, subtitle: Option<&str>) {
        let mut subtitles = self
            .subtitles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match subtitle {
            Some(subtitle) => {
                subtitles.insert(id.to_owned(), subtitle.to_owned());
            }
            None => {
                subtitles.remove(id);
            }
        }
    }

    /// The subtitle `id` shows instead of its extension's title, if one is set.
    #[must_use]
    pub fn subtitle(&self, id: &str) -> Option<String> {
        self.subtitles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Every override, sorted by command id.
    #[must_use]
    pub fn subtitles(&self) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = self
            .subtitles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(id, subtitle)| (id.clone(), subtitle.clone()))
            .collect();
        all.sort();
        all
    }
}

/// One installed command, as `launchCommand` finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The extension the command belongs to, `@<author>/<extension id>`.
    pub extension_id: String,
    /// The manifest's `name`.
    pub extension_name: String,
    /// The manifest's `author`.
    pub author: String,
    /// The manifest's `owner`, when it names one.
    pub owner: Option<String>,
    /// The command's name.
    pub name: String,
    /// Its root id.
    pub id: String,
}

impl Known {
    /// Every command of `index`'s extensions.
    #[must_use]
    pub fn all(index: &compass_core::AppIndex) -> Vec<Self> {
        index
            .extensions()
            .iter()
            .map(|command| Self {
                extension_id: command.extension_id.clone(),
                extension_name: command.extension_name.clone(),
                author: command.author.clone(),
                owner: command.owner.clone(),
                name: command.name.clone(),
                id: command.id.clone(),
            })
            .collect()
    }
}

/// Hands a token to the launcher window.
pub type Deliver = Arc<dyn Fn(u64) + Send + Sync>;

/// [`Commands`] for one running command.
pub struct EngineCommands {
    /// The running command's root id.
    command_id: String,
    known: Vec<Known>,
    launches: Arc<Launches>,
    deliver: Deliver,
}

impl std::fmt::Debug for EngineCommands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineCommands")
            .field("command_id", &self.command_id)
            .field("known", &self.known.len())
            .finish_non_exhaustive()
    }
}

impl EngineCommands {
    /// Serves the command `command_id` of `extension_id`, finding siblings
    /// among `known`, keeping launches in `launches` and handing their tokens
    /// to `deliver`.
    ///
    /// Only `extension_id`'s own commands are kept. `launchCommand` launches
    /// a sibling: a command of another extension would run with arguments
    /// and context this one chose, under that extension's storage, tokens and
    /// host-program grants, without the person picking it. Raycast asks
    /// before one extension launches another's command; until Compass asks
    /// too, such a launch is `No such command`.
    #[must_use]
    pub fn new(
        command_id: String,
        extension_id: &str,
        known: Vec<Known>,
        launches: Arc<Launches>,
        deliver: Deliver,
    ) -> Self {
        Self {
            command_id,
            known: known
                .into_iter()
                .filter(|known| known.extension_id == extension_id)
                .collect(),
            launches,
            deliver,
        }
    }
}

impl Commands for EngineCommands {
    fn find_command(
        &self,
        extension_name: &str,
        owner_or_author: &str,
        command_id: &str,
    ) -> Option<String> {
        self.known
            .iter()
            .find(|known| {
                known.extension_name == extension_name
                    && (known.author == owner_or_author
                        || known.owner.as_deref() == Some(owner_or_author))
                    && known.name == command_id
            })
            .map(|known| known.id.clone())
    }

    fn activate(&self, entrypoint_id: &str, props: LaunchProps) {
        let arguments_json = (!props.arguments.is_empty()).then(|| {
            serde_json::Value::Object(
                props
                    .arguments
                    .into_iter()
                    .map(|(key, value)| (key, serde_json::Value::String(value)))
                    .collect(),
            )
            .to_string()
        });
        let context =
            (props.launch_context.is_some() || props.fallback_text.is_some()).then(|| Context {
                launch_context: props.launch_context.unwrap_or(serde_json::Value::Null),
                fallback_text: props.fallback_text,
            });
        let token = self.launches.open(
            Launch {
                id: entrypoint_id.to_owned(),
                arguments_json,
                preferences: false,
                fallback_text: None,
            },
            context,
        );
        (self.deliver)(token);
    }

    fn set_subtitle_override(&self, subtitle: Option<&str>) {
        self.launches.set_subtitle(&self.command_id, subtitle);
    }

    fn items_changed(&self) {
        // The launcher reads the overrides each time it opens; nothing is
        // drawn from the engine in between.
    }

    fn open_preferences(&self) {
        let token = self.launches.open(
            Launch {
                id: self.command_id.clone(),
                arguments_json: None,
                preferences: true,
                fallback_text: None,
            },
            None,
        );
        (self.deliver)(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_worker_host::command_service::CommandService;
    use compass_worker_host::tsapi::Call;

    fn service() -> (
        CommandService<EngineCommands>,
        Arc<Launches>,
        Arc<Mutex<Vec<u64>>>,
    ) {
        let launches = Arc::new(Launches::default());
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let commands = EngineCommands::new(
            "@ada/notes:list".into(),
            "@ada/notes",
            vec![
                Known {
                    extension_id: "@ada/notes".into(),
                    extension_name: "notes".into(),
                    author: "ada".into(),
                    owner: None,
                    name: "create".into(),
                    id: "@ada/notes:create".into(),
                },
                Known {
                    extension_id: "@bob/mail".into(),
                    extension_name: "mail".into(),
                    author: "bob".into(),
                    owner: None,
                    name: "send".into(),
                    id: "@bob/mail:send".into(),
                },
            ],
            Arc::clone(&launches),
            Arc::new({
                let delivered = Arc::clone(&delivered);
                move |token| delivered.lock().unwrap().push(token)
            }),
        );
        (CommandService::new(commands), launches, delivered)
    }

    fn call(method: &str, params: serde_json::Value) -> Call {
        Call {
            jsonrpc: "2.0".into(),
            id: Some(1),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn a_sibling_is_launched_through_the_window_with_its_arguments_and_context() {
        let (service, launches, delivered) = service();
        let reply = service
            .handle(&call(
                "Command/launchCommand",
                serde_json::json!({"options": {
                    "extensionName": "notes", "ownerOrAuthorName": "ada", "name": "create",
                    "arguments": {"title": "Groceries", "count": 3},
                    "context": {"from": "list"}, "fallbackText": "milk",
                }}),
            ))
            .unwrap();
        assert!(!reply.contains("error"), "{reply}");
        let token = delivered.lock().unwrap()[0];
        assert_eq!(
            launches.take(token),
            Some(Launch {
                id: "@ada/notes:create".into(),
                arguments_json: Some(r#"{"title":"Groceries"}"#.into()),
                preferences: false,
                fallback_text: None,
            })
        );
        assert_eq!(launches.take(token), None, "a launch is taken once");
        assert_eq!(
            launches.take_context("@ada/notes:create"),
            Some(Context {
                launch_context: serde_json::json!({"from": "list"}),
                fallback_text: Some("milk".into()),
            })
        );
        assert_eq!(launches.take_context("@ada/notes:create"), None);
    }

    #[test]
    fn another_extension_s_command_is_no_such_command() {
        let (service, launches, delivered) = service();
        let reply = service
            .handle(&call(
                "Command/launchCommand",
                serde_json::json!({"options": {
                    "extensionName": "mail", "ownerOrAuthorName": "bob", "name": "send",
                    "arguments": {"to": "someone@example.invalid"},
                }}),
            ))
            .unwrap();
        assert!(reply.contains("No such command"), "{reply}");
        assert!(delivered.lock().unwrap().is_empty(), "nothing was launched");
        assert_eq!(launches.take_context("@bob/mail:send"), None);
    }

    #[test]
    fn a_command_that_is_not_installed_is_no_such_command() {
        let (service, _, delivered) = service();
        let reply = service
            .handle(&call(
                "Command/launchCommand",
                serde_json::json!({"options": {
                    "extensionName": "notes", "ownerOrAuthorName": "someone-else", "name": "create",
                }}),
            ))
            .unwrap();
        assert!(reply.contains("No such command"), "{reply}");
        assert!(delivered.lock().unwrap().is_empty());
    }

    #[test]
    fn the_subtitle_is_overridden_and_an_empty_one_clears_it() {
        let (service, launches, _) = service();
        let _ = service.handle(&call(
            "Command/updateCommandMetadata",
            serde_json::json!({"payload": {"subtitle": "3 unread"}}),
        ));
        assert_eq!(
            launches.subtitle("@ada/notes:list"),
            Some("3 unread".into())
        );
        assert_eq!(
            launches.subtitles(),
            [("@ada/notes:list".to_owned(), "3 unread".to_owned())]
        );
        let _ = service.handle(&call(
            "Command/updateCommandMetadata",
            serde_json::json!({"payload": {"subtitle": ""}}),
        ));
        assert_eq!(launches.subtitle("@ada/notes:list"), None);
    }

    #[test]
    fn preferences_open_the_running_command_s_form() {
        let (service, launches, delivered) = service();
        let _ = service.handle(&call(
            "Command/openExtensionPreferences",
            serde_json::json!({}),
        ));
        let token = delivered.lock().unwrap()[0];
        assert_eq!(
            launches.take(token),
            Some(Launch {
                id: "@ada/notes:list".into(),
                arguments_json: None,
                preferences: true,
                fallback_text: None,
            })
        );
    }
}
