CREATE TABLE plugin_timer (
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    due TIMESTAMPTZ NOT NULL,
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    claimed_until TIMESTAMPTZ,
    scope_kind TEXT CHECK (scope_kind IN ('community', 'channel', 'user')),
    scope UUID,
    owner_kind TEXT NOT NULL DEFAULT 'deployment'
        CHECK (owner_kind IN ('deployment', 'community', 'direct', 'user')),
    owner UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    PRIMARY KEY (plugin, key),
    CONSTRAINT plugin_timer_scope_whole CHECK ((scope_kind IS NULL) = (scope IS NULL))
);
CREATE INDEX plugin_timer_by_due ON plugin_timer (due);
CREATE INDEX plugin_timer_by_scope ON plugin_timer (scope_kind, scope) WHERE scope IS NOT NULL;
CREATE INDEX plugin_timer_by_owner ON plugin_timer (plugin, owner_kind, owner);
INSERT INTO plugin_timer (plugin, key, due, payload, attempts, scope_kind, scope, owner_kind, owner)
SELECT payload->>'plugin', payload->>'key', due, payload->>'payload', attempts,
       payload->>'scopeKind', (payload->>'scope')::uuid, payload->>'ownerKind',
       (payload->>'owner')::uuid
FROM job WHERE kind = 'firePluginTimer' AND failed_at IS NULL
  AND EXISTS (SELECT 1 FROM plugin p WHERE p.id = job.payload->>'plugin');
DROP INDEX job_plugin_timer_owner;
DROP INDEX job_plugin_timer_scope;
DROP INDEX held_message_by_author;
CREATE INDEX held_message_author_idx ON held_message (author);
ALTER TABLE held_message ADD COLUMN not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
CREATE TABLE attachment_preview_job (
    attachment_id UUID PRIMARY KEY REFERENCES attachment (id) ON DELETE CASCADE,
    priority SMALLINT NOT NULL DEFAULT 0,
    not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    hold_until TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX attachment_preview_job_next_idx
    ON attachment_preview_job (priority DESC, not_before);
INSERT INTO attachment_preview_job (attachment_id, priority, not_before, attempts, hold_until)
SELECT key::uuid, CASE WHEN class <= 1 THEN 10 ELSE 0 END, not_before, attempts,
       COALESCE((payload->>'holdUntil')::timestamptz, now())
FROM job WHERE kind IN ('makePicturePreview', 'makeVideoPoster') AND failed_at IS NULL
  AND EXISTS (SELECT 1 FROM attachment a WHERE a.id = job.key::uuid);
DROP INDEX job_send_email_user;
CREATE TABLE email_outbox (
    id UUID PRIMARY KEY,
    priority SMALLINT NOT NULL,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    address TEXT,
    mail JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_outbox_next ON email_outbox (priority DESC, not_before, id);
INSERT INTO email_outbox (id, priority, "user", address, mail, attempts, not_before)
SELECT id, CASE class WHEN 1 THEN 30 WHEN 2 THEN 20 ELSE 0 END,
       (payload->>'user')::uuid, payload->>'address', payload->'mail', attempts, not_before
FROM job WHERE kind = 'sendEmail'
  AND EXISTS (SELECT 1 FROM "user" u WHERE u.id = (payload->>'user')::uuid);
DROP INDEX user_standing_due;
ALTER TABLE react DROP CONSTRAINT react_custom_emoji_fkey;
ALTER TABLE react ADD CONSTRAINT react_custom_emoji_fkey
    FOREIGN KEY (custom_emoji) REFERENCES custom_emoji (id) ON DELETE CASCADE;
DROP INDEX custom_emoji_name_key;
CREATE UNIQUE INDEX custom_emoji_name_key ON custom_emoji (community, lower(name));
ALTER TABLE custom_emoji DROP COLUMN deleted_at;
ALTER TABLE mention DROP CONSTRAINT mention_target_role_fkey;
ALTER TABLE mention ADD CONSTRAINT mention_target_role_fkey
    FOREIGN KEY (target_role) REFERENCES community_role (id) ON DELETE CASCADE;
ALTER TABLE community_member_role DROP CONSTRAINT community_member_role_role_fkey;
ALTER TABLE community_member_role ADD CONSTRAINT community_member_role_role_fkey
    FOREIGN KEY (role) REFERENCES community_role (id) ON DELETE CASCADE;
ALTER TABLE community_role DROP COLUMN deleted_at;
DROP INDEX refresh_token_expires;
DROP INDEX session_expires;
UPDATE deployment_role SET permissions = permissions & ~(1::bigint << 15);
DROP TABLE job;
