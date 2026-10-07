-- Whose share of a plugin's storage and timers a scope draws on (`app::plugin::storage::Owner`):
-- a community for itself and its channels and their threads, a DM or group DM ('direct', named by
-- its root channel) for itself and its threads, a user for their own scope, and the deployment for
-- the plugin's own. The manifest's `storageQuota` and the timer limit apply to each owner apart,
-- so no one community can use up what a plugin may keep for every other.

-- How many bytes of keys and values each owner's scopes hold, per plugin.
CREATE TABLE plugin_storage_usage (
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('deployment', 'community', 'direct', 'user')),
    owner UUID NOT NULL,
    bytes BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (plugin, owner_kind, owner)
);

-- The owner of a channel scope: its community, or its thread's parent's, or the DM it is.
CREATE FUNCTION pg_temp.channel_owner(channel_id UUID, OUT kind TEXT, OUT id UUID) AS $$
    SELECT CASE WHEN coalesce(c.community, p.community) IS NULL THEN 'direct' ELSE 'community' END,
           coalesce(c.community, p.community, c.parent_channel, c.id)
    FROM channel c LEFT JOIN channel p ON p.id = c.parent_channel
    WHERE c.id = channel_id
$$ LANGUAGE sql STABLE;

INSERT INTO plugin_storage_usage (plugin, owner_kind, owner, bytes)
SELECT s.plugin, o.kind, o.id, sum(octet_length(s.key) + octet_length(s.value))
FROM plugin_storage s
CROSS JOIN LATERAL (
    SELECT (pg_temp.channel_owner(s.scope)).kind, (pg_temp.channel_owner(s.scope)).id
    WHERE s.scope_kind = 'channel'
    UNION ALL
    SELECT s.scope_kind, s.scope WHERE s.scope_kind <> 'channel'
) o
WHERE o.kind IS NOT NULL
GROUP BY s.plugin, o.kind, o.id;

-- Each timer's owner, counted against the timer limit.
ALTER TABLE plugin_timer
    ADD COLUMN owner_kind TEXT NOT NULL DEFAULT 'deployment'
        CHECK (owner_kind IN ('deployment', 'community', 'direct', 'user')),
    ADD COLUMN owner UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';
UPDATE plugin_timer t SET owner_kind = o.kind, owner = o.id
FROM plugin_timer s
CROSS JOIN LATERAL (
    SELECT (pg_temp.channel_owner(s.scope)).kind, (pg_temp.channel_owner(s.scope)).id
    WHERE s.scope_kind = 'channel'
    UNION ALL
    SELECT s.scope_kind, s.scope WHERE s.scope_kind IN ('community', 'user')
) o
WHERE t.plugin = s.plugin AND t.key = s.key AND o.kind IS NOT NULL;
CREATE INDEX plugin_timer_by_owner ON plugin_timer (plugin, owner_kind, owner);
