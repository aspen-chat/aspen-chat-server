//! Server-side link preview generation.
//!
//! Aspen is the authoritative source of link-preview metadata and thumbnails
//! for every message body: URL extraction, outbound HTTP, HTML parsing, and
//! image storage all happen here so the user's IP is never exposed to a
//! third-party origin just by rendering a chat message, and so every
//! connected client (across platforms) paints the same card from the same
//! metadata.
//!
//! 1. [`extract_preview_urls`] walks a markdown body with `pulldown-cmark`,
//!    collecting explicit link destinations plus bare `http(s)://` autolinks
//!    from text events (skipping anything inside code spans / code blocks),
//!    dedupes, and caps at [`MAX_LINK_PREVIEWS_PER_MESSAGE`].
//! 2. [`spawn_preview_fetch`] fires off a `tokio::spawn` background task that
//!    (a) fetches each URL's HTML with `reqwest` under strict byte / time
//!    limits, (b) parses Open Graph / Twitter Card / `<title>` / `<meta
//!    name="description">` / `<meta name="theme-color">` via `html5ever`'s
//!    tokenizer, (c) downloads the referenced `og:image` and uploads the
//!    bytes to the S3 media store, (d) writes a fresh set of
//!    `message_link_preview` rows, and (e) broadcasts a message `Update`
//!    carrying the new `link_previews` so connected clients can swap the
//!    empty-preview card stack on the message for the populated one without
//!    reloading the channel.
//! 3. [`load_previews`] batches preview rows back out for REST reads,
//!    templating each row's `image_id` into a public download URL via
//!    [`MediaStore::public_url`].
//! 4. [`delete_images_for_message`] tears down the S3 objects for a message
//!    before `delete_message` / the content-edit refetch path lets the row
//!    itself go away, so we don't leak image blobs.
//!
//! Preview thumbnails are downloaded by clients directly from the
//! anonymous-read endpoint behind [`crate::app::media_store::MediaStore::public_url`];
//! the API never serves the bytes itself.
//!
//! The metadata side of (b) goes through a process-local LRU cache keyed by
//! URL so repeated mentions of the same link don't hammer the third-party
//! origin. Image bytes are intentionally *not* cached — every materialised
//! preview row owns a fresh S3 object, which keeps the delete lifecycle
//! trivial (no ref-counting, no orphan sweeps).

mod fetch;
mod html_meta;
mod urls;
mod video;

pub use urls::{extract_preview_urls, extract_urls};

use crate::api::link_preview::{LinkPreview, VideoEmbed, image_storage_key};
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::app::context::GlobalServerContext;
use crate::app::media_store::MediaStore;
use crate::app::{self, LinkPreviewImageId, MessageId};
use crate::database::schema::message_link_preview;
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable};
use diesel_async::AsyncPgConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use fetch::{fetch_metadata, http_client};
use futures_util::stream::StreamExt;
use html_meta::ParsedMetadata;
use std::collections::HashMap;
use tracing::warn;
use url::Url;

/// Hard ceiling on how many preview cards a single message can carry.
///
/// Three is enough for a message that genuinely shares a handful of related
/// links (a comparison, a list of references) while keeping the per-message
/// outbound-fetch cost bounded. The limit is enforced at extraction time in
/// [`extract_preview_urls`], so downstream code — including the DB insert
/// and the client render path — can assume preview lists are already
/// small and free of duplicates.
pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

/// Maximum number of image bytes we'll read from any single preview image.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

// ---------------------------------------------------------------------------
// DB row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = message_link_preview)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LinkPreviewRow {
    pub message_id: MessageId,
    #[allow(dead_code)] // used only for the ORDER BY column selection
    pub position: i32,
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_id: Option<LinkPreviewImageId>,
    // Stored on the row so an operator inspecting the database can match a
    // thumbnail back to its origin content type without round-tripping S3
    // metadata. Not surfaced on the wire DTO: clients learn the type from
    // the `Content-Type` header on the public download.
    #[allow(dead_code)]
    pub image_mime_type: Option<String>,
    pub theme_color: Option<String>,
    pub video_src: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub image_width: Option<i32>,
    pub image_height: Option<i32>,
}

