//! This file is rewritten in place by `cargo run -p aspen-migrate -- new`,
//! which parses the current contents with `syn` and re-emits them via
//! `prettyplease`. `//`-style comments and blank lines here do not
//! survive that round-trip. Put per-migration documentation in each
//! migration's own `mod.rs`, where it sits next to the SQL it describes.
//! Per-migration modules. Each one declares a `pub static M: SqlMigration`
//! (or a custom `impl Migration` for code-style migrations); `registry.rs`
//! is the single ordered list that ties them together.
pub mod m20250503_010148_setup;
pub mod m20250503_021918_message_time;
pub mod m20250504_215759_add_password_hash;
pub mod m20260216_043215_sort_index;
pub mod m20260216_052527_uncaptured;
pub mod m20260216_212308_attachments;
pub mod m20260317_053302_message_timestamp;
pub mod m20260317_065719_message_attachments;
pub mod m20260323_235015_channel_type_enum;
pub mod m20260324_124808_message_attachment_relations;
pub mod m20260409_000000_pin_table;
pub mod m20260409_000001_soft_deletes;
pub mod m20260409_000002_media_storage_keys;
pub mod m20260409_000003_community_soft_deletes;
pub mod m20260409_000004_invite_table;
pub mod m20260417_200000_message_channel_id_index;
pub mod m20260420_000000_message_link_preview;
pub mod m20260502_002140_media_pending_uploads;
