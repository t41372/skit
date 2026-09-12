use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::path_completion::{PathCompletionProvider, PathCompletionRequest};
use skit_i18n::{Locale, Localize, Message};
use skit_tui::{
    collect_form, collect_run_form, collect_run_form_with_path_completion, run, run_preflighted,
    run_with_path_completion,
};
use skit_ui::{Action, Effect, FormField, FormPurpose, FormView, LibraryState, RunFormView};

#[derive(Debug)]
struct HostError;

#[derive(Debug)]
struct EmptyPathProvider;

impl PathCompletionProvider for EmptyPathProvider {
    fn complete(&self, _request: &PathCompletionRequest) -> Option<String> {
        None
    }
}

impl Localize for HostError {
    fn message(&self) -> Message {
        Message::new("terminal host failed")
    }
}

/// The sign that a host effect owns the terminal.
///
/// The child writes it while the terminal is suspended, so the capture keeps one exact boundary
/// between the suspend and the resume.
const HOST_EFFECT_MARKER: &[u8] = b"HOST EFFECT RUNNING";

#[test]
#[ignore = "runs only as the child of the PTY lifecycle owner"]
fn collect_form_child() {
    let form = FormView {
        purpose: FormPurpose::Settings,
        title: "PTY form".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: vec![FormField::text("name", "Name", "Ada")],
        focused: 0,
        submit_label: "Save".to_owned(),
    };
    let result = collect_form(
        form,
        |_effect: Effect| -> Result<Action, HostError> { Ok(Action::ClearStatus) },
        Locale::En,
    )
    .unwrap();
    assert_eq!(result, None);
}

#[test]
#[ignore = "runs only as the child of the public-wrapper PTY owner"]
fn public_terminal_wrapper_child() {
    let mode = std::env::var("SKIT_TUI_WRAPPER").expect("the PTY owner sets one wrapper");
    match mode.as_str() {
        "run" => run(LibraryState::default(), harmless_host, Locale::En).unwrap(),
        "run-with-path" => run_with_path_completion(
            LibraryState::default(),
            harmless_host,
            Locale::En,
            Arc::new(EmptyPathProvider),
        )
        .unwrap(),
        "preflight-refuse" => {
            let marker = std::env::var("SKIT_TUI_PREFLIGHT_MARKER").unwrap();
            let preflight_marker = marker.clone();
            run_preflighted(
                LibraryState::default(),
                move |_effect| {
                    std::fs::write(&preflight_marker, "preflight").unwrap();
                    Err(HostError)
                },
                move |_effect| -> Result<Action, HostError> {
                    std::fs::write(&marker, "host").unwrap();
                    Ok(Action::ClearStatus)
                },
                Locale::En,
            )
            .unwrap();
        }
        "suspend-modes" => run(
            LibraryState::default(),
            |_effect: Effect| -> Result<Action, HostError> {
                let mut out = std::io::stdout();
                out.write_all(HOST_EFFECT_MARKER).unwrap();
                out.write_all(b"\n").unwrap();
                out.flush().unwrap();
                Ok(Action::ClearStatus)
            },
            Locale::En,
        )
        .unwrap(),
        "collect-run" => {
            let form = RunFormView::from_declarations(
                "demo",
                "Wrapper run",
                &[],
                &BTreeMap::new(),
                &[],
                "",
                &BTreeMap::new(),
                "",
            );
            assert_eq!(
                collect_run_form(form, harmless_host, Locale::En).unwrap(),
                None
            );
        }
        "collect-run-with-path" => {
            let form = RunFormView::from_declarations(
                "demo",
                "Wrapper run with path completion",
                &[],
                &BTreeMap::new(),
                &[],
                "",
                &BTreeMap::new(),
                "",
            );
            assert_eq!(
                collect_run_form_with_path_completion(
                    form,
                    harmless_host,
                    Locale::En,
                    Arc::new(EmptyPathProvider),
                )
                .unwrap(),
                None
            );
        }
        other => panic!("unknown wrapper mode {other}"),
    }
}

fn harmless_host(_effect: Effect) -> Result<Action, HostError> {
    Ok(Action::ClearStatus)
}

#[test]
fn generic_form_outer_terminal_lifecycle_uses_a_real_pty() {
    run_child_in_pty("collect_form_child", None, "Name", &[]);
}

