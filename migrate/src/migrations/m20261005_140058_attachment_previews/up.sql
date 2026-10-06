-- A smaller copy of a picture or video for showing it inline, made by the server from the
-- original (`app::attachment::preview`): where it is stored, what it is, and its size in
-- pixels; all or none.
ALTER TABLE attachment
    ADD COLUMN preview_storage_key TEXT,
    ADD COLUMN preview_mime_type TEXT,
    ADD COLUMN preview_width INTEGER,
    ADD COLUMN preview_height INTEGER,
    ADD CONSTRAINT attachment_preview_whole CHECK (
        (preview_storage_key IS NULL) = (preview_mime_type IS NULL)
        AND (preview_storage_key IS NULL) = (preview_width IS NULL)
        AND (preview_storage_key IS NULL) = (preview_height IS NULL)
    );

-- Attachments waiting for a preview. A server making previews claims a row by pushing
-- `not_before` past the time it needs, under `FOR UPDATE SKIP LOCKED`, and deletes it once the
-- attachment has its preview or is found to need none. The highest `priority` goes first: what
-- was just uploaded before what was uploaded before previews were made. Until `hold_until`, a
-- message holding the attachment waits for its preview (`held_message`).
CREATE TABLE attachment_preview_job (
    attachment_id UUID PRIMARY KEY REFERENCES attachment (id) ON DELETE CASCADE,
    priority SMALLINT NOT NULL DEFAULT 0,
    not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    hold_until TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX attachment_preview_job_next_idx
    ON attachment_preview_job (priority DESC, not_before);

-- Every picture and video already uploaded waits for one too.
INSERT INTO attachment_preview_job (attachment_id)
SELECT id FROM attachment
WHERE ready_at IS NOT NULL
  AND (mime_type LIKE 'image/%' OR mime_type LIKE 'video/%');

-- Messages sent with an attachment whose preview is still being made, waiting for it
-- (`app::message::held`). Any server releases one once none of its attachments holds it, by
-- posting it as it was sent, in the language it was sent in; until then only its author knows
-- of it. A server releasing one claims it by pushing `not_before` past the time it needs.
CREATE TABLE held_message (
    id UUID PRIMARY KEY,
    author UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    attachments UUID[] NOT NULL,
    echo_to_parent BOOLEAN NOT NULL,
    locale TEXT NOT NULL,
    held_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX held_message_attachments_idx ON held_message USING GIN (attachments);
CREATE INDEX held_message_author_idx ON held_message (author);
