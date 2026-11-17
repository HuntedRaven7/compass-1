//! `compass-input-server`: the snippet keyword expander's keyboard hook.
//!
//! A port of `src/snippet/` (the C++ `compass-input-server`). The engine
//! starts this helper, registers the snippet keywords with it, and hears back
//! when one is typed; it answers with the expansion on the clipboard and asks
//! the helper to erase the keyword and paste. The helper is separate because
//! it is the one part of Compass that needs to read every keyboard and to
//! create a virtual one — privileges that belong on a small program that
//! speaks only to its parent over stdin and stdout, not on the launcher.
//!
//! * [`tracker`] — the key-sequence state machine;
//! * [`service`] — each call the engine makes, and the injections;
//! * [`keymap`] — keycodes to text through xkbcommon (and a table for tests);
//! * [`device`] — `/dev/input` and `/dev/uinput` through `evdev`;
//! * [`server`] — the process and its loop;
//! * [`seat`] — serving only the user at the keyboard;
//! * [`recording`] — reading `evtest` logs, for replaying real typing.
//!
//! The wire is `compass_core::input_server`: figura's JSON-RPC in
//! little-endian length-prefixed frames, the same bytes the C++ exchanges.
//!
//! # Permissions
//!
//! Reading `/dev/input/event*` and writing `/dev/uinput` need either root
//! or `CAP_DAC_OVERRIDE`. The packages grant it with
//! `setcap cap_dac_override+ep` (Arch's `compass.install`) or the NixOS
//! module's `security.wrappers`; see `packaging/README.md`. A file
//! capability goes to whoever runs the file, so a privileged helper reads
//! keys and injects only while its user has the active session on a seat
//! ([`seat`]).

#![deny(missing_docs)]

pub mod device;
pub mod keymap;
pub mod recording;
pub mod seat;
pub mod server;
pub mod service;
pub mod tracker;
