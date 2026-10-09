//! Rewriters for `migrations/mod.rs` and `registry.rs` shared by the
//! `new` and `delete` commands. Both files are parsed with `syn` and
//! re-emitted with `prettyplease`, so `//`-style comments and blank
//! lines do not survive the round-trip — `//!` inner doc comments do,
//! because `syn` models them as attributes.

use anyhow::{Context, Result, anyhow};
use quote::ToTokens;
use syn::{Expr, Ident, Item, parse_quote, parse_str};

/// Inner doc block that `append_mod_declaration` and
/// `append_registry_entry` stamp at the top of the files they rewrite.
/// It tells any human opening `migrations/mod.rs` or `registry.rs`
/// that their `//` comments and blank lines will be discarded the next
/// time the scaffolder runs, and points them at the migration's own
/// `mod.rs` as the correct home for per-migration prose.
const AUTOGEN_WARNING: &str = "\
//! This file is rewritten in place by `cargo run -p aspen-migrate -- new`,
//! which parses the current contents with `syn` and re-emits them via
//! `prettyplease`. `//`-style comments and blank lines here do not
//! survive that round-trip. Put per-migration documentation in each
//! migration's own `mod.rs`, where it sits next to the SQL it describes.
";

/// Distinctive substring of the warning used to detect it on repeat
/// runs. Chosen so that hand-written prose is unlikely to trip it.
const AUTOGEN_WARNING_MARKER: &str = "aspen-migrate -- new";

/// Idempotently prepend `AUTOGEN_WARNING` as inner doc attributes on
/// `attrs`. Detection uses a marker substring rather than byte-exact
/// comparison so cosmetic edits to the warning wording (or other
/// `//!` prose the user adds above it) don't cause duplicate stamps.
fn ensure_autogen_warning(attrs: &mut Vec<syn::Attribute>) {
    let already_present = attrs
        .iter()
        .any(|attr| attr_doc_string(attr).is_some_and(|s| s.contains(AUTOGEN_WARNING_MARKER)));
    if already_present {
        return;
    }
    let stub: syn::File =
        syn::parse_str(AUTOGEN_WARNING).expect("AUTOGEN_WARNING must be a valid Rust file");
    for (i, attr) in stub.attrs.into_iter().enumerate() {
        attrs.insert(i, attr);
    }
}

/// Return the literal body of a `#[doc = "..."]` / `#![doc = "..."]`
/// attribute, if the attribute has that exact shape. Anything else
/// (non-`doc` attribute, `#[doc(...)]` meta list, non-string literal)
/// returns `None`.
fn attr_doc_string(attr: &syn::Attribute) -> Option<String> {
    if !attr.path().is_ident("doc") {
        return None;
    }
    let syn::Meta::NameValue(nv) = &attr.meta else {
        return None;
    };
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(s),
        ..
    }) = &nv.value
    else {
        return None;
    };
    Some(s.value())
}