#[test]
fn every_public_terminal_wrapper_owns_a_real_terminal_lifecycle() {
    for (mode, marker) in [
        ("run", "Library"),
        ("run-with-path", "Library"),
        ("collect-run", "Extra arguments"),
        ("collect-run-with-path", "Extra arguments"),
    ] {
        run_child_in_pty("public_terminal_wrapper_child", Some(mode), marker, &[]);
    }
    let marker = tempfile::NamedTempFile::new().unwrap();
    run_child_in_pty(
        "public_terminal_wrapper_child",
        Some("preflight-refuse"),
        "Library",
        &[Exchange {
            input: b"\x12",
            wait: Wait::File(marker.path()),
        }],
    );
    assert_eq!(std::fs::read_to_string(marker.path()).unwrap(), "preflight");
}

/// A host effect must leave and re-enter every screen mode.
///
/// Mouse capture and focus reporting are not part of the alternate screen. If the suspend keeps
/// focus reporting on, the process that owns the terminal receives `ESC [ I` and `ESC [ O` as input
/// at every window change.
///
/// ConPTY re-renders its own screen and does not pass every control sequence through the master, so
/// this byte contract is a Unix contract.
#[cfg(unix)]
#[test]
fn a_host_effect_leaves_and_re_enters_every_screen_mode() {
    let output = run_child_in_pty(
        "public_terminal_wrapper_child",
        Some("suspend-modes"),
        "Library",
        &[
            // Ctrl+R on the library is Reload, the same key the refused preflight above uses to
            // reach the host boundary.
            Exchange {
                input: b"\x12",
                wait: Wait::Output(HOST_EFFECT_MARKER),
            },
            // Raw mode is off while the host effect runs, so Ctrl+C would be a signal. Wait for the
            // resumed library frame before the harness sends it. The resume clears the terminal,
            // and that clear asks for the cursor position; only a wait answers the question, so
            // the frame that follows the answer is the sign that the resume is complete.
            Exchange {
                input: b"",
                wait: Wait::Output(b"Library"),
            },
        ],
    );

    let library_index = find_bytes(&output, b"Library").expect("the library never rendered");
    let marker_index =
        find_bytes(&output, HOST_EFFECT_MARKER).expect("the host effect never reported itself");
    let suspended = &output[library_index..marker_index];
    for disabled in [
        b"\x1b[?1049l".as_slice(),
        b"\x1b[?1003l".as_slice(),
        b"\x1b[?1004l".as_slice(),
    ] {
        assert!(
            find_bytes(suspended, disabled).is_some(),
            "the terminal did not write {} before the host effect: {}",
            String::from_utf8_lossy(disabled),
            String::from_utf8_lossy(suspended)
        );
    }

    let resumed = &output[marker_index..];
    for enabled in [
        b"\x1b[?1049h".as_slice(),
        b"\x1b[?1003h".as_slice(),
        b"\x1b[?1004h".as_slice(),
    ] {
        assert!(
            find_bytes(resumed, enabled).is_some(),
            "the terminal did not write {} after the host effect: {}",
            String::from_utf8_lossy(enabled),
            String::from_utf8_lossy(resumed)
        );
    }
}

/// One exchange after the first marker: keys to send, then the sign to wait for.
struct Exchange<'a> {
    input: &'a [u8],
    wait: Wait<'a>,
}

