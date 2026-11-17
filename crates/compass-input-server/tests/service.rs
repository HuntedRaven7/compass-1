//! The calls the engine makes, and what the virtual keyboard is made to type.
//!
//! The keyboard writes to a recorder, never a device: nothing here reaches
//! the session's input.

use std::collections::VecDeque;
use std::time::Duration;

use compass_core::input_server::wire::{Call, Event, ExpansionMode, InjectExpand, InjectUndo};
use compass_input_server::keymap::UsKeys;
use compass_input_server::service::{KEY_LEFT, KEY_V, LiveInput, Service};
use compass_input_server::tracker::KEY_BACKSPACE;
use compass_platform_linux::keyboard::{
    EventSink, KEY_LEFTCTRL, KEY_LEFTSHIFT, KeyEvent, VirtualKeyboard,
};
use serde_json::{Value, json};

#[derive(Debug, Default)]
struct Recorder {
    events: Vec<KeyEvent>,
    waited: Vec<Duration>,
}

impl EventSink for Recorder {
    fn emit(&mut self, event: KeyEvent) {
        self.events.push(event);
    }
    fn delay(&mut self, duration: Duration) {
        self.waited.push(duration);
    }
}

/// Live input scripted per interrupt check: `true` means the person typed.
#[derive(Debug, Default)]
struct Scripted {
    checks: VecDeque<bool>,
    drained: usize,
}

impl LiveInput for Scripted {
    fn drain(&mut self) {
        self.drained += 1;
    }
    fn interrupted(&mut self) -> bool {
        self.checks.pop_front().unwrap_or(false)
    }
}

fn service() -> Service<UsKeys, Recorder> {
    let mut keyboard = VirtualKeyboard::new(Recorder::default());
    keyboard.set_char_map(UsKeys::char_map());
    Service::new(UsKeys::new(), Ok(keyboard))
}

fn presses(service: &Service<UsKeys, Recorder>) -> Vec<u16> {
    service
        .keyboard()
        .unwrap()
        .sink()
        .events
        .iter()
        .filter_map(|event| match event {
            KeyEvent::Press(code) => Some(*code),
            _ => None,
        })
        .collect()
}

fn call(service: &mut Service<UsKeys, Recorder>, call: Call) -> Value {
    service
        .call(call, &mut Scripted::default())
        .expect("the call succeeds")
}

#[test]
fn registered_keywords_fire_and_removed_ones_do_not() {
    let mut service = service();
    assert_eq!(
        call(
            &mut service,
            Call::CreateSnippet {
                trigger: "x".into(),
                mode: ExpansionMode::Keydown,
            }
        ),
        json!({"ok": false}),
        "CreateSnippetResponse{{}} as the C++ answers it"
    );
    assert_eq!(service.key(45, 1), Some(Event::Trigger("x".into())));
    call(
        &mut service,
        Call::RemoveSnippet {
            trigger: "x".into(),
        },
    );
    assert_eq!(service.key(45, 1), None);
}

#[test]
fn an_empty_keyword_is_refused_rather_than_reporting_every_key() {
    let mut service = service();
    assert!(
        service
            .call(
                Call::CreateSnippet {
                    trigger: String::new(),
                    mode: ExpansionMode::Keydown,
                },
                &mut Scripted::default(),
            )
            .is_err()
    );
    assert_eq!(service.key(45, 1), None, "a key press fired a trigger");
}

#[test]
fn an_expansion_erases_the_keyword_then_pastes() {
    let mut service = service();
    call(
        &mut service,
        Call::InjectExpand(InjectExpand {
            chars_to_delete: 4,
            pre_paste_delay_us: 1500,
            terminal: false,
            cursor_left_moves: 0,
        }),
    );
    assert_eq!(
        presses(&service),
        vec![
            KEY_BACKSPACE,
            KEY_BACKSPACE,
            KEY_BACKSPACE,
            KEY_BACKSPACE,
            KEY_LEFTCTRL,
            KEY_V
        ]
    );
    assert!(
        service
            .keyboard()
            .unwrap()
            .sink()
            .waited
            .contains(&Duration::from_micros(1500)),
        "the pre-paste delay is waited"
    );
}

