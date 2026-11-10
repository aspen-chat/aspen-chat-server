-- A message sent while its files are still uploading is a held message
-- (`app::message::held`): it waits for every attachment it names to be confirmed
-- (`waiting = 'uploads'`), which the request confirming the last of them finds through
-- `held_message_attachments_idx`. `uploads_due` is when its job gives up waiting for them; it
-- stays set once they are all there, marking the row as one that waited for its uploads. A
-- slash command waits for its files the same way, and then, like a message, for their
-- previews.
ALTER TABLE held_message
    ADD COLUMN uploads_due TIMESTAMPTZ,
    DROP CONSTRAINT held_message_waiting_check,
    ADD CONSTRAINT held_message_waiting_check
        CHECK (waiting IN ('preview', 'mana', 'scheduled', 'uploads')),
    DROP CONSTRAINT held_message_waiting_kind,
    ADD CONSTRAINT held_message_waiting_kind
        CHECK (waiting = 'mana' OR kind = 'message'
               OR (kind = 'command' AND uploads_due IS NOT NULL)),
    ADD CONSTRAINT held_message_uploads_due
        CHECK (waiting <> 'uploads' OR uploads_due IS NOT NULL);
