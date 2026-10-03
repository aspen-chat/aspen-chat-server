DROP TABLE report;
DROP TABLE report_case;
DROP TABLE report_category;
ALTER TABLE message DROP COLUMN warning, DROP COLUMN linked_messages;

-- PostgreSQL cannot remove a value from an enum, so the type is rebuilt without it once its
-- messages are gone; the poll index's predicate is typed by the enum, so it is rebuilt around
-- the change.
CREATE TEMPORARY TABLE doomed_message ON COMMIT DROP AS
    SELECT id FROM message WHERE kind::text = 'warning';
DELETE FROM react WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message_attachment WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM pin WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM mention WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message WHERE id IN (SELECT id FROM doomed_message);
DROP INDEX message_poll_shown_once;
ALTER TYPE message_kind RENAME TO message_kind_old;
CREATE TYPE message_kind AS ENUM (
    'standard', 'poll', 'poll_closed', 'thread_echo', 'call', 'missed_call', 'command'
);
ALTER TABLE message ALTER COLUMN kind DROP DEFAULT;
ALTER TABLE message ALTER COLUMN kind TYPE message_kind USING kind::text::message_kind;
ALTER TABLE message ALTER COLUMN kind SET DEFAULT 'standard';
DROP TYPE message_kind_old;
CREATE UNIQUE INDEX message_poll_shown_once ON message (poll) WHERE kind = 'poll';

UPDATE deployment_role SET permissions = permissions & ~(128 | 256 | 512 | 1024);
UPDATE moderation_log SET action = 'banForeignUser' WHERE action = 'banUser';
UPDATE moderation_log SET action = 'liftForeignUserBan' WHERE action = 'liftUserBan';
-- Bans of this deployment's own users have no place once only foreign users may be banned.
UPDATE "user" SET banned_at = NULL, banned_by = NULL WHERE home_domain IS NULL;
DROP INDEX user_banned;
ALTER TABLE "user"
    DROP CONSTRAINT user_ban_details_need_ban,
    DROP COLUMN banned_until,
    DROP COLUMN ban_reason,
    ADD CONSTRAINT user_ban_is_foreign CHECK (banned_at IS NULL OR home_domain IS NOT NULL);
