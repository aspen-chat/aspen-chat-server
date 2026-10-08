-- The smallest UUIDv7 of the millisecond `at` falls in: every id made at or after `at` is at
-- least this, so a time bound on rows keyed by UUIDv7 becomes a range on their key's index.
-- `app::read_state::position_at` computes the same id.
CREATE FUNCTION aspen_uuid_floor(at timestamptz) RETURNS uuid
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT (
        substr(h, 1, 8) || '-' || substr(h, 9, 4) || '-7000-8000-000000000000'
    )::uuid
    FROM (SELECT lpad(to_hex(GREATEST(floor(extract(epoch FROM at) * 1000), 0)::bigint), 12, '0') AS h) ms
$$;

-- Whether anything still uses an icon (`app::icon::in_use`): each place an icon can be named,
-- looked up by it, including the profiles reports and warnings keep as they were.
CREATE INDEX user_icon ON "user" (icon) WHERE icon IS NOT NULL;
CREATE INDEX community_icon ON community (icon) WHERE icon IS NOT NULL;
CREATE INDEX custom_emoji_icon ON custom_emoji (icon);
CREATE INDEX message_warning_icon ON message ((warning->'profile'->>'icon'))
    WHERE warning IS NOT NULL;
CREATE INDEX report_profile_icon ON report ((profile->>'icon')) WHERE profile IS NOT NULL;

-- The messages that name a poll, for the moderation log and for the foreign key, which clears
-- them when the poll goes.
CREATE INDEX message_poll ON message (poll) WHERE poll IS NOT NULL;
-- The messages that answer a bot's command, for the foreign key.
CREATE INDEX message_command_bot ON message (command_bot) WHERE command_bot IS NOT NULL;

-- A community's live channels in their order: what its reads, its visibility model, and its
-- channel count read, without the threads and deleted channels `channel_community` also holds.
CREATE INDEX channel_community_live ON channel (community, sort_index)
    WHERE parent_channel IS NULL AND deleted_at IS NULL;

-- Each voice server's calls (its reports of what it holds, and taking it away), and each
-- user's seats in calls (rechecking or ending them all).
CREATE INDEX voice_session_voice_server ON voice_session (voice_server);
CREATE INDEX voice_participant_user ON voice_participant ("user");
-- A voice server's recent failures, counted within the window and pruned past it.
CREATE INDEX voice_server_failure_recent ON voice_server_failure (voice_server, reported_at);

-- A user's capability URLs, revoked together when their sign-ins end.
CREATE INDEX plugin_capability_user ON plugin_capability ("user");

-- The host a federation domain names, without its port: the block lists that cover a domain
-- are found by its host and its parents' (`app::federation::directory::lists_of`).
CREATE FUNCTION aspen_domain_host(domain text) RETURNS text
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$ SELECT split_part(domain, ':', 1) $$;
CREATE INDEX federation_list_entry_host ON federation_list_entry (aspen_domain_host(domain), list);

-- Resolved report cases, newest closed first, as the Resolved tab lists them.
CREATE INDEX report_case_resolved ON report_case (closed_at DESC, id DESC)
    WHERE status = 'resolved';

-- Digests due: only verified addresses are sent one, so only they wait here.
DROP INDEX user_email_digest_due;
CREATE INDEX user_email_digest_due ON user_email (digest_next_at)
    WHERE digest AND verified_at IS NOT NULL;

-- Each answer's voters in the order they voted: its first few in the tally, and the rest a page
-- at a time (`app::poll::read_voters`).
CREATE INDEX poll_vote_by_option ON poll_vote (poll, option_index, "timestamp", "user");

-- The channel a message is read in for searching: its own, or, for a thread reply, the
-- thread's parent, so one indexed column scopes a search to channels and their threads
-- together; a thread's parent never changes.
ALTER TABLE message ADD COLUMN home_channel UUID;
UPDATE message SET home_channel = COALESCE(c.parent_channel, c.id)
FROM channel c WHERE c.id = message.channel;
ALTER TABLE message ALTER COLUMN home_channel SET NOT NULL;
-- When each person's DM was last active: when they joined it, or its latest message since, so
-- their DM list is read most recently active first through an index, a page at a time.
ALTER TABLE dm_recipient ADD COLUMN active_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE dm_recipient SET active_at = GREATEST(
    joined_at,
    COALESCE((SELECT max(m."timestamp") FROM message m WHERE m.channel = dm_recipient.channel),
             joined_at)
);
CREATE INDEX dm_recipient_by_activity ON dm_recipient ("user", active_at DESC, channel DESC);
-- What every inserted message records about where it is, whichever code inserts it: its
-- `home_channel`, and, in a DM or group DM, that the DM is active for its people.
CREATE FUNCTION message_inserted() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    parent UUID;
    kind channel_type;
