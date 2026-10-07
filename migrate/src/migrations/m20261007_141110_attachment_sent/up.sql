-- Whether the attachment has ever been in a message. A confirmed attachment that never was,
-- a day after it was confirmed, is swept with its objects (`app::attachment::sweep_unsent`);
-- one an edit took out of its message stays.
ALTER TABLE attachment ADD COLUMN sent BOOLEAN NOT NULL DEFAULT false;

UPDATE attachment SET sent = true
WHERE EXISTS (SELECT 1 FROM message_attachment WHERE attachment_id = attachment.id);

CREATE INDEX attachment_unsent_idx ON attachment (ready_at) WHERE NOT sent;

-- The messages holding an attachment, for deleting one and for checking it is in none.
CREATE INDEX message_attachment_attachment_idx ON message_attachment (attachment_id);
