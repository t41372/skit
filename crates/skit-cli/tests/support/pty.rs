//! One pseudo-terminal harness for every test that talks to a live child.
//!
//! Each rule below was learned from a real CI failure and carries its wave evidence. A local
//! harness that repeats this plumbing repeats the lessons one file at a time; this module states
//! them once.
//!
//! 1. **Enter is a carriage return.** Prompts read keys through the `console` crate, and only a
//!    carriage return becomes Enter on Windows; a line feed arrives as an ordinary character and
//!    both sides wait (`console/src/windows_term/mod.rs:449`; Unix reads either,
//!    `unix_term.rs:323`). Every write goes through [`keystrokes`] (wave 9, `8016c43`).
//! 2. **Type only after the prompt that owns the answer is visible.** Prompt text is the one
//!    signal both hosts honor: a terminal line buffer holds an early answer, but a console reads
//!    input one record at a time and never replays what arrived before the prompt
//!    (`windows_term/mod.rs:531-560`). Clock- and silence-paced writes were both refuted on the
//!    real host (waves 9-11, `444bbd9`/`4c2609c`, gated in `fa8464b`); the prompt-synchronized
//!    harnesses pass (wave 14, `30d9ec7`).
//! 3. **Answer cursor questions counted, whenever reading.** Unix crossterm asks `\x1b[6n` and
//!    waits; Windows asks the console API and never sends the escape
//!    (`crossterm/src/cursor/sys/{unix,windows}.rs`). Count the questions in the stream and
//!    answer each exactly once — a one-shot flag cannot answer a second question (waves 4 and 14).
//! 4. **End on the child, never on the terminal saying its output is over.** A pseudo-console
//!    does not reliably deliver the reader's end-of-input, so a teardown that joins the drain
//!    thread can never return; wait for the child under a deadline, then read what is buffered
//!    for a bounded moment (waves 12-13, `330ff09`/`d8737f1`). The same rule governs a wait: the
//!    child's state decides, and a closed reader is only a reason to stop expecting more bytes.
//!    The master is held for the whole session and released at teardown, after the child has
//!    exited. The pseudo-console lives in the master, and the reader and the writer are plain
//!    pipe handles that do not keep it alive (`portable-pty-0.9.0/src/win/conpty.rs`), so
//!    dropping the master at spawn closes the console under a live child
//!    (`PsuedoCon::drop` calls `ClosePseudoConsole`, `win/psuedocon.rs:73-77`) and the reader
//!    then reports an end of input the child never sent.
//! 5. **The reader thread fills a channel and is never joined on end-of-input.** The drain is
//!    detached; nothing downstream depends on it finishing.
//! 6. **Read the visible text, never the control stream.** A pseudo-console writes its own
//!    sequences into the same stream as the child's output: Windows opens a session with a
//!    cursor question, mode switches, and a window title carrying the whole binary path
//!    (`\x1b]0;D:\a\...\skit.exe\x07`). An assertion that measures or matches raw bytes reads
//!    that chrome as if the product had printed it. [`strip_terminal_control`] gives the text a
//!    person sees (wave 4 fold owners).
//!
//! Two harness families share these rules. [`PtyChild`] is the full channel-driven harness.
//! The free functions ([`keystrokes`], [`settle_buffer`], [`wait_for_exit`]) serve the bespoke
//! harnesses that keep their own capture buffers.

// Two test binaries include this file and each uses its own subset of the API, so the unused
// remainder in either binary is expected (the `plain_add_pty` support precedent).
#![allow(dead_code)]

use std::{
    io::{Read as _, Write as _},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, ExitStatus, PtySize, native_pty_system};

/// One byte string a terminal program writes when it asks where the cursor is.
pub(crate) const CURSOR_QUERY: &[u8] = b"\x1b[6n";
/// The answer a real terminal gives: the cursor sits at the top-left corner.
pub(crate) const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// How long a prompt or a cursor question may take to appear on a loaded CI host.
const WAIT_BUDGET: Duration = Duration::from_secs(30);
/// How long a finished exchange may take to end in the child's exit.
const EXIT_BUDGET: Duration = Duration::from_secs(60);
/// How long a settle may watch for quiet before it gives up.
const SETTLE_BOUND: Duration = Duration::from_secs(5);
/// The default quiet window a settle waits for.
const SETTLE_QUIET: Duration = Duration::from_millis(60);
/// How long a poll rests when the terminal has stopped delivering but the child still runs.
const POLL_PAUSE: Duration = Duration::from_millis(10);

