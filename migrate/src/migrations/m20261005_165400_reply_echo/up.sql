-- `echo` is the live `thread_echo` message that shows a thread reply in the thread's parent
-- channel, kept on the reply as well as on the echo's `echo_of` so a reply read without its
-- parent channel still says whether it has one. Both are written in the transaction that makes
-- the echo, the reply naming an echo inserted after it, so the reference is checked at commit.
ALTER TABLE message
    ADD COLUMN echo UUID REFERENCES message (id) DEFERRABLE INITIALLY DEFERRED;

-- A reply has at most one live echo; once that is deleted it may be echoed again.
DROP INDEX message_echo_of;
CREATE UNIQUE INDEX message_echo_of ON message (echo_of) WHERE deleted_at IS NULL;
-- Both references are indexed whole, deleted echoes included, for the checks their foreign keys
-- make when a message is removed.
CREATE INDEX message_echo_of_all ON message (echo_of) WHERE echo_of IS NOT NULL;
CREATE INDEX message_echo ON message (echo) WHERE echo IS NOT NULL;

-- Last, since the deferred check it queues forbids changing the table's indexes after it.
UPDATE message AS reply
SET echo = echo.id
FROM message AS echo
WHERE echo.echo_of = reply.id
  AND echo.deleted_at IS NULL;
