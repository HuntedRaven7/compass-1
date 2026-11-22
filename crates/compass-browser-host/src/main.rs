//! The binary the browser spawns by absolute path.
//!
//! Only native messaging is on stdin/stdout; diagnostics go to stderr. The tab
//! source is the prototype's empty one — #316 replaces it with a client of the
//! engine's IPC socket. Keeping that swap behind [`compass_browser_host::TabSource`]
//! is the whole point of the prototype.

use std::io::{self, Write};

fn main() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    if let Err(error) =
        compass_browser_host::serve(&mut reader, &mut writer, &compass_browser_host::EmptyTabs)
    {
        // Never write diagnostics to stdout: it is the framing channel.
        eprintln!("compass-browser-host: {error}");
        std::process::exit(1);
    }
    let _ = writer.flush();
}