BEGIN
    SELECT parent_channel, ty INTO parent, kind FROM channel WHERE id = NEW.channel;
    NEW.home_channel := COALESCE(parent, NEW.channel);
    IF kind IN ('dm', 'group_dm') THEN
        UPDATE dm_recipient SET active_at = NEW."timestamp"
        WHERE channel = NEW.channel AND active_at < NEW."timestamp";
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER message_inserted BEFORE INSERT ON message
    FOR EACH ROW EXECUTE FUNCTION message_inserted();
-- Searching text within some places: one GIN index over the place and the words, so the
-- places bound the search rather than every match on the deployment (`app::search`).
CREATE EXTENSION IF NOT EXISTS btree_gin;
DROP INDEX message_search;
CREATE INDEX message_search ON message USING gin (home_channel, to_tsvector('simple', content))
    WHERE deleted_at IS NULL;
-- Searching places by anything else, newest first.
CREATE INDEX message_by_home_channel ON message (home_channel, id) WHERE deleted_at IS NULL;

-- A case's reports, newest first: the latest few that a read of it carries.
CREATE INDEX report_by_case ON report ("case", created_at, id);

-- The dashboard's lists in their orders: people by the name they show, communities by name.
CREATE INDEX user_dashboard_name ON "user" (lower(COALESCE(display_name, name)), id)
    WHERE deleted_at IS NULL AND NOT system;
CREATE INDEX community_dashboard_name ON community (lower(name), id) WHERE deleted_at IS NULL;

-- Plugins' keys compare by their bytes, so a prefix is a range of the key's index rather than
-- a filter over everything a scope holds (`app::plugin::storage::list`).
ALTER TABLE plugin_storage ALTER COLUMN key TYPE TEXT COLLATE "C";

-- A plugin's total is the sum of its owners' shares, read when shown, so no write for one
-- owner waits on another's for the plugin's one row.
ALTER TABLE plugin DROP COLUMN storage_bytes;

-- When each member last came online, kept beside the membership, so a community's member
-- sample reads its most recently seen members through an index rather than ranking everyone.
ALTER TABLE community_user ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE community_user cu SET last_seen_at = u.last_seen_at FROM "user" u WHERE u.id = cu."user";
CREATE INDEX community_user_recent ON community_user (community, last_seen_at DESC, "user");

-- Each member's names as their community searches and sorts them, kept beside the membership:
-- `shown_name`, the name the community shows (nickname, else display name, else username), and
-- `search_name`, all three, each after a space, so a search matches the start of any of them.
-- Triggers keep both current through every change to a nickname or a profile's names.
ALTER TABLE community_user ADD COLUMN shown_name TEXT NOT NULL DEFAULT '';
ALTER TABLE community_user ADD COLUMN search_name TEXT NOT NULL DEFAULT '';
CREATE FUNCTION community_user_names() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    username TEXT;
    shown TEXT;
BEGIN
    SELECT name, display_name INTO username, shown FROM "user" WHERE id = NEW."user";
    NEW.shown_name := lower(COALESCE(NEW.nickname, shown, username, ''));
    NEW.search_name := ' ' || lower(concat_ws(' ', username, shown, NEW.nickname));
    RETURN NEW;
END
$$;
CREATE TRIGGER community_user_names BEFORE INSERT OR UPDATE OF nickname ON community_user
    FOR EACH ROW EXECUTE FUNCTION community_user_names();
CREATE FUNCTION user_names_changed() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE community_user
    SET shown_name = lower(COALESCE(nickname, NEW.display_name, NEW.name)),
        search_name = ' ' || lower(concat_ws(' ', NEW.name, NEW.display_name, nickname))
    WHERE "user" = NEW.id;
    RETURN NULL;
END
$$;
CREATE TRIGGER user_names_changed AFTER UPDATE OF name, display_name ON "user"
    FOR EACH ROW
    WHEN (OLD.name IS DISTINCT FROM NEW.name OR OLD.display_name IS DISTINCT FROM NEW.display_name)
    EXECUTE FUNCTION user_names_changed();
UPDATE community_user cu
SET shown_name = lower(COALESCE(cu.nickname, u.display_name, u.name)),
    search_name = ' ' || lower(concat_ws(' ', u.name, u.display_name, cu.nickname))
FROM "user" u WHERE u.id = cu."user";
-- A community's members by the name it shows, a page at a time, and searched by any part of
-- any of their names within the community alone.
CREATE INDEX community_user_by_shown_name ON community_user (community, shown_name, "user");
CREATE INDEX community_user_search ON community_user USING gin (community, search_name gin_trgm_ops);
