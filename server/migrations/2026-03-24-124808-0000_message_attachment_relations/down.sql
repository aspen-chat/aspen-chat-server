-- This file should undo anything in `up.sql`
ALTER TABLE message_attachment DROP CONSTRAINT message_fk;
ALTER TABLE message_attachment DROP CONSTRAINT attachment_fk;