DROP TABLE dm_recipient;
ALTER TABLE message DROP COLUMN thread, DROP COLUMN echo_of;
ALTER TABLE channel
    DROP COLUMN parent_channel,
    DROP COLUMN starter_message,
    DROP COLUMN reply_count,
    DROP COLUMN last_reply_at,
    DROP COLUMN dm_key;

-- Rows only the new types and kind describe go before the enums lose those values: threads,
-- DMs, and group DMs with everything in them, and echoes.
CREATE TEMPORARY TABLE doomed_channel ON COMMIT DROP AS
    SELECT id FROM channel WHERE ty::text IN ('thread', 'dm', 'group_dm');
CREATE TEMPORARY TABLE doomed_message ON COMMIT DROP AS
    SELECT id FROM message
    WHERE kind::text = 'thread_echo' OR channel IN (SELECT id FROM doomed_channel);
DELETE FROM react WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message_attachment WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM pin
    WHERE message_id IN (SELECT id FROM doomed_message)
       OR channel IN (SELECT id FROM doomed_channel);
DELETE FROM message WHERE id IN (SELECT id FROM doomed_message);
DELETE FROM channel WHERE id IN (SELECT id FROM doomed_channel);

-- PostgreSQL cannot remove a value from an enum, so the types are rebuilt without them.
ALTER TYPE channel_type RENAME TO channel_type_old;
CREATE TYPE channel_type AS ENUM ('voice', 'text');
ALTER TABLE channel ALTER COLUMN ty TYPE channel_type USING ty::text::channel_type;
DROP TYPE channel_type_old;

-- The poll index's predicate is typed by the enum, so it is rebuilt around the change.
DROP INDEX message_poll_shown_once;
ALTER TYPE message_kind RENAME TO message_kind_old;
CREATE TYPE message_kind AS ENUM ('standard', 'poll', 'poll_closed');
ALTER TABLE message ALTER COLUMN kind DROP DEFAULT;
ALTER TABLE message ALTER COLUMN kind TYPE message_kind USING kind::text::message_kind;
ALTER TABLE message ALTER COLUMN kind SET DEFAULT 'standard';
DROP TYPE message_kind_old;
CREATE UNIQUE INDEX message_poll_shown_once ON message (poll) WHERE kind = 'poll';
