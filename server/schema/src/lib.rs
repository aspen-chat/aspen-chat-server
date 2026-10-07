//! The database's tables as Diesel sees them, written by `diesel print-schema` into
//! `schema.rs` from the database the migrations in `migrate/` make.

mod schema;
pub use schema::*;
