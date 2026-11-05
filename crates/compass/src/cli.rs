//! The `compass` command-line surface.
//!
//! Kept deliberately close to the C++ CLI in `src/cli/` so muscle memory keeps
//! working: `toggle`, `ping` and the socket semantics are the same, and the
//! C++ spellings `open`/`close` survive as aliases of `show`/`hide`.
//!
//! Every C++ subcommand has a counterpart: `version`, `server`, `ping`,
//! `query`, `toggle`/`open`/`close`, `cmd ls`/`launch`, `deeplink`, `dmenu`,
//! `theme`, `fs query`, `app launch`, `config default`, `script
//! template`/`check`, `state open` and `logs`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use compass_ipc::SocketPath;

use crate::engine::Engine;

/// Long help shared by the top level and `doctor`, so `compass --help` and
/// `compass doctor --help` both document the exit codes.
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  the command succeeded; for `doctor --check-only`, no check failed
     (warnings alone still exit 0 — they are supported degradations)
  1  the command failed, or `doctor --check-only` found a failing check
  2  the command line could not be parsed";

/// Compass launcher control, from the shell.
#[derive(Debug, Parser)]
#[command(
    name = "compass",
    version,
    about = "Control and diagnose the Compass launcher",
    after_help = EXIT_CODE_HELP,
    after_long_help = EXIT_CODE_HELP,
)]
pub struct Cli {
    /// Which engine to use. Only the built-in one is left; kept so an old
    /// `COMPASS_ENGINE` setting is reported by `compass doctor` rather than
    /// rejected, and hidden from the help.
    #[arg(
        long,
        global = true,
        hide = true,
        value_enum,
        env = "COMPASS_ENGINE",
        default_value_t = Engine::Rust,
    )]
    pub engine: Engine,

    /// Path to the IPC socket, overriding `$XDG_RUNTIME_DIR/compass/ipc.sock`.
    ///
    /// Used verbatim: no `compass/` component is appended. This is how tests
    /// and a second instance stay out of the live session's way.
    #[arg(long, global = true, value_name = "PATH", env = "COMPASS_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Increase log verbosity (repeatable). `RUST_LOG` overrides it.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The URL schemes `compass <url>` takes as a deeplink: Compass's own, the
/// upstream `vicinae:` that extensions and the Vicinae store still emit, and
/// Raycast's, as the URL handler's desktop entry registers them.
pub const DEEPLINK_SCHEMES: [&str; 4] = ["compass", "vicinae", "raycast", "com.raycast"];

/// The command line with a bare deeplink (`compass raycast://oauth?…`, as a
/// desktop entry's `Exec=compass %u` runs it) turned into `compass deeplink
/// <url>`; anything else unchanged.
#[must_use]
pub fn with_deeplink(mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    // The first argument that is not a global option or its value: the
    // subcommand, or the bare URL. `compass --socket <path> compass://...`
    // is as much a bare deeplink as `compass compass://...`.
    let mut at = 1;
    while let Some(arg) = args.get(at).and_then(|arg| arg.to_str()) {
        if GLOBAL_OPTIONS_WITH_VALUE.contains(&arg) {
            at += 2;
        } else if arg.starts_with('-') && arg != "-" {
            at += 1;
        } else {
            break;
        }
    }
    let is_deeplink = args
        .get(at)
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| {
            arg.split_once(':')
                .is_some_and(|(scheme, _)| DEEPLINK_SCHEMES.contains(&scheme))
        });
    if is_deeplink {
        args.insert(at, "deeplink".into());
    }
    args
}

/// The command line with `start` added when it names no command, so a bare
/// `compass` (`flatpak run org.tunaos.compass`, Software's Open button) opens
/// the launcher as the desktop entry's `compass start` does. `--help` and
/// `--version` stay as they are.
#[must_use]
pub fn with_default_command(mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut at = 1;
    while let Some(arg) = args.get(at).and_then(|arg| arg.to_str()) {
        if ["-h", "--help", "-V", "--version"].contains(&arg) {
            return args;
        }
        if GLOBAL_OPTIONS_WITH_VALUE.contains(&arg) {
            at += 2;
        } else if arg.starts_with('-') && arg != "-" {
            at += 1;
        } else {
            return args;
        }
    }
    if at >= args.len() {
        args.push("start".into());
    }
    args
}

