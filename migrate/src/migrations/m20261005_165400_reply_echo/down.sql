-- The index allows one echo per reply, so the deleted echoes of a reply that has a later one
-- stop naming it.
UPDATE message AS stale
SET echo_of = NULL
WHERE stale.echo_of IS NOT NULL
  AND stale.deleted_at IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM message AS newer
      WHERE newer.echo_of = stale.echo_of
        AND newer.id <> stale.id
        AND (newer.deleted_at IS NULL OR newer.id > stale.id)
  );
DROP INDEX message_echo;
DROP INDEX message_echo_of_all;
DROP INDEX message_echo_of;
CREATE UNIQUE INDEX message_echo_of ON message (echo_of);
ALTER TABLE message DROP COLUMN echo;