impl LinkPreviewRow {
    fn into_wire(self, media_store: &MediaStore) -> LinkPreview {
        let image_url = self
            .image_id
            .map(|id| media_store.public_url(&image_storage_key(id)));
        let video = match (self.video_src, self.video_width, self.video_height) {
            (Some(src), Some(width), Some(height)) => Some(VideoEmbed {
                src,
                width: width.unsigned_abs(),
                height: height.unsigned_abs(),
            }),
            _ => None,
        };
        LinkPreview {
            url: self.url,
            title: self.title,
            description: self.description,
            site_name: self.site_name,
            image_url,
            image_width: self.image_width.map(i32::unsigned_abs),
            image_height: self.image_height.map(i32::unsigned_abs),
            theme_color: self.theme_color,
            video,
        }
    }
}

/// In-memory representation of a single fully-fetched preview, used to hand
/// the metadata + image-id pair from the concurrent fetchers into the DB
/// commit step.
struct Materialised {
    url: String,
    metadata: ParsedMetadata,
    image: Option<PreviewImage>,
}

/// A preview picture as stored: its id, its type, and its size in pixels when its header says.
struct PreviewImage {
    id: LinkPreviewImageId,
    mime_type: String,
    size: Option<(i32, i32)>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = message_link_preview)]
struct NewLinkPreviewRow<'a> {
    message_id: MessageId,
    position: i32,
    url: &'a str,
    title: Option<&'a str>,
    description: Option<&'a str>,
    site_name: Option<&'a str>,
    image_id: Option<LinkPreviewImageId>,
    image_mime_type: Option<&'a str>,
    theme_color: Option<&'a str>,
    video_src: Option<&'a str>,
    video_width: Option<i32>,
    video_height: Option<i32>,
    image_width: Option<i32>,
    image_height: Option<i32>,
}

// ---------------------------------------------------------------------------
// Preview images
// ---------------------------------------------------------------------------

/// Download a preview image and push it through the media store.
///
/// Failures are intentionally swallowed (logged at `warn`): the text preview
/// is still worth showing without a thumbnail, and the cost of aborting the
/// whole preview because one image didn't come back in time would be strictly
/// worse.
async fn fetch_and_store_image(
    state: &GlobalServerContext,
    image_url: &str,
) -> Option<PreviewImage> {
    let response = match http_client().get(image_url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                url = image_url,
                error = e.to_string(),
                "preview image fetch failed"
            );
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !mime_type.starts_with("image/") {
        return None;
    }
    // Store just the `image/foo` portion — strip any `; charset=...` tail.
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or(&mime_type)
        .trim()
        .to_owned();
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return None;
        };
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            // Image too big; don't bother uploading a partial.
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return None;
    }
    // Its size, read from its header, lets readers make room for it before it loads; a
    // picture whose header says nothing sensible simply has none.
    let size = imagesize::blob_size(&bytes).ok().and_then(|size| {
        let width = i32::try_from(size.width).ok().filter(|w| *w > 0)?;
        let height = i32::try_from(size.height).ok().filter(|h| *h > 0)?;
        Some((width, height))
    });
    let id = LinkPreviewImageId::new();
    let key = image_storage_key(id);
    if let Err(e) = state.media_store.put_bytes(&key, bytes, &mime_type).await {
        warn!(
            error = e.to_string(),
            url = image_url,
            "failed to upload preview image to media store"
        );
        return None;
    }
    Some(PreviewImage {
        id,
        mime_type,
        size,
    })
}

// ---------------------------------------------------------------------------
// Entry points for the rest of the app
// ---------------------------------------------------------------------------

/// Spawn a background task to materialise previews for a message.
///
/// This is fire-and-forget: callers publish the `Create` event with an empty
/// `link_previews` list and immediately return to the client. When the task
/// finishes it (atomically) replaces the existing preview rows and publishes
/// a message `Update` carrying the new `link_previews` so subscribers can
/// update their rendered messages in place.
pub fn spawn_preview_fetch(state: GlobalServerContext, message_id: MessageId, content: String) {
    tokio::spawn(async move {
        if let Err(e) = run_preview_fetch(&state, message_id, &content).await {
            warn!(
                message_id = message_id.0.to_string(),
                error = e.to_string(),
                "link preview background task failed"
            );
        }
    });
}

