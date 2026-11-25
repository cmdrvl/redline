//! Deterministic prose and document diff that reports every change as exact
//! byte spans in the original inputs.
//!
//! The library takes byte slices and touches no filesystem, clock or network,
//! so it builds for `wasm32-unknown-unknown`. The command-line tool lives
//! behind the `cli` feature.

#![forbid(unsafe_code)]

pub mod canon;
pub mod classify;
#[cfg(feature = "cli")]
pub mod cli;
pub mod diff;
pub mod engine;
pub mod refusal;
pub mod report;
pub mod tokens;

pub use canon::{CanonicalDoc, Edges, Mode, canonicalize};
pub use classify::HunkClass;
pub use engine::{Input, ModeChoice, Options, compare, hunk_digest};
pub use refusal::{Refusal, RefusalCode};
pub use report::{Hunk, Outcome, Report, Summary};
