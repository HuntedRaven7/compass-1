//! What the server does with each call: `SnippetService`'s methods.
//!
//! Generic over the key state, the virtual keyboard's sink and the live
//! input, so the injection sequences — which keys, in which order, and when
//! a person's own typing cuts them short — are tested against recorders
//! rather than a real `/dev/uinput`.

use std::time::Duration;

use compass_core::input_server::wire::{self, Call, Event, InjectExpand, InjectUndo};
use compass_platform_linux::keyboard::{EventSink, Modifiers, VirtualKeyboard};
use serde_json::Value;

use crate::tracker::{KEY_BACKSPACE, KeyState, Tracker};

/// `KEY_V`.
pub const KEY_V: u16 = 47;
/// `KEY_LEFT`.
pub const KEY_LEFT: u16 = 105;

/// The person's input while an injection runs.
pub trait LiveInput {
    /// Throw away what is queued (`drainInputEvents`): keys typed before the
    /// injection must not count as interrupting it.
    fn drain(&mut self);
    /// Whether a key was pressed or the pointer moved since the last look
    /// (`checkInputInterrupt`). What is looked at is consumed, as in the C++.
    fn interrupted(&mut self) -> bool;
}

/// The input server's state.
#[derive(Debug)]
pub struct Service<K, S: EventSink> {
    tracker: Tracker<K>,
    keyboard: Result<VirtualKeyboard<S>, String>,
}

impl<K: KeyState, S: EventSink> Service<K, S> {
    /// A service reading keys through `keys` and injecting through
    /// `keyboard` — or, when the virtual keyboard could not be made, only
    /// detecting triggers and saying so in `getCapabilities`.
    pub fn new(keys: K, keyboard: Result<VirtualKeyboard<S>, String>) -> Self {
        Self {
            tracker: Tracker::new(keys),
            keyboard,
        }
    }

    /// The tracker.
    pub fn tracker(&self) -> &Tracker<K> {
        &self.tracker
    }

    /// The virtual keyboard, when there is one.
    pub fn keyboard(&self) -> Option<&VirtualKeyboard<S>> {
        self.keyboard.as_ref().ok()
    }

    /// Whether injection works: `KeyboardCapabilities::injection`.
    #[must_use]
    pub fn can_inject(&self) -> bool {
        self.keyboard.is_ok()
    }

    /// One `EV_KEY` event from a keyboard.
    pub fn key(&mut self, code: u16, value: i32) -> Option<Event> {
        self.tracker.key(code, value)
    }

    /// Forgets what has been typed, as `resetContext` does.
    pub fn forget_typing(&mut self) {
        self.tracker.reset();
    }

    /// The modifier wait ran out.
    pub fn flush_pending(&mut self) -> Option<Event> {
        self.tracker.flush_pending()
    }

    /// Whether an expansion waits on the modifiers.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.tracker.has_pending()
    }

    /// Carries out one call and answers its result.
    ///
    /// # Errors
    ///
    /// A keymap that does not compile, and a snippet with an empty keyword;
    /// every other call succeeds, as in the C++ (an injection without a
    /// virtual keyboard does nothing).
    pub fn call(&mut self, call: Call, input: &mut dyn LiveInput) -> Result<Value, String> {
        match call {
            Call::SetKeymap(layout) => {
                let map = self.tracker.keys_mut().set_layout(&layout)?;
                if let Ok(keyboard) = &mut self.keyboard {
                    keyboard.set_char_map(map);
                }
                Ok(Value::Null)
            }
            Call::CreateSnippet { trigger, .. } if trigger.is_empty() => {
                // An empty keyword matches after every key, so all it could
                // do is report when each key is pressed.
                Err("refusing a snippet with an empty keyword".to_owned())
            }
            Call::CreateSnippet { trigger, mode } => {
                self.tracker.add(trigger, mode.into());
                Ok(wire::create_snippet_result())
            }
            Call::RemoveSnippet { trigger } => {
                self.tracker.remove(&trigger);
                Ok(wire::remove_snippet_result())
            }
            Call::ResetContext => {
                self.tracker.reset();
                Ok(Value::Null)
            }
            Call::InjectExpand(req) => {
                self.inject_expand(req, input);
                Ok(Value::Null)
            }
            Call::InjectUndo(req) => {
                self.inject_undo(&req, input);
                Ok(Value::Null)
            }
            Call::InjectPaste { terminal } => {
                if let Ok(keyboard) = &mut self.keyboard {
                    keyboard.send_key_with_mods(KEY_V, paste_mods(terminal));
                }
                Ok(Value::Null)
            }
            Call::SetKeyDelay(micros) => {
                // An `int` in the C++, handed to `usleep` as unsigned: a
                // negative delay would sleep for an hour. Clamped instead.
                if let Ok(keyboard) = &mut self.keyboard {
                    keyboard.set_key_delay_us(u32::try_from(micros).unwrap_or(0));
                }
                Ok(Value::Null)
            }
            Call::GetCapabilities => Ok(wire::capabilities_result(self.can_inject())),
        }
    }

    /// `injectExpand`: erase the trigger, paste, then walk back to the
    /// cursor placeholder unless the person starts typing.
    fn inject_expand(&mut self, req: InjectExpand, input: &mut dyn LiveInput) {
        let Ok(keyboard) = &mut self.keyboard else {
            return;
        };
        keyboard.repeat_key(KEY_BACKSPACE, req.chars_to_delete);
        if req.pre_paste_delay_us > 0 {
            keyboard
                .sink_mut()
                .delay(Duration::from_micros(u64::from(req.pre_paste_delay_us)));
        }
        keyboard.send_key_with_mods(KEY_V, paste_mods(req.terminal));

        if req.cursor_left_moves > 0 {
            input.drain();
            for _ in 0..req.cursor_left_moves {
                keyboard.send_key_with_mods(KEY_LEFT, Modifiers::NONE);
                if input.interrupted() {
                    self.tracker.reset();
                    return;
                }
            }
        }
    }

    /// `injectUndo`: erase the expansion and type the trigger back, unless
    /// the person starts typing.
    fn inject_undo(&mut self, req: &InjectUndo, input: &mut dyn LiveInput) {
        let Ok(keyboard) = &mut self.keyboard else {
            return;
        };
        input.drain();
        for _ in 0..req.backspace_count {
            keyboard.send_key_with_mods(KEY_BACKSPACE, Modifiers::NONE);
            if input.interrupted() {
                self.tracker.reset();
                return;
            }
        }
        keyboard.type_text(&req.trigger_text);
    }
}

/// Ctrl+V, or Ctrl+Shift+V for a terminal.
fn paste_mods(terminal: bool) -> Modifiers {
    if terminal {
        Modifiers::CTRL.union(Modifiers::SHIFT)
    } else {
        Modifiers::CTRL
    }
}
