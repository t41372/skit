#![forbid(unsafe_code)]

mod add;
mod forms;
mod integrity;
mod language;
mod params;
mod paths;
mod presets;
mod state;
mod store;

pub use add::{AddFileRequest, AddMode, AddPreparation, AddUseCaseError, add_file};
pub use forms::{
    FormField, FormPlan, PlanSource, ResolveError, plan_for_entry, prefill, resolve_values,
    validate_values,
};
pub use integrity::{TimestampError, format_utc_timestamp, sha256_source_hash};
pub use language::{
    DepsFlavor, ExecutablePolicy, Family, LanguageSpec, infer_kind, infer_kind_with_policy,
    kind_for_extension, kind_for_shebang_text, known_kinds, python_version_pin,
    shebang_program_from_line, spec_for, stored_name,
};
pub use params::{
    Binding, Delivery, ParamDecl, ParamDefault, ParamType, declared_from_meta, is_secret_name,
    synthesized_placeholder,
};
pub use paths::{PathContext, PathError, Platform, discover_roots, resolve_roots};
pub use presets::{PresetFromLastError, save_preset_from_last};
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{
    Entry, EntryDraft, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store,
};
