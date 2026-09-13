//! The `vicinae` command-line surface.
//!
//! Kept deliberately close to the C++ CLI in `src/cli/` so muscle memory keeps
//! working: `toggle`, `ping` and the socket semantics are the same, and the
//! C++ spellings `open`/`close` survive as aliases of `show`/`hide`.
//!
//! This is the Phase 2 slice only (PLAN §6): window control, liveness and the
//! diagnostic. `launch`, `cmd`, `dmenu`, `deeplink` and `logs` stay with the
//! C++ binary until their parity-ledger rows go green.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use compass_ipc::SocketPath;

use crate::engine::Engine;

/// Long help shared by the top level and `doctor`, so `vicinae --help` and
/// `vicinae doctor --help` both document the exit codes.
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  the command succeeded; for `doctor --check-only`, no check failed
     (warnings alone still exit 0 — they are supported degradations)
  1  the command failed, or `doctor --check-only` found a failing check
  2  the command line could not be parsed";

/// Compass launcher control, from the shell.
#[derive(Debug, Parser)]
#[command(
    name = "vicinae",
    version,
    about = "Control and diagnose the Compass launcher",
    after_help = EXIT_CODE_HELP,
    after_long_help = EXIT_CODE_HELP,
)]
pub struct Cli {
    /// Engine implementation to talk to.
    ///
    /// This binary is the Rust engine and cannot dispatch to the C++ one; see
    /// `PLAN.md` §5. `--engine cpp` parses, is reported by `doctor`, and makes
    /// engine-dependent commands refuse rather than quietly do the Rust thing.
    #[arg(
        long,
        global = true,
        value_enum,
        env = "COMPASS_ENGINE",
        default_value_t = Engine::Rust,
    )]
    pub engine: Engine,

    /// Path to the IPC socket, overriding `$XDG_RUNTIME_DIR/vicinae/ipc.sock`.
    ///
    /// Used verbatim: no `vicinae/` component is appended. This is how tests
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

/// Subcommands of `vicinae`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
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
    /// Serves the IPC socket until a `shutdown` request or a termination
    /// signal. This build is headless: it answers `ping`, `query` and `doctor`,
    /// and refuses the window commands, which have no window to act on yet.
    Serve,

    /// Ask a running engine to shut down.
    Shutdown,

    /// Search the running engine's index and print the ranked hits.
    Query {
        /// Search text. Joined with spaces if given as several words, so
        /// `vicinae query text editor` and `vicinae query "text editor"` agree.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,

        /// Emit the hits as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Extension management.
    #[command(subcommand)]
    Ext(ExtCommand),

    /// One-off experiments that answer a question the code cannot.
    ///
    /// Hidden: these are addressed to whoever is answering the question — CI,
    /// or a person on a real machine — not to users, and each should be deleted
    /// or folded into a real subsystem once its question has an answer.
    #[command(hide = true, subcommand)]
    Spike(Spike),

    /// Open the launcher window.
    ///
    /// Runs the launcher in the foreground until it is dismissed or something
    /// is launched. It does not talk to `serve` and does not need one running:
    /// it indexes and ranks in-process. See ADR-0011 for why the window is its
    /// own command rather than something the engine hosts.
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
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn window_commands_parse() {
        assert_eq!(parse(&["vicinae", "toggle"]).command, Command::Toggle);
        assert_eq!(parse(&["vicinae", "show"]).command, Command::Show);
        assert_eq!(parse(&["vicinae", "hide"]).command, Command::Hide);
        assert_eq!(parse(&["vicinae", "ping"]).command, Command::Ping);
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
        }) = parse(&["vicinae", "spike", "global-shortcut", "--json"]).command
        else {
            panic!("expected the global-shortcut spike");
        };
        assert_eq!(trigger, None);
        assert_eq!(id, "compass.spike.toggle");
        assert_eq!(wait, 60);
        assert!(json);
    }

    #[test]
    fn the_sandbox_spike_parses() {
        let Command::Spike(Spike::Sandbox { json }) =
            parse(&["vicinae", "spike", "sandbox", "--json"]).command
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
        assert!(Cli::try_parse_from(["vicinae", "spike", "global-shortcut"]).is_ok());
    }

    #[test]
    fn the_ui_command_parses() {
        assert_eq!(parse(&["vicinae", "ui"]).command, Command::Ui);
    }

    #[test]
    fn the_engine_commands_parse() {
        assert_eq!(parse(&["vicinae", "serve"]).command, Command::Serve);
        assert_eq!(parse(&["vicinae", "shutdown"]).command, Command::Shutdown);
    }

    #[test]
    fn a_multi_word_query_is_joined_rather_than_rejected() {
        // `vicinae query text editor` is what a person types; requiring the
        // quotes would be a papercut on the most-used command.
        let Command::Query { text, json } = parse(&["vicinae", "query", "text", "editor"]).command
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
        assert!(Cli::try_parse_from(["vicinae", "query"]).is_err());
        assert!(Cli::try_parse_from(["vicinae", "query", ""]).is_ok());
    }

    #[test]
    fn the_cpp_spellings_survive_as_aliases() {
        assert_eq!(parse(&["vicinae", "open"]).command, Command::Show);
        assert_eq!(parse(&["vicinae", "close"]).command, Command::Hide);
    }

    #[test]
    fn doctor_flags_default_off_and_parse_together() {
        assert_eq!(
            parse(&["vicinae", "doctor"]).command,
            Command::Doctor {
                check_only: false,
                json: false
            }
        );
        assert_eq!(
            parse(&["vicinae", "doctor", "--check-only", "--json"]).command,
            Command::Doctor {
                check_only: true,
                json: true
            }
        );
    }

    #[test]
    fn engine_defaults_to_rust_and_can_be_overridden() {
        assert_eq!(parse(&["vicinae", "ping"]).engine, Engine::Rust);
        assert_eq!(
            parse(&["vicinae", "--engine", "cpp", "ping"]).engine,
            Engine::Cpp
        );
    }

    #[test]
    fn an_unknown_engine_is_a_usage_error() {
        assert!(Cli::try_parse_from(["vicinae", "--engine", "go", "ping"]).is_err());
    }

    #[test]
    fn socket_override_is_used_verbatim() {
        let cli = parse(&["vicinae", "--socket", "/tmp/x.sock", "toggle"]);
        assert_eq!(cli.socket_path().as_path(), Path::new("/tmp/x.sock"));
        assert!(!cli.socket_path().is_fallback());
    }

    #[test]
    fn global_flags_may_follow_the_subcommand() {
        let cli = parse(&[
            "vicinae",
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
        assert!(Cli::try_parse_from(["vicinae"]).is_err());
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
        let cli = parse(&["vicinae", "ext", "list"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: false }));

        let cli = parse(&["vicinae", "ext", "list", "--json"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: true }));
    }
}
