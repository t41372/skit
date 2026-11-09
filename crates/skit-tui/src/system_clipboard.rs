//! The system clipboard of a local session.
//!
//! A copy in a text box always goes to the terminal as OSC 52, which is the only route that works
//! over SSH. Some terminals ignore OSC 52 or block it by default: Terminal.app ignores it, and
//! iTerm2 and tmux block it. A local session therefore also gives each copy to the clipboard
//! program of the platform when one is there: `pbcopy` on macOS, and on the other Unix systems
//! `wl-copy` under Wayland, then `xclip` or `xsel` under X11. Windows Terminal takes OSC 52, so
//! Windows uses no program.
//!
//! A copy is something the person asks for, not a change of a setting, so it does not change any
//! configuration. The program runs on a thread of its own, so the interface never waits for it,
//! and a missing program is not an error.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The longest time the end of a session waits for the copy that is still running.
const EXIT_WAIT: Duration = Duration::from_secs(1);

/// The platform families that choose a clipboard program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Platform {
    MacOs,
    Windows,
    /// Linux and the other Unix systems.
    Unix,
}

impl Platform {
    /// The platform of this build.
    pub(crate) const fn current() -> Self {
        Self::of(cfg!(target_os = "macos"), cfg!(windows))
    }

    const fn of(macos: bool, windows: bool) -> Self {
        if macos {
            Self::MacOs
        } else if windows {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// One program that puts its standard input on the system clipboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CopyProgram {
    program: OsString,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
}

impl CopyProgram {
    pub(crate) fn new<P, A>(program: P, arguments: A) -> Self
    where
        P: Into<OsString>,
        A: IntoIterator,
        A::Item: Into<OsString>,
    {
        Self {
            program: program.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
            environment: Vec::new(),
        }
    }

    /// Give the program one more environment value.
    pub(crate) fn with_environment(
        mut self,
        name: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    /// Run the program with `text` as its standard input, and wait for it to end.
    fn run(&self, text: &str) -> io::Result<ExitStatus> {
        let mut child = Command::new(&self.program)
            .args(&self.arguments)
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // Dropping the pipe after the write closes it, so the program reads to its end.
        let written = child
            .stdin
            .take()
            .map_or(Ok(()), |mut stdin| stdin.write_all(text.as_bytes()));
        let status = child.wait()?;
        written.map(|()| status)
    }
}

/// The clipboard programs to try in order, for one platform and one environment.
///
/// `set` reports whether an environment variable has a value. An SSH session gets no program:
/// the clipboard of this machine is not the clipboard of the person, and OSC 52 reaches the
/// terminal on the other end.
pub(crate) fn copy_programs(platform: Platform, set: impl Fn(&str) -> bool) -> Vec<CopyProgram> {
    if ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .into_iter()
        .any(&set)
    {
        return Vec::new();
    }
    let mut programs = Vec::new();
    match platform {
        // pbcopy reads its input in the encoding of the locale, and a terminal can start skit
        // with no UTF-8 locale. UTF-8 here keeps every character of the copy.
        Platform::MacOs => {
            programs.push(
                CopyProgram::new("pbcopy", [] as [&str; 0]).with_environment("LC_CTYPE", "UTF-8"),
            );
        }
        Platform::Windows => {}
        Platform::Unix => {
            if set("WAYLAND_DISPLAY") {
                programs.push(CopyProgram::new("wl-copy", [] as [&str; 0]));
            }
            if set("DISPLAY") {
                programs.push(CopyProgram::new("xclip", ["-selection", "clipboard"]));
                programs.push(CopyProgram::new("xsel", ["--clipboard", "--input"]));
            }
        }
    }
    programs
}

/// Give `text` to the first program that copies it. A program that is missing or that fails is
/// skipped.
fn copy_with(programs: &[CopyProgram], text: &str) {
    for program in programs {
        if program.run(text).is_ok_and(|status| status.success()) {
            return;
        }
    }
}

/// The system clipboard of one session.
///
/// It gives each copy to the clipboard programs on a thread of its own, one copy after the other
/// in the order of the copies, so the last copy is the one that stays. The end of the session
/// waits a short time for a copy that is still running.
#[derive(Debug)]
pub(crate) struct SystemClipboard {
    copies: Option<Sender<String>>,
    worker: Option<JoinHandle<()>>,
    exit_wait: Duration,
}

impl SystemClipboard {
    /// The clipboard of this platform and this session.
    pub(crate) fn for_this_session() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        Self::new(copy_programs(Platform::current(), set), EXIT_WAIT)
    }

    /// A clipboard that gives each copy to `programs`. With no program, a copy does nothing.
    pub(crate) fn new(programs: Vec<CopyProgram>, exit_wait: Duration) -> Self {
        if programs.is_empty() {
            return Self {
                copies: None,
                worker: None,
                exit_wait,
            };
        }
        let (copies, received) = mpsc::channel::<String>();
        let worker = thread::spawn(move || {
            for text in received {
                copy_with(&programs, &text);
            }
        });
        Self {
            copies: Some(copies),
            worker: Some(worker),
            exit_wait,
        }
    }

    /// Give one copy to the clipboard program. It never waits for the program.
    pub(crate) fn copy(&self, text: &str) {
        if let Some(copies) = &self.copies {
            let _ = copies.send(text.to_owned());
        }
    }
}

impl Drop for SystemClipboard {
    fn drop(&mut self) {
        // The worker ends after the copies that it has, because no copy can come any more.
        self.copies = None;
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + self.exit_wait;
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ways this can fail: a platform gets the wrong program or the wrong order; an SSH session
    // gives the copy to the clipboard of the wrong machine; a variable with no value counts as set.
    #[test]
    fn each_platform_and_session_chooses_its_clipboard_programs() {
        let names = |platform, variables: &[&str]| {
            copy_programs(platform, |name| variables.contains(&name))
                .into_iter()
                .map(|program| program.program.into_string().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Platform::MacOs, &[]), ["pbcopy"]);
        assert_eq!(
            copy_programs(Platform::MacOs, |_| false)[0].environment,
            [(OsString::from("LC_CTYPE"), OsString::from("UTF-8"))]
        );
        assert!(names(Platform::Windows, &["DISPLAY", "WAYLAND_DISPLAY"]).is_empty());
        assert!(names(Platform::Unix, &[]).is_empty());
        assert_eq!(names(Platform::Unix, &["WAYLAND_DISPLAY"]), ["wl-copy"]);
        assert_eq!(names(Platform::Unix, &["DISPLAY"]), ["xclip", "xsel"]);
        assert_eq!(
            names(Platform::Unix, &["WAYLAND_DISPLAY", "DISPLAY"]),
            ["wl-copy", "xclip", "xsel"]
        );
        for ssh in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            assert!(names(Platform::MacOs, &[ssh]).is_empty(), "{ssh}");
            assert!(names(Platform::Unix, &[ssh, "DISPLAY"]).is_empty(), "{ssh}");
        }
        assert_eq!(Platform::of(true, false), Platform::MacOs);
        assert_eq!(Platform::of(false, true), Platform::Windows);
        assert_eq!(Platform::of(false, false), Platform::Unix);
        assert_eq!(
            Platform::current(),
            Platform::of(cfg!(target_os = "macos"), cfg!(windows))
        );
        // The session reads its own environment; an empty value is no value.
        let _ = SystemClipboard::for_this_session();
    }

    // Ways this can fail: a copy waits for the program, so a slow program stops the interface; a
    // missing or a failing program stops the copy; the program does not get its environment; two
    // copies end in the wrong order; the end of the session cuts off a running copy, or waits
    // without end for a program that hangs.
    #[cfg(unix)]
    #[test]
    fn a_copy_reaches_the_first_program_that_copies_without_a_wait() {
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("clipboard.txt");
        let environment = directory.path().join("environment.txt");
        let script = "sleep 0.2; printf %s \"$SKIT_TEST_MARK\" > \"$2\"; cat > \"$1\"";
        let programs = vec![
            CopyProgram::new("skit-test-no-such-clipboard-program", [] as [&str; 0]),
            CopyProgram::new("sh", ["-c", "exit 3"]),
            CopyProgram::new(
                "sh",
                [
                    OsString::from("-c"),
                    OsString::from(script),
                    OsString::from("sh"),
                    saved.clone().into_os_string(),
                    environment.clone().into_os_string(),
                ],
            )
            .with_environment("SKIT_TEST_MARK", "UTF-8"),
        ];
        let clipboard = SystemClipboard::new(programs, Duration::from_secs(10));
        let started = Instant::now();
        clipboard.copy("first");
        clipboard.copy("日本 second");
        assert!(
            started.elapsed() < Duration::from_millis(150),
            "a copy waited for the program"
        );
        drop(clipboard);
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "日本 second");
        assert_eq!(std::fs::read_to_string(&environment).unwrap(), "UTF-8");

        let hanging = SystemClipboard::new(
            vec![CopyProgram::new("sh", ["-c", "sleep 5"])],
            Duration::from_millis(50),
        );
        hanging.copy("never saved");
        let ending = Instant::now();
        drop(hanging);
        assert!(
            ending.elapsed() < Duration::from_secs(2),
            "the end of the session waited for a program that hangs"
        );

        // A clipboard with no program does nothing and ends at once.
        let none = SystemClipboard::new(Vec::new(), Duration::from_secs(10));
        none.copy("nothing");
        drop(none);
    }

    // Ways this can fail: a program that ends without reading the whole copy counts as a copy, so
    // the next program never gets it.
    #[cfg(unix)]
    #[test]
    fn a_program_that_does_not_read_the_copy_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("clipboard.txt");
        // More text than a pipe holds, so the write fails when the first program ends.
        let text = "x".repeat(1 << 20);
        let saving = CopyProgram::new(
            "sh",
            [
                OsString::from("-c"),
                OsString::from("cat > \"$1\""),
                OsString::from("sh"),
                saved.clone().into_os_string(),
            ],
        );
        copy_with(&[CopyProgram::new("sh", ["-c", "exit 0"]), saving], &text);
        assert_eq!(std::fs::read_to_string(&saved).unwrap().len(), text.len());
    }
}