/// Whether the harness answers cursor questions by itself while it reads.
///
/// `On` is the rule (invariant 3). `Off` serves the two lanes that script their own reply as an
/// input byte: answering there as well would feed the reply's bytes to the child twice, and the
/// second copy arrives as ordinary keys.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnswerQueries {
    On,
    Off,
}

/// A live child on a pseudo-terminal, with the platform rules applied in one place.
pub(crate) struct PtyChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// The terminal itself, held until teardown so the console outlives the child (invariant 4).
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
    /// How many cursor questions this terminal has already answered.
    answered_queries: usize,
    /// The most recent keys written, for a timeout message.
    last_sent: Vec<u8>,
    /// How far [`Self::expect`] has consumed the output.
    consumed: usize,
    answer: AnswerQueries,
}

impl PtyChild {
    /// Start the child on a fresh pseudo-terminal.
    ///
    /// The caller owns the command (program, arguments, environment, working directory); the
    /// harness owns the terminal. The master is kept until teardown: it owns the console the
    /// child is attached to (invariant 4).
    pub(crate) fn spawn(command: CommandBuilder, size: PtySize, answer: AnswerQueries) -> Self {
        let pair = native_pty_system().openpty(size).unwrap();
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let master = pair.master;
        let mut reader = master.try_clone_reader().unwrap();
        let writer = master.take_writer().unwrap();
        let (sender, chunks) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
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
        Self {
            child,
            master,
            writer,
            chunks,
            output: Vec::new(),
            answered_queries: 0,
            last_sent: Vec::new(),
            consumed: 0,
            answer,
        }
    }

