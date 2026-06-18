//! Workspace state-machine transitions and the event log.

use arghda_core::{event, EventKind, State, Workspace};
use std::fs;

#[test]
fn full_lifecycle_moves_file_and_logs_events() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ws = Workspace::init(root).unwrap();

    fs::write(
        ws.state_dir(State::Inbox).join("Foo.agda"),
        "module Foo where\n",
    )
    .unwrap();
    assert_eq!(ws.state_of("Foo.agda"), Some(State::Inbox));

    ws.transition("Foo.agda", State::Inbox, State::Working, None)
        .unwrap();
    assert_eq!(ws.state_of("Foo.agda"), Some(State::Working));

    ws.transition(
        "Foo.agda",
        State::Working,
        State::Proven,
        Some("typecheck clean".into()),
    )
    .unwrap();
    assert_eq!(ws.state_of("Foo.agda"), Some(State::Proven));

    let events = event::read_all(root).unwrap();
    assert_eq!(events.len(), 2, "two transitions logged");
    assert_eq!(events[0].kind, EventKind::Claim);
    assert_eq!(events[0].from, Some(State::Inbox));
    assert_eq!(events[0].to, Some(State::Working));
    assert_eq!(events[1].kind, EventKind::Promote);
    assert_eq!(events[1].note.as_deref(), Some("typecheck clean"));
    // Timestamps are RFC3339 Z-form.
    assert!(events[0].ts.ends_with('Z') && events[0].ts.len() == 20);
}

#[test]
fn disallowed_transition_is_rejected_and_leaves_no_trace() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    fs::write(ws.state_dir(State::Inbox).join("Bar.agda"), "x").unwrap();

    // inbox -> proven is not a legal move.
    assert!(ws
        .transition("Bar.agda", State::Inbox, State::Proven, None)
        .is_err());

    // File stays put; nothing is logged.
    assert_eq!(ws.state_of("Bar.agda"), Some(State::Inbox));
    assert!(event::read_all(tmp.path()).unwrap().is_empty());
}

#[test]
fn transition_of_absent_file_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    assert!(ws
        .transition("Ghost.agda", State::Inbox, State::Working, None)
        .is_err());
}

#[test]
fn requeue_round_trips_back_to_inbox() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    fs::write(ws.state_dir(State::Rejected).join("Re.agda"), "x").unwrap();
    ws.transition("Re.agda", State::Rejected, State::Inbox, None)
        .unwrap();
    assert_eq!(ws.state_of("Re.agda"), Some(State::Inbox));
    let events = event::read_all(tmp.path()).unwrap();
    assert_eq!(events[0].kind, EventKind::Requeue);
}
