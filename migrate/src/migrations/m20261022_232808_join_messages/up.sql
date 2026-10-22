-- The system message announcing that someone joined a community.
ALTER TYPE message_kind ADD VALUE 'member_joined';

-- The text channel new members are shown first and their join messages are posted in
-- (`app::default_channel`), and whether join messages are posted at all.
ALTER TABLE community
    ADD COLUMN default_channel UUID REFERENCES channel (id),
    ADD COLUMN suppress_join_messages BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX community_default_channel ON community (default_channel)
    WHERE default_channel IS NOT NULL;

-- Each community starts with the text channel `app::default_channel::choose` would pick: one
-- everyone's role may view (View channel, bit 32, after its category's and then its own
-- override for everyone's role), then the oldest, by its UUIDv7 id. Recent messages, which
-- `choose` breaks ties of the same millisecond with, are left out: no two channels made before
-- now share one but those a benchmark seeds.
UPDATE community SET default_channel = chosen.id
FROM (
    SELECT DISTINCT ON (c.community) c.community, c.id
    FROM channel c
    JOIN community_role r
        ON r.community = c.community AND r.everyone AND r.deleted_at IS NULL
    LEFT JOIN category cat ON cat.id = c.parent_category AND cat.deleted_at IS NULL
    LEFT JOIN category_override co ON co.category = cat.id AND co.role = r.id
    LEFT JOIN channel_override cho ON cho.channel = c.id AND cho.role = r.id
    WHERE c.ty = 'text' AND c.parent_channel IS NULL AND c.deleted_at IS NULL
    ORDER BY
        c.community,
        ((((r.permissions & ~coalesce(co.deny, 0)) | coalesce(co.allow, 0))
            & ~coalesce(cho.deny, 0)) | coalesce(cho.allow, 0)) & 4294967296 <> 0 DESC,
        c.id
) chosen
WHERE community.id = chosen.community;
