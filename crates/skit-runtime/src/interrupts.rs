//! The interrupt keys of the terminal while a run has it.
//!
//! A child that skit starts shares the terminal and the process group of skit. The terminal sends
//! the signal of Ctrl+C (`SIGINT`) and of Ctrl+\ (`SIGQUIT`) to each process in that group, so
//! skit receives the signal together with the child. The key is for the run: the child decides
//! what it does. skit must not stop before the child. It waits, then it keeps the records of the
//! run and reports the outcome, as `system(3)` does when it ignores the two signals while it waits.
//!
//! skit installs one handler for each signal after it starts its first child, so that child gets
//! the signal disposition that skit received. While no child runs, the handler does the default
//! action: it stops skit with the same signal, so a shell sees the same status as before. While a
//! child runs, the handler does nothing. A handler, unlike an ignored signal, does not go into a
//! new program, so each later child starts with the default action of the signal.
//!
//! The order has a cost. In the moment between the start of a child and the end of the call that
//! installs the handler, a key still stops skit. Most of that moment is in the start call: the
//! child already runs before the call returns to skit. The moment is shorter than a millisecond,
//! or about 20 milliseconds on a host that is very busy. A person needs at least about 100
//! milliseconds to see that a run started and press a key, so no person presses a key in that
//! moment. The order stays because `skit run` starts one child, and that child must keep an
//! ignored Ctrl+C: a non-interactive shell starts a background job with Ctrl+C ignored. skit
//! cannot read the disposition that it received and install the handler first only when the
//! signal is not ignored: signal-hook and tokio cannot read a disposition, nix and rustix read it
//! only through `unsafe` calls, and this crate forbids `unsafe` code. A blocked signal cannot
//! close the moment either: the child keeps the signal mask of skit, and a mask holds for one
//! thread only, so the signal can go to another thread of skit.
//!
//! On Windows, the console sends the events of Ctrl+C and Ctrl+Break to each process that it
//! serves, so the child gets its own event. While a child runs, skit listens for both events
//! through tokio, and its handler reports each event as handled, so the default handler does not
//! stop skit. While no child runs, nothing listens, and the handler lets the default handler stop
//! skit, as before. skit sets no handler that ignores the events, so no child gets that setting.
//! The listeners start at the same place as on Unix, just after the child, so the same short
//! moment exists. The run then gets the exit code of the script, untouched, as each run on Windows
//! does: a script that the default handler stops exits with `STATUS_CONTROL_C_EXIT`
//! (`0xC000013A`).

#[cfg(unix)]
mod unix {
    use std::sync::{
        Mutex, Once, PoisonError,
        atomic::{AtomicBool, Ordering},
    };

    use signal_hook::consts::{SIGINT, SIGQUIT};

    /// True while no child runs. The handler then does the default action of its signal.
    static DEFAULT_ACTION: AtomicBool = AtomicBool::new(true);
    /// How many children skit waits for now.
    static WAITS: Mutex<usize> = Mutex::new(0);
    static INSTALL: Once = Once::new();

    /// The interrupt keys belong to the child while this value lives.
    #[derive(Debug)]
    pub(crate) struct ChildInterrupts(());

    impl ChildInterrupts {
        /// Give the interrupt keys to a child that skit started and waits for.
        ///
        /// Call this after the start, so that the child gets the disposition that skit received.
        pub(crate) fn begin() -> Self {
            INSTALL.call_once(|| {
                // When a handler cannot go in, the signal keeps its default action. skit then
                // stops with the child, as it did before this module.
                for signal in [SIGINT, SIGQUIT] {
                    let _ =
                        signal_hook::flag::register_conditional_default(signal, &DEFAULT_ACTION);
                }
            });
            let mut waits = WAITS.lock().unwrap_or_else(PoisonError::into_inner);
            *waits = waits.saturating_add(1);
            DEFAULT_ACTION.store(false, Ordering::SeqCst);
            Self(())
        }
    }

    impl Drop for ChildInterrupts {
        fn drop(&mut self) {
            let mut waits = WAITS.lock().unwrap_or_else(PoisonError::into_inner);
            *waits = waits.saturating_sub(1);
            if *waits == 0 {
                DEFAULT_ACTION.store(true, Ordering::SeqCst);
            }
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::ChildInterrupts;

#[cfg(windows)]
mod windows {
    use tokio::signal::windows::{CtrlBreak, CtrlC, ctrl_break, ctrl_c};

    /// The events of Ctrl+C and Ctrl+Break belong to the child while this value lives.
    ///
    /// Each listener makes the console handler of skit report its event as handled. The handler
    /// goes in with the first listener and stays, and with no listener it lets the default handler
    /// run.
    #[derive(Debug)]
    pub(crate) struct ChildInterrupts {
        _ctrl_c: Option<CtrlC>,
        _ctrl_break: Option<CtrlBreak>,
    }

    impl ChildInterrupts {
        /// Give the interrupt keys to a child that skit started and waits for.
        pub(crate) fn begin() -> Self {
            // When a listener cannot start, the event keeps its default action. skit then stops
            // with the child, as it did before this module.
            Self {
                _ctrl_c: ctrl_c().ok(),
                _ctrl_break: ctrl_break().ok(),
            }
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::ChildInterrupts;

/// A platform with neither Unix signals nor a Windows console keeps the default actions.
#[cfg(not(any(unix, windows)))]
#[derive(Debug)]
pub(crate) struct ChildInterrupts;

#[cfg(not(any(unix, windows)))]
impl ChildInterrupts {
    pub(crate) const fn begin() -> Self {
        Self
    }
}
