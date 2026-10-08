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
-- thread's parent. Filled by `message_home_channel` on insert, since a thread's parent never
-- changes, so one indexed column scopes a search to channels and their threads together.
ALTER TABLE message ADD COLUMN home_channel UUID;
UPDATE message SET home_channel = COALESCE(c.parent_channel, c.id)
FROM channel c WHERE c.id = message.channel;
ALTER TABLE message ALTER COLUMN home_channel SET NOT NULL;
CREATE FUNCTION message_home_channel() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.home_channel := COALESCE(
        (SELECT parent_channel FROM channel WHERE id = NEW.channel),
        NEW.channel
    );
    RETURN NEW;
END
$$;
CREATE TRIGGER message_home_channel BEFORE INSERT ON message
    FOR EACH ROW EXECUTE FUNCTION message_home_channel();
-- Searching text within some places: one GIN index over the place and the words, so the
-- places bound the search rather than every match on the deployment (`app::search`).
CREATE EXTENSION IF NOT EXISTS btree_gin;
DROP INDEX message_search;
CREATE INDEX message_search ON message USING gin (home_channel, to_tsvector('simple', content))
    WHERE deleted_at IS NULL;
-- Searching places by anything else, newest first.
CREATE INDEX message_by_home_channel ON message (home_channel, id) WHERE deleted_at IS NULL;
