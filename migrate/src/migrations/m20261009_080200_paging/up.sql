-- The lists moderators page through, each in the order it is read, newest first, so a page
-- after any other is a walk of its index.
CREATE INDEX community_ban_listed ON community_ban (community, banned_at DESC, "user" DESC);
CREATE INDEX voice_mute_listed ON voice_mute (community, muted_at DESC, "user" DESC);
CREATE INDEX invite_listed ON invite (community, created_at DESC, code DESC)
    WHERE deleted_at IS NULL;
CREATE INDEX invite_listed_by_creator ON invite (community, created_by, created_at DESC, code DESC)
    WHERE deleted_at IS NULL;
