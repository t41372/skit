//! Sequential property walker for the frontend-neutral reducer and the Ratatui session.
//!
//! Operations store late-bound ordinals. The driver resolves each ordinal against the state,
//! geometry, and local actions from the most recent frame. A pre-generated reference-state
//! machine cannot read those live values unless it duplicates the product reducer. Plain
//! `proptest` keeps the live model as the single source and still shrinks the operation vector.

#[path = "model_walker/asciicast.rs"]
mod asciicast;
#[path = "model_walker/driver.rs"]
mod driver;
#[path = "model_walker/fake_host.rs"]
mod fake_host;
#[path = "model_walker/fixtures.rs"]
mod fixtures;
#[path = "model_walker/invariants.rs"]
mod invariants;
#[path = "model_walker/local_inventory.rs"]
mod local_inventory;
#[path = "model_walker/strategy.rs"]
mod strategy;
