//! The binary the browser spawns by absolute path.
//!
//! Only native messaging is on stdin/stdout; diagnostics go to stderr. The
//! browser reports its tabs with a `tabsChanged` message and the host relays
//! them to Compass; a small thread polls Compass for focus requests and writes
//! them back as `focus` messages.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_browser_host::bridge::IpcBridge;
use compass_browser_host::protocol::HostResponse;
use compass_browser_host::{EmptyTabs, FrameWriter, SharedWriter};

fn main() {
    let default_id =
        std::env::var("COMPASS_BROWSER_ID").unwrap_or_else(|_| "org.mozilla.firefox".to_owned());
    let bridge = match IpcBridge::new(default_id) {
        Ok(bridge) => Arc::new(bridge),
        Err(error) => {
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
                let ids = relay_bridge.reported_ids();
                if ids.is_empty() {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                let mut reachable = false;
                for id in ids {
                    match relay_bridge.focus_wait(&id) {
                        Ok(Some(tab_id)) => {
                            reachable = true;
                            let Ok(bytes) = serde_json::to_vec(&HostResponse::Focus { tab_id })
                            else {
                                continue;
                            };
                            if writer.frame(&bytes).is_err() {
                                return;
                            }
                        }
                        Ok(None) => reachable = true,
                        Err(error) => {
                            // One browser's poll failing does not stop the
                            // others; back off if none answered at all.
                            eprintln!("compass-browser-host: focus poll failed: {error}");
                        }
                    }
                }
                if !reachable {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        })
    {
        eprintln!("compass-browser-host: no focus relay: {error}");
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = SharedWriter(stdout);
    if let Err(error) =
        compass_browser_host::serve_with_bridge(&mut reader, &mut writer, &EmptyTabs, Some(&bridge))
    {
        eprintln!("compass-browser-host: {error}");
        bridge.disconnect_all();
        std::process::exit(1);
    }
    // The browser closed the pipe. Tell the engine so its tab list and
    // BrowserExtension capability stop advertising a browser that is gone.
    bridge.disconnect_all();
}
