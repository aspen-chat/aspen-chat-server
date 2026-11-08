DELETE FROM job WHERE kind = 'releaseHeldMessage'
    AND key IN (SELECT id::text FROM held_message WHERE send_at IS NOT NULL);
DELETE FROM held_message WHERE send_at IS NOT NULL;

DROP INDEX held_message_mana_by_age;
CREATE INDEX held_message_mana_by_age ON held_message (channel, held_at, id)
    WHERE waiting = 'mana';
DROP INDEX held_message_failed;
DROP INDEX held_message_scheduled;

ALTER TABLE held_message
    DROP CONSTRAINT held_message_failure,
    DROP CONSTRAINT held_message_scheduled_send_at,
    DROP COLUMN stuck_since,
    DROP COLUMN failed_at,
    DROP COLUMN failure,
    DROP COLUMN send_at,
    DROP CONSTRAINT held_message_waiting_check,
    ADD CONSTRAINT held_message_waiting_check CHECK (waiting IN ('preview', 'mana'));
