//! Core domain types, configuration, and error boundaries.
//!
//! This module owns the types that every other subsystem depends on. It contains
//! no behaviour beyond validation and serialization of the load-test contract.

pub mod config;
pub mod constants;
pub mod result;
pub mod snapshot;

pub use config::{Config, Mode};
pub use result::ExperimentResult;
pub use snapshot::StatsSnapshot;
