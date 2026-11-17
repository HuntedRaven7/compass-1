//! The process: stdin and stdout to the engine, the devices, the loop.
//!
//! `SnippetService::listen`, with threads and a channel where the C++ has
//! one epoll set: a thread reads stdin, one reads each device, and inotify
//! reports new nodes; this thread handles what they send, in order, and is
//! the only one that writes to stdout or to the virtual keyboard.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use compass_core::input_server::wire::{self, Call};
use compass_core::input_server::{MAX_MESSAGE_SIZE, MessageBuffer, frame};
use compass_platform_linux::keyboard::VirtualKeyboard;

use crate::device::{self, DeviceMessage, Kind, UinputSink};
use crate::keymap::XkbKeys;
use crate::recording::EV_KEY;
use crate::seat::{Gate, Verdict};
use crate::service::{LiveInput, Service};
use crate::tracker::MODIFIER_TIMEOUT;

/// Everything the loop is told.
#[derive(Debug)]
pub enum Message {
    /// Bytes from the engine.
    Control(Vec<u8>),
    /// The engine closed stdin: time to go.
    ControlClosed,
    /// A device thread or the hot-plug watch.
    Device(DeviceMessage),
}

impl From<DeviceMessage> for Message {
    fn from(message: DeviceMessage) -> Self {
        Self::Device(message)
    }
}

/// Runs the server until the engine closes stdin. Returns the exit code.
#[must_use]
pub fn run() -> u8 {
    let (keys, map) = match XkbKeys::new() {
        Ok(found) => found,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let keyboard = UinputSink::create()
        .map(|sink| {
            let mut keyboard = VirtualKeyboard::new(sink);
            keyboard.set_char_map(map);
            keyboard
        })
        .map_err(|error| format!("Failed to open /dev/uinput: {error}"));
    if let Err(error) = &keyboard {
        eprintln!("{error}; triggers are detected but nothing can be injected");
    }
    let mut service = Service::new(keys, keyboard);
    let mut gate = Gate::new();
    match gate.verdict() {
        Verdict::Allowed => {}
        Verdict::Refused => eprintln!(
            "this user does not have the active session on a seat; no key is read or typed until \
             it does"
        ),
        Verdict::Unknown => eprintln!(
            "no logind on the system bus to say who is at the keyboard; serving whoever the \
             install lets run this helper"
        ),
    }

    let (tx, rx) = mpsc::channel::<Message>();
    spawn_stdin_reader(tx.clone());

    let _watcher = match device::watch_hotplug(tx.clone()) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            eprintln!(
                "not watching {} for new devices: {error}",
                device::INPUT_DIR
            );
            None
        }
    };

    let mut devices: BTreeMap<PathBuf, Kind> = BTreeMap::new();
    for node in device::event_nodes() {
        register(&mut devices, node, device::open, &tx);
    }
    for (node, kind) in &devices {
        let label = if kind.keyboard { "Keyboard" } else { "Pointer" };
        eprintln!("{label} ready: {}", node.display());
    }
    if devices.values().all(|kind| !kind.keyboard) {
        eprintln!(
            "no keyboard could be opened under {}; snippet keywords will not expand",
            device::INPUT_DIR
        );
    }

    let mut out = std::io::stdout().lock();
    let mut buffer = MessageBuffer::new();
    let mut input = Live {
        rx: &rx,
        deferred: VecDeque::new(),
    };

    loop {
        let message = match input.deferred.pop_front() {
            Some(message) => message,
            None if service.has_pending() => match rx.recv_timeout(MODIFIER_TIMEOUT) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!("Modifier release timeout, emitting anyway");
                    if let Some(event) = service.flush_pending() {
                        send(&mut out, &event.encode());
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return 0,
            },
            None => match rx.recv() {
                Ok(message) => message,
                Err(_) => return 0,
            },
        };

        match message {
            Message::ControlClosed => {
                eprintln!("the engine closed the connection");
                return 0;
            }
            Message::Control(bytes) => {
                for request in buffer.push(&bytes) {
                    let answer = match Call::decode(&request) {
                        Ok((id, call)) if injects(&call) && !gate.allows() => {
                            wire::reply_error(id, NOT_AT_THE_KEYBOARD)
                        }
                        Ok((id, call)) => match service.call(call, &mut input) {
                            Ok(result) => wire::reply(id, &result),
                            Err(error) => {
                                eprintln!("{error}");
                                wire::reply_error(id, &error)
                            }
                        },
                        Err((Some(id), error)) => wire::reply_error(id, &error),
                        Err((None, error)) => {
                            eprintln!("unreadable request: {error}");
                            continue;
                        }
                    };
                    send(&mut out, &answer);
                }
                if buffer
                    .announced()
                    .is_some_and(|size| size > MAX_MESSAGE_SIZE)
                {
                    eprintln!("a request over {MAX_MESSAGE_SIZE} bytes; closing");
                    return 1;
                }
            }
            Message::Device(DeviceMessage::Appeared(node)) => {
                if !devices.contains_key(&node) {
                    register(&mut devices, node, |path| Ok(device::open_new(path)), &tx);
                }
            }
            Message::Device(DeviceMessage::Gone(node)) => {
                if devices.remove(&node).is_some() {
                    eprintln!("Removed device {}", node.display());
                }
            }
            Message::Device(DeviceMessage::Input { kind, event, .. }) => {
                if !kind.keyboard || event.kind != EV_KEY {
                    continue;
                }
                if !gate.allows() {
                    // Another user's typing: not one key of it reaches the
                    // tracker, and nothing typed before the switch is kept.
                    service.forget_typing();
                    continue;
                }
                if let Some(event) = service.key(event.code, event.value) {
                    send(&mut out, &event.encode());
                }
            }
        }
    }
}