/// Append `pub mod <dir_name>;` to `migrations/mod.rs` by parsing the
/// file into a `syn::File`, pushing a new `Item::Mod`, and serializing
/// it back with `prettyplease`. Idempotent: if a module of that name
/// is already declared we return the (reformatted) file unchanged, so
/// re-running `new` after a crash does not duplicate the declaration.
/// Also stamps `AUTOGEN_WARNING` at the top on first pass.
///
/// This is a lossy rewrite — `//`-style comments and blank lines do
/// not survive the round-trip through `syn`. Inner doc comments
/// (`//!`) do survive, because `syn` models them as attributes.
pub(super) fn append_mod_declaration(contents: &str, dir_name: &str) -> Result<String> {
    let mut file = syn::parse_file(contents)
        .with_context(|| "failed to parse migrations/mod.rs as Rust source")?;
    let ident: Ident = parse_str(dir_name)
        .with_context(|| format!("`{dir_name}` is not a valid Rust identifier"))?;

    let already_declared = file
        .items
        .iter()
        .any(|item| matches!(item, Item::Mod(m) if m.ident == ident));
    if !already_declared {
        let new_item: Item = parse_quote! { pub mod #ident; };
        file.items.push(new_item);
    }

    ensure_autogen_warning(&mut file.attrs);
    Ok(prettyplease::unparse(&file))
}

/// Append `&migrations::<dir_name>::M` to the `MIGRATIONS` slice in
/// `registry.rs` by parsing the file, locating the static item, and
/// pushing a new element onto its array literal. Errors loudly if the
/// registry has been reshaped into something this rewriter can't
/// recognize (no `MIGRATIONS` static, or its value isn't `&[...]`) —
/// silently producing a broken file would be much worse than refusing
/// and telling the user to register by hand.
///
/// Idempotent: if an element matching `&migrations::<dir_name>::M` is
/// already present (by token-stream equality) the array is left alone.
/// Also stamps `AUTOGEN_WARNING` at the top on first pass.
///
/// Like `append_mod_declaration`, this is a lossy rewrite — only the
/// AST survives the round-trip.
pub(super) fn append_registry_entry(contents: &str, dir_name: &str) -> Result<String> {
    let mut file =
        syn::parse_file(contents).with_context(|| "failed to parse registry.rs as Rust source")?;
    let ident: Ident = parse_str(dir_name)
        .with_context(|| format!("`{dir_name}` is not a valid Rust identifier"))?;
    let new_elem: Expr = parse_quote! { &migrations::#ident::M };
    let new_elem_tokens = new_elem.to_token_stream().to_string();

    let shape_error = || {
        anyhow!(
            "could not find a `pub static MIGRATIONS: &[...] = &[...];` item in registry.rs; \
             automated registration refuses to guess — register the new module by hand",
        )
    };

    let static_item = file
        .items
        .iter_mut()
        .find_map(|item| match item {
            Item::Static(s) if s.ident == "MIGRATIONS" => Some(s),
            _ => None,
        })
        .ok_or_else(shape_error)?;

    let Expr::Reference(ref_expr) = static_item.expr.as_mut() else {
        return Err(shape_error());
    };
    let Expr::Array(array) = ref_expr.expr.as_mut() else {
        return Err(shape_error());
    };

    let already_present = array
        .elems
        .iter()
        .any(|existing| existing.to_token_stream().to_string() == new_elem_tokens);
    if !already_present {
        array.elems.push(new_elem);
    }

    ensure_autogen_warning(&mut file.attrs);
    Ok(prettyplease::unparse(&file))
}

/// Remove `pub mod <dir_name>;` from `migrations/mod.rs`. Mirrors
/// `append_mod_declaration`: same parse-with-`syn`, emit-with-
/// `prettyplease` round-trip (so `//`-comments and blank lines do not
/// survive), same autogen-warning stamping. Returns `(updated_source,
/// was_present)`; the boolean lets the caller treat a missing entry as
/// a partial-state recovery rather than an error, which matters because
/// a developer may have already hand-edited one file and be running
/// `delete` to clean up the others.
pub(super) fn remove_mod_declaration(contents: &str, dir_name: &str) -> Result<(String, bool)> {
    let mut file = syn::parse_file(contents)
        .with_context(|| "failed to parse migrations/mod.rs as Rust source")?;
    let ident: Ident = parse_str(dir_name)
        .with_context(|| format!("`{dir_name}` is not a valid Rust identifier"))?;

    let before = file.items.len();
    file.items
        .retain(|item| !matches!(item, Item::Mod(m) if m.ident == ident));
    let was_present = file.items.len() != before;

    ensure_autogen_warning(&mut file.attrs);
    Ok((prettyplease::unparse(&file), was_present))
}

/// Remove `&migrations::<dir_name>::M` from the `MIGRATIONS` slice in
/// `registry.rs`. Same recognition rules as `append_registry_entry` —
/// if the static has been reshaped into something we don't understand
/// we refuse rather than silently emit a broken file. Returns
/// `(updated_source, was_present)`.
pub(super) fn remove_registry_entry(contents: &str, dir_name: &str) -> Result<(String, bool)> {
    let mut file =
        syn::parse_file(contents).with_context(|| "failed to parse registry.rs as Rust source")?;
    let ident: Ident = parse_str(dir_name)
        .with_context(|| format!("`{dir_name}` is not a valid Rust identifier"))?;
    let target: Expr = parse_quote! { &migrations::#ident::M };
    let target_tokens = target.to_token_stream().to_string();

    let shape_error = || {
        anyhow!(
            "could not find a `pub static MIGRATIONS: &[...] = &[...];` item in registry.rs; \
             automated removal refuses to guess — unregister the module by hand",
        )
    };

    let static_item = file
        .items
        .iter_mut()
        .find_map(|item| match item {
            Item::Static(s) if s.ident == "MIGRATIONS" => Some(s),
            _ => None,
        })
        .ok_or_else(shape_error)?;

    let Expr::Reference(ref_expr) = static_item.expr.as_mut() else {
        return Err(shape_error());
    };
    let Expr::Array(array) = ref_expr.expr.as_mut() else {
        return Err(shape_error());
    };

    let before = array.elems.len();
    let kept: syn::punctuated::Punctuated<Expr, syn::Token![,]> = std::mem::take(&mut array.elems)
        .into_iter()
        .filter(|existing| existing.to_token_stream().to_string() != target_tokens)
        .collect();
    let was_present = kept.len() != before;
    array.elems = kept;

    ensure_autogen_warning(&mut file.attrs);
    Ok((prettyplease::unparse(&file), was_present))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply the same AST mutations the append helpers do (stamp the
    /// autogen warning, reformat through `prettyplease`) to a raw
    /// snippet. This lets assertions express intent ("the result is
    /// the same Rust source as `expected`") without hard-coding
    /// prettyplease's whitespace choices or the exact wording of the
    /// warning block.
    fn rewritten(rust_source: &str) -> String {
        let mut file = syn::parse_file(rust_source).unwrap();
        ensure_autogen_warning(&mut file.attrs);
        prettyplease::unparse(&file)
    }

    #[test]
    fn append_mod_declaration_appends_line() {
        let before = "//! header\n\npub mod m20260101_000000_first;\n";
        let expected = "//! header\n\npub mod m20260101_000000_first;\n\
                        pub mod m20260102_000000_second;\n";
        let after = append_mod_declaration(before, "m20260102_000000_second").unwrap();
        assert_eq!(after, rewritten(expected));
        assert!(
            after.contains(AUTOGEN_WARNING_MARKER),
            "output should carry the autogen warning marker: {after}",
        );
    }

    #[test]
    fn append_mod_declaration_handles_missing_trailing_newline() {
        // A source without a trailing newline still comes back
        // well-terminated, since prettyplease emits a whole file.
        let before = "pub mod m20260101_000000_first;";
        let after = append_mod_declaration(before, "m20260102_000000_second").unwrap();
        assert!(
            after.ends_with('\n'),
            "result should end with newline: {after:?}"
        );
        assert!(
            after.contains("pub mod m20260101_000000_first;"),
            "result should keep existing declaration: {after}",
        );
        assert!(
            after.contains("pub mod m20260102_000000_second;"),
            "result should add new declaration: {after}",
        );
        assert!(
            after.contains(AUTOGEN_WARNING_MARKER),
            "result should stamp the autogen warning: {after}",
        );
    }

    #[test]
    fn append_mod_declaration_is_idempotent() {
        // Run the helper once to produce a file that already carries
        // the warning + both declarations, then run it again and check
        // the second pass is a no-op.
        let seed = "pub mod m20260101_000000_first;\npub mod m20260102_000000_second;\n";
        let first_pass = append_mod_declaration(seed, "m20260102_000000_second").unwrap();
        let second_pass = append_mod_declaration(&first_pass, "m20260102_000000_second").unwrap();
        assert_eq!(first_pass, second_pass);
        assert_eq!(first_pass.matches(AUTOGEN_WARNING_MARKER).count(), 1);
    }

    #[test]
    fn append_registry_entry_inserts_before_closing_bracket() {
        let before = "\
use crate::Migration;
use crate::migrations;

pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
];
";
        let expected = "\
use crate::Migration;
use crate::migrations;

pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
    &migrations::m20260102_000000_second::M,
];
";
        let after = append_registry_entry(before, "m20260102_000000_second").unwrap();
        assert_eq!(after, rewritten(expected));
        assert!(
            after.contains(AUTOGEN_WARNING_MARKER),
            "output should carry the autogen warning marker: {after}",
        );
    }

    #[test]
    fn append_registry_entry_is_idempotent() {
        // Same shape as append_mod_declaration_is_idempotent: first
        // pass produces the warning-stamped form, second pass must be
        // a fixed point.
        let seed = "\
pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
];
";
        let first_pass = append_registry_entry(seed, "m20260101_000000_first").unwrap();
        let second_pass = append_registry_entry(&first_pass, "m20260101_000000_first").unwrap();
        assert_eq!(first_pass, second_pass);
        assert_eq!(first_pass.matches(AUTOGEN_WARNING_MARKER).count(), 1);
    }

    #[test]
    fn append_registry_entry_errors_on_unrecognized_shape() {
        // No `MIGRATIONS` static at all — the rewriter must refuse
        // rather than guess where to put the new entry.
        let before = "pub static OTHER: u32 = 0;\n";
        let err = append_registry_entry(before, "m20260101_000000_first").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("MIGRATIONS"), "unexpected error: {msg}");
    }

    #[test]
    fn remove_mod_declaration_drops_matching_line() {
        let before = "pub mod m20260101_000000_first;\npub mod m20260102_000000_second;\n";
        let expected = "pub mod m20260101_000000_first;\n";
        let (after, was_present) =
            remove_mod_declaration(before, "m20260102_000000_second").unwrap();
        assert!(was_present);
        assert_eq!(after, rewritten(expected));
    }

    #[test]
    fn remove_mod_declaration_reports_missing() {
        // Partial-state recovery: the declaration may already be gone
        // because the developer started cleanup by hand. The helper
        // must return `false` rather than erroring so `delete` can
        // still finish the job on the other two locations.
        let before = "pub mod m20260101_000000_first;\n";
        let (after, was_present) =
            remove_mod_declaration(before, "m20260102_000000_second").unwrap();
        assert!(!was_present);
        assert_eq!(after, rewritten(before));
    }

    #[test]
    fn remove_registry_entry_drops_matching_element() {
        let before = "\
use crate::Migration;
use crate::migrations;

pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
    &migrations::m20260102_000000_second::M,
];
";
        let expected = "\
use crate::Migration;
use crate::migrations;

pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
];
";
        let (after, was_present) =
            remove_registry_entry(before, "m20260102_000000_second").unwrap();
        assert!(was_present);
        assert_eq!(after, rewritten(expected));
    }

    #[test]
    fn remove_registry_entry_reports_missing() {
        let before = "\
pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20260101_000000_first::M,
];
";
        let (after, was_present) =
            remove_registry_entry(before, "m20260102_000000_second").unwrap();
        assert!(!was_present);
        assert_eq!(after, rewritten(before));
    }

    #[test]
    fn remove_registry_entry_errors_on_unrecognized_shape() {
        let before = "pub static OTHER: u32 = 0;\n";
        let err = remove_registry_entry(before, "m20260101_000000_first").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("MIGRATIONS"), "unexpected error: {msg}");
    }
}