/// The global options that take their value as the next argument.
const GLOBAL_OPTIONS_WITH_VALUE: [&str; 2] = ["--socket", "--engine"];

impl Cli {
    /// The socket path this invocation should use.
    #[must_use]
    pub fn socket_path(&self) -> SocketPath {
        match &self.socket {
            Some(path) => SocketPath::exact(path.clone()),
            None => SocketPath::from_env(),
        }
    }
}

/// Subcommands of `compass`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Start a resident launcher, starting its engine if needed.
    /// Reopening activates the existing window. Escape hides it.
    Start {
        /// Wait for activation without opening a window. Does not enable autostart.
        #[arg(long)]
        hidden: bool,
    },

    /// The launcher window under `start`, which runs it as a child so that a
    /// launcher that fails can be started again without losing the engine.
    /// Internal: `start` holds the instance lease and owns the engine.
    #[command(name = "launcher-child", hide = true)]
    LauncherChild {
        /// Wait for activation without opening a window.
        #[arg(long)]
        hidden: bool,
    },

    /// Toggle the launcher window between shown and hidden.
    Toggle,

    /// Show the launcher window.
    #[command(alias = "open")]
    Show,

    /// Hide the launcher window.
    #[command(alias = "close")]
    Hide,

    /// Check that the engine is alive, printing its protocol version and pid.
    Ping,

    /// Run the engine.
    ///
    /// Runs until `compass shutdown` or a termination signal. `toggle`, `show`
    /// and `hide` reach the launcher window started with `compass start` or
    /// `compass ui`, and are refused when there is none.
    Serve {
        /// Do not bind the global launcher hotkey.
        ///
        /// Compass normally asks the desktop to bind Super+Space, which on
        /// GNOME shows a permission prompt. Pass this when your compositor
        /// already binds a key to `compass toggle`. Everything else works the
        /// same.
        #[arg(long)]
        no_hotkey: bool,
    },

    /// Ask a running engine to shut down.
    Shutdown,

    /// Search the running engine's index and print the ranked hits.
    Query {
        /// Search text. Joined with spaces if given as several words, so
        /// `compass query text editor` and `compass query "text editor"` agree.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,

        /// Emit the hits as JSON.
        #[arg(long)]
        json: bool,

        /// Keep only hits from this provider, e.g. `applications` or
        /// `commands`. Filters the ranked list, so it can return fewer than
        /// `launcher.max_results`.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<String>,
    },

    /// List, choose and check themes.
    #[command(subcommand, alias = "th")]
    Theme(ThemeCommand),

    /// Show version and build information.
    #[command(alias = "ver")]
    Version,

    /// Start the engine in the foreground, as `vicinae server` does.
    ///
    /// Refuses while one is already running unless `--replace` is passed,
    /// which kills it first.
    Server {
        /// Open the launcher window once the engine is started.
        #[arg(long)]
        open: bool,
        /// Replace the currently running instance if there is one.
        #[arg(long)]
        replace: bool,
        /// Path to the main config file.
        #[arg(long, value_name = "PATH")]
        config: Option<PathBuf>,
        /// Do not start the extension runtime. TypeScript extensions will not run.
        #[arg(long)]
        no_extension_runtime: bool,
    },

    /// Command utilities.
    #[command(subcommand, alias = "command")]
    Cmd(CmdCommand),

    /// System application commands.
    #[command(subcommand)]
    App(AppCommand),

    /// File search related commands.
    #[command(subcommand)]
    Fs(FsCommand),

    /// Script command utilities.
    #[command(subcommand)]
    Script(ScriptCommand),

    /// Query Compass state.
    #[command(subcommand)]
    State(StateCommand),

    /// Show the engine's log.
    Logs {
        /// Number of trailing lines to print.
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Keep printing new log lines as they are written.
        #[arg(short, long)]
        follow: bool,
    },

    /// The keyboard helper behind snippet keyword expansion.
    #[command(subcommand)]
    InputServer(InputServerCommand),

    /// Extension management.
    #[command(subcommand)]
    Ext(ExtCommand),

    /// The `compass.json` configuration: where it is, its schema, and migration.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Your Compass configuration and data: listing and deleting them.
    #[command(subcommand)]
    Data(DataCommand),

    /// One-off experiments that answer a question the code cannot.
    ///
    /// Hidden: these are addressed to whoever is answering the question — CI,
    /// or a person on a real machine — not to users, and each should be deleted
    /// or folded into a real subsystem once its question has an answer.
    #[command(hide = true, subcommand)]
    Spike(Spike),

    /// Hand a deeplink to the running engine.
    ///
    /// What the desktop runs for `compass://`, `vicinae://`, `raycast://` and
    /// `com.raycast:` URLs, and what a bare `compass <url>` becomes. It carries an OAuth
    /// provider's redirect (`raycast://oauth?code=…&state=…`) back to the
    /// extension that asked, and opens a store extension's detail page for
    /// `vicinae://extensions/<author>/<name>` (the Raycast store's for the
    /// `raycast://` spellings); other deeplinks are refused by name.
    #[command(alias = "link")]
    Deeplink {
        /// The URL, verbatim.
        url: String,
    },

    /// Suite 1: run installed extensions headlessly and judge each first frame.
    ///
    /// Hidden: it is CI's, not a user's. Starts an engine of its own on a
    /// private socket, with this process's environment, so the caller chooses
    /// the extensions and the data directories. Exits non-zero when any
    /// command fails. See `crates/compass/src/conformance.rs`.
    #[command(hide = true)]
    Conformance {
        /// A JSON plan naming the commands and their inputs; without one, the
        /// first command of every installed extension.
        #[arg(long)]
        plan: Option<std::path::PathBuf>,

        /// Seconds each command has to draw a frame with something in it.
        #[arg(long, default_value_t = 30)]
        timeout: u64,

        /// Emit the report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Render a list view from stdin, and print the entry chosen.
    ///
    /// Shows the lines of standard input in the running launcher and waits;
    /// prints the chosen entry (or its index with `--format index`, or the
    /// search text when passed instead) and exits 0, or exits 1 when the list
    /// is dismissed.
    Dmenu(DmenuArgs),

    /// Open the launcher window.
    ///
    /// Attaches to an existing engine when available. Without one, indexes
    /// in-process and exits on dismissal. Use `start` for a resident session.
    Ui,

    /// Report what works on this machine and what does not.
    #[command(after_help = EXIT_CODE_HELP, after_long_help = EXIT_CODE_HELP)]
    Doctor {
        /// Print only the problems, and exit non-zero if any check failed.
        #[arg(long)]
        check_only: bool,

        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },
}