async fn run_preview_fetch(
    state: &GlobalServerContext,
    message_id: MessageId,
    content: &str,
) -> app::Result<()> {
    let urls = extract_preview_urls(content);
    if urls.is_empty() {
        // Nothing to do; any previously-attached previews (e.g. from a prior
        // version of this message) have already been cleared by the edit
        // path, and the caller has already published the "empty previews"
        // event on the transition that got us here.
        return Ok(());
    }

    // Fetch metadata for each URL concurrently.
    let metadata_results: Vec<(Url, Option<ParsedMetadata>)> =
        futures_util::future::join_all(urls.into_iter().map(|url| async move {
            let meta = fetch_metadata(&url).await;
            (url, meta)
        }))
        .await;

    // Download images in parallel too. Each preview owns its image, so we
    // mint a fresh `LinkPreviewImageId` per row.
    let with_images = futures_util::future::join_all(metadata_results.into_iter().map(
        |(url, metadata)| async move {
            let metadata = metadata?;
            let image = match &metadata.image_url {
                Some(image_url) => fetch_and_store_image(state, image_url).await,
                None => None,
            };
            Some(Materialised {
                url: url.to_string(),
                metadata,
                image,
            })
        },
    ))
    .await;
    let materialised: Vec<Materialised> = with_images.into_iter().flatten().collect();
    let new_image_ids: Vec<LinkPreviewImageId> = materialised
        .iter()
        .filter_map(|m| m.image.as_ref().map(|image| image.id))
        .collect();

    let media_store = state.media_store.as_ref();
    let wire_previews: Vec<LinkPreview> = materialised
        .iter()
        .map(|m| LinkPreview {
            url: m.url.clone(),
            title: m.metadata.title.clone(),
            description: m.metadata.description.clone(),
            site_name: m.metadata.site_name.clone(),
            image_url: m
                .image
                .as_ref()
                .map(|image| media_store.public_url(&image_storage_key(image.id))),
            image_width: m
                .image
                .as_ref()
                .and_then(|image| image.size)
                .map(|(w, _)| w.unsigned_abs()),
            image_height: m
                .image
                .as_ref()
                .and_then(|image| image.size)
                .map(|(_, h)| h.unsigned_abs()),
            theme_color: m.metadata.theme_color.clone(),
            video: m.metadata.video.clone(),
        })
        .collect();

    let mut conn = state.connection_pool.get().await?;
    let txn_result: app::Result<Vec<LinkPreviewImageId>> = conn
        .transaction::<_, app::Error, _>(|conn| {
            let wire_previews = wire_previews.clone();
            let materialised_ref = &materialised;
            async move {
                // Collect the image ids currently attached to this message so
                // we can drop them from S3 after the new generation commits.
                let stale_ids: Vec<Option<LinkPreviewImageId>> = message_link_preview::table
                    .select(message_link_preview::image_id)
                    .filter(message_link_preview::message_id.eq(message_id))
                    .load(conn.as_mut())
                    .await?;
                diesel::delete(message_link_preview::table)
                    .filter(message_link_preview::message_id.eq(message_id))
                    .execute(conn.as_mut())
                    .await?;
                for (i, m) in materialised_ref.iter().enumerate() {
                    let image_id = m.image.as_ref().map(|image| image.id);
                    let image_mime_type = m.image.as_ref().map(|image| image.mime_type.as_str());
                    let image_size = m.image.as_ref().and_then(|image| image.size);
                    let row = NewLinkPreviewRow {
                        message_id,
                        position: i as i32,
                        url: m.url.as_str(),
                        title: m.metadata.title.as_deref(),
                        description: m.metadata.description.as_deref(),
                        site_name: m.metadata.site_name.as_deref(),
                        image_id,
                        image_mime_type,
                        theme_color: m.metadata.theme_color.as_deref(),
                        video_src: m.metadata.video.as_ref().map(|v| v.src.as_str()),
                        video_width: m
                            .metadata
                            .video
                            .as_ref()
                            .and_then(|v| i32::try_from(v.width).ok()),
                        video_height: m
                            .metadata
                            .video
                            .as_ref()
                            .and_then(|v| i32::try_from(v.height).ok()),
                        image_width: image_size.map(|(w, _)| w),
                        image_height: image_size.map(|(_, h)| h),
                    };
                    diesel::insert_into(message_link_preview::table)
                        .values(&row)
                        .execute(conn.as_mut())
                        .await?;
                }
                // NATS-then-commit: publish before the transaction commits so
                // a transient NATS failure rolls back the row writes too.
                // This mirrors the ordering rule in AGENTS.md.
                let event = ServerEvent::Message(MessageEvent::Update {
                    id: message_id,
                    content: None,
                    attachments: None,
                    edited_at: None,
                    link_previews: Some(wire_previews),
                    thread: None,
                    mentions: None,
                    linked_messages: None,
                    altered_by: None,
                });
                app::publish_event(
                    state,
                    conn.as_mut(),
                    app::EventScope::Message(message_id),
                    &event,
                )
                .await?;
                Ok(stale_ids.into_iter().flatten().collect::<Vec<_>>())
            }
            .scope_boxed()
        })
        .await;

    match txn_result {
        Ok(stale_image_ids) => {
            // Best-effort delete of the old generation's S3 objects. If the
            // deletion fails we leak an orphan; a future sweep job can clean
            // them up — same trade-off as `attachment::delete_attachment`.
            for id in stale_image_ids {
                if let Err(e) = state.media_store.delete(&image_storage_key(id)).await {
                    warn!(
                        error = e.to_string(),
                        id = id.0.to_string(),
                        "failed to delete stale preview image from media store"
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            // Transaction failed (message was hard-deleted between the send
            // and now, NATS publish failed, etc). Drop any S3 uploads we
            // made so we don't leak orphaned preview bytes.
            for id in new_image_ids {
                if let Err(del_err) = state.media_store.delete(&image_storage_key(id)).await {
                    warn!(
                        error = del_err.to_string(),
                        id = id.0.to_string(),
                        "failed to clean up orphaned preview image after commit failure"
                    );
                }
            }
            Err(e)
        }
    }
}

/// Load all previews for `message_ids` in a single query and bucket them by
/// message id preserving the on-disk `position` order.
///
/// The `media_store` argument is the same one threaded through the rest of
/// the app; it's used here to template each row's `image_id` into a public
/// download URL so the wire DTO is what clients actually paint.
pub async fn load_previews(
    conn: &mut AsyncPgConnection,
    media_store: &MediaStore,
    message_ids: &[MessageId],
) -> app::Result<HashMap<MessageId, Vec<LinkPreview>>> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<LinkPreviewRow> = message_link_preview::table
        .select(<LinkPreviewRow as diesel::SelectableHelper<
            diesel::pg::Pg,
        >>::as_select())
        .filter(message_link_preview::message_id.eq_any(message_ids))
        .order_by((
            message_link_preview::message_id.asc(),
            message_link_preview::position.asc(),
        ))
        .load(conn)
        .await?;
    let mut out: HashMap<MessageId, Vec<LinkPreview>> = HashMap::new();
    for row in rows {
        out.entry(row.message_id)
            .or_default()
            .push(row.into_wire(media_store));
    }
    Ok(out)
}

/// Delete S3 objects for all previews attached to a message, then remove the
/// rows themselves.
///
/// Invoked by `delete_message` (before the soft-delete so the FK cascade
/// doesn't race us to the rows) and by the content-edit refetch path. The
/// media-store delete is best-effort; a transient S3 failure is logged and
/// swallowed, matching the pattern used in `attachment::delete_attachment`.
pub async fn delete_images_for_message(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    message_id: MessageId,
) -> app::Result<()> {
    let image_ids: Vec<Option<LinkPreviewImageId>> = message_link_preview::table
        .select(message_link_preview::image_id)
        .filter(message_link_preview::message_id.eq(message_id))
        .load(conn)
        .await?;
    diesel::delete(message_link_preview::table)
        .filter(message_link_preview::message_id.eq(message_id))
        .execute(conn)
        .await?;
    for id in image_ids.into_iter().flatten() {
        if let Err(e) = state.media_store.delete(&image_storage_key(id)).await {
            warn!(
                error = e.to_string(),
                id = id.0.to_string(),
                "failed to delete link preview image from media store"
            );
        }
    }
    Ok(())
}
