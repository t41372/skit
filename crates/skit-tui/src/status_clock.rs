//! How long a status line stays on screen, measured on an injected clock.
//!
//! The reducer says what kind of news a status is, and `StatusKind::display_time` says how long
//! that kind stays. This module applies the rule in a terminal session: a receipt or a note
//! clears itself when its time ends, and a warning or an error clears at the next key press or
//! click. The caller passes the current instant on every call, so a test moves the clock without
//! sleeping.

use std::time::Instant;

use ratatui_crossterm::crossterm::event::{Event, KeyEventKind, MouseEventKind};
use skit_ui::{Action, LibraryState, StatusKind};

use crate::terminal::TerminalEventWait;

/// The status line that shows, and the instant it first showed.
#[derive(Clone, Debug, Default)]
pub(crate) struct StatusClock {
    shown: Option<ShownStatus>,
}

#[derive(Clone, Debug)]
struct ShownStatus {
    /// The [`LibraryState::status_serial`] of the status.
    serial: u64,
    kind: StatusKind,
    since: Instant,
}

impl StatusClock {
    /// Start the time of a status that a state change showed.
    ///
    /// Call this after every state change. The status that already shows keeps its start instant.
    /// A new status starts a new time, also when it has the same words and kind: a second
    /// operation gives a second receipt.
    pub(crate) fn observe(&mut self, state: &LibraryState, now: Instant) {
        if state.status().is_none() {
            self.shown = None;
            return;
        }
        let serial = state.status_serial();
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.serial == serial)
        {
            return;
        }
        self.shown = Some(ShownStatus {
            serial,
            kind: state.status_kind(),
            since: now,
        });
    }

    /// The instant at which the status that shows clears itself, if it does.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        let shown = self.shown.as_ref()?;
        shown
            .kind
            .display_time()
            .map(|time| shown.since.checked_add(time).unwrap_or(shown.since))
    }

    /// Clear a status whose time ended. Return whether the state changed.
    pub(crate) fn expire(&mut self, state: &mut LibraryState, now: Instant) -> bool {
        if self.deadline().is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.shown = None;
        let _ = state.update(Action::ClearStatus);
        true
    }

    /// Clear a warning or an error when the user presses a key, pastes, or clicks.
    ///
    /// A receipt keeps its time. Return whether the state changed.
    pub(crate) fn acknowledge(&mut self, state: &mut LibraryState, event: &Event) -> bool {
        let is_input = match event {
            Event::Key(key) => key.kind != KeyEventKind::Release,
            Event::Mouse(mouse) => matches!(mouse.kind, MouseEventKind::Down(_)),
            Event::Paste(_) => true,
            Event::FocusGained | Event::FocusLost | Event::Resize(_, _) => false,
        };
        if !is_input || state.status().is_none() || state.status_kind().display_time().is_some() {
            return false;
        }
        self.shown = None;
        let _ = state.update(Action::ClearStatus);
        true
    }

    /// Wait for the next event, but wake up in time to clear the status that shows.
    pub(crate) fn event_wait(&self, wait: TerminalEventWait, now: Instant) -> TerminalEventWait {
        let Some(deadline) = self.deadline() else {
            return wait;
        };
        let remaining = deadline.saturating_duration_since(now);
        match wait {
            TerminalEventWait::Blocking => TerminalEventWait::Poll(remaining),
            TerminalEventWait::Poll(duration) => TerminalEventWait::Poll(duration.min(remaining)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui_crossterm::crossterm::event::{
        KeyCode, KeyEvent, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    };
    use skit_application::LibraryScan;

    use super::*;

    fn state_with(kind: StatusKind, text: &str) -> LibraryState {
        let mut state = LibraryState::from_scan(LibraryScan::default());
        let _ = state.update(Action::SetStatus {
            kind,
            message: text.to_owned(),
        });
        state
    }

    fn key(kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind,
            state: KeyEventState::NONE,
        })
    }

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    // Ways this can fail: the receipt never clears, clears before its time, or clears at a key
    // press; the deadline moves while the same status shows; a later status keeps the old start.
    #[test]
    fn a_receipt_clears_exactly_when_its_time_ends() {
        let start = Instant::now();
        let mut state = state_with(StatusKind::Success, "Saved");
        let mut clock = StatusClock::default();
        clock.observe(&state, start);
        assert_eq!(clock.deadline(), Some(start + Duration::from_secs(4)));

        let almost = start + Duration::from_millis(3_999);
        clock.observe(&state, almost);
        assert_eq!(clock.deadline(), Some(start + Duration::from_secs(4)));
        assert!(!clock.expire(&mut state, almost));
        assert_eq!(state.status(), Some("Saved"));

        assert!(!clock.acknowledge(&mut state, &key(KeyEventKind::Press)));
        assert_eq!(state.status(), Some("Saved"));

        assert!(clock.expire(&mut state, start + Duration::from_secs(4)));
        assert_eq!(state.status(), None);
        assert_eq!(clock.deadline(), None);
        assert!(!clock.expire(&mut state, start + Duration::from_secs(9)));
    }

    #[test]
    fn a_new_status_starts_a_new_time() {
        let start = Instant::now();
        let mut state = state_with(StatusKind::Success, "Saved");
        let mut clock = StatusClock::default();
        clock.observe(&state, start);

        let later = start + Duration::from_secs(3);
        let _ = state.update(Action::SetStatus {
            kind: StatusKind::Info,
            message: "Another".to_owned(),
        });
        clock.observe(&state, later);
        assert_eq!(clock.deadline(), Some(later + Duration::from_secs(4)));
        assert!(!clock.expire(&mut state, start + Duration::from_secs(4)));
        assert_eq!(state.status(), Some("Another"));

        // The same text with another kind is a new status too.
        let _ = state.update(Action::SetStatus {
            kind: StatusKind::Success,
            message: "Another".to_owned(),
        });
        let latest = start + Duration::from_secs(5);
        clock.observe(&state, latest);
        assert_eq!(clock.deadline(), Some(latest + Duration::from_secs(4)));

        // So is the same text with the same kind: a second operation gave a second receipt.
        let _ = state.update(Action::SetStatus {
            kind: StatusKind::Success,
            message: "Another".to_owned(),
        });
        let again = start + Duration::from_secs(8);
        clock.observe(&state, again);
        assert_eq!(clock.deadline(), Some(again + Duration::from_secs(4)));
        assert!(!clock.expire(&mut state, latest + Duration::from_secs(4)));
        assert_eq!(state.status(), Some("Another"));
    }

    #[test]
    fn a_problem_has_no_time_and_clears_at_the_next_key_paste_or_click() {
        for kind in [StatusKind::Warning, StatusKind::Error] {
            for input in [
                key(KeyEventKind::Press),
                key(KeyEventKind::Repeat),
                Event::Paste("text".to_owned()),
                mouse(MouseEventKind::Down(MouseButton::Left)),
                mouse(MouseEventKind::Down(MouseButton::Right)),
            ] {
                let start = Instant::now();
                let mut state = state_with(kind, "Failed");
                let mut clock = StatusClock::default();
                clock.observe(&state, start);
                assert_eq!(clock.deadline(), None);
                assert!(!clock.expire(&mut state, start + Duration::from_secs(3_600)));
                assert_eq!(state.status(), Some("Failed"));

                for passive in [
                    key(KeyEventKind::Release),
                    mouse(MouseEventKind::Moved),
                    mouse(MouseEventKind::ScrollDown),
                    mouse(MouseEventKind::Up(MouseButton::Left)),
                    mouse(MouseEventKind::Drag(MouseButton::Left)),
                    Event::FocusGained,
                    Event::FocusLost,
                    Event::Resize(80, 24),
                ] {
                    assert!(!clock.acknowledge(&mut state, &passive), "{passive:?}");
                    assert_eq!(state.status(), Some("Failed"), "{passive:?}");
                }
                assert!(clock.acknowledge(&mut state, &input), "{input:?}");
                assert_eq!(state.status(), None);
                assert_eq!(state.status_kind(), StatusKind::Info);
                assert!(!clock.acknowledge(&mut state, &input));
            }
        }
    }

    #[test]
    fn a_cleared_status_stops_the_clock() {
        let start = Instant::now();
        let mut state = state_with(StatusKind::Info, "Note");
        let mut clock = StatusClock::default();
        clock.observe(&state, start);
        let _ = state.update(Action::ClearStatus);
        clock.observe(&state, start + Duration::from_secs(1));
        assert_eq!(clock.deadline(), None);
        assert!(!clock.expire(&mut state, start + Duration::from_secs(10)));
    }

    #[test]
    fn the_event_wait_wakes_up_for_the_deadline_and_never_waits_longer() {
        let start = Instant::now();
        let state = state_with(StatusKind::Success, "Saved");
        let mut clock = StatusClock::default();
        assert_eq!(
            clock.event_wait(TerminalEventWait::Blocking, start),
            TerminalEventWait::Blocking
        );
        clock.observe(&state, start);
        let now = start + Duration::from_secs(1);
        assert_eq!(
            clock.event_wait(TerminalEventWait::Blocking, now),
            TerminalEventWait::Poll(Duration::from_secs(3))
        );
        assert_eq!(
            clock.event_wait(TerminalEventWait::Poll(Duration::from_millis(25)), now),
            TerminalEventWait::Poll(Duration::from_millis(25))
        );
        let late = start + Duration::from_secs(10);
        assert_eq!(
            clock.event_wait(TerminalEventWait::Blocking, late),
            TerminalEventWait::Poll(Duration::ZERO)
        );

        let problem = state_with(StatusKind::Error, "Failed");
        let mut sticky = StatusClock::default();
        sticky.observe(&problem, start);
        assert_eq!(
            sticky.event_wait(TerminalEventWait::Blocking, now),
            TerminalEventWait::Blocking
        );
    }
}
