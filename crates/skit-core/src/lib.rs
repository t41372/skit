#![forbid(unsafe_code)]

mod language;
mod paths;
mod state;
mod store;

pub use language::{
    DepsFlavor, Family, LanguageSpec, kind_for_extension, kind_for_shebang_text, known_kinds,
    python_version_pin, shebang_program_from_line, spec_for, stored_name,
};
pub use paths::{PathContext, PathError, Platform, discover_roots, resolve_roots};
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{Entry, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store};
