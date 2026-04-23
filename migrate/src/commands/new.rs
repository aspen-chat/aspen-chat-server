//! `new <slug>` — scaffold a new migration directory and register it
//! in `migrations/mod.rs` + `registry.rs`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use super::ast_rewrite::{append_mod_declaration, append_registry_entry};

/// Lay down `migrate/src/migrations/m<utc_now>_<slug>/{mod.rs,up.sql,down.sql}`
/// AND register the new module in `migrations/mod.rs` + `registry.rs`.
/// All disk writes happen after the in-memory rewrites succeed, so a
/// malformed `registry.rs` is caught before any files exist on disk —
/// and since the timestamp prefix from `Utc::now()` is strictly later
/// than every existing entry, append-style inserts preserve the
/// chronological invariant `registry.rs` depends on.
pub fn scaffold_new(slug: &str) -> Result<()> {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        bail!("slug must not be empty");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        bail!("slug must be lowercase ascii letters, digits, and underscores only");
    }

    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let dir_name = format!("m{stamp}_{trimmed}");
    let module_id = format!("{stamp}_{trimmed}");

    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = crate_root.join("src/migrations").join(&dir_name);
    let mod_rs_path = crate_root.join("src/migrations/mod.rs");
    let registry_rs_path = crate_root.join("src/registry.rs");

    if dir.exists() {
        bail!("{} already exists", dir.display());
    }

    // Plan the registration rewrites up-front. If either file is in an
    // unexpected shape (someone reshaped the MIGRATIONS slice, say), we
    // want to learn about that now — before we've created any
    // migration files that the user would then have to clean up.
    let mod_rs_current = std::fs::read_to_string(&mod_rs_path)
        .with_context(|| format!("failed to read {}", mod_rs_path.display()))?;
    let registry_rs_current = std::fs::read_to_string(&registry_rs_path)
        .with_context(|| format!("failed to read {}", registry_rs_path.display()))?;
    let mod_rs_updated = append_mod_declaration(&mod_rs_current, &dir_name)?;
    let registry_rs_updated = append_registry_entry(&registry_rs_current, &dir_name)?;

    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    std::fs::write(dir.join("up.sql"), "-- write the forward migration here\n")?;
    std::fs::write(
        dir.join("down.sql"),
        "-- write the inverse of up.sql here\n",
    )?;
    std::fs::write(
        dir.join("mod.rs"),
        format!(
            "use crate::SqlMigration;\n\n\
             pub static M: SqlMigration = SqlMigration {{\n    \
                 id: \"{module_id}\",\n    \
                 up: include_str!(\"up.sql\"),\n    \
                 down: include_str!(\"down.sql\"),\n\
             }};\n"
        ),
    )?;

    std::fs::write(&mod_rs_path, mod_rs_updated)
        .with_context(|| format!("failed to write {}", mod_rs_path.display()))?;
    std::fs::write(&registry_rs_path, registry_rs_updated)
        .with_context(|| format!("failed to write {}", registry_rs_path.display()))?;

    println!("Created  {}", dir.display());
    println!("Declared in {}", mod_rs_path.display());
    println!("Appended to MIGRATIONS in {}", registry_rs_path.display());
    println!();
    println!(
        "Next: edit up.sql / down.sql (or replace mod.rs with a hand-written \
         `impl Migration` for custom Rust), then run `cargo run -p aspen-migrate -- up`."
    );
    Ok(())
}
