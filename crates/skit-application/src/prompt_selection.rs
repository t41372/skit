//! Persistent state for the most recently picked prompt runner.

use std::fmt::Debug;

use crate::form_state::StateWriteError;

/// Persistence port for interactive prompt-runner selection state.
///
/// This value is only a picker prefill. It must never participate in non-interactive runner
/// resolution.
pub trait PromptSelectionStore: Debug {
    /// Load the most recently picked runner. Missing or corrupt state returns an empty string.
    #[must_use]
    fn load_last_runner(&self) -> String;

    /// Atomically replace the most recently picked runner.
    fn save_last_runner(&self, name: &str) -> Result<(), StateWriteError>;
}

/// Shared prompt-selection use cases for every frontend.
#[derive(Debug)]
pub struct PromptSelectionService<R> {
    store: R,
}

impl<R> PromptSelectionService<R>
where
    R: PromptSelectionStore,
{
    /// Construct the service around one storage adapter.
    #[must_use]
    pub const fn new(store: R) -> Self {
        Self { store }
    }

    /// Return the current interactive picker prefill.
    #[must_use]
    pub fn last_runner(&self) -> String {
        self.store.load_last_runner()
    }

    /// Remember one explicit picker choice.
    pub fn remember_runner(&self, name: &str) -> Result<(), StateWriteError> {
        self.store.save_last_runner(name)
    }

    /// Expose the port for composition-level inspection and focused tests.
    #[must_use]
    pub const fn store(&self) -> &R {
        &self.store
    }
}
