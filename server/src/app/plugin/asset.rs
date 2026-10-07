//! The files a plugin's views are served from (`views`): its manifest's `assets` directory, read
//! when it is installed and kept in `plugin_asset`, held by each server with the plugin
//! (`registry`), and served at `/api/v1/plugins/{id}/assets/{path}` (`api::plugin::asset`)
//! sandboxed (`VIEW_POLICY`), so a page of a plugin's has an origin of its own, opaque and shared
//! with nothing, however it is opened.

use super::manifest::{Manifest, valid_asset_path};
use crate::app;
use aspen_schema::plugin_asset;
use bytes::Bytes;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;
use std::path::Path;

/// The largest one file may be.
pub const MAX_FILE: u64 = 2 << 20;
/// The most a plugin's files may be together.
pub const MAX_TOTAL: u64 = 20 << 20;
/// The most files a plugin may have.
pub const MAX_FILES: usize = 500;

/// What every view's file is served with: a sandbox without `allow-same-origin`, so the page's
/// origin is opaque and it reaches nothing of the app's, scripts and styles of its own or
/// inline, pictures and fonts of its own, and no connection anywhere: the bridge is its only
/// way out.
pub const VIEW_POLICY: &str = "sandbox allow-scripts allow-forms allow-popups \
    allow-popups-to-escape-sandbox; default-src 'none'; script-src 'self' 'unsafe-inline'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
    media-src 'self' data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'";

/// One of a plugin's files.
#[derive(Debug, Clone)]
pub struct Asset {
    pub content_type: String,
    pub bytes: Bytes,
}

/// The content type of a file by its extension; `None` for one a view has no use for.
pub fn content_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

/// Reads the files beneath `dir`, by their paths relative to it, refusing what a view cannot
/// be served, and checks that each kind of channel's view is among them.
pub fn read_dir(manifest: &Manifest, dir: &Path) -> Result<Vec<(String, String, Vec<u8>)>, String> {
    let mut found = Vec::new();
    let mut total = 0u64;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(here) = pending.pop() {
        let entries = std::fs::read_dir(&here).map_err(|e| format!("{here:?}: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let full = entry.path();
            let relative = full
                .strip_prefix(dir)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if !valid_asset_path(&relative) {
                return Err(format!("{relative:?} is not a name a view's file may have"));
            }
            let Some(content_type) = content_type(&relative) else {
                return Err(format!("{relative} is not of a kind a view is served"));
            };
            let size = entry.metadata().map_err(|e| e.to_string())?.len();
            if size > MAX_FILE {
                return Err(format!("{relative} is larger than {MAX_FILE} bytes"));
            }
            total += size;
            let bytes = std::fs::read(&full).map_err(|e| format!("{relative}: {e}"))?;
            found.push((relative, content_type.to_string(), bytes));
        }
    }
    if found.len() > MAX_FILES || total > MAX_TOTAL {
        return Err(format!(
            "a plugin's views are at most {MAX_FILES} files and {MAX_TOTAL} bytes"
        ));
    }
    for (name, kind) in &manifest.channel_types {
        if !found.iter().any(|(path, _, _)| path == &kind.view) {
            return Err(format!(
                "channel kind {name}'s view, {}, is not among its assets",
                kind.view
            ));
        }
    }
    Ok(found)
}

/// Replaces the plugin's files with `files`, inside the caller's transaction.
pub async fn replace(
    conn: &mut AsyncPgConnection,
    plugin: &str,
    files: &[(String, String, Vec<u8>)],
) -> app::Result<()> {
    diesel::delete(plugin_asset::table.filter(plugin_asset::plugin.eq(plugin)))
        .execute(conn)
        .await?;
    for (path, content_type, bytes) in files {
        diesel::insert_into(plugin_asset::table)
            .values((
                plugin_asset::plugin.eq(plugin),
                plugin_asset::path.eq(path),
                plugin_asset::content_type.eq(content_type),
                plugin_asset::bytes.eq(bytes),
            ))
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// The plugin's files, by path.
pub async fn load(
    conn: &mut AsyncPgConnection,
    plugin: &str,
) -> app::Result<HashMap<String, Asset>> {
    let rows: Vec<(String, String, Vec<u8>)> = plugin_asset::table
        .select((
            plugin_asset::path,
            plugin_asset::content_type,
            plugin_asset::bytes,
        ))
        .filter(plugin_asset::plugin.eq(plugin))
        .load(conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(path, content_type, bytes)| {
            (
                path,
                Asset {
                    content_type,
                    bytes: Bytes::from(bytes),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_view_uses_is_served() {
        assert_eq!(content_type("board.html"), Some("text/html; charset=utf-8"));
        assert_eq!(
            content_type("app.JS"),
            Some("text/javascript; charset=utf-8")
        );
        assert_eq!(content_type("plugin.wasm"), None);
        assert_eq!(content_type("README"), None);
    }
}
