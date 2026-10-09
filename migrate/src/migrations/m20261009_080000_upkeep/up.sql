-- Community bans by when they end, which the sweep of ended ones walks (`sweepExpired`).
CREATE INDEX community_ban_until ON community_ban (until) WHERE until IS NOT NULL;
-- Voice failures by when they were reported, which the same sweep walks across every server.
CREATE INDEX voice_server_failure_reported ON voice_server_failure (reported_at);

-- Each community's member count, which the dashboard lists and sorts by, recounted by a
-- recurring job (`recountMembers`) rather than kept by every join and leave.
ALTER TABLE community ADD COLUMN member_count INTEGER NOT NULL DEFAULT 0;
UPDATE community c SET member_count = (SELECT count(*) FROM community_user cu WHERE cu.community = c.id);
CREATE INDEX community_by_members ON community (member_count, id) WHERE deleted_at IS NULL;

-- The deployment's totals, one row a day (UTC), written each hour by `recordStats` from the
-- day's last row and what changed since: people (the system account left out) and
-- communities, neither deleted, as of `taken_at`. The dashboard's overview reads the latest
-- row and adds what changed since; its growth chart reads the days.
CREATE TABLE deployment_stats (
    day DATE PRIMARY KEY,
    taken_at TIMESTAMPTZ NOT NULL,
    users BIGINT NOT NULL,
    communities BIGINT NOT NULL
);
-- What changed since a row was taken, each found through its own index; communities made
-- since are found by their UUIDv7 ids.
CREATE INDEX user_created ON "user" (created_at) WHERE NOT system;
CREATE INDEX user_deleted ON "user" (deleted_at) WHERE deleted_at IS NOT NULL AND NOT system;
CREATE INDEX community_deleted ON community (deleted_at) WHERE deleted_at IS NOT NULL;
-- Every day so far, from each account's and community's making and deletion.
WITH made AS (
    SELECT created_at AS at, 1 AS users, 0 AS communities FROM "user" WHERE NOT system
    UNION ALL
    SELECT deleted_at, -1, 0 FROM "user" WHERE deleted_at IS NOT NULL AND NOT system
    UNION ALL
    SELECT to_timestamp(('x' || lpad(substr(replace(id::text, '-', ''), 1, 12), 16, '0'))::bit(64)::bigint / 1000.0),
           0, 1
    FROM community
    UNION ALL
    SELECT deleted_at, 0, -1 FROM community WHERE deleted_at IS NOT NULL
),
per_day AS (
    SELECT (at AT TIME ZONE 'UTC')::date AS day, sum(users) AS users, sum(communities) AS communities
    FROM made GROUP BY 1
),
days AS (
    SELECT generate_series(
        COALESCE((SELECT min(day) FROM per_day), (now() AT TIME ZONE 'UTC')::date),
        (now() AT TIME ZONE 'UTC')::date, interval '1 day')::date AS day
)
INSERT INTO deployment_stats (day, taken_at, users, communities)
SELECT days.day,
       LEAST((days.day + 1)::timestamp AT TIME ZONE 'UTC', now()),
       sum(COALESCE(per_day.users, 0)) OVER (ORDER BY days.day),
       sum(COALESCE(per_day.communities, 0)) OVER (ORDER BY days.day)
FROM days LEFT JOIN per_day USING (day);

-- An icon that may no longer be used is checked by a job of its own (`forgetIcon`, keyed by
-- it), which deletes it and its picture if nothing uses it: queued a day after its upload is
-- confirmed, for one never put to use, and as soon as what used it lets it go or is deleted.
CREATE FUNCTION aspen_forget_icon(icon uuid, after interval) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO job (id, kind, key, class, due, not_before, payload)
    VALUES (gen_random_uuid(), 'forgetIcon', icon::text, 4, now() + after, now() + after, '{}')
    ON CONFLICT (kind, key) DO UPDATE
    SET not_before = LEAST(job.not_before, excluded.not_before), due = LEAST(job.due, excluded.due)
    WHERE job.running_since IS NULL AND job.failed_at IS NULL
$$;
CREATE FUNCTION aspen_icon_let_go() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.icon IS NOT NULL AND (TG_OP = 'DELETE' OR OLD.icon IS DISTINCT FROM NEW.icon) THEN
        PERFORM aspen_forget_icon(OLD.icon, interval '0');
    END IF;
    RETURN NULL;
END
$$;
CREATE TRIGGER user_icon_let_go AFTER UPDATE OF icon OR DELETE ON "user"
    FOR EACH ROW EXECUTE FUNCTION aspen_icon_let_go();
CREATE TRIGGER community_icon_let_go AFTER UPDATE OF icon OR DELETE ON community
    FOR EACH ROW EXECUTE FUNCTION aspen_icon_let_go();
CREATE TRIGGER deployment_settings_icon_let_go AFTER UPDATE OF icon ON deployment_settings
    FOR EACH ROW EXECUTE FUNCTION aspen_icon_let_go();
CREATE FUNCTION aspen_icon_confirmed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.ready_at IS NULL AND NEW.ready_at IS NOT NULL THEN
        PERFORM aspen_forget_icon(NEW.id, interval '1 day');
    END IF;
    RETURN NULL;
END
$$;
CREATE TRIGGER icon_confirmed AFTER UPDATE OF ready_at ON icon
    FOR EACH ROW EXECUTE FUNCTION aspen_icon_confirmed();
-- Every icon already confirmed is checked once.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
SELECT gen_random_uuid(), 'forgetIcon', id::text, 4, now(), now(), '{}'
FROM icon WHERE ready_at IS NOT NULL;
