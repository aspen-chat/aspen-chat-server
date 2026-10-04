UPDATE deployment_role SET permissions = permissions & ~2048::BIGINT;
ALTER TABLE deployment_settings
    DROP CONSTRAINT deployment_settings_gates_need_domain,
    DROP CONSTRAINT deployment_settings_bots_shared_list,
    DROP CONSTRAINT deployment_settings_users_shared_list,
    DROP COLUMN bots_immigration_invite_required,
    DROP COLUMN bots_shared_list,
    DROP COLUMN bots_immigration,
    DROP COLUMN bots_emigration,
    DROP COLUMN users_immigration_invite_required,
    DROP COLUMN users_shared_list,
    DROP COLUMN users_immigration,
    DROP COLUMN users_emigration,
    DROP COLUMN file_transfers,
    DROP COLUMN custom_emoji_limit,
    DROP COLUMN everyone_mention_limit,
    DROP COLUMN bots_max_per_user,
    DROP COLUMN bots_enabled,
    DROP COLUMN require_two_factor,
    DROP COLUMN registration_invite_required,
    DROP COLUMN federation_domain,
    DROP COLUMN revision;
ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_settings_singleton_check TO deployment_profile_singleton_check;
ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_settings_icon_fkey TO deployment_profile_icon_fkey;
ALTER INDEX deployment_settings_pkey RENAME TO deployment_profile_pkey;
ALTER TABLE deployment_settings RENAME TO deployment_profile;
