use std::{
    io::{Read as _, Write as _},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_i18n::{Locale, Localize, Message};
use skit_tui::collect_form;
use skit_ui::{Action, Effect, FormField, FormPurpose, FormView};

#[derive(Debug)]
struct HostError;

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
fn generic_form_outer_terminal_lifecycle_uses_a_real_pty() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 20,
            cols: 72,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args(["--ignored", "--exact", "collect_form_child", "--nocapture"]);
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
            .windows(b"Name".len())
            .any(|window| window == b"Name")
        {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before rendering the generic form: {}",
            String::from_utf8_lossy(&output)
        );
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
