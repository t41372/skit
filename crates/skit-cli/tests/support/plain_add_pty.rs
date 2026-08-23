#![cfg(unix)]

use std::{
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

pub(crate) struct PlainAddPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    captured: Arc<(Mutex<Vec<u8>>, Condvar)>,
    drain: thread::JoinHandle<()>,
}

#[allow(dead_code)]
impl PlainAddPty {
    pub(crate) fn spawn(
        data: &Path,
        state: &Path,
        config: &Path,
        cwd: &Path,
        locale: &str,
        args: &[&str],
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 160,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", data);
        command.env("SKIT_STATE_DIR", state);
        command.env("SKIT_CONFIG_DIR", config);
        command.env("SKIT_LANG", locale);
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let captured = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let reader_capture = Arc::clone(&captured);
        let drain = thread::spawn(move || {
            let mut chunk = [0_u8; 1024];
            while let Ok(read) = reader.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                let (bytes, changed) = &*reader_capture;
                bytes.lock().unwrap().extend_from_slice(&chunk[..read]);
                changed.notify_all();
            }
        });
        Self {
            child,
            writer,
            captured,
            drain,
        }
    }

    pub(crate) fn wait_for(&self, needle: &str) {
        self.wait_for_after(needle, 0);
    }

    pub(crate) fn checkpoint(&self) -> usize {
        self.captured.0.lock().unwrap().len()
    }

    pub(crate) fn wait_for_after(&self, needle: &str, checkpoint: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (bytes, changed) = &*self.captured;
        let mut bytes = bytes.lock().unwrap();
        loop {
            let rendered = String::from_utf8_lossy(&bytes[checkpoint.min(bytes.len())..]);
            if rendered.contains(needle) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for {needle:?}:\n{rendered}"
            );
            let (next, timeout) = changed.wait_timeout(bytes, remaining).unwrap();
            bytes = next;
            assert!(!timeout.timed_out(), "timed out waiting for {needle:?}");
        }
    }

    pub(crate) fn send_line(&mut self, answer: &str) {
        // Translate any embedded line feed too, so an answer that carries one still reads as
        // Enter on a host where only a carriage return does
        // (`console/src/windows_term/mod.rs:449`). The terminator below is already one.
        self.writer
            .write_all(&keystrokes(answer.as_bytes()))
            .unwrap();
        self.writer.write_all(b"\r").unwrap();
        self.writer.flush().unwrap();
    }

    pub(crate) fn interrupt(&mut self) {
        self.writer.write_all(&[3]).unwrap();
        self.writer.flush().unwrap();
    }

    pub(crate) fn finish(mut self) -> (u32, String) {
        let status = self.child.wait().unwrap();
        drop(self.writer);
        self.drain.join().unwrap();
        let bytes = self.captured.0.lock().unwrap();
        let output = String::from_utf8_lossy(&bytes)
            .replace("\r\n", "\n")
            .replace('\r', "");
        (status.exit_code(), output)
    }
}

/// Deliver the typed part of one answer the way a terminal delivers it.
///
/// Unix reads a line feed and a carriage return alike as Enter
/// (`console/src/unix_term.rs:323`), so this changes nothing here and keeps one convention with the
/// other terminal harnesses.
fn keystrokes(answer: &[u8]) -> Vec<u8> {
    answer
        .iter()
        .map(|byte| if *byte == b'\n' { b'\r' } else { *byte })
        .collect()
}
