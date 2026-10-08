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