    /// Pull everything already buffered, answering any new cursor questions.
    fn drain(&mut self) {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.output.extend_from_slice(&chunk);
        }
        self.answer_new_cursor_queries();
    }

    /// Answer every cursor question this terminal has not answered yet (invariant 3).
    fn answer_new_cursor_queries(&mut self) {
        if self.answer == AnswerQueries::Off {
            return;
        }
        let asked = self
            .output
            .windows(CURSOR_QUERY.len())
            .filter(|window| *window == CURSOR_QUERY)
            .count();
        while self.answered_queries < asked {
            // The write may fail after the child has exited; a late answer must not panic
            // (invariant 4).
            let _ = self.writer.write_all(CURSOR_REPLY);
            let _ = self.writer.flush();
            self.answered_queries = self.answered_queries.saturating_add(1);
        }
    }

    /// Where the output stands now, as a byte offset for a later `wait_for_after`.
    pub(crate) fn checkpoint(&mut self) -> usize {
        self.drain();
        self.output.len()
    }

    /// The raw bytes read after `checkpoint`.
    pub(crate) fn raw_after(&mut self, checkpoint: usize) -> Vec<u8> {
        self.drain();
        self.output[checkpoint.min(self.output.len())..].to_vec()
    }

    /// Everything read so far, lossily decoded.
    pub(crate) fn visible(&mut self) -> String {
        self.drain();
        String::from_utf8_lossy(&self.output).into_owned()
    }

    /// Wait until `render` of the output after `checkpoint` contains `needle` (invariant 2).
    ///
    /// The render turns bytes into the text a needle can match — identity for line output,
    /// a control-stripping view for a full-screen interface. Returns the rendered text. The
    /// timeout panic names the child's state and the last keys written, so a report reads as a
    /// diagnosis.
    pub(crate) fn wait_for_after_rendered(
        &mut self,
        checkpoint: usize,
        needle: &str,
        render: impl Fn(&[u8]) -> String,
    ) -> String {
        let deadline = Instant::now() + WAIT_BUDGET;
        loop {
            self.drain();
            let visible = render(&self.output[checkpoint.min(self.output.len())..]);
            if visible.contains(needle) {
                return visible;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                let state = self.child_state();
                let sent = String::from_utf8_lossy(&self.last_sent).into_owned();
                let answered = self.answered_queries;
                let total = self.output.len();
                let raw =
                    String::from_utf8_lossy(&self.output[checkpoint.min(total)..]).into_owned();
                panic!(
                    "timed out waiting for {needle:?} after checkpoint {checkpoint}; {state}; \
                     last keys written: {sent:?}; \
                     cursor questions answered: {answered}; total bytes read: {total}; \
                     new bytes: {}; new terminal output:\n{visible}\nraw bytes since the checkpoint:\n{raw:?}",
                    total.saturating_sub(checkpoint)
                );
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "child exited while waiting for {needle:?}; new terminal output:\n{visible}"
                    );
                }
                // The terminal saying its output is over ends the reading, not the exchange
                // (invariant 4). The child decides: report only once it has exited, so the
                // message names the cause instead of the messenger.
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "child exited while waiting for {needle:?}; new terminal output:\n{visible}"
                    );
                    thread::sleep(POLL_PAUSE);
                }
            }
        }
    }

    /// Wait until the raw output after `checkpoint` contains `needle`.
    pub(crate) fn wait_for_after(&mut self, checkpoint: usize, needle: &str) -> String {
        self.wait_for_after_rendered(checkpoint, needle, |bytes| {
            String::from_utf8_lossy(bytes).into_owned()
        })
    }

    /// Wait for the next `prompt` this exchange has not consumed yet, and step past it.
    ///
    /// This is the line-prompt protocol: each answer waits for the prompt that owns it, so
    /// reprompts and end-of-input behavior stay deterministic without guessing how long the child
    /// takes (invariant 2).
    pub(crate) fn expect(&mut self, prompt: &str) {
        let shown = self.wait_for_after(self.consumed, prompt);
        let position = shown
            .find(prompt)
            .expect("wait_for_after returned text without the needle");
        // The rendered view is the raw lossy decode here, so byte lengths line up with the raw
        // buffer as long as the needle itself decoded cleanly.
        self.consumed += shown[..position].len() + prompt.len();
    }

    /// Type `answer` the way a terminal types it (invariant 1). A write to an exited child panics.
    pub(crate) fn send(&mut self, answer: &[u8]) {
        self.last_sent = answer.to_vec();
        self.writer.write_all(&keystrokes(answer)).unwrap();
        self.writer.flush().unwrap();
    }

    /// Type `answer`, reporting rather than panicking when the child is already gone.
    pub(crate) fn try_send(&mut self, answer: &[u8]) -> bool {
        self.last_sent = answer.to_vec();
        if self.writer.write_all(&keystrokes(answer)).is_err() {
            return false;
        }
        let _ = self.writer.flush();
        true
    }

    /// Write raw bytes with no translation — a scripted control reply, not a keystroke.
    pub(crate) fn write_raw(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Wait for `prompt`, then type `answer` (invariants 1 and 2 together).
    pub(crate) fn send_after_prompt(&mut self, prompt: &str, answer: &[u8]) {
        self.expect(prompt);
        if !answer.is_empty() {
            self.send(answer);
        }
    }

    /// Wait until the child has written nothing for the default quiet window.
    pub(crate) fn settle(&mut self) {
        self.settle_quiet(SETTLE_QUIET);
    }

    /// Wait until the child has written nothing for `quiet`, bounded so a chatty child cannot
    /// hold the test.
    pub(crate) fn settle_quiet(&mut self, quiet: Duration) {
        let deadline = Instant::now() + SETTLE_BOUND;
        while Instant::now() < deadline {
            match self.chunks.recv_timeout(quiet) {
                Ok(chunk) => {
                    self.output.extend_from_slice(&chunk);
                    self.answer_new_cursor_queries();
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    return;
                }
            }
        }
    }

    /// Wait until the child asks where the cursor is after `checkpoint`. Draining has already
    /// answered it.
    pub(crate) fn wait_cursor_query_after(&mut self, checkpoint: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.drain();
            if self.output[checkpoint.min(self.output.len())..]
                .windows(CURSOR_QUERY.len())
                .any(|window| window == CURSOR_QUERY)
            {
                return;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!("timed out waiting for the terminal cursor-position query");
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "child exited before it requested the cursor position"
                    );
                }
                // The child decides here too (invariant 4).
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "child exited before it requested the cursor position"
                    );
                    thread::sleep(POLL_PAUSE);
                }
            }
        }
    }

    /// Wait for the child to exit within `budget`, reading whatever arrives meanwhile
    /// (invariant 4).
    pub(crate) fn wait_exit_within(&mut self, budget: Duration) -> ExitStatus {
        let deadline = Instant::now() + budget;
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!(
                    "the terminal child never exited; output:\n{}",
                    String::from_utf8_lossy(&self.output)
                );
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                // A closed reader returns at once, so rest before asking the child again.
                Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(POLL_PAUSE),
            }
        }
    }

    /// End the child now. For a wrapper's `Drop`, so an assert that fails mid-session does not
    /// leave a live process behind.
    pub(crate) fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// What the child is doing, for a timeout message.
    pub(crate) fn child_state(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!("child exited with {}", status.exit_code()),
            Ok(None) => "child still running".to_owned(),
            Err(error) => format!("child status unreadable: {error}"),
        }
    }

    /// Whether the child has exited, and with what code.
    pub(crate) fn try_exit_code(&mut self) -> Option<u32> {
        self.try_wait_status().map(|status| status.exit_code())
    }

    /// Whether the child has exited, with the complete status for a caller that reads signals.
    pub(crate) fn try_wait_status(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().unwrap()
    }

    /// End the exchange on the child's exit, then read what is still buffered for a bounded
    /// moment (invariant 4). Returns the exit code and everything the terminal showed.
    pub(crate) fn finish(mut self) -> (u32, String) {
        let status = self.wait_exit_within(EXIT_BUDGET);
        // Release the terminal only now, with the child already gone: the console lives here, and
        // closing it under a live child ends the reading early (invariant 4).
        drop(self.writer);
        drop(self.master);
        let deadline = Instant::now() + SETTLE_BOUND;
        while Instant::now() < deadline {
            match self.chunks.recv_timeout(SETTLE_QUIET) {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
        (
            status.exit_code(),
            String::from_utf8_lossy(&self.output).into_owned(),
        )
    }
}

/// Deliver one canned answer the way a terminal delivers it (invariant 1).
pub(crate) fn keystrokes(answer: &[u8]) -> Vec<u8> {
    answer
        .iter()
        .map(|byte| if *byte == b'\n' { b'\r' } else { *byte })
        .collect()
}

/// Read whatever a bespoke capture buffer still receives, for a bounded moment.
///
/// This is the settle for harnesses that keep their own `Arc<Mutex<Vec<u8>>>` capture instead of
/// [`PtyChild`]'s channel.
pub(crate) fn settle_buffer(captured: &Arc<Mutex<Vec<u8>>>) {
    let deadline = Instant::now() + SETTLE_BOUND;
    let mut seen = captured.lock().unwrap().len();
    let mut quiet_since = Instant::now();
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        let now = captured.lock().unwrap().len();
        if now == seen {
            if quiet_since.elapsed() >= SETTLE_QUIET {
                return;
            }
        } else {
            seen = now;
            quiet_since = Instant::now();
        }
    }
}

/// Wait for a bespoke harness's child to exit, under the shared deadline (invariant 4).
pub(crate) fn wait_for_exit(child: &mut Box<dyn portable_pty::Child + Send + Sync>) -> ExitStatus {
    let deadline = Instant::now() + EXIT_BUDGET;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "the terminal child never exited");
        thread::sleep(Duration::from_millis(10));
    }
}

/// The text a person sees, with the terminal's own sequences removed (invariant 6).
///
/// Drops CSI sequences (`ESC [` up to a final byte), OSC sequences (`ESC ]` up to `BEL` or
/// `ESC \`), any other single escape, and the carriage returns a terminal uses to return to the
/// left margin. What remains is the child's printable output, which an assertion can measure or
/// match.
pub(crate) fn strip_terminal_control(input: &str) -> String {
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
