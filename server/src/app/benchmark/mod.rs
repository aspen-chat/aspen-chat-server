//! Benchmark populations: written straight into the database by `bench seed` (`seed`), removed
//! by `bench purge` (`purge`), both run from `crate::operator`.

mod purge;
mod seed;

pub use purge::purge;
pub use seed::seed;
