-- Who uploaded each attachment: only they may read it before it is in a message, put it in
-- one, or delete it. Attachments uploaded before this was recorded have none.
ALTER TABLE attachment ADD COLUMN uploader UUID REFERENCES "user" (id) ON DELETE SET NULL;
