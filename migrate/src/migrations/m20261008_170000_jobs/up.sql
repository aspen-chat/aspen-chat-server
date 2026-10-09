-- Work done in the background, in bounded steps, by whichever server claims it (`app::jobs`).
-- A job is claimed by pushing `not_before` past the time its step needs (its lease), under
-- `FOR UPDATE SKIP LOCKED`, so no two servers run one at once and one left by a server that
-- stopped is claimed again once its lease runs out. A one-shot job is deleted once done; a
-- recurring one (`every`) is due again a period after it was last due.
CREATE TABLE job (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL,
    -- What names the job among others of its kind, for finding, replacing, or cancelling it:
    -- a recurring job's is empty, a one-shot job may have none.
    key TEXT,
    -- How soon it must be done (`JobClass::rank`): lower first.
    class SMALLINT NOT NULL,
    -- When it was due, which its age counts from, and a recurring job's next run follows.
    due TIMESTAMPTZ NOT NULL,
    -- When it may next be claimed.
    not_before TIMESTAMPTZ NOT NULL,
    every INTERVAL,
    payload JSONB NOT NULL,
    -- How far a job worked in steps has come, written in the transaction of the step it follows.
    progress JSONB,
    -- Claims since it last made progress.
    attempts INTEGER NOT NULL DEFAULT 0,
    -- When the claim running it began; empty while it waits.
    running_since TIMESTAMPTZ,
    -- When it was given up, kept for the operator with why, until it is retried or swept.
    failed_at TIMESTAMPTZ,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (kind, key)
);
-- What a server claims: the next of each kind it runs, in each class, in order.
CREATE INDEX job_next ON job (kind, class, not_before, id) WHERE failed_at IS NULL;
-- What the dashboard lists: what runs, what waits by class, and what was given up.
CREATE INDEX job_running ON job (running_since) WHERE running_since IS NOT NULL;
CREATE INDEX job_waiting ON job (class, due, id)
    WHERE failed_at IS NULL AND running_since IS NULL AND every IS NULL;
CREATE INDEX job_failed ON job (failed_at DESC) WHERE failed_at IS NOT NULL;

-- Viewing jobs is a deployment permission of its own; roles that manage the deployment's
-- settings are given it.
UPDATE deployment_role SET permissions = permissions | (1::bigint << 15)
WHERE permissions & (1::bigint << 11) <> 0;

-- Sessions and sign-ins by when they expire, which the sweep that deletes ended ones walks.
CREATE INDEX session_expires ON session (expires);
CREATE INDEX refresh_token_expires ON refresh_token (expires);

-- Each open poll is closed at its deadline by a job of its own.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
SELECT gen_random_uuid(), 'closePoll', id::text, 1, closes_at, closes_at,
       jsonb_build_object('poll', id)
FROM poll WHERE closed_at IS NULL;

-- A deleted role is marked, its permissions and overrides taken away at once, and its holders
-- and the tags of it taken off by a job a batch at a time (`purgeRole`), after which the row
-- goes. Its holders and tags therefore no longer go with it by the foreign key in one statement.
ALTER TABLE community_role ADD COLUMN deleted_at TIMESTAMPTZ;
ALTER TABLE community_member_role DROP CONSTRAINT community_member_role_role_fkey;
ALTER TABLE community_member_role ADD CONSTRAINT community_member_role_role_fkey
    FOREIGN KEY (role) REFERENCES community_role (id) ON DELETE NO ACTION;
ALTER TABLE mention DROP CONSTRAINT mention_target_role_fkey;
ALTER TABLE mention ADD CONSTRAINT mention_target_role_fkey
    FOREIGN KEY (target_role) REFERENCES community_role (id) ON DELETE NO ACTION;

-- A deleted custom emoji is marked and announced at once, its name free again, and its
-- reactions taken off by a job a batch at a time (`purgeCustomEmoji`) before it goes.
ALTER TABLE custom_emoji ADD COLUMN deleted_at TIMESTAMPTZ;
DROP INDEX custom_emoji_name_key;
CREATE UNIQUE INDEX custom_emoji_name_key ON custom_emoji (community, lower(name))
    WHERE deleted_at IS NULL;