/// `compass dmenu`'s options, the C++ CLI's.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct DmenuArgs {
    /// Set the navigation title.
    #[arg(short = 'n', long)]
    pub navigation_title: Option<String>,
    /// Set the title of the main section. Use the {count} placeholder to
    /// render the current count.
    #[arg(short = 's', long)]
    pub section_title: Option<String>,
    /// Control the format of the output (data, index).
    #[arg(short = 'f', long, default_value = "data", value_parser = ["data", "index"], ignore_case = true)]
    pub format: String,
    /// Placeholder text to use in the search bar.
    #[arg(short = 'p', long)]
    pub placeholder: Option<String>,
    /// Initial search query.
    #[arg(short = 'q', long)]
    pub query: Option<String>,
    /// Window width in pixels.
    #[arg(short = 'W', long)]
    pub width: Option<u32>,
    /// Window height in pixels.
    #[arg(short = 'H', long)]
    pub height: Option<u32>,
    /// Do not insert a section heading.
    #[arg(long)]
    pub no_section: bool,
    /// Do not show quick look if available for a given entry.
    #[arg(long)]
    pub no_quick_look: bool,
    /// Do not show metadata section in quick look.
    #[arg(long)]
    pub no_metadata: bool,
    /// Hide the status bar footer.
    #[arg(long)]
    pub no_footer: bool,
}

/// Theme subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ThemeCommand {
    /// List available themes.
    List {
        /// Emit the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Set the theme. Use `system` to follow the desktop.
    Set {
        /// Theme name: system, catppuccin, dracula, nord, gruvbox, tokyo-night, solarized, or a
        /// theme file's name (see `compass theme list`).
        theme: String,
    },
    /// Go back to the System theme, which follows the desktop.
    Reset,
    /// Print out the theme template, every key it can set.
    #[command(alias = "tmpl")]
    Template,
    /// Check whether the target theme file is valid.
    Check {
        /// The `.toml` theme file.
        file: PathBuf,
    },
    /// Print the paths themes are searched at.
    Paths,
}

