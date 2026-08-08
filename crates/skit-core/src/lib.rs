#![forbid(unsafe_code)]

mod add;
mod assembly;
mod config;
mod forms;
mod integrity;
mod language;
mod launch;
mod params;
mod paths;
mod pep723;
mod presets;
mod programs;
mod state;
mod store;
mod uv;

pub use add::{AddFileRequest, AddMode, AddPreparation, AddUseCaseError, add_file};
pub use assembly::{Assembly, AssemblyError, assemble_delivery};
pub use config::{LaunchConfig, load_launch_config};
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
pub use launch::{LaunchOptions, LaunchPlan, LaunchPlanError, ProgramResolver, build_launch_plan};
pub use params::{
    Binding, Delivery, ParamDecl, ParamDefault, ParamType, declared_from_meta, is_secret_name,
    synthesized_placeholder,
};
pub use paths::{PathContext, PathError, Platform, discover_roots, resolve_roots};
pub use pep723::{Pep723Metadata, build_pep723, has_pep723, inject_pep723, parse_pep723};
pub use presets::{PresetFromLastError, save_preset_from_last};
pub use programs::ProgramSearch;
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{
    Entry, EntryDraft, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store,
};
pub use uv::effective_uv_metadata;