/// Why an injection was refused.
const NOT_AT_THE_KEYBOARD: &str = "not injecting: this user does not have the active session";

/// Whether `call` types into the focused window.
fn injects(call: &Call) -> bool {
    matches!(
        call,
        Call::InjectExpand(_) | Call::InjectUndo(_) | Call::InjectPaste { .. }
    )
}

/// Opens `node` with `open` and, if it is a keyboard or a pointer, starts
/// reading it.
fn register(
    devices: &mut BTreeMap<PathBuf, Kind>,
    node: PathBuf,
    open: impl FnOnce(&std::path::Path) -> std::io::Result<Option<(evdev::Device, Kind)>>,
    tx: &Sender<Message>,
) {
    match open(&node) {
        Ok(Some((device, kind))) => {
            devices.insert(node.clone(), kind);
            device::spawn_reader(node, device, kind, tx.clone());
        }
        Ok(None) => {}
        Err(error) => eprintln!("Failed to open {}: {error}", node.display()),
    }
}

/// Writes one framed message and flushes it. A write failure means the
/// engine is gone; the next read of stdin will end the loop.
fn send(out: &mut impl Write, payload: &[u8]) {
    if let Err(error) = out.write_all(&frame(payload)).and_then(|()| out.flush()) {
        eprintln!("could not write to the engine: {error}");
    }
}

fn spawn_stdin_reader(tx: Sender<Message>) {
    let spawned = std::thread::Builder::new()
        .name("stdin".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut chunk = [0u8; 8192];
            loop {
                match stdin.read(&mut chunk) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(Message::ControlClosed);
                        return;
                    }
                    Ok(read) => {
                        if tx.send(Message::Control(chunk[..read].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("could not start the stdin reader: {error}");
    }
}

/// The queue during an injection: device events are looked at (and used up)
/// by the interrupt check, everything else waits its turn.
struct Live<'a> {
    rx: &'a Receiver<Message>,
    deferred: VecDeque<Message>,
}

impl LiveInput for Live<'_> {
    fn drain(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            if !matches!(message, Message::Device(DeviceMessage::Input { .. })) {
                self.deferred.push_back(message);
            }
        }
    }

    fn interrupted(&mut self) -> bool {
        let mut interrupted = false;
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Device(DeviceMessage::Input { kind, event, .. }) => {
                    if kind.pointer || (event.kind == EV_KEY && event.value == 1) {
                        interrupted = true;
                    }
                }
                other => self.deferred.push_back(other),
            }
        }
        interrupted
    }
}
