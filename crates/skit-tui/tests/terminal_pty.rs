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
use skit_tui::{collect_form, collect_run_form, run, run_preflighted, run_with_path_completion};
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
        other => panic!("unknown wrapper mode {other}"),
    }
}

fn harmless_host(_effect: Effect) -> Result<Action, HostError> {
    Ok(Action::ClearStatus)
}

#[test]
fn generic_form_outer_terminal_lifecycle_uses_a_real_pty() {
    run_child_in_pty("collect_form_child", None, "Name", None);
}

#[test]
fn every_public_terminal_wrapper_owns_a_real_terminal_lifecycle() {
    for (mode, marker) in [
        ("run", "Library"),
        ("run-with-path", "Library"),
        ("collect-run", "Extra arguments"),
    ] {
        run_child_in_pty("public_terminal_wrapper_child", Some(mode), marker, None);
    }
    let marker = tempfile::NamedTempFile::new().unwrap();
    run_child_in_pty(
        "public_terminal_wrapper_child",
        Some("preflight-refuse"),
        "Library",
        Some((b"\x12", marker.path())),
    );
    assert_eq!(std::fs::read_to_string(marker.path()).unwrap(), "preflight");
}

fn run_child_in_pty(
    test_name: &str,
    mode: Option<&str>,
    marker: &str,
    after_marker: Option<(&[u8], &std::path::Path)>,
) {
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
    if let Some((_, marker)) = after_marker {
        command.env("SKIT_TUI_PREFLIGHT_MARKER", marker);
    }
    command.env("TERM", "xterm-256color");
    command.env("NO_COLOR", "1");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
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

    let deadline = Instant::now() + Duration::from_secs(6);
    let mut output = Vec::new();
    let mut answered_cursor_query = 0;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("timed out waiting for the generic form");
        let chunk = chunks
            .recv_timeout(remaining.min(Duration::from_millis(100)))
            .expect("PTY output closed before the generic form appeared");
        output.extend_from_slice(&chunk);
        let queries = output
            .windows(b"\x1b[6n".len())
            .filter(|window| *window == b"\x1b[6n")
            .count();
        while answered_cursor_query < queries {
            writer.write_all(b"\x1b[1;1R").unwrap();
            writer.flush().unwrap();
            answered_cursor_query += 1;
        }
        if output
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
        {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before rendering the generic form: {}",
            String::from_utf8_lossy(&output)
        );
    }

    if let Some((input, marker)) = after_marker {
        writer.write_all(input).unwrap();
        writer.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if std::fs::metadata(marker).is_ok_and(|metadata| metadata.len() > 0) {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before the preflight checkpoint: {}",
                String::from_utf8_lossy(&output)
            );
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for the preflight checkpoint");
            match chunks.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(chunk) => output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => thread::yield_now(),
            }
        }
    }

    writer.write_all(b"\x03\x03").unwrap();
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "PTY child failed: {status:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the generic form to exit"
        );
        match chunks.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => output.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
        }
    }
}
