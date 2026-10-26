ALTER TABLE message DROP COLUMN growth;

-- PostgreSQL cannot remove a value from an enum, so the type is rebuilt without it once its
-- messages are gone; the poll index's predicate is typed by the enum, so it is rebuilt around
-- the change.
CREATE TEMPORARY TABLE doomed_message ON COMMIT DROP AS
    SELECT id FROM message WHERE kind::text = 'community_grew';
DELETE FROM react WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message_attachment WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM pin WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM mention WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message WHERE id IN (SELECT id FROM doomed_message);
DROP INDEX message_poll_shown_once;
ALTER TYPE message_kind RENAME TO message_kind_old;
CREATE TYPE message_kind AS ENUM (
    'standard', 'poll', 'poll_closed', 'thread_echo', 'call', 'missed_call', 'command', 'warning',
    'member_joined'
);
ALTER TABLE message ALTER COLUMN kind DROP DEFAULT;
ALTER TABLE message ALTER COLUMN kind TYPE message_kind USING kind::text::message_kind;
ALTER TABLE message ALTER COLUMN kind SET DEFAULT 'standard';
DROP TYPE message_kind_old;
CREATE UNIQUE INDEX message_poll_shown_once ON message (poll) WHERE kind = 'poll';

ALTER TABLE community RENAME COLUMN large_at TO everyone_limited_at;

ALTER TABLE deployment_settings
    RENAME CONSTRAINT deployment_settings_large_community_members_check
    TO deployment_settings_everyone_mention_limit_check;
ALTER TABLE deployment_settings RENAME COLUMN large_community_members TO everyone_mention_limit;
