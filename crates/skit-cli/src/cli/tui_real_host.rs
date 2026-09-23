//! Real production-host fixture for the local UI walker.
//!
//! This module is test-only. It composes the production stores and `TuiHost` with low-level
//! recording adapters. It does not model product behavior.

mod adapters;
mod host;
mod observation;
mod projection;
mod projection_cause;
mod projection_typed;
mod seed;
mod stable_namespace;
mod stable_sandbox;
#[cfg(test)]
mod tests;

pub(super) use host::RealWalkerHost;
pub(super) use observation::{
    ArtifactLeakOracleFact, HostObservation, LeakOracleFacts, ModifiedLeakOraclePair,
    RendererDraftFact, RendererReviewNameFact, SourceIdentityLeakOraclePair,
};
pub(super) use seed::{
    WalkerDirectoryRoot, WalkerDirectorySeed, WalkerExternalReferenceSeed, WalkerExternalSeed,
    WalkerFilePickerTree, WalkerFormSeed, WalkerLastRunSeed, WalkerSeedSpec,
};
pub(super) use stable_namespace::{
    SandboxError, StableHostError, StableHostPrimaryError, StableSandboxNamespace,
};
