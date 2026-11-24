//! The binary the browser spawns by absolute path.
//!
//! Only native messaging is on stdin/stdout; diagnostics go to stderr. The
//! browser reports its tabs with a `tabsChanged` message and the host relays
//! them to Compass; a small thread polls Compass for focus requests and writes
//! them back as `focus` messages.

use std::io;
use std::sync::{Arc, Mutex};

use compass_browser_host::bridge::IpcBridge;
use compass_browser_host::protocol::HostResponse;
use compass_browser_host::{EmptyTabs, SharedWriter, write_frame};

fn main() {
    let browser_id =
        std::env::var("COMPASS_BROWSER_ID").unwrap_or_else(|_| "org.mozilla.firefox".to_owned());
    let bridge = match IpcBridge::connect(browser_id) {
        Ok(bridge) => Arc::new(bridge),
        Err(error) => {
            // Never write diagnostics to stdout: it is the framing channel.
            eprintln!("compass-browser-host: {error}");
            std::process::exit(1);
        }
    };

    let stdout = Arc::new(Mutex::new(io::stdout()));
    let relay_bridge = Arc::clone(&bridge);
    let relay_writer = SharedWriter(Arc::clone(&stdout));
    if let Err(error) = std::thread::Builder::new()
        .name("browser-focus-relay".to_owned())
        .spawn(move || {
            let mut writer = relay_writer;
            loop {
                let Some(tab_id) = relay_bridge.focus_wait() else {
                    continue;
                };
                let Ok(bytes) = serde_json::to_vec(&HostResponse::Focus { tab_id }) else {
                    continue;
                };
                if write_frame(&mut writer, &bytes).is_err() {
                    break;
                }
            }
        })
    {
        // Without the relay the host still reports tabs; focus requests just
        // collect until a restart. Say so rather than failing the browser.
        eprintln!("compass-browser-host: no focus relay: {error}");
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = SharedWriter(stdout);
    if let Err(error) =
        compass_browser_host::serve_with_bridge(&mut reader, &mut writer, &EmptyTabs, Some(&bridge))
    {
        eprintln!("compass-browser-host: {error}");
        std::process::exit(1);
    }
}
