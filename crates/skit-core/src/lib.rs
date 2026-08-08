#![forbid(unsafe_code)]

mod add;
mod integrity;
mod language;
mod paths;
mod state;
mod store;

pub use add::{AddFileRequest, AddMode, AddPreparation, AddUseCaseError, add_file};
pub use integrity::{TimestampError, format_utc_timestamp, sha256_source_hash};
pub use language::{
    DepsFlavor, ExecutablePolicy, Family, LanguageSpec, infer_kind, infer_kind_with_policy,
    kind_for_extension, kind_for_shebang_text, known_kinds, python_version_pin,
    shebang_program_from_line, spec_for, stored_name,
};
pub use paths::{PathContext, PathError, Platform, discover_roots, resolve_roots};
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{
    Entry, EntryDraft, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store,
};
