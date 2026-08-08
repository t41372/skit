#![forbid(unsafe_code)]

mod add;
mod assembly;
mod config;
mod declared_edit;
mod forms;
mod integrity;
mod language;
mod launch;
mod params;
mod paths;
mod pep723;
mod presets;
mod process;
mod programs;
mod python_add;
mod python_analysis;
mod python_managed;
mod run;
mod state;
mod store;
mod uv;

pub use add::{AddFileRequest, AddMode, AddPreparation, AddUseCaseError, add_file};
pub use assembly::{Assembly, AssemblyError, assemble_delivery};
pub use config::{LaunchConfig, load_launch_config};
pub use declared_edit::{DeclaredEditResult, DeclaredEdits, edit_declared};
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
pub use pep723::{
    Pep723Metadata, build_pep723, has_pep723, inject_pep723, parse_pep723, set_pep723_axes,
};
pub use presets::{PresetFromLastError, save_preset_from_last};
pub use process::{RunError, run_launch};
pub use programs::ProgramSearch;
pub use python_add::{PythonAddRequest, add_python_file};
pub use python_analysis::suggest_python_dependencies;
pub use python_managed::{PythonManagedAnalysis, PythonManagedCandidate, analyze_python_managed};
pub use run::{
    ExtraArgsResolution, PrepareRunError, PreparedRun, RunRequest, prepare_raw_run, prepare_run,
    remembered_values, resolve_extra_args,
};
pub use state::{EntryState, LastRun, StateError, StateStore};
pub use store::{
    Entry, EntryDraft, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta, Store,
};
pub use uv::effective_uv_metadata;
