ALTER TABLE voice_session DROP COLUMN had_company;

-- Messages of the kind being removed go first, with what hangs off them.
CREATE TEMPORARY TABLE doomed_message ON COMMIT DROP AS
    SELECT id FROM message WHERE kind::text = 'missed_call';
DELETE FROM react WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message_attachment WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM pin WHERE message_id IN (SELECT id FROM doomed_message);
DELETE FROM mention WHERE message IN (SELECT id FROM doomed_message);
DELETE FROM message WHERE id IN (SELECT id FROM doomed_message);

-- PostgreSQL cannot remove a value from an enum, so the type is rebuilt without it; the poll
-- index's predicate is typed by the enum, so it is rebuilt around the change.
DROP INDEX message_poll_shown_once;
ALTER TYPE message_kind RENAME TO message_kind_old;
CREATE TYPE message_kind AS ENUM ('standard', 'poll', 'poll_closed', 'thread_echo', 'call');
ALTER TABLE message ALTER COLUMN kind DROP DEFAULT;
ALTER TABLE message ALTER COLUMN kind TYPE message_kind USING kind::text::message_kind;
ALTER TABLE message ALTER COLUMN kind SET DEFAULT 'standard';
DROP TYPE message_kind_old;
CREATE UNIQUE INDEX message_poll_shown_once ON message (poll) WHERE kind = 'poll';