/// `compass cmd` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum CmdCommand {
    /// List loaded commands.
    #[command(alias = "list")]
    Ls {
        /// Output command list as json.
        #[arg(short, long)]
        json: bool,
    },
    /// Launch a command.
    Launch {
        /// The command entrypoint ID to launch, e.g. `commands:clipboard-history`.
        entrypoint: String,
        /// Arguments to pass to the command.
        args: Vec<String>,
        /// Working directory forwarded to the command.
        #[arg(long)]
        cwd: Option<String>,
        /// Fallback text: what the command's search starts with.
        #[arg(short, long)]
        query: Option<String>,
    },
}

/// `compass app` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum AppCommand {
    /// Launch or focus an app from Compass.
    Launch {
        /// The ID of the application, e.g. `firefox.desktop`.
        app_id: String,
        /// Arguments to pass to the launched application.
        args: Vec<String>,
        /// Always launch a new instance.
        #[arg(long)]
        new: bool,
    },
}

/// `compass fs` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum FsCommand {
    /// Measure the file-index database without starting the engine or changing its data.
    Stats {
        /// Emit byte counts and unavailable reasons as JSON, without indexed paths.
        #[arg(long)]
        json: bool,
    },

    /// Return a list of indexed files matching the given query.
    #[command(alias = "q")]
    Query {
        /// What to search for, at least three characters.
        query: String,
        /// Limit the number of results (up to 10,000).
        #[arg(short = 'n', long, default_value_t = 100)]
        limit: u32,
        /// Filter by file category: image, video, audio, document, archive,
        /// application, directory, other.
        #[arg(short, long, value_parser = [
            "image", "video", "audio", "document", "archive", "application", "directory", "other",
        ])]
        category: Option<String>,
        /// Output result set as json.
        #[arg(short, long)]
        json: bool,
    },
}

/// `compass script` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ScriptCommand {
    /// Generate a new script template.
    Template {
        /// Title for the script.
        #[arg(short, long)]
        title: String,
        /// Language for the script.
        #[arg(short, long, default_value = "bash")]
        lang: String,
        /// Output mode (fullOutput, compact, inline, silent, terminal).
        #[arg(short, long, default_value = "fullOutput")]
        mode: String,
    },
    /// Validate a script command file.
    Check {
        /// Path to the script file to validate.
        file: PathBuf,
    },
}

/// `compass state` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum StateCommand {
    /// Exit with code 0 if the window is open.
    Open,
}

/// `compass input-server` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum InputServerCommand {
    /// Whether it is on, running and able to type, and why not.
    Status {
        /// Emit the status as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Turn it on (`input_server.enabled`), starting it now if the engine runs.
    Enable,
    /// Turn it off; snippet keywords stop expanding.
    Disable,
}

/// Configuration subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConfigCommand {
    /// Print where `compass.json` and Vicinae's `settings.json` are.
    Path,

    /// Print the JSON Schema for `compass.json`.
    ///
    /// Editors can use it to complete keys and flag misspelled ones.
    Schema,

    /// Translate Vicinae's `settings.json` into `compass.json`.
    ///
    /// Without `--write` this only prints the result and what was and was not
    /// carried across. Vicinae's file is never changed.
    Migrate {
        /// The settings file to read. Defaults to Vicinae's, in `~/.config/compass`
        /// or, before Compass has run once, `~/.config/vicinae`.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,

        /// Where to write. Defaults to Compass's `compass.json`.
        #[arg(long, value_name = "PATH")]
        to: Option<PathBuf>,

        /// Write the result instead of only printing it.
        #[arg(long)]
        write: bool,

        /// Replace an existing `compass.json`, keeping it as `compass.json.bak`.
        #[arg(long, requires = "write")]
        force: bool,

        /// Emit the migration report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Print out the default config: every setting at its default.
    Default,
}

/// `compass data` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum DataCommand {
    /// List Compass's configuration, data, cache and state, and delete them with `--yes`.
    ///
    /// Without `--yes` nothing is deleted: the exact paths are listed with what
    /// would happen to each. Only `compass` under each XDG base directory is
    /// removed, with the `vicinae` link Compass left beside it. Vicinae's own
    /// directories stay unless `--include-vicinae` is passed. A symlink is
    /// removed as a link, never what it points to. Refuses while Compass is
    /// running. Uninstalling the package never deletes any of this.
    Remove {
        /// Delete the listed paths instead of only listing them.
        #[arg(long)]
        yes: bool,

        /// Also delete Vicinae's own `vicinae` directories, which Compass
        /// otherwise leaves for an installed Vicinae.
        #[arg(long)]
        include_vicinae: bool,
    },
}

