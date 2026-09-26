DROP INDEX message_poll_shown_once;
ALTER TABLE message DROP COLUMN poll, DROP COLUMN kind;
DROP TABLE poll_vote;
DROP TABLE poll_option;
DROP TABLE poll;
DROP TYPE message_kind;
