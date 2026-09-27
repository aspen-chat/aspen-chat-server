// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "channel_type"))]
    pub struct ChannelType;

    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "message_kind"))]
    pub struct MessageKind;
}

diesel::table! {
    attachment (id) {
        id -> Uuid,
        mime_type -> Text,
        file_name -> Text,
        timestamp -> Timestamptz,
        storage_key -> Text,
        ready_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    category (id) {
        id -> Uuid,
        community -> Uuid,
        name -> Text,
        sort_index -> Int4,
        deleted_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ChannelType;

    channel (id) {
        id -> Uuid,
        community -> Nullable<Uuid>,
        parent_category -> Nullable<Uuid>,
        name -> Text,
        ty -> ChannelType,
        sort_index -> Int4,
        deleted_at -> Nullable<Timestamptz>,
        parent_channel -> Nullable<Uuid>,
        starter_message -> Nullable<Uuid>,
        reply_count -> Int4,
        last_reply_at -> Nullable<Timestamptz>,
        dm_key -> Nullable<Text>,
    }
}

diesel::table! {
    community (id) {
        id -> Uuid,
        name -> Text,
        icon -> Nullable<Uuid>,
        deleted_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    community_user (user, community) {
        user -> Uuid,
        community -> Uuid,
        sort_index -> Int4,
    }
}

diesel::table! {
    dm_recipient (channel, user) {
        channel -> Uuid,
        user -> Uuid,
        joined_at -> Timestamptz,
    }
}

diesel::table! {
    icon (id) {
        id -> Uuid,
        icon_mime_type -> Text,
        timestamp -> Timestamptz,
        storage_key -> Text,
        ready_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    invite (code) {
        code -> Text,
        community -> Uuid,
        created_by -> Uuid,
        created_at -> Timestamptz,
        expires_at -> Nullable<Timestamptz>,
        deleted_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MessageKind;

    message (id) {
        id -> Uuid,
        author -> Uuid,
        channel -> Uuid,
        content -> Text,
        timestamp -> Timestamptz,
        deleted_at -> Nullable<Timestamptz>,
        edited_at -> Nullable<Timestamptz>,
        kind -> MessageKind,
        poll -> Nullable<Uuid>,
        thread -> Nullable<Uuid>,
        echo_of -> Nullable<Uuid>,
    }
}

diesel::table! {
    message_attachment (message_id, attachment_id) {
        message_id -> Uuid,
        attachment_id -> Uuid,
    }
}

diesel::table! {
    message_link_preview (message_id, position) {
        message_id -> Uuid,
        position -> Int4,
        url -> Text,
        title -> Nullable<Text>,
        description -> Nullable<Text>,
        site_name -> Nullable<Text>,
        image_id -> Nullable<Uuid>,
        image_mime_type -> Nullable<Text>,
        theme_color -> Nullable<Text>,
        video_src -> Nullable<Text>,
        video_width -> Nullable<Int4>,
        video_height -> Nullable<Int4>,
    }
}

diesel::table! {
    other_server_auth_token (token) {
        token -> Text,
        expires -> Timestamp,
        user -> Uuid,
        domain -> Text,
    }
}

diesel::table! {
    passkey (id) {
        id -> Uuid,
        user -> Uuid,
        credential_id -> Bytea,
        credential -> Jsonb,
        name -> Text,
        created_at -> Timestamptz,
        last_used_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    pin (message_id) {
        message_id -> Uuid,
        channel -> Uuid,
        timestamp -> Timestamptz,
        sort_index -> Int4,
    }
}

diesel::table! {
    poll (id) {
        id -> Uuid,
        channel -> Uuid,
        created_by -> Uuid,
        question -> Text,
        multiple_choice -> Bool,
        anonymous -> Bool,
        created_at -> Timestamptz,
        closes_at -> Timestamptz,
        closed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    poll_option (poll, index) {
        poll -> Uuid,
        index -> Int4,
        label -> Text,
        emoji -> Nullable<Text>,
    }
}

diesel::table! {
    poll_vote (poll, option_index, user) {
        poll -> Uuid,
        option_index -> Int4,
        user -> Uuid,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    react (emoji, author, message) {
        emoji -> Text,
        author -> Uuid,
        message -> Uuid,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    recovery_code (user, code_hash) {
        user -> Uuid,
        code_hash -> Bytea,
        used_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    refresh_token (token) {
        token -> Text,
        expires -> Timestamp,
        user -> Uuid,
        verified_at -> Timestamptz,
    }
}

diesel::table! {
    session (token) {
        token -> Text,
        expires -> Timestamp,
        refresh_token -> Text,
    }
}

diesel::table! {
    totp_secret (user) {
        user -> Uuid,
        secret -> Bytea,
        created_at -> Timestamptz,
        confirmed_at -> Nullable<Timestamptz>,
        last_used_step -> Nullable<Int8>,
    }
}

diesel::table! {
    user (id) {
        id -> Uuid,
        name -> Text,
        password_hash -> Text,
        icon -> Nullable<Uuid>,
        created_at -> Timestamptz,
        last_seen_at -> Timestamptz,
        deleted_at -> Nullable<Timestamptz>,
        display_name -> Nullable<Text>,
        pronouns -> Nullable<Text>,
        bio -> Nullable<Text>,
        status_text -> Nullable<Text>,
        status_emoji -> Nullable<Text>,
    }
}

diesel::table! {
    user_preferences (user) {
        user -> Uuid,
        values -> Jsonb,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    voice_participant (session, user) {
        session -> Uuid,
        user -> Uuid,
        joined_at -> Timestamptz,
        muted -> Bool,
        deafened -> Bool,
        sharing_screen -> Bool,
    }
}

diesel::table! {
    voice_server (id) {
        id -> Uuid,
        name -> Text,
        url -> Text,
        capacity -> Int4,
        enabled -> Bool,
        created_at -> Timestamptz,
        last_report_at -> Nullable<Timestamptz>,
        reported_participants -> Int4,
    }
}

diesel::table! {
    voice_server_failure (voice_server, user) {
        voice_server -> Uuid,
        user -> Uuid,
        reported_at -> Timestamptz,
    }
}

diesel::table! {
    voice_session (id) {
        id -> Uuid,
        channel -> Uuid,
        voice_server -> Uuid,
        created_at -> Timestamptz,
        alone_since -> Nullable<Timestamptz>,
    }
}

diesel::joinable!(category -> community (community));
diesel::joinable!(channel -> category (parent_category));
diesel::joinable!(channel -> community (community));
diesel::joinable!(community_user -> community (community));
diesel::joinable!(community_user -> user (user));
diesel::joinable!(dm_recipient -> channel (channel));
diesel::joinable!(dm_recipient -> user (user));
diesel::joinable!(invite -> community (community));
diesel::joinable!(invite -> user (created_by));
diesel::joinable!(message -> poll (poll));
diesel::joinable!(message -> user (author));
diesel::joinable!(message_attachment -> attachment (attachment_id));
diesel::joinable!(message_attachment -> message (message_id));
diesel::joinable!(message_link_preview -> message (message_id));
diesel::joinable!(other_server_auth_token -> user (user));
diesel::joinable!(passkey -> user (user));
diesel::joinable!(pin -> channel (channel));
diesel::joinable!(pin -> message (message_id));
diesel::joinable!(poll -> channel (channel));
diesel::joinable!(poll -> user (created_by));
diesel::joinable!(poll_option -> poll (poll));
diesel::joinable!(poll_vote -> user (user));
diesel::joinable!(react -> message (message));
diesel::joinable!(react -> user (author));
diesel::joinable!(recovery_code -> user (user));
diesel::joinable!(refresh_token -> user (user));
diesel::joinable!(session -> refresh_token (refresh_token));
diesel::joinable!(totp_secret -> user (user));
diesel::joinable!(user_preferences -> user (user));
diesel::joinable!(voice_participant -> user (user));
diesel::joinable!(voice_participant -> voice_session (session));
diesel::joinable!(voice_server_failure -> user (user));
diesel::joinable!(voice_server_failure -> voice_server (voice_server));
diesel::joinable!(voice_session -> channel (channel));
diesel::joinable!(voice_session -> voice_server (voice_server));

diesel::allow_tables_to_appear_in_same_query!(
    attachment,
    category,
    channel,
    community,
    community_user,
    dm_recipient,
    icon,
    invite,
    message,
    message_attachment,
    message_link_preview,
    other_server_auth_token,
    passkey,
    pin,
    poll,
    poll_option,
    poll_vote,
    react,
    recovery_code,
    refresh_token,
    session,
    totp_secret,
    user,
    user_preferences,
    voice_participant,
    voice_server,
    voice_server_failure,
    voice_session,
);
