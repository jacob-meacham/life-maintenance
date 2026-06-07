#![warn(clippy::pedantic)]
//! lifemaint — track and complete recurring home/life maintenance tasks.
//!
//! Git files are the source of truth; the `lm` CLI is the canonical interface.

pub mod cli;
pub mod config;
pub mod error;
pub mod model;
pub mod schedule;
pub mod schema;
pub mod service;
pub mod status;
pub mod store;

pub use error::{Error, Result};
pub use model::{Completion, Event, Punt, Task, Vendor};
pub use schedule::{Fixed, Schedule};
pub use service::{ReportKind, Service};
pub use status::{compute_status, Bucket, TaskStatus};
pub use store::DataDir;

/// The crate version, from Cargo.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_matches_cargo() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }
}
