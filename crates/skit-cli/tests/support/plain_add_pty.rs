#![cfg(unix)]

//! The plain-add prompt harness, as a thin shape over the shared terminal rules.
//!
//! The prompt-synchronized discipline this file always had — wait for the prompt text, then
//! answer — is invariant 2 of `support/pty.rs`, the shape every harness that passes on Windows
//! shares. The consumers keep this wrapper's call surface; the plumbing lives in one place.

use std::path::{Path, PathBuf};

use portable_pty::{CommandBuilder, PtySize};

#[path = "pty.rs"]
mod pty;

pub(crate) struct PlainAddPty {
    child: pty::PtyChild,
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
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", data);
        command.env("SKIT_STATE_DIR", state);
        command.env("SKIT_CONFIG_DIR", config);
        command.env("SKIT_LANG", locale);
        let size = PtySize {
            rows: 30,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        };
        Self {
            child: pty::PtyChild::spawn(command, size, pty::AnswerQueries::On),
        }
    }

    pub(crate) fn wait_for(&mut self, needle: &str) {
        self.wait_for_after(needle, 0);
    }

    pub(crate) fn checkpoint(&mut self) -> usize {
        self.child.checkpoint()
    }

    pub(crate) fn wait_for_after(&mut self, needle: &str, checkpoint: usize) {
        self.child.wait_for_after(checkpoint, needle);
    }

    pub(crate) fn send_line(&mut self, answer: &str) {
        self.child.send(answer.as_bytes());
        self.child.send(b"\r");
    }

    pub(crate) fn interrupt(&mut self) {
        self.child.write_raw(&[3]);
    }

    pub(crate) fn finish(self) -> (u32, String) {
        let (code, output) = self.child.finish();
        (code, output.replace("\r\n", "\n").replace('\r', ""))
    }
}
