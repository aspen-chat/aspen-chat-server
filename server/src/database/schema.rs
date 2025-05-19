// @generated automatically by Diesel CLI.

diesel::table! {
    category (id) {
        id -> Uuid,
        community -> Uuid,
        name -> Text,
    }
}

diesel::table! {
    channel (id) {
        id -> Uuid,
        community -> Nullable<Uuid>,
        parent_category -> Nullable<Uuid>,
        name -> Text,
        ty -> Int4,
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
    message (id) {
        id -> Uuid,
        author -> Uuid,
        channel -> Uuid,
        time -> Timestamp,
        content -> Text,
    }
}

diesel::table! {
    react (emoji, author, message) {
        emoji -> Text,
        author -> Uuid,
        message -> Uuid,
    }
}

diesel::table! {
    user (id) {
        id -> Uuid,
        name -> Text,
        password_hash -> Text,
        icon -> Nullable<Uuid>,
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
    refresh_token (token) {
        token -> Text,
        expires -> Timestamp,
        user -> Uuid,
    }
}

diesel::table! {
    icon (id) {
        id -> Uuid,
        data -> Bytea,
        icon_mime_type -> Text,
    }
}

diesel::joinable!(category -> community (community));
diesel::joinable!(channel -> category (parent_category));
diesel::joinable!(channel -> community (community));
diesel::joinable!(community_user -> community (community));
diesel::joinable!(community_user -> user (user));
diesel::joinable!(message -> channel (channel));
diesel::joinable!(message -> user (author));
diesel::joinable!(react -> message (message));
diesel::joinable!(react -> user (author));
diesel::joinable!(session -> refresh_token (refresh_token));
diesel::joinable!(refresh_token -> user (user));
diesel::joinable!(user -> icon (icon));
diesel::joinable!(community -> icon (icon));

diesel::allow_tables_to_appear_in_same_query!(
    category,
    channel,
    community,
    community_user,
    message,
    react,
    user,
);
