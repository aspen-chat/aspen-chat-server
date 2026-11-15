//! UniFFI's bindings generator, as `voice_client/README.md` runs it: `generate --library` reads
//! the metadata the built library carries and writes the Swift module and the Kotlin package.

fn main() {
    uniffi::uniffi_bindgen_main()
}
