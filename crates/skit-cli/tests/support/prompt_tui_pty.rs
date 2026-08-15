use std::{
    io::{Read as _, Write as _},
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem as _};

pub struct TuiPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
}

impl TuiPty {
    pub fn spawn(data: &Path, state: &Path, config: &Path, home: &Path) -> Self {
        let pair = NativePtySystem::default()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open PTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_skit"));
        command.arg("tui");
        command.cwd(home);
        command.env("SKIT_DATA_DIR", data);
        command.env("SKIT_STATE_DIR", state);
        command.env("SKIT_CONFIG_DIR", config);
        command.env("SKIT_LANG", "en");
        command.env("HOME", home);
        command.env("USERPROFILE", home);
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        let child = pair.slave.spawn_command(command).expect("spawn skit tui");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let writer = pair.master.take_writer().expect("take PTY writer");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if tx.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            child,
            writer,
            chunks: rx,
            output: Vec::new(),
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write PTY input");
        self.writer.flush().expect("flush PTY input");
    }

    pub fn wait_for(&mut self, needle: &str) -> String {
        self.wait_for_timeout(needle, Duration::from_secs(4))
    }

    pub fn wait_for_timeout(&mut self, needle: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let visible = self.visible();
            if visible.contains(needle) {
                return visible;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!("timed out waiting for {needle:?}; terminal output:\n{visible}")
            };
            match self.chunks.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.child.try_wait().expect("poll TUI child").is_some() {
                        let visible = self.visible();
                        panic!("TUI exited while waiting for {needle:?}; terminal output:\n{visible}");
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let visible = self.visible();
                    panic!("PTY reader closed while waiting for {needle:?}; terminal output:\n{visible}");
                }
            }
        }
    }

    pub fn visible(&mut self) -> String {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.output.extend_from_slice(&chunk);
        }
        strip_terminal_control(&String::from_utf8_lossy(&self.output))
    }
}

impl Drop for TuiPty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn strip_terminal_control(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            if bytes[index] != b'\r' {
                output.push(bytes[index]);
            }
            index += 1;
            continue;
        }
        index += 1;
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'[' => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
mod tests {
    use super::strip_terminal_control;

    #[test]
    fn rust_additive_prompt_tui_pty_strips_control_sequences_without_losing_text() {
        assert_eq!(
            strip_terminal_control("\u{1b}[2Jhello\r\n\u{1b}]0;title\u{7}world\u{1b}[0m"),
            "hello\nworld"
        );
    }
}