/// Extension management subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ExtCommand {
    /// List available extensions and their status.
    List {
        /// Emit the list as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// The spikes. See [`crate::spike`] for what each one is for.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Spike {
    /// Spike A: bind a global shortcut through the portal and wait for it.
    ///
    /// Reports whether binding is permitted, whether the trigger granted is the
    /// one requested, and whether pressing the key reaches us. The third
    /// question is why this exists: it can only be answered by a real desktop
    /// with a real keypress, which is what the VM tier provides.
    GlobalShortcut {
        /// Preferred trigger, e.g. `SUPER+space`. Omit to express no
        /// preference, which is a different and also interesting answer.
        #[arg(long)]
        trigger: Option<String>,

        /// Shortcut id, echoed back on activation.
        #[arg(long, default_value = "compass.spike.toggle")]
        id: String,

        /// Seconds to wait for an activation after binding.
        #[arg(long, default_value_t = 60)]
        wait: u64,

        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },

    /// Spike B: can we sandbox a process inside an already-sandboxed Flatpak?
    ///
    /// Applies a Landlock ruleset and a seccomp filter to this process and
    /// tests whether each actually denies what it should while still allowing
    /// what it should. Phase 4's extension-host sandbox is designed on the
    /// assumption that both nest inside bubblewrap's; this measures it.
    ///
    /// Irreversible by nature: the process it runs in is confined afterwards,
    /// which is why it is its own command and not a doctor check.
    Sandbox {
        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::path::Path;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("should parse")
    }

    #[test]
    fn a_command_line_with_no_command_starts_the_launcher() {
        let with = |args: &[&str]| {
            with_default_command(args.iter().map(std::ffi::OsString::from).collect())
        };
        let start = |args: &[&str]| Cli::try_parse_from(with(args)).map(|cli| cli.command);
        assert_eq!(
            start(&["compass"]).ok(),
            Some(Command::Start { hidden: false })
        );
        assert_eq!(
            start(&["compass", "--socket", "/tmp/x.sock", "-v"]).ok(),
            Some(Command::Start { hidden: false })
        );
        assert_eq!(start(&["compass", "toggle"]).ok(), Some(Command::Toggle));
        for help in [["compass", "--help"], ["compass", "-V"]] {
            assert_eq!(with(&help), help.map(std::ffi::OsString::from).to_vec());
        }
    }

    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_bare_deeplink_becomes_the_deeplink_command() {
        let argv = |args: &[&str]| -> Vec<std::ffi::OsString> {
            args.iter().map(std::ffi::OsString::from).collect()
        };
        for url in [
            "raycast://oauth?code=c&state=s",
            "com.raycast:/oauth?code=c&state=s",
            "vicinae://extensions/x",
        ] {
            let parsed = Cli::try_parse_from(with_deeplink(argv(&["compass", url]))).expect(url);
            assert_eq!(parsed.command, Command::Deeplink { url: url.into() });
        }
        assert_eq!(
            with_deeplink(argv(&["compass", "toggle"])),
            argv(&["compass", "toggle"])
        );
        let url = "compass://launch/commands/clipboard-history";
        for args in [
            &["compass", "--socket", "/tmp/s", url][..],
            &["compass", "-v", "--engine=rust", url][..],
            &["compass", "--engine", "rust", "-vv", url][..],
        ] {
            let parsed = Cli::try_parse_from(with_deeplink(argv(args)))
                .unwrap_or_else(|err| panic!("{args:?}: {err}"));
            assert_eq!(parsed.command, Command::Deeplink { url: url.into() });
        }
        assert_eq!(
            with_deeplink(argv(&["compass", "https://example.com"])),
            argv(&["compass", "https://example.com"]),
            "a web URL is not ours to take"
        );
    }

    #[test]
    fn window_commands_parse() {
        assert_eq!(parse(&["compass", "toggle"]).command, Command::Toggle);
        assert_eq!(parse(&["compass", "show"]).command, Command::Show);
        assert_eq!(parse(&["compass", "hide"]).command, Command::Hide);
        assert_eq!(parse(&["compass", "ping"]).command, Command::Ping);
    }

    #[test]
    fn the_spike_parses_with_its_defaults() {
        // The defaults matter: the VM job passes only --trigger and --json, so
        // a changed default id here silently changes what the harness asserts.
        let Command::Spike(Spike::GlobalShortcut {
            trigger,
            id,
            wait,
            json,
        }) = parse(&["compass", "spike", "global-shortcut", "--json"]).command
        else {
            panic!("expected the global-shortcut spike");
        };
        assert_eq!(trigger, None);
        assert_eq!(id, "compass.spike.toggle");
        assert_eq!(wait, 60);
        assert!(json);
    }

    #[test]
    fn the_parity_harness_invocation_parses() {
        // crates/compass-testkit/src/parity.rs builds this exact argv, one call
        // per query, and it is the whole interface Suite 0 (§8.1) diffs the two
        // engines through. An argv assembled in one crate and parsed in another
        // has no compiler between the two, which is the same reason
        // `the_spike_parses_with_its_defaults` above exists.
        let cli = Cli::try_parse_from([
            "compass",
            "--socket",
            "/tmp/compass-parity/ipc.sock",
            "query",
            "--json",
            "firefox",
        ])
        .expect("the parity harness's argv must parse");

        let Command::Query { text, json, .. } = cli.command else {
            panic!("expected the query command");
        };
        assert_eq!(text, vec!["firefox".to_owned()]);
        assert!(
            json,
            "parity parses stdout as JSON, so --json must take effect"
        );

        // Two controls, because a test that only asserts the good case would
        // pass just as well against a CLI that accepts anything.

        // The order parity used before measurement: rejected, not tolerated.
        assert!(
            Cli::try_parse_from(["compass", "--json", "query", "firefox"]).is_err(),
            "a global --json would mean the original invocation was fine after all"
        );

        // And parity no longer passes --engine, because until the Phase 7
        // cutover the binary IS the engine: this one refuses `--engine cpp` at
        // runtime by design, and the C++ binary has no such flag to give.
        // Parsing is not the refusal — that happens later — so this only pins
        // that the flag still exists and still defaults to the Rust engine.
        let defaulted = Cli::try_parse_from(["compass", "query", "firefox"])
            .expect("query without --engine must parse");
        assert_eq!(defaulted.engine, Engine::Rust);
    }

    #[test]
    fn the_seeded_grant_names_the_spike_default_id() {
        // The VM image pre-seeds a GlobalShortcuts grant into dconf so that
        // `BindShortcuts` completes without a human at GNOME's consent dialog.
        // That bypass keys off the shortcut *id* alone: gnome-control-center
        // shows the dialog for any id it has not stored, so a rename here and
        // not there silently reinstates the dialog — and Spike A goes back to
        // hanging for its full timeout, which reads as a portal regression
        // rather than as a typo. Cheap to couple, expensive to debug.
        let seed = include_str!("../../../packaging/vmtest/compass-shortcuts.dconf");
        let Command::Spike(Spike::GlobalShortcut { id, .. }) =
            parse(&["compass", "spike", "global-shortcut"]).command
        else {
            panic!("expected the global-shortcut spike");
        };
        assert!(
            seed.contains(&format!("'{id}'")),
            "the seeded dconf grant does not mention the spike's default id {id:?}",
        );
    }

    #[test]
    fn the_sandbox_spike_parses() {
        let Command::Spike(Spike::Sandbox { json }) =
            parse(&["compass", "spike", "sandbox", "--json"]).command
        else {
            panic!("expected the sandbox spike");
        };
        assert!(json);
    }

    #[test]
    fn the_spike_is_hidden_but_reachable() {
        // Hidden from --help, yet it must still parse: a spike nobody can run
        // is worse than no spike, and `hide` is easy to confuse with disabling.
        let spike = Cli::command()
            .get_subcommands()
            .find(|c| c.get_name() == "spike")
            .expect("the spike subcommand exists")
            .clone();
        assert!(spike.is_hide_set(), "the spike should not appear in --help");
        assert!(Cli::try_parse_from(["compass", "spike", "global-shortcut"]).is_ok());
    }

    #[test]
    fn the_ui_command_parses() {
        assert_eq!(parse(&["compass", "ui"]).command, Command::Ui);
    }

    #[test]
    fn hidden_start_is_explicit() {
        assert_eq!(
            parse(&["compass", "start"]).command,
            Command::Start { hidden: false }
        );
        assert_eq!(
            parse(&["compass", "start", "--hidden"]).command,
            Command::Start { hidden: true }
        );
        assert!(Cli::try_parse_from(["compass", "ui", "--hidden"]).is_err());
    }

    #[test]
    fn the_engine_commands_parse() {
        assert_eq!(
            parse(&["compass", "serve"]).command,
            Command::Serve { no_hotkey: false }
        );
        assert_eq!(
            parse(&["compass", "serve", "--no-hotkey"]).command,
            Command::Serve { no_hotkey: true }
        );
        assert_eq!(parse(&["compass", "shutdown"]).command, Command::Shutdown);
    }

    #[test]
    fn a_multi_word_query_is_joined_rather_than_rejected() {
        // `compass query text editor` is what a person types; requiring the
        // quotes would be a papercut on the most-used command.
        let Command::Query { text, json, .. } =
            parse(&["compass", "query", "text", "editor"]).command
        else {
            panic!("expected a query");
        };
        assert_eq!(text, ["text", "editor"]);
        assert!(!json);
    }

    #[test]
    fn an_empty_query_is_a_usage_error() {
        // Distinct from `query ""`, which is the legitimate "show me everything"
        // case and must keep working.
        assert!(Cli::try_parse_from(["compass", "query"]).is_err());
        assert!(Cli::try_parse_from(["compass", "query", ""]).is_ok());
    }

    #[test]
    fn the_cpp_spellings_survive_as_aliases() {
        assert_eq!(parse(&["compass", "open"]).command, Command::Show);
        assert_eq!(parse(&["compass", "close"]).command, Command::Hide);
    }

    #[test]
    fn doctor_flags_default_off_and_parse_together() {
        assert_eq!(
            parse(&["compass", "doctor"]).command,
            Command::Doctor {
                check_only: false,
                json: false
            }
        );
        assert_eq!(
            parse(&["compass", "doctor", "--check-only", "--json"]).command,
            Command::Doctor {
                check_only: true,
                json: true
            }
        );
    }

    #[test]
    fn engine_defaults_to_rust_and_can_be_overridden() {
        assert_eq!(parse(&["compass", "ping"]).engine, Engine::Rust);
        assert_eq!(
            parse(&["compass", "--engine", "cpp", "ping"]).engine,
            Engine::Cpp
        );
    }

    #[test]
    fn an_unknown_engine_is_a_usage_error() {
        assert!(Cli::try_parse_from(["compass", "--engine", "go", "ping"]).is_err());
    }

    #[test]
    fn socket_override_is_used_verbatim() {
        let cli = parse(&["compass", "--socket", "/tmp/x.sock", "toggle"]);
        assert_eq!(cli.socket_path().as_path(), Path::new("/tmp/x.sock"));
        assert!(!cli.socket_path().is_fallback());
    }

    #[test]
    fn global_flags_may_follow_the_subcommand() {
        let cli = parse(&[
            "compass",
            "toggle",
            "--socket",
            "/tmp/y.sock",
            "--engine",
            "cpp",
        ]);
        assert_eq!(cli.command, Command::Toggle);
        assert_eq!(cli.engine, Engine::Cpp);
        assert_eq!(cli.socket_path().as_path(), Path::new("/tmp/y.sock"));
    }

    #[test]
    fn a_subcommand_is_required() {
        assert!(Cli::try_parse_from(["compass"]).is_err());
    }

    #[test]
    fn help_documents_the_exit_codes() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("Exit codes:"));
        assert!(help.contains("no check failed"));

        let doctor_help = Cli::command()
            .find_subcommand_mut("doctor")
            .expect("doctor subcommand")
            .render_long_help()
            .to_string();
        assert!(doctor_help.contains("Exit codes:"));
    }

    #[test]
    fn ext_list_parses() {
        let cli = parse(&["compass", "ext", "list"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: false }));

        let cli = parse(&["compass", "ext", "list", "--json"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: true }));
    }

    #[test]
    fn the_cpp_subcommands_all_parse_with_their_cpp_flags() {
        assert_eq!(parse(&["compass", "version"]).command, Command::Version);
        assert_eq!(parse(&["compass", "ver"]).command, Command::Version);
        assert_eq!(
            parse(&[
                "compass",
                "server",
                "--replace",
                "--open",
                "--config",
                "/c.json"
            ])
            .command,
            Command::Server {
                open: true,
                replace: true,
                config: Some(PathBuf::from("/c.json")),
                no_extension_runtime: false,
            }
        );
        assert_eq!(
            parse(&["compass", "command", "list", "-j"]).command,
            Command::Cmd(CmdCommand::Ls { json: true })
        );
        assert_eq!(
            parse(&[
                "compass", "cmd", "launch", "@a/b:c", "one", "two", "-q", "milk", "--cwd", "/tmp"
            ])
            .command,
            Command::Cmd(CmdCommand::Launch {
                entrypoint: "@a/b:c".into(),
                args: vec!["one".into(), "two".into()],
                cwd: Some("/tmp".into()),
                query: Some("milk".into()),
            })
        );
        assert!(Cli::try_parse_from(["compass", "cmd", "launch"]).is_err());
        assert_eq!(
            parse(&[
                "compass",
                "app",
                "launch",
                "firefox.desktop",
                "--new",
                "https://a"
            ])
            .command,
            Command::App(AppCommand::Launch {
                app_id: "firefox.desktop".into(),
                args: vec!["https://a".into()],
                new: true,
            })
        );
        assert_eq!(
            parse(&[
                "compass", "fs", "q", "report", "-n", "5", "-c", "document", "-j"
            ])
            .command,
            Command::Fs(FsCommand::Query {
                query: "report".into(),
                limit: 5,
                category: Some("document".into()),
                json: true,
            })
        );
        assert!(
            Cli::try_parse_from(["compass", "fs", "query", "abc", "-c", "Documents"]).is_err(),
            "the C++'s category names, and only those"
        );
        assert_eq!(
            parse(&["compass", "fs", "stats", "--json"]).command,
            Command::Fs(FsCommand::Stats { json: true })
        );
        assert_eq!(
            parse(&["compass", "fs", "stats"]).command,
            Command::Fs(FsCommand::Stats { json: false })
        );
        assert_eq!(
            parse(&["compass", "fs", "query", "abc"]).command,
            Command::Fs(FsCommand::Query {
                query: "abc".into(),
                limit: 100,
                category: None,
                json: false,
            })
        );
        assert_eq!(
            parse(&["compass", "script", "template", "-t", "Hi"]).command,
            Command::Script(ScriptCommand::Template {
                title: "Hi".into(),
                lang: "bash".into(),
                mode: "fullOutput".into(),
            })
        );
        assert_eq!(
            parse(&["compass", "script", "check", "a.sh"]).command,
            Command::Script(ScriptCommand::Check {
                file: PathBuf::from("a.sh")
            })
        );
        assert_eq!(
            parse(&["compass", "state", "open"]).command,
            Command::State(StateCommand::Open)
        );
        assert_eq!(
            parse(&["compass", "logs"]).command,
            Command::Logs {
                lines: 50,
                follow: false
            }
        );
        assert_eq!(
            parse(&["compass", "logs", "-n", "3", "-f"]).command,
            Command::Logs {
                lines: 3,
                follow: true
            }
        );
        assert_eq!(
            parse(&["compass", "config", "default"]).command,
            Command::Config(ConfigCommand::Default)
        );
        assert_eq!(
            parse(&["compass", "th", "tmpl"]).command,
            Command::Theme(ThemeCommand::Template)
        );
        assert_eq!(
            parse(&["compass", "link", "vicinae://x"]).command,
            Command::Deeplink {
                url: "vicinae://x".into()
            }
        );
    }

    #[test]
    fn theme_commands_parse() {
        assert_eq!(
            parse(&["compass", "theme", "list"]).command,
            Command::Theme(ThemeCommand::List { json: false })
        );
        assert_eq!(
            parse(&["compass", "theme", "set", "dracula"]).command,
            Command::Theme(ThemeCommand::Set {
                theme: "dracula".to_owned()
            })
        );
        assert_eq!(
            parse(&["compass", "theme", "reset"]).command,
            Command::Theme(ThemeCommand::Reset)
        );
    }

    #[test]
    fn data_remove_is_a_dry_run_unless_told_otherwise() {
        assert_eq!(
            parse(&["compass", "data", "remove"]).command,
            Command::Data(DataCommand::Remove {
                yes: false,
                include_vicinae: false
            })
        );
        assert_eq!(
            parse(&["compass", "data", "remove", "--yes", "--include-vicinae"]).command,
            Command::Data(DataCommand::Remove {
                yes: true,
                include_vicinae: true
            })
        );
    }
}
