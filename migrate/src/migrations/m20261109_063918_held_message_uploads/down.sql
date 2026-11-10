DELETE FROM job WHERE kind = 'releaseHeldMessage'
    AND key IN (SELECT id::text FROM held_message WHERE uploads_due IS NOT NULL);
DELETE FROM held_message WHERE uploads_due IS NOT NULL;

ALTER TABLE held_message
    DROP CONSTRAINT held_message_uploads_due,
    DROP CONSTRAINT held_message_waiting_kind,
    ADD CONSTRAINT held_message_waiting_kind CHECK (waiting = 'mana' OR kind = 'message'),
    DROP COLUMN uploads_due,
    DROP CONSTRAINT held_message_waiting_check,
    ADD CONSTRAINT held_message_waiting_check
        CHECK (waiting IN ('preview', 'mana', 'scheduled'));