ALTER TABLE react DROP CONSTRAINT react_custom_emoji_fkey;
ALTER TABLE react ADD CONSTRAINT react_custom_emoji_fkey
    FOREIGN KEY (custom_emoji) REFERENCES custom_emoji (id) ON DELETE NO ACTION;

-- Foreign users by when their home last confirmed them, never first, which a standing pass
-- walks to ask about those due the longest.
CREATE INDEX user_standing_due ON "user" (home_confirmed_at)
    WHERE home_domain IS NOT NULL AND deleted_at IS NULL;

-- Mail waiting to be sent is a job of its own, its class taken from its priority; the outbox
-- goes. A piece's recipient is in its payload, by which the account's or address's going takes
-- its mail with it.
INSERT INTO job (id, kind, key, class, due, not_before, payload, attempts)
SELECT id, 'sendEmail', NULL,
       CASE WHEN priority >= 30 THEN 1 WHEN priority >= 15 THEN 2 ELSE 3 END,
       not_before, not_before,
       jsonb_build_object('user', "user", 'address', address, 'mail', mail), attempts
FROM email_outbox;
DROP TABLE email_outbox;
CREATE INDEX job_send_email_user ON job ((payload->>'user')) WHERE kind = 'sendEmail';
-- A newsletter post sent and not yet queued is queued by a job.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
SELECT gen_random_uuid(), 'queueNewsletter', id::text, 3, sent_at, now(),
       jsonb_build_object('post', id)
FROM newsletter_post WHERE sent_at IS NOT NULL AND queued_at IS NULL;

-- Making a picture's preview or a video's poster is a job of its own, keyed by its attachment,
-- whose payload holds until when messages holding the attachment wait for it (`holdUntil`); the
-- preview queue goes. What was just uploaded is interactive, what was queued when previews
-- began to be made bulk.
INSERT INTO job (id, kind, key, class, due, not_before, payload, attempts)
SELECT gen_random_uuid(),
       CASE WHEN a.mime_type LIKE 'video/%' THEN 'makeVideoPoster' ELSE 'makePicturePreview' END,
       q.attachment_id::text, CASE WHEN q.priority > 0 THEN 1 ELSE 3 END,
       q.not_before, q.not_before, jsonb_build_object('holdUntil', q.hold_until), q.attempts
FROM attachment_preview_job q JOIN attachment a ON a.id = q.attachment_id;
DROP TABLE attachment_preview_job;

-- Each held message is posted by a job of its own (`releaseHeldMessage`), keyed by it, which
-- holds its claims and attempts; an author's are posted in the order they were sent, through
-- `held_message_by_author`.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
SELECT gen_random_uuid(), 'releaseHeldMessage', id::text, 1, held_at, now(), '{}'
FROM held_message;
ALTER TABLE held_message DROP COLUMN not_before, DROP COLUMN attempts;
DROP INDEX held_message_author_idx;
CREATE INDEX held_message_by_author ON held_message (author, held_at, id);

-- Each plugin's timer is a job of its own (`firePluginTimer`), keyed by its plugin and its key
-- (`plugin/key`), due when the timer is, whose payload names the plugin, the key, what it hands
-- the plugin, the scope it was set in, and the owner whose share it counts in. The timer table
-- goes.
INSERT INTO job (id, kind, key, class, due, not_before, payload, attempts)
SELECT gen_random_uuid(), 'firePluginTimer', plugin || '/' || key, 2, due, due,
       jsonb_strip_nulls(jsonb_build_object(
           'plugin', plugin, 'key', key, 'payload', payload, 'scopeKind', scope_kind,
           'scope', scope, 'ownerKind', owner_kind, 'owner', owner)),
       attempts
FROM plugin_timer;
DROP TABLE plugin_timer;
-- A plugin's timers in one owner's share, which setting one counts and purging the plugin walks.
CREATE INDEX job_plugin_timer_owner
    ON job ((payload->>'plugin'), (payload->>'ownerKind'), (payload->>'owner'))
    WHERE kind = 'firePluginTimer';
-- The timers set in a scope, which go with what the plugin keeps there.
CREATE INDEX job_plugin_timer_scope ON job ((payload->>'scopeKind'), (payload->>'scope'))
    WHERE kind = 'firePluginTimer' AND payload ? 'scope';
