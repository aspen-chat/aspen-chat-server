//! `rust_i18n::i18n!` reads the catalogue in `server/locales/` as this crate compiles, but cannot
//! tell Cargo it did; this does, so a changed catalogue rebuilds the crate.
fn main() {
    println!("cargo:rerun-if-changed=../locales");
}
