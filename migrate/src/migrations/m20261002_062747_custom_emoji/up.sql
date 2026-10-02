-- A community's own emoji: a small picture, named, used in its messages and as a reaction.
-- Names are unique within the community ignoring case, since a message box names one as
-- `:name:`. The picture is an icon row, which goes with the emoji.
CREATE TABLE custom_emoji (
    id UUID PRIMARY KEY,
    community UUID NOT NULL REFERENCES community (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    icon UUID NOT NULL REFERENCES icon (id),
    created_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX custom_emoji_name_key ON custom_emoji (community, lower(name));

-- A reaction with a custom emoji names it, and goes when the emoji does; its `emoji` text is
-- the emoji's reference, `<:id>`, as messages write it, so a message's reactions stay one set
-- keyed by that text.
ALTER TABLE react ADD COLUMN custom_emoji UUID REFERENCES custom_emoji (id) ON DELETE CASCADE;
CREATE INDEX react_by_custom_emoji ON react (custom_emoji) WHERE custom_emoji IS NOT NULL;

-- Manage custom emoji (1 << 12, 4096) goes to every role that holds Manage messages (1 << 8,
-- 256), which the Moderator and Admin templates give and the everyone role does not.
UPDATE community_role SET permissions = permissions | 4096 WHERE permissions & 256 = 256;
