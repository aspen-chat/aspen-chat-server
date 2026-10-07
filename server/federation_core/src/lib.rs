//! The parts of federation (`app::federation`) that need nothing of the server: what one
//! deployment signs for another (`jws`), and the gates and shared list deciding who may come and
//! go (`policy`).

pub mod jws;
pub mod policy;
