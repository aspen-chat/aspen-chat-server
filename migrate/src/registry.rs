//! This file is rewritten in place by `cargo run -p aspen-migrate -- new`,
//! which parses the current contents with `syn` and re-emits them via
//! `prettyplease`. `//`-style comments and blank lines here do not
//! survive that round-trip. Put per-migration documentation in each
//! migration's own `mod.rs`, where it sits next to the SQL it describes.
use crate::Migration;
use crate::migrations;
pub static MIGRATIONS: &[&dyn Migration] = &[
    &migrations::m20250503_010148_setup::M,
    &migrations::m20250503_021918_message_time::M,
    &migrations::m20250504_215759_add_password_hash::M,
    &migrations::m20260216_043215_sort_index::M,
    &migrations::m20260216_052527_uncaptured::M,
    &migrations::m20260216_212308_attachments::M,
    &migrations::m20260317_053302_message_timestamp::M,
    &migrations::m20260317_065719_message_attachments::M,
    &migrations::m20260323_235015_channel_type_enum::M,
    &migrations::m20260324_124808_message_attachment_relations::M,
    &migrations::m20260409_000000_pin_table::M,
    &migrations::m20260409_000001_soft_deletes::M,
    &migrations::m20260409_000002_media_storage_keys::M,
    &migrations::m20260409_000003_community_soft_deletes::M,
    &migrations::m20260409_000004_invite_table::M,
    &migrations::m20260417_200000_message_channel_id_index::M,
    &migrations::m20260420_000000_message_link_preview::M,
    &migrations::m20260502_002140_media_pending_uploads::M,
    &migrations::m20260925_214711_message_edited_at::M,
    &migrations::m20260925_232722_link_preview_video::M,
    &migrations::m20260925_235811_polls::M,
    &migrations::m20260926_002117_poll_option_emoji::M,
    &migrations::m20260926_014131_user_profile::M,
    &migrations::m20260926_032241_membership_sort_index::M,
    &migrations::m20260926_041014_voice::M,
    &migrations::m20260926_043002_voice_session_alone_since::M,
    &migrations::m20260926_063058_voice_participant_sharing_screen::M,
];
