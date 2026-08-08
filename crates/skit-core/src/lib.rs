#![forbid(unsafe_code)]

mod paths;
mod state;
mod store;

pub use paths::{PathContext, PathError, Platform, discover_roots, resolve_roots};
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{Entry, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store};
