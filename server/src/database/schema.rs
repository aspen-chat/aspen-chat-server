// @generated automatically by Diesel CLI.

diesel::table! {
    attachment (id) {
        id -> Uuid,
        mime_type -> Text,
        file_name -> Text,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    category (id) {
        id -> Uuid,
        community -> Uuid,
        name -> Text,
        sort_index -> Int4,
    }
}

diesel::table! {
    channel (id) {
        id -> Uuid,
        community -> Nullable<Uuid>,
        parent_category -> Nullable<Uuid>,
        name -> Text,
        ty -> Int4,
        sort_index -> Int4,
    }
}

diesel::table! {
    community (id) {
        id -> Uuid,
        name -> Text,
        icon -> Nullable<Uuid>,
    }
}

diesel::table! {
    community_user (user, community) {
        user -> Uuid,
        community -> Uuid,
    }
}

diesel::table! {
    icon (id) {
        id -> Uuid,
        data -> Bytea,
        icon_mime_type -> Text,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    message (id) {
        id -> Uuid,
        author -> Uuid,
        channel -> Uuid,
        content -> Text,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    message_attachment (message_id, attachment_id) {
        message_id -> Uuid,
        attachment_id -> Uuid,
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
    react (emoji, author, message) {
        emoji -> Text,
        author -> Uuid,
        message -> Uuid,
        timestamp -> Timestamptz,
    }
}

diesel::table! {
    refresh_token (token) {
        token -> Text,
        expires -> Timestamp,
        user -> Uuid,
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
    user (id) {
        id -> Uuid,
        name -> Text,
        password_hash -> Text,
        icon -> Nullable<Uuid>,
        created_at -> Timestamptz,
        last_seen_at -> Timestamptz,
    }
}

diesel::joinable!(category -> community (community));
diesel::joinable!(channel -> category (parent_category));
diesel::joinable!(channel -> community (community));
diesel::joinable!(community_user -> community (community));
diesel::joinable!(community_user -> user (user));
diesel::joinable!(message -> channel (channel));
diesel::joinable!(message -> user (author));
diesel::joinable!(other_server_auth_token -> user (user));
diesel::joinable!(react -> message (message));
diesel::joinable!(react -> user (author));
diesel::joinable!(refresh_token -> user (user));
diesel::joinable!(session -> refresh_token (refresh_token));

diesel::allow_tables_to_appear_in_same_query!(
    attachment,
    category,
    channel,
    community,
    community_user,
    icon,
    message,
    message_attachment,
    other_server_auth_token,
    react,
    refresh_token,
    session,
    user,
);
