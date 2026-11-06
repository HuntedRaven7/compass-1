//! Tests of the coordinator itself -- booting, the selection arithmetic every
//! page shares, the executor -- and the action audit, which drives every
//! module at once.

use super::*;

#[test]
fn hidden_boot_requires_an_engine_and_waits_for_activation() {
    let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (sender, _outcomes) = tokio::sync::mpsc::unbounded_channel();
    let (mut hidden, _) = LauncherApp::boot(AppFlags {
        start_hidden: true,
        link: Some(EngineLink::new(receiver, sender)),
        ..AppFlags::default()
    });
    assert!(hidden.window.is_none());
    assert!(
        hidden.pending_window.is_none(),
        "hidden boot must never allocate a surface"
    );
    let _ = hidden.update(Message::Command(UiCommand::Show));
    assert!(
        hidden.pending_window.is_some(),
        "the engine can summon the hidden session"
    );

    let (standalone, _) = LauncherApp::boot(AppFlags {
        start_hidden: true,
        ..AppFlags::default()
    });
    assert!(
        standalone.pending_window.is_some(),
        "never strand an undriven UI invisibly"
    );
}

mod action_audit;

#[test]
fn iced_executor_drives_tokio_tasks() {
    let executor = <iced::executor::Default as iced::Executor>::new().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    iced::Executor::spawn(&executor, async move {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn the_selection_clamps_by_default_because_the_cpp_does() {
    // This used to assert the opposite. `Config::wrapNavigation` is
    // `false` in the engine being ported, so Up at the top stays at the
    // top -- and `compass_core::list_navigation` holds the rule.
    assert_eq!(next_selection(3, 0, Direction::Up, false), 0);
    assert_eq!(next_selection(3, 2, Direction::Down, false), 2);
    assert_eq!(next_selection(3, 0, Direction::Down, false), 1);
    assert_eq!(next_selection(3, 2, Direction::Up, false), 1);
}

#[test]
fn the_selection_wraps_when_the_setting_asks_for_it() {
    assert_eq!(next_selection(3, 0, Direction::Up, true), 2);
    assert_eq!(next_selection(3, 2, Direction::Down, true), 0);
}

#[test]
fn selection_on_an_empty_list_is_not_an_index_into_nothing() {
    assert_eq!(next_selection(0, 0, Direction::Up, false), 0);
    assert_eq!(next_selection(0, 0, Direction::Down, true), 0);
}

#[test]
fn a_selection_left_past_the_end_is_clamped_rather_than_underflowing() {
    // Reachable if a list shrinks without the selection being reset. The
    // arithmetic here is unsigned, so getting this wrong is a panic.
    assert_eq!(next_selection(2, 9, Direction::Up, false), 0);
    assert_eq!(next_selection(1, 5, Direction::Down, false), 0);
}
