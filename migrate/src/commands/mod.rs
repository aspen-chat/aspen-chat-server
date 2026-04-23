//! CLI commands that operate on the migration source tree (rather
//! than on the database). Runner commands (`up`, `down`, etc.) live in
//! `aspen_migrate::migration`; this module is the home for commands
//! that edit `migrate/src/migrations/` and `registry.rs` directly.

mod ast_rewrite;
pub mod delete;
pub mod new;
