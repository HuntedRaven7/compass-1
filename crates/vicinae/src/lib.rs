//! The `vicinae` command-line front end.
//!
//! Phase 2 of `docs/rust-engine/PLAN.md` §6: an IPC client for controlling a
//! running engine ([`ipc`]), and the diagnostic that tells a user — and a bug
//! report — what this machine can and cannot do ([`doctor`]).
//!
//! Everything is exposed as a library rather than buried in `main.rs` so the
//! checks, the report rendering and the command dispatch are all reachable from
//! tests. `main` is four lines.

#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod cli;
pub mod doctor;
pub mod engine;
pub mod ipc;
pub mod serve;

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Parser;
use compass_ipc::{Request, Response};

pub use cli::{Cli, Command};
pub use engine::Engine;

/// Exit code when the command did what it was asked.
pub const EXIT_OK: u8 = 0;
/// Exit code when the command failed, or `doctor --check-only` found a failure.
pub const EXIT_FAILURE: u8 = 1;

/// Parses the command line, runs the command, and reports failures.
///
/// Usage errors exit 2 from inside `clap`; see [`cli::EXIT_CODE_HELP`].
#[must_use]
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Runs an already-parsed command line on a fresh runtime.
///
/// The client commands get a current-thread runtime: they open one connection,
/// send one frame and exit, so a thread pool is pure startup cost on something
/// a person has bound to a key. `serve` gets a multi-threaded one, because
/// ranking a query is CPU work and on a single thread one slow query would
/// stall every other connection.
pub fn run(cli: Cli) -> Result<ExitCode> {
    let runtime = if matches!(cli.command, Command::Serve) {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
    };
    runtime.block_on(dispatch(cli))
}

async fn dispatch(cli: Cli) -> Result<ExitCode> {
    let socket = cli.socket_path();

    match cli.command {
        Command::Doctor { check_only, json } => {
            // `doctor` runs whatever engine was asked for: refusing to
            // diagnose a machine because of the flag under diagnosis would be
            // perverse. The selection is reported as its own check instead.
            let report = doctor::run_on_this_machine(&socket, cli.engine).await;

            if json {
                println!("{}", serde_json::to_string_pretty(&report.to_json())?);
            } else {
                print!("{}", report.render_human(check_only));
            }

            Ok(ExitCode::from(doctor_exit_code(check_only, &report)))
        }

        Command::Ping => {
            require_servable_engine(cli.engine)?;
            println!("{}", ipc::ping(&socket).await?);
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Serve => {
            require_servable_engine(cli.engine)?;
            serve::run(&socket).await?;
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Shutdown => {
            require_servable_engine(cli.engine)?;
            match ipc::send(&socket, Request::Shutdown).await? {
                Response::ShuttingDown => Ok(ExitCode::from(EXIT_OK)),
                other => bail!("the engine answered {other:?} instead of shutting down"),
            }
        }

        Command::Query { text, json } => {
            require_servable_engine(cli.engine)?;
            let hits = ipc::query(&socket, &text.join(" ")).await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                print!("{}", render_hits(&hits));
            }
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Toggle => window_command(&socket, cli.engine, Request::Toggle).await,
        Command::Show => window_command(&socket, cli.engine, Request::Show).await,
        Command::Hide => window_command(&socket, cli.engine, Request::Hide).await,
    }
}

async fn window_command(
    socket: &compass_ipc::SocketPath,
    engine: Engine,
    request: Request,
) -> Result<ExitCode> {
    require_servable_engine(engine)?;
    ipc::send_ack(socket, request).await?;
    Ok(ExitCode::from(EXIT_OK))
}

/// Renders query hits for a terminal.
///
/// Deliberately plain and column-aligned rather than decorated: this output is
/// read by people debugging the index and piped into `grep` and `awk` at least
/// as often as it is read directly.
#[must_use]
pub fn render_hits(hits: &[compass_ipc::QueryHit]) -> String {
    if hits.is_empty() {
        return "no matches\n".to_owned();
    }

    let width = hits
        .iter()
        .map(|h| h.title.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for hit in hits {
        use std::fmt::Write as _;
        let pad = width - hit.title.chars().count();
        let _ = write!(out, "{:>3}  {}{:pad$}", hit.score, hit.title, "");
        match &hit.subtitle {
            Some(subtitle) => {
                let _ = writeln!(out, "  {subtitle}");
            }
            None => out.push('\n'),
        }
    }
    out
}

/// The exit code `doctor` reports.
///
/// Without `--check-only` the report is the product and the command succeeded
/// by producing it, whatever it says. With `--check-only` the exit code *is*
/// the product: non-zero for a failing check, zero for warnings, because
/// warnings are degradations this build is designed to run under (PLAN §3.5.1)
/// and failing CI on them would just get `--check-only` removed from the job.
#[must_use]
pub fn doctor_exit_code(check_only: bool, report: &doctor::Report) -> u8 {
    if check_only && report.has_failures() {
        EXIT_FAILURE
    } else {
        EXIT_OK
    }
}

/// Refuses commands that would have to be dispatched to an engine this binary
/// cannot start. See [`engine`] for why the default is `rust`.
fn require_servable_engine(engine: Engine) -> Result<()> {
    if engine.is_served_by_this_binary() {
        return Ok(());
    }
    anyhow::bail!(
        "--engine {engine} was selected, but this binary is the Rust engine and cannot dispatch \
         to the C++ one (PLAN.md §5: the dispatching front end arrives with the Phase 7 \
         cutover).\n\
         \x20 - run the C++ `vicinae` directly, or\n\
         \x20 - pass --engine rust / set COMPASS_ENGINE=rust\n\
         `vicinae doctor` still runs under either setting and reports which one is in effect."
    )
}

fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;

    let default = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("vicinae={default},compass_ipc={default}")));

    // A second call (from a test harness, say) is not an error worth aborting
    // for: the first subscriber wins.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_ipc::{DoctorCheck, DoctorStatus};

    fn report_of(statuses: &[DoctorStatus]) -> doctor::Report {
        doctor::Report {
            engine: Engine::Rust,
            socket: "/tmp/t.sock".to_string(),
            checks: statuses
                .iter()
                .enumerate()
                .map(|(i, status)| DoctorCheck {
                    name: format!("c{i}"),
                    status: *status,
                    detail: None,
                })
                .collect(),
        }
    }

    #[test]
    fn check_only_exits_zero_when_every_check_passes() {
        let report = report_of(&[DoctorStatus::Ok, DoctorStatus::Ok]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_OK);
    }

    #[test]
    fn check_only_exits_zero_on_warnings_alone() {
        let report = report_of(&[DoctorStatus::Ok, DoctorStatus::Warn]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_OK);
    }

    #[test]
    fn check_only_exits_non_zero_on_a_failure() {
        let report = report_of(&[DoctorStatus::Warn, DoctorStatus::Fail]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_FAILURE);
        assert_ne!(EXIT_FAILURE, EXIT_OK);
    }

    #[test]
    fn without_check_only_a_failing_report_still_exits_zero() {
        let report = report_of(&[DoctorStatus::Fail]);
        assert_eq!(doctor_exit_code(false, &report), EXIT_OK);
    }

    #[test]
    fn the_rust_engine_is_servable_here() {
        assert!(require_servable_engine(Engine::Rust).is_ok());
    }

    #[test]
    fn the_cpp_engine_is_refused_with_an_explanation() {
        let err = require_servable_engine(Engine::Cpp).expect_err("cpp is not servable");
        let text = err.to_string();
        assert!(text.contains("cannot dispatch"));
        assert!(text.contains("--engine rust"));
        assert!(text.contains("doctor"));
    }
}
