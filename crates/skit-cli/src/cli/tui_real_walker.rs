//! Generic walker smoke coverage over the real production TUI host.

mod corpus;
mod engine;
mod frontend;
mod recording;
#[cfg(test)]
mod tests;
mod trace;

pub(super) use corpus::{
    CorpusKeyEvent, CorpusMouseKind, CorpusOperation, LocaleSmokeTarget, corpus_canvas_size,
    corpus_operation_value,
};
pub(super) use engine::{
    CorpusEngine, CorpusWalkerFactory, close_corpus_engine, random_walk_factory, real_initial_state,
};
pub(super) use recording::{
    canonical_corpus_artifact_values, canonical_corpus_operations, record_real_locale_smoke,
    record_real_review_corpus_in, record_real_smoke_main, record_real_smoke_replay,
    record_real_smoke_stable_pair_in,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) use recording::{
    draft_quarantine_corpus_operations, invalid_corpus_operation,
    rendered_sandbox_root_corpus_operations, review_corpus_contract_operations,
    review_name_cut_corpus_operations,
};
pub(super) use trace::{
    RealTrace, RecordedRealCorpus, RecordedRealTrace, SchemaThreeSink, StableCorpusTracePair,
    StablePairPhase, parse_library_state, validate_effect_chain_termination,
    validate_reducer_action,
};
