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
        width -> Nullable<Int4>,
        height -> Nullable<Int4>,
        uploader -> Nullable<Uuid>,
        description -> Nullable<Text>,
        preview_storage_key -> Nullable<Text>,
        preview_mime_type -> Nullable<Text>,
        preview_width -> Nullable<Int4>,
        preview_height -> Nullable<Int4>,
    }
}

diesel::table! {
    attachment_preview_job (attachment_id) {
        attachment_id -> Uuid,
        priority -> Int2,
        not_before -> Timestamptz,
        attempts -> Int4,
        hold_until -> Timestamptz,
    }
}

diesel::table! {
    benchmark_community (run, community) {
        run -> Text,
        community -> Uuid,
    }
}

diesel::table! {
    benchmark_run (run) {
        run -> Text,
        created_at -> Timestamptz,
        plan -> Jsonb,
    }
}

diesel::table! {
    benchmark_user (run, user) {
        run -> Text,
        user -> Uuid,
    }
}

diesel::table! {
    bot_command_list (bot) {
        bot -> Uuid,
        commands -> Jsonb,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    bot_token (bot) {
        bot -> Uuid,
        digest -> Bytea,
        created_at -> Timestamptz,
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
    category_collapse (user, category) {
        user -> Uuid,
        category -> Uuid,
    }
}

diesel::table! {
    category_override (category, role) {
        category -> Uuid,
        role -> Uuid,
        allow -> Int8,
        deny -> Int8,
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
        plugin_type -> Nullable<Text>,
    }
}

diesel::table! {
    channel_mute (user, channel) {
        user -> Uuid,
        channel -> Uuid,
        until -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    channel_override (channel, role) {
        channel -> Uuid,
        role -> Uuid,
        allow -> Int8,
        deny -> Int8,
    }
}

diesel::table! {
    community (id) {
        id -> Uuid,
        name -> Text,
        icon -> Nullable<Uuid>,
        deleted_at -> Nullable<Timestamptz>,
        owner -> Nullable<Uuid>,
        everyone_limited_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    community_ban (community, user) {
        community -> Uuid,
        user -> Uuid,
        banned_by -> Nullable<Uuid>,
        reason -> Nullable<Text>,
        banned_at -> Timestamptz,
        until -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    community_member_role (user, role) {
        user -> Uuid,
        community -> Uuid,
        role -> Uuid,
    }
}

diesel::table! {
    community_plugin (community, plugin) {
        community -> Uuid,
        plugin -> Text,
        enabled -> Bool,
        settings -> Jsonb,
        updated_at -> Timestamptz,
        updated_by -> Nullable<Uuid>,
    }
}

diesel::table! {
    community_role (id) {
        id -> Uuid,
        community -> Uuid,
        name -> Text,
        position -> Int4,
        permissions -> Int8,
        everyone -> Bool,
        bot -> Nullable<Uuid>,
        hue -> Nullable<Int2>,
        hoist -> Bool,
    }
}

diesel::table! {
    community_user (user, community) {
        user -> Uuid,
        community -> Uuid,
        sort_index -> Int4,
        joined_at -> Timestamptz,
        nickname -> Nullable<Text>,
    }
}

diesel::table! {
    custom_emoji (id) {
        id -> Uuid,
        community -> Uuid,
        name -> Text,
        icon -> Uuid,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    deployment_role (id) {
        id -> Uuid,
        name -> Text,
        position -> Int4,
        permissions -> Int8,
        hue -> Nullable<Int2>,
    }
}

diesel::table! {
    deployment_settings (singleton) {
        singleton -> Bool,
        display_name -> Nullable<Text>,
        icon -> Nullable<Uuid>,
        revision -> Int8,
        federation_domain -> Nullable<Text>,
        registration_invite_required -> Bool,
        require_two_factor -> Bool,
        bots_enabled -> Bool,
        bots_max_per_user -> Int4,
        everyone_mention_limit -> Int4,
        custom_emoji_limit -> Int4,
        file_transfers -> Bool,
        users_emigration -> Text,
        users_immigration -> Text,
        users_shared_list -> Bool,
        users_immigration_invite_required -> Bool,
        bots_emigration -> Text,
        bots_immigration -> Text,
        bots_shared_list -> Bool,
        bots_immigration_invite_required -> Bool,
        email_required -> Bool,
        email_verification_required -> Bool,
        newsletter_enabled -> Bool,
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
    email_outbox (id) {
        id -> Uuid,
        priority -> Int2,
        user -> Uuid,
        address -> Nullable<Text>,
        mail -> Jsonb,
        attempts -> Int4,
        not_before -> Timestamptz,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    federated_deployment (domain) {
        domain -> Text,
        origin -> Text,
        added_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
        note -> Nullable<Text>,
        public_key -> Nullable<Bytea>,
        first_contact_at -> Nullable<Timestamptz>,
        last_contact_at -> Nullable<Timestamptz>,
        offered_key -> Nullable<Bytea>,
        offered_key_at -> Nullable<Timestamptz>,
        protocol_version -> Nullable<Int4>,
        protocol_minimum -> Nullable<Int4>,
        capabilities -> Array<Nullable<Text>>,
        software_name -> Nullable<Text>,
        software_version -> Nullable<Text>,
    }
}

diesel::table! {
    federation_key (id) {
        id -> Uuid,
        private_key -> Bytea,
        public_key -> Bytea,
        created_at -> Timestamptz,
        retired_at -> Nullable<Timestamptz>,
        handover -> Nullable<Text>,
    }
}

diesel::table! {
    federation_list_entry (domain, list) {
        domain -> Text,
        list -> Text,
        added_by -> Nullable<Uuid>,
        added_at -> Timestamptz,
    }
}

diesel::table! {
    file_offer (id) {
        id -> Uuid,
        channel -> Nullable<Uuid>,
        sender -> Nullable<Uuid>,
        file_name -> Text,
        file_size -> Int8,
        allow_direct -> Bool,
        valid_for_seconds -> Int4,
        offered_at -> Timestamptz,
    }
}

diesel::table! {
    file_transfer (id) {
        id -> Uuid,
        offer -> Uuid,
        receiver -> Nullable<Uuid>,
        mode -> Text,
        started_at -> Timestamptz,
        ended_at -> Nullable<Timestamptz>,
        outcome -> Nullable<Text>,
        ended_by -> Nullable<Uuid>,
    }
}

diesel::table! {
    held_message (id) {
        id -> Uuid,
        author -> Uuid,
        channel -> Uuid,
        content -> Text,
        attachments -> Array<Nullable<Uuid>>,
        echo_to_parent -> Bool,
        locale -> Text,
        held_at -> Timestamptz,
        not_before -> Timestamptz,
        attempts -> Int4,
    }
}

diesel::table! {
    icon (id) {
        id -> Uuid,
        icon_mime_type -> Text,
        timestamp -> Timestamptz,
        storage_key -> Text,
        ready_at -> Nullable<Timestamptz>,
        uploaded_by -> Nullable<Uuid>,
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
    mention (id) {
        id -> Int8,
        message -> Uuid,
        channel -> Uuid,
        target_user -> Nullable<Uuid>,
        target_role -> Nullable<Uuid>,
        everyone -> Bool,
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
        mentions -> Jsonb,
        call_seconds -> Nullable<Int4>,
        command_bot -> Nullable<Uuid>,
        linked_messages -> Array<Nullable<Uuid>>,
        warning -> Nullable<Jsonb>,
        altered_by -> Array<Nullable<Text>>,
        card -> Nullable<Jsonb>,
    }
}

diesel::table! {
    message_annotation (id) {
        id -> Uuid,
        plugin -> Text,
        message -> Uuid,
        kind -> Text,
        severity -> Text,
        label -> Jsonb,
        detail -> Nullable<Jsonb>,
        link -> Nullable<Text>,
        updated_at -> Timestamptz,
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
        image_width -> Nullable<Int4>,
        image_height -> Nullable<Int4>,
    }
}

diesel::table! {
    moderation_log (id) {
        id -> Uuid,
        actor -> Nullable<Uuid>,
        action -> Text,
        community -> Nullable<Uuid>,
        channel -> Nullable<Uuid>,
        subject -> Nullable<Text>,
        at -> Timestamptz,
    }
}

diesel::table! {
    newsletter_post (id) {
        id -> Uuid,
        subject -> Text,
        body -> Text,
        author -> Nullable<Uuid>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        sent_at -> Nullable<Timestamptz>,
        sent_by -> Nullable<Uuid>,
        queued_through -> Nullable<Uuid>,
        queued_at -> Nullable<Timestamptz>,
        recipients -> Int8,
    }
}

diesel::table! {
    notification_setting (id) {
        id -> Uuid,
        user -> Uuid,
        community -> Nullable<Uuid>,
        channel -> Nullable<Uuid>,
        level -> Text,
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
    plugin (id) {
        id -> Text,
        version -> Text,
        manifest -> Jsonb,
        component -> Nullable<Bytea>,
        granted -> Array<Nullable<Text>>,
        mode -> Text,
        enabled -> Bool,
        position -> Int4,
        settings -> Jsonb,
        storage_bytes -> Int8,
        installed_at -> Timestamptz,
        updated_at -> Timestamptz,
        removed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    plugin_asset (plugin, path) {
        plugin -> Text,
        path -> Text,
        content_type -> Text,
        bytes -> Bytea,
    }
}

diesel::table! {
    plugin_capability (secret) {
        secret -> Text,
        plugin -> Text,
        user -> Uuid,
        name -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    plugin_notice (id) {
        id -> Uuid,
        plugin -> Text,
        user -> Uuid,
        channel -> Uuid,
        message -> Nullable<Uuid>,
        text -> Jsonb,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    plugin_storage (plugin, scope_kind, scope, key) {
        plugin -> Text,
        scope_kind -> Text,
        scope -> Uuid,
        key -> Text,
        value -> Bytea,
    }
}

diesel::table! {
    plugin_timer (plugin, key) {
        plugin -> Text,
        key -> Text,
        due -> Timestamptz,
        payload -> Text,
        attempts -> Int4,
        claimed_until -> Nullable<Timestamptz>,
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
        allow_write_ins -> Bool,
    }
}

diesel::table! {
    poll_option (poll, index) {
        poll -> Uuid,
        index -> Int4,
        label -> Text,
        emoji -> Nullable<Text>,
        write_in -> Bool,
        written_by -> Nullable<Uuid>,
        removed_at -> Nullable<Timestamptz>,
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
    push_key (id) {
        id -> Uuid,
        private_key -> Bytea,
        public_key -> Bytea,
        created_at -> Timestamptz,
        retired_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    push_subscription (id) {
        id -> Uuid,
        user -> Uuid,
        refresh_token -> Text,
        endpoint -> Text,
        p256dh -> Bytea,
        auth -> Bytea,
        push_key -> Uuid,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    react (emoji, author, message) {
        emoji -> Text,
        author -> Uuid,
        message -> Uuid,
        timestamp -> Timestamptz,
        custom_emoji -> Nullable<Uuid>,
    }
}

diesel::table! {
    read_state (user, channel) {
        user -> Uuid,
        channel -> Uuid,
        message -> Uuid,
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
        method -> Text,
    }
}

diesel::table! {
    registration_invite (code) {
        code -> Text,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
        expires_at -> Nullable<Timestamptz>,
        max_uses -> Int4,
        uses -> Int4,
        revoked_at -> Nullable<Timestamptz>,
        note -> Nullable<Text>,
        used_up_at -> Nullable<Timestamptz>,
        community_invite -> Nullable<Text>,
    }
}

diesel::table! {
    report (id) {
        id -> Uuid,
        case -> Uuid,
        reporter -> Uuid,
        category -> Uuid,
        explanation -> Nullable<Text>,
        aspects -> Array<Nullable<Text>>,
        profile -> Nullable<Jsonb>,
        created_at -> Timestamptz,
        nickname -> Nullable<Text>,
    }
}

diesel::table! {
    report_case (id) {
        id -> Uuid,
        kind -> Text,
        subject -> Uuid,
        message -> Nullable<Uuid>,
        status -> Text,
        opened_at -> Timestamptz,
        last_reported_at -> Timestamptz,
        closed_at -> Nullable<Timestamptz>,
        closed_by -> Nullable<Uuid>,
        resolution -> Nullable<Jsonb>,
        community -> Nullable<Uuid>,
    }
}

diesel::table! {
    report_category (id) {
        id -> Uuid,
        builtin -> Nullable<Text>,
        name -> Nullable<Text>,
        description -> Nullable<Text>,
        position -> Int4,
        hidden -> Bool,
        created_at -> Timestamptz,
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
        registered_with -> Nullable<Text>,
        bot -> Bool,
        bot_owner -> Nullable<Uuid>,
        bot_public -> Bool,
        home_domain -> Nullable<Text>,
        home_id -> Nullable<Uuid>,
        home_icon -> Nullable<Uuid>,
        home_confirmed_at -> Nullable<Timestamptz>,
        banned_at -> Nullable<Timestamptz>,
        banned_by -> Nullable<Uuid>,
        system -> Bool,
        ban_reason -> Nullable<Text>,
        banned_until -> Nullable<Timestamptz>,
        name_hue -> Nullable<Int2>,
        plugin -> Nullable<Text>,
        public_email -> Nullable<Text>,
    }
}

diesel::table! {
    user_annotation (id) {
        id -> Uuid,
        plugin -> Text,
        user -> Uuid,
        kind -> Text,
        severity -> Text,
        label -> Jsonb,
        detail -> Nullable<Jsonb>,
        link -> Nullable<Text>,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    user_block (blocker, blocked) {
        blocker -> Uuid,
        blocked -> Uuid,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    user_deployment_role (user, role) {
        user -> Uuid,
        role -> Uuid,
    }
}

diesel::table! {
    user_email (user) {
        user -> Uuid,
        address -> Text,
        verified_at -> Nullable<Timestamptz>,
        shown -> Bool,
        newsletter -> Bool,
        digest -> Bool,
        digest_time_zone -> Text,
        digest_hour -> Int2,
        digest_next_at -> Nullable<Timestamptz>,
        digest_since -> Nullable<Timestamptz>,
        locale -> Text,
        unsubscribe_token -> Text,
    }
}

diesel::table! {
    user_foreign_deployment (user, domain) {
        user -> Uuid,
        domain -> Text,
        first_used_at -> Timestamptz,
        last_used_at -> Timestamptz,
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
    voice_ring (session, user) {
        session -> Uuid,
        user -> Uuid,
        caller -> Uuid,
        rung_at -> Timestamptz,
        until -> Timestamptz,
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
        started_by -> Nullable<Uuid>,
        had_company -> Bool,
    }
}

diesel::joinable!(attachment -> user (uploader));
diesel::joinable!(attachment_preview_job -> attachment (attachment_id));
diesel::joinable!(benchmark_community -> benchmark_run (run));
diesel::joinable!(benchmark_community -> community (community));
diesel::joinable!(benchmark_user -> benchmark_run (run));
diesel::joinable!(benchmark_user -> user (user));
diesel::joinable!(bot_command_list -> user (bot));
diesel::joinable!(bot_token -> user (bot));
diesel::joinable!(category -> community (community));
diesel::joinable!(category_collapse -> category (category));
diesel::joinable!(category_collapse -> user (user));
diesel::joinable!(category_override -> category (category));
diesel::joinable!(category_override -> community_role (role));
diesel::joinable!(channel -> category (parent_category));
diesel::joinable!(channel -> community (community));
diesel::joinable!(channel_mute -> channel (channel));
diesel::joinable!(channel_mute -> user (user));
diesel::joinable!(channel_override -> channel (channel));
diesel::joinable!(channel_override -> community_role (role));
diesel::joinable!(community -> user (owner));
diesel::joinable!(community_ban -> community (community));
diesel::joinable!(community_member_role -> community_role (role));
diesel::joinable!(community_plugin -> community (community));
diesel::joinable!(community_plugin -> plugin (plugin));
diesel::joinable!(community_plugin -> user (updated_by));
diesel::joinable!(community_role -> community (community));
diesel::joinable!(community_role -> user (bot));
diesel::joinable!(community_user -> community (community));
diesel::joinable!(community_user -> user (user));
diesel::joinable!(custom_emoji -> community (community));
diesel::joinable!(custom_emoji -> icon (icon));
diesel::joinable!(custom_emoji -> user (created_by));
diesel::joinable!(deployment_settings -> icon (icon));
diesel::joinable!(dm_recipient -> channel (channel));
diesel::joinable!(dm_recipient -> user (user));
diesel::joinable!(email_outbox -> user (user));
diesel::joinable!(federated_deployment -> user (added_by));
diesel::joinable!(federation_list_entry -> federated_deployment (domain));
diesel::joinable!(federation_list_entry -> user (added_by));
diesel::joinable!(file_offer -> channel (channel));
diesel::joinable!(file_offer -> user (sender));
diesel::joinable!(file_transfer -> file_offer (offer));
diesel::joinable!(held_message -> channel (channel));
diesel::joinable!(held_message -> user (author));
diesel::joinable!(icon -> user (uploaded_by));
diesel::joinable!(invite -> community (community));
diesel::joinable!(invite -> user (created_by));
diesel::joinable!(mention -> channel (channel));
diesel::joinable!(mention -> community_role (target_role));
diesel::joinable!(mention -> message (message));
diesel::joinable!(mention -> user (target_user));
diesel::joinable!(message -> poll (poll));
diesel::joinable!(message_annotation -> message (message));
diesel::joinable!(message_annotation -> plugin (plugin));
diesel::joinable!(message_attachment -> attachment (attachment_id));
diesel::joinable!(message_attachment -> message (message_id));
diesel::joinable!(message_link_preview -> message (message_id));
diesel::joinable!(moderation_log -> channel (channel));
diesel::joinable!(moderation_log -> community (community));
diesel::joinable!(moderation_log -> user (actor));
diesel::joinable!(notification_setting -> channel (channel));
diesel::joinable!(notification_setting -> community (community));
diesel::joinable!(notification_setting -> user (user));
diesel::joinable!(passkey -> user (user));
diesel::joinable!(pin -> channel (channel));
diesel::joinable!(pin -> message (message_id));
diesel::joinable!(plugin_asset -> plugin (plugin));
diesel::joinable!(plugin_capability -> plugin (plugin));
diesel::joinable!(plugin_capability -> user (user));
diesel::joinable!(plugin_notice -> channel (channel));
diesel::joinable!(plugin_notice -> message (message));
diesel::joinable!(plugin_notice -> plugin (plugin));
diesel::joinable!(plugin_notice -> user (user));
diesel::joinable!(plugin_storage -> plugin (plugin));
diesel::joinable!(plugin_timer -> plugin (plugin));
diesel::joinable!(poll -> channel (channel));
diesel::joinable!(poll -> user (created_by));
diesel::joinable!(poll_option -> poll (poll));
diesel::joinable!(poll_option -> user (written_by));
diesel::joinable!(poll_vote -> user (user));
diesel::joinable!(push_subscription -> push_key (push_key));
diesel::joinable!(push_subscription -> refresh_token (refresh_token));
diesel::joinable!(push_subscription -> user (user));
diesel::joinable!(react -> custom_emoji (custom_emoji));
diesel::joinable!(react -> message (message));
diesel::joinable!(react -> user (author));
diesel::joinable!(read_state -> channel (channel));
diesel::joinable!(read_state -> user (user));
diesel::joinable!(recovery_code -> user (user));
diesel::joinable!(refresh_token -> user (user));
diesel::joinable!(registration_invite -> invite (community_invite));
diesel::joinable!(report -> report_case (case));
diesel::joinable!(report -> report_category (category));
diesel::joinable!(report -> user (reporter));
diesel::joinable!(report_case -> community (community));
diesel::joinable!(report_case -> message (message));
diesel::joinable!(session -> refresh_token (refresh_token));
diesel::joinable!(totp_secret -> user (user));
diesel::joinable!(user -> plugin (plugin));
diesel::joinable!(user_annotation -> plugin (plugin));
diesel::joinable!(user_annotation -> user (user));
diesel::joinable!(user_deployment_role -> deployment_role (role));
diesel::joinable!(user_deployment_role -> user (user));
diesel::joinable!(user_email -> user (user));
diesel::joinable!(user_foreign_deployment -> user (user));
diesel::joinable!(user_preferences -> user (user));
diesel::joinable!(voice_participant -> user (user));
diesel::joinable!(voice_participant -> voice_session (session));
diesel::joinable!(voice_ring -> voice_session (session));
diesel::joinable!(voice_server_failure -> user (user));
diesel::joinable!(voice_server_failure -> voice_server (voice_server));
diesel::joinable!(voice_session -> channel (channel));
diesel::joinable!(voice_session -> user (started_by));
diesel::joinable!(voice_session -> voice_server (voice_server));

diesel::allow_tables_to_appear_in_same_query!(
    attachment,
    attachment_preview_job,
    benchmark_community,
    benchmark_run,
    benchmark_user,
    bot_command_list,
    bot_token,
    category,
    category_collapse,
    category_override,
    channel,
    channel_mute,
    channel_override,
    community,
    community_ban,
    community_member_role,
    community_plugin,
    community_role,
    community_user,
    custom_emoji,
    deployment_role,
    deployment_settings,
    dm_recipient,
    email_outbox,
    federated_deployment,
    federation_key,
    federation_list_entry,
    file_offer,
    file_transfer,
    held_message,
    icon,
    invite,
    mention,
    message,
    message_annotation,
    message_attachment,
    message_link_preview,
    moderation_log,
    newsletter_post,
    notification_setting,
    passkey,
    pin,
    plugin,
    plugin_asset,
    plugin_capability,
    plugin_notice,
    plugin_storage,
    plugin_timer,
    poll,
    poll_option,
    poll_vote,
    push_key,
    push_subscription,
    react,
    read_state,
    recovery_code,
    refresh_token,
    registration_invite,
    report,
    report_case,
    report_category,
    session,
    totp_secret,
    user,
    user_annotation,
    user_block,
    user_deployment_role,
    user_email,
    user_foreign_deployment,
    user_preferences,
    voice_participant,
    voice_ring,
    voice_server,
    voice_server_failure,
    voice_session,
);
