//! Generates the libobs bindings the helper uses and tells the linker where libobs is.
//!
//! On Linux `pkg-config` finds libobs. On Windows and macOS OBS Studio ships no development
//! files, so `LIBOBS_INCLUDE_DIR` (the `libobs` directory of an OBS Studio source checkout
//! matching the installed version, with an `obsconfig.h` written from its template) and
//! `LIBOBS_LIB_DIR` (where `obs.lib` is, or on macOS a `libobs.dylib` linking to OBS.app's
//! `Contents/Frameworks/libobs.framework/Versions/A/libobs`) must be set.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=LIBOBS_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=LIBOBS_LIB_DIR");

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .allowlist_function("obs_.*|base_set_log_handler|os_gettime_ns")
        .allowlist_type("obs_.*|encoder_packet|vec2|video_.*|audio_.*|speaker_layout|log_handler_t")
        .allowlist_var("OBS_.*|VIDEO_.*|AUDIO_.*|SPEAKERS_.*|MODULE_.*|LOG_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    let mut plugin_dir = None;
    let mut data_dir = None;
    if let Ok(include) = env::var("LIBOBS_INCLUDE_DIR") {
        builder = builder.clang_arg(format!("-I{include}"));
        if let Ok(lib) = env::var("LIBOBS_LIB_DIR") {
            println!("cargo:rustc-link-search=native={lib}");
        }
        println!("cargo:rustc-link-lib=obs");
        // On macOS the headers come from a source checkout and the library from OBS Studio's
        // own bundle (`libobs.framework`, named by its `@rpath` install name, so the helper
        // carries the bundle's Frameworks directory as its rpath), which is also where its
        // plugins and data are at run time.
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,/Applications/OBS.app/Contents/Frameworks");
            plugin_dir = Some("/Applications/OBS.app/Contents/PlugIns".to_string());
            data_dir = Some("/Applications/OBS.app/Contents/Resources/data".to_string());
        }
    } else {
        let libobs = pkg_config::Config::new()
            .probe("libobs")
            .expect("libobs development files: install OBS Studio, or set LIBOBS_INCLUDE_DIR and LIBOBS_LIB_DIR");
        // The headers include one another as `<obs.h>`, so their own directory is needed on
        // the search path whether the .pc file names it or its parent.
        for path in &libobs.include_paths {
            builder = builder.clang_arg(format!("-I{}", path.display()));
            builder = builder.clang_arg(format!("-I{}/obs", path.display()));
        }
        let includedir = pkg_config::get_variable("libobs", "includedir")
            .unwrap_or_else(|_| "/usr/include".into());
        builder = builder.clang_arg(format!("-I{includedir}/obs"));
        let prefix = pkg_config::get_variable("libobs", "prefix").unwrap_or_else(|_| "/usr".into());
        let libdir = pkg_config::get_variable("libobs", "libdir")
            .unwrap_or_else(|_| format!("{prefix}/lib"));
        plugin_dir = Some(format!("{libdir}/obs-plugins"));
        data_dir = Some(format!("{prefix}/share/obs"));
    }
    // Where the plugins and their data are on this kind of system; overridable at runtime.
    println!(
        "cargo:rustc-env=OBS_DEFAULT_PLUGIN_DIR={}",
        plugin_dir.unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=OBS_DEFAULT_DATA_DIR={}",
        data_dir.unwrap_or_default()
    );

    let bindings = builder.generate().expect("libobs bindings");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");
}