enum Wait<'a> {
    /// The child writes this file.
    File(&'a std::path::Path),
    /// The child writes these bytes to the terminal, after every earlier wait.
    #[cfg(unix)]
    Output(&'a [u8]),
}

/// Report where the bytes occur in the capture, if they occur.
fn find_bytes(capture: &[u8], wanted: &[u8]) -> Option<usize> {
    capture
        .windows(wanted.len())
        .position(|window| window == wanted)
}

/// The terminal side of one PTY child: its capture, its answers, and how far the waits have read.
struct PtyWatch<W>
where
    W: std::io::Write,
{
    chunks: mpsc::Receiver<Vec<u8>>,
    writer: W,
    output: Vec<u8>,
    read_through: usize,
    answered_cursor_query: usize,
}

impl<W> PtyWatch<W>
where
    W: std::io::Write,
{
    /// Take the next chunk of terminal output, and answer every cursor query it completes.
    ///
    /// Every wait uses this one step. A wait that does not answer a query leaves the child waiting
    /// for an answer that never arrives.
    fn receive(&mut self, remaining: Duration) -> Result<(), mpsc::RecvTimeoutError> {
        let chunk = self
            .chunks
            .recv_timeout(remaining.min(Duration::from_millis(100)))?;
        self.output.extend_from_slice(&chunk);
        let queries = self
            .output
            .windows(b"\x1b[6n".len())
            .filter(|window| *window == b"\x1b[6n")
            .count();
        while self.answered_cursor_query < queries {
            self.writer.write_all(b"\x1b[1;1R").unwrap();
            self.writer.flush().unwrap();
            self.answered_cursor_query += 1;
        }
        Ok(())
    }

    /// Report whether the bytes arrived after every earlier wait, and read through them.
    ///
    /// A resume can share one chunk with the marker before it, so the read position moves to the
    /// end of the match and not to the end of the capture.
    fn arrived(&mut self, wanted: &[u8]) -> bool {
        let Some(found) = find_bytes(&self.output[self.read_through..], wanted) else {
            return false;
        };
        self.read_through = self.read_through + found + wanted.len();
        true
    }
}

fn run_child_in_pty(
    test_name: &str,
    mode: Option<&str>,
    marker: &str,
    exchanges: &[Exchange<'_>],
) -> Vec<u8> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 20,
            cols: 72,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args(["--ignored", "--exact", test_name, "--nocapture"]);
    if let Some(mode) = mode {
        command.env("SKIT_TUI_WRAPPER", mode);
    }
    for exchange in exchanges {
        // A `match` names every arm. On a host without `Wait::Output` an `if let` on this enum is
        // irrefutable, and the lint for that is an error under the quality gate.
        match exchange.wait {
            Wait::File(marker) => {
                command.env("SKIT_TUI_PREFLIGHT_MARKER", marker);
            }
            #[cfg(unix)]
            Wait::Output(_) => {}
        }
    }
    command.env("TERM", "xterm-256color");
    command.env("NO_COLOR", "1");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = pair.master.take_writer().unwrap();
    let (sender, chunks) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut watch = PtyWatch {
        chunks,
        writer,
        output: Vec::new(),
        read_through: 0,
        answered_cursor_query: 0,
    };

    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("timed out waiting for the generic form");
        watch
            .receive(remaining)
            .expect("PTY output closed before the generic form appeared");
        if watch.arrived(marker.as_bytes()) {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before rendering the generic form: {}",
            String::from_utf8_lossy(&watch.output)
        );
    }

    for exchange in exchanges {
        watch.writer.write_all(&keystrokes(exchange.input)).unwrap();
        watch.writer.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let arrived = match exchange.wait {
                Wait::File(marker) => {
                    std::fs::metadata(marker).is_ok_and(|metadata| metadata.len() > 0)
                }
                #[cfg(unix)]
                Wait::Output(wanted) => watch.arrived(wanted),
            };
            if arrived {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before the next checkpoint: {}",
                String::from_utf8_lossy(&watch.output)
            );
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for the next checkpoint");
            match watch.receive(remaining) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => thread::yield_now(),
            }
        }
    }

    watch.writer.write_all(b"\x03\x03").unwrap();
    watch.writer.flush().unwrap();
    // An instrumented child writes its coverage profile as it exits, and parallel load makes that
    // take more than six seconds. This deadline only stops a hung child from holding the suite.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "PTY child failed: {status:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the generic form to exit"
        );
        // The child exits, so this only keeps the capture complete. It does not answer a cursor
        // query: every wait above answered the queries of the frames it waited for.
        match watch.chunks.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => watch.output.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
        }
    }
    watch.output
}

// This crate deliberately keeps its own compliant harness instead of sharing
// `skit-cli/tests/support/pty.rs`: a cross-crate share needs a workspace member, and a new
// member enters the release-guarded sdist census and Cargo lock — not worth churning for this
// much plumbing. The canonical rules and their wave evidence live in that module; this harness
// already follows all five (exit-keyed waits, counted answers, translated Enter).
/// Deliver one canned answer the way a terminal delivers it.
///
/// A terminal sends Enter as a carriage return. Prompts read keys through the `console` crate, and
/// there only a carriage return becomes Enter on Windows: a line feed arrives as an ordinary
/// character, so the prompt keeps waiting and both sides stop
/// (`console/src/windows_term/mod.rs:449`). Unix reads either one as Enter
/// (`console/src/unix_term.rs:323`), so translating here gives both hosts one convention and leaves
/// Unix exactly as it was.
fn keystrokes(answer: &[u8]) -> Vec<u8> {
    answer
        .iter()
        .map(|byte| if *byte == b'\n' { b'\r' } else { *byte })
        .collect()
}
