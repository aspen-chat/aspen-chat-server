//! Benchmark populations: written straight into the database by `bench seed` (`seed`), removed
//! by `bench purge` (`purge`), both run from the `aspen-chat-server` binary's `operator`.

mod purge;
mod seed;

pub use purge::purge;
pub use seed::seed;
