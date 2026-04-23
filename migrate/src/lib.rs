//! Aspen's in-house Rust migration runner.
//!
//! The binary in `main.rs` is the developer-facing CLI; this library
//! exists so the runner is callable from tests and (future) integration
//! harnesses without going through a process boundary.

pub mod migration;
pub mod migrations;
pub mod registry;

pub use migration::{
    AppliedMigration, Migration, SqlMigration, ensure_migrations_table, list_applied, run_apply,
    run_down, run_import_diesel, run_redo, run_revert, run_status, run_up,
};
pub use registry::MIGRATIONS;
