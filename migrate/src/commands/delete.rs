//! `delete <name>` — discard a locally-authored migration that has
//! never been shipped.

use std::io::{self, Write as _};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use colored::Colorize;

use super::ast_rewrite::{remove_mod_declaration, remove_registry_entry};

/// Delete a locally-authored migration. Rewrites `migrations/mod.rs`
/// and `registry.rs` to drop the references, then removes the
/// `migrate/src/migrations/m<id>/` directory from disk.
///
/// Three pieces of state have to stay consistent for the crate to
/// compile: the directory, the `pub mod` line, and the `MIGRATIONS`
/// entry. We plan both rewrites in memory first so a malformed
/// `registry.rs` (someone reshaped the slice) fails before we start
/// mutating the tree. The rewrites are then committed *before* the
/// directory is removed — that way any interruption between the two
/// steps leaves the crate buildable (source files reference no missing
/// directory), never the reverse.
///
/// The command intentionally does NOT touch the database. Deleting a
/// migration after it has been applied anywhere would orphan the
/// corresponding `__aspen_migrations` row — the warning printed before
/// the prompt is the only guard against that; the `-y` bypass is for
/// developers who have already confirmed it in other ways.
pub fn delete_migration(name: &str, assume_yes: bool) -> Result<()> {
    let dir_name = resolve_module_dir_name(name);

    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = crate_root.join("src/migrations").join(&dir_name);
    let mod_rs_path = crate_root.join("src/migrations/mod.rs");
    let registry_rs_path = crate_root.join("src/registry.rs");

    let mod_rs_current = std::fs::read_to_string(&mod_rs_path)
        .with_context(|| format!("failed to read {}", mod_rs_path.display()))?;
    let registry_rs_current = std::fs::read_to_string(&registry_rs_path)
        .with_context(|| format!("failed to read {}", registry_rs_path.display()))?;

    let (mod_rs_updated, mod_rs_had_entry) = remove_mod_declaration(&mod_rs_current, &dir_name)?;
    let (registry_rs_updated, registry_rs_had_entry) =
        remove_registry_entry(&registry_rs_current, &dir_name)?;
    let dir_exists = dir.exists();

    if !mod_rs_had_entry && !registry_rs_had_entry && !dir_exists {
        bail!(
            "no migration named {dir_name:?} found (no directory, no declaration in \
             migrations/mod.rs, no entry in registry.rs)",
        );
    }

    print_delete_plan(
        &dir_name,
        dir_exists,
        mod_rs_had_entry,
        registry_rs_had_entry,
    );

    if !assume_yes && !confirm_delete(&dir_name)? {
        println!("Aborted. Nothing was changed.");
        return Ok(());
    }

    if mod_rs_had_entry {
        std::fs::write(&mod_rs_path, mod_rs_updated)
            .with_context(|| format!("failed to write {}", mod_rs_path.display()))?;
    }
    if registry_rs_had_entry {
        std::fs::write(&registry_rs_path, registry_rs_updated)
            .with_context(|| format!("failed to write {}", registry_rs_path.display()))?;
    }
    if dir_exists {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove {}", dir.display()))?;
    }

    println!("Deleted migration {dir_name}.");
    Ok(())
}

/// Accept either `m20260423_063751_asd` (the on-disk module name) or
/// `20260423_063751_asd` (the migration id used by `apply` / `revert`)
/// and return the on-disk form. The heuristic is: if the argument
/// already looks like `m` followed by a digit, it's the module form;
/// otherwise prepend `m`. A malformed argument will surface downstream
/// as a "no migration found" error, which is the clearest signal.
fn resolve_module_dir_name(arg: &str) -> String {
    let bytes = arg.as_bytes();
    if bytes.first() == Some(&b'm') && bytes.get(1).is_some_and(u8::is_ascii_digit) {
        arg.to_owned()
    } else {
        format!("m{arg}")
    }
}

/// Human-readable preview of the filesystem operations the confirmed
/// command will perform. Printed before the prompt so the user can
/// sanity-check the plan (especially on partial-state recovery, where
/// some of the three pieces may already be absent).
fn print_delete_plan(
    dir_name: &str,
    dir_exists: bool,
    mod_rs_had_entry: bool,
    registry_rs_had_entry: bool,
) {
    println!("About to delete migration {dir_name}:");
    println!(
        "  {} migrate/src/migrations/{dir_name}/",
        if dir_exists {
            "remove"
        } else {
            "skip  (already absent)"
        },
    );
    println!(
        "  {} `pub mod {dir_name};` from migrate/src/migrations/mod.rs",
        if mod_rs_had_entry {
            "remove"
        } else {
            "skip  (already absent)"
        },
    );
    println!(
        "  {} `&migrations::{dir_name}::M` from migrate/src/registry.rs",
        if registry_rs_had_entry {
            "remove"
        } else {
            "skip  (already absent)"
        },
    );
    println!();
    let warning = "WARNING".yellow();
    let permanent = "PERMANENT".red();
    println!("{warning}: deleting a migration is {permanent}. Only do this if you are certain");
    println!("this migration has NEVER been applied to a production database. An applied");
    println!(
        "migration that no longer exists in the registry would leave its \
         `__aspen_migrations`"
    );
    println!("row orphaned and the deployment unable to roll the change back. If in doubt,");
    println!("author a new migration that reverses the change instead.");
    println!();
}

/// Require the operator to type `yes` at the terminal. Anything else
/// (including EOF from a non-interactive stdin) aborts — the default
/// for a destructive command must be no-op, not destroy.
fn confirm_delete(dir_name: &str) -> Result<bool> {
    print!("Type `yes` to delete {dir_name}: ");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut line = String::new();
    let n = io::stdin()
        .read_line(&mut line)
        .context("failed to read confirmation from stdin")?;
    if n == 0 {
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_module_dir_name_accepts_both_forms() {
        // The `m`-prefixed form is the on-disk / registry form and
        // passes through unchanged.
        assert_eq!(
            resolve_module_dir_name("m20260423_063751_asd"),
            "m20260423_063751_asd",
        );
        // The bare id form (what `apply` / `revert` take) gets `m`
        // prepended so both command families can use the same
        // identifier.
        assert_eq!(
            resolve_module_dir_name("20260423_063751_asd"),
            "m20260423_063751_asd",
        );
    }
}