#[test]
fn a_terminal_gets_ctrl_shift_v() {
    let mut service = service();
    call(&mut service, Call::InjectPaste { terminal: true });
    assert_eq!(presses(&service), vec![KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V]);
}

#[test]
fn the_cursor_walk_stops_when_the_person_types() {
    let mut service = service();
    let mut input = Scripted {
        checks: VecDeque::from([false, false, true]),
        drained: 0,
    };
    service
        .call(
            Call::InjectExpand(InjectExpand {
                chars_to_delete: 1,
                pre_paste_delay_us: 0,
                terminal: false,
                cursor_left_moves: 10,
            }),
            &mut input,
        )
        .unwrap();
    let lefts = presses(&service)
        .into_iter()
        .filter(|code| *code == KEY_LEFT)
        .count();
    assert_eq!(
        lefts, 3,
        "the third left saw the interruption and was the last"
    );
    assert_eq!(input.drained, 1, "keys typed before the walk do not count");
}

#[test]
fn an_undo_erases_the_expansion_and_types_the_keyword_back() {
    let mut service = service();
    call(
        &mut service,
        Call::InjectUndo(InjectUndo {
            backspace_count: 2,
            trigger_text: ";S".into(),
        }),
    );
    assert_eq!(
        presses(&service),
        vec![KEY_BACKSPACE, KEY_BACKSPACE, 39, KEY_LEFTSHIFT, 31]
    );
}

#[test]
fn an_interrupted_undo_types_nothing_back() {
    let mut service = service();
    let mut input = Scripted {
        checks: VecDeque::from([true]),
        drained: 0,
    };
    service
        .call(
            Call::InjectUndo(InjectUndo {
                backspace_count: 5,
                trigger_text: ";sig".into(),
            }),
            &mut input,
        )
        .unwrap();
    assert_eq!(presses(&service), vec![KEY_BACKSPACE]);
}

#[test]
fn without_a_virtual_keyboard_triggers_still_fire_and_injection_is_a_no_op() {
    let mut service: Service<UsKeys, Recorder> =
        Service::new(UsKeys::new(), Err("no /dev/uinput".into()));
    assert_eq!(
        call(&mut service, Call::GetCapabilities),
        json!({"injection": false})
    );
    call(&mut service, Call::InjectPaste { terminal: false });
    call(
        &mut service,
        Call::CreateSnippet {
            trigger: "x".into(),
            mode: ExpansionMode::Keydown,
        },
    );
    assert_eq!(service.key(45, 1), Some(Event::Trigger("x".into())));
}

#[test]
fn a_negative_key_delay_is_zero() {
    let mut service = service();
    call(&mut service, Call::SetKeyDelay(-5));
    call(&mut service, Call::InjectPaste { terminal: false });
    let waited = &service.keyboard().unwrap().sink().waited;
    assert!(waited.contains(&Duration::ZERO), "{waited:?}");
}

#[test]
fn a_layout_that_does_not_compile_is_an_error() {
    let mut service = service();
    let result = service.call(
        Call::SetKeymap(compass_core::input_server::wire::LayoutInfo {
            layout: "zz".into(),
            ..Default::default()
        }),
        &mut Scripted::default(),
    );
    assert!(result.is_err());
}

#[test]
fn reset_context_forgets_the_typing() {
    let mut service = service();
    call(
        &mut service,
        Call::CreateSnippet {
            trigger: "sx".into(),
            mode: ExpansionMode::Keydown,
        },
    );
    assert_eq!(service.key(31, 1), None);
    call(&mut service, Call::ResetContext);
    assert_eq!(service.key(45, 1), None);
}
