-- One threshold decides when a community is large (`app::large_community`): it loses Mention
-- everyone from its everyone role, its channels tell members of tags rather than every
-- message, and its default channel announces both.
ALTER TABLE deployment_settings RENAME COLUMN everyone_mention_limit TO large_community_members;
ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_settings_everyone_mention_limit_check
    TO deployment_settings_large_community_members_check;

-- When the community gained the member that made it large; once set, it stays large however
-- its membership moves.
ALTER TABLE community RENAME COLUMN everyone_limited_at TO large_at;

-- The system message announcing that a community has become large, in its default channel.
ALTER TYPE message_kind ADD VALUE 'community_grew';

-- For a `community_grew` message, what changed (`app::large_community::CommunityGrowth`).
ALTER TABLE message ADD COLUMN growth JSONB;
