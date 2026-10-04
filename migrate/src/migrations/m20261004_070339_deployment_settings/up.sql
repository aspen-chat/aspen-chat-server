-- The deployment's settings: how it presents itself, the policies its administrators set, and
-- its federation gates. One row, always present; `singleton` is its key and can hold nothing
-- but true. `revision` counts changes, so a server told of one knows when its read has caught
-- up (`app::deployment_settings`). `federation_domain` is the domain the deployment first
-- served federation at, which never changes once set: other deployments pin its key there.
ALTER TABLE deployment_profile RENAME TO deployment_settings;
ALTER INDEX deployment_profile_pkey RENAME TO deployment_settings_pkey;
ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_profile_icon_fkey TO deployment_settings_icon_fkey;
ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_profile_singleton_check TO deployment_settings_singleton_check;
ALTER TABLE deployment_settings
    ADD COLUMN revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN federation_domain TEXT,
    ADD COLUMN registration_invite_required BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN require_two_factor BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN bots_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN bots_max_per_user INTEGER NOT NULL DEFAULT 25 CHECK (bots_max_per_user >= 0),
    ADD COLUMN everyone_mention_limit INTEGER NOT NULL DEFAULT 200
        CHECK (everyone_mention_limit >= 0),
    ADD COLUMN custom_emoji_limit INTEGER NOT NULL DEFAULT 1000 CHECK (custom_emoji_limit >= 0),
    ADD COLUMN file_transfers BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN users_emigration TEXT NOT NULL DEFAULT 'closed'
        CHECK (users_emigration IN ('closed', 'open', 'allowList', 'blockList')),
    ADD COLUMN users_immigration TEXT NOT NULL DEFAULT 'closed'
        CHECK (users_immigration IN ('closed', 'open', 'allowList', 'blockList')),
    ADD COLUMN users_shared_list BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN users_immigration_invite_required BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN bots_emigration TEXT NOT NULL DEFAULT 'closed'
        CHECK (bots_emigration IN ('closed', 'open', 'allowList', 'blockList')),
    ADD COLUMN bots_immigration TEXT NOT NULL DEFAULT 'closed'
        CHECK (bots_immigration IN ('closed', 'open', 'allowList', 'blockList')),
    ADD COLUMN bots_shared_list BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN bots_immigration_invite_required BOOLEAN NOT NULL DEFAULT FALSE,
    -- A shared list serves both directions, so both gates must read the same kind of list.
    ADD CONSTRAINT deployment_settings_users_shared_list CHECK (
        NOT users_shared_list
        OR (users_emigration IN ('allowList', 'blockList') AND users_emigration = users_immigration)
    ),
    ADD CONSTRAINT deployment_settings_bots_shared_list CHECK (
        NOT bots_shared_list
        OR (bots_emigration IN ('allowList', 'blockList') AND bots_emigration = bots_immigration)
    ),
    -- A gate opens only for a deployment with a domain to be known by.
    ADD CONSTRAINT deployment_settings_gates_need_domain CHECK (
        federation_domain IS NOT NULL
        OR (users_emigration = 'closed' AND users_immigration = 'closed'
            AND bots_emigration = 'closed' AND bots_immigration = 'closed')
    );

-- Manage deployment settings (bit 11) takes over the profile from Manage federation (bit 6), so
-- every role that could change the profile still can.
UPDATE deployment_role SET permissions = permissions | 2048 WHERE permissions & 64 <> 0;
